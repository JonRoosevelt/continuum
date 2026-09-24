use std::process::Command;

#[cfg(target_os = "linux")]
use std::path::PathBuf;

pub fn install() -> anyhow::Result<()> {
    install_platform()
}

pub fn uninstall() -> anyhow::Result<()> {
    uninstall_platform()
}

#[cfg(target_os = "macos")]
fn install_platform() -> anyhow::Result<()> {
    let exe = std::env::current_exe()?.display().to_string();
    let path = dirs::home_dir()
        .map(|home| home.join("Library/LaunchAgents/dev.continuum.agent.plist"))
        .ok_or_else(|| anyhow::anyhow!("could not resolve the home directory"))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, macos_plist(&exe))?;
    run("launchctl", &["load", "-w", &path.display().to_string()]);
    println!("installed launch agent at {}", path.display());
    println!("the tray app will start at login (quit it from the menu-bar icon)");
    Ok(())
}

#[cfg(target_os = "macos")]
fn uninstall_platform() -> anyhow::Result<()> {
    let path = dirs::home_dir()
        .map(|home| home.join("Library/LaunchAgents/dev.continuum.agent.plist"))
        .ok_or_else(|| anyhow::anyhow!("could not resolve the home directory"))?;
    run("launchctl", &["unload", "-w", &path.display().to_string()]);
    if path.exists() {
        std::fs::remove_file(&path)?;
    }
    println!("removed {}", path.display());
    Ok(())
}

#[cfg(target_os = "linux")]
fn install_platform() -> anyhow::Result<()> {
    let exe = std::env::current_exe()?.display().to_string();
    let path = systemd_unit_path()
        .ok_or_else(|| anyhow::anyhow!("could not resolve the config directory"))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, systemd_unit(&exe))?;
    run("systemctl", &["--user", "daemon-reload"]);
    run("systemctl", &["--user", "enable", "--now", "continuum"]);
    println!("installed systemd user unit at {}", path.display());
    Ok(())
}

#[cfg(target_os = "linux")]
fn uninstall_platform() -> anyhow::Result<()> {
    run("systemctl", &["--user", "disable", "--now", "continuum"]);
    if let Some(path) = systemd_unit_path() {
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
        run("systemctl", &["--user", "daemon-reload"]);
        println!("removed {}", path.display());
    }
    Ok(())
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn install_platform() -> anyhow::Result<()> {
    anyhow::bail!("service installation is not supported on this platform")
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn uninstall_platform() -> anyhow::Result<()> {
    anyhow::bail!("service installation is not supported on this platform")
}

#[cfg(target_os = "macos")]
fn macos_plist(exe: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \
         \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n\
         <dict>\n\
         \x20 <key>Label</key><string>dev.continuum.agent</string>\n\
         \x20 <key>ProgramArguments</key><array><string>{exe}</string></array>\n\
         \x20 <key>RunAtLoad</key><true/>\n\
         <key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict>\n\
         </dict>\n\
         </plist>\n"
    )
}

#[cfg(target_os = "linux")]
fn systemd_unit_path() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("systemd/user/continuum.service"))
}

#[cfg(target_os = "linux")]
fn systemd_unit(exe: &str) -> String {
    format!(
        "[Unit]\n\
         Description=Continuum clipboard sync\n\
         After=graphical-session.target\n\
         PartOf=graphical-session.target\n\n\
         [Service]\n\
         ExecStart={exe} --headless\n\
         Restart=on-failure\n\
         RestartSec=2\n\n\
         [Install]\n\
         WantedBy=default.target\n"
    )
}

#[allow(dead_code)]
fn run(program: &str, args: &[&str]) {
    match Command::new(program).args(args).status() {
        Ok(status) if status.success() => {}
        Ok(status) => eprintln!(
            "warning: `{program} {}` exited with {status}",
            args.join(" ")
        ),
        Err(err) => eprintln!("warning: could not run `{program}`: {err}"),
    }
}
