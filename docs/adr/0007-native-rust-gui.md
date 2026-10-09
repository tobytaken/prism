# Prism is a native Rust GUI, in-process with the Rust library

As part of the [native rewrite map](https://github.com/tobytaken/prism-electron/issues/96), Prism replaces its Electron + React frontend with a single native Rust GUI binary, drawn by a GPU-rendered, non-webview toolkit. The main reasons are RAM usage and binary size. Electron's idle memory and bundled Chromium set a floor that no amount of renderer optimization removes, and a monitoring app meant to stay open should not weigh heavily on the host it observes. All TypeScript, Electron, and Electron Forge packaging are removed rather than kept alongside. The app is unreleased, so the change carries no compatibility obligations.

The GUI links the Rust GPU library directly and calls it in-process. The `prism_backend` sidecar and its stdio framing are retired, because they existed only to bridge Electron's Node main process to Rust. `prismd` remains a separate privileged daemon that owns persistent GPU configuration writes and tuning state, as ADR-0005 and ADR-0006 require. Its protocol may change where the new UX needs it, but the daemon/GUI process boundary is fixed by privilege, not by toolkit.

UI logic such as tuning edits, profile state, and telemetry history lives in a toolkit-agnostic Rust crate tested with plain `cargo test`. The view layer is kept thin. This contains the lock-in of whichever toolkit is chosen. At the time of writing, the candidate toolkits (gpui, which is pre-1.0 and changes often, and Slint) are still to be decided by a [spike](https://github.com/tobytaken/prism-electron/issues/102), and that choice will be recorded separately.

## Considered Options

- **Keep Electron and optimize it.** Rejected because the RAM and binary-size floor comes from Chromium itself, not from Prism's code.
- **Webview-based Rust stacks (Tauri/Wry, Dioxus-desktop).** Rejected because on Linux they render through WebKitGTK, which the project already moved away from when it replaced the original Tauri app.
- **GTK4/libadwaita (relm4, gtk4-rs).** Rejected despite being the lowest-risk native path on Linux. Prism wants its own modern, industrial visual identity rather than a GNOME-native look.
- **Keep the sidecar as a process boundary.** Rejected because it adds an IPC layer and a second binary with no privilege or isolation benefit for an unprivileged GUI.

## Consequences

- **Flatpak monitoring.** The [packaging research](https://github.com/tobytaken/prism-electron/issues/99) found that the Flatpak GUI must read GPU inventory and telemetry in-process without `prismd`, which Flathub acceptance depends on. This reverses today's daemon-only behavior under Flatpak.
- **Version handshake.** Because a sandboxed or portable GUI updates independently of the host daemon, the GUI–`prismd` protocol needs a version handshake.
