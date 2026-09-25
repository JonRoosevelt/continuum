use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_app_kit::{
    NSPasteboard, NSPasteboardAccessBehavior, NSPasteboardItem, NSPasteboardWriting,
};
use objc2_foundation::{NSArray, NSData, NSString};

use continuum_core::{ClipItem, Representation, PNG_MIME, TIFF_MIME};

use crate::backend::{AccessBehavior, ClipboardBackend, ClipboardError};
use crate::mime;

pub struct MacClipboard {
    last_change_count: isize,
}

impl MacClipboard {
    #[must_use]
    pub fn new() -> Self {
        let count = NSPasteboard::generalPasteboard().changeCount();
        Self {
            last_change_count: count,
        }
    }

    /// macOS 15.4+ per-app pasteboard access policy for the general pasteboard.
    #[must_use]
    pub fn access_behavior() -> AccessBehavior {
        let behavior = NSPasteboard::generalPasteboard().accessBehavior();
        if behavior == NSPasteboardAccessBehavior::AlwaysAllow {
            AccessBehavior::AlwaysAllow
        } else if behavior == NSPasteboardAccessBehavior::AlwaysDeny {
            AccessBehavior::AlwaysDeny
        } else if behavior == NSPasteboardAccessBehavior::Ask {
            AccessBehavior::Ask
        } else {
            AccessBehavior::Default
        }
    }
}

impl Default for MacClipboard {
    fn default() -> Self {
        Self::new()
    }
}

impl ClipboardBackend for MacClipboard {
    fn read(&mut self) -> Result<Option<Vec<ClipItem>>, ClipboardError> {
        let pasteboard = NSPasteboard::generalPasteboard();
        let count = pasteboard.changeCount();
        if count == self.last_change_count {
            return Ok(None);
        }
        self.last_change_count = count;
        read_pasteboard(&pasteboard)
    }

    fn read_current(&mut self) -> Result<Option<Vec<ClipItem>>, ClipboardError> {
        read_pasteboard(&NSPasteboard::generalPasteboard())
    }

    fn write(&mut self, items: &[ClipItem]) -> Result<(), ClipboardError> {
        let pasteboard = NSPasteboard::generalPasteboard();
        pasteboard.clearContents();

        let mut objects: Vec<Retained<ProtocolObject<dyn NSPasteboardWriting>>> = Vec::new();
        for item in items {
            let pasteboard_item = NSPasteboardItem::new();
            let mut wrote = false;
            for representation in &item.representations {
                let Some(target) = mime::native::to_native(&representation.mime) else {
                    continue;
                };
                let data = NSData::with_bytes(&representation.bytes);
                let type_ns = NSString::from_str(target);
                if pasteboard_item.setData_forType(&data, &type_ns) {
                    wrote = true;
                }
            }
            if wrote {
                objects.push(ProtocolObject::from_retained(pasteboard_item));
            }
        }

        if objects.is_empty() {
            return Err(ClipboardError::Write(
                "clipboard payload has no supported representations".into(),
            ));
        }

        let array = NSArray::from_retained_slice(&objects);
        if !pasteboard.writeObjects(&array) {
            return Err(ClipboardError::Write(
                "NSPasteboard::writeObjects failed".into(),
            ));
        }

        self.last_change_count = pasteboard.changeCount();
        Ok(())
    }
}

fn read_pasteboard(pasteboard: &NSPasteboard) -> Result<Option<Vec<ClipItem>>, ClipboardError> {
    let Some(items) = pasteboard.pasteboardItems() else {
        return Ok(None);
    };

    let mut result = Vec::new();
    for item in items.iter() {
        let types = item.types();
        let type_names: Vec<String> = types.iter().map(|ty| ty.to_string()).collect();
        if mime::is_sensitive(&type_names) {
            continue;
        }
        let mut representations = Vec::new();
        for ty in types.iter() {
            let Some(target) = mime::native::from_native(&ty.to_string()) else {
                continue;
            };
            let Some(data) = item.dataForType(&ty) else {
                continue;
            };
            representations.push(Representation::new(target, data.to_vec()));
        }
        if !representations.is_empty() {
            normalize_images(&mut representations);
            result.push(ClipItem::new(representations)?);
        }
    }

    Ok((!result.is_empty()).then_some(result))
}

/// Prefer PNG for images (interoperable with Linux); convert a lone TIFF to PNG.
fn normalize_images(representations: &mut Vec<Representation>) {
    if representations.iter().any(|rep| rep.mime == PNG_MIME) {
        representations.retain(|rep| rep.mime != TIFF_MIME);
        return;
    }
    if let Some(index) = representations.iter().position(|rep| rep.mime == TIFF_MIME) {
        if let Some(png) = tiff_to_png(&representations[index].bytes) {
            representations[index] = Representation::new(PNG_MIME, png);
        }
    }
}

#[cfg(target_os = "macos")]
fn tiff_to_png(tiff: &[u8]) -> Option<Vec<u8>> {
    let image = image::load_from_memory_with_format(tiff, image::ImageFormat::Tiff).ok()?;
    let mut png = Vec::new();
    image
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .ok()?;
    Some(png)
}

#[cfg(test)]
mod tests {
    use super::*;
    use continuum_core::PLAIN_TEXT_MIME;
    use objc2_app_kit::NSPasteboardTypeString;

    #[test]
    #[ignore = "touches the real pasteboard; run locally with --ignored"]
    #[allow(unsafe_code)]
    fn roundtrips_plain_text() {
        let pasteboard = NSPasteboard::generalPasteboard();
        // SAFETY: framework-provided constant; read-only.
        let text_type = unsafe { NSPasteboardTypeString };
        let original = pasteboard
            .stringForType(text_type)
            .map(|value| value.to_string());

        let item = ClipItem::new(vec![Representation::text(
            PLAIN_TEXT_MIME,
            "continuum-roundtrip",
        )])
        .unwrap();
        let mut clipboard = MacClipboard::new();
        clipboard.write(std::slice::from_ref(&item)).unwrap();

        let read = read_pasteboard(&pasteboard)
            .unwrap()
            .expect("pasteboard changed after write");
        assert_eq!(read[0].plain_text(), Some("continuum-roundtrip"));

        pasteboard.clearContents();
        if let Some(text) = original {
            let restored = NSString::from_str(&text);
            pasteboard.setString_forType(&restored, text_type);
        }
    }

    #[test]
    #[ignore = "touches the real pasteboard; run locally with --ignored"]
    #[allow(unsafe_code)]
    fn roundtrips_multiple_items() {
        let pasteboard = NSPasteboard::generalPasteboard();
        // SAFETY: framework-provided constant; read-only.
        let text_type = unsafe { NSPasteboardTypeString };
        let original = pasteboard
            .stringForType(text_type)
            .map(|value| value.to_string());

        let first = ClipItem::new(vec![Representation::text(PLAIN_TEXT_MIME, "first")]).unwrap();
        let second = ClipItem::new(vec![Representation::text(PLAIN_TEXT_MIME, "second")]).unwrap();
        let mut clipboard = MacClipboard::new();
        clipboard.write(&[first, second]).unwrap();

        let read = read_pasteboard(&pasteboard)
            .unwrap()
            .expect("pasteboard changed after write");
        assert_eq!(read.len(), 2);
        assert_eq!(read[0].plain_text(), Some("first"));
        assert_eq!(read[1].plain_text(), Some("second"));

        pasteboard.clearContents();
        if let Some(text) = original {
            let restored = NSString::from_str(&text);
            pasteboard.setString_forType(&restored, text_type);
        }
    }
}
