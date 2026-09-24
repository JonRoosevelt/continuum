mod config;
#[cfg(feature = "tray")]
mod gui;
mod monitor;
mod net;
#[cfg(feature = "tray")]
mod tray;

use std::collections::{HashSet, VecDeque};
use std::sync::{mpsc, Arc};

use continuum_core::{ClipboardItem, ContentHash, Version};
use continuum_net::Identity;
use monitor::Monitor;

const RECENT_CAPACITY: usize = 256;

struct Recent {
    set: HashSet<ContentHash>,
    order: VecDeque<ContentHash>,
}

impl Recent {
    fn new() -> Self {
        Self {
            set: HashSet::new(),
            order: VecDeque::new(),
        }
    }

    /// Returns `true` if the hash was not seen recently.
    fn insert(&mut self, hash: ContentHash) -> bool {
        if !self.set.insert(hash) {
            return false;
        }
        self.order.push_back(hash);
        if self.order.len() > RECENT_CAPACITY {
            if let Some(oldest) = self.order.pop_front() {
                self.set.remove(&oldest);
            }
        }
        true
    }
}

pub(crate) struct Core {
    pub(crate) monitor: Monitor,
    registry: net::Registry,
    recent: Recent,
    last_version: Option<Version>,
    last_item: Option<ClipboardItem>,
    paused: bool,
}

impl Core {
    pub(crate) fn new(
        identity: Arc<Identity>,
        config: &config::Config,
        on_remote: impl Fn(ClipboardItem) + Send + Sync + 'static,
    ) -> anyhow::Result<Self> {
        let device_id = identity.device_id();
        Ok(Self {
            monitor: Monitor::open(device_id)?,
            registry: net::start(Arc::clone(&identity), config, on_remote)?,
            recent: Recent::new(),
            last_version: None,
            last_item: None,
            paused: false,
        })
    }

    pub(crate) fn poll(&mut self) -> anyhow::Result<()> {
        if let Some(item) = self.monitor.poll()? {
            self.last_version = Some(item.version);
            if self.recent.insert(item.hash) {
                tracing::info!(
                    hash = %item.hash,
                    preview = %item.plain_text().map_or_else(|| "<non-text>".into(), preview),
                    "local clipboard changed"
                );
                if !self.paused {
                    net::broadcast(&self.registry, &item);
                }
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
        if !self.recent.insert(item.hash) {
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

fn load_identity() -> anyhow::Result<Identity> {
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
         continuum peer add <name> <host:port> <public_key_hex>\n  \
         continuum peer list\n  \
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
    let (sender, receiver) = mpsc::channel::<ClipboardItem>();
    let mut core = Core::new(Arc::clone(&loaded.identity), &loaded.config, move |item| {
        let _ = sender.send(item);
    })?;

    loop {
        while let Ok(item) = receiver.try_recv() {
            core.apply_remote(item);
        }
        core.poll()?;
        std::thread::sleep(core.monitor.next_delay());
    }
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
