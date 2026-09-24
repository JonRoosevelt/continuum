use std::io::{self, BufRead as _, Write as _};
use std::net::{TcpListener, TcpStream};

use continuum_net::device_id_from_public;
use continuum_net::noise::{short_authentication_string, xx_initiate, xx_respond};
use serde::{Deserialize, Serialize};

use crate::config::{Config, PeerConfig};
use crate::load_identity;

const PAIRING_PORT: u16 = 8771;
const SYNC_PORT: u16 = 8770;

#[derive(Serialize, Deserialize)]
struct PairingInfo {
    name: String,
    sync_port: u16,
}

pub fn pair(host: &str) -> anyhow::Result<()> {
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

    let code = short_authentication_string(identity.public_key(), &remote_key);
    println!(
        "Pairing with \"{}\" at {host}",
        peer_label(&theirs.name, &remote_key)
    );
    println!("Verification code: {code}");

    if !confirm("Codes match on both devices?")? {
        session.send(&mut stream, b"abort")?;
        anyhow::bail!("pairing aborted");
    }
    session.send(&mut stream, b"confirm")?;
    if session.recv(&mut stream)? != b"ok" {
        anyhow::bail!("the other device did not confirm");
    }

    let address = format!("{host}:{}", theirs.sync_port);
    add_peer(
        &peer_label(&theirs.name, &remote_key),
        &address,
        &remote_key,
    )?;
    println!("Paired: added {address}. Restart Continuum to connect.");
    Ok(())
}

pub fn pair_listen() -> anyhow::Result<()> {
    let identity = load_identity()?;
    let listener = TcpListener::bind(("0.0.0.0", PAIRING_PORT))?;
    println!("Waiting for a pairing request on port {PAIRING_PORT}…");

    let (mut stream, remote) = listener.accept()?;
    stream.set_nodelay(true)?;
    let mut session = xx_respond(&mut stream, &identity)?;

    let remote_key = session
        .remote_static()
        .ok_or_else(|| anyhow::anyhow!("peer did not present a static key"))?
        .to_vec();

    let theirs: PairingInfo = serde_json::from_slice(&session.recv(&mut stream)?)?;
    let info = PairingInfo {
        name: local_name(&identity),
        sync_port: SYNC_PORT,
    };
    session.send(&mut stream, &serde_json::to_vec(&info)?)?;

    let code = short_authentication_string(identity.public_key(), &remote_key);
    println!(
        "Pairing request from \"{}\" ({})",
        peer_label(&theirs.name, &remote_key),
        remote.ip()
    );
    println!("Verification code: {code}");

    if session.recv(&mut stream)? == b"abort" {
        println!("The other device aborted.");
        return Ok(());
    }

    if confirm("Codes match on both devices?")? {
        let address = format!("{}:{}", remote.ip(), theirs.sync_port);
        add_peer(
            &peer_label(&theirs.name, &remote_key),
            &address,
            &remote_key,
        )?;
        session.send(&mut stream, b"ok")?;
        println!("Paired: added {address}. Restart Continuum to connect.");
    } else {
        session.send(&mut stream, b"denied")?;
        println!("Pairing declined.");
    }
    Ok(())
}

fn local_name(identity: &continuum_net::Identity) -> String {
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

fn add_peer(name: &str, address: &str, public_key: &[u8]) -> anyhow::Result<()> {
    let path = crate::config::default_config_path()
        .ok_or_else(|| anyhow::anyhow!("could not resolve the config directory"))?;
    let mut config = Config::load_or_create(&path)?;
    config.upsert_peer(PeerConfig {
        name: name.to_string(),
        address: address.to_string(),
        public_key: hex::encode(public_key),
    });
    config.save(&path)?;
    Ok(())
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
