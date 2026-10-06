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

## Games

The **Games** page links your Steam games to your GPU tuning profiles, one profile per GPU; GPUs left at **Leave unchanged** aren't touched, and many games can share a profile. **Add Steam game…** lists fully installed apps from every library folder of your own Steam installs: Valve's package (`~/.local/share/Steam`, or the legacy `~/Steam`), Debian's `~/.steam/debian-installation`, Flatpak (`~/.var/app/com.valvesoftware.Steam`) and Snap (`~/snap/steam/common`). The daemon reads these from the caller's home directory with the caller's own file access, so you only see libraries you can read yourself, and the Flatpak app needs no extra file access. Flatpak Steam's library paths under your home are looked up in its persisted home as well. Compatibility tools (any install folder with a `toolmanifest.vdf`, such as Proton and the Steam Linux Runtime) and known non-game apps such as Steamworks Common Redistributables are left out. A game that is no longer installed keeps its links and is marked **Not installed**.

Associations and the **Switch profiles automatically while my games run** opt-in (off by default) belong to each user and are kept in the tuning store (schema version 7). Deleting a profile removes its links. Prism doesn't switch profiles for games yet.
