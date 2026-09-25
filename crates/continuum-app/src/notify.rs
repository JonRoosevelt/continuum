use continuum_core::DeviceId;

pub fn start(name: &str, size: u64, from: DeviceId) {
    on_start(name, size, from);
}

pub fn finish(name: &str, from: DeviceId) {
    on_finish(name, from);
}

#[cfg(target_os = "linux")]
const LARGE_TRANSFER: u64 = 2 * 1024 * 1024;

#[cfg(target_os = "linux")]
fn on_start(name: &str, size: u64, from: DeviceId) {
    if size >= LARGE_TRANSFER {
        show(&format!(
            "Receiving {name} ({}) from {}…",
            human(size),
            from.short()
        ));
    }
}

#[cfg(target_os = "linux")]
fn on_finish(name: &str, from: DeviceId) {
    show(&format!("Received {name} from {}", from.short()));
}

#[cfg(target_os = "linux")]
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

#[cfg(target_os = "linux")]
fn show(message: &str) {
    tracing::info!(%message, "notification");
    spawn_notification(message);
}

#[cfg(target_os = "linux")]
fn spawn_notification(message: &str) {
    use std::process::Command;

    let clean = message.replace('"', "'");
    let mut command = Command::new("hyprctl");
    command.args(["notify", "-1", "5000", "rgb(00d4aa)", &clean]);
    if let Some(signature) = hyprland_signature() {
        command.env("HYPRLAND_INSTANCE_SIGNATURE", signature);
    }
    if let Ok(runtime) = std::env::var("XDG_RUNTIME_DIR") {
        command.env("XDG_RUNTIME_DIR", runtime);
    }
    match command.spawn() {
        Ok(_) => {}
        Err(err) => tracing::warn!(%err, "could not spawn hyprctl"),
    }
}

#[cfg(target_os = "linux")]
fn hyprland_signature() -> Option<String> {
    if let Ok(signature) = std::env::var("HYPRLAND_INSTANCE_SIGNATURE") {
        return Some(signature);
    }
    let runtime = std::env::var("XDG_RUNTIME_DIR").ok()?;
    let entries = std::fs::read_dir(format!("{runtime}/hypr")).ok()?;
    entries
        .flatten()
        .find_map(|entry| entry.file_name().into_string().ok())
}

#[cfg(all(target_os = "macos", feature = "tray"))]
fn on_start(name: &str, size: u64, from: DeviceId) {
    crate::hud::show(name, size, &from.short());
}

#[cfg(all(target_os = "macos", feature = "tray"))]
fn on_finish(_name: &str, _from: DeviceId) {
    crate::hud::hide();
}

#[cfg(all(target_os = "macos", not(feature = "tray")))]
fn on_start(name: &str, size: u64, from: DeviceId) {
    tracing::info!(name, size, from = %from.short(), "receiving transfer");
}

#[cfg(all(target_os = "macos", not(feature = "tray")))]
fn on_finish(name: &str, from: DeviceId) {
    tracing::info!(name, from = %from.short(), "transfer complete");
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn on_start(_name: &str, _size: u64, _from: DeviceId) {}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn on_finish(_name: &str, _from: DeviceId) {}
