pub mod wayland;
pub mod x11;

use crate::backend::{ClipboardBackend, ClipboardError};

pub fn open() -> Result<Box<dyn ClipboardBackend>, ClipboardError> {
    let mut errors: Vec<String> = Vec::new();

    let force_x11 = std::env::var_os("CONTINUUM_FORCE_X11").is_some();
    if !force_x11 && std::env::var_os("WAYLAND_DISPLAY").is_some() {
        match wayland::WaylandClipboard::new() {
            Ok(backend) => return Ok(Box::new(backend)),
            Err(err) => errors.push(format!("wayland: {err}")),
        }
    }

    match x11::X11Clipboard::new() {
        Ok(backend) => Ok(Box::new(backend)),
        Err(err) => {
            errors.push(format!("x11: {err}"));
            Err(ClipboardError::Read(format!(
                "no Linux clipboard backend available ({})",
                errors.join("; ")
            )))
        }
    }
}
