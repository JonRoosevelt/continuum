#![allow(unsafe_code)]

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, Sel};
use objc2::{define_class, msg_send, sel, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSApplication, NSBackingStoreType, NSButton, NSColor, NSFont, NSTextAlignment, NSTextField,
    NSWindow, NSWindowStyleMask,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

use crate::config::PeerConfig;
use crate::pairing::{self, PAIRING_PORT};

const WIDTH: f64 = 380.0;
const HEIGHT: f64 = 220.0;
const MARGIN: f64 = 20.0;
const CONTENT_WIDTH: f64 = WIDTH - MARGIN * 2.0;

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
    window: Retained<NSWindow>,
    host: Retained<NSTextField>,
    code: Retained<NSTextField>,
    status: Retained<NSTextField>,
    pair_button: Retained<NSButton>,
    wait_button: Retained<NSButton>,
    confirm_button: Retained<NSButton>,
    cancel_button: Retained<NSButton>,
    _target: Retained<PairingTarget>,
    events: Receiver<UiEvent>,
    decision: Option<Sender<Decision>>,
    cancel: Option<Arc<AtomicBool>>,
    active: bool,
}

thread_local! {
    static WINDOW: RefCell<Option<PairingWindow>> = const { RefCell::new(None) };
}

define_class!(
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    struct PairingTarget;

    unsafe impl NSObjectProtocol for PairingTarget {}

    impl PairingTarget {
        #[unsafe(method(pairClicked:))]
        fn pair_clicked(&self, _sender: &AnyObject) {
            on_pair();
        }

        #[unsafe(method(waitClicked:))]
        fn wait_clicked(&self, _sender: &AnyObject) {
            on_wait();
        }

        #[unsafe(method(confirmClicked:))]
        fn confirm_clicked(&self, _sender: &AnyObject) {
            on_confirm();
        }

        #[unsafe(method(cancelClicked:))]
        fn cancel_clicked(&self, _sender: &AnyObject) {
            on_cancel();
        }
    }
);

impl PairingTarget {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        // SAFETY: NSObject's `init` has no additional requirements.
        unsafe { msg_send![super(this), init] }
    }
}

pub fn open() {
    let Some(mtm) = MainThreadMarker::new() else {
        tracing::warn!("pairing window requested off the main thread");
        return;
    };

    WINDOW.with(|cell| {
        let mut slot = cell.borrow_mut();
        if slot.is_none() {
            *slot = Some(PairingWindow::new(mtm));
        }
        let Some(window) = slot.as_ref() else {
            return;
        };
        window.window.makeKeyAndOrderFront(None);
        // An accessory app is normally inactive, so the text field will not accept
        // typing until the app is explicitly activated.
        #[allow(deprecated)]
        NSApplication::sharedApplication(mtm).activateIgnoringOtherApps(true);
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
    });
}

impl PairingWindow {
    fn new(mtm: MainThreadMarker) -> Self {
        // SAFETY: the content rect and style mask are valid for a titled window.
        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(mtm),
                NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(WIDTH, HEIGHT)),
                NSWindowStyleMask::Titled | NSWindowStyleMask::Closable,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        // SAFETY: the window is retained for the process lifetime and re-shown later.
        unsafe { window.setReleasedWhenClosed(false) };
        window.setTitle(&NSString::from_str("Pair a device"));
        window.center();

        let content = window
            .contentView()
            .expect("a titled window always has a content view");

        let target = PairingTarget::new(mtm);

        let title = label(mtm, "Pair with another device");
        title.setFrame(NSRect::new(
            NSPoint::new(MARGIN, HEIGHT - 24.0),
            NSSize::new(CONTENT_WIDTH, 18.0),
        ));
        title.setFont(Some(&NSFont::boldSystemFontOfSize(15.0)));

        let hint = label(
            mtm,
            "Enter the other device's IP address, then Pair — or click Wait for device.",
        );
        hint.setFrame(NSRect::new(
            NSPoint::new(MARGIN, HEIGHT - 42.0),
            NSSize::new(CONTENT_WIDTH, 14.0),
        ));
        hint.setFont(Some(&NSFont::systemFontOfSize(11.0)));
        hint.setTextColor(Some(&NSColor::secondaryLabelColor()));

        let host = NSTextField::textFieldWithString(&NSString::from_str(""), mtm);
        host.setFrame(NSRect::new(
            NSPoint::new(MARGIN, HEIGHT - 78.0),
            NSSize::new(210.0, 24.0),
        ));
        host.setPlaceholderString(Some(&NSString::from_str("192.168.1.42")));

        let pair_button = button(mtm, "Pair", sel!(pairClicked:), &target);
        pair_button.setFrame(NSRect::new(
            NSPoint::new(MARGIN + 220.0, HEIGHT - 80.0),
            NSSize::new(120.0, 28.0),
        ));

