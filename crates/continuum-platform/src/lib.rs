//! Platform clipboard capture and injection.
//!
//! Backend selection is compile-time gated per target OS and resolved at
//! runtime on Linux (Wayland data-control first, X11/XWayland fallback).

pub mod backend;

pub use backend::{ClipboardBackend, ClipboardError};
