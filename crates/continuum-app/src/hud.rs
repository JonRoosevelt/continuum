#![allow(unsafe_code)]

use std::cell::RefCell;
use std::time::{Duration, Instant};

use objc2::rc::Retained;
use objc2::{MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSBackingStoreType, NSColor, NSFont, NSPanel, NSProgressIndicator, NSProgressIndicatorStyle,
    NSScreen, NSStatusWindowLevel, NSTextField, NSVisualEffectBlendingMode, NSVisualEffectMaterial,
    NSVisualEffectState, NSVisualEffectView, NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

const WIDTH: f64 = 280.0;
const HEIGHT: f64 = 56.0;
const TOP_MARGIN: f64 = 44.0;
const AUTO_HIDE: Duration = Duration::from_secs(30);
const MIN_VISIBLE: Duration = Duration::from_millis(1800);

struct Hud {
    panel: Retained<NSPanel>,
    label: Retained<NSTextField>,
    progress: Retained<NSProgressIndicator>,
    shown_at: Option<Instant>,
    deadline: Option<Instant>,
}

thread_local! {
    static HUD: RefCell<Option<Hud>> = const { RefCell::new(None) };
}

pub fn show(name: &str, size: u64) {
    let Some(mtm) = MainThreadMarker::new() else {
        tracing::warn!("transfer hud requested off the main thread");
        return;
    };

    HUD.with(|cell| {
        let mut slot = cell.borrow_mut();
        if slot.is_none() {
            let Some(hud) = Hud::new(mtm) else {
                tracing::warn!("could not create the transfer hud");
                return;
            };
            *slot = Some(hud);
        }
        let Some(hud) = slot.as_mut() else {
            return;
        };

        let text = format!("Receiving {name} ({})…", human(size));
        hud.label.setStringValue(&NSString::from_str(&text));
        hud.shown_at = Some(Instant::now());
        hud.deadline = Some(Instant::now() + AUTO_HIDE);
        // SAFETY: the sender argument is unused by NSProgressIndicator.
        unsafe { hud.progress.startAnimation(None) };
        hud.panel.orderFrontRegardless();
        tracing::info!(
            name,
            size,
            visible = hud.panel.isVisible(),
            "showing transfer hud"
        );
    });
}

pub fn hide() {
    HUD.with(|cell| {
        let mut slot = cell.borrow_mut();
        let Some(hud) = slot.as_mut() else {
            return;
        };
        hud.deadline = None;
        // Keep it on screen briefly even for instant transfers, so it is noticed.
        if let Some(shown_at) = hud.shown_at {
            if shown_at.elapsed() < MIN_VISIBLE {
                hud.deadline = Some(shown_at + MIN_VISIBLE);
                return;
            }
        }
        hud.shown_at = None;
        // SAFETY: the sender argument is unused by NSProgressIndicator.
        unsafe { hud.progress.stopAnimation(None) };
        hud.panel.orderOut(None);
        tracing::info!("hiding transfer hud");
    });
}

pub fn tick() {
    HUD.with(|cell| {
        let mut slot = cell.borrow_mut();
        let Some(hud) = slot.as_mut() else {
            return;
        };
        let expired = hud
            .deadline
            .is_some_and(|deadline| Instant::now() >= deadline);
        if expired {
            hud.deadline = None;
            // SAFETY: the sender argument is unused by NSProgressIndicator.
            unsafe { hud.progress.stopAnimation(None) };
            hud.panel.orderOut(None);
        }
    });
}

impl Hud {
    fn new(mtm: MainThreadMarker) -> Option<Self> {
        let screen = NSScreen::mainScreen(mtm)?;
        let screen_frame = screen.frame();
        let origin = NSPoint::new(
            screen_frame.origin.x + (screen_frame.size.width - WIDTH) / 2.0,
            screen_frame.origin.y + screen_frame.size.height - HEIGHT - TOP_MARGIN,
        );
        let frame = NSRect::new(origin, NSSize::new(WIDTH, HEIGHT));

        let panel = NSPanel::initWithContentRect_styleMask_backing_defer(
            NSPanel::alloc(mtm),
            frame,
            NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel,
            NSBackingStoreType::Buffered,
            false,
        );
        // The handles live for the process lifetime, so AppKit must not free the
        // panel when it is ordered out.
        // SAFETY: `setReleasedWhenClosed:` has no other requirements.
        unsafe { panel.setReleasedWhenClosed(false) };
        panel.setLevel(NSStatusWindowLevel);
        panel.setOpaque(false);
        panel.setBackgroundColor(Some(&NSColor::colorWithWhite_alpha(0.12, 0.9)));
        panel.setHasShadow(true);
        panel.setIgnoresMouseEvents(true);
        panel.setHidesOnDeactivate(false);
        panel.setCollectionBehavior(
            NSWindowCollectionBehavior::CanJoinAllSpaces
                | NSWindowCollectionBehavior::FullScreenAuxiliary
                | NSWindowCollectionBehavior::Stationary,
        );

        let content = NSVisualEffectView::initWithFrame(
            NSVisualEffectView::alloc(mtm),
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(WIDTH, HEIGHT)),
        );
        content.setMaterial(NSVisualEffectMaterial::HUDWindow);
        content.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
        content.setState(NSVisualEffectState::Active);
        content.setWantsLayer(true);
        if let Some(layer) = content.layer() {
            layer.setCornerRadius(12.0);
            layer.setMasksToBounds(true);
        }
        panel.setContentView(Some(&content));

        let label = NSTextField::labelWithString(&NSString::from_str(""), mtm);
        label.setFrame(NSRect::new(
            NSPoint::new(16.0, 31.0),
            NSSize::new(WIDTH - 32.0, 17.0),
        ));
        label.setTextColor(Some(&NSColor::whiteColor()));
        label.setFont(Some(&NSFont::systemFontOfSize(13.0)));

        let progress = NSProgressIndicator::initWithFrame(
            NSProgressIndicator::alloc(mtm),
            NSRect::new(NSPoint::new(16.0, 11.0), NSSize::new(WIDTH - 32.0, 12.0)),
        );
        progress.setStyle(NSProgressIndicatorStyle::Bar);
        progress.setIndeterminate(true);
        // A non-activating accessory app is usually inactive, which otherwise
        // freezes the indeterminate animation.
        // SAFETY: the setter only takes a boolean.
        unsafe { progress.setUsesThreadedAnimation(true) };

        content.addSubview(&label);
        content.addSubview(&progress);

        Some(Self {
            panel,
            label,
            progress,
            shown_at: None,
            deadline: None,
        })
    }
}

fn human(size: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    if size >= MB {
        format!("{:.1} MB", size as f64 / MB as f64)
    } else if size >= KB {
        format!("{:.0} KB", size as f64 / KB as f64)
    } else {
        format!("{size} B")
    }
}
