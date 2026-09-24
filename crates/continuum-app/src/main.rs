mod tray;

use tao::event::{Event, StartCause};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tray::TrayApp;
use tray_icon::menu::MenuEvent;
use tray_icon::TrayIconEvent;

enum UserEvent {
    Menu(MenuEvent),
    Tray(TrayIconEvent),
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

    let mut app: Option<TrayApp> = None;

    event_loop.run(move |event, _target, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            Event::NewEvents(StartCause::Init) => match TrayApp::new() {
                Ok(created) => {
                    tracing::info!("continuum tray started");
                    app = Some(created);
                }
                Err(err) => {
                    tracing::error!(%err, "failed to create tray");
                    *control_flow = ControlFlow::Exit;
                }
            },
            Event::UserEvent(UserEvent::Menu(event)) => {
                if let Some(app) = app.as_mut() {
                    app.on_menu(event.id(), control_flow);
                }
            }
            Event::UserEvent(UserEvent::Tray(_event)) => {}
            _ => {}
        }
    });
}

fn init_tracing() {
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();
}
