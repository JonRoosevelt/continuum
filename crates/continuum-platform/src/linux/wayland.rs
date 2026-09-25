use std::io::Read as _;
use std::time::{Duration, Instant};

use continuum_core::{
    content_hash, ClipItem, ContentHash, Representation, PLAIN_TEXT_MIME, PNG_MIME,
};
use wl_clipboard_rs::copy::{
    self, ClipboardType as CopyClipboardType, MimeSource, Options, Seat as CopySeat, Source,
};
use wl_clipboard_rs::paste::{
    self, get_contents, get_mime_types, ClipboardType as PasteClipboardType,
    MimeType as PasteMimeType, Seat as PasteSeat,
};

use crate::backend::{ClipboardBackend, ClipboardError};
use crate::files;
use crate::mime;

pub struct WaylandClipboard {
    last_hash: Option<ContentHash>,
}

impl WaylandClipboard {
    pub fn new() -> Result<Self, ClipboardError> {
        match get_mime_types(PasteClipboardType::Regular, PasteSeat::Unspecified) {
            Ok(_)
            | Err(
                paste::Error::ClipboardEmpty
                | paste::Error::NoSeats
                | paste::Error::NoMimeType
                | paste::Error::SeatNotFound,
            ) => Ok(Self { last_hash: None }),
            Err(_) => Err(ClipboardError::Unavailable),
        }
    }

    fn offered_mime_types(&self) -> Result<Option<Vec<String>>, ClipboardError> {
        match get_mime_types(PasteClipboardType::Regular, PasteSeat::Unspecified) {
            Ok(set) => Ok(Some(set.into_iter().collect())),
            Err(
                paste::Error::ClipboardEmpty
                | paste::Error::NoSeats
                | paste::Error::NoMimeType
                | paste::Error::SeatNotFound,
            ) => Ok(None),
            Err(err) => Err(ClipboardError::Read(err.to_string())),
        }
    }

    fn fetch(&self, mime_type: PasteMimeType<'_>) -> Result<Option<Vec<u8>>, ClipboardError> {
        match get_contents(
            PasteClipboardType::Regular,
            PasteSeat::Unspecified,
            mime_type,
        ) {
            Ok((mut pipe, _actual_mime)) => {
                let mut bytes = Vec::new();
                pipe.read_to_end(&mut bytes)
                    .map_err(|err| ClipboardError::Read(err.to_string()))?;
                Ok((!bytes.is_empty()).then_some(bytes))
            }
            Err(
                paste::Error::ClipboardEmpty
                | paste::Error::NoSeats
                | paste::Error::NoMimeType
                | paste::Error::SeatNotFound,
            ) => Ok(None),
            Err(err) => Err(ClipboardError::Read(err.to_string())),
        }
    }

