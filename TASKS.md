# Continuum — Task List

Cross-platform, tray-only clipboard sync between macOS and Linux over the LAN.
Rust. TCP + Noise IK (snow). mDNS discovery. Pinned identities. LAN-only, no cloud.

Legend: `[ ]` todo · `[~]` in progress · `[x]` done · blocked items note the blocker.

## Phase 0 — Scaffolding

- [x] Init git repo, Cargo workspace layout (`crates/`: `continuum-core`, `continuum-platform`, `continuum-net`, `continuum-app`), `.gitignore`, rustfmt/clippy config
- [x] Define `ClipboardItem` model: multi-item, MIME/UTI map, BLAKE3 content hash, origin id, Lamport timestamp
- [x] Tray skeleton (`tray-icon` + `muda`) with placeholder menu and accessory (no-Dock) activation policy; real `.app`/`LSUIElement` bundle deferred to Phase 5
- [ ] NOTE: `tray-icon` 0.24 has no `ksni` backend — Linux path is GTK + libappindicator/appindicator. Decide Linux tray backend (tray-icon w/ GTK vs. direct `ksni`) before Phase 4.

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
- [ ] Peer connection manager: one persistent connection per peer, keepalive, exponential backoff with jitter (wired in Phase 3)
- [ ] mDNS discovery (`mdns-sd`, in-process — no Avahi dep) advertising a rotating ephemeral id; manual IP:port fallback (manual works today)
- [ ] OS keychain / libsecret for the identity key (currently 0600 file); periodic rekey

## Phase 3 — Sync engine

- [x] Protocol: `Message::Clipboard(ClipboardItem)` over bincode (inline payload; announce-then-fetch deferred)
- [x] Dedup + echo suppression: recent-hash set, plus backend change-tracking so our own writes are not re-read
- [x] Conflict resolution: LWW by `Version` (Lamport counter, device-id tie-break), stale remotes dropped
- [x] Multi-device fan-out: broadcast on local change to every connected peer; per-peer send queue
- [ ] Blob streaming/chunking with per-chunk + whole-payload hash verification; inline small text (<64 KiB)
- [ ] Reconnect catch-up: current clipboard pulled on (re)connect
- [x] Peer connection manager: persistent per-peer connection with exponential backoff (keepalive/jitter pending)
- [x] Headless mode (`--headless`) for servers and testing without a GUI

## Phase 4 — Pairing + tray UX

- [ ] Pairing flow: short code / QR (SPAKE2) → exchange + pin static keys; explicit revoke; re-pair on key change
- [ ] Tray menu: status, pause sync, pair device, send clipboard now, quit
- [ ] Config file + CLI fallback for headless / GNOME-without-extension operation

## Phase 5 — Packaging + release

- [ ] macOS: signed + notarized `.app`, Hardened Runtime, `SMAppService` login item, Homebrew cask
- [ ] Linux: systemd user unit + `.desktop` autostart, `.deb`/`.rpm`/AppImage/AUR (no Flatpak/Snap)
- [ ] CI release matrix (`dist`) building on `macos-latest` + `ubuntu-latest`
- [ ] Docs: README, threat model, crypto spec, GNOME-Wayland limitation note

## Phase 6 — Validation

- [x] End-to-end macOS ⇄ Linux (omarchy / Hyprland over Tailscale): plain text verified both directions
- [ ] Remaining E2E formats: rich text, image, multi-item, large payload
- [ ] Latency benchmark harness (detect → write) and idle CPU/memory profiling
- [ ] Two-device conflict + echo-loop regression tests
- [ ] Runtime-verify the Linux X11 backend (only Wayland exercised so far)

## Later (post-v1)

- [ ] Clipboard history (encrypted at rest, auto-expire, keyring-sealed)
- [ ] GNOME Shell extension for native Mutter support
- [ ] QUIC transport; relay/star topology; encrypted blob cache on disk
- [ ] Auto-update (Sparkle 2 on macOS, AppImageUpdate on Linux)
