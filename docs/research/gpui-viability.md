# Research: is gpui viable for Prism on Linux?

Ticket: [#97](https://github.com/tobytaken/prism-electron/issues/97), a child of the map [#96](https://github.com/tobytaken/prism-electron/issues/96) (rewrite Prism as a native Rust GUI). The spike [#102](https://github.com/tobytaken/prism-electron/issues/102) has to confirm these findings on real hardware.

Question: can **gpui** (Zed's UI framework) meet the map's toolkit gates on Linux? The hard gates are Wayland on NVIDIA's proprietary driver, Flatpak, solid text input, and a custom canvas for an interactive fan-curve editor and live charts. X11, accessibility, API stability, documentation and ecosystem are weighted. Resource use (RAM, binary size) is the main scoring criterion.

This document reports facts and how confident we are in them. It doesn't choose a toolkit.

Researched 2026-10-09. Sources: zed-industries/zed at `df48a139` (main, 2026-10-09), crates.io API, longbridge/gpui-kit, flathub/dev.zed.Zed, and GitHub issues. A throwaway probe was built on the research host (Ubuntu "resolute", Intel UHD 630 + AMD RX 460, **no NVIDIA GPU, no running display server**). So nothing NVIDIA-specific was run here.

## Confidence levels

- **High**: read directly from gpui/Zed source or docs, crates.io metadata, or a maintainer statement.
- **Medium**: an end-user report on the upstream tracker that matches the source, or a measurement on this host that may not carry over to NVIDIA hardware.
- **Low**: inference, or a single unverified report.

## Verdict in one paragraph

**Viable with caveats.** All four hard gates have first-party support in current gpui. The Linux renderer was rewritten on `wgpu` 29 (Vulkan, with a GL fallback) in February 2026, explicitly to fix the NVIDIA and Wayland freezes the old `blade` renderer had. Zed itself ships on Flathub. gpui has a Wayland `text-input-v3` and X11 XIM IME path. It also has a `canvas` element and a `PathBuilder` (lyon) for stroked and filled paths. AccessKit (AT-SPI on Linux) landed in May 2026. The caveats are about **how you depend on it** and **footprint**:

- The official `gpui` crate on crates.io is stale (0.2.2, October 2025), and the Linux platform crates aren't published by Zed at all. In practice you either pin a git revision of the Zed monorepo or use Longbridge's weekly `gpui-pre` snapshots.
- The API is explicitly pre-1.0 with frequent breaking changes.
- A minimal app is 13–22 MiB stripped, depending on the release profile, and about 30 MiB with gpui-kit. On a software renderer it idles at about 155–170 MiB RSS (about 45–60 MiB anonymous). The NVIDIA number is unmeasured.
- There's no built-in tray icon.

## 1. Hard gates

### 1.1 Wayland on NVIDIA proprietary drivers

| Finding | Confidence | Source |
|---|---|---|
| On Linux, gpui renders through `gpui_wgpu`, which uses `wgpu` 29 with the `vulkan` and `gles` backends. Before choosing an adapter, it creates the instance with `Backends::VULKAN \| Backends::GL`. | High | `crates/gpui_wgpu/Cargo.toml`, `crates/gpui_wgpu/src/wgpu_context.rs` (`instance()`); workspace `wgpu = "29.0"` |
| `blade` was removed and the Linux renderer was rewritten on wgpu in [zed#46758](https://github.com/zed-industries/zed/pull/46758) (merged 2026-02-13). The PR's stated reason was the frozen-window bugs on NVIDIA with Smithay-based compositors ([niri#2335](https://github.com/YaLTeR/niri/issues/2335), zed#44814, zed#40481). | High | PR body |
| Adapter selection tries each adapter by actually configuring the surface. The source comments name this NVIDIA case: "NVIDIA reporting Vulkan Wayland support but failing because the Wayland compositor runs on the Intel GPU". Order: `ZED_DEVICE_ID` override, then the compositor's GPU (read from the Wayland/X11 DRM device via sysfs), then discrete over integrated, then Vulkan over GL. | High | `wgpu_context.rs` `select_adapter_and_device`, `sort_adapters`; `gpui_linux/src/linux/platform.rs` `compositor_gpu_hint_from_dev_t` |
| GPU device-loss handling now exists. A `set_device_lost_callback` flags it, and the atlas and renderer rebuild resources. The long-standing request [zed#23288](https://github.com/zed-industries/zed/issues/23288) is still open, so treat suspend/resume recovery as unproven. | High (code exists) / Medium (it works) | `wgpu_context.rs` L24-55, `wgpu_atlas.rs` `handle_device_lost`, `wgpu_renderer.rs` "device recovery" |
| Remaining NVIDIA reports since the wgpu switch: [zed#50734](https://github.com/zed-industries/zed/issues/50734) niri+NVIDIA frozen window, fixed in 0.225.13 (2026-03). [zed#56383](https://github.com/zed-industries/zed/issues/56383) segfault in `libnvidia-egl-wayland` on the 535 driver; the maintainer says the 550 driver fixes it (closed 2026-06). [zed#52944](https://github.com/zed-industries/zed/issues/52944) host crash on X11 + `nvidia-open` 595 via the GL/EGL path; the workaround was `__NV_FORCE_ENABLE_X11_EGL_PLATFORM`, and a bot closed it 2026-08. **Only two NVIDIA-titled rendering issues are open**: [zed#52517](https://github.com/zed-industries/zed/issues/52517) (Kali, hybrid Intel/NVIDIA, "Invalid surface") and [zed#51324](https://github.com/zed-industries/zed/issues/51324) (startup wakes a suspended dGPU, see §5). | Medium | GitHub issue search `nvidia in:title`, 2026-10-09 |
| Zed's Linux docs list GPU troubleshooting (`ZED_DEVICE_ID`, `DRI_PRIME`, `vkcube`, `ZED_LOG=wgpu=info`) but no NVIDIA-specific known-bad list. Most remaining driver-specific notes are about AMD (`amdvlk`). | High | `docs/src/linux.md` |

Assessment: NVIDIA + Wayland is a **supported, actively fixed path**, and Zed has a large NVIDIA Linux user base exercising it. The main risk has moved from "freezes" (blade) to edge cases: old drivers (535), the GL/EGL fallback on X11, hybrid laptops, and suspend/resume.

### 1.2 Flatpak

| Finding | Confidence | Source |
|---|---|---|
| Zed ships on Flathub as `dev.zed.Zed`, updated weekly (v1.23.2 on 2026-10-07). It uses the plain `org.freedesktop.Sdk` 26.08 runtime with `--device=dri --socket=wayland --socket=fallback-x11 --share=ipc`. No GTK/GNOME runtime is needed. | High | [flathub/dev.zed.Zed `dev.zed.Zed.yaml`](https://github.com/flathub/dev.zed.Zed/blob/master/dev.zed.Zed.yaml) |
| gpui uses XDG desktop portals through `ashpd` 0.13 (`file_chooser`, `open_uri`, `settings` for appearance, `inhibit`), plus `oo7` (Secret Service) and `notify-rust`. These suit a sandbox. | High | `crates/gpui_linux/Cargo.toml`, `platform.rs` `inhibit_idle_sleep` |
| Open Flatpak-related gpui problems are about GPU *selection* on hybrid laptops ([flathub/dev.zed.Zed#382](https://github.com/flathub/dev.zed.Zed/issues/382): NVIDIA dGPU not used, so it falls back to the iGPU; `ZED_DEVICE_ID` override). They aren't about rendering failing. The other open Zed Flatpak issues are about Zed's own host-escape and terminal features. | Medium | issue searches |
| Inside Flatpak, NVIDIA Vulkan comes from the `org.freedesktop.Platform.GL.nvidia-*` extension, which must match the host driver. That's standard for any Vulkan app, not specific to gpui. | Medium (general Flatpak behaviour, not re-verified here) | Flatpak runtime extension model |

Note: gpui's `ZED_DEVICE_ID` env var is a Zed name baked into gpui. Prism would inherit that name.

### 1.3 Text input (forms, numeric fields, IME)

| Finding | Confidence | Source |
|---|---|---|
| gpui core has the input plumbing (`EntityInputHandler`, focus, key dispatch, actions/keymaps, `tab_stop`) but **no ready-made text field**. `examples/input.rs` shows how to build one. Zed uses its own `editor` crate, which is not a library. | High | `crates/gpui/src/input.rs`, `examples/input.rs`, README |
| IME: Wayland uses `zwp_text_input_v3` with cursor-rectangle reporting. X11 uses XIM via Zed's `xim-rs` fork. Open Linux IME issues are minor: [zed#65302](https://github.com/zed-industries/zed/issues/65302) (preedit not styled on Wayland, S3) and [zed#60578](https://github.com/zed-industries/zed/issues/60578) (X11 XIM doesn't reconnect after an error, S2). | High (impl) / Medium (quality) | `gpui_linux/src/linux/wayland/client.rs`, `gpui_linux/Cargo.toml` (`xim`), issue search |
| `gpui-component` ships `Input`, `Textarea`, `NumberInput`, `OtpInput`, `InputGroup`, search and a code editor. Recent fixes cover IME candidate placement (gpui-kit#3286, closed 2026-09-28) and byte-offset panics (gpui-kit#2699). An open bug: a focused `Input` has no AccessKit node (gpui-kit#3183). | High (exists) / Medium (maturity) | [`crates/component/src/input/`](https://github.com/longbridge/gpui-kit/tree/main/crates/component/src/input), gpui-kit issues |

Assessment: text input passes the gate **if Prism uses gpui-component's inputs** (or copies them). Building inputs on raw gpui is real work.

### 1.4 Custom canvas: fan-curve editor and live charts

| Finding | Confidence | Source |
|---|---|---|
| `canvas(prepaint, paint)` gives you an element that gets its bounds plus `&mut Window` for immediate-mode painting. `PathBuilder` (lyon tessellation) supports `move_to`, `line_to`, quadratic and cubic Béziers, `arc_to`, `stroke(width)`, `fill()` and `dash_array`. `Window::paint_path`, `paint_quad`, `paint_svg` and `paint_image` draw. | High | `crates/gpui/src/elements/canvas.rs`, `path_builder.rs`, `window.rs` |
| Interaction (drag handles on curve points) uses ordinary mouse-down/move/up listeners plus hit-testing in the element. Custom `Element` impls give full control over layout and paint. | High | README "Low level, imperative UI with Elements" |
| `gpui-component` includes line, area, bar, candlestick, pie, radar and sankey charts, plus a `plot` module (scales, axes, tooltip). Other libraries: `plotters-gpui` and `gpui-d3rs`/`gpui-px` (both dormant). | High | gpui-kit `crates/component/src/chart/`, [awesome-gpui](https://github.com/zed-industries/awesome-gpui) |
| The probe built for this ticket draws a 1 Hz updating polyline through `canvas` + `PathBuilder::stroke`, using only public API. It compiles against `gpui-pre 0.3.8`. | High | §4 probe |
| Overdraw/performance: [zed#8043](https://github.com/zed-industries/zed/issues/8043) "GPUI overdraws too much" is open (S2, since 2024). For a dashboard you keep open, idle redraw cost matters. | Medium | issue |

Assessment: passes. This is gpui's strength: low-level painting with retained entities.

## 2. Weighted criteria

### 2.1 X11

Supported (`x11` feature: `x11rb`, XIM, RandR scaling with a `GPUI_X11_SCALE_FACTOR` override, DRI3 for GPU detection). Wayland and X11 are both default features and picked at runtime (setting `WAYLAND_DISPLAY=` forces X11). One X11+NVIDIA risk is the GL/EGL path (zed#52944). Confidence High. Sources: `gpui_linux/Cargo.toml`, `docs/src/linux.md` §X11 scale factor.

### 2.2 Accessibility

| Finding | Confidence | Source |
|---|---|---|
| AccessKit landed in [zed#56065](https://github.com/zed-industries/zed/pull/56065) (2026-05-27). Follow-ups: an opt-out for apps (#57954), landmarks/menus (#60397, 2026-07), accessibility identifiers (#61926, 2026-08). `gpui_linux` depends on `accesskit_unix` 0.22 (AT-SPI over zbus) for both Wayland and X11 windows. | High | PRs, `Cargo.toml` (`accesskit = 0.24`) |
| Opt-in model: elements are exposed only if they have an `id` **and** a `role` (`div().id(..).role(Role::Button)`). Guide: `crates/gpui/src/_accessibility.rs`, example `examples/a11y.rs`. | High | `_accessibility.rs` |
| gpui-kit advertises AccessKit roles, names, states and actions in its interaction layer, with tests. There are known gaps (gpui-kit#3183). | Medium | gpui-kit README, issue |

Assessment: young (about 4.5 months) but real, and on Linux it goes through the standard AT-SPI bridge. Expect gaps. Usable for a keyboard + screen-reader baseline.

### 2.3 API stability and how to depend on it

| Finding | Confidence | Source |
|---|---|---|
| The README says gpui is "still pre-1.0. There will often be breaking changes between versions. You'll also need to use the latest version of stable Rust." It uses edition 2024. | High | `crates/gpui/README.md`, workspace `Cargo.toml` |
| crates.io `gpui` latest is **0.2.2, published 2025-10-22**. Nothing has been published since, although the in-repo crate still says `version = "0.2.2"` and has changed heavily. Its Linux platform now lives in `gpui_platform`, `gpui_linux` and `gpui_wgpu`, which are `publish = false` (workspace default) and **absent from crates.io**. The README's `gpui_platform = { version = "*" }` instruction can't be satisfied from crates.io today. | High | crates.io API; zed `Cargo.toml` `[workspace.package] publish = false` |
| Longbridge (crates.io user `huacnlee`) publishes **`gpui-pre`** snapshots: about 20 crates (`gpui-pre`, `gpui-pre-platform`, `gpui-pre-linux`, `gpui-pre-wgpu`, …) cut from a named Zed commit roughly weekly. 0.3.0 came out 2026-09-03; 0.3.8 came out 2026-10-05 from `zed@279fe07` (2026-10-04). All Apache-2.0. | High | crates.io API |
| `gpui-kit` 0.7.1 (2026-10-05) pins `gpui-pre =0.3.8` and re-exports gpui, so "applications list a single dependency". Mixing a git `gpui` with gpui-component produced duplicate-type errors (gpui-kit#2532). The current advice is to depend only on `gpui-kit`. gpui-component went 0.5 → 0.6 → 0.7 in about five weeks (Sep–Oct 2026). | High | crates.io, gpui-kit README, [gpui-kit#2532](https://github.com/longbridge/gpui-kit/issues/2532) |

The realistic options for Prism:

- (a) Pin `gpui-kit = "=0.7.x"`, which brings `gpui-pre` from crates.io. You get a reproducible, vendorable crates.io dependency, and the upgrade cadence follows Longbridge.
- (b) Use a git dependency on `zed-industries/zed` at a fixed `rev` (`gpui` + `gpui_platform`) without gpui-component. This tracks upstream directly, but `cargo vendor` pulls the whole monorepo's git source, and Flatpak/AUR offline builds need the git source vendored.

Either way, expect a breaking upgrade every few weeks if you stay current.

### 2.4 Documentation

Thin and code-first. The README says "the best way to learn about these APIs is to read the Zed source code or drop a question in the Zed Discord". It has rustdoc module guides (`_ownership_and_data_flow`, `_accessibility`, `docs/contexts.md`, `docs/key_dispatch.md`) and about 45 examples in `crates/gpui/examples` (including `painting.rs`, `data_table.rs`, `input.rs`, `testing.rs`, `a11y.rs`). gpui-kit has a docs site ([gpui-kit.com](https://gpui-kit.com)) and a component gallery ("story"). There's a community [gpui-book](https://github.com/MatinAniss/gpui-book), now dormant. Confidence High.

### 2.5 Ecosystem and non-Zed apps

- **gpui-component / gpui-kit** (Longbridge): 16.6k stars, pushed daily, 75+ components, used in the shipped commercial app **Longbridge Pro**. It also covers headless UI integration testing. (gpui-kit README, GitHub API 2026-10-09.)
- [awesome-gpui](https://github.com/zed-industries/awesome-gpui) (Zed-maintained) lists about 95 projects. Examples: zedis (Redis GUI, 2.1k★), Navop (1.8k★), Sonora (1.8k★), Arbor, hummingbird (music player), termy, and Linux system utilities such as **rustat** (Linux system monitor with GPU and sensors, 3★) and bandmeter (network monitor). There are few large Linux-first apps besides Zed.
- Related crates on crates.io: `gpui-router`, `gpui-devtools`, `native-theme-gpui`, `swr-gpui`, `gpui-updater`, `herogpui`.

## 3. Fit with Prism's architecture

| Topic | Finding | Confidence | Source |
|---|---|---|---|
| Executor model | gpui runs its own executors. `ForegroundExecutor` runs on the main/UI thread, integrated with the platform loop (`calloop` on Linux). `BackgroundExecutor` is a thread pool. `cx.spawn(async move \|this, cx\| …)` gives a weak entity handle and an `AsyncApp`. `background_executor().timer(d)` drives periodic work. It isn't tokio, so a tokio-based client has to run on its own runtime thread and send results back over channels. zbus can run on `async-io` without tokio (gpui itself uses zbus via accesskit and ashpd). | High | `crates/gpui/src/executor.rs`, `app/context.rs`, probe |
| State model | App state lives in `Entity<T>` (GPUI-owned, `Rc`-like). Views are entities that implement `Render`. Updates go through `entity.update(cx, …)` + `cx.notify()`, and `subscribe/observe` drive the event flow. A toolkit-agnostic view-model crate fits as plain structs held inside entities. The thin view layer maps its outputs to elements and forwards intents. Polling `prismd` or receiving pushes from it becomes a foreground task that updates the entity. | High | README "Big Picture", `_ownership_and_data_flow.rs` |
| Testing | `#[gpui::test]`, `TestAppContext`/`VisualTestContext` simulate input. gpui-kit adds headless window tests with accessibility assertions. [zed#64718](https://github.com/zed-industries/zed/pull/64718) (2026-09-25) added Linux headless rendering. | High | README, PRs |
| Background / no window | `QuitMode::{Default, LastWindowClosed, Explicit}`: on Linux the default quits when the last window closes. `Explicit` keeps the process alive with no windows. gpui also supports the `wlr-layer-shell` protocol (zed#35610). | High | `crates/gpui/src/app.rs`, PR |
| Tray icon | **gpui has none** (no `tray` or StatusNotifierItem code in gpui/gpui_linux). Options: [`ksni`](https://crates.io/crates/ksni) 0.3.6 (2026-07, pure-Rust SNI over D-Bus, no GTK) on its own thread, sending messages into gpui. Not `tray-icon`: on Linux it pulls in `gtk` 0.18 / libappindicator. GNOME needs the AppIndicator extension for any SNI tray. | High | source grep, crates.io deps |
| Window chrome | `WindowDecorations::{Server, Client}`. On GNOME Wayland (no `xdg-decoration`) the app must draw its own title bar and buttons. Zed does, and gpui-kit has `TitleBar` (gpui-kit#3035 was a usage error, not a bug). | High | `platform.rs`, gpui-kit#3035 |

## 4. Resources: binary size, linked libraries, RAM

A throwaway probe: one window, a text label, and a `canvas` polyline updated at 1 Hz from a `cx.spawn` timer. It used `gpui-pre =0.3.8` + `gpui-pre-platform =0.3.8` (features `wayland,x11`) with Rust stable. The build needed `RUST_FONTCONFIG_DLOPEN=on` because the host has no pkg-config or `-dev` packages. Without it, `libfontconfig.so.1` is also linked.

| Measurement | Value | Confidence |
|---|---|---|
| Crates in `Cargo.lock` | 704 | High |
| Stripped binary, `lto="thin"`, `opt-level=3`, `codegen-units=1` | **22.3 MiB** (23,340,928 B) | High (this host) |
| Stripped binary, `lto="fat"`, `opt-level="s"`, `panic="abort"` | **13.2 MiB** (13,861,920 B) | High (this host) |
| `gpui-kit` 0.7.1 README hello-world (one button, default features incl. Lucide icon assets), thin LTO, opt 3 | **30.4 MiB** (31,868,984 B), 850 crates in `Cargo.lock`; same `NEEDED` libs | High (this host) |
| `NEEDED` libraries (`ldd`) | `libxcb.so.1`, `libxkbcommon.so.0`, `libxkbcommon-x11.so.0` (and so `libxcb-xkb.so.1`, `libXau`, `libXdmcp`), `libgcc_s`, `libm`, `libc`. Plus `libfontconfig.so.1` in a default build. `libvulkan.so.1`, `libwayland-client.so.0` and the GL/EGL libraries are **dlopened** (`wayland-backend` feature `dlopen`, wgpu loader). | High |
| Build deps for packagers | `libxkbcommon-dev`, `libxkbcommon-x11-dev` (linked even when running on Wayland, because the default features include `x11`), `libxcb1-dev`, `libfontconfig-dev`, `pkg-config`. Optional: `libvulkan` at runtime. | High |
| Idle memory, Xvfb + Mesa software (lavapipe/llvmpipe), 600×400 window, sampled after 20 s | gpui probe (thin): **VmRSS 171 MiB**, Pss 142 MiB, RssAnon 60 MiB, RssFile 84 MiB, RssShmem 28 MiB, 100 threads. gpui probe (size-opt): VmRSS 156 MiB, Pss 130 MiB, RssAnon 44 MiB. gpui-kit probe: VmRSS 163 MiB, Pss 136 MiB, RssAnon 43 MiB, 97 threads. All map `libLLVM.so.21`, `libgallium`, `libvulkan_lvp`, `libvulkan_asahi`, `libEGL_mesa`. | Medium: a software renderer with LLVM JIT, a single sample each, a host under build load; **not representative of NVIDIA** |

Reading the RAM figures: about 45–60 MiB is anonymous memory (heap, glyph/atlas and lavapipe's swapchain). The rest is mostly shared file mappings of the Mesa/LLVM stack. gpui-component doesn't noticeably change idle memory for a trivial window. Zed's own report [zed#63327](https://github.com/zed-industries/zed/issues/63327) (open) shows that `gpui_wgpu` creates its instance with `VULKAN | GL`, so it maps every installed ICD and the EGL/gallium stack even when it uses a hardware Vulkan adapter. That costs about 25 MiB (lavapipe+LLVM) plus about 9 MiB (EGL/gallium). The fix, [zed#63346](https://github.com/zed-industries/zed/pull/63346), is still open. On NVIDIA the proprietary user-space libraries (`libnvidia-glcore`, `libnvidia-gpucomp`, …) will dominate RSS instead. **The spike must measure on the target hardware and should report PSS and anonymous memory, not only RSS.** The 100 threads come from gpui's background pool (`num_cpus`) plus lavapipe workers on a 16-thread CPU.

## 5. Risks the spike (#102) must probe

1. **Prism shouldn't run on the GPU it monitors.** gpui prefers the compositor GPU, then *discrete* over integrated. On hybrid laptops, enumerating adapters wakes a runtime-suspended NVIDIA dGPU and stalls cold start for 3–4 s ([zed#51324](https://github.com/zed-industries/zed/issues/51324), open; dup [zed#61439](https://github.com/zed-industries/zed/issues/61439)). A monitor that wakes the dGPU and shows up in `nvidia-smi` as a GPU client is a product bug. Probe: start-up on a PRIME on-demand laptop, check `nvidia-smi` and `/sys/bus/pci/devices/*/power/runtime_status`. Check whether `VK_LOADER_DRIVERS_SELECT`/`ZED_DEVICE_ID` or a patched adapter filter keeps Prism off the dGPU.
2. **Idle RAM and PSS on NVIDIA proprietary + Wayland** (GNOME and KDE), compared with the Electron baseline. Also try `VK_LOADER_DRIVERS_DISABLE` (the zed#63346 approach) and a Vulkan-only instance.
3. **Idle CPU/GPU with a 1 Hz live chart** for a window kept open all day, minimised, or on another workspace (frame callbacks throttled?). Related: zed#8043 (overdraw), [zed#53522](https://github.com/zed-industries/zed/issues/53522) (GNOME 50 Wayland sluggishness, open).
4. **Suspend/resume and driver reload** on NVIDIA: does the device-loss recovery path actually bring the window back?
5. **Old or odd drivers**: the 535 branch (`libnvidia-egl-wayland` segfault, zed#56383), the X11 + `nvidia-open` GL/EGL path (zed#52944), and fallback when Vulkan is missing inside Flatpak (a GL extension mismatch).
6. **Flatpak build**: freedesktop 26.08 runtime, `cargo vendor` offline build (crates.io `gpui-pre` vs git monorepo), `--device=dri`, portal-only file access. Also check that the GL.nvidia extension's Vulkan ICD is picked up.
7. **Fan-curve editor ergonomics**: drag handles, snapping, keyboard nudging, and a `NumberInput` bound to the same points, with AccessKit roles on the handles (sliders/spinbuttons) checked in Orca/Accerciser.
8. **Dependency churn**: upgrade the probe across two `gpui-pre`/`gpui-kit` releases and count the breakages. Decide between `gpui-kit` pin and a git rev pin.
9. **Tray via `ksni`** alongside gpui's event loop with `QuitMode::Explicit`: close to tray, reopen the window, autostart.
10. **Binary size budget**: the size of the release build with gpui-kit, plus the size of a deb/rpm/AppImage/Flatpak. Check whether trimming to `wayland` only (dropping `x11`, `xkbcommon-x11` and XIM) is worth the X11 loss.

## Sources

- gpui source: <https://github.com/zed-industries/zed/tree/main/crates/gpui>, `crates/gpui_linux`, `crates/gpui_wgpu`, `crates/gpui_platform` (read at `df48a139`, 2026-10-09)
- gpui README: <https://github.com/zed-industries/zed/blob/main/crates/gpui/README.md>
- Zed Linux docs: <https://github.com/zed-industries/zed/blob/main/docs/src/linux.md> (<https://zed.dev/docs/linux>)
- Blade → wgpu: <https://github.com/zed-industries/zed/pull/46758>
- AccessKit: <https://github.com/zed-industries/zed/pull/56065>
- ICD mapping cost: <https://github.com/zed-industries/zed/issues/63327>, <https://github.com/zed-industries/zed/pull/63346>
- dGPU wake: <https://github.com/zed-industries/zed/issues/51324>
- crates.io: <https://crates.io/crates/gpui>, <https://crates.io/crates/gpui-pre>, <https://crates.io/crates/gpui-pre-platform>, <https://crates.io/crates/gpui-kit>, <https://crates.io/crates/gpui-component>, <https://crates.io/crates/ksni>, <https://crates.io/crates/tray-icon>
- gpui-kit: <https://github.com/longbridge/gpui-kit>, <https://gpui-kit.com>
- Flathub Zed: <https://github.com/flathub/dev.zed.Zed>
- awesome-gpui: <https://github.com/zed-industries/awesome-gpui>

## Appendix: probe recipe

`Cargo.toml` dependencies (the release profile is as in §4):

```toml
gpui = { package = "gpui-pre", version = "=0.3.8" }
gpui_platform = { package = "gpui-pre-platform", version = "=0.3.8", features = ["wayland", "x11"] }
```

This host had no pkg-config or `-dev` packages, so it was built with `RUST_FONTCONFIG_DLOPEN=on` and `-L` pointing at symlinks to the runtime `.so.N` files (plus `libxkbcommon-x11`/`libxcb-xkb` unpacked from `apt-get download`). Memory was sampled from `/proc/<pid>/status` and `smaps_rollup` under `xvfb-run -s "-screen 0 1920x1080x24"`.

<details><summary><code>src/main.rs</code> (window + label + 1 Hz canvas polyline)</summary>

```rust
use std::time::Duration;

use gpui::{
    App, Bounds, Context, PathBuilder, Window, WindowBounds, WindowOptions, canvas, div, point,
    prelude::*, px, rgb, size,
};
use gpui_platform::application;

struct Probe {
    samples: Vec<f32>,
    tick: u64,
}

impl Probe {
    fn new(cx: &mut Context<Self>) -> Self {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                let ok = this
                    .update(cx, |this, cx| {
                        this.tick += 1;
                        let v = ((this.tick as f32) * 0.7).sin() * 0.4 + 0.5;
                        this.samples.push(v);
                        if this.samples.len() > 120 {
                            this.samples.remove(0);
                        }
                        cx.notify();
                    })
                    .is_ok();
                if !ok {
                    break;
                }
            }
        })
        .detach();
        Self { samples: vec![0.5; 2], tick: 0 }
    }
}

impl Render for Probe {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let samples = self.samples.clone();
        div()
            .flex()
            .flex_col()
            .gap_2()
            .p_4()
            .size_full()
            .bg(rgb(0x1e1e1e))
            .text_color(rgb(0xffffff))
            .child(format!("tick {}", self.tick))
            .child(
                canvas(
                    |_, _, _| {},
                    move |bounds, _, window, _| {
                        let mut path = PathBuilder::stroke(px(2.));
                        let n = samples.len().max(2) as f32 - 1.;
                        for (i, v) in samples.iter().enumerate() {
                            let x = bounds.origin.x + bounds.size.width * (i as f32 / n);
                            let y = bounds.origin.y + bounds.size.height * (1. - v);
                            if i == 0 {
                                path.move_to(point(x, y));
                            } else {
                                path.line_to(point(x, y));
                            }
                        }
                        if let Ok(path) = path.build() {
                            window.paint_path(path, rgb(0x76b900));
                        }
                    },
                )
                .size_full(),
            )
    }
}

fn main() {
    application().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(600.), px(400.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_, cx| cx.new(Probe::new),
        )
        .unwrap();
        cx.activate(true);
    });
}
```

</details>
