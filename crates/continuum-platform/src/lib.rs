//! Platform clipboard capture and injection.
//!
//! Backend selection is compile-time gated per target OS and resolved at
//! runtime on Linux (Wayland data-control first, X11/XWayland fallback).

pub mod backend;
pub mod mime;

#[cfg(target_os = "linux")]
pub mod linux;

pub use backend::{AccessBehavior, ClipboardBackend, ClipboardError};

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::MacClipboard;

pub fn open() -> Result<Box<dyn ClipboardBackend>, ClipboardError> {
    #[cfg(target_os = "macos")]
    {
        Ok(Box::new(MacClipboard::new()))
    }
    #[cfg(target_os = "linux")]
    {
        linux::open()
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        Err(ClipboardError::Unavailable)
    }
}

#[must_use]
pub fn access_behavior() -> AccessBehavior {
    #[cfg(target_os = "macos")]
    {
        MacClipboard::access_behavior()
    }
    #[cfg(not(target_os = "macos"))]
    {
        AccessBehavior::AlwaysAllow
    }
}
