use std::fmt;

use serde::{Deserialize, Serialize};

use crate::error::CoreError;
use crate::hash::hex;

pub const DEVICE_ID_LEN: usize = 32;

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct DeviceId([u8; DEVICE_ID_LEN]);

impl DeviceId {
    pub fn random() -> Result<Self, CoreError> {
        let mut bytes = [0u8; DEVICE_ID_LEN];
        getrandom::getrandom(&mut bytes)?;
        Ok(Self(bytes))
    }

    #[must_use]
    pub const fn from_bytes(bytes: [u8; DEVICE_ID_LEN]) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; DEVICE_ID_LEN] {
        &self.0
    }

    #[must_use]
    pub fn to_hex(&self) -> String {
        hex(&self.0)
    }

    #[must_use]
    pub fn short(&self) -> String {
        hex(&self.0[..4])
    }
}

impl From<[u8; DEVICE_ID_LEN]> for DeviceId {
    fn from(bytes: [u8; DEVICE_ID_LEN]) -> Self {
        Self(bytes)
    }
}

impl fmt::Display for DeviceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl fmt::Debug for DeviceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DeviceId({})", self.short())
    }
}
