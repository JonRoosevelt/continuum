use std::cell::RefCell;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;

use continuum_core::DeviceId;
use gtk::glib;
use gtk::prelude::*;

use crate::config::PeerConfig;
use crate::discovery::AddressMap;
use crate::pairing::{self, PAIRING_PORT};

const MAX_ROWS: usize = 4;

enum UiEvent {
    Code(String),
    Paired(PeerConfig),
    Cancelled,
    Failed(String),
}

enum Decision {
    Confirm,
    Abort,
}

struct PairingWindow {
    window: gtk::Window,
    host: gtk::Entry,
    code: gtk::Label,
    status: gtk::Label,
    nearby_list: gtk::ListBox,
    nearby_label: gtk::Label,
    nearby_note: gtk::Label,
    nearby: Vec<(DeviceId, SocketAddr)>,
    map: AddressMap,
    paired: Vec<DeviceId>,
    local: Option<DeviceId>,
    pair_button: gtk::Button,
    wait_button: gtk::Button,
    confirm_button: gtk::Button,
    cancel_button: gtk::Button,
    events: Receiver<UiEvent>,
    decision: Option<Sender<Decision>>,
    cancel: Option<Arc<AtomicBool>>,
    active: bool,
}

thread_local! {
    static WINDOW: RefCell<Option<PairingWindow>> = const { RefCell::new(None) };
}

pub fn open(map: AddressMap, paired: Vec<DeviceId>) {
    WINDOW.with(|cell| {
        let mut slot = cell.borrow_mut();
        match slot.as_mut() {
            Some(window) => {
                window.map = map;
                window.paired = paired;
                window.refresh_nearby();
                window.window.show_all();
                window.window.present();
            }
            None => {
                let window = PairingWindow::new(map, paired);
                window.window.show_all();
                window.window.present();
                *slot = Some(window);
            }
        }
    });
}

pub fn poll() {
    WINDOW.with(|cell| {
        let mut slot = cell.borrow_mut();
        let Some(window) = slot.as_mut() else {
            return;
        };
        while let Ok(event) = window.events.try_recv() {
            window.handle(event);
        }
        window.refresh_nearby();
    });
}

impl PairingWindow {
    fn new(map: AddressMap, paired: Vec<DeviceId>) -> Self {
        let local = crate::load_identity()
            .ok()
            .map(|identity| identity.device_id());

        let window = gtk::Window::new(gtk::WindowType::Toplevel);
        window.set_title("Pair a device");
        window.set_default_size(420, 372);
        window.set_position(gtk::WindowPosition::Center);
        window.connect_delete_event(|window, _| {
            window.hide();
            glib::Propagation::Stop
        });

        let content = gtk::Box::new(gtk::Orientation::Vertical, 8);
        content.set_border_width(20);
        window.add(&content);

        let title = gtk::Label::new(None);
        title.set_xalign(0.0);
        title.set_markup("<b>Pair with another device</b>");

        let hint = gtk::Label::new(Some(
            "Pick a nearby device, or enter an IP address and Pair — or click Wait for device.",
        ));
        hint.set_xalign(0.0);
        hint.set_line_wrap(true);

        let nearby_label = gtk::Label::new(Some("Nearby devices:"));
        nearby_label.set_xalign(0.0);

        let nearby_list = gtk::ListBox::new();
        nearby_list.set_selection_mode(gtk::SelectionMode::None);

        let nearby_note = gtk::Label::new(Some("No devices found yet."));
        nearby_note.set_xalign(0.0);

        let manual = gtk::Label::new(Some("Or type an address manually:"));
        manual.set_xalign(0.0);

        let host_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let host = gtk::Entry::new();
        host.set_placeholder_text(Some("192.168.1.42"));
        host.set_hexpand(true);
        let pair_button = gtk::Button::with_label("Pair");
        host_row.pack_start(&host, true, true, 0);
        host_row.pack_start(&pair_button, false, false, 0);

        let wait_button = gtk::Button::with_label("Wait for device");

        let code = gtk::Label::new(None);
        code.set_xalign(0.5);

        let status = gtk::Label::new(Some("Ready."));
        status.set_xalign(0.0);

        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let confirm_button = gtk::Button::with_label("Confirm");
        let cancel_button = gtk::Button::with_label("Cancel");
        actions.pack_start(&confirm_button, false, false, 0);
        actions.pack_start(&cancel_button, false, false, 0);

        content.pack_start(&title, false, false, 0);
        content.pack_start(&hint, false, false, 0);
        content.pack_start(&nearby_label, false, false, 0);
        content.pack_start(&nearby_list, false, false, 0);
        content.pack_start(&nearby_note, false, false, 0);
        content.pack_start(&manual, false, false, 0);
        content.pack_start(&host_row, false, false, 0);
        content.pack_start(&wait_button, false, false, 0);
        content.pack_start(&code, false, false, 0);
        content.pack_start(&status, false, false, 0);
        content.pack_start(&actions, false, false, 0);

        pair_button.connect_clicked(|_| on_pair());
        wait_button.connect_clicked(|_| on_wait());
        confirm_button.connect_clicked(|_| on_confirm());
        cancel_button.connect_clicked(|_| on_cancel());
        host.connect_activate(|entry| on_device(entry.text().to_string().trim().to_string()));

        let (_events_tx, events) = mpsc::channel();

        let state = Self {
            window,
            host,
            code,
            status,
            nearby_list,
            nearby_label,
            nearby_note,
            nearby: Vec::new(),
            map,
            paired,
            local,
            pair_button,
            wait_button,
            confirm_button,
            cancel_button,
            events,
            decision: None,
            cancel: None,
            active: false,
        };
        state.set_interactive(true, false, false);
        state
    }

