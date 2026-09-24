use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerConfig {
    pub name: String,
    pub address: String,
    pub public_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub listen: String,
    #[serde(default)]
    pub peers: Vec<PeerConfig>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            listen: "0.0.0.0:8770".to_string(),
            peers: Vec::new(),
        }
    }
}

impl Config {
    pub fn load_or_create(path: &Path) -> anyhow::Result<Self> {
        if path.exists() {
            Ok(serde_json::from_str(&fs::read_to_string(path)?)?)
        } else {
            let config = Self::default();
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(path, serde_json::to_string_pretty(&config)?)?;
            Ok(config)
        }
    }
}

#[must_use]
pub fn default_config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("continuum").join("config.json"))
}
