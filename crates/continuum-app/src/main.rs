mod monitor;
mod tray;

use std::time::{Duration, Instant};

use continuum_core::DeviceId;
use monitor::Monitor;
use tao::event::{Event, StartCause};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tray::TrayApp;
use tray_icon::menu::MenuEvent;
use tray_icon::TrayIconEvent;

enum UserEvent {
    Menu(MenuEvent),
    Tray(TrayIconEvent),
}

struct App {
    tray: TrayApp,
    monitor: Monitor,
}

impl App {
    fn new() -> anyhow::Result<Self> {
        let device_id = DeviceId::random()?;
        tracing::info!(device = %device_id.short(), "device identity ready");
        report_access_behavior();
        Ok(Self {
            tray: TrayApp::new()?,
            monitor: Monitor::open(device_id)?,
        })
    }

    fn poll(&mut self) -> anyhow::Result<()> {
        if let Some(item) = self.monitor.poll()? {
            tracing::info!(
                hash = %item.hash,
                items = item.items.len(),
                preview = %item.plain_text().map_or_else(|| "<non-text>".into(), preview),
                "local clipboard changed"
            );
        }
        Ok(())
    }
}

fn main() {
    init_tracing();

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

    let mut app: Option<App> = None;

    event_loop.run(move |event, _target, control_flow| {
        *control_flow = ControlFlow::WaitUntil(
            Instant::now()
                + app
                    .as_ref()
                    .map_or(Duration::from_secs(1), |a| a.monitor.next_delay()),
        );

        match event {
            Event::NewEvents(StartCause::Init) => match App::new() {
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
                    if let Err(err) = app.poll() {
                        tracing::warn!(%err, "clipboard poll failed");
                    }
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
