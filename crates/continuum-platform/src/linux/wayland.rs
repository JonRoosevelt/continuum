use std::io::Read as _;

use continuum_core::{
    content_hash, ClipItem, ContentHash, Representation, HTML_MIME, PLAIN_TEXT_MIME, PNG_MIME,
    RTF_MIME,
};
use wl_clipboard_rs::copy::{
    self, ClipboardType as CopyClipboardType, MimeSource, Options, Seat as CopySeat, Source,
};
use wl_clipboard_rs::paste::{
    self, get_contents, get_mime_types, ClipboardType as PasteClipboardType,
    MimeType as PasteMimeType, Seat as PasteSeat,
};

use crate::backend::{ClipboardBackend, ClipboardError};
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

        // Plain text is normalized to PLAIN_TEXT_MIME regardless of the concrete text MIME
        // wl-clipboard-rs selected, so read hashing matches what write() offers.
        let mut representations = Vec::new();
        if let Some(bytes) = self.fetch(PasteMimeType::Text)? {
            representations.push(Representation::new(PLAIN_TEXT_MIME, bytes));
        }
        if let Some(bytes) = self.fetch(PasteMimeType::Specific(HTML_MIME))? {
            representations.push(Representation::new(HTML_MIME, bytes));
        }
        if let Some(bytes) = self.fetch(PasteMimeType::Specific(RTF_MIME))? {
            representations.push(Representation::new(RTF_MIME, bytes));
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

        let mut sources = Vec::new();
        for item in items {
            for representation in &item.representations {
                let mime_type = match representation.mime.as_str() {
                    PLAIN_TEXT_MIME => copy::MimeType::Text,
                    HTML_MIME | RTF_MIME | PNG_MIME => {
                        copy::MimeType::Specific(representation.mime.clone())
                    }
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
        // foreground(false) spawns the serving thread and returns immediately.
        options
            .clipboard(CopyClipboardType::Regular)
            .seat(CopySeat::All)
            .foreground(false);
        copy::copy_multi(options, sources).map_err(|err| ClipboardError::Write(err.to_string()))?;

        self.last_hash = Some(content_hash(items));
        Ok(())
    }
}
