use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerConfig {
    pub name: String,
    #[serde(default)]
    pub address: Option<String>,
    pub public_key: String,
}

impl PeerConfig {
    #[must_use]
    pub fn display_address(&self) -> &str {
        self.address.as_deref().unwrap_or("(discovered)")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub name: String,
    pub listen: String,
    #[serde(default)]
    pub peers: Vec<PeerConfig>,
    #[serde(default)]
    pub download_dir: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            name: default_name(),
            listen: "0.0.0.0:8770".to_string(),
            peers: Vec::new(),
            download_dir: None,
        }
    }
}

fn default_name() -> String {
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .unwrap_or_default()
}

impl Config {
    pub fn load_or_create(path: &Path) -> anyhow::Result<Self> {
        if path.exists() {
            Ok(serde_json::from_str(&fs::read_to_string(path)?)?)
        } else {
            let config = Self::default();
            config.save(path)?;
            Ok(config)
        }
    }

    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }

    /// Adds a peer, replacing any existing entry with the same name or public key.
    pub fn upsert_peer(&mut self, peer: PeerConfig) {
        if let Some(existing) = self
            .peers
            .iter_mut()
            .find(|known| known.name == peer.name || known.public_key == peer.public_key)
        {
            *existing = peer;
        } else {
            self.peers.push(peer);
        }
    }

    /// Removes the peer matching `query` by name (case-insensitive), full public key,
    /// or short device id, returning it if found.
    pub fn remove_peer(&mut self, query: &str) -> Option<PeerConfig> {
        let index = self.peers.iter().position(|peer| {
            peer.name.eq_ignore_ascii_case(query)
                || peer.public_key == query
                || peer_short(&peer.public_key).is_some_and(|short| short == query)
        })?;
        Some(self.peers.remove(index))
    }
}

fn peer_short(public_key: &str) -> Option<String> {
    let key = hex::decode(public_key).ok()?;
    Some(continuum_net::device_id_from_public(&key).short())
}

#[must_use]
pub fn default_config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("continuum").join("config.json"))
}