    /// Blocks until the clipboard reflects `representation`, so a poll right after a large
    /// `wl-copy` write does not read the stale clipboard and rebroadcast it.
    fn await_written(&self, representation: &Representation) {
        let deadline = Instant::now() + Duration::from_millis(3000);
        while Instant::now() < deadline {
            if let Ok(Some(bytes)) = self.fetch(PasteMimeType::Specific(&representation.mime)) {
                if bytes == representation.bytes {
                    return;
                }
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }
}

impl ClipboardBackend for WaylandClipboard {
    fn read(&mut self) -> Result<Option<Vec<ClipItem>>, ClipboardError> {
        let Some(items) = self.read_current()? else {
            return Ok(None);
        };
        // Wayland data-control exposes no change counter; content hashing is the shared
        // change-detection mechanism, and write() seeds the same hash to suppress echoes.
        let hash = content_hash(&items);
        if self.last_hash == Some(hash) {
            return Ok(None);
        }
        self.last_hash = Some(hash);
        Ok(Some(items))
    }

    fn read_current(&mut self) -> Result<Option<Vec<ClipItem>>, ClipboardError> {
        let Some(offered) = self.offered_mime_types()? else {
            return Ok(None);
        };
        if mime::is_sensitive(&offered) {
            return Ok(None);
        }

        if offered.iter().any(|mime| mime == files::URI_LIST_MIME) {
            if let Some(bytes) = self.fetch(PasteMimeType::Specific(files::URI_LIST_MIME))? {
                let mut items = Vec::new();
                for path in files::parse_uri_list(&bytes) {
                    if let Some((name, content)) = files::read_file(&path) {
                        items.push(ClipItem::new(vec![files::encode(&name, &content)])?);
                    }
                }
                if !items.is_empty() {
                    return Ok(Some(items));
                }
            }
        }

        // Plain text is normalized to PLAIN_TEXT_MIME regardless of the concrete text MIME
        // wl-clipboard-rs selected, so read hashing matches what write() offers.
        let mut representations = Vec::new();
        if let Some(bytes) = self.fetch(PasteMimeType::Text)? {
            representations.push(Representation::new(PLAIN_TEXT_MIME, bytes));
        }
        if let Some(bytes) = self.fetch(PasteMimeType::Specific(PNG_MIME))? {
            representations.push(Representation::new(PNG_MIME, bytes));
        }

        if representations.is_empty() {
            return Ok(None);
        }
        Ok(Some(vec![ClipItem::new(representations)?]))
    }

    fn write(&mut self, items: &[ClipItem]) -> Result<(), ClipboardError> {
        if items.is_empty() {
            return Err(ClipboardError::Write(
                "clipboard payload has no items".into(),
            ));
        }

        let files_only = items.iter().all(|item| {
            item.representations.len() == 1 && files::is_file(&item.representations[0])
        });
        if files_only {
            let mut paths = Vec::new();
            let mut saved = Vec::new();
            for item in items {
                let Some((name, content)) = files::decode(&item.representations[0]) else {
                    continue;
                };
                let path = files::save(&name, &content)
                    .map_err(|err| ClipboardError::Write(err.to_string()))?;
                let saved_name = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("file")
                    .to_string();
                saved.push(ClipItem::new(vec![files::encode(&saved_name, &content)])?);
                paths.push(path);
            }
            if paths.is_empty() {
                return Err(ClipboardError::Write("no files could be saved".into()));
            }
            write_files_offer(&paths)?;
            self.last_hash = Some(content_hash(&saved));
            return Ok(());
        }

        let total: usize = items
            .iter()
            .flat_map(|item| &item.representations)
            .filter(|rep| is_supported(&rep.mime))
            .map(|rep| rep.bytes.len())
            .sum();

        if total > INLINE_MAX {
            let written = write_via_wl_copy(items)?;
            self.await_written(&written);
            let seeded = ClipItem::new(vec![written])?;
            self.last_hash = Some(content_hash(std::slice::from_ref(&seeded)));
        } else {
            write_inline(items)?;
            self.last_hash = Some(content_hash(items));
        }
        Ok(())
    }
}

// wl-clipboard-rs truncates payloads above one pipe buffer (~64 KiB); above this size we
// hand the data to `wl-copy`, which streams it reliably.
const INLINE_MAX: usize = 48 * 1024;

fn is_supported(mime: &str) -> bool {
    matches!(mime, PLAIN_TEXT_MIME | PNG_MIME)
}

/// Offers a file list under the MIME types file managers look for: `text/uri-list` (KDE, most
/// apps) and `x-special/gnome-copied-files` (GNOME/Nautilus, which gates Paste on it).
fn write_files_offer(paths: &[std::path::PathBuf]) -> Result<(), ClipboardError> {
    let mut options = Options::new();
    options
        .clipboard(CopyClipboardType::Regular)
        .seat(CopySeat::All)
        .foreground(false)
        .omit_additional_text_mime_types(true);
    let sources = vec![
        MimeSource {
            source: Source::Bytes(files::uri_list_bytes(paths).into_boxed_slice()),
            mime_type: copy::MimeType::Specific(files::URI_LIST_MIME.to_string()),
        },
        MimeSource {
            source: Source::Bytes(files::gnome_copied_files_bytes(paths).into_boxed_slice()),
            mime_type: copy::MimeType::Specific(files::GNOME_COPIED_FILES_MIME.to_string()),
        },
    ];
    copy::copy_multi(options, sources).map_err(|err| ClipboardError::Write(err.to_string()))
}

fn write_inline(items: &[ClipItem]) -> Result<(), ClipboardError> {
    let mut sources = Vec::new();
    for item in items {
        for representation in &item.representations {
            let mime_type = match representation.mime.as_str() {
                PLAIN_TEXT_MIME => copy::MimeType::Text,
                PNG_MIME => copy::MimeType::Specific(representation.mime.clone()),
                _ => continue,
            };
            sources.push(MimeSource {
                source: Source::Bytes(representation.bytes.clone().into_boxed_slice()),
                mime_type,
            });
        }
    }
    if sources.is_empty() {
        return Err(ClipboardError::Write(
            "clipboard payload has no supported representations".into(),
        ));
    }

    let mut options = Options::new();
    options
        .clipboard(CopyClipboardType::Regular)
        .seat(CopySeat::All)
        .foreground(false);
    copy::copy_multi(options, sources).map_err(|err| ClipboardError::Write(err.to_string()))
}

/// Large payloads: offer the single best representation through `wl-copy`.
///
/// Only one representation can be offered this way, so text is preferred over HTML: handing
/// HTML to `wl-copy` makes it offer `text/plain` with the same HTML bytes, which corrupts
/// pasted text.
fn write_via_wl_copy(items: &[ClipItem]) -> Result<Representation, ClipboardError> {
    let preference = [PNG_MIME, PLAIN_TEXT_MIME];
    let representation = preference
        .iter()
        .find_map(|mime| items.iter().find_map(|item| item.representation(mime)))
        .ok_or_else(|| ClipboardError::Write("no supported representation".into()))?
        .clone();

    let mut child = std::process::Command::new("wl-copy")
        .args(["--type", &representation.mime, "--foreground"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|err| ClipboardError::Write(format!("wl-copy unavailable: {err}")))?;

    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| ClipboardError::Write("wl-copy stdin unavailable".into()))?;
    let bytes = representation.bytes.clone();
    std::thread::spawn(move || {
        use std::io::Write as _;
        let _ = stdin.write_all(&bytes);
        drop(stdin);
        let _ = child.wait();
    });
    Ok(representation)
}
