# Daemon state reaches the GUI as events, and tuning is never written in-process

[How do telemetry and daemon state flow into the Slint UI?](https://github.com/tobytaken/prism-electron/issues/107) decided how GPU telemetry and `prismd`'s state reach the Slint GUI ([ADR-0007](0007-native-rust-gui.md), [ADR-0008](0008-slint-gui-toolkit.md)). When a `prismd` session is connected, telemetry comes from the daemon. The GUI samples NVML in-process only when there is no session, for example a Flatpak or AppImage without a host daemon, or a stopped daemon. NVML is loaded lazily, so on a normal native install it is never loaded into the GUI. This keeps the GUI small, which is the main reason for the rewrite. It also keeps Flatpak monitoring working when the `GL.nvidia` extension doesn't match the host driver.

On connect, the GUI takes one snapshot of daemon state. After that it changes only through pushed session events, each carrying the full new value for its scope. The GUI never polls. Telemetry is an opt-in `TelemetrySampled` event at 1 Hz, read once per tick and shared by every subscribed session. Events are the only source of state; a command's response only feeds what is shown to the user (outcomes, corrections, Activity). The `prism-ui` core lives on the UI thread. The session reader, the in-process sampler and the effect workers are std threads that post into it with `slint::invoke_from_event_loop`, so nothing wakes the event loop on a timer.

Tuning is only ever written through `prismd`. Without a session, Prism has no **tuning access** and every control is read-only, with the reason shown. There is no in-process write path, not even for a GUI run as root, because it would bypass the tuning store, edit holds and automatic switching that ADR-0005 makes the daemon own. Since edits are staged, a write is always an Apply: one `ApplyProfile` request per GPU, which carries the tuning-state revision the draft was based on and the fans reviewed for cooling. The per-control live requests are removed.

## Considered Options

- **Always sample in-process.** This would mean one telemetry path. Rejected because it loads `libnvidia-ml` into the GUI on every host and breaks Flatpak telemetry when the NVIDIA extension doesn't match the driver, even with a working daemon.
- **Daemon-only on native packages, in-process only in Flatpak and AppImage.** Rejected because a native install with a stopped daemon would lose monitoring, which the UX foundations rule out.
- **Poll the daemon, as the Electron app did (every 0.5–1 s).** Rejected because polling wakes both processes when nothing has changed, and a late response can overwrite newer state.
- **In-process tuning writes when the GUI has privileges.** Rejected because it splits tuning state between two writers.

## Consequences

- **Two telemetry paths.** The in-process sampler and the daemon subscription produce the same core inputs, except cooling-control state, which only the daemon has. Samples are timestamped when the GUI receives them, so history stays one continuous series when the source changes.
- **The daemon keeps a short activity history.** It keeps a boot-scoped list of recent events (profile applications, automatic switches, startup outcomes) so the Activity panel can show what happened while Prism was closed.
- **Protocol churn.** `GetPerformance` and the seven per-control live requests are removed. New events cover telemetry, cooling control, profiles, startup state, games, inventory, driver jobs and activity. The version handshake (ADR-0007) is decided in [How does Prism onboard a host prismd and handshake its protocol version?](https://github.com/tobytaken/prism-electron/issues/112).
