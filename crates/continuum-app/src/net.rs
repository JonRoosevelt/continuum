use std::collections::HashMap;
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::{self, Thread};
use std::time::Duration;

use continuum_core::{ClipboardItem, DeviceId};
use continuum_net::{device_id_from_public, Identity, Message, Peer};

use crate::config::{Config, PeerConfig};
use crate::discovery::AddressMap;

static CONNECTION_SEQ: AtomicU64 = AtomicU64::new(0);
const RECONNECT_MIN: Duration = Duration::from_millis(500);
const RECONNECT_MAX: Duration = Duration::from_secs(3);
// A peer with no resolvable address waits on the discovery waker rather than hot-looping.
const DISCOVERY_WAIT: Duration = Duration::from_secs(10);
// Keep below the transport frame limit so an oversized payload is skipped, not fatal.
const MAX_PAYLOAD: usize = 128 * 1024 * 1024;

pub(crate) enum NetEvent {
    Clipboard(ClipboardItem),
    TransferStart {
        from: DeviceId,
        name: String,
        size: u64,
    },
    TransferDone {
        from: DeviceId,
        name: String,
    },
    PeerUp(DeviceId),
    PeerDown(DeviceId),
}

pub(crate) struct Slot {
    sender: mpsc::Sender<Vec<u8>>,
    conn_id: u64,
    dialer: DeviceId,
}

pub(crate) type Registry = Arc<Mutex<HashMap<DeviceId, Slot>>>;

/// Wakes parked reconnect loops so discovery changes and clipboard activity can bring
/// links up immediately.
#[derive(Clone)]
pub(crate) struct Waker {
    threads: Arc<Mutex<Vec<Thread>>>,
}

impl Default for Waker {
    fn default() -> Self {
        Self::new()
    }
}

impl Waker {
    pub(crate) fn new() -> Self {
        Self {
            threads: Arc::new(Mutex::new(Vec::new())),
        }
    }

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

#[allow(dead_code)]
#[must_use]
pub(crate) fn online_devices(registry: &Registry) -> Vec<DeviceId> {
    registry
        .lock()
        .expect("registry mutex")
        .keys()
        .copied()
        .collect()
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

pub(crate) fn broadcast_message(registry: &Registry, message: &Message) {
    let Ok(bytes) = message.encode() else {
        return;
    };
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

/// Runtime control over the live peer set, so pairing and unpairing take effect without
/// restarting the process.
#[derive(Clone)]
pub(crate) struct NetHandle {
    identity: Arc<Identity>,
    addresses: AddressMap,
    waker: Waker,
    allowed: Arc<Mutex<Vec<Vec<u8>>>>,
    registry: Registry,
    on_event: Arc<dyn Fn(NetEvent) + Send + Sync>,
    dialers: Arc<Mutex<HashMap<DeviceId, Dialer>>>,
    local_id: DeviceId,
}

struct Dialer {
    stop: Arc<AtomicBool>,
    thread: Thread,
}

impl NetHandle {
    /// Pins a peer's key for inbound connections and starts its dialer.
    pub(crate) fn add_peer(&self, peer: &PeerConfig) {
        let Ok(key) = hex::decode(&peer.public_key) else {
            tracing::warn!(name = %peer.name, "invalid public key; not adding peer");
            return;
        };
        let remote_id = device_id_from_public(&key);
        {
            let mut allowed = self.allowed.lock().expect("allowed mutex");
            if !allowed
                .iter()
                .any(|known| known.as_slice() == key.as_slice())
            {
                allowed.push(key.clone());
            }
            let mut dialers = self.dialers.lock().expect("dialers mutex");
            if dialers.contains_key(&remote_id) {
                return;
            }
            let stop = Arc::new(AtomicBool::new(false));
            let thread = spawn_dialer(
                Arc::clone(&self.identity),
                key,
                remote_id,
                peer.name.clone(),
                peer.address.clone(),
                Arc::clone(&self.registry),
                Arc::clone(&self.on_event),
                Arc::clone(&self.addresses),
                self.waker.clone(),
                self.local_id,
                Arc::clone(&stop),
            );
            dialers.insert(remote_id, Dialer { stop, thread });
        }
        tracing::info!(device = %remote_id.short(), "peer added at runtime");
        self.waker.wake();
    }

    /// Stops a peer's dialer, drops any live connection, and unpins its key so further
    /// inbound connections are rejected.
    pub(crate) fn remove_peer(&self, remote_id: DeviceId) {
        self.allowed
            .lock()
            .expect("allowed mutex")
            .retain(|key| device_id_from_public(key) != remote_id);
        if let Some(dialer) = self
            .dialers
            .lock()
            .expect("dialers mutex")
            .remove(&remote_id)
        {
            dialer.stop.store(true, Ordering::Relaxed);
            dialer.thread.unpark();
        }
        let dropped = self
            .registry
            .lock()
            .expect("registry mutex")
            .remove(&remote_id)
            .is_some();
        if dropped {
            (self.on_event)(NetEvent::PeerDown(remote_id));
        }
        tracing::info!(device = %remote_id.short(), "peer removed at runtime");
    }
}

pub(crate) fn start<E>(
    identity: Arc<Identity>,
    config: &Config,
    addresses: AddressMap,
    waker: Waker,
    on_event: E,
) -> anyhow::Result<(Registry, Waker, NetHandle)>
where
    E: Fn(NetEvent) + Send + Sync + 'static,
{
    let registry: Registry = Arc::new(Mutex::new(HashMap::new()));
    let on_event: Arc<dyn Fn(NetEvent) + Send + Sync> = Arc::new(on_event);
    let local_id = identity.device_id();
    let allowed: Arc<Mutex<Vec<Vec<u8>>>> = Arc::new(Mutex::new(Vec::new()));

    let handle = NetHandle {
        identity: Arc::clone(&identity),
        addresses: Arc::clone(&addresses),
        waker: waker.clone(),
        allowed: Arc::clone(&allowed),
        registry: Arc::clone(&registry),
        on_event: Arc::clone(&on_event),
        dialers: Arc::new(Mutex::new(HashMap::new())),
        local_id,
    };

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
                    let allow = |key: &[u8]| {
                        allowed
                            .lock()
                            .expect("allowed mutex")
                            .iter()
                            .any(|known| known.as_slice() == key)
                    };
                    match Peer::accept(stream, &identity, allow) {
                        Ok(peer) => register(peer, false, local_id, registry, on_event),
                        Err(err) => tracing::warn!(%err, "rejected inbound peer"),
                    }
                });
            }
        });
    }

    for peer in &config.peers {
        handle.add_peer(peer);
    }

    Ok((registry, waker, handle))
}

