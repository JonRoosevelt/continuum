mod config;
mod monitor;
mod net;
mod tray;

use std::collections::{HashSet, VecDeque};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use continuum_core::{ClipboardItem, ContentHash, Version};
use continuum_net::Identity;
use monitor::Monitor;
use tao::event::{Event, StartCause};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
use tray::TrayApp;
use tray_icon::menu::MenuEvent;
use tray_icon::TrayIconEvent;

const RECENT_CAPACITY: usize = 256;

enum UserEvent {
    Menu(MenuEvent),
    Tray(TrayIconEvent),
    Remote(ClipboardItem),
}

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

struct Core {
    monitor: Monitor,
    registry: net::Registry,
    recent: Recent,
    last_version: Option<Version>,
}

impl Core {
    fn new(
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
        })
    }

    fn poll(&mut self) -> anyhow::Result<()> {
        if let Some(item) = self.monitor.poll()? {
            self.last_version = Some(item.version);
            if self.recent.insert(item.hash) {
                tracing::info!(
                    hash = %item.hash,
                    preview = %item.plain_text().map_or_else(|| "<non-text>".into(), preview),
                    "local clipboard changed"
                );
                net::broadcast(&self.registry, &item);
            }
        }
        Ok(())
    }

    fn apply_remote(&mut self, item: ClipboardItem) {
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
                tracing::info!(
                    from = %item.origin.short(),
                    preview = %item.plain_text().map_or_else(|| "<non-text>".into(), preview),
                    "applied remote clipboard"
                );
            }
            Err(err) => tracing::warn!(%err, "failed to write remote clipboard"),
        }
    }
}

struct App {
    core: Core,
    tray: TrayApp,
}

struct Loaded {
    identity: Arc<Identity>,
    config: config::Config,
}

fn load() -> anyhow::Result<Loaded> {
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

    if std::env::args().any(|arg| arg == "--headless") {
        if let Err(err) = run_headless() {
            tracing::error!(%err, "headless runtime failed");
            std::process::exit(1);
        }
        return;
    }

    run_gui();
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

fn run_gui() {
    #[allow(unused_mut)]
    let mut event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();

    #[cfg(target_os = "macos")]
    {
        use tao::platform::macos::{ActivationPolicy, EventLoopExtMacOS};
        event_loop.set_activation_policy(ActivationPolicy::Accessory);
    }

    let proxy = event_loop.create_proxy();
    MenuEvent::set_event_handler(Some(move |event| {
        let _ = proxy.send_event(UserEvent::Menu(event));
    }));
    let proxy = event_loop.create_proxy();
    TrayIconEvent::set_event_handler(Some(move |event| {
        let _ = proxy.send_event(UserEvent::Tray(event));
    }));
    let remote_proxy = event_loop.create_proxy();

    let mut app: Option<App> = None;

    event_loop.run(move |event, _target, control_flow| {
        *control_flow = ControlFlow::WaitUntil(
            Instant::now()
                + app
                    .as_ref()
                    .map_or(Duration::from_secs(1), |a| a.core.monitor.next_delay()),
        );

        match event {
            Event::NewEvents(StartCause::Init) => match build_gui_app(&remote_proxy) {
                Ok(created) => {
                    tracing::info!("continuum started");
                    app = Some(created);
                }
                Err(err) => {
                    tracing::error!(%err, "failed to start continuum");
                    *control_flow = ControlFlow::Exit;
                }
            },
            Event::NewEvents(StartCause::ResumeTimeReached { .. }) => {
                if let Some(app) = app.as_mut() {
                    if let Err(err) = app.core.poll() {
                        tracing::warn!(%err, "clipboard poll failed");
                    }
                }
            }
            Event::UserEvent(UserEvent::Remote(item)) => {
                if let Some(app) = app.as_mut() {
                    app.core.apply_remote(item);
                }
            }
            Event::UserEvent(UserEvent::Menu(event)) => {
                if let Some(app) = app.as_mut() {
                    app.tray.on_menu(event.id(), control_flow);
                }
            }
            Event::UserEvent(UserEvent::Tray(_event)) => {}
            _ => {}
        }
    });
}

fn build_gui_app(proxy: &EventLoopProxy<UserEvent>) -> anyhow::Result<App> {
    let loaded = load()?;
    let proxy = proxy.clone();
    let core = Core::new(Arc::clone(&loaded.identity), &loaded.config, move |item| {
        let _ = proxy.send_event(UserEvent::Remote(item));
    })?;
    Ok(App {
        core,
        tray: TrayApp::new()?,
    })
}

fn report_access_behavior() {
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

fn preview(text: &str) -> String {
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
