# PROTOTYPE, throwaway: toolkit spike

Wayfinder ticket **Which toolkit does Prism use? (spike)** (tobytaken/prism-electron#102), on the map **Rewrite Prism as a native Rust GUI** (#96).

This workspace builds the same screen twice: direction **C · Tracks**, the GPU workspace inside its custom title bar and status bar. It has the Power track with a draggable limit line and the pending line, the Fans track with the inline fan-curve editor and live marker, the fixed staged-edit strip, a Ctrl+K command palette (to test text input), and light/dark themes that follow the system live.

| Crate | What |
|---|---|
| `core/` | Toolkit-agnostic UI core: simulated or real NVML telemetry (read-only), staged edits, fan-curve hit-testing and dragging, history, stats, commands. The design tokens live in `core/src/theme.rs`, the one place both apps read colours from. `cargo test -p spike-core`. |
| `gpui/` | gpui via `gpui-kit =0.7.1` (`gpui-pre 0.3.8`, gpui-component `Input` for the palette). |
| `slint/` | Slint `=1.18.1`, winit backend + FemtoVG (OpenGL), `ui/app.slint`. |
| `fonts/` | Inter and JetBrains Mono (OFL), embedded by both apps. |

Nothing writes to a GPU. **Apply** only moves the staged values into the simulated applied state.

## Run

```sh
cd prototypes/toolkit-spike
cargo run --profile spike -p spike-gpui  -- --sim      # or spike-slint
```

- `--sim`: two simulated GPUs, as in the HTML prototype. Without it the app reads all NVIDIA GPUs through NVML (read-only).
- `--light` / `--dark`: override the system theme. The title-bar THEME button cycles System → Light → Dark.
- `SPIKE_SELFTEST=1`: sends scripted pointer and keyboard input through the toolkit's real event dispatch while the 1 Hz updates run. It drags the power line, drags a curve point, then opens the palette, types "perf" and presses Enter. Results go to stderr.
- `SPIKE_STATS=1`: logs the frame count every 10 s.

Measured builds use `cargo build --release` (fat LTO, `codegen-units=1`, `strip`, `panic=abort`).

The results, measurements and verdict are on the ticket.

## Measure

`measure/` holds the throwaway harness. `measure.py` launches a release binary and samples `/proc/<pid>/smaps_rollup`, CPU, the frame counter and `nvidia-smi`. `kwin.sh` minimizes and restores the spike window through KWin scripting, and `theme_test.sh` + `mock_portal.py` flip the XDG `color-scheme` on a private D-Bus session (`dbus-run-session -- measure/theme_test.sh <binary> <tag>`). Raw output is in `measure/results.txt`, and screenshots are in `screenshots/`.
