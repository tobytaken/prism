# Prism Electron

Prism Electron is a Linux-only Electron + Rust desktop app for inspecting and configuring NVIDIA GPUs. It replaces the Chromium-hosting part of the original Tauri app while keeping the GPU logic and privileged daemon in Rust.

## Architecture

- Electron provides the Chromium shell, secure preload bridge, and Linux packaging.
- The React renderer lives under [`src`](./src).
- `prism_backend` is an unprivileged Rust sidecar started by Electron.
- `prismd` is a privileged host daemon that owns persistent GPU configuration writes.
- Native installs can read GPU inventory directly when `prismd` is unavailable.
- Flatpak installs require `prismd` for reads and writes.

## Development

Install dependencies:

```bash
bun install
```

Run the Electron app:

```bash
bun run dev
```

Run renderer tests:

```bash
bun run test
```

Run the Rust tests:

```bash
cargo test --manifest-path rust/Cargo.toml
```

## Sidecar and Daemon

Build the Electron sidecar:

```bash
bun run build:sidecar:debug
```

Build the daemon:

```bash
bun run build:daemon:release
```

Install the daemon on the host for local testing:

```bash
bash scripts/install-prismd-host.sh
```

The daemon listens on `/run/prism/prismd.sock` and stores desired GPU settings in `/var/lib/prism/state.json`. Its versioned tuning store lives in `/var/lib/prism/tuning-state.json` and holds each user's GPU tuning profiles, which only that user sees, and each GPU's current profile, which every user sees by name. The first time the app connects, it imports profiles saved by earlier versions of the app and renames its old `gpu-tuning-profiles.json` file to `gpu-tuning-profiles.json.imported`. If the tuning store can't be read, or was written by a newer version of Prism, the daemon leaves it untouched, the Performance page reports the problem, and manual tuning keeps working.

The default daemon config is:

```toml
admin_group = "wheel"
# admin_user = "your-username"
```

## Packaging

Install the required host tooling before building release artifacts:

```bash
bun install
cargo install cargo-deb cargo-generate-rpm
```

GUI package makers also require distro tooling on the host:

- `deb`: `dpkg` and `fakeroot`
- `rpm`: `rpmbuild`
- `flatpak`: `flatpak-builder` and `eu-strip`

GUI app:

```bash
bun run package:gui:deb
bun run package:gui:rpm
bun run package:gui:flatpak
```

Host daemon:

```bash
bun run package:daemon:deb
bun run package:daemon:rpm
```

## Flatpak host setup

The Flatpak build expects a host-installed `prismd` and permission to access `/run/prism`. The app does not try to install the daemon from inside the sandbox.

For Flatpak-host setups where group-based socket access is awkward, set `admin_user` in `/etc/prism/prismd.toml` and restart the daemon:

```bash
sudo systemctl restart prismd
```

## Startup behavior

Settings → Startup offers the host-wide **Off** (default), **Startup profiles** and **Restore last applied** choices. Use **Manage profiles…** in each GPU's Performance selector to choose its startup profile, or **No startup profile**. Anyone authorized to tune GPUs can change these choices; the latest change wins, and other users see the startup profile by name. A designation follows the profile's latest saved values and name, and deleting the profile clears the designation.

The host daemon applies startup profiles once per kernel boot id without the app running. It reserves each designated GPU's operation queue, waits about 30 seconds for an absent GPU, and validates the entire profile before writing anything. The systemd service signals readiness after startup completes, before the display manager and user sessions become available. Unsupported or out-of-range settings skip the whole GPU; values are never clamped. Performance shows dismissible per-GPU startup outcomes, including per-setting partial failures. Dismissals belong to the user and last for that boot.

**Restore last applied** restores, per GPU, each setting's most recent successful manual value from a profile application or live tuning change, including explicit **Use default** requests, with the same validation and waiting. A GPU keeps its previous current profile only when its values are restored in full. Before writing restored values, the daemon persists and syncs a record of the attempt in the tuning store; stopping the daemon while the host shuts down (systemd reports `stopping`) marks the boot clean, while a daemon restart mid-boot records nothing. If the previous boot attempted a restore and wasn't clean, the next boot skips the restore once and says so in the startup outcome. Startup profiles skip this guard. When a manual write reaches the GPU but its last applied values can't be recorded, the write still reports success with "This change was not recorded for startup.", and Settings marks Restore last applied as possibly stale (kept in `/var/lib/prism/last-applied-possibly-stale.json`) until that setting's next successful record.

