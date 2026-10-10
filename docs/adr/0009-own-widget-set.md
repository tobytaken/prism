# Prism draws its own widgets instead of Slint's std-widgets

Prism's Slint view layer ([ADR-0008](0008-slint-gui-toolkit.md)) uses its own small widget set built on Slint's primitives (`Rectangle`, `TouchArea`, `FocusScope`, `TextInput`, `Flickable`, `PopupWindow`), not the bundled `std-widgets`. The design direction, C · Tracks, depends on hard edges, inverted selection, mono uppercase labels and an orange "pending" state on every control. `std-widgets` styles (fluent, material, cupertino, cosmic, qt) are picked at build time and can only be recoloured through `Palette`, so none of them can reach that look. The only `std-widgets` import is `Palette.color-scheme`, read to follow the desktop's light/dark preference. [How are C · Tracks' design tokens and widgets built in Slint?](https://github.com/tobytaken/prism-electron/issues/106) decided this together with the tokens. Those tokens live in one `theme.slint` file so that later theming only swaps values.

## Considered Options

- **`std-widgets` with `Palette` overrides.** Rejected because the result looks like a recoloured Fluent or Material app rather than C.
- **Mixed: own visual widgets, `std-widgets` for scroll views and line edits.** Rejected because the scrollbars and text fields would then follow a different style from everything around them.

## Consequences

- **Focus and accessibility are Prism's job.** Each widget implements keyboard focus, with a keyboard-only focus ring in `ink` rather than orange, since orange already means pending. Each widget also declares its own `accessible-role`, label, value and actions. Without those, AccessKit sees nothing. A widget isn't done until it declares them.
- **Pending is a widget state.** Steppers, segmented buttons, toggles and selects take a pending value and draw C's `→ value` themselves, so the staged-edit look can't drift between views.
- **The set grows only on demand.** A one-off control stays in its view until a second screen needs it.