        let wait_button = button(mtm, "Wait for device", sel!(waitClicked:), &target);
        wait_button.setFrame(NSRect::new(
            NSPoint::new(MARGIN, HEIGHT - 116.0),
            NSSize::new(160.0, 28.0),
        ));

        let code = label(mtm, "");
        code.setFrame(NSRect::new(
            NSPoint::new(MARGIN, HEIGHT - 154.0),
            NSSize::new(CONTENT_WIDTH, 28.0),
        ));
        code.setFont(Some(&NSFont::boldSystemFontOfSize(20.0)));
        code.setAlignment(NSTextAlignment::Center);

        let status = label(mtm, "Ready.");
        status.setFrame(NSRect::new(
            NSPoint::new(MARGIN, HEIGHT - 172.0),
            NSSize::new(CONTENT_WIDTH, 14.0),
        ));
        status.setFont(Some(&NSFont::systemFontOfSize(11.0)));
        status.setTextColor(Some(&NSColor::secondaryLabelColor()));

        let confirm_button = button(mtm, "Confirm", sel!(confirmClicked:), &target);
        confirm_button.setFrame(NSRect::new(
            NSPoint::new(MARGIN, 12.0),
            NSSize::new(110.0, 28.0),
        ));

        let cancel_button = button(mtm, "Cancel", sel!(cancelClicked:), &target);
        cancel_button.setFrame(NSRect::new(
            NSPoint::new(MARGIN + 120.0, 12.0),
            NSSize::new(110.0, 28.0),
        ));

        content.addSubview(&title);
        content.addSubview(&hint);
        content.addSubview(&host);
        content.addSubview(&pair_button);
        content.addSubview(&wait_button);
        content.addSubview(&code);
        content.addSubview(&status);
        content.addSubview(&confirm_button);
        content.addSubview(&cancel_button);

        let (_events_tx, events) = mpsc::channel();

        let state = Self {
            window,
            host,
            code,
            status,
            pair_button,
            wait_button,
            confirm_button,
            cancel_button,
            _target: target,
            events,
            decision: None,
            cancel: None,
            active: false,
        };
        state.set_interactive(true, false, false);
        state
    }

    fn handle(&mut self, event: UiEvent) {
        match event {
            UiEvent::Code(code) => {
                self.code.setStringValue(&NSString::from_str(&code));
                self.set_status("Compare this code on both devices, then confirm.");
                self.set_interactive(false, true, true);
            }
            UiEvent::Paired(peer) => {
                match pairing::save_peer(&peer) {
                    Ok(()) => {
                        tracing::info!(peer = %peer.name, address = %peer.address, "paired device");
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
                self.code.setStringValue(&NSString::from_str(""));
                self.set_status("Pairing cancelled.");
                self.finish();
            }
            UiEvent::Failed(message) => {
                self.code.setStringValue(&NSString::from_str(""));
                self.set_status(&format!("Pairing failed: {message}"));
                self.finish();
            }
        }
    }

    fn set_status(&self, text: &str) {
        self.status.setStringValue(&NSString::from_str(text));
    }

    fn set_interactive(&self, ready: bool, confirm: bool, cancel: bool) {
        self.pair_button.setEnabled(ready);
        self.wait_button.setEnabled(ready);
        self.host.setEnabled(ready);
        self.confirm_button.setEnabled(confirm);
        self.cancel_button.setEnabled(cancel);
    }

    fn finish(&mut self) {
        self.active = false;
        self.decision = None;
        self.cancel = None;
        self.set_interactive(true, false, false);
    }
}

fn start(window: &mut PairingWindow, initiator: bool) {
    if window.active {
        return;
    }
    let host = window.host.stringValue().to_string();
    let host = host.trim().to_string();
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
    window.code.setStringValue(&NSString::from_str(""));
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
    with_window(|window| start(window, true));
}

fn on_wait() {
    with_window(|window| start(window, false));
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
        let mut slot = cell.borrow_mut();
        if let Some(window) = slot.as_mut() {
            f(window);
        }
    });
}

fn label(mtm: MainThreadMarker, text: &str) -> Retained<NSTextField> {
    NSTextField::labelWithString(&NSString::from_str(text), mtm)
}

fn button(
    mtm: MainThreadMarker,
    title: &str,
    action: Sel,
    target: &PairingTarget,
) -> Retained<NSButton> {
    // SAFETY: the target is retained by the window and responds to `action`.
    unsafe {
        NSButton::buttonWithTitle_target_action(
            &NSString::from_str(title),
            Some(target),
            Some(action),
            mtm,
        )
    }
}
