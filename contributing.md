# Contributing

Follow these guidelines when changing Longshot or its shared UI components.

## Keep functions at one abstraction level

Each function should have a clear responsibility and operate at that responsibility's abstraction level.

- Views describe layout. Keep arithmetic, image analysis, page geometry, validation, and state transitions in model or geometry helpers.
- A view that composes the sidebar, preview, and inspector should delegate each component's layout to its own helper.
- Compute display values and labels outside layout composition when they require calculations or domain knowledge.
- Extract helpers to clarify responsibilities, rather than breaking every expression into a separate function.

## Prefer data-oriented design

Prefer structs with methods over standalone free functions. Group related data in a struct and put the operations, calculations, and derived values that belong to that data on the struct itself.

Use constructors and methods to express the responsibilities of models, geometry, and detection settings. Keep free functions for operations that have no natural data owner; do not introduce an empty struct solely to hold unrelated functions.

## Use the existing UI vocabulary

Use `vse_ui` as the source of styling, typography, and spacing. Inspect its existing helpers before adding anything new.

- Use `theme::spacing()` tokens for padding and gaps. Do not introduce one-off pixel values or a parallel spacing system.
- `widget::button` already supplies the standard button style. Use predefined variants such as `theme::button::suggested`, `icon`, and navigation styles when appropriate.
- Use existing container styles and semantic text helpers such as `widget::text::body`, `caption`, and `title3`.
- Do not recreate the shared theme, palette, button styles, or typography locally.
- If a shared component is missing, adapt its implementation from libcosmic into `vse_ui`, then use that component here.

Keep application-specific dimensions and canvas geometry separate from shared theme tokens. These belong in geometry helpers or the existing metrics module, rather than scattered through views.

## Prefer readable composition

Minimize nesting and preserve the flow from content to its wrappers. Prefer `Apply` when wrapping widgets:

```rust
use vse_ui::{Apply, theme, widget};

let action = widget::text::body("Apply cut")
    .apply(widget::button)
    .style(theme::button::suggested)
    .on_press_maybe(can_apply.then_some(Message::ConfirmCut));

let controls = widget::column![action]
    .spacing(theme::spacing().space_xxs)
    .apply(widget::container)
    .padding(theme::spacing().space_s)
    .style(theme::container::card);
```

Some nesting is useful; avoid deeply nested widget constructors when a chain or a named component reads more clearly.

## Keep the UI responsive

- Run image decoding, pixel analysis, PDF generation, and other expensive work in background tasks. An async function alone does not move CPU work off the UI thread; use a blocking worker for that work.
- Debounce checks triggered by continuous movement, and discard outdated results when the cut or document changes.
- Sharing image data through `Arc` is fine. Inspect only the row or region needed by the operation.
- Keep detection settings configurable in code. Show meaningful results in the UI without exposing implementation details unnecessarily.
- Give long-running operations a clear busy state and meaningful progress. Prevent duplicate submissions while an operation is running.

## Disable unavailable actions

An unavailable button should have no press handler. Use `on_press_maybe(None)` or leave the handler unset, allowing the shared widget to display its disabled state. Do not attach a handler that merely does nothing.

## Validate changes

Use checks appropriate to the change:

```sh
cargo fmt --check
cargo check --offline
cargo test --offline
```

Add regression tests for meaningful behavior changes and bugs, especially page boundaries, asynchronous results, and export behavior. Avoid tests that merely repeat layout or styling implementation details. Check the running UI when changing interaction or appearance.
