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
- [x] Linux Wayland capture/inject: `wl-clipboard-rs`, `ext-data-control`/`wlr-data-control` via the crate
- [x] Sensitive-type filtering: NSPasteboard.org concealed/transient/auto-generated, `com.apple.is-remote-clipboard`, `x-kde-passwordManagerHint` — default deny

## Phase 1 — deferred / tech debt

- [ ] X11 INCR large-payload transfer (read and write), currently skipped/inline-only
- [ ] Event-driven capture: X11 XFixes `select_selection_input` and Wayland data-control offers, replacing content-hash polling
- [ ] Wayland multi-item semantics (currently all items flattened into one `copy_multi` offer)

## Phase 2 — Net + crypto

- [ ] Identity: generate Ed25519 identity + X25519 static key; store in macOS Keychain / libsecret; 0600 key-file fallback
- [ ] Noise IK session over length-prefixed TCP frames (`snow`), with replay window and periodic rekey
- [ ] Peer connection manager: one persistent connection per peer, keepalive, exponential backoff with jitter
- [ ] mDNS discovery (`mdns-sd`, in-process — no Avahi dep) advertising a rotating ephemeral id; manual IP:port fallback

## Phase 3 — Sync engine

- [ ] Announce-then-fetch protocol: announce {hash, MIME, size, Lamport, origin}, stream bytes on demand
- [ ] Dedup + echo suppression: content-hash seen-set with TTL, skip own-injected changeCount/generation
- [ ] Conflict resolution: last-writer-wins with Lamport clock, deterministic device-id tie-break
- [ ] Blob streaming/chunking with per-chunk + whole-payload hash verification; inline small text (<64 KiB)
- [ ] Multi-device mesh fan-out and reconnect catch-up via Lamport state

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

- [ ] End-to-end test macOS ⇄ Linux: text, rich text, image, multi-item, large payload
- [ ] Latency benchmark harness (detect → write) and idle CPU/memory profiling
- [ ] Two-device conflict + echo-loop regression tests

## Later (post-v1)

- [ ] Clipboard history (encrypted at rest, auto-expire, keyring-sealed)
- [ ] GNOME Shell extension for native Mutter support
- [ ] QUIC transport; relay/star topology; encrypted blob cache on disk
- [ ] Auto-update (Sparkle 2 on macOS, AppImageUpdate on Linux)
