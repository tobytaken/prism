# Packaging Prism and prismd: Flatpak, deb, rpm, AppImage, AUR

Research for [#99](https://github.com/tobytaken/prism-electron/issues/99), part of map [#96](https://github.com/tobytaken/prism-electron/issues/96).
Gathered 2026-10-09 from primary sources (upstream docs, tool READMEs, source code, and the shipped packaging of comparable apps). Every claim links to its source.

## Answer in one paragraph

Ship **two artifacts**: `prism` (the GUI) and `prismd` (the privileged daemon), as **separate packages** wherever the format has packages: deb, rpm, and an AUR split PKGBUILD. The native packages install, configure and (where distro policy allows) enable `prismd.service`. **Flatpak and AppImage ship only the GUI.** They do not install the daemon in v1. Monitoring works without the daemon (in-process NVML). Control needs a host `prismd` from the native package, and an in-app onboarding flow explains how to get it. A LACT-style "install the daemon from inside the Flatpak" escape hatch is possible later, but it costs a Flathub linter exception and a `flatbox`-like runner, so defer it. Build the GUI once on an old-glibc baseline and repackage the same binary into deb, rpm and AppImage. Build Flatpak from source on `org.freedesktop.Platform//26.08` with `flatpak-cargo-generator`. Build the AUR package from source per the Arch Rust guidelines.

## 1. Comparable apps

### LACT (closest analogue: GUI plus root daemon, NVIDIA via NVML)

- **Formats per release (v0.10.1, 2026-08-29):** `.flatpak`, Arch `.pkg.tar.zst`, debs per distro version (Debian 13, Ubuntu 24.04, Ubuntu 26.04), rpms per distro version (Fedora 43/44, openSUSE TW), and a separate **`lact-headless`** daemon-only package for every distro. [GitHub release API](https://api.github.com/repos/ilya-zlobintsev/LACT/releases/latest). The per-distro builds exist because the GUI links GTK4/libadwaita. Debs are ~4 MB and the Flatpak bundle is ~6.5 MB (same source).
- **Native packages:** the user runs `sudo systemctl enable --now lactd`. The GUI talks to the daemon over a Unix socket owned by `wheel` or `sudo`. `admin_user`/`admin_group` in `/etc/lact/config.yaml` override this. [LACT README](https://github.com/ilya-zlobintsev/LACT/blob/master/README.md). Prism's `prismd` already copies this model (`contrib/prismd.toml`, `default_admin_group()` in `rust/src/daemon.rs`).
- **Arch:** LACT is in the official `extra` repo as a single `lact` package (GUI and daemon are one binary with subcommands). It is built from a git tag with `cargo fetch --locked` / `cargo build --frozen` and does not enable the service. [Arch PKGBUILD](https://gitlab.archlinux.org/archlinux/packaging/packages/lact/-/blob/main/PKGBUILD)
- **Flatpak:** [flatpak/README.md](https://github.com/ilya-zlobintsev/LACT/blob/master/flatpak/README.md), [manifest](https://github.com/ilya-zlobintsev/LACT/blob/master/flatpak/io.github.ilya_zlobintsev.LACT.yaml), [startup.sh](https://github.com/ilya-zlobintsev/LACT/blob/master/flatpak/startup.sh), [daemon.sh](https://github.com/ilya-zlobintsev/LACT/blob/master/flatpak/daemon.sh)
  - The wrapper `startup.sh` checks for `/run/lactd.sock`. If the socket is missing, it shows a `yad` dialog and then writes `/etc/systemd/system/lactd.service` on the host via `flatpak-spawn --host` + `pkexec` (or `run0 --pipe`), then runs `systemctl enable --now lactd`.
  - The host unit runs the daemon **from the Flatpak's deployed files** through [flatbox](https://github.com/ilya-zlobintsev/flatbox), a static binary that "discovers flatpak application/runtime/extension paths and sets them up for the service". As a result, the daemon depends on no host libraries.
  - finish-args include `--talk-name=org.freedesktop.Flatpak` (host escape), `--system-talk-name=org.freedesktop.systemd1`, `--filesystem=/run/lactd.sock`, `--device=dri`.
  - "It is also possible to skip the service setup if you want to use LACT only for information and monitoring. This can work entirely in the Flatpak sandbox."
  - "**Flatpak specifically requires `admin_user` to be set** … Simply being part of a group like `wheel` does not grant the UI access to the daemon from Flatpak." The setup script writes `admin_user` for the installing user.
  - It uses the GNOME runtime because of GTK.

### CoreCtrl (AMD; distro packages only)

- CoreCtrl is shipped only through distro repositories (Arch, Fedora, Debian/Ubuntu, openSUSE OBS, Gentoo overlay). It has no Flatpak or AppImage. [CoreCtrl README](https://gitlab.com/corectrl/corectrl/-/blob/master/README.md)
- Its root helper is started on demand through polkit (actions `org.corectrl.helper.init`, `org.corectrl.helperkiller.init`). Users add a polkit rule to skip the password prompt. [SlackBuilds README for corectrl](https://slackware.uk/sbo/15.0/system/corectrl/README.SBo)
- This model doesn't fit Prism: ADRs 0002/0005/0006 need a daemon that runs at boot and owns state, not a helper started on demand. The relevant takeaway is that polkit-gated privilege escalation from the GUI is a normal pattern.

### Zed (gpui on Linux)

- Zed is on Flathub on plain **`org.freedesktop` 26.08** (no GNOME/KDE runtime). [dev.zed.Zed.yaml](https://github.com/flathub/dev.zed.Zed/blob/master/dev.zed.Zed.yaml). A gpui app therefore needs nothing beyond the freedesktop runtime. (Zed repackages its prebuilt tarball under a Flathub exception. Prism should build from source.)
- Its Linux tarball bundles the libraries `ldd` reports, except glibc/libgcc and the GLib family. [script/bundle-linux](https://github.com/zed-industries/zed/blob/main/script/bundle-linux)
- gpui's Linux backend now renders through `gpui_wgpu`, uses `font-kit`, and pulls in `xkbcommon`, `x11rb`, and `wayland-client` for its default `wayland` + `x11` features. [crates/gpui_linux/Cargo.toml](https://github.com/zed-industries/zed/blob/main/crates/gpui_linux/Cargo.toml). By contrast, winit (used by iced) and wgpu `dlopen` the Wayland/X11/xkbcommon/Vulkan libraries. [winit Cargo.toml](https://github.com/rust-windowing/winit/blob/master/Cargo.toml) (`x11-dlopen`, `wayland-dlopen`, `xkbcommon-dl`), [wgpu-hal Cargo.toml](https://github.com/gfx-rs/wgpu/blob/trunk/wgpu-hal/Cargo.toml) (`ash` + `libloading`). Either way, the GUI's link-time dependencies are only a handful of ubiquitous desktop libraries. Unlike LACT's GTK build, **one deb and one rpm can serve all distros**, as long as the build uses an old glibc.

## 2. NVIDIA / NVML access per format

| | Where `libnvidia-ml.so.1` comes from | Device nodes |
|---|---|---|
| deb | Debian: `libnvidia-ml1` (non-free) at `/usr/lib/x86_64-linux-gnu/nvidia/current/` [packages.debian.org](https://packages.debian.org/search?searchon=contents&keywords=libnvidia-ml.so.1&mode=filename&suite=stable&arch=amd64). Ubuntu ships it in its driver packages. | host |
| rpm | RPM Fusion: `xorg-x11-drv-nvidia-cuda-libs` / `-cuda` ("optional if using nvidia-smi or cuda") [RPM Fusion NVIDIA howto](https://rpmfusion.org/Howto/NVIDIA). LACT: "Nvidia support requires the Nvidia proprietary driver with CUDA libraries installed." | host |
| Arch | `nvidia-utils` ships `usr/lib/libnvidia-ml.so.1` [archlinux.org file list](https://archlinux.org/packages/extra/x86_64/nvidia-utils/files/) | host |
| Flatpak | The `org.freedesktop.Platform.GL.nvidia-<ver>` extension extracts all driver `lib*` from NVIDIA's installer and symlinks `libnvidia-ml.so` [nvidia-extract.c](https://github.com/flathub/org.freedesktop.Platform.GL.nvidia/blob/master/nvidia-extractor/nvidia-extract.c) (L238, L560). The extension must match the host driver version. | `--device=dri` binds `/dev/dri`, `/dev/nvidiactl`, `/dev/nvidia-modeset`, `/dev/nvidia-uvm*` and `/dev/nvidia0..19` [flatpak-run.c](https://github.com/flatpak/flatpak/blob/main/common/flatpak-run.c) |
| AppImage | host (must **not** be bundled; see below) | host |

- `nvml-wrapper` `dlopen`s `libnvidia-ml.so.1` at runtime. [nvml-wrapper lib.rs](https://github.com/rust-nvml/nvml-wrapper/blob/main/nvml-wrapper/src/lib.rs) (`LIB_PATH`). Automatic dependency scanners (cargo-deb `$auto`, rpm auto-req) therefore will not see it. Package names also differ per distro and driver branch. **Do not hard-depend on NVML.** Use `Suggests`/`optdepends`, and have the app explain clearly when NVML is missing.
- Read-only NVML queries (temperatures, clocks, utilisation, power, fan) work unprivileged, so the **GUI can monitor in-process in every format, including inside the Flatpak sandbox**. Writes (clocks, power limits, fan control) go through `prismd`. Today's code forces daemon-only mode under Flatpak (`rust/src/bridge.rs`: `FLATPAK_ID` ⇒ require daemon). The rewrite should revisit this; see risk R1.

## 3. Per-format recommendation

### Cross-cutting: crate layout and build

- Give the GUI and the daemon their **own binary crates in a workspace** (e.g. `crates/prism`, `crates/prismd`), each with its own `[package.metadata.deb]` / `[package.metadata.generate-rpm]` tables. This replaces today's single crate with `variants.prismd`. The daemon package then never pulls GUI dependencies, and each tool's `-p <crate>` workspace support applies. [cargo-deb README](https://github.com/kornelski/cargo-deb/blob/main/README.md) ("Workspaces"; "Variants")
- **One GUI binary, many wrappers.** CI builds `prism` and `prismd` once, in a container with an old glibc (e.g. Debian 12 / glibc 2.36), or with `cargo zigbuild --target x86_64-unknown-linux-gnu.2.xx`, which has documented caveats. [cargo-zigbuild README](https://github.com/rust-cross/cargo-zigbuild/blob/main/README.md). CI then feeds the same binaries to cargo-deb (`--no-build`), cargo-generate-rpm and appimagetool. Flatpak and AUR build from source by policy.
- The release profile (`lto`, `codegen-units = 1`, `strip`, `panic = "abort"`) matters more for binary size than packaging does. cargo-deb strips debug symbols by default and can split them out with `--separate-debug-symbols`. [cargo-deb README](https://github.com/kornelski/cargo-deb/blob/main/README.md)
- **Shared "daemon onboarding" state machine** in the toolkit-agnostic UI-core crate. Its states:
  - daemon reachable
  - socket exists but `NoPermission` → show the `admin_group`/`admin_user` fix
  - unit installed but inactive → offer **Enable** through systemd's D-Bus API (`EnableUnitFiles` + `StartUnit`, gated by polkit `org.freedesktop.systemd1.manage-unit-files` / `manage-units`, `auth_admin_keep`) [systemd polkit policy](https://github.com/systemd/systemd/blob/main/src/core/org.freedesktop.systemd1.policy.in)
  - not installed → per-distro install instructions and links

  All formats reuse this flow. It also covers the distro differences in enable-by-default below.
- Add a **protocol version handshake** to prismd. A Flatpak GUI updates on Flathub's schedule, independently of the host's `prismd` package (R5).

### deb (cargo-deb 3.8.0)

- `prismd` package contents:
  - `/usr/bin/prismd`
  - the unit, installed via `[package.metadata.deb.systemd-units]` (requires `maintainer-scripts`, even an empty dir). cargo-deb then generates `dh_installsystemd`-style fragments, **enabling and starting by default** (`enable`/`start`/`restart-after-upgrade` default `true`). [cargo-deb systemd.md](https://github.com/kornelski/cargo-deb/blob/main/systemd.md). This matches Debian convention.
  - `/etc/prism/prismd.toml`. Files in `/etc` are treated as conffiles automatically. [cargo-deb README](https://github.com/kornelski/cargo-deb/blob/main/README.md)
  - a tmpfiles/sysusers entry if needed (also supported by `systemd-units`)
- `prism` package: binary, `.desktop`, icon, metainfo. `depends = "$auto"`, `Recommends: prismd`. Don't depend on NVML packages.
- One universal `.deb` built on the glibc baseline, instead of LACT's per-distro matrix.

### rpm (cargo-generate-rpm 0.21.0)

- Same split. cargo-generate-rpm supports assets with `config = "noreplace"`, file `user`/`group`/`caps`, all scriptlets (`post_install_script`, `pre_uninstall_script`, `post_uninstall_script`, `post_trans_script`, …), `requires`, and builtin or `find-requires` auto-req. Its default payload compression is zstd. [cargo-generate-rpm README](https://github.com/cat-in-136/cargo-generate-rpm/blob/master/README.md)
- **It has no systemd macros.** Hand-write scriptlets equivalent to `%systemd_post` / `%systemd_preun` / `%systemd_postun_with_restart`. Those macros call `systemd-update-helper install-system-units` on first install, `remove-system-units` on removal, and `mark-restart-system-units` on upgrade. [macros.systemd.in](https://github.com/systemd/systemd/blob/main/src/rpm/macros.systemd.in). `install-units` **applies presets** ("Enable and preset units"). [update-helper.c](https://github.com/systemd/systemd/blob/main/src/update-helper/update-helper.c). On Fedora, preset policy leaves third-party services disabled, so the user enables `prismd` via the GUI onboarding flow (or `systemctl enable --now prismd`).
- Keep `auto-req` on for the GUI (or declare `requires` explicitly). Leave NVML out of `requires`.
- Immutable Fedora variants (Silverblue, Kinoite, Bazzite) can layer the rpm with `rpm-ostree install`, which RPM Fusion's own NVIDIA instructions also use. [RPM Fusion NVIDIA howto](https://rpmfusion.org/Howto/NVIDIA). See risk R4.

### AUR

- Ship one split `PKGBUILD` with `pkgbase=prism`, `pkgname=(prism prismd)`, built from a release tag per the Arch Rust guidelines:
  - `makedepends=(cargo)`
  - `prepare(): cargo fetch --locked --target host-tuple`
  - `build(): cargo build --frozen --release`
  - `check(): cargo test --frozen --workspace`
  - `depends` typically `glibc gcc-libs`, plus whatever the toolkit links

  [Arch developer manual: Rust package guidelines](https://manual.archlinux.page/package-guidelines/rust/)
- `prismd`: `optdepends=('nvidia-utils: NVML')`, plus a `backup=(etc/prism/prismd.toml)` entry. Config lives in `/etc/<pkg>` per the [Arch package guidelines](https://manual.archlinux.page/package-guidelines/). Following Arch practice (e.g. the official [lact PKGBUILD](https://gitlab.archlinux.org/archlinux/packaging/packages/lact/-/blob/main/PKGBUILD)), the package **does not enable the service**. A `.install` message plus the GUI onboarding flow cover enabling.
- Optional extras: `prism-bin`, repackaging the CI release artifacts (the AUR requires the `-bin` suffix for prebuilt deliverables), and `prism-git` (the `-git` suffix is required for VCS builds). [AUR submission guidelines](https://wiki.archlinux.org/title/AUR_submission_guidelines)

### Flatpak (Flathub)

- `runtime: org.freedesktop.Platform`, `runtime-version: '26.08'` (freedesktop-sdk 26.08.0 released 2026-09-01; Rust 1.98 in SDK) [freedesktop-sdk releases](https://gitlab.com/freedesktop-sdk/freedesktop-sdk/-/releases). Add `sdk-extensions: org.freedesktop.Sdk.Extension.rust-stable`. Each freedesktop branch is supported for 2 years, with a new major every August. [Flathub runtimes](https://docs.flathub.org/docs/for-app-authors/runtimes). Use the freedesktop runtime, not GNOME's: a wgpu GUI doesn't need GTK, and the smaller runtime fits the binary-size goal.
- Build offline from source: generate `cargo-sources.json` from `Cargo.lock` with `flatpak-cargo-generator`, set `CARGO_HOME=/run/build/<module>/cargo`, then run `cargo --offline fetch` and `cargo build --offline --release`. Alternatively, CI can publish a `cargo vendor` tarball, which is easier to automate. [flatpak-cargo-generator README](https://github.com/flatpak/flatpak-builder-tools/blob/master/cargo/README.md). Flathub forbids network access during build and prebuilt binaries in submissions. [Flathub requirements](https://docs.flathub.org/docs/for-app-authors/requirements)
- finish-args:
  - `--socket=wayland`
  - `--socket=fallback-x11` (Flathub never grants wayland without fallback-x11 [linter](https://docs.flathub.org/docs/for-app-authors/linter))
  - `--share=ipc`
  - `--device=dri`, which is enough for both Vulkan rendering and NVML device nodes (§2)
  - `--filesystem=/run/prism` for the socket directory. Subpaths of `/run` are allowed except `/run/flatpak` and `/run/host`. [Flatpak sandbox permissions](https://docs.flatpak.org/en/latest/sandbox-permissions.html)
- App ID must be under a namespace the author controls, e.g. `io.github.tobytaken.Prism`. Changing it after publication is painful, so settle it after the repo rename. [Flathub requirements](https://docs.flathub.org/docs/for-app-authors/requirements)
- **Daemon:** not installed by the Flatpak in v1. Monitoring works in-sandbox via NVML. For control, onboarding points at the `prismd` deb/rpm/AUR package. The metainfo must mention the post-install step: "If the user needs to perform additional post installation steps … they must be mentioned or linked". [Flathub requirements](https://docs.flathub.org/docs/for-app-authors/requirements)
- **Later, optional:** a LACT-style self-install of the host daemon. It needs `--talk-name=org.freedesktop.Flatpak` (linter `finish-args-flatpak-spawn-access`: "Granted on sufficient explanation"; LACT and Zed both hold it). It also needs a way to run prismd on the host without host libraries: either flatbox, or a glibc-baseline `prismd` copied out of `/app`. A fully static musl prismd can't `dlopen` NVML.

### AppImage

- GUI only. Build an AppDir (binary, `.desktop`, icon, metainfo, `AppRun`) and pack it with `appimagetool`. [appimagetool README](https://github.com/AppImage/appimagetool/blob/main/README.md). The type-2 runtime is now static (musl) and "libfuse2 is no longer required on the target system". [type2-runtime README](https://github.com/AppImage/type2-runtime/blob/main/README.md). It adds about 0.9 MB (`runtime-x86_64` = 944,632 bytes) [release assets](https://github.com/AppImage/type2-runtime/releases/tag/continuous).
- **Do not bundle** `libGL`, `libEGL`, `libGLX`, `libGLdispatch`, `libdrm`, glibc, Vulkan/NVIDIA driver libs. [AppImage excludelist](https://github.com/AppImageCommunity/pkg2appimage/blob/master/excludelist). With a wgpu GUI that `dlopen`s most system libraries, the AppImage is roughly the stripped binary plus the runtime, compressed.
- No sandbox, so in-process NVML works exactly as in the native packages. The daemon comes from the native package, using the same onboarding flow. Updates: `appimagetool -u/--updateinformation` with zsync, or rely on GitHub releases.
- Of the five targets, this one adds the least for this app: every distro already gets a native package, and an AppImage still can't deliver the daemon. Keep it cheap, as a by-product of the CI binary. Dropping it later is an acceptable option.

## 4. Summary table

| Format | Tooling | GUI | prismd | Daemon enable | NVML for GUI monitoring |
|---|---|---|---|---|---|
| deb | cargo-deb (`--no-build`, per-crate metadata, `systemd-units`) | `prism` (universal .deb) | `prismd` .deb | enabled + started by package (Debian convention) | host, in-process |
| rpm | cargo-generate-rpm + hand-written systemd scriptlets | `prism` | `prismd` | preset (typically disabled) → GUI "Enable" via systemd D-Bus/polkit | host, in-process |
| AUR | split PKGBUILD from source (+ optional `-bin`, `-git`) | `prism` | `prismd` | not enabled (Arch practice) → `.install` note + GUI | host (`nvidia-utils`) |
| Flatpak | flatpak-builder, freedesktop 26.08 + rust-stable, flatpak-cargo-generator | yes | **no** (host package; LACT-style installer deferred) | GUI onboarding | in-sandbox via GL.nvidia extension |
| AppImage | appimagetool + static type2 runtime | yes | **no** (host package) | GUI onboarding | host, in-process |

## 5. Open risks

- **R1. Flathub acceptance hinges on monitoring without the daemon.** "Applications that rely on host components or complicated post installation setups for core functionality will not be accepted. Exceptions … case by case." [Flathub requirements](https://docs.flathub.org/docs/for-app-authors/requirements). If the Flatpak GUI is daemon-only (as `bridge.rs` does today), it risks rejection. In-sandbox NVML monitoring is LACT's argument too. This couples to the map's open "Daemon → UI data flow" question: keep an in-process NVML path in the GUI even if prismd also streams telemetry.
- **R2. The Flatpak NVIDIA extension must match the host driver.** Flatpak picks `GL.nvidia-<ver>` for the running driver. If the extension isn't built yet for a new driver, or the user has the wrong one, both Vulkan rendering (wgpu falls back to software or GL) and NVML (version mismatch) fail. Mitigation: prefer daemon-supplied telemetry when prismd is reachable, and surface a clear error.
- **R3. Socket permissions from the sandbox.** LACT documents that `wheel` group membership does not give a Flatpak GUI access to its daemon socket, so it sets `admin_user`. Prism's `prismd.toml` already has `admin_user` "for Flatpak-host integration" (mode 0666 + peer-credential check). Verify this in the toolkit spike, and have onboarding tell the user exactly which setting to change. Also, Flatpak silently skips `--filesystem` paths that don't exist when the sandbox starts ([flatpak-exports.c](https://github.com/flatpak/flatpak/blob/main/common/flatpak-exports.c)). A GUI started before `/run/prism` exists won't see the socket until it restarts. Consider shipping a `tmpfiles.d` entry for `/run/prism`, or tell the user to restart after installing the daemon.
- **R4. Immutable distros (Bazzite, Silverblue, SteamOS).** These are popular with NVIDIA gamers. Native packages need layering (`rpm-ostree`) or aren't possible at all (SteamOS). Only a LACT-style self-install from the Flatpak covers them without layering. Revisit after v1, accepting the `org.freedesktop.Flatpak` exception and a flatbox-like runner.
- **R5. GUI/daemon version skew.** Flatpak and AppImage update independently of the host `prismd`, so the protocol needs a version handshake and a compatibility window. Today it has none.
- **R6. glibc baseline and link-time dependencies differ by toolkit.** gpui links some system libraries (xkbcommon, xcb, font stack). An iced/winit build `dlopen`s most of them. The universal deb/rpm/AppImage must be built on the oldest supported distro, and `$auto`/auto-req results must be checked on that builder.
- **R7. Distros disagree on enabling services.** Debian enables, Fedora presets (usually disabled), Arch never enables. The GUI must handle "installed but not running" (systemd D-Bus + polkit), or users will think control is broken.
- **R8. NVML package naming.** NVML can't be a hard dependency (dlopen'd; names vary per distro and driver branch), so missing NVML is a runtime condition the GUI must explain.

## 6. Not researched / out of scope here

- Release signing and repositories (apt/dnf repo hosting, COPR, OBS). LACT uses COPR for Fedora, and its release includes `lact.pubkey`.
- Snap, Nix, and distro-official inclusion.
- Exact GitHub Actions wiring (Flathub's own flatpak-github-actions, archlinux container for `makepkg`). The tools above all run headless in containers.