    fn current_nearby(&self) -> Vec<(DeviceId, SocketAddr)> {
        let entries = self
            .map
            .lock()
            .map(|map| map.iter().map(|(id, addr)| (*id, *addr)).collect())
            .unwrap_or_default();
        filter_nearby(entries, self.local)
    }

    fn refresh_nearby(&mut self) {
        let entries = self.current_nearby();
        if entries == self.nearby {
            return;
        }
        self.nearby = entries;

        for child in self.nearby_list.children() {
            self.nearby_list.remove(&child);
        }

        for (id, address) in self.nearby.iter().take(MAX_ROWS).copied() {
            let paired = self.paired.contains(&id);
            let mut title = format!("{}  ·  {address}", id.short());
            if paired {
                title.push_str("  (paired)");
            }
            let row = gtk::ListBoxRow::new();
            let container = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            let button = gtk::Button::with_label(&title);
            button.set_relief(gtk::ReliefStyle::None);
            button.set_halign(gtk::Align::Start);
            button.set_hexpand(true);
            let host = address.ip().to_string();
            button.connect_clicked(move |_| on_device(host.clone()));
            container.pack_start(&button, true, true, 0);
            if paired {
                let unpair = gtk::Button::with_label("Unpair");
                unpair.connect_clicked(move |_| on_unpair(id));
                container.pack_start(&unpair, false, false, 0);
            }
            row.add(&container);
            self.nearby_list.add(&row);
            row.show_all();
        }

        if self.nearby.is_empty() {
            self.nearby_note.set_text("No devices found yet.");
        } else {
            self.nearby_note.set_text("");
        }

        let extra = self.nearby.len().saturating_sub(MAX_ROWS);
        if extra > 0 {
            self.nearby_label
                .set_text(&format!("Nearby devices: (+{extra} more)"));
        } else {
            self.nearby_label.set_text("Nearby devices:");
        }
    }

    fn handle(&mut self, event: UiEvent) {
        match event {
            UiEvent::Code(code) => {
                self.code.set_text(&code);
                self.set_status("Compare this code on both devices, then confirm.");
                self.set_interactive(false, true, true);
            }
            UiEvent::Paired(peer) => {
                match pairing::save_peer(&peer) {
                    Ok(()) => {
                        tracing::info!(peer = %peer.name, address = %peer.display_address(), "paired device");
                        self.set_status(&format!(
                            "Paired with {}. Restart Continuum to connect.",
                            peer.name
                        ));
                    }
                    Err(err) => {
                        tracing::warn!(%err, "failed to save paired device");
                        self.set_status(&format!("Paired, but saving failed: {err}"));
                    }
                }
                self.finish();
            }
            UiEvent::Cancelled => {
                self.code.set_text("");
                self.set_status("Pairing cancelled.");
                self.finish();
            }
            UiEvent::Failed(message) => {
                self.code.set_text("");
                self.set_status(&format!("Pairing failed: {message}"));
                self.finish();
            }
        }
    }

    fn set_status(&self, text: &str) {
        self.status.set_text(text);
    }

    fn set_interactive(&self, ready: bool, confirm: bool, cancel: bool) {
        self.pair_button.set_sensitive(ready);
        self.wait_button.set_sensitive(ready);
        self.host.set_sensitive(ready);
        self.confirm_button.set_sensitive(confirm);
        self.cancel_button.set_sensitive(cancel);
    }

    fn finish(&mut self) {
        self.active = false;
        self.decision = None;
        self.cancel = None;
        self.set_interactive(true, false, false);
    }
}