The once-per-boot marker and outcomes live in `/run/prism/startup-state.json`. The service preserves this directory across stops and restarts. Unreadable tuning or boot state suspends startup behavior. Legacy `state.json` configurations migrate once to ownerless **Previous startup settings** profiles, with startup behavior set to **Startup profiles**. UUID keys survive absent GPUs; bus IDs and NVIDIA indices must resolve uniquely against the current inventory. Unresolved configurations are listed with their values in a per-user dismissible notice. Saved clock locks are no longer restored. The first authorized user to open Prism can take ownership with **Add to my profiles** on Performance. The original file is archived as `state.json.migrated`; schema version 6 prevents repeat migration. **Off** performs no boot-time tuning.

## Remembered fan curves

After you successfully apply a custom fan curve, Prism remembers the curve and shared or separate fan arrangement for your account and that individual GPU. Switching to Automatic or Fixed, closing Prism, reconnecting, and restarting the host retain this choice. Selecting Curve from shared control recalls the arrangement as an unapplied draft; review it and Apply to activate it. In separate controls, selecting Curve recalls that fan's curve when available and preserves other fan edits. Recalled curves must still meet the GPU's current limits. Discarded edits, failed applications, temporary automatic applications and their restorations, and normal curve output changes do not replace the remembered choice.

The daemon stores remembered curves and optional profile cooling configurations in tuning-store schema version 9, using the existing uid-and-username ownership safeguards and GPU UUIDs. Earlier stores upgrade without creating remembered choices or adding cooling to existing profiles; version 8 remembered curves are preserved.

## Games

The **Games** page links your games to your GPU tuning profiles, one profile per GPU; GPUs left at **Leave unchanged** aren't touched, and many games can share a profile. **Add Steam game…** lists fully installed apps from every library folder of your own Steam installs: Valve's package (`~/.local/share/Steam`, or the legacy `~/Steam`), Debian's `~/.steam/debian-installation`, Flatpak (`~/.var/app/com.valvesoftware.Steam`) and Snap (`~/snap/steam/common`). The daemon reads these from the caller's home directory with the caller's own file access, so you only see libraries you can read yourself, and the Flatpak app needs no extra file access. Flatpak Steam's library paths under your home are looked up in its persisted home as well. Compatibility tools (any install folder with a `toolmanifest.vdf`, such as Proton and the Steam Linux Runtime) and known non-game apps such as Steamworks Common Redistributables are left out. A game that is no longer installed keeps its links and is marked **Not installed**.

**Add running program…** lists your own processes for games outside Steam's library, including Heroic, Lutris and non-Steam shortcuts. Entries show the executable (or Wine mapped `.exe`), verified host location, launcher ancestry and relevant environment tags. Known helpers appear last. **Browse for file…** also adds a program that is not running; the Flatpak app recovers the host location from the document portal before sending it to the daemon.

Manual recognition depends on how the program runs. AppImages, scripts, virtual machines, Flatpak-packaged games and emulators are flagged because location may identify a temporary executable or a shared runtime instead of the game. Native recognition compares executable file identity; Wine/Proton recognition compares mapped-image identity. The stored host location is resolved each scan so updates that replace the file keep working. Moving or deleting it shows **Program not found** and keeps its profile links.

Manual program associations use tuning-store schema version 10; existing Steam associations migrate unchanged. An older daemon preserves this newer store and reports it as unavailable, so downgrading requires restoring a compatible store backup.

Associations and the **Switch profiles automatically while my games run** opt-in (off by default) belong to each user and are kept in the tuning store. Deleting a profile removes its links. The daemon applies linked profiles while games run and restores the actual pre-game tuning values when they end, including applied changes never saved to a profile.

Automatic switching uses the shared cooling controller to apply and restore complete cooling configurations: automatic mode, fixed targets, or full curves with their shared or separate fan arrangement. Restored curves calculate their output from the current temperature. A profile omitting cooling leaves it unchanged, unless an earlier profile in that activation wrote cooling; switching to a profile that omits it then restores the pre-game configuration immediately. Temporary automatic applications, restorations and curve output changes do not replace last applied manual tuning values or remembered curves.

Restoration baselines live in `/run/prism/switching-state.json`, stamped with the kernel boot id. Schema version 2 adds complete cooling baselines. Older versions are preserved untouched and block automatic switching for that boot rather than discard or reinterpret their restoration state; a normal host reboot clears this boot-scoped file. Unreadable or newer switching state likewise blocks automatic switching.
