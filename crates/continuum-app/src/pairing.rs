use std::io::{self, BufRead as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use continuum_net::device_id_from_public;
use continuum_net::noise::{short_authentication_string, xx_initiate, xx_respond, Session};
use continuum_net::Identity;
use serde::{Deserialize, Serialize};

use crate::config::{Config, PeerConfig};
use crate::load_identity;

pub const PAIRING_PORT: u16 = 8771;
const SYNC_PORT: u16 = 8770;
const ACCEPT_POLL: Duration = Duration::from_millis(50);

#[derive(Serialize, Deserialize)]
struct PairingInfo {
    name: String,
    sync_port: u16,
}

/// Initiator side of a pairing handshake, returned once the peers have agreed on a
/// verification code. The caller confirms or aborts; nothing is written to disk here.
pub struct StartedPair {
    pub code: String,
    pub peer_name: String,
    pub peer_key: Vec<u8>,
    pub peer_address: String,
    stream: TcpStream,
    session: Session,
}

impl StartedPair {
    pub fn confirm(mut self) -> anyhow::Result<PeerConfig> {
        self.session.send(&mut self.stream, b"confirm")?;
        if self.session.recv(&mut self.stream)? != b"ok" {
            anyhow::bail!("the other device did not confirm");
        }
        Ok(add_peer(
            &self.peer_name,
            &self.peer_address,
            &self.peer_key,
        ))
    }

    pub fn abort(mut self) {
        let _ = self.session.send(&mut self.stream, b"abort");
    }
}

/// Responder side of a pairing handshake. The initiator's decision arrives over the
/// same connection, so `accept` waits for it before acknowledging.
pub struct StartedPairListener {
    pub code: String,
    pub peer_name: String,
    pub peer_key: Vec<u8>,
    pub peer_address: String,
    pub remote_ip: String,
    stream: TcpStream,
    session: Session,
}

impl StartedPairListener {
    // Only the macOS pairing window calls this; it is unused without the tray feature.
    #[allow(dead_code)]
    pub fn accept(mut self) -> anyhow::Result<PeerConfig> {
        if !self.await_peer_decision()? {
            anyhow::bail!("the other device aborted");
        }
        self.approve()
    }

    pub fn await_peer_decision(&mut self) -> anyhow::Result<bool> {
        Ok(self.session.recv(&mut self.stream)? == b"confirm")
    }

    fn config(&self) -> PeerConfig {
        add_peer(&self.peer_name, &self.peer_address, &self.peer_key)
    }

    pub fn approve(mut self) -> anyhow::Result<PeerConfig> {
        self.session.send(&mut self.stream, b"ok")?;
        Ok(self.config())
    }

    pub fn deny(mut self) {
        let _ = self.session.send(&mut self.stream, b"denied");
    }
}

pub fn start_pair(host: &str) -> anyhow::Result<StartedPair> {
    let identity = load_identity()?;
    let mut stream = TcpStream::connect((host, PAIRING_PORT))?;
    stream.set_nodelay(true)?;
    let mut session = xx_initiate(&mut stream, &identity)?;

    let remote_key = session
        .remote_static()
        .ok_or_else(|| anyhow::anyhow!("peer did not present a static key"))?
        .to_vec();

    let info = PairingInfo {
        name: local_name(&identity),
        sync_port: SYNC_PORT,
    };
    session.send(&mut stream, &serde_json::to_vec(&info)?)?;
    let theirs: PairingInfo = serde_json::from_slice(&session.recv(&mut stream)?)?;

    Ok(StartedPair {
        code: short_authentication_string(identity.public_key(), &remote_key),
        peer_name: peer_label(&theirs.name, &remote_key),
        peer_key: remote_key,
        peer_address: format!("{host}:{}", theirs.sync_port),
        stream,
        session,
    })
}

pub struct PairListener {
    listener: TcpListener,
    identity: Identity,
}

pub fn bind_pair_listener() -> anyhow::Result<PairListener> {
    let identity = load_identity()?;
    let listener = TcpListener::bind(("0.0.0.0", PAIRING_PORT))?;
    Ok(PairListener { listener, identity })
}

impl PairListener {
    pub fn accept(self) -> anyhow::Result<StartedPairListener> {
        self.accept_with_cancel(&AtomicBool::new(false))
    }

    /// Polls the accept loop so a waiting UI can release the port on cancellation.
    pub fn accept_with_cancel(self, cancel: &AtomicBool) -> anyhow::Result<StartedPairListener> {
        self.listener.set_nonblocking(true)?;
        let (mut stream, remote) = loop {
            if cancel.load(Ordering::Relaxed) {
                anyhow::bail!("pairing cancelled");
            }
            match self.listener.accept() {
                Ok(pair) => break pair,
                Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
                    std::thread::sleep(ACCEPT_POLL);
                }
                Err(err) => return Err(err.into()),
            }
        };
        stream.set_nonblocking(false)?;
        stream.set_nodelay(true)?;
        let mut session = xx_respond(&mut stream, &self.identity)?;

        let remote_key = session
            .remote_static()
            .ok_or_else(|| anyhow::anyhow!("peer did not present a static key"))?
            .to_vec();

        let theirs: PairingInfo = serde_json::from_slice(&session.recv(&mut stream)?)?;
        let info = PairingInfo {
            name: local_name(&self.identity),
            sync_port: SYNC_PORT,
        };
        session.send(&mut stream, &serde_json::to_vec(&info)?)?;

        Ok(StartedPairListener {
            code: short_authentication_string(self.identity.public_key(), &remote_key),
            peer_name: peer_label(&theirs.name, &remote_key),
            peer_key: remote_key,
            peer_address: format!("{}:{}", remote.ip(), theirs.sync_port),
            remote_ip: remote.ip().to_string(),
            stream,
            session,
        })
    }
}

