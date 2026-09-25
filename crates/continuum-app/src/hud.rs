#![allow(unsafe_code)]

use std::cell::RefCell;
use std::time::{Duration, Instant};

use objc2::rc::Retained;
use objc2::{MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSBackingStoreType, NSColor, NSControlSize, NSEvent, NSFont, NSImage, NSImageView,
    NSLineBreakMode, NSPanel, NSProgressIndicator, NSProgressIndicatorStyle, NSScreen,
    NSStatusWindowLevel, NSTextField, NSVisualEffectBlendingMode, NSVisualEffectMaterial,
    NSVisualEffectState, NSVisualEffectView, NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

const WIDTH: f64 = 300.0;
const HEIGHT: f64 = 64.0;
const TOP_MARGIN: f64 = 44.0;
const CORNER_RADIUS: f64 = 14.0;
const ICON_SIZE: f64 = 32.0;
const ICON_X: f64 = 16.0;
const TEXT_X: f64 = 58.0;
const SPINNER_SIZE: f64 = 16.0;
const SPINNER_RIGHT: f64 = 30.0;
const TEXT_WIDTH: f64 = WIDTH - SPINNER_RIGHT - 8.0 - TEXT_X;
const AUTO_HIDE: Duration = Duration::from_secs(30);
const MIN_VISIBLE: Duration = Duration::from_millis(1800);

struct Hud {
    panel: Retained<NSPanel>,
    title: Retained<NSTextField>,
    subtitle: Retained<NSTextField>,
    progress: Retained<NSProgressIndicator>,
    shown_at: Option<Instant>,
    deadline: Option<Instant>,
}

thread_local! {
    static HUD: RefCell<Option<Hud>> = const { RefCell::new(None) };
}

pub fn show(name: &str, size: u64, device: &str) {
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

        hud.panel.setFrameOrigin(target_origin(mtm));
        hud.title.setStringValue(&NSString::from_str(name));
        hud.subtitle.setStringValue(&NSString::from_str(&format!(
            "from {device} · {}",
            human(size)
        )));
        hud.shown_at = Some(Instant::now());
        hud.deadline = Some(Instant::now() + AUTO_HIDE);
        // SAFETY: the sender argument is unused by NSProgressIndicator.
        unsafe { hud.progress.startAnimation(None) };
        hud.panel.orderFrontRegardless();
        tracing::info!(
            name,
            size,
            device,
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
        let frame = NSRect::new(target_origin(mtm), NSSize::new(WIDTH, HEIGHT));

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
        // A clear window background so only the rounded effect view shows; an opaque
        // one leaves square corners poking out behind the rounded blur.
        panel.setBackgroundColor(Some(&NSColor::clearColor()));
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
            layer.setCornerRadius(CORNER_RADIUS);
            layer.setMasksToBounds(true);
            layer.setBorderWidth(0.5);
            layer.setBorderColor(Some(&NSColor::colorWithWhite_alpha(1.0, 0.12).CGColor()));
        }
        panel.setContentView(Some(&content));

        let icon = NSImageView::initWithFrame(
            NSImageView::alloc(mtm),
            NSRect::new(
                NSPoint::new(ICON_X, (HEIGHT - ICON_SIZE) / 2.0),
                NSSize::new(ICON_SIZE, ICON_SIZE),
            ),
        );
        if let Some(image) = download_icon() {
            image.setTemplate(true);
            icon.setImage(Some(&image));
        }
        icon.setContentTintColor(Some(&NSColor::whiteColor()));

        let title = NSTextField::labelWithString(&NSString::from_str(""), mtm);
        title.setFrame(NSRect::new(
            NSPoint::new(TEXT_X, 33.0),
            NSSize::new(TEXT_WIDTH, 17.0),
        ));
        title.setFont(Some(&NSFont::boldSystemFontOfSize(13.0)));
        title.setTextColor(Some(&NSColor::whiteColor()));
        title.setLineBreakMode(NSLineBreakMode::ByTruncatingMiddle);

        let subtitle = NSTextField::labelWithString(&NSString::from_str(""), mtm);
        subtitle.setFrame(NSRect::new(
            NSPoint::new(TEXT_X, 16.0),
            NSSize::new(TEXT_WIDTH, 15.0),
        ));
        subtitle.setFont(Some(&NSFont::systemFontOfSize(11.0)));
        subtitle.setTextColor(Some(&NSColor::secondaryLabelColor()));
        subtitle.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);

        let progress = NSProgressIndicator::initWithFrame(
            NSProgressIndicator::alloc(mtm),
            NSRect::new(
                NSPoint::new(WIDTH - SPINNER_RIGHT, (HEIGHT - SPINNER_SIZE) / 2.0),
                NSSize::new(SPINNER_SIZE, SPINNER_SIZE),
            ),
        );
        progress.setStyle(NSProgressIndicatorStyle::Spinning);
        progress.setControlSize(NSControlSize::Small);
        progress.setIndeterminate(true);
        // A non-activating accessory app is usually inactive, which otherwise
        // freezes the indeterminate animation.
        // SAFETY: the setter only takes a boolean.
        unsafe { progress.setUsesThreadedAnimation(true) };

        content.addSubview(&icon);
        content.addSubview(&title);
        content.addSubview(&subtitle);
        content.addSubview(&progress);

        Some(Self {
            panel,
            title,
            subtitle,
            progress,
            shown_at: None,
            deadline: None,
        })
    }
}

fn download_icon() -> Option<Retained<NSImage>> {
    let symbol = NSString::from_str("arrow.down.doc.fill");
    if let Some(image) = NSImage::imageWithSystemSymbolName_accessibilityDescription(&symbol, None)
    {
        return Some(image);
    }
    let fallback = NSString::from_str("doc");
    NSImage::imageWithSystemSymbolName_accessibilityDescription(&fallback, None)
}

fn target_origin(mtm: MainThreadMarker) -> NSPoint {
    let mouse = NSEvent::mouseLocation();
    let screen_frame = NSScreen::screens(mtm)
        .iter()
        .map(|screen| screen.frame())
        .find(|frame| point_in_rect(mouse, *frame))
        .or_else(|| NSScreen::mainScreen(mtm).map(|screen| screen.frame()));
    let Some(frame) = screen_frame else {
        return NSPoint::new(0.0, 0.0);
    };
    NSPoint::new(
        frame.origin.x + (frame.size.width - WIDTH) / 2.0,
        frame.origin.y + frame.size.height - HEIGHT - TOP_MARGIN,
    )
}

fn point_in_rect(point: NSPoint, rect: NSRect) -> bool {
    point.x >= rect.origin.x
        && point.x <= rect.origin.x + rect.size.width
        && point.y >= rect.origin.y
        && point.y <= rect.origin.y + rect.size.height
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
