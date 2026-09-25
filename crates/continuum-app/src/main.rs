mod config;
#[cfg(feature = "tray")]
mod gui;
mod monitor;
mod net;
mod notify;
mod pairing;
mod service;
#[cfg(feature = "tray")]
mod tray;

use std::sync::{mpsc, Arc};

use continuum_core::{ClipboardItem, DeviceId, Version};
use continuum_net::{Identity, Message};
use monitor::Monitor;

const LARGE_TRANSFER: u64 = 2 * 1024 * 1024;

pub(crate) struct Core {
    pub(crate) monitor: Monitor,
    registry: net::Registry,
    waker: net::Waker,
    last_version: Option<Version>,
    last_item: Option<ClipboardItem>,
    paused: bool,
}

impl Core {
    pub(crate) fn new(
        identity: Arc<Identity>,
        config: &config::Config,
        on_event: impl Fn(net::NetEvent) + Send + Sync + 'static,
    ) -> anyhow::Result<Self> {
        let device_id = identity.device_id();
        let (registry, waker) = net::start(Arc::clone(&identity), config, on_event)?;
        Ok(Self {
            monitor: Monitor::open(device_id)?,
            registry,
            waker,
            last_version: None,
            last_item: None,
            paused: false,
        })
    }

    pub(crate) fn poll(&mut self) -> anyhow::Result<()> {
        if let Some(item) = self.monitor.poll()? {
            self.last_version = Some(item.version);
            tracing::info!(
                hash = %item.hash,
                preview = %item.plain_text().map_or_else(|| "<non-text>".into(), preview),
                "local clipboard changed"
            );
            if !self.paused {
                if let Some((name, size)) = file_transfer(&item) {
                    net::broadcast_message(
                        &self.registry,
                        &Message::TransferStart {
                            from: item.origin,
                            name: name.clone(),
                            size,
                        },
                    );
                    net::broadcast(&self.registry, &item);
                    net::broadcast_message(
                        &self.registry,
                        &Message::TransferDone {
                            from: item.origin,
                            name,
                        },
                    );
                } else {
                    net::broadcast(&self.registry, &item);
                }
                // Clipboard activity also nudges any parked reconnect loops, so links
                // come up immediately when a peer has just returned.
                self.waker.wake();
            }
            self.last_item = Some(item);
        }
        Ok(())
    }

    pub(crate) fn apply_remote(&mut self, item: ClipboardItem) {
        if self.paused {
            tracing::debug!("sync paused; ignoring remote clipboard");
            return;
        }
        if self.last_version.is_some_and(|last| item.version <= last) {
            tracing::debug!(from = %item.origin.short(), "dropped stale remote clipboard");
            return;
        }
        self.monitor.observe(item.version);
        match self.monitor.write(&item.items) {
            Ok(()) => {
                self.last_version = Some(item.version);
                self.last_item = Some(item.clone());
                tracing::info!(
                    from = %item.origin.short(),
                    preview = %item.plain_text().map_or_else(|| "<non-text>".into(), preview),
                    "applied remote clipboard"
                );
            }
            Err(err) => tracing::warn!(%err, "failed to write remote clipboard"),
        }
    }

    pub(crate) fn handle_event(&mut self, event: net::NetEvent) {
        match event {
            net::NetEvent::Clipboard(item) => self.apply_remote(item),
            net::NetEvent::TransferStart { from, name, size } => {
                if size >= LARGE_TRANSFER {
                    notify::receiving(&name, size, from);
                }
            }
            net::NetEvent::TransferDone { from, name } => notify::received(&name, from),
            net::NetEvent::PeerUp(device) => self.announce_to(device),
            net::NetEvent::PeerDown(device) => {
                tracing::info!(device = %device.short(), "peer offline");
            }
        }
    }

    fn announce_to(&self, device: DeviceId) {
        match &self.last_item {
            Some(item) => {
                tracing::info!(device = %device.short(), "announcing current clipboard");
                net::send_to(&self.registry, device, item);
            }
            None => tracing::debug!(device = %device.short(), "no clipboard to announce"),
        }
    }

    #[allow(dead_code)]
    pub(crate) fn send_now(&self) {
        match &self.last_item {
            Some(item) => {
                tracing::info!(preview = %item.plain_text().map_or_else(|| "<non-text>".into(), preview), "sending clipboard now");
                net::broadcast(&self.registry, item);
            }
            None => tracing::info!("nothing to send yet"),
        }
    }

    #[must_use]
    #[allow(dead_code)]
    pub(crate) fn peer_count(&self) -> usize {
        net::peer_count(&self.registry)
    }