fn begin(window: &mut PairingWindow, initiator: bool, host: Option<String>) {
    if window.active {
        return;
    }
    let host = host.unwrap_or_else(|| window.host.text().trim().to_string());
    if initiator && host.is_empty() {
        window.set_status("Enter the other device's IP address first.");
        return;
    }

    let (events_tx, events_rx) = mpsc::channel();
    let (decision_tx, decision_rx) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));

    window.events = events_rx;
    window.decision = Some(decision_tx);
    window.cancel = Some(Arc::clone(&cancel));
    window.active = true;
    window.code.set_text("");
    window.set_interactive(false, false, true);

    if initiator {
        window.set_status(&format!("Connecting to {host}…"));
        std::thread::spawn(move || run_initiator(host, events_tx, decision_rx));
    } else {
        window.set_status(&format!("Waiting for a device on port {PAIRING_PORT}…"));
        std::thread::spawn(move || run_responder(events_tx, decision_rx, cancel));
    }
}

fn run_initiator(host: String, events: Sender<UiEvent>, decision: Receiver<Decision>) {
    match pairing::start_pair(&host) {
        Ok(started) => {
            let _ = events.send(UiEvent::Code(started.code.clone()));
            match decision.recv() {
                Ok(Decision::Confirm) => match started.confirm() {
                    Ok(peer) => {
                        let _ = events.send(UiEvent::Paired(peer));
                    }
                    Err(err) => {
                        let _ = events.send(UiEvent::Failed(err.to_string()));
                    }
                },
                _ => {
                    started.abort();
                    let _ = events.send(UiEvent::Cancelled);
                }
            }
        }
        Err(err) => {
            let _ = events.send(UiEvent::Failed(err.to_string()));
        }
    }
}

fn run_responder(events: Sender<UiEvent>, decision: Receiver<Decision>, cancel: Arc<AtomicBool>) {
    let listener = match pairing::bind_pair_listener() {
        Ok(listener) => listener,
        Err(err) => {
            let _ = events.send(UiEvent::Failed(err.to_string()));
            return;
        }
    };
    let started = match listener.accept_with_cancel(&cancel) {
        Ok(started) => started,
        Err(_) if cancel.load(Ordering::Relaxed) => {
            let _ = events.send(UiEvent::Cancelled);
            return;
        }
        Err(err) => {
            let _ = events.send(UiEvent::Failed(err.to_string()));
            return;
        }
    };
    let _ = events.send(UiEvent::Code(started.code.clone()));
    match decision.recv() {
        Ok(Decision::Confirm) => match started.accept() {
            Ok(peer) => {
                let _ = events.send(UiEvent::Paired(peer));
            }
            Err(err) => {
                let _ = events.send(UiEvent::Failed(err.to_string()));
            }
        },
        _ => {
            started.deny();
            let _ = events.send(UiEvent::Cancelled);
        }
    }
}

fn on_pair() {
    with_window(|window| begin(window, true, None));
}

fn on_wait() {
    with_window(|window| begin(window, false, None));
}

fn on_device(host: String) {
    with_window(|window| {
        window.host.set_text(&host);
        begin(window, true, Some(host));
    });
}

fn on_unpair(id: DeviceId) {
    with_window(|window| match pairing::forget_peer(id) {
        Ok(name) => {
            window.paired.retain(|paired| *paired != id);
            window.set_status(&format!(
                "Unpaired {name}. Restart Continuum to disconnect. Also unpair there if you want to fully remove it."
            ));
            window.nearby.clear();
            window.refresh_nearby();
        }
        Err(err) => {
            tracing::warn!(%err, "failed to unpair device");
            window.set_status(&format!("Could not unpair: {err}"));
        }
    });
}

fn on_confirm() {
    with_window(|window| {
        if !window.active {
            return;
        }
        if let Some(decision) = &window.decision {
            let _ = decision.send(Decision::Confirm);
        }
        window.set_status("Confirming…");
        window.set_interactive(false, false, false);
    });
}

fn on_cancel() {
    with_window(|window| {
        if !window.active {
            return;
        }
        if let Some(cancel) = &window.cancel {
            cancel.store(true, Ordering::Relaxed);
        }
        if let Some(decision) = &window.decision {
            let _ = decision.send(Decision::Abort);
        }
        window.set_status("Cancelling…");
        window.set_interactive(false, false, false);
    });
}

fn with_window(f: impl FnOnce(&mut PairingWindow)) {
    WINDOW.with(|cell| {
        if let Some(window) = cell.borrow_mut().as_mut() {
            f(window);
        }
    });
}

fn filter_nearby(
    entries: Vec<(DeviceId, SocketAddr)>,
    local: Option<DeviceId>,
) -> Vec<(DeviceId, SocketAddr)> {
    let mut entries: Vec<_> = entries
        .into_iter()
        .filter(|(id, _)| Some(*id) != local)
        .collect();
    entries.sort();
    entries
}
