# Prism

Prism is a Linux GPU control center for watching, tuning and managing NVIDIA GPUs and their drivers. It is being rewritten as a native Rust GUI built with Slint ([ADR 0007](docs/adr/0007-native-rust-gui.md), [ADR 0008](docs/adr/0008-slint-gui-toolkit.md), [#120](https://github.com/tobytaken/prism-electron/issues/120)). The Electron app has been removed; this repository currently holds the Rust GPU library and `prismd`.

Prism is licensed under the GNU General Public License, version 3 or later (`GPL-3.0-or-later`); see [`LICENSE`](./LICENSE).

## Architecture

- `prismd` is a privileged host daemon that owns all tuning writes and tuning state.
- The Cargo workspace keeps its crates under [`crates`](./crates). The dependency graph runs one way: `prism-gpu ← prism-drivers ← prism-protocol ← prismd`.
  - [`prism-gpu`](./crates/prism-gpu) reads and tunes GPUs through NVML. It holds fan curves and cooling, and knows nothing of `prismd`.
  - [`prism-drivers`](./crates/prism-drivers) detects the host and its NVIDIA driver, plans driver installs, and runs driver jobs.
  - [`prism-protocol`](./crates/prism-protocol) holds the `prismd` socket's wire types, framing and client, and the shared contract fixtures.
  - [`prismd`](./crates/prismd) is the daemon, along with its tests and its deb/rpm packaging metadata.
- The native GUI will link these crates in-process.

## Development

Run the checks CI runs:

```bash
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo deny check licenses
```

CI runs only when started manually. In GitHub, open **Actions → CI → Run workflow**, select a branch, and start the workflow to run formatting, Clippy, tests, the license check and the `prismd` package smoke builds.

`cargo deny` fails when a dependency's license isn't on the GPLv3-compatible allow-list in [`deny.toml`](./deny.toml). [`THIRD-PARTY-LICENSES`](./THIRD-PARTY-LICENSES) lists the licenses of every dependency; after changing `Cargo.lock`, regenerate it with [`cargo-about`](https://github.com/EmbarkStudios/cargo-about) and commit the result:

```bash
bash scripts/generate-third-party-licenses.sh
```

## Daemon

Build the daemon:

```bash
cargo build --release -p prismd
```

Install the daemon on the host for local testing:

```bash
bash scripts/install-prismd-host.sh
```

The daemon listens on `/run/prism/prismd.sock`. Its versioned tuning store lives in `/var/lib/prism/tuning-state.json` and holds each user's GPU tuning profiles, which only that user sees, and each GPU's current profile, which every user sees by name. If the tuning store can't be read, or was written by a newer version of Prism, the daemon leaves it untouched, reports the problem, and manual tuning keeps working.

The default daemon config is:

```toml
admin_group = "wheel"
# admin_user = "your-username"
```

For setups where group-based socket access is awkward, such as a Flatpak app, set `admin_user` in `/etc/prism/prismd.toml` and restart the daemon:

```bash
sudo systemctl restart prismd
```

## Packaging

Install the packaging tools:

```bash
cargo install cargo-deb cargo-generate-rpm
```

Build the `prismd` packages:

```bash
bash scripts/package-prismd-deb.sh
bash scripts/package-prismd-rpm.sh
```

## Daemon behavior

The sections below describe what the daemon does. Their references to pages and controls describe the removed Electron app; the native GUI will expose the same behavior.

### Startup behavior

Settings → Startup offers the host-wide **Off** (default), **Startup profiles** and **Restore last applied** choices. Use **Manage profiles…** in each GPU's Performance selector to choose its startup profile, or **No startup profile**. Anyone authorized to tune GPUs can change these choices; the latest change wins, and other users see the startup profile by name. A designation follows the profile's latest saved values and name, and deleting the profile clears the designation.

The host daemon applies startup profiles once per kernel boot id without the app running. It reserves each designated GPU's operation queue, waits about 30 seconds for an absent GPU, and validates the entire profile before writing anything. The systemd service signals readiness after startup completes, before the display manager and user sessions become available. Unsupported or out-of-range settings skip the whole GPU; values are never clamped. Performance shows dismissible per-GPU startup outcomes, including per-setting partial failures. Dismissals belong to the user and last for that boot.

**Restore last applied** restores, per GPU, each setting's most recent successful manual value from a profile application or live tuning change, including explicit **Use default** requests, with the same validation and waiting. A GPU keeps its previous current profile only when its values are restored in full. Before writing restored values, the daemon persists and syncs a record of the attempt in the tuning store; stopping the daemon while the host shuts down (systemd reports `stopping`) marks the boot clean, while a daemon restart mid-boot records nothing. If the previous boot attempted a restore and wasn't clean, the next boot skips the restore once and says so in the startup outcome. Startup profiles skip this guard. When a manual write reaches the GPU but its last applied values can't be recorded, the write still reports success with "This change was not recorded for startup.", and Settings marks Restore last applied as possibly stale (kept in `/var/lib/prism/last-applied-possibly-stale.json`) until that setting's next successful record.

The once-per-boot marker and outcomes live in `/run/prism/startup-state.json`. The service preserves this directory across stops and restarts. Unreadable tuning or boot state suspends startup behavior. **Off** performs no boot-time tuning.

### Remembered fan curves

After you successfully apply a custom fan curve, Prism remembers the curve and shared or separate fan arrangement for your account and that individual GPU. Switching to Automatic or Fixed, closing Prism, reconnecting, and restarting the host retain this choice. Selecting Curve from shared control recalls the arrangement as an unapplied draft; review it and Apply to activate it. In separate controls, selecting Curve recalls that fan's curve when available and preserves other fan edits. Recalled curves must still meet the GPU's current limits. Discarded edits, failed applications, temporary automatic applications and their restorations, and normal curve output changes do not replace the remembered choice.

The daemon stores remembered curves and optional profile cooling configurations in tuning-store schema version 9, using the existing uid-and-username ownership safeguards and GPU UUIDs. Earlier stores upgrade without creating remembered choices or adding cooling to existing profiles; version 8 remembered curves are preserved.

### Games

**Automatic switching is experimental and not yet validated for release.** The
[9 October live validation](docs/validation/2026-10-09-nvidia-game-recognition.md)
found recognition failures in distribution-package Steam, both native and Proton.
The [repair replay](docs/validation/2026-10-09-nvidia-game-recognition-repair.md)
subsequently passed recognition for native Oxygen Not Included and Proton Balatro
on the tested host, including their manually identified program forms. This does
not validate automatic GPU tuning. Required native and Proton forms of Flatpak
Steam remain unvalidated.
[Issue #49](https://github.com/tobytaken/prism-electron/issues/49) stays open until
all required forms pass, including actual automatic application and restoration.
The paths and matching mechanisms below describe the implementation, not verified
support. Snap Steam, gamescope, plain Wine, umu, Heroic, Lutris and document-portal
host-path recovery remain unclaimed without their own live evidence. This gate
does not establish or invalidate profile management or startup behavior.

The **Games** page links your games to your GPU tuning profiles, one profile per GPU; GPUs left at **Leave unchanged** aren't touched, and many games can share a profile. **Add Steam game…** lists fully installed apps from every library folder of your own Steam installs: Valve's package (`~/.local/share/Steam`, or the legacy `~/Steam`), Debian's `~/.steam/debian-installation`, Flatpak (`~/.var/app/com.valvesoftware.Steam`) and Snap (`~/snap/steam/common`). The daemon reads these from the caller's home directory with the caller's own file access, so you only see libraries you can read yourself, and the Flatpak app needs no extra file access. Flatpak Steam's library paths under your home are looked up in its persisted home as well. Compatibility tools (any install folder with a `toolmanifest.vdf`, such as Proton and the Steam Linux Runtime) and known non-game apps such as Steamworks Common Redistributables are left out. A game that is no longer installed keeps its links and is marked **Not installed**.

**Add running program…** lists your own processes for games outside Steam's library, including Heroic, Lutris and non-Steam shortcuts. Entries show the executable (or Wine mapped `.exe`), verified host location, launcher ancestry and relevant environment tags. Known helpers appear last. **Browse for file…** also adds a program that is not running; the Flatpak app recovers the host location from the document portal before sending it to the daemon.

Manual recognition depends on how the program runs. AppImages, scripts, virtual machines, Flatpak-packaged games and emulators are flagged because location may identify a temporary executable or a shared runtime instead of the game. Native recognition compares executable file identity; Wine/Proton recognition compares mapped-image identity. The stored host location is resolved each scan so updates that replace the file keep working. Moving or deleting it shows **Program not found** and keeps its profile links.

Manual program associations use tuning-store schema version 10; existing Steam associations migrate unchanged. An older daemon preserves this newer store and reports it as unavailable, so downgrading requires restoring a compatible store backup.

Associations and the **Switch profiles automatically while my games run** opt-in (off by default) belong to each user and are kept in the tuning store. Deleting a profile removes its links. The daemon applies linked profiles while games run and restores the actual pre-game tuning values when they end, including applied changes never saved to a profile.

Automatic switching uses the shared cooling controller to apply and restore complete cooling configurations: automatic mode, fixed targets, or full curves with their shared or separate fan arrangement. Restored curves calculate their output from the current temperature. A profile omitting cooling leaves it unchanged, unless an earlier profile in that activation wrote cooling; switching to a profile that omits it then restores the pre-game configuration immediately. Temporary automatic applications, restorations and curve output changes do not replace last applied manual tuning values or remembered curves.

Restoration baselines live in `/run/prism/switching-state.json`, stamped with the kernel boot id. Schema version 3 adds expected numeric readings and manual overrides alongside complete cooling baselines. Older versions are preserved untouched and block automatic switching for that boot rather than discard or reinterpret their restoration state; a normal host reboot clears this boot-scoped file. Unreadable or newer switching state likewise blocks automatic switching.

A manual profile application or live tuning attempt (including cooling) while linked games run overrides automation on that GPU, even if the attempt fails. An automatic operation already writing finishes its setters; waiting automatic operations are dropped. The resulting manual values remain after the games end. Saving a profile is not a tuning attempt; a save based on values shown before an automatic switch is rejected for review.

Before an automatic application or restoration, the daemon compares fresh readings with its last observations of the controls it manages. An observed difference keeps the values and reports **Manual override · changed outside Prism** in the Game tuning row and GPU header. Numeric controls are checked at these operations, without continuous monitoring. Cooling uses the shared controller's observable fan policy; physical speed does not reveal an independent manual target, and ordinary curve output changes are not external edits. An observable external cooling change suspends curve control, preserves its definition and leaves external controls alone until explicit manual cooling application.