    #[must_use]
    #[allow(dead_code)]
    pub(crate) const fn paused(&self) -> bool {
        self.paused
    }

    #[allow(dead_code)]
    pub(crate) fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
    }
}

pub(crate) struct Loaded {
    pub(crate) identity: Arc<Identity>,
    pub(crate) config: config::Config,
}

pub(crate) fn load() -> anyhow::Result<Loaded> {
    let identity_path = continuum_net::default_identity_path()
        .ok_or_else(|| anyhow::anyhow!("could not resolve the config directory"))?;
    let identity = Arc::new(Identity::load_or_generate(&identity_path)?);
    let device_id = identity.device_id();
    tracing::info!(device = %device_id.short(), "device identity ready");
    report_access_behavior();

    let config_path = config::default_config_path()
        .ok_or_else(|| anyhow::anyhow!("could not resolve the config directory"))?;
    let config = config::Config::load_or_create(&config_path)?;
    tracing::info!(listen = %config.listen, peers = config.peers.len(), "config loaded");

    Ok(Loaded { identity, config })
}

fn main() {
    init_tracing();

    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None => run_default(),
        Some("--headless") => run_headless_exit(),
        Some("show-id") => exit_on_error(show_id()),
        Some("status") => exit_on_error(show_status()),
        Some("peer") => exit_on_error(peer_command(&args[1..])),
        Some("dump") => exit_on_error(dump()),
        Some("put") => exit_on_error(put()),
        Some("pair") => exit_on_error(pair_command(&args[1..])),
        Some("pair-listen") => exit_on_error(pairing::pair_listen()),
        Some("install-service") => exit_on_error(service::install()),
        Some("uninstall-service") => exit_on_error(service::uninstall()),
        Some("-h" | "--help") => print_help(),
        Some(other) => {
            eprintln!("unknown command: {other}\n");
            print_help();
            std::process::exit(2);
        }
    }
}

fn run_default() {
    #[cfg(feature = "tray")]
    gui::run();

    #[cfg(not(feature = "tray"))]
    run_headless_exit();
}

