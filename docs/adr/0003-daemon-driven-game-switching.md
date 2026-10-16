# The daemon recognizes games and switches profiles

For [issue #5](https://github.com/tobytaken/prism/issues/5), automatic per-game switching is performed by the host daemon, not the app: the daemon recognizes running games and applies their associated GPU tuning profiles without the app running. Closing Prism's window quits the app, a Flatpak build can't see host processes, and ADR-0002 already holds that tuning should hold without opening Prism; the daemon runs on the host with access to every process, so it is the only place that can switch reliably in every supported setup.

Game associations belong to the user who owns the linked profile and fire only for that user's running games, so the daemon needs its own copy of each user's associations and the latest values of the linked profiles, as it does for startup profiles; the mechanism belongs to #8.

Switching follows a **running game**, never a launcher or window focus, and aims to take effect within a few seconds of the game starting rather than before its first frame, so no per-game launch hook is required.

## Considered options

- **Switch only while the app is open**: rejected because the app quits when its window closes, so switching would silently stop.
- **Add a background or tray mode to the app**: rejected because it still fails for Flatpak builds and sessions without a tray, and adds a second long-running component beside the daemon.
- **Apply profiles through per-game launch hooks, such as Steam launch options**: rejected as fragile and requiring setup per game; a short delay after start is harmless for power limits and clock offsets.
