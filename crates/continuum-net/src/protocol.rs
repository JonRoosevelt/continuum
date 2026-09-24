use continuum_core::ClipboardItem;
use serde::{Deserialize, Serialize};

use crate::error::NetError;

#[derive(Clone, Serialize, Deserialize)]
pub enum Message {
    Clipboard(Box<ClipboardItem>),
    Ping,
    Pong,
}

impl Message {
    pub fn encode(&self) -> Result<Vec<u8>, NetError> {
        bincode::serialize(self).map_err(|err| NetError::Codec(err.to_string()))
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, NetError> {
        bincode::deserialize(bytes).map_err(|err| NetError::Codec(err.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use continuum_core::{ClipItem, DeviceId, Representation, Version, PLAIN_TEXT_MIME};

    #[test]
    fn clipboard_message_roundtrips() {
        let device = DeviceId::from_bytes([3; 32]);
        let item = ClipItem::new(vec![Representation::text(PLAIN_TEXT_MIME, "hi")]).unwrap();
        let payload = ClipboardItem::new(device, Version::new(1, device), vec![item]).unwrap();

        let encoded = Message::Clipboard(Box::new(payload.clone()))
            .encode()
            .unwrap();
        match Message::decode(&encoded).unwrap() {
            Message::Clipboard(decoded) => {
                assert_eq!(decoded.hash, payload.hash);
                assert_eq!(decoded.plain_text(), Some("hi"));
            }
            _ => panic!("expected a clipboard message"),
        }
    }

    #[test]
    fn multiple_items_roundtrip() {
        let device = DeviceId::from_bytes([4; 32]);
        let items = vec![
            ClipItem::new(vec![Representation::text(PLAIN_TEXT_MIME, "one")]).unwrap(),
            ClipItem::new(vec![Representation::text(PLAIN_TEXT_MIME, "two")]).unwrap(),
        ];
        let payload = ClipboardItem::new(device, Version::new(2, device), items).unwrap();

        let encoded = Message::Clipboard(Box::new(payload)).encode().unwrap();
        match Message::decode(&encoded).unwrap() {
            Message::Clipboard(decoded) => {
                assert_eq!(decoded.items.len(), 2);
                assert_eq!(decoded.plain_text(), Some("one"));
            }
            _ => panic!("expected a clipboard message"),
        }
    }
}
