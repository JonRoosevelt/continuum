use serde::{Deserialize, Serialize};

use crate::error::CoreError;
use crate::hash::ContentHash;
use crate::id::DeviceId;
use crate::lamport::Version;

pub const PLAIN_TEXT_MIME: &str = "text/plain;charset=utf-8";
pub const HTML_MIME: &str = "text/html";
pub const RTF_MIME: &str = "text/rtf";
pub const PNG_MIME: &str = "image/png";
pub const TIFF_MIME: &str = "image/tiff";

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Representation {
    pub mime: String,
    pub bytes: Vec<u8>,
}

impl Representation {
    #[must_use]
    pub fn new(mime: impl Into<String>, bytes: impl Into<Vec<u8>>) -> Self {
        Self {
            mime: mime.into(),
            bytes: bytes.into(),
        }
    }

    #[must_use]
    pub fn text(mime: impl Into<String>, text: &str) -> Self {
        Self::new(mime, text.as_bytes().to_vec())
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipItem {
    pub representations: Vec<Representation>,
}

impl ClipItem {
    pub fn new(mut representations: Vec<Representation>) -> Result<Self, CoreError> {
        if representations.is_empty() {
            return Err(CoreError::EmptyItem);
        }
        representations.sort_by(|a, b| a.mime.cmp(&b.mime));
        representations.dedup_by(|a, b| a.mime == b.mime);
        Ok(Self { representations })
    }

    #[must_use]
    pub fn representation(&self, mime: &str) -> Option<&Representation> {
        self.representations.iter().find(|r| r.mime == mime)
    }

    #[must_use]
    pub fn plain_text(&self) -> Option<&str> {
        let rep = self.representation(PLAIN_TEXT_MIME)?;
        std::str::from_utf8(&rep.bytes).ok()
    }

    #[must_use]
    pub fn content_hash(&self) -> ContentHash {
        hash_items(std::slice::from_ref(self))
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipboardItem {
    pub version: Version,
    pub origin: DeviceId,
    pub items: Vec<ClipItem>,
    pub hash: ContentHash,
}

impl ClipboardItem {
    pub fn new(
        origin: DeviceId,
        version: Version,
        items: Vec<ClipItem>,
    ) -> Result<Self, CoreError> {
        if items.is_empty() {
            return Err(CoreError::EmptyPayload);
        }
        let hash = hash_items(&items);
        Ok(Self {
            version,
            origin,
            items,
            hash,
        })
    }

    #[must_use]
    pub fn plain_text(&self) -> Option<&str> {
        self.items.first().and_then(ClipItem::plain_text)
    }
}

#[must_use]
pub fn content_hash(items: &[ClipItem]) -> ContentHash {
    hash_items(items)
}

fn hash_items(items: &[ClipItem]) -> ContentHash {
    let mut hasher = blake3::Hasher::new();
    hasher.update(&(items.len() as u64).to_le_bytes());
    for item in items {
        hasher.update(&(item.representations.len() as u64).to_le_bytes());
        for rep in &item.representations {
            hasher.update(&(rep.mime.len() as u64).to_le_bytes());
            hasher.update(rep.mime.as_bytes());
            hasher.update(&(rep.bytes.len() as u64).to_le_bytes());
            hasher.update(&rep.bytes);
        }
    }
    ContentHash::from_bytes(*hasher.finalize().as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device() -> DeviceId {
        DeviceId::from_bytes([9; 32])
    }

    #[test]
    fn rejects_empty_item() {
        assert!(matches!(ClipItem::new(vec![]), Err(CoreError::EmptyItem)));
    }

    #[test]
    fn representations_are_sorted_and_deduped() {
        let item = ClipItem::new(vec![
            Representation::text(HTML_MIME, "<b>hi</b>"),
            Representation::text(PLAIN_TEXT_MIME, "hi"),
            Representation::text(HTML_MIME, "<i>ignored</i>"),
        ])
        .unwrap();
        assert_eq!(item.representations.len(), 2);
        assert_eq!(item.representations[0].mime, HTML_MIME);
        assert_eq!(item.representations[1].mime, PLAIN_TEXT_MIME);
    }

    #[test]
    fn hash_is_order_independent() {
        let a = ClipItem::new(vec![
            Representation::text(PLAIN_TEXT_MIME, "hi"),
            Representation::text(HTML_MIME, "<b>hi</b>"),
        ])
        .unwrap();
        let b = ClipItem::new(vec![
            Representation::text(HTML_MIME, "<b>hi</b>"),
            Representation::text(PLAIN_TEXT_MIME, "hi"),
        ])
        .unwrap();
        assert_eq!(a.content_hash(), b.content_hash());
    }

    #[test]
    fn hash_changes_with_content() {
        let a = ClipItem::new(vec![Representation::text(PLAIN_TEXT_MIME, "hi")]).unwrap();
        let b = ClipItem::new(vec![Representation::text(PLAIN_TEXT_MIME, "ho")]).unwrap();
        assert_ne!(a.content_hash(), b.content_hash());
    }

    #[test]
    fn payload_hash_matches_item_hash() {
        let item = ClipItem::new(vec![Representation::text(PLAIN_TEXT_MIME, "hi")]).unwrap();
        let payload =
            ClipboardItem::new(device(), Version::new(1, device()), vec![item.clone()]).unwrap();
        assert_eq!(payload.hash, item.content_hash());
        assert_eq!(payload.plain_text(), Some("hi"));
    }
}