#[allow(clippy::too_many_arguments)]
fn spawn_dialer(
    identity: Arc<Identity>,
    key: Vec<u8>,
    remote_id: DeviceId,
    name: String,
    static_address: Option<String>,
    registry: Registry,
    on_event: Arc<dyn Fn(NetEvent) + Send + Sync>,
    addresses: AddressMap,
    waker: Waker,
    local_id: DeviceId,
    stop: Arc<AtomicBool>,
) -> Thread {
    let spawned = thread::spawn(move || {
        let current = thread::current();
        let current_id = current.id();
        waker.threads.lock().expect("waker mutex").push(current);
        let mut backoff = RECONNECT_MIN;
        while !stop.load(Ordering::Relaxed) {
            let target = {
                let discovered = addresses
                    .lock()
                    .expect("discovery map")
                    .get(&remote_id)
                    .copied();
                discovered
                    .map(|addr| addr.to_string())
                    .or_else(|| static_address.clone())
            };
            let Some(target) = target else {
                tracing::debug!(%name, "no address for peer; waiting for discovery");
                thread::park_timeout(DISCOVERY_WAIT);
                continue;
            };
            match Peer::connect(target.as_str(), &identity, &key) {
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
                Err(err) => tracing::debug!(%name, %target, %err, "connect failed"),
            }
            if stop.load(Ordering::Relaxed) {
                break;
            }
            thread::park_timeout(backoff);
            backoff = (backoff * 2).min(RECONNECT_MAX);
        }
        waker
            .threads
            .lock()
            .expect("waker mutex")
            .retain(|thread| thread.id() != current_id);
    });
    spawned.thread().clone()
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

    let result = peer.run(receiver, |message| match message {
        Message::Clipboard(item) => on_event(NetEvent::Clipboard(*item)),
        Message::TransferStart { from, name, size } => {
            on_event(NetEvent::TransferStart { from, name, size });
        }
        Message::TransferDone { from, name } => {
            on_event(NetEvent::TransferDone { from, name });
        }
        Message::Ping | Message::Pong => {}
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
