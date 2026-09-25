use continuum_core::DeviceId;

pub fn receiving(name: &str, size: u64, from: DeviceId) {
    show(&format!(
        "Receiving {name} ({}) from {}…",
        human(size),
        from.short()
    ));
}

pub fn received(name: &str, from: DeviceId) {
    show(&format!("Received {name} from {}", from.short()));
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

#[cfg(target_os = "linux")]
fn show(message: &str) {
    use std::process::Command;

    let clean = message.replace('"', "'");
    let mut command = Command::new("hyprctl");
    command.args(["notify", "-1", "3000", "rgb(00d4aa)", &clean]);
    if let Some(signature) = hyprland_signature() {
        command.env("HYPRLAND_INSTANCE_SIGNATURE", signature);
    }
    if let Ok(runtime) = std::env::var("XDG_RUNTIME_DIR") {
        command.env("XDG_RUNTIME_DIR", runtime);
    }
    let _ = command.spawn();
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

#[cfg(target_os = "macos")]
fn show(message: &str) {
    use std::process::Command;

    let script = format!(
        "display notification \"{}\" with title \"Continuum\"",
        message.replace('"', "'")
    );
    let _ = Command::new("osascript").args(["-e", &script]).spawn();
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn show(_message: &str) {}
