use std::sync::Arc;
use std::time::{Duration, Instant};

use tao::event::{Event, StartCause};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
use tray_icon::menu::MenuEvent;
use tray_icon::TrayIconEvent;

use crate::net::NetEvent;
use crate::tray::{TrayAction, TrayApp};
use crate::Core;

enum UserEvent {
    Menu(MenuEvent),
    Tray(TrayIconEvent),
    Net(NetEvent),
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
                #[cfg(target_os = "macos")]
                crate::hud::tick();
                #[cfg(target_os = "macos")]
                crate::pairing_window::poll();
                if let Some(app) = app.as_mut() {
                    if let Err(err) = app.core.poll() {
                        tracing::warn!(%err, "clipboard poll failed");
                    }
                    update_status(app);
                }
            }
            Event::UserEvent(UserEvent::Net(event)) => {
                if let Some(app) = app.as_mut() {
                    app.core.handle_event(event);
                }
            }
            Event::UserEvent(UserEvent::Menu(event)) => {
                if let Some(app) = app.as_mut() {
                    handle_action(app, app.tray.action(event.id()), control_flow);
                }
            }
            Event::UserEvent(UserEvent::Tray(_event)) => {}
            _ => {}
        }
    });
}

fn handle_action(app: &mut App, action: TrayAction, control_flow: &mut ControlFlow) {
    match action {
        TrayAction::ToggleSync => {
            let paused = !app.core.paused();
            app.core.set_paused(paused);
            app.tray.set_paused(paused);
            update_status(app);
        }
        TrayAction::SendNow => app.core.send_now(),
        TrayAction::Pair => {
            #[cfg(target_os = "macos")]
            crate::pairing_window::open(
                app.core.discovered_addresses(),
                app.core.paired_device_ids(),
                app.core.discovered_map(),
            );
            #[cfg(not(target_os = "macos"))]
            tracing::info!("run `continuum pair <host:port>` on this machine to pair a device");
        }
        TrayAction::Quit => *control_flow = ControlFlow::Exit,
        TrayAction::None => {}
    }
}

fn update_status(app: &App) {
    let peers = app.core.peer_count();
    let state = if app.core.paused() {
        "paused"
    } else if peers == 0 {
        "no peers"
    } else {
        "syncing"
    };
    app.tray
        .set_status(&format!("Continuum — {state} · {peers} peer(s)"));
}

fn build_app(proxy: &EventLoopProxy<UserEvent>) -> anyhow::Result<App> {
    let loaded = crate::load()?;
    let proxy = proxy.clone();
    let core = Core::new(Arc::clone(&loaded.identity), &loaded.config, move |event| {
        let _ = proxy.send_event(UserEvent::Net(event));
    })?;
    Ok(App {
        core,
        tray: TrayApp::new()?,
    })
}