fn exit_on_error(result: anyhow::Result<()>) {
    if let Err(err) = result {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}

fn show_id() -> anyhow::Result<()> {
    let identity = load_identity()?;
    let device = identity.device_id();
    println!("device id : {device}");
    println!("short     : {}", device.short());
    println!("public key: {}", hex::encode(identity.public_key()));
    Ok(())
}

fn show_status() -> anyhow::Result<()> {
    let config_path = config::default_config_path()
        .ok_or_else(|| anyhow::anyhow!("could not resolve the config directory"))?;
    let config = config::Config::load_or_create(&config_path)?;
    let identity = load_identity()?;

    println!("device   : {}", identity.device_id().short());
    println!("listen   : {}", config.listen);
    println!("config   : {}", config_path.display());
    println!("peers    : {}", config.peers.len());
    for peer in &config.peers {
        let short: String = peer.public_key.chars().take(16).collect();
        println!("  - {:<12} {:<24} {short}…", peer.name, peer.address);
    }
    Ok(())
}

fn peer_command(args: &[String]) -> anyhow::Result<()> {
    match args.first().map(String::as_str) {
        Some("add") => {
            let [_, name, address, public_key] = args else {
                anyhow::bail!(
                    "usage: continuum peer add <name> <host:port> <public_key_hex> (get it from `continuum show-id`)"
                );
            };
            if hex::decode(public_key)?.len() != 32 {
                anyhow::bail!("public key must be 32 bytes (64 hex characters)");
            }
            let path = config::default_config_path()
                .ok_or_else(|| anyhow::anyhow!("could not resolve the config directory"))?;
            let mut config = config::Config::load_or_create(&path)?;
            config.upsert_peer(config::PeerConfig {
                name: name.clone(),
                address: address.clone(),
                public_key: public_key.clone(),
            });
            config.save(&path)?;
            println!("added peer {name} ({address})");
            Ok(())
        }
        Some("list") | None => show_status(),
        Some(other) => anyhow::bail!("unknown peer subcommand: {other}"),
    }
}

fn dump() -> anyhow::Result<()> {
    let mut backend = continuum_platform::open()
        .map_err(|err| anyhow::anyhow!("clipboard unavailable: {err}"))?;
    match backend.read_current()? {
        Some(items) => {
            for (index, item) in items.iter().enumerate() {
                println!("item {index}: hash {}", item.content_hash());
                for rep in &item.representations {
                    println!("  {:<28} {} bytes", rep.mime, rep.bytes.len());
                }
            }
        }
        None => println!("(clipboard empty or holds no supported representations)"),
    }
    Ok(())
}

fn put() -> anyhow::Result<()> {
    use std::io::Read as _;
    let mut input = Vec::new();
    std::io::stdin().read_to_end(&mut input)?;
    let text = String::from_utf8_lossy(&input).into_owned();
    let item = continuum_core::ClipItem::new(vec![continuum_core::Representation::text(
        continuum_core::PLAIN_TEXT_MIME,
        &text,
    )])?;
    let mut backend = continuum_platform::open()
        .map_err(|err| anyhow::anyhow!("clipboard unavailable: {err}"))?;
    backend.write(&[item])?;
    println!("copied {} bytes; holding the selection", input.len());
    // On Wayland the process must outlive the copy to keep serving the selection.
    loop {
        std::thread::sleep(std::time::Duration::from_secs(3600));
    }
}

fn pair_command(args: &[String]) -> anyhow::Result<()> {
    let Some(host) = args.first() else {
        anyhow::bail!(
            "usage: continuum pair <host>  (the other device runs `continuum pair-listen`)"
        );
    };
    pairing::pair(host)
}

pub(crate) fn load_identity() -> anyhow::Result<Identity> {
    let path = continuum_net::default_identity_path()
        .ok_or_else(|| anyhow::anyhow!("could not resolve the config directory"))?;
    Ok(Identity::load_or_generate(&path)?)
}

fn print_help() {
    println!(
        "Continuum — clipboard sync between macOS and Linux\n\n\
         USAGE:\n  \
         continuum                 run the tray app\n  \
         continuum --headless      run without a GUI\n  \
         continuum show-id         print this device's id and public key\n  \
         continuum status          show config, listen address and peers\n  \
         continuum dump            print the current clipboard representations\n  \
         continuum put             copy stdin to the clipboard and hold it\n  \
         continuum peer add <name> <host:port> <public_key_hex>\n  \
         continuum peer list\n  \
         continuum pair <host>      pair with a device running `pair-listen`\n  \
         continuum pair-listen      accept a pairing request\n  \
         continuum install-service / uninstall-service\n  \
         continuum --help"
    );
}

fn run_headless_exit() {
    if let Err(err) = run_headless() {
        tracing::error!(%err, "headless runtime failed");
        std::process::exit(1);
    }
}

fn run_headless() -> anyhow::Result<()> {
    let loaded = load()?;
    let (sender, receiver) = mpsc::channel::<net::NetEvent>();
    let mut core = Core::new(Arc::clone(&loaded.identity), &loaded.config, move |event| {
        let _ = sender.send(event);
    })?;

    loop {
        // Block until an event arrives or the poll deadline elapses, so remote peer and
        // clipboard activity is handled immediately instead of waiting for the next poll.
        match receiver.recv_timeout(core.monitor.next_delay()) {
            Ok(event) => core.handle_event(event),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
        while let Ok(event) = receiver.try_recv() {
            core.handle_event(event);
        }
        core.poll()?;
    }
    Ok(())
}

pub(crate) fn report_access_behavior() {
    use continuum_platform::AccessBehavior;
    match continuum_platform::access_behavior() {
        AccessBehavior::AlwaysAllow => {}
        AccessBehavior::AlwaysDeny => tracing::warn!(
            "macOS is set to always deny pasteboard access for Continuum; sync will not work. \
             Allow access in System Settings > Privacy & Security."
        ),
        behavior => tracing::warn!(
            ?behavior,
            "pasteboard access may prompt; if sync stalls, allow Continuum in System Settings"
        ),
    }
}

fn file_transfer(item: &ClipboardItem) -> Option<(String, u64)> {
    use continuum_platform::files;
    if item.items.is_empty() {
        return None;
    }
    let mut names = Vec::new();
    let mut total = 0u64;
    for clip in &item.items {
        if clip.representations.len() != 1 {
            return None;
        }
        let representation = &clip.representations[0];
        if !files::is_file(representation) {
            return None;
        }
        let (name, size) = files::summary(representation)?;
        names.push(name);
        total += size;
    }
    let name = if names.len() == 1 {
        names.remove(0)
    } else {
        format!("{} files", names.len())
    };
    Some((name, total))
}

pub(crate) fn preview(text: &str) -> String {
    const MAX_CHARS: usize = 40;
    let mut truncated: String = text.chars().take(MAX_CHARS).collect();
    if text.chars().count() > MAX_CHARS {
        truncated.push('…');
    }
    truncated
}

fn init_tracing() {
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();
}
