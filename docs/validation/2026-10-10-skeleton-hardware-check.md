# Walking skeleton hardware validation

The 10 October 2026 pass for [issue #129](https://github.com/tobytaken/prism-electron/issues/129) checked the walking skeleton and its palette on real hardware, covering what the toolkit spike ([#102](https://github.com/tobytaken/prism-electron/issues/102)) and CI can't. Minimizing, screen off, suspend/resume, Flatpak with a matching `GL.nvidia`, and raising the window on a second launch **pass** on this KDE Plasma Wayland desktop. Two checks **fail**:

- IME composition draws the placeholder under the composing text ([#163](https://github.com/tobytaken/prism-electron/issues/163)).
- A Flatpak whose `GL.nvidia` doesn't match the driver gives the wrong advice ([#165](https://github.com/tobytaken/prism-electron/issues/165)), or with no GL at all keeps running with no window ([#166](https://github.com/tobytaken/prism-electron/issues/166)).

Three more findings outside the checks have their own issues:

- Charts hide suspended time ([#164](https://github.com/tobytaken/prism-electron/issues/164)).
- Release builds lack the spike's size profile ([#167](https://github.com/tobytaken/prism-electron/issues/167)).
- The first launch after an update keeps about 40 MiB of shader compiler heap ([#169](https://github.com/tobytaken/prism-electron/issues/169)).

GNOME Wayland and a PRIME on-demand laptop **weren't tested**, because this host has neither. [#168](https://github.com/tobytaken/prism-electron/issues/168) carries them.

Tested revision: `e2f0bbf95ab7cd9d8a004b203cc4a70ef1b59dff`. No production code was changed during validation. [Machine-readable evidence](2026-10-10-skeleton-hardware-check.json) preserves the observations below. Full local logs, CSVs, screenshots and a `measurements.txt` of figures read during the pass are in the ignored `artifacts/issue-129/` directory. Times are CEST.

## Host and execution

- CachyOS, kernel `7.2.9-2-cachyos`, KDE Plasma 6.7.5 Wayland (KWin 6.7.5), with three outputs (3840×2160 at 1.25, 2560×1440 at 1, 3840×2160), all driven by the NVIDIA GPU.
- NVIDIA GeForce RTX 5090, driver `615.78.08`, plus an AMD Radeon 610M iGPU. Mesa 26.2.4.
- `prism` built with `cargo build --locked --release -p prism` (rustc 1.97.1, Slint 1.18.1). It read GPU telemetry in-process through NVML, with no `prismd`, and the status bar read “PRISMD NOT CONNECTED · MONITORING IN-PROCESS”.
- Measurement:
  - Frames were counted with Slint's `SLINT_DEBUG_PERFORMANCE=refresh_lazy,console`, which prints the frames drawn each second.
  - Memory came from `smaps_rollup`, and VRAM from `nvidia-smi -q -d PIDS`.
  - Sampling cadence came from the voluntary context switches of the UI and sampler threads in `/proc`, since `strace` and `perf` weren't available.
- A KWin script minimized and restored the window and read whether it was minimized and active. `ydotool` typed through uinput, so its keys passed through fcitx5 like a real keyboard's.
- The user approved the screen-off, suspend and key-injection steps and woke the machine from suspend.

## Results

| Check | Result | Evidence |
| --- | --- | --- |
| IME composition in the palette | **Fail** (#163) | fcitx5 5.1.23 with Mozc. Composition itself works: typing `nihon` showed the preedit `にほn` inline, Space converted it to `日本`, and Enter committed it without running the highlighted “Go to Games”. Filtering waited for the commit, after which the palette showed “No matching commands”. But the “Type a command” placeholder stayed drawn under the preedit until the commit. |
| 30+ minutes minimized on NVIDIA | **Pass** | Minimized for 36 min 50 s (2210 s), sampled every 30 s for the first 35 min. RSS / PSS / USS stayed at 194.3 / 122.4 / 114.6 MiB (PSS varied by 0.2 MiB) and VRAM at 37 MiB. 2202 of the 2210 seconds drew no frames. The other 8 were the minimize transition and a 7 s burst about 3 min in, whose cause wasn't identified (probably a compositor or taskbar thumbnail request). |
| Telemetry history without gaps after restore | **Pass** | In a separate 6 min minimized run, both the sampler thread and the UI thread woke in each of the 359 measured seconds, so every reading was taken in and stamped as it arrived. After the 36 min run, the 5 m Temperature chart filled its whole width. Readings held back until restore would all have been stamped at the restore and piled up at the right edge. The chart joins across missing time (#164) and the skeleton has no range picker yet ([#135](https://github.com/tobytaken/prism-electron/issues/135)), so the chart alone can't show gaps. |
| Screen off (DPMS) | **Pass** | `kscreen-doctor --dpms off` for 30 s with the window visible: 28 s drew no frames. After the screen came back on, frames resumed at 1 fps, and the chart and readouts updated again. |
| Suspend/resume | **Pass** | `systemctl suspend` (deep) for 66 s. Afterwards Prism was alive, drew at 1 fps and read NVML again (34 °C, 56 W), with PSS 122.8 MiB and VRAM 37 MiB. Separately, the chart joined the samples on either side of the suspend with no gap. The history's monotonic clock stops during suspend and the chart breaks only at unknown readings (#164). |
| GNOME Wayland | **Not tested** (#168) | GNOME isn't installed on this host. |
| Flatpak with `GL.nvidia` | **Pass** | `GL.nvidia-615-78-08` matched the driver. Prism rendered on NVIDIA (`libEGL_nvidia` from the extension, 37 MiB VRAM) and read NVML in-process from the extension's `libnvidia-ml` (33 °C, 57 W, fan 30 %). |
| Flatpak, extension doesn't match the driver | **Fail** (#165, #166) | With `FLATPAK_GL_DRIVERS=nvidia-614-00-00:default`, how Flatpak falls back after a driver update, Prism opened on Mesa's software renderer. It showed “No GPU telemetry” because `libnvidia-ml.so.1` couldn't be opened, then told the user to check the host driver, which was fine. With no `default` fallback, so no usable GL at all, Prism ran with no window and no error, holding the single-instance name. |
| PRIME on-demand laptop | **Not tested** (#168) | This host is a desktop with every output on the RTX 5090, so the dGPU never runtime-suspends (`runtime_status` stays `active`). |
| Second launch raises a minimized window | **Pass on KDE; GNOME not tested** (#168) | Launched from KRunner, the second instance received activation token `kwin-159` and handed it over, and KWin then reported the window not minimized and active. Launched from a shell or with `kioclient exec`, it got no token and the window stayed minimized. Wayland only unminimizes a window through activation, which needs the token ([ADR-0012](../adr/0012-quit-on-close-no-background-presence.md)). Desktop launchers pass one. In those token-less cases `prism` still printed “its window was raised instead”, which overstates what happened. |

## Baseline

| | Spike (#102) | This pass |
| --- | --- | --- |
| Binary | 15.8 MiB, stripped, spike profile | **29.8 MiB as built, 22.8 MiB stripped**; 16.9 MiB with the spike's profile (#167) |
| Idle, real NVML, warm shader cache: RSS / PSS / USS | 170 / 101 / 91 MiB | **131.6 / 68–70 / 64.2 MiB** (65.0 MiB PSS with the spike's profile) |
| Idle PSS, cold shader cache | — | 107 MiB with the shader disk cache disabled; 122 MiB on the first launch of a new build |
| VRAM on the RTX 5090 | 35 MiB | **37 MiB** |
| Flatpak PSS, first launch | — | 170.8 MiB |

The issue quotes the spike's ~81 MiB PSS, which was measured on simulated data. Its real-NVML figure, 101 MiB, is the comparable one. Idle memory was measured as in the spike: a 20 s settle, then samples every 5 s for 60 s with the pointer outside the window, which gave 69.9 MiB PSS. Four further runs, sampled for 30 s, gave 68.1–68.5 MiB.

**The first launch of a new build keeps its shader compiler memory for as long as Prism runs.** The minimized run's 122 MiB came from the first launch of a freshly built binary. With `__GL_SHADER_DISK_CACHE=0` the release build idles at 107 MiB PSS with 40.7 MiB of heap, against 7.2 MiB of heap with a warm cache. The NVIDIA driver compiles Prism's shaders on the first launch after a new build or driver update, and that heap isn't returned while Prism runs. The Flatpak keeps its own cache, so its first launch was cold too. As a result, the first run after every update uses about 37–52 MiB more (#169).

**The release profile isn't the spike's.** The workspace has no `[profile.release]`, so the binary is unstripped and built with Cargo's default LTO and codegen units. Building with the spike's settings (fat LTO, `codegen-units=1`, `strip`, `panic=abort`) gives 16.9 MiB, close to the spike's 15.8 MiB.

**Follow-up, 13 October 2026 ([#167](https://github.com/tobytaken/prism-electron/issues/167)):** the workspace now has the spike's release profile, except that panics unwind, as the README explains. The packaged binary, built with `cargo build --locked --release -p prism` in AlmaLinux 8 as CI builds it, is **24.9 MiB** against the spike's 15.8 MiB. Most of the gap is the GUI's growth since this pass. At `e2f0bbf`, the spike's full settings, `panic=abort` included, give 16.8 MiB with rustc 1.99.0 (16.9 MiB above was rustc 1.97.1). At `715b2bd`, Cargo's default profile gives 47.4 MiB, or 30.3 MiB stripped. Adding `panic = "abort"` to the workspace profile gives 22.0 MiB, so unwinding accounts for 2.9 MiB of the gap. The [evidence](2026-10-10-skeleton-hardware-check.json) keeps these under `releaseProfileFollowUp`.

## Not covered by this pass

- GNOME Wayland, second-launch focus on GNOME, and a PRIME on-demand laptop (#168).
- The Flatpak was the release binary repackaged with `flatpak build-init`/`build-finish` and the manifest's `finish-args`, not built by `flatpak-builder` from the manifest. The host's glibc 2.44 matches the 26.08 runtime's. This checks the sandbox, `GL.nvidia` and NVML, not the manifest's build. The CI Flatpak bundle should be rechecked when [#154](https://github.com/tobytaken/prism-electron/issues/154) ships it.
- A `GL.nvidia` that really doesn't match the driver was simulated with `FLATPAK_GL_DRIVERS`, not reproduced with a driver update.
- IME was driven by injected keys through fcitx5 and Mozc, not typed by hand. ibus wasn't tested.
- The sampling cadence was measured over 6 minimized minutes, not the full 36.

## Cleanup

The test Flatpak, its local remote and the temporary desktop entry were removed. fcitx5 was returned to its inactive `keyboard-us` state. No `prismd` was run and no tuning was written.
