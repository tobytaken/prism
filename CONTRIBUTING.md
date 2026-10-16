# Contributing to Prism

This guide covers Prism's architecture, building and testing it, and packaging and releasing it. [README.md](./README.md) is for users; [docs/prismd.md](./docs/prismd.md) describes what the daemon does.

## Architecture

The ADRs in [`docs/adr`](./docs/adr) record why Prism is built this way, and [`CONTEXT.md`](./CONTEXT.md) defines its vocabulary.

- `prismd` is a privileged host daemon that owns all tuning writes and tuning state.
- `prism` is the GUI. It reads GPU telemetry in-process through NVML and, later, from `prismd`, but never writes tuning itself.
- The Cargo workspace keeps its crates under [`crates`](./crates). The dependency graph runs one way: `prism-gpu ← prism-drivers ← prism-protocol ← prismd`, and `prism-gpu ← prism-ui ← prism`.
  - [`prism-gpu`](./crates/prism-gpu) reads and tunes GPUs through NVML. It holds fan curves and cooling, and knows nothing of `prismd`. Callers choose the providers GPUs are read from; its `simulated` feature adds simulated GPUs with named fixtures for development and tests, which release builds leave out.
  - [`prism-drivers`](./crates/prism-drivers) detects the host and its NVIDIA driver, plans driver installs, and runs driver jobs.
  - [`prism-protocol`](./crates/prism-protocol) holds the `prismd` socket's wire types, framing and client, and the shared contract fixtures.
  - [`prismd`](./crates/prismd) is the daemon, along with its tests and its deb/rpm packaging metadata.
  - [`prism-ui`](./crates/prism-ui) is the GUI's logic as a pure state machine, with no Slint, threads or I/O. It takes inputs (telemetry samples, the GPU inventory, user intents) and returns the effects to perform. It also keeps the telemetry history and formats what the view shows.
  - [`prism`](./crates/prism) is the GUI binary. It has the Slint view (`ui/`, with every visual token in `ui/theme.slint`), the 1 Hz sampler thread, the composition root, which takes an `Environment` of GPU providers, settings store and clock so tests and the simulator can swap them, and the GUI's deb/rpm packaging metadata.

## Development

Run the checks CI runs:

```bash
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo deny check licenses
```

