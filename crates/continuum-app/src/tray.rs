use tray_icon::menu::{CheckMenuItem, Menu, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

use tao::event_loop::ControlFlow;

pub const MENU_TOGGLE_SYNC: &str = "toggle-sync";
pub const MENU_SEND_NOW: &str = "send-now";
pub const MENU_PAIR: &str = "pair";
pub const MENU_QUIT: &str = "quit";

pub struct TrayApp {
    _tray: TrayIcon,
    toggle_sync: CheckMenuItem,
    paused: bool,
    quit_id: MenuId,
    send_id: MenuId,
    pair_id: MenuId,
    toggle_id: MenuId,
}

impl TrayApp {
    pub fn new() -> anyhow::Result<Self> {
        let toggle_sync = CheckMenuItem::with_id(MENU_TOGGLE_SYNC, "Pause sync", true, false, None);
        let send_now = MenuItem::with_id(MENU_SEND_NOW, "Send clipboard now", true, None);
        let pair = MenuItem::with_id(MENU_PAIR, "Pair device…", true, None);
        let quit = MenuItem::with_id(MENU_QUIT, "Quit Continuum", true, None);

        let menu = Menu::new();
        menu.append_items(&[
            &MenuItem::with_id("status", "Continuum — idle", false, None),
            &PredefinedMenuItem::separator(),
            &toggle_sync,
            &send_now,
            &PredefinedMenuItem::separator(),
            &pair,
            &PredefinedMenuItem::separator(),
            &quit,
        ])?;

        let tray = TrayIconBuilder::new()
            .with_tooltip("Continuum")
            .with_icon(status_icon())
            .with_icon_as_template(true)
            .with_menu(Box::new(menu))
            .build()?;

        Ok(Self {
            _tray: tray,
            quit_id: quit.id().clone(),
            send_id: send_now.id().clone(),
            pair_id: pair.id().clone(),
            toggle_id: toggle_sync.id().clone(),
            toggle_sync,
            paused: false,
        })
    }

    pub fn on_menu(&mut self, id: &MenuId, control_flow: &mut ControlFlow) {
        if id == &self.quit_id {
            tracing::info!("quit requested");
            *control_flow = ControlFlow::Exit;
        } else if id == &self.toggle_id {
            self.paused = !self.paused;
            self.toggle_sync.set_checked(self.paused);
            tracing::info!(paused = self.paused, "sync toggled");
        } else if id == &self.send_id {
            tracing::info!("send clipboard now requested");
        } else if id == &self.pair_id {
            tracing::info!("pairing requested (not yet implemented)");
        }
    }
}

fn status_icon() -> Icon {
    let size = 22u32;
    let center = (size as f32 - 1.0) / 2.0;
    let mut rgba = vec![0u8; (size * size * 4) as usize];
    for y in 0..size {
        for x in 0..size {
            let dx = x as f32 - center;
            let dy = y as f32 - center;
            let dist = (dx * dx + dy * dy).sqrt();
            let alpha = if (3.5..=8.5).contains(&dist) { 255 } else { 0 };
            let idx = ((y * size + x) * 4) as usize;
            rgba[idx] = 255;
            rgba[idx + 1] = 255;
            rgba[idx + 2] = 255;
            rgba[idx + 3] = alpha;
        }
    }
    Icon::from_rgba(rgba, size, size).expect("generated icon is valid")
}
