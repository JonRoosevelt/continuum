use std::sync::Arc;
use std::time::{Duration, Instant};

use continuum_core::ClipboardItem;
use tao::event::{Event, StartCause};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
use tray_icon::menu::MenuEvent;
use tray_icon::TrayIconEvent;

use crate::tray::TrayApp;
use crate::Core;

enum UserEvent {
    Menu(MenuEvent),
    Tray(TrayIconEvent),
    Remote(ClipboardItem),
}

struct App {
    core: Core,
    tray: TrayApp,
}

pub fn run() {
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
            Event::NewEvents(StartCause::Init) => match build_app(&remote_proxy) {
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

fn build_app(proxy: &EventLoopProxy<UserEvent>) -> anyhow::Result<App> {
    let loaded = crate::load()?;
    let proxy = proxy.clone();
    let core = Core::new(Arc::clone(&loaded.identity), &loaded.config, move |item| {
        let _ = proxy.send_event(UserEvent::Remote(item));
    })?;
    Ok(App {
        core,
        tray: TrayApp::new()?,
    })
}
