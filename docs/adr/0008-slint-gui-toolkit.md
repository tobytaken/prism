# Prism's GUI is built with Slint

Prism's native GUI ([ADR-0007](0007-native-rust-gui.md)) is built with Slint, using its winit backend and the FemtoVG (OpenGL) renderer. A [spike](https://github.com/tobytaken/prism-electron/issues/102) built the same hardest screen in gpui and in Slint: the C · Tracks GPU workspace, with the draggable power limit, the inline fan curve editor, the staged-edit strip, a text field, and light/dark following the system. Both passed every check the spike ran on NVIDIA and Wayland. Slint won clearly on the two criteria the rewrite exists for. Its release binary was 15.8 MiB against 28.6 MiB, it used about 35 % less memory at idle (81 against 123 MiB PSS), and it took about a third of the VRAM. It also brings a stable 1.x API, accessibility, a tray icon and edge resizing for frameless windows, and it reproduces the design's letter-spaced mono labels, which gpui can't.

## Considered Options

- **gpui** (via gpui-kit). It has a real drawing canvas and a rich component kit, and the look closest to Zed. Rejected for its larger footprint, its pre-1.0 API pinned to a weekly snapshot, and its Vulkan instance path, which also loads other GPUs' drivers and risks waking a suspended NVIDIA GPU on hybrid laptops.
- **iced**. It has the strongest canvas, but no accessibility, about one breaking release a year, and a reported NVIDIA Wayland redraw stall. It was the reserve and wasn't needed.

## Consequences

- **Charts are path data, not a canvas.** Slint has no immediate-mode drawing API. Live charts and the fan curve editor are `Path` elements whose commands Rust rebuilds when the data changes, and dashed lines have to be built from segments. This was cheap at Prism's 1 Hz sample rate. Revisit it if the charts need much higher rates or many more series.
- **The view layer is a `.slint` file over Rust state.** State and intents live in a Slint `global` implemented in Rust, and the `.slint` file only lays out and draws what it is given. That keeps the toolkit-agnostic UI core from ADR-0007 honest, but the glue code that copies UI-core state into Slint properties and models is a cost.
- **Licence.** Slint's royalty-free licence requires an in-app "uses Slint" attribution; the alternative is GPLv3. [Which Slint licence does Prism ship under?](https://github.com/tobytaken/prism-electron/issues/108) decides which.
