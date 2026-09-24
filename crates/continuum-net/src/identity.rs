use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use continuum_core::DeviceId;

use crate::error::NetError;

pub const NOISE_PARAMS: &str = "Noise_IK_25519_ChaChaPoly_BLAKE2s";

const KEY_LEN: usize = 32;
const FILE_LEN: usize = KEY_LEN * 2;

pub struct Identity {
    private: Vec<u8>,
    public: Vec<u8>,
}

impl Identity {
    pub fn load_or_generate(path: &Path) -> Result<Self, NetError> {
        if path.exists() {
            Self::load(path)
        } else {
            Self::generate_at(path)
        }
    }

    fn load(path: &Path) -> Result<Self, NetError> {
        let bytes = fs::read(path)?;
        if bytes.len() != FILE_LEN {
            return Err(NetError::Identity(format!(
                "expected {FILE_LEN} bytes in {}, found {}",
                path.display(),
                bytes.len()
            )));
        }
        Ok(Self {
            private: bytes[..KEY_LEN].to_vec(),
            public: bytes[KEY_LEN..].to_vec(),
        })
    }

    fn generate_at(path: &Path) -> Result<Self, NetError> {
        let identity = Self::generate()?;

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            file.set_permissions(fs::Permissions::from_mode(0o600))?;
        }
        let mut contents = Vec::with_capacity(FILE_LEN);
        contents.extend_from_slice(&identity.private);
        contents.extend_from_slice(&identity.public);
        file.write_all(&contents)?;
        file.sync_all()?;

        Ok(identity)
    }

    pub fn generate() -> Result<Self, NetError> {
        let params = NOISE_PARAMS.parse()?;
        let keypair = snow::Builder::new(params).generate_keypair()?;
        Ok(Self {
            private: keypair.private,
            public: keypair.public,
        })
    }

    #[must_use]
    pub fn device_id(&self) -> DeviceId {
        device_id_from_public(&self.public)
    }

    #[must_use]
    pub fn public_key(&self) -> &[u8] {
        &self.public
    }

    #[must_use]
    pub fn private_key(&self) -> &[u8] {
        &self.private
    }
}

#[must_use]
pub fn default_identity_path() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("continuum").join("identity.key"))
}

#[must_use]
pub fn device_id_from_public(public_key: &[u8]) -> DeviceId {
    DeviceId::from_bytes(*blake3::hash(public_key).as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_persists_and_reloads() {
        let dir = std::env::temp_dir().join(format!("continuum-test-{}", std::process::id()));
        let path = dir.join("identity.key");
        let _ = fs::remove_file(&path);

        let first = Identity::load_or_generate(&path).unwrap();
        let second = Identity::load_or_generate(&path).unwrap();

        assert_eq!(first.public_key(), second.public_key());
        assert_eq!(first.device_id(), second.device_id());

        let _ = fs::remove_dir_all(&dir);
    }
}
