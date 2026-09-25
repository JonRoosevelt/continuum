use std::collections::HashMap;
use std::net::TcpListener;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::{self, Thread};
use std::time::Duration;

use continuum_core::{ClipboardItem, DeviceId};
use continuum_net::{Identity, Message, Peer};

use crate::config::Config;

static CONNECTION_SEQ: AtomicU64 = AtomicU64::new(0);
const RECONNECT_MIN: Duration = Duration::from_millis(500);
const RECONNECT_MAX: Duration = Duration::from_secs(3);
// Keep below the transport frame limit so an oversized payload is skipped, not fatal.
const MAX_PAYLOAD: usize = 128 * 1024 * 1024;

pub(crate) enum NetEvent {
    Clipboard(ClipboardItem),
    PeerUp(DeviceId),
    PeerDown(DeviceId),
}

pub(crate) struct Slot {
    sender: mpsc::Sender<Vec<u8>>,
    conn_id: u64,
    dialer: DeviceId,
}

pub(crate) type Registry = Arc<Mutex<HashMap<DeviceId, Slot>>>;

/// Wakes parked reconnect loops so clipboard activity can bring links up immediately.
pub(crate) struct Waker {
    threads: Arc<Mutex<Vec<Thread>>>,
}

impl Waker {
    pub(crate) fn wake(&self) {
        for thread in self.threads.lock().expect("waker mutex").iter() {
            thread.unpark();
        }
    }
}

#[allow(dead_code)]
#[must_use]
pub(crate) fn peer_count(registry: &Registry) -> usize {
    registry.lock().expect("registry mutex").len()
}

pub(crate) fn broadcast(registry: &Registry, item: &ClipboardItem) {
    let Ok(bytes) = Message::Clipboard(Box::new(item.clone())).encode() else {
        tracing::warn!("failed to encode clipboard message");
        return;
    };
    if bytes.len() > MAX_PAYLOAD {
        tracing::warn!(
            bytes = bytes.len(),
            "clipboard payload too large to send; skipping"
        );
        return;
    }
    let peers = registry.lock().expect("registry mutex");
    for slot in peers.values() {
        let _ = slot.sender.send(bytes.clone());
    }
}

pub(crate) fn send_to(registry: &Registry, device: DeviceId, item: &ClipboardItem) {
    let Ok(bytes) = Message::Clipboard(Box::new(item.clone())).encode() else {
        return;
    };
    if bytes.len() > MAX_PAYLOAD {
        tracing::warn!(
            bytes = bytes.len(),
            "clipboard payload too large to send; skipping"
        );
        return;
    }
    if let Some(slot) = registry.lock().expect("registry mutex").get(&device) {
        let _ = slot.sender.send(bytes);
    }
}

pub(crate) fn start<E>(
    identity: Arc<Identity>,
    config: &Config,
    on_event: E,
) -> anyhow::Result<(Registry, Waker)>
where
    E: Fn(NetEvent) + Send + Sync + 'static,
{
    let registry: Registry = Arc::new(Mutex::new(HashMap::new()));
    let on_event: Arc<dyn Fn(NetEvent) + Send + Sync> = Arc::new(on_event);
    let local_id = identity.device_id();
    let threads: Arc<Mutex<Vec<Thread>>> = Arc::new(Mutex::new(Vec::new()));
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
        let on_event = Arc::clone(&on_event);
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let registry = Arc::clone(&registry);
                let identity = Arc::clone(&identity);
                let allowed = Arc::clone(&allowed);
                let on_event = Arc::clone(&on_event);
                thread::spawn(move || {
                    let allow = |key: &[u8]| allowed.iter().any(|known| known.as_slice() == key);
                    match Peer::accept(stream, &identity, allow) {
                        Ok(peer) => register(peer, false, local_id, registry, on_event),
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
        let on_event = Arc::clone(&on_event);
        let threads = Arc::clone(&threads);
        thread::spawn(move || {
            threads.lock().expect("waker mutex").push(thread::current());
            let mut backoff = RECONNECT_MIN;
            loop {
                match Peer::connect(&address, &identity, &key) {
                    Ok(peer) => {
                        backoff = RECONNECT_MIN;
                        register(
                            peer,
                            true,
                            local_id,
                            Arc::clone(&registry),
                            Arc::clone(&on_event),
                        );
                    }
                    Err(err) => tracing::debug!(%name, %err, "connect failed"),
                }
                thread::park_timeout(backoff);
                backoff = (backoff * 2).min(RECONNECT_MAX);
            }
        });
    }

    Ok((registry, Waker { threads }))
}

/// Registers a connection, resolving duplicates so both peers converge on the same one.
///
/// Both sides dial, so a pair can briefly have two connections. The winner is the one dialed
/// by the peer with the smaller `DeviceId`; if that connection is absent, the other is used so
/// a single available link is never discarded.
fn register(
    mut peer: Peer,
    outbound: bool,
    local_id: DeviceId,
    registry: Registry,
    on_event: Arc<dyn Fn(NetEvent) + Send + Sync>,
) {
    let remote = peer.device_id();
    let dialer = if outbound { local_id } else { remote };
    let preferred = dialer == local_id.min(remote);
    let conn_id = CONNECTION_SEQ.fetch_add(1, Ordering::Relaxed);
    let (sender, receiver) = mpsc::channel::<Vec<u8>>();

    {
        let mut peers = registry.lock().expect("registry mutex");
        if let Some(existing) = peers.get(&remote) {
            let existing_preferred = existing.dialer == local_id.min(remote);
            if !(preferred && !existing_preferred) {
                tracing::debug!(device = %remote.short(), "dropping duplicate connection");
                return;
            }
        }
        peers.insert(
            remote,
            Slot {
                sender,
                conn_id,
                dialer,
            },
        );
    }

    tracing::info!(device = %remote.short(), outbound, "peer registered");
    on_event(NetEvent::PeerUp(remote));

    let result = peer.run(receiver, |message| {
        if let Message::Clipboard(item) = message {
            on_event(NetEvent::Clipboard(*item));
        }
    });

    let removed = {
        let mut peers = registry.lock().expect("registry mutex");
        if peers.get(&remote).map(|slot| slot.conn_id) == Some(conn_id) {
            peers.remove(&remote);
            true
        } else {
            false
        }
    };

    match &result {
        Ok(()) => tracing::info!(device = %remote.short(), "peer disconnected"),
        Err(err) => tracing::info!(device = %remote.short(), %err, "peer disconnected"),
    }
    if removed {
        on_event(NetEvent::PeerDown(remote));
    }
}