#[allow(dead_code)]
pub fn start_pair_listen() -> anyhow::Result<StartedPairListener> {
    bind_pair_listener()?.accept()
}

pub fn pair(host: &str) -> anyhow::Result<()> {
    let started = start_pair(host)?;
    println!("Pairing with \"{}\" at {host}", started.peer_name);
    println!("Verification code: {}", started.code);

    if !confirm("Codes match on both devices?")? {
        started.abort();
        anyhow::bail!("pairing aborted");
    }
    let peer = started.confirm()?;
    save_peer(&peer)?;
    println!(
        "Paired: added {}. Restart Continuum to connect.",
        peer.address
    );
    Ok(())
}

pub fn pair_listen() -> anyhow::Result<()> {
    let listener = bind_pair_listener()?;
    println!("Waiting for a pairing request on port {PAIRING_PORT}…");

    let mut started = listener.accept()?;
    println!(
        "Pairing request from \"{}\" ({})",
        started.peer_name, started.remote_ip
    );
    println!("Verification code: {}", started.code);

    if !started.await_peer_decision()? {
        println!("The other device aborted.");
        return Ok(());
    }

    if confirm("Codes match on both devices?")? {
        let peer = started.config();
        save_peer(&peer)?;
        started.approve()?;
        println!(
            "Paired: added {}. Restart Continuum to connect.",
            peer.address
        );
    } else {
        started.deny();
        println!("Pairing declined.");
    }
    Ok(())
}

pub fn save_peer(peer: &PeerConfig) -> anyhow::Result<()> {
    let path = crate::config::default_config_path()
        .ok_or_else(|| anyhow::anyhow!("could not resolve the config directory"))?;
    let mut config = Config::load_or_create(&path)?;
    config.upsert_peer(peer.clone());
    config.save(&path)?;
    Ok(())
}

fn local_name(identity: &Identity) -> String {
    match crate::config::default_config_path().and_then(|path| Config::load_or_create(&path).ok()) {
        Some(config) if !config.name.is_empty() => config.name,
        _ => identity.device_id().short(),
    }
}

fn peer_label(name: &str, key: &[u8]) -> String {
    if name.is_empty() {
        device_id_from_public(key).short()
    } else {
        name.to_string()
    }
}

fn add_peer(name: &str, address: &str, public_key: &[u8]) -> PeerConfig {
    PeerConfig {
        name: name.to_string(),
        address: address.to_string(),
        public_key: hex::encode(public_key),
    }
}

fn confirm(question: &str) -> anyhow::Result<bool> {
    print!("{question} [y/N] ");
    io::stdout().flush()?;
    let mut line = String::new();
    io::stdin().lock().read_line(&mut line)?;
    Ok(matches!(
        line.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}