CI runs the screenshot tests in their own job, in a pinned container image (see [GUI](#gui)). The commands above run them on this host, where they usually match too.

CI runs only when started manually, or on a release tag (see [Releases](#releases)). In GitHub, open **Actions → CI → Run workflow**, select a branch, and start the workflow to run formatting, Clippy, tests, the license check, shellcheck and the package builds in all five formats, which the run keeps as artifacts: the `prismd` and `prism-gpu` debs and rpms, the AUR packages, the AppImage and the Flatpak bundle. The package jobs also check that no release build enables the `simulated` feature and that the metainfo and desktop file validate. [`scripts/check-prism-package.sh`](./scripts/check-prism-package.sh) checks what each GUI package installs and its license field. [`scripts/check-prism-install.sh`](./scripts/check-prism-install.sh) installs the GUI's deb, rpm and Arch package on Ubuntu 22.04, Debian 12, AlmaLinux 8, Fedora and Arch, and checks every library `prism` links or loads is there. [`scripts/check-prismd-install.sh`](./scripts/check-prismd-install.sh) installs and removes the `prismd` packages on Ubuntu 24.04, Debian 12, AlmaLinux 8, Fedora and Arch, each booted with systemd by [`scripts/run-in-systemd-container.sh`](./scripts/run-in-systemd-container.sh), and checks the service is left as [Packaging](#packaging) describes. That script never uses `--privileged`: a privileged container's systemd-logind can suspend the host.

`cargo deny` fails when a dependency's license isn't on the GPLv3-compatible allow-list in [`deny.toml`](./deny.toml). [`THIRD-PARTY-LICENSES`](./THIRD-PARTY-LICENSES) lists the licenses of every dependency and embedded font; after changing `Cargo.lock`, regenerate it with [`cargo-about`](https://github.com/EmbarkStudios/cargo-about) and commit the result:

```bash
bash scripts/generate-third-party-licenses.sh
```

Building the GUI needs `pkg-config` and the fontconfig development headers (`libfontconfig-dev` on Debian and Ubuntu, `fontconfig-devel` on Fedora).

## GUI

Run the GUI on this host's NVIDIA GPUs:

```bash
cargo run -p prism
```

Run it on simulated GPUs instead, with one of the fixtures `tunable`, `two-identical`, `no-fan-control`, `edge-ranges`, `nvml-missing`, `empty-inventory` or `driver-not-loaded`:

```bash
PRISM_SIMULATE=tunable cargo run -p prism --features simulated
```

Light and dark follow the desktop. To pin one, set `appearance = "light"` or `appearance = "dark"` in `~/.config/prism/settings.toml` (`$XDG_CONFIG_HOME/prism/settings.toml`). Settings → Appearance writes the same setting.

The view tests in `crates/prism/tests` drive the real window headless with Slint's testing backend, so they need no display. They find elements through Slint's debug info, which `prism`'s build script compiles in outside release builds.

The accessibility audit in `crates/prism/tests/accessibility.rs` walks the window in every simulated fixture state. It fails when an interactive element has no label, a spinbox or slider has no value or range, a switch or checkbox has no checked state, or a tuning control stays editable without tuning access. Each accessible action a widget declares needs a test that invokes it.

The screenshot tests in `crates/prism/tests/screenshots.rs` draw the window with Slint's software renderer in light and dark and compare it against the reference PNGs in `crates/prism/tests/screenshots/`. CI runs them in one pinned container image. A mismatch saves the new screenshot under `target/tmp/screenshots/`. After an intended visual change, or a Slint bump, regenerate the references in that image with Docker and commit them:

```bash
bash scripts/update-screenshots.sh          # regenerate the references
bash scripts/update-screenshots.sh --check  # or only run the screenshot tests there
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

It enables `prismd.service` and starts it, or restarts it if it's already running, so rerunning the script after a rebuild runs the new daemon. If reloading systemd, enabling or starting fails, it says so and exits non-zero; `systemctl status prismd` and `journalctl -u prismd` show why.

[docs/prismd.md](./docs/prismd.md) describes the socket, tuning access, the daemon config and what the daemon does at startup and while games run.

## Packaging

Release builds use the [toolkit spike](https://github.com/tobytaken/prism/issues/102)'s size profile: fat LTO, one codegen unit and stripped symbols. Unlike the spike's, panics unwind instead of aborting, because `prismd` catches a panicking GPU operation so it fails only its own request and the daemon keeps serving. Cargo's per-package overrides can't set `panic`, and aborting only in the GUI would take a second profile that every build and packaging script would have to name, for 2.9 MiB.

Prism ships in five formats. The GUI is `prism-gpu` and the daemon `prismd`, as separate deb, rpm and AUR packages; the AppImage and the Flatpak hold only the GUI, which needs a host `prismd` from one of those packages to tune. Every package installs the desktop file [`contrib/io.github.tobytaken.Prism.desktop`](./contrib/io.github.tobytaken.Prism.desktop), the AppStream metainfo [`contrib/io.github.tobytaken.Prism.metainfo.xml`](./contrib/io.github.tobytaken.Prism.metainfo.xml) and the icon [`assets/io.github.tobytaken.Prism.svg`](./assets/io.github.tobytaken.Prism.svg), all named after the app ID `io.github.tobytaken.Prism`, along with `LICENSE` and `THIRD-PARTY-LICENSES`. Each gives `GPL-3.0-or-later` in its license field; the deb in its copyright file, and the AppImage and Flatpak in the metainfo.

| Format | GUI | Daemon | `prismd.service` after install |
| --- | --- | --- | --- |
| deb | `prism-gpu`, recommends `prismd` | `prismd`; `/etc/prism/prismd.toml` is a conffile | enabled and started |
| rpm | `prism-gpu`, recommends `prismd` | `prismd`; the config is `noreplace` | as the presets say: disabled on Fedora |
| AUR | `prism-gpu`, optionally depends on `prismd` | `prismd`; the config is a backup file | not enabled; the install says how |
| AppImage | yes | no | |
| Flatpak | yes | no | |

The `prismd` packages also create `/run/prism` through `tmpfiles.d`. The rpm's scriptlets in [`contrib/rpm/prismd`](./contrib/rpm/prismd) mirror systemd's `%systemd_post`, `%systemd_preun` and `%systemd_postun_with_restart`, since cargo-generate-rpm has none.

Install the packaging tools:

```bash
cargo install cargo-deb cargo-generate-rpm
```

The deb, rpm and AppImage repackage two release binaries, which CI builds in an AlmaLinux 8 container so they run on glibc 2.28 and newer. Build them there, or with `cargo build --locked --release -p prism -p prismd` for this host only, then package them:

```bash
bash scripts/package-prismd-deb.sh
bash scripts/package-prismd-rpm.sh
bash scripts/package-prism-deb.sh
bash scripts/package-prism-rpm.sh
bash scripts/package-prism-appimage.sh
```

The AUR's split PKGBUILD, [`contrib/aur/PKGBUILD`](./contrib/aur/PKGBUILD), builds both packages from source per the Arch Rust guidelines. [`scripts/render-aur-pkgbuild.sh`](./scripts/render-aur-pkgbuild.sh) writes it to `target/aur` with the version and the checksum of a source tarball it makes from a git ref with `git archive`, which includes `Cargo.lock`. [`scripts/build-aur-package.sh`](./scripts/build-aur-package.sh) builds it with `makepkg`, as root in a clean Arch container, and writes `.SRCINFO`:

```bash
bash scripts/render-aur-pkgbuild.sh
docker run --rm -v "$PWD:/src" -w /src archlinux:latest bash scripts/build-aur-package.sh
```

The AppImage bundles no libraries, so GL, Vulkan, the NVIDIA libraries and glibc come from the host. The Flatpak builds from source per its manifest; its script needs `flatpak-builder` and Python with `aiohttp` and `tomlkit`, as in the `ghcr.io/flathub-infra/flatpak-github-actions:freedesktop-26.08` image CI uses:

```bash
bash scripts/package-prism-flatpak.sh
```

The manifest builds this checkout. [`scripts/render-flathub-manifest.sh`](./scripts/render-flathub-manifest.sh) writes the one to submit to Flathub to `target/flathub`, building the release tag's commit, beside the generated `cargo-sources.json`.

### Releases

Pushing a tag `vX.Y.Z` runs the [release workflow](./.github/workflows/release.yml). It checks that the tag names the workspace version and that the metainfo's newest `<release>` is that version. Then it runs CI on the tag and publishes a GitHub release with every package, the source tarball the AUR PKGBUILD downloads, the AUR and Flathub files, and their checksums. The release notes name the commit it was built from. If any CI job fails, nothing is published.

To release version `X.Y.Z`:

1. Run CI on `main` (**Actions → CI → Run workflow**) and check that every job passes.
2. On a branch, set `version` under `[workspace.package]` in [`Cargo.toml`](./Cargo.toml) and run `cargo update --workspace` to update `Cargo.lock`. At the top of `<releases>` in [`contrib/io.github.tobytaken.Prism.metainfo.xml`](./contrib/io.github.tobytaken.Prism.metainfo.xml), add a `<release version="X.Y.Z" date="YYYY-MM-DD">` entry with its details URL and notes. `bash scripts/check-release-version.sh vX.Y.Z` checks the two agree. Merge the branch.
3. Tag the merged commit on `main` and push the tag:

   ```bash
   git tag -a vX.Y.Z -m "Prism X.Y.Z"
   git push origin vX.Y.Z
   ```

4. Follow the run under **Actions → Release**, or with `gh run watch`. When it finishes, the release is on the repository's Releases page.

If the run fails, fix the cause on `main`, then delete the tag (`git push --delete origin vX.Y.Z` and `git tag -d vX.Y.Z`) and tag again. A published release stays published, so a fix after it needs a new version.

The metainfo's URLs, its screenshots and the AUR's source only resolve once the repository is public.
