# Continuum

Tray-only clipboard sync between macOS and Linux over the local network. No cloud,
no account, no telemetry — copy on one machine and paste on another.

Status: plain-text sync between macOS and Linux (Wayland) is working end-to-end
over LAN/Tailscale. Rich formats, images, packaging and more are tracked in
[TASKS.md](TASKS.md).

## How it works

- Each OS captures clipboard changes (macOS `NSPasteboard`, Linux Wayland
  data-control / X11 selections) and injects remote content back.
- Peers talk over an encrypted **Noise IK** TCP connection (`X25519`,
  `ChaCha20-Poly1305`, `BLAKE2s`) with pinned static keys.
- Changes are broadcast to every connected peer; `BLAKE3` hashes deduplicate and
  a Lamport clock resolves conflicts (last-writer-wins).
- Identity is an `X25519` keypair; `DeviceId = BLAKE3(public key)`.

## Build

```sh
cargo build --release                 # tray app (macOS + Linux w/ GTK)
cargo build --release -p continuum-app --no-default-features   # headless, no GTK
scripts/bundle-macos.sh               # builds dist/Continuum.app (LSUIElement)
```

## Run

```sh
continuum                 # tray app
continuum --headless      # daemon, no GUI
continuum install-service # launchd (macOS) / systemd user unit (Linux)
continuum uninstall-service
scripts/install.sh        # build → ~/.local/bin → install service
```

## Pair two devices

```sh
# on device A
continuum pair-listen
# on device B
continuum pair <A-host>
```

Both sides print the same 6-digit verification code; confirm on each. Each
device pins the other's public key into its config and connects on next start.

## CLI

| command | purpose |
| --- | --- |
| `continuum show-id` | print this device's id and public key |
| `continuum status` | config path, listen address, paired peers |
| `continuum peer add <name> <host:port> <public_key_hex>` | manual pairing |
| `continuum peer list` | list peers |
| `continuum dump` | print current clipboard representations (debug) |
| `continuum pair <host>` / `pair-listen` | interactive pairing |
| `continuum install-service` / `uninstall-service` | login/autostart service |

## Config

- macOS: `~/Library/Application Support/continuum/`
- Linux: `~/.config/continuum/`

`config.json`:

```json
{
  "name": "my-laptop",
  "listen": "0.0.0.0:8770",
  "peers": [
    { "name": "desktop", "address": "100.x.y.z:8770", "public_key": "<64 hex chars>" }
  ]
}
```

## Security

- LAN-only; no cloud, account, or telemetry.
- Every payload is end-to-end encrypted with Noise; a peer is only trusted if its
  static key is pinned in config (pairing, or `peer add`).
- Pairing uses a Noise XX handshake plus a 6-digit short authentication string
  that both devices must match, defeating a man-in-the-middle.
- Images are normalized to PNG (a macOS-only TIFF is converted) so they interoperate with Linux.
- Sensitive content is skipped by default (`org.nspasteboard` transient/concealed/
  auto-generated, `com.apple.is-remote-clipboard`, `x-kde-passwordManagerHint`).
- Clipboard content is never written to disk (no history in v1).

Known gaps: the identity key is stored as a `0600` file, not yet in the OS
keychain / libsecret. On Wayland, payloads above 48 KiB are written via `wl-copy`
(wl-clipboard-rs truncates large clipboard writes); X11 large payload / INCR is
not implemented. `continuum put` copies stdin to the clipboard (and holds it).

## Platform notes

- **macOS**: menu-bar-only app (`LSUIElement`). Distribution requires a Developer
  ID signature and notarization; `scripts/bundle-macos.sh` ad-hoc signs for local
  use. macOS 15.4+ may ask for pasteboard access — allow Continuum in System
  Settings if sync stalls.
- **Linux Wayland**: works on compositors exposing data-control (KDE, Sway,
  Hyprland, niri, COSMIC, labwc, …). **GNOME/Mutter does not expose it** — use an
  X11 session or XWayland. The tray needs an AppIndicator extension on GNOME;
  otherwise run headless and use the CLI.
- **Linux X11**: supported (compile-checked).
