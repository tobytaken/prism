# Research: native Rust GUI toolkits other than gpui

Ticket: [#98](https://github.com/tobytaken/prism-electron/issues/98), a child of the map [#96](https://github.com/tobytaken/prism-electron/issues/96). Sibling: [#97](https://github.com/tobytaken/prism-electron/issues/97) (gpui in depth, still open when this was written). Feeds the toolkit spike [#102](https://github.com/tobytaken/prism-electron/issues/102).

Question: besides gpui, which **non-webview** native Rust GUI toolkits are serious candidates for Prism, and how do they compare on the map's gates and criteria? This document reports facts, confidence and a ranking. The spike makes the decision.

Researched 2026-10-09. Versions and dates come from the crates.io API and the GitHub API on that day. The research host has no display session, so **no idle-RAM numbers were measured here**. Binary sizes were measured (see section 3).

## Confidence levels

- **High**: stated by the toolkit's owners (repo, changelog, docs, Cargo features, crates.io), or measured here.
- **Medium**: inferred from first-party code or from several first-party issue reports.
- **Low**: one third-party report, or not verified.

## 1. Short answer

| Rank | Toolkit | Verdict | Why |
|---|---|---|---|
| 1 | **Slint** 1.18.1 | Spike it | Only candidate that passes every gate **and** scores well on the weighted criteria: 1.x API with compatibility promises, docs, AccessKit accessibility, built-in tray icon, frameless window + `WindowMoveArea`, OpenGL (EGL) default renderer. Caveats: charts and the fan-curve editor are built from `Path` elements, not a canvas API. The royalty-free licence needs an "uses Slint" attribution, or GPLv3. |
| 2 | **iced** 0.14.0 | Spike it | Best canvas (`canvas::Program`) and the best match for a toolkit-agnostic core (Elm-style `update(Message)`, `Task`, `Subscription`, `daemon` mode). Caveats: no accessibility upstream, about one breaking release a year, and an **open NVIDIA + GNOME Wayland "UI stops redrawing after being minimised"** report (#3496, Oct 2026) on its wgpu/Vulkan path. |
| (gpui) | gpui (git) | Decided by #97 | Survey level: closest to the wanted Zed look by definition, richest component kit (`gpui-component` 0.7.1). Biggest risk is dependency hygiene: the crates.io release (0.2.2) is a year old and no longer builds against current dependencies (section 3), so you pin git. |
| 3 | egui 0.36.2 | Fallback | Excellent for charts and editing (`Painter`, `egui_plot`), AccessKit on by default, biggest ecosystem. Immediate-mode look and layout make a polished Zed-like design costly. Breaking releases every 1–3 months. |
| 4 | libcosmic (git) | Only as "iced with batteries" | System76's iced fork with accessibility and many widgets. Git-only dependency, COSMIC design language, MPL-2.0. |
| 5 | Freya 0.4.3 | Watch | Skia renderer, AccessKit, tray and plotters integrations. Mostly a single maintainer. Skia makes binaries large (see section 3). 0.5 is in release candidates. |
| 6 | Xilem / Masonry 0.4.0 | Not yet | The README calls it "experimental". Good foundations (AccessKit, Vello, Parley). Last release 2025-10-29. |
| — | Floem 0.2.0 | Drop | No crates.io release since 2024-11-14. No accessibility (#8 open). |
| — | Makepad | Drop | crates.io release (1.0.0) is from 2025-05-13 while work happens on the `dev` branch. Own platform layer, no accessibility (#196 open). The project now presents itself as an "AI-accelerated" environment. |
| — | GTK4 (gtk4-rs + relm4), Qt (cxx-qt), Dioxus Native, Vizia | Out of scope / drop | See section 5. |

**Recommendation for #102:** the top two from this survey are **Slint**, then **iced**.

- #102 spikes 1–2 toolkits. If gpui passes #97, spike **gpui + Slint**. They cover the space well: gpui is the best fit for the look but the riskiest dependency; Slint is the safest dependency, with OpenGL instead of wgpu on NVIDIA and built-in tray, frameless window and accessibility.
- If gpui fails #97, spike **Slint + iced**. iced has the strongest canvas and the cleanest Elm-style seam for the toolkit-agnostic UI core, but weaker accessibility and stability.

## 2. Scoring

Hard gates: ✅ pass, ⚠️ pass with a risk the spike must probe, ❌ fail. No candidate was run on NVIDIA hardware for this survey, so ✅ means the evidence shows no known problem, not that it was tested. Weighted criteria: 1 (poor) to 5 (excellent).

| Criterion | Slint | iced | egui | libcosmic | Freya | Xilem | Floem | Makepad |
|---|---|---|---|---|---|---|---|---|
| **Gate:** Wayland on NVIDIA | ✅ GL/EGL default | ⚠️ wgpu, #3496 | ⚠️ wgpu default, #8416, glow fallback | ⚠️ as iced | ⚠️ Skia, #2302 fixed | ⚠️ Vello compute | ⚠️ | ⚠️ |
| **Gate:** Flatpak | ✅ | ✅ (Halloy on Flathub) | ✅ | ✅ (Flathub apps) | ✅ | ✅ | ✅ | ✅ |
| **Gate:** text input + IME | ✅ | ✅ (IME since 0.14) | ✅ | ✅ | ✅ | ⚠️ | ⚠️ | ⚠️ |
| **Gate:** canvas / interactive charts | ⚠️ `Path` + `TouchArea` | ✅ `canvas` | ✅ `Painter` | ✅ `canvas` | ✅ Skia canvas | ✅ | ✅ | ✅ |
| Stripped binary (sec. 3) | 16.5 MB | 12.4 MB | 16.0 MB | not measured | 25.0 MB | 14.1 MB | 13.2 MB | not measured |
| Idle RAM | not measured | not measured | not measured | — | — | — | — | — |
| X11 | 5 | 5 | 5 | 5 | 5 | 4 | 4 | 4 |
| Accessibility (AT-SPI) | 4 | 1 | 4 | 4 | 3 | 4 | 1 | 1 |
| API stability / maturity | 5 | 2 | 2 | 2 | 2 | 1 | 1 | 1 |
| Docs | 5 | 3 | 4 | 3 | 3 | 2 | 2 | 2 |
| Ecosystem / components | 3 | 4 | 5 | 4 | 3 | 1 | 2 | 2 |
| Custom "Zed-like" look, frameless | 4 | 4 | 2 | 2 | 4 | 3 | 4 | 4 |
| Tray / background | 5 | 3 | 1 | 3 | 4 | 1 | 1 | 1 |
| Async fit / UI-core seam | 4 | 5 | 3 | 4 | 3 | 3 | 3 | 2 |

The numbers are a judgement from the evidence below, not a measurement. RAM and binary size are the primary criteria. Binary size was measured; RAM was not, so the ranking does **not** yet reflect the primary criterion properly. That is what the spike is for.

## 3. Binary size, measured here

Method: a minimal app per toolkit (a counter: label plus button; iced, egui and Slint also have a single-line text input). Each is built standalone with `cargo build --release` and the profile `strip = true`, `lto = true` (fat), `codegen-units = 1`, default `opt-level = 3`. Toolchain: rustc 1.99.0 (2026-09-28), x86_64 Linux. Versions are pinned exactly. The scaffolds lived outside the repo (`~/.cache/prism-gui-size-bench`) and are not committed.

| Toolkit | Version / features | Stripped binary | Dynamic libraries (`ldd`, beyond libc/libm/libgcc_s) |
|---|---|---|---|
| iced | 0.14.0, default (wgpu + tiny-skia) | **12.4 MB** (12,407,512 B) | none (Vulkan/EGL/Wayland/xkbcommon are loaded with `dlopen` at runtime) |
| Floem | 0.2.0, default | 13.2 MB (13,232,216 B) | none |
| Xilem | 0.4.0, default (Vello/wgpu) | 14.1 MB (14,083,736 B) | none |
| egui (eframe) | 0.36.2, default (wgpu, accesskit, wayland, x11) | 16.0 MB (16,026,288 B) | none |
| Slint | 1.18.1, `backend-winit`, `renderer-femtovg`, `accessibility` (no Qt, no software renderer, no tray) | 16.5 MB (16,530,456 B) | fontconfig, freetype (plus their deps: expat, zlib, png, brotli, bz2) |
| Freya | 0.4.3, default (Skia) | 25.0 MB (25,047,464 B) | libstdc++, freetype, fontconfig, EGL, GL, X11/xcb, png, brotli, … |
| gpui | 0.2.2 (crates.io) | **did not build** | `xattr` 0.2.3, pulled in by the year-old crates.io release, fails against the current `libc` (`cannot find value ENOATTR`). #97 measures gpui from git. |

Reading the numbers (judgement):

- All the pure-Rust stacks are in the same 12–17 MB band for a trivial app. The binary-size differences between iced, Floem, Xilem, egui and Slint are smaller than what Prism's own code and dependencies will add (NVML wrapper, serde, the async runtime). Binary size alone won't pick the winner. Skia (Freya) is the outlier at about +9 MB.
- Slint's number is without its software renderer and tray features; the defaults would add a little. Slint and Freya link fontconfig/freetype dynamically, so these libraries must be present on the host or in the Flatpak runtime; both are part of the freedesktop runtime.
- Idle RAM is the more important number and is **not** in this table. Measure it on the target (NVIDIA + Wayland) in the spike.

## 4. Per toolkit

### Slint

- **Version / cadence:** 1.18.1, 2026-09-21. Minor releases roughly every two months (1.16.0 2026-04-16, 1.17.0 2026-06-24, 1.18.0 2026-09-16) ([CHANGELOG](https://github.com/slint-ui/slint/blob/master/CHANGELOG.md)). 24.1k stars. High.
- **API stability:** semver 1.x. Behaviour changes go behind mandatory `compat-1-x` Cargo features so that new patch versions don't break you ([Cargo.toml](https://github.com/slint-ui/slint/blob/master/api/rs/slint/Cargo.toml), [blog post linked there](https://slint.dev/blog/rust-adding-default-cargo-feature.html)). High.
- **Licence:** GPLv3, a royalty-free licence, or a commercial licence. The royalty-free licence covers desktop apps but requires that "you disclose that you use Slint (for example with the `AboutSlint` widget or the Slint badge)" ([LICENSE.md](https://github.com/slint-ui/slint/blob/master/LICENSE.md)). Prism is MIT, so using the royalty-free licence means an attribution in the app; this is the only candidate with a non-permissive licence. High.
- **Rendering / Wayland:** desktop backends are `winit` (Wayland and X11) and `qt`; renderers are FemtoVG (OpenGL, or wgpu with `renderer-femtovg-wgpu`), Skia (OpenGL/Vulkan), software, and experimental Vello ([backends and renderers](https://docs.slint.dev/latest/docs/slint/guide/backends-and-renderers/backends_and_renderers/)). The default feature set is `backend-default`, `renderer-femtovg`, `renderer-software`, `accessibility`, `system-tray`. OpenGL through EGL avoids the wgpu/Vulkan swapchain path that the iced and egui NVIDIA reports involve. High for the facts; Medium that this makes NVIDIA Wayland safer.
- **NVIDIA:** a search of the tracker for "nvidia" found no open NVIDIA-specific Wayland bug. Medium.
- **Text input:** `LineEdit`, `TextEdit`, IME support, and `input-method-hints` (1.18.0). A Wayland text-selection lag (#10912, Slint 1.15) was traced to repeated text shaping per mouse-move event. A fix landed in March 2026 (#11021, per the maintainers' comments). High.
- **Canvas:** no immediate-mode canvas. The `Path` element takes SVG path commands, as a string bound to a property, or as `MoveTo`/`LineTo`/`CubicTo` sub-elements, with fill and stroke ([Path](https://docs.slint.dev/latest/docs/slint/reference/elements/path/)). A fan-curve editor is `Path` + draggable points in a `TouchArea`. Live charts mean producing path-command strings from Rust, or drawing into an image buffer. Workable, but the spike should check redraw cost at 1 Hz × several series. High for the facts; Medium for the cost concern.
- **Frameless / custom title bar:** `Window.no-frame`, `resize-border-width`, and since 1.18.0 a `WindowMoveArea` element ("start an interactive window move, like a native title bar") plus a `window-title-bar` accessible role (CHANGELOG). High.
- **Tray / background:** `SystemTrayIcon` element since 1.17.0 (2026-06-24), backed by `ksni` on Linux and on by default (`system-tray` feature). High.
- **Accessibility:** `accessibility` feature (AccessKit in the winit backend). 1.18.0 exposes text-input content and selection to assistive technology. Recent releases keep adding `accessible-*` properties. High.
- **Async / UI core:** UI is a `.slint` DSL compiled to Rust. `slint::spawn_local` runs futures on the Slint event loop; from other threads use `invoke_from_event_loop`. Tokio needs `async_compat`, smol works directly ([spawn_local docs](https://docs.rs/slint/1.18.1/slint/fn.spawn_local.html)). The DSL is a separate language, so view-models stay in Rust and the view layer can't hold logic by accident. That suits the map's "thin view layer". The cost is glue code that copies view-model state into Slint properties and models. High for the facts; the fit is a judgement.
- **Look:** styling lives in the DSL. Built-in widget styles (fluent, material, cupertino, cosmic, qt) can be replaced by your own components. Live preview in the LSP. Medium (not prototyped).

### iced

- **Version / cadence:** 0.14.0, 2025-12-07; previous 0.13.0 on 2024-09-18. Roughly one breaking release a year. `master` is active (last commit 2026-10-07, including "Update `winit` with Wayland crash fix") ([CHANGELOG](https://github.com/iced-rs/iced/blob/master/CHANGELOG.md)). 31.7k stars, MIT. High.
- **0.14 highlights relevant here:** reactive rendering (#2662), input method support (#2777), end-to-end and headless testing (#3059, #2698), `x11` and `wayland` feature flags (#2869), window drag and drag-resize tasks (#2642), `table` widget. High.
- **Rendering / NVIDIA:** wgpu (Vulkan on Linux) with a tiny-skia software fallback. Open reports:
  - [#3496](https://github.com/iced-rs/iced/issues/3496) (opened 2026-10, iced 0.14, Ubuntu 26.04 GNOME 50 Wayland, RTX 3070, driver 595.91.07): "UI sometimes stops redrawing on Wayland … usually happens after the app has been minimised for a while". A Wayland trace shows the next frame scheduled about 8 minutes out. For a monitor you leave open, this is the riskiest single finding in this survey. High (first-party tracker), cause unknown.
  - [#2572](https://github.com/iced-rs/iced/issues/2572): SIGSEGV after a few seconds on NVIDIA + Hyprland (2024, iced 0.13, drivers 555/560). Old and may be stale. Low.
- **Flatpak:** iced apps ship on Flathub, e.g. Halloy (`org.squidowl.halloy`, 2026.9 on the Flathub API). High.
- **Canvas:** `canvas::Program` with `Frame`, path builder, fill/stroke, text, geometry `Cache`, and event handling. This is the most direct fit for a draggable fan curve and live charts of all candidates. High.
- **Accessibility:** none upstream. "Implement accessibility support" [#552](https://github.com/iced-rs/iced/issues/552) is open; AccessKit PRs #3111 and #1849 are drafts. libcosmic's fork has it (below). High.
- **Frameless / tray / background:** `decorations: false` plus `window::drag` and drag-resize tasks. No tray; use `ksni` or `tray-icon` beside it. `iced::daemon` "will not stop running when all its windows are closed" ([docs](https://docs.rs/iced/0.14.0/iced/fn.daemon.html)), which fits a background monitor. High.
- **Async / UI core:** Elm architecture. `update(state, Message) -> Task`, `Subscription` for streams such as daemon telemetry, executor feature flags (tokio or smol). A toolkit-agnostic reducer maps one-to-one onto this. High for the facts; the fit is a judgement.
- **Ecosystem:** `iced_aw` 0.14.1 (2026-04-27), libcosmic widgets. `plotters-iced` 0.11.0 (2024-09-18) targets iced 0.13 and looks stale. High.

### egui

- **Version / cadence:** 0.36.2, 2026-09-08. Releases with breaking changes every 1–3 months (0.34.2 2026-05-04, 0.35.0 2026-06-25, 0.36.0 2026-08-05). 0.37 is unreleased with colour emoji, an experimental `StyleProvider` theme system and accessibility improvements ([CHANGELOG](https://github.com/emilk/egui/blob/main/CHANGELOG.md)). 31.1k stars, MIT/Apache-2.0. High.
- **Rendering / NVIDIA:** eframe's default features include `wgpu`, `accesskit`, `wayland`, `x11` ([eframe Cargo.toml](https://github.com/emilk/egui/blob/0.36.2/crates/eframe/Cargo.toml)). The `glow` (OpenGL) renderer is an alternative. Open report [#8416](https://github.com/emilk/egui/issues/8416) (2026-08, RTX 2070 SUPER, driver 610.57.04, Wayland): the app freezes with "A timeout was encountered while trying to acquire the next frame", usually after switching back to the window. The same pattern as iced #3496: a wgpu/Vulkan presentation stall on NVIDIA Wayland. High (report); Medium that glow avoids it.
- **Canvas:** `Painter` (shapes, paths, text) and `egui_plot` 0.37.0. Ideal for charts and an editor. High.
- **Look:** immediate mode. Layout is single-pass and styling is global and per-widget. Getting a polished, animated Zed-like design is hard; 0.37's `StyleProvider` is behind an `experimental` flag. Frameless works (`ViewportBuilder::with_decorations(false)`, `ViewportCommand::StartDrag`, a `custom_window_frame` example). Medium.
- **Async / UI core:** no async runtime. Poll channels each frame and call `request_repaint` from background threads. The view reads the view-model directly, which is simple but makes it easy to leak logic into the view. Medium.
- **Tray:** none. High.

### libcosmic

- Not on crates.io; git dependency. MPL-2.0. 957 stars, last commit 2026-10-08. "A platform toolkit based on iced for creating applets and applications for the COSMIC desktop" ([README](https://github.com/pop-os/libcosmic)). High.
- Vendors System76's iced fork as a submodule (`[dependencies.iced] path = "./iced"`) with `a11y` (AccessKit), `wayland` (layer-shell and other Wayland extensions) and `winit` (X11) features ([Cargo.toml](https://github.com/pop-os/libcosmic/blob/master/Cargo.toml)). High.
- Apps can go on Flathub; COSMIC applets cannot (README). High.
- Look: COSMIC design language and theme, driven by COSMIC's theme portal. Making Prism look like Zed instead of COSMIC would fight the toolkit. Medium.
- NVIDIA/Wayland: [#1131](https://github.com/pop-os/libcosmic/issues/1131) (open, 2026-02): the fork's Wayland frame-callback state machine can deadlock after suspend/resume or DPMS off. Same symptom class as iced #3496. High (report).

### Freya

- 0.4.3, 2026-08-30 (0.4.0 on 2026-07-16); 0.5.0-rc.9 on 2026-10-04. 3.2k stars, MIT. Mostly the work of one maintainer (marc2332). High.
- Skia renderer (via `freya-engine`), winit, AccessKit. Optional features include `tray-icon`, `freya-plotters-backend` + `plotters`, `freya-testing`, `freya-router` (crates.io dependency list). High.
- NVIDIA: [#2302](https://github.com/marc2332/freya/issues/2302) "Segmentation fault on closing secondary windows under nvidia + linux + wayland + vulkan", closed 2026-09-24. High.
- Risk: bus factor, and Skia's binary size.

### Xilem / Masonry

- 0.4.0, 2025-10-29 (0.3.0 on 2025-05-10). Last commit 2026-09-14. README: "An experimental Rust architecture for reactive UI", built on winit, Vello + wgpu, Parley/Fontique, AccessKit ([README](https://github.com/linebender/xilem)). Apache-2.0. High.
- Strong foundations, accessibility designed in, but too young for a product spec in 2026. Vello uses GPU compute shaders. Not ranked for the spike.

### Floem

- 0.2.0, 2024-11-14; no crates.io release since. Commits are sparse (2026-09-24, then 2026-06-21, 2026-05-11). README: "still maturing. We will make occasional breaking changes". Renders via wgpu (vger or Vello), Skia, or tiny-skia. AccessKit support is an open issue ([#8](https://github.com/lapce/floem/issues/8)). High.
- Dropped: a toolkit you would have to git-pin, with no accessibility and slow upstream.

### Makepad

- `makepad-widgets` 1.0.0, 2025-05-13 on crates.io. Development is on the `dev` branch (commits daily, last 2026-10-09). Own platform layer (not winit) with its own Wayland/X11/OpenGL code. The README now leads with "an AI-accelerated application and game development environment". MIT. High.
- Wayland fixes are recent (e.g. #1179 "app exits silently when flush() returns WouldBlock", closed 2026-09-28; #1173 inverted scroll, closed 2026-09-04). Accessibility: "On Accessibility" [#196](https://github.com/makepad/makepad/issues/196) open. High.
- Dropped: stale crates.io release, no accessibility, and a direction that doesn't match a small Linux utility.

### Where gpui slots in (survey level; #97 has the detail)

- crates.io `gpui` is 0.2.2 from 2025-10-22, while Zed's `main` has moved on (crates split into `gpui_platform`, `gpui_linux`, `gpui_wgpu`). Realistically you pin a git revision. High.
- On `main`, the Linux platform crate depends on `gpui_wgpu` and `accesskit_unix` under its `wayland` feature ([gpui_linux/Cargo.toml](https://github.com/zed-industries/zed/blob/main/crates/gpui_linux/Cargo.toml)). That means the same wgpu/Vulkan NVIDIA questions as iced and egui apply. High for the dependency facts.
- `gpui-component` 0.7.1 (2026-10-05, 16.7k stars, repo now `longbridge/gpui-kit`) is the richest ready-made component set of any candidate, and the shortest road to a Zed-like look. High.
- Ranking: gpui sits next to Slint and iced at the top. It is the best on "look" and the worst on dependency hygiene and API stability. #97 decides whether its gates hold.

## 5. Other stacks considered and dropped

- **GTK4 via gtk4-rs (0.11.5, 2026-09-20) + relm4 (0.11.0):** the most mature Linux story for IME, AT-SPI and Flatpak (GNOME runtime). It is a C toolkit with a libadwaita look, not the native-Rust direction the map asks for. It is also not webview-based, so it stays a legitimate fallback if every Rust toolkit fails a gate. High (versions).
- **Qt via cxx-qt (0.10.0):** heavy C++ dependency and QML; works against the binary-size goal. Dropped.
- **Dioxus Native / Blitz (`dioxus-native` 0.8.0-alpha.1, `blitz-dom` 0.3.0-beta.2):** non-webview, but HTML/CSS-shaped, which brings back the "web-like feel" the map wants to lose. Alpha. Dropped.
- **Vizia (0.4.0, 2026-04-23):** aimed at audio plugins, small community (586 recent downloads). Dropped.

## 6. Cross-cutting risks for the spike

1. **wgpu/Vulkan presentation stalls on NVIDIA Wayland after the window is hidden.** Independent reports in iced (#3496), egui (#8416) and libcosmic (#1131) describe a UI that stops redrawing after it is minimised, switched away from, or after suspend/DPMS. Prism is meant to stay open as a monitor, so the spike should run every candidate through: minimise for 30+ minutes, alt-tab, lock the screen / DPMS off, suspend/resume, then restore. Run it on GNOME and KDE Wayland with a current NVIDIA driver. Compare wgpu-Vulkan against an OpenGL path where the toolkit has one (Slint FemtoVG, egui glow).
2. **Flatpak + NVIDIA** depends on the matching `org.freedesktop.Platform.GL.nvidia-*` extension for every toolkit. Test the GL and Vulkan paths inside the sandbox, not only on the host.
3. **Idle RAM** has not been measured for any candidate. Measure RSS and PSS (`/proc/<pid>/smaps_rollup`) after 60 s idle and after 10 min of 1 Hz telemetry, on NVIDIA Wayland. GPU driver mappings dominate small apps on NVIDIA, so compare toolkits on the same machine, not against published numbers.
4. **Slint chart cost:** check that rebuilding `Path` commands for several 1 Hz series doesn't cause visible CPU use or jank.
5. **iced accessibility:** if accessibility is weighted heavily, decide up front whether "no AT-SPI until upstream lands it" is acceptable, or whether libcosmic's fork is worth its git-only, COSMIC-styled trade-off.

## Sources

- crates.io API (`/api/v1/crates/<name>` and `/versions`), queried 2026-10-09: iced, slint, egui, eframe, floem, xilem, masonry, makepad-widgets, freya, gpui, gpui-component, vizia, accesskit, winit, wgpu, dioxus-native, blitz-dom, gtk4, relm4, libadwaita, cxx-qt, ksni, tray-icon, plotters-iced, egui_plot, iced_aw.
- GitHub API repo metadata and issue search, queried 2026-10-09, for the repos linked above.
- Slint: [CHANGELOG](https://github.com/slint-ui/slint/blob/master/CHANGELOG.md), [api/rs/slint/Cargo.toml](https://github.com/slint-ui/slint/blob/master/api/rs/slint/Cargo.toml), [LICENSE.md](https://github.com/slint-ui/slint/blob/master/LICENSE.md), [backends and renderers](https://docs.slint.dev/latest/docs/slint/guide/backends-and-renderers/backends_and_renderers/), [Window](https://docs.slint.dev/latest/docs/slint/reference/window/window/), [Path](https://docs.slint.dev/latest/docs/slint/reference/elements/path/), [spawn_local](https://docs.rs/slint/1.18.1/slint/fn.spawn_local.html), [#10912](https://github.com/slint-ui/slint/issues/10912).
- iced: [CHANGELOG](https://github.com/iced-rs/iced/blob/master/CHANGELOG.md), [daemon](https://docs.rs/iced/0.14.0/iced/fn.daemon.html), [#3496](https://github.com/iced-rs/iced/issues/3496), [#2572](https://github.com/iced-rs/iced/issues/2572), [#552](https://github.com/iced-rs/iced/issues/552), [#3111](https://github.com/iced-rs/iced/pull/3111).
- egui: [CHANGELOG](https://github.com/emilk/egui/blob/main/CHANGELOG.md), [eframe Cargo.toml @0.36.2](https://github.com/emilk/egui/blob/0.36.2/crates/eframe/Cargo.toml), [#8416](https://github.com/emilk/egui/issues/8416).
- libcosmic: [README](https://github.com/pop-os/libcosmic), [Cargo.toml](https://github.com/pop-os/libcosmic/blob/master/Cargo.toml), [#1131](https://github.com/pop-os/libcosmic/issues/1131).
- Freya: [README](https://github.com/marc2332/freya), [#2302](https://github.com/marc2332/freya/issues/2302).
- Xilem: [README](https://github.com/linebender/xilem). Floem: [README](https://github.com/lapce/floem), [#8](https://github.com/lapce/floem/issues/8). Makepad: [README](https://github.com/makepad/makepad), [#196](https://github.com/makepad/makepad/issues/196), [#1179](https://github.com/makepad/makepad/issues/1179).
- gpui: [gpui_linux/Cargo.toml](https://github.com/zed-industries/zed/blob/main/crates/gpui_linux/Cargo.toml), [gpui-component](https://github.com/longbridge/gpui-component).
- Flathub API: `https://flathub.org/api/v2/appstream/org.squidowl.halloy`.
- Older third-party size numbers exist (e.g. [Lukas Kalbertodt, 2023](http://lukaskalbertodt.github.io/2023/02/03/tauri-iced-egui-performance-comparison.html): iced 17 MB, egui 18 MB, unstripped, 2023 versions, no RAM data). They are too old and too differently built to compare, so section 3 measures instead.
