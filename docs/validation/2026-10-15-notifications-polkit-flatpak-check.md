# Notifications, polkit Enable and Flatpak daemon access

The 15 October 2026 pass for [issue #134](https://github.com/tobytaken/prism/issues/134) checked, on a real desktop, what CI can't: clicking a notification to raise Prism, **Enable prismd** under polkit, and a Flatpak GUI talking to a host `prismd`. Every check this KDE Plasma Wayland host could run **passes**:

- Clicking a notification raises and focuses the minimized window, natively and in the Flatpak.
- **Enable prismd** enables, starts and connects to a disabled `prismd`, and cancelling polkit's prompt changes nothing.
- The Flatpak gets tuning access as a member of the admin group and a read-only session that names the missing group otherwise. It tells *not installed* from *not running* through `/run/prism`.

One finding outside the checks has its own issue. A notification Plasma files during Do Not Disturb can't be clicked from its history ([#231](https://github.com/tobytaken/prism/issues/231)).

GNOME Wayland, Fedora, Ubuntu and a host `prismd` from the native deb or rpm **weren't tested**, because this host has none of them. [#230](https://github.com/tobytaken/prism/issues/230) carries them.

Tested revision: `f748fac7bfe8393f1346783ebfe6757649fe9026`. No production code was changed during validation. [Machine-readable evidence](2026-10-15-notifications-polkit-flatpak-check.json) preserves the observations below. Full local logs, D-Bus captures, screenshots and the helper scripts are in the ignored `artifacts/issue-134/` directory. Times are CEST.

## Host and execution

- CachyOS, kernel `7.2.10-1-cachyos`, systemd 262, KDE Plasma 6.7.5 Wayland (KWin 6.7.5). plasmashell is the notification server.
- polkit 127 with `polkit-kde-agent` 6.7.5. Its stock rule authenticates administrators as members of `wheel`, and the user `toby` is in `wheel`.
- NVIDIA GeForce RTX 5090, driver `615.78.08`, plus an AMD Radeon 610M iGPU.
- `prism` and `prismd` were built with `cargo build --locked --release -p prism -p prismd` (rustc 1.97.1).
- prismd was installed as [`scripts/install-prismd-host.sh`](../../scripts/install-prismd-host.sh) does, with the binary, unit, tmpfiles entry and `prismd.toml`, but left **disabled and stopped**, as Fedora's presets leave a new package. There's no native package for this distribution yet: the AUR package that Settings → Daemon suggests doesn't exist until [#154](https://github.com/tobytaken/prism/issues/154) publishes it.
- The Flatpak was the release binary repackaged with `flatpak build-finish` and the manifest's `finish-args`, then installed from a local repository, as in the [#129 pass](2026-10-10-skeleton-hardware-check.md).
- Driving and observing:
  - `ydotool` typed into Prism's command palette, only while a KWin script reported Prism's window active. This reached Settings → Daemon and ran **Enable prismd**, which runs the same action as its button.
  - A KWin script minimized and restored the window and read whether it was minimized and active.
  - `dbus-monitor` recorded the `org.freedesktop.Notifications` traffic.
  - The user authenticated every polkit prompt and clicked each notification popup.

## Results

| Check | Result | Evidence |
| --- | --- | --- |
| Notification click raises the minimized window on KDE (native) | **Pass** | Prism showed Games and was minimized when `prismd` was stopped at 20:22:28.5. 10 s later Prism sent *Prism lost its connection to prismd*, with a `default` action “Open Prism” and the `desktop-entry` hint. Clicking the popup at 20:22:41.19 made Plasma send `ActivationToken` `kwin-45`, then `ActionInvoked` `default`. By 20:22:41.8, KWin reported the window not minimized and active, on Settings → Daemon (“prismd is enabled, but has stopped”). |
| Notification click raises the minimized window in the Flatpak | **Pass** | Same steps, through the sandbox's D-Bus proxy with only `--talk-name=org.freedesktop.Notifications`. Stopped at 20:23:31.8, sent at 20:23:41.7, clicked at 20:23:48.26 with token `kwin-46`. By 20:23:49.1, KWin reported the window not minimized and active, on Settings → Daemon (“Start it on the host”). The signals reached the sandboxed app. |
| Notification click raises the minimized window on GNOME Wayland | **Not tested** (#230) | GNOME isn't installed on this host. |
| Enable prismd, authenticated | **Pass** | With `prismd.service` loaded but disabled and inactive, native Settings → Daemon said “prismd is installed, but isn't enabled” and offered **Enable prismd**. Running it ran `systemctl enable --now prismd.service`, and polkit asked for `org.freedesktop.systemd1.manage-unit-files` (“Authentication is required to manage system service or unit files”). After authentication, systemd enabled and started prismd, and Prism connected by itself, with no restart: “Connected to prismd”, tuning access “Yes”, “as toby, a member of group wheel”. Repeated three times, all passing. |
| Cancelling polkit's prompt is harmless | **Pass** | With prismd disabled again, **Enable prismd** brought up the prompt, and Esc dismissed it at 20:13:15. polkitd logged that authentication FAILED for `manage-unit-files`, and `systemctl` exited. prismd stayed disabled and inactive. Prism showed no error, and Settings → Daemon still read “prismd is installed, but isn't enabled” with **Enable prismd** ready. |
| Enable prismd on Fedora (presets leave it disabled) | **Not tested** (#230) | No Fedora host. The disabled, freshly installed state was reproduced on CachyOS instead. |
| Enable prismd on Ubuntu | **Not tested** (#230) | No Ubuntu host. |
| Flatpak: prismd not installed | **Pass** | With no `/run/prism` on the host, the status bar read “PRISMD · NOT INSTALLED” and “MONITORING IN-PROCESS”. Settings → Daemon said “prismd isn't installed”, gave `yay -S prismd` and `sudo systemctl enable --now prismd`, and added “Just installed it? Restart Prism.” |
| Flatpak: prismd not running | **Pass** | After prismd's files were installed, which creates `/run/prism`, and the Flatpak restarted, the status bar read “PRISMD · NOT RUNNING”. Settings → Daemon said “prismd is installed, but isn't enabled or has stopped. Start it on the host:” with `sudo systemctl enable --now prismd`. The Flatpak offers no Enable button, as intended. |
| Flatpak: admin user gets tuning access | **Pass** | With prismd running, the Flatpak connected through `/run/prism/prismd.sock`: “Connected to prismd”, tuning access “Yes”, “as toby, a member of group wheel”, prismd 0.1.0, protocol 0.1, and the status bar read “MONITORING THROUGH PRISMD”. No tuning was written. |
| Flatpak: user outside the admin group gets a read-only session | **Pass, simulated** (#230 for a real second user) | Setting `admin_group = "adm"`, a group `toby` isn't in, and starting prismd turned the running Flatpak's session read-only without a restart. The status bar read “PRISMD · NOT ALLOWED” and “MONITORING THROUGH PRISMD”. Settings → Daemon: “No tuning access. User toby isn't in group adm, so prismd lets this Prism monitor GPUs but not tune them.”, with `sudo usermod -aG adm toby` and “read-only: toby isn't in group adm”. The GPU page said “Tuning needs prismd · toby isn't in group adm · Set up”. The palette held *Pause automatic switching* back with “Needs tuning access · toby isn't in group adm”. |
| Flatpak against a prismd from the native deb or rpm | **Not tested** (#230) | The checks above used prismd's files installed by hand on CachyOS. They are the same files the packages install, but not the packages themselves. |

## Findings

**A notification filed during Do Not Disturb can't be clicked from Plasma's history (#231).** The first native run happened with Do Not Disturb on, so Plasma showed no popup and put the notification in its history. The notification was still live: prismd was down and Prism hadn't withdrawn it. But the history entry couldn't be clicked, and the bus carried no `ActionInvoked`, so Prism stayed minimized. Whether this is Plasma's rule for history entries or something Prism's hints could change isn't known yet.

**The unhealthy tuning store came from an older prismd.** Every connected screenshot shows “AUTO · BLOCKED” and one problem: “Automatic switching is suspended because Prism's tuning store can't be used.” `/var/lib/prism/tuning-state.json`, written on 9 October by an earlier prismd, has no schema version. prismd can't use it, so it treats it as an [unhealthy tuning store](../../CONTEXT.md) and preserves it untouched. The glossary defines that term only for a store that can't be read or was written by a newer Prism, so a store from an older prismd needs a place there too. This doesn't affect the checks above.

## Not covered by this pass

- GNOME Wayland, Fedora and Ubuntu, and a host prismd from the native deb or rpm (#230).
- The user without tuning access was simulated by pointing `admin_group` at a group `toby` isn't in, not by a second user's login session (#230).
- Tuning access was checked from what prismd's `Welcome` reports and what the GUI offers. No tuning write was made (#230).
- The Flatpak was a repackaged release binary, not the CI bundle from `flatpak-builder` (#154).

## Cleanup

prismd was disabled, stopped and removed: the binary, unit, tmpfiles entry, `/etc/prism` and `/run/prism`. `/var/lib/prism`, which predates the pass, was left untouched. The test Flatpak and its local remote were removed.
