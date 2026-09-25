use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{mpsc, Arc};
use std::time::Duration;

use gtk::glib;

use crate::net::NetEvent;
use crate::tray::{TrayAction, TrayApp};
use crate::Core;

const TICK: Duration = Duration::from_millis(100);

struct App {
    core: Core,
    tray: TrayApp,
    quit: bool,
}

pub fn run() -> anyhow::Result<()> {
    gtk::init().map_err(|err| anyhow::anyhow!("gtk init failed: {err}"))?;

    let loaded = crate::load()?;
    let (sender, receiver) = mpsc::channel::<NetEvent>();
    let core = Core::new(Arc::clone(&loaded.identity), &loaded.config, move |event| {
        let _ = sender.send(event);
    })?;
    let tray = TrayApp::new()?;

    let app = Rc::new(RefCell::new(App {
        core,
        tray,
        quit: false,
    }));

    {
        let app = Rc::clone(&app);
        glib::timeout_add_local(TICK, move || {
            tick(&app, &receiver);
            if app.borrow().quit {
                gtk::main_quit();
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
    }

    gtk::main();
    Ok(())
}

fn tick(app: &Rc<RefCell<App>>, receiver: &mpsc::Receiver<NetEvent>) {
    loop {
        match tray_icon::menu::MenuEvent::receiver().try_recv() {
            Ok(event) => {
                let action = app.borrow().tray.action(event.id());
                apply_action(app, action);
            }
            Err(_) => break,
        }
    }

    while let Ok(event) = receiver.try_recv() {
        app.borrow_mut().core.handle_event(event);
    }

    {
        let mut app = app.borrow_mut();
        if let Err(err) = app.core.poll() {
            tracing::warn!(%err, "clipboard poll failed");
        }
    }
    update_status(app);
}

fn apply_action(app: &Rc<RefCell<App>>, action: TrayAction) {
    match action {
        TrayAction::ToggleSync => {
            let mut app = app.borrow_mut();
            let paused = !app.core.paused();
            app.core.set_paused(paused);
            app.tray.set_paused(paused);
        }
        TrayAction::SendNow => app.borrow().core.send_now(),
        // The Linux pairing UI is a follow-up; the daemon can already be paired from
        // the other device or via the `continuum pair` CLI.
        TrayAction::Pair => {
            tracing::info!("run `continuum pair <host>` or `continuum pair-listen` to pair")
        }
        TrayAction::Quit => app.borrow_mut().quit = true,
        TrayAction::None => {}
    }
    update_status(app);
}

fn update_status(app: &Rc<RefCell<App>>) {
    let app = app.borrow();
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
