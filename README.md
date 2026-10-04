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

The daemon listens on `/run/prism/prismd.sock` and stores desired GPU settings in `/var/lib/prism/state.json`. Its versioned tuning state lives in `/var/lib/prism/tuning-state.json`. If that file can't be read, or was written by a newer version of Prism, the daemon leaves it untouched, the Performance page reports the problem, and manual tuning keeps working.

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
