use std::collections::HashMap;
use std::net::{SocketAddr, SocketAddrV4};
use std::sync::{Arc, Mutex};
use std::thread;

use continuum_core::DeviceId;
use mdns_sd::{ResolvedService, ServiceDaemon, ServiceEvent, ServiceInfo};

use crate::net::Waker;

const SERVICE_TYPE: &str = "_continuum._tcp.local.";
const TXT_VERSION: &str = "1";

pub(crate) type AddressMap = Arc<Mutex<HashMap<DeviceId, SocketAddr>>>;

pub(crate) struct Discovery {
    daemon: ServiceDaemon,
    addresses: AddressMap,
}

impl Discovery {
    #[allow(dead_code)]
    pub(crate) fn discovered(&self) -> Vec<(DeviceId, SocketAddr)> {
        self.addresses
            .lock()
            .map(|map| map.iter().map(|(id, addr)| (*id, *addr)).collect())
            .unwrap_or_default()
    }
}

impl Drop for Discovery {
    fn drop(&mut self) {
        let _ = self.daemon.shutdown();
    }
}

pub(crate) fn start(
    device_id: DeviceId,
    port: u16,
    addresses: AddressMap,
    waker: Waker,
) -> anyhow::Result<Discovery> {
    let daemon = ServiceDaemon::new().map_err(|err| anyhow::anyhow!("mDNS unavailable: {err}"))?;

    let hostname = format!("{}.local.", host_label(&device_id));
    let instance = format!("continuum-{}", device_id.short());
    let properties = [("id", device_id.to_hex()), ("v", TXT_VERSION.to_string())];
    let service = ServiceInfo::new(
        SERVICE_TYPE,
        &instance,
        &hostname,
        "",
        port,
        &properties[..],
    )
    .map_err(|err| anyhow::anyhow!("invalid mDNS service info: {err}"))?
    .enable_addr_auto();
    daemon
        .register(service)
        .map_err(|err| anyhow::anyhow!("failed to advertise on mDNS: {err}"))?;
    tracing::info!(%instance, port, "advertising _continuum._tcp.local.");

    let receiver = daemon
        .browse(SERVICE_TYPE)
        .map_err(|err| anyhow::anyhow!("failed to browse mDNS: {err}"))?;
    tracing::info!("browsing _continuum._tcp.local.");

    let browse_addresses = Arc::clone(&addresses);
    let own_id = device_id;
    thread::spawn(move || {
        let mut names: HashMap<String, DeviceId> = HashMap::new();
        while let Ok(event) = receiver.recv() {
            match event {
                ServiceEvent::ServiceResolved(service) => {
                    let Some(id) = parse_device_id(&service) else {
                        tracing::debug!(
                            fullname = %service.get_fullname(),
                            "ignoring service without a valid id"
                        );
                        continue;
                    };
                    if id == own_id {
                        continue;
                    }
                    let Some(ip) = service.get_addresses_v4().into_iter().min() else {
                        tracing::debug!(device = %id.short(), "resolved peer has no IPv4 address");
                        continue;
                    };
                    let address = SocketAddr::V4(SocketAddrV4::new(ip, service.get_port()));
                    names.insert(service.get_fullname().to_string(), id);
                    let changed = {
                        let mut map = browse_addresses.lock().expect("discovery map");
                        map.insert(id, address) != Some(address)
                    };
                    if changed {
                        tracing::info!(device = %id.short(), %address, "discovered peer");
                        waker.wake();
                    }
                }
                ServiceEvent::ServiceRemoved(_, fullname) => {
                    if let Some(id) = names.remove(&fullname) {
                        let removed = browse_addresses
                            .lock()
                            .expect("discovery map")
                            .remove(&id)
                            .is_some();
                        if removed {
                            tracing::info!(device = %id.short(), "peer left the network");
                            waker.wake();
                        }
                    }
                }
                _ => {}
            }
        }
        tracing::debug!("mDNS browse ended");
    });

    Ok(Discovery { daemon, addresses })
}

fn parse_device_id(service: &ResolvedService) -> Option<DeviceId> {
    let bytes: [u8; 32] = hex::decode(service.get_property_val_str("id")?)
        .ok()?
        .try_into()
        .ok()?;
    Some(DeviceId::from_bytes(bytes))
}

fn host_label(device_id: &DeviceId) -> String {
    let raw = gethostname::gethostname();
    let label: String = raw
        .to_string_lossy()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .take(63)
        .collect::<String>()
        .to_ascii_lowercase();
    if label.is_empty() {
        format!("continuum-{}", device_id.short())
    } else {
        label
    }
}
