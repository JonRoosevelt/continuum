use std::collections::HashMap;
use std::net::TcpListener;
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::Duration;

use continuum_core::{ClipboardItem, DeviceId};
use continuum_net::{Identity, Message, Peer};

use crate::config::Config;

pub type Registry = Arc<Mutex<HashMap<DeviceId, mpsc::Sender<Vec<u8>>>>>;

pub fn broadcast(registry: &Registry, item: &ClipboardItem) {
    let Ok(bytes) = Message::Clipboard(Box::new(item.clone())).encode() else {
        tracing::warn!("failed to encode clipboard message");
        return;
    };
    let peers = registry.lock().expect("registry mutex");
    for sender in peers.values() {
        let _ = sender.send(bytes.clone());
    }
}

pub fn start<F>(identity: Arc<Identity>, config: &Config, on_remote: F) -> anyhow::Result<Registry>
where
    F: Fn(ClipboardItem) + Send + Sync + 'static,
{
    let registry: Registry = Arc::new(Mutex::new(HashMap::new()));
    let on_remote: Arc<dyn Fn(ClipboardItem) + Send + Sync> = Arc::new(on_remote);
    let allowed: Arc<Vec<Vec<u8>>> = Arc::new(
        config
            .peers
            .iter()
            .filter_map(|peer| hex::decode(&peer.public_key).ok())
            .collect(),
    );

    let listener = TcpListener::bind(&config.listen)?;
    {
        let registry = Arc::clone(&registry);
        let identity = Arc::clone(&identity);
        let allowed = Arc::clone(&allowed);
        let on_remote = Arc::clone(&on_remote);
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let registry = Arc::clone(&registry);
                let identity = Arc::clone(&identity);
                let allowed = Arc::clone(&allowed);
                let on_remote = Arc::clone(&on_remote);
                thread::spawn(move || {
                    let allow = |key: &[u8]| allowed.iter().any(|known| known.as_slice() == key);
                    match Peer::accept(stream, &identity, allow) {
                        Ok(peer) => run_peer(peer, registry, on_remote),
                        Err(err) => tracing::warn!(%err, "rejected inbound peer"),
                    }
                });
            }
        });
    }

    for peer in &config.peers {
        let Ok(key) = hex::decode(&peer.public_key) else {
            tracing::warn!(name = %peer.name, "invalid public key in config; skipping peer");
            continue;
        };
        let address = peer.address.clone();
        let name = peer.name.clone();
        let registry = Arc::clone(&registry);
        let identity = Arc::clone(&identity);
        let on_remote = Arc::clone(&on_remote);
        thread::spawn(move || {
            let mut backoff = Duration::from_millis(500);
            loop {
                match Peer::connect(&address, &identity, &key) {
                    Ok(peer) => {
                        tracing::info!(%name, "connected to peer");
                        backoff = Duration::from_millis(500);
                        run_peer(peer, Arc::clone(&registry), Arc::clone(&on_remote));
                        tracing::info!(%name, "peer disconnected");
                    }
                    Err(err) => tracing::debug!(%name, %err, "connect failed"),
                }
                thread::sleep(backoff);
                backoff = (backoff * 2).min(Duration::from_secs(30));
            }
        });
    }

    Ok(registry)
}

fn run_peer(
    mut peer: Peer,
    registry: Registry,
    on_remote: Arc<dyn Fn(ClipboardItem) + Send + Sync>,
) {
    let device_id = peer.device_id();
    let (sender, receiver) = mpsc::channel::<Vec<u8>>();
    registry
        .lock()
        .expect("registry mutex")
        .insert(device_id, sender);

    let result = peer.run(receiver, |message| {
        if let Message::Clipboard(item) = message {
            on_remote(*item);
        }
    });

    registry.lock().expect("registry mutex").remove(&device_id);
    if let Err(err) = result {
        tracing::debug!(device = %device_id.short(), %err, "peer loop ended");
    }
}
