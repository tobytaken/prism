# Prism quits when its window closes and has no background presence

[Does Prism keep running in the background, and who notifies the user?](https://github.com/tobytaken/prism-electron/issues/117) decided that closing Prism's window quits the app. There is no tray icon, no hidden window and no session agent, and Prism has no setting to start with the session. Tuning doesn't need the GUI: `prismd` applies the startup behavior at boot (ADR-0002) and does automatic switching (ADR-0003). Activity and the status bar show what happened while Prism was closed (ADR-0010). A background GUI would only add convenience and notifications, and the [research](https://github.com/tobytaken/prism-electron/issues/116) found it saves nothing. A hidden Slint window uses as much memory as a visible one (~61 MiB PSS). RAM is the main reason for the rewrite, so the GUI either runs with its window open or minimized, or doesn't run at all.

Only the running GUI sends desktop notifications, through `org.freedesktop.Notifications` (`notify-rust`, with `--talk-name=org.freedesktop.Notifications` in Flatpak). They cover three events: a failure pause (including a cooling fault), a driver job ending, and a `prismd` connection that is still lost after about 10 s. There are none while Prism's window has focus, and one Settings switch turns them off. Clicking a notification raises Prism at the relevant place. Notifications have no write actions, because Retry and Resume need tuning access and belong where the state is visible. Prism is single-instance through the D-Bus name `io.github.tobytaken.Prism`, so launching it again raises the existing window.

## Considered Options

- **Optional or default tray mode (close hides to tray).** Rejected because a hidden window keeps its full footprint. Stock GNOME has no tray host, so Prism would need to detect that and fall back. Slint's own tray won't register on Flathub. A hidden GUI also keeps its `prismd` session open, so its edit holds would stay alive. This also means ADR-0008's "a tray icon" benefit goes unused.
- **A windowless session agent that notifies while the GUI is closed (~2 MiB).** Deferred, not rejected. It adds a second long-running component and its own autostart rules for native installs and Flatpak. Read-only sessions (ADR-0011) already give it everything it needs, so it can be added later without changing `prismd`.
- **An "Open Prism at login" setting.** Rejected because it only opens a window that can't reliably start minimized on Wayland. The Flatpak autostart portal is missing on COSMIC and wlroots, and the desktop's own autostart settings already cover anyone who wants it.

## Consequences

- Closing with unapplied tuning changes asks "Discard and close / Keep editing". Quitting ends the session and its edit hold.
- Problems that happen while Prism is closed reach the user only the next time Prism opens, through Activity and the status bar.
- On Wayland, Prism can only raise its window, when a notification is clicked or Prism is launched again, by passing on the XDG activation token it receives (from the notification server or the second launch). Without that token GNOME may not focus the window, so the build plan has to verify this.
