# Continuum — Task List

Cross-platform, tray-only clipboard sync between macOS and Linux over the LAN.
Rust. TCP + Noise IK (snow). mDNS discovery. Pinned identities. LAN-only, no cloud.

Legend: `[ ]` todo · `[~]` in progress · `[x]` done · blocked items note the blocker.

## Phase 0 — Scaffolding

- [x] Init git repo, Cargo workspace layout (`crates/`: `continuum-core`, `continuum-platform`, `continuum-net`, `continuum-app`), `.gitignore`, rustfmt/clippy config
- [x] Define `ClipboardItem` model: multi-item, MIME/UTI map, BLAKE3 content hash, origin id, Lamport timestamp
- [x] Tray skeleton (`tray-icon` + `muda`) with placeholder menu and accessory (no-Dock) activation policy; real `.app`/`LSUIElement` bundle deferred to Phase 5
- [x] NOTE: `tray-icon` 0.24 has no `ksni` backend — Linux tray is GTK + libappindicator. Decision: keep it behind the default `tray` feature and ship a headless build + CLI for Linux/GNOME.

## Phase 1 — Clipboard platform layer

- [x] macOS capture: `objc2-app-kit`, adaptive `changeCount` poll (100 ms active → 1 s idle), read all items/types
- [x] macOS inject: write multi-item pasteboard, record resulting `changeCount` for echo suppression
- [x] macOS 15.4+ pasteboard-privacy handling: detect `accessBehavior`, surface guidance to System Settings
- [x] Linux X11/XWayland capture: `x11rb` selection ConvertSelection + property read (compile-checked only; XFixes event watch not yet wired)
- [x] Linux X11/XWayland inject: become selection owner, serve requests (inline; INCR not implemented)
- [x] Linux Wayland capture/inject: `wl-clipboard-rs` (runtime-verified on omarchy / Hyprland)
- [x] Sensitive-type filtering: NSPasteboard.org concealed/transient/auto-generated, `com.apple.is-remote-clipboard`, `x-kde-passwordManagerHint` — default deny

## Phase 1 — deferred / tech debt

- [ ] X11 INCR large-payload transfer (read and write), currently skipped/inline-only
- [ ] Event-driven capture: X11 XFixes `select_selection_input` and Wayland data-control offers, replacing content-hash polling
- [ ] Wayland multi-item semantics (currently all items flattened into one `copy_multi` offer)

## Phase 2 — Net + crypto

- [x] Identity: X25519 static keypair, `DeviceId` = BLAKE3(pubkey), persisted 0600 at the config dir
- [x] Noise IK session over length-prefixed TCP frames (`snow`), with pinned-key peer verification
- [x] Peer connection manager: both sides dial, duplicate connections resolved by device id, keepalive (ping + idle timeout), reconnect backoff
- [ ] mDNS discovery (`mdns-sd`, in-process — no Avahi dep) advertising a rotating ephemeral id; manual IP:port fallback (manual works today)
- [ ] Reconnect backoff jitter
- [ ] OS keychain / libsecret for the identity key (currently 0600 file); periodic rekey

## Phase 3 — Sync engine

- [x] Protocol: `Message::Clipboard(ClipboardItem)` over bincode (inline payload; announce-then-fetch deferred)
- [x] Dedup + echo suppression: recent-hash set, plus backend change-tracking so our own writes are not re-read
- [x] Conflict resolution: LWW by `Version` (Lamport counter, device-id tie-break), stale remotes dropped
- [x] Multi-device fan-out: broadcast on local change to every connected peer; per-peer send queue
- [ ] Blob streaming/chunking with per-chunk + whole-payload hash verification; inline small text (<64 KiB)
- [x] Reconnect + announce: on peer up the current clipboard is pushed, and clipboard activity wakes parked reconnect loops; offline→online re-pairs and syncs (verified mac↔omarchy)
- [x] Peer connection manager: persistent per-peer connection with exponential backoff (keepalive/jitter pending)
- [x] Headless mode (`--headless`) for servers and testing without a GUI

## Phase 4 — Pairing + tray UX

- [x] Pairing flow: Noise XX + 6-digit SAS on both devices (`continuum pair` / `pair-listen`); keys pinned to config (verified locally)
- [x] Tray menu: live status, pause sync, send clipboard now, pair (hint), quit
- [x] Config file + CLI fallback for headless / GNOME: `show-id`, `status`, `peer add/list`, `pair`, `pair-listen`
- [ ] Revoke a paired device from the CLI; re-pair-on-key-change prompt

## Phase 5 — Packaging + release

- [x] macOS: `.app` bundle script with `LSUIElement`, ad-hoc signed (verified); launch agent via `install-service`
- [x] Linux: systemd user unit via `install-service` + autostart `.desktop` template
- [x] Docs: README with usage, config, security/crypto, platform notes
- [x] CI: fmt + clippy + test workflow on macOS and Linux
- [ ] macOS: Developer ID signing + notarization, Hardened Runtime, `SMAppService`, Homebrew cask (needs signing cert)
- [ ] Linux distro packages: `.deb`/`.rpm`/AppImage/AUR (no Flatpak/Snap)
- [ ] Release artifact matrix (`dist` / GoReleaser-style), auto-update

## Phase 6 — Validation

- [x] End-to-end macOS ⇄ Linux (omarchy / Hyprland over Tailscale): plain text verified both directions
- [x] E2E formats: HTML and PNG images both directions (byte-identical hashes), incl. macOS TIFF->PNG conversion crossing to Linux
- [ ] Remaining E2E formats: multi-item selection, large payload/X11 INCR
- [ ] Latency benchmark harness (detect → write) and idle CPU/memory profiling
- [ ] Two-device conflict + echo-loop regression tests
- [ ] Runtime-verify the Linux X11 backend (only Wayland exercised so far)

## Later (post-v1)

- [ ] Clipboard history (encrypted at rest, auto-expire, keyring-sealed)
- [ ] GNOME Shell extension for native Mutter support
- [ ] QUIC transport; relay/star topology; encrypted blob cache on disk
- [ ] Auto-update (Sparkle 2 on macOS, AppImageUpdate on Linux)
