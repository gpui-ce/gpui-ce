# WGPUI

WGPUI is a fork of [gpui-ce](https://github.com/gpui-ce/gpui-ce) — itself a community fork of Zed's GPUI framework — with one central goal: **a single cross-platform rendering and windowing backend**. Where gpui-ce keeps per-OS code paths (Metal on macOS, Vulkan/D3D12 on Linux/Windows, Cocoa/Win32/Wayland windowing), WGPUI replaces all of that with a single [wgpu](https://github.com/gfx-rs/wgpu) + [winit](https://github.com/rust-windowing/winit) backend. The result is a framework that compiles and runs on Windows, macOS, Linux, and WebAssembly from one unified code path.

The public API is kept intentionally compatible with gpui-ce so existing applications require minimal changes to migrate.

---

## Adding to your project

```toml
[dependencies]
gpui = { git = "https://github.com/gpui-ce/wgpui" }

[dev-dependencies]
gpui = { git = "https://github.com/gpui-ce/wgpui", features = ["test-support"] }
```

Imports use the `gpui` crate name as normal:

```rust
use gpui::prelude::*;
use gpui::{App, Application, Context, Render, Window, div, px};
```

---

## Custom Device Gotcha

If you embed an external renderer that uses indirect draws with non-zero `firstInstance`
(for example, Helio scene rendering inside `WgpuSurfaceHandle`), your device must enable
`wgpu::Features::INDIRECT_FIRST_INSTANCE`.

Without this feature, many backends/drivers skip indirect draws where `firstInstance > 0`,
which can look like only the first object in a scene renders.

WGPUI now enables this feature when creating its internal device. If you provide your own
wgpu device/context around WGPUI, ensure you request the same feature set.

## Hello World

```rust
use gpui::{
    App, Application, Bounds, Context, SharedString, Window,
    WindowBounds, WindowOptions, div, prelude::*, px, rgb, size,
};

struct HelloWorld {
    text: SharedString,
}

impl Render for HelloWorld {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .items_center()
            .justify_center()
            .size(px(500.0))
            .bg(rgb(0x1e1e2e))
            .text_xl()
            .text_color(rgb(0xcdd6f4))
            .child(format!("Hello, {}!", &self.text))
    }
}

fn main() {
    Application::new().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(500.), px(500.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_, cx| cx.new(|_| HelloWorld { text: "World".into() }),
        )
        .unwrap();
    });
}
```

```
cargo run --example hello_world
```

---

## Core concepts

### Application and windows

Everything starts with `Application::new().run(|cx: &mut App| { ... })`. Inside the callback, open windows with `cx.open_window(options, |_, cx| cx.new(|_| MyView))`. A window requires a root view — any type that implements `Render`.

### Entity\<T\>

`Entity<T>` is a reference-counted handle to application state owned by GPUI. All reads and mutations go through the context:

```rust
// Create
let counter: Entity<Counter> = cx.new(|_cx| Counter { value: 0 });

// Read
let value = counter.read(cx).value;

// Mutate — notifies subscribers and queues a re-render
counter.update(cx, |counter, cx| {
    counter.value += 1;
    cx.notify();
});

// Downgrade to avoid cycles
let weak: WeakEntity<Counter> = counter.downgrade();
```

### Render

Implement `Render` on an `Entity<T>` to make it a "view" — a piece of UI that GPUI knows how to draw:

```rust
impl Render for Counter {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .text_xl()
            .child(format!("Count: {}", self.value))
    }
}
```

### RenderOnce

For UI components that are constructed just to be rendered once (stateless helpers, sub-components), implement `RenderOnce` and derive `IntoElement`:

```rust
#[derive(IntoElement)]
struct Badge {
    label: SharedString,
    color: Hsla,
}

impl RenderOnce for Badge {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        div()
            .px_2()
            .py_1()
            .rounded_md()
            .bg(self.color)
            .text_xs()
            .child(self.label)
    }
}
```

### Context types

| Type | When you have it | What it gives you |
|---|---|---|
| `&App` / `&mut App` | Anywhere a context is needed | Entity reads and updates, global state |
| `&mut Context<T>` | Inside `Entity<T>::update` | Everything in `App`, plus `cx.notify()`, `cx.emit()`, `cx.spawn()` |
| `&mut Window` | Render and event handlers | Focus, actions, input state, direct painting |
| `AsyncApp` / `AsyncWindowContext` | Inside `cx.spawn` closures | Async entity updates across await points |

### Layout (Tailwind-style)

WGPUI uses [Taffy](https://github.com/nicowillis/taffy) for flexbox/grid layout with a chainable API modelled on Tailwind CSS:

```rust
// Flex row, centered, with gap
div()
    .flex()
    .flex_row()
    .items_center()
    .justify_between()
    .gap_4()
    .p_4()
    .child(left_panel)
    .child(right_panel)

// Flex column, full height sidebar
div()
    .flex()
    .flex_col()
    .h_full()
    .w_64()
    .bg(colors.surface)
    .children(items.iter().map(|item| sidebar_row(item)))
```

```
cargo run --example layout
```

### Styling

Style methods mirror Tailwind classes. Interactive states use closures:

```rust
div()
    .id("my-button")           // required for interactive elements
    .px_4()
    .py_2()
    .rounded_md()
    .bg(colors.accent)
    .text_color(colors.selected_text)
    .cursor_pointer()
    .hover(|style| style.bg(colors.accent_hover))
    .active(|style| style.bg(colors.accent_active))
    .focus(|style| style.border_color(colors.accent))
    .child("Click me")
```

Conditional styling:

```rust
div()
    .when(is_selected, |this| this.bg(colors.selection))
    .when_some(error_message, |this, msg| {
        this.border_1().border_color(colors.error).child(msg)
    })
```

```
cargo run --example styling
```

### Interactive elements and events

Elements need an `.id(...)` to participate in hit-testing. Event handlers receive the event, window, and context:

```rust
div()
    .id("click-target")
    .on_click(cx.listener(|this: &mut MyView, event: &ClickEvent, _window, cx| {
        if event.click_count() == 2 {
            this.handle_double_click(cx);
        } else {
            this.handle_click(cx);
        }
    }))
    .on_hover(cx.listener(|this, is_hovered: &bool, _window, cx| {
        this.hovered = *is_hovered;
        cx.notify();
    }))
```

```
cargo run --example interactive_elements
```

### Components with `use_state`

For simple per-element state without a full `Entity`, use `window.use_state`:

```rust
fn counter_widget(window: &mut Window, cx: &mut App) -> impl IntoElement {
    let state: Entity<UseStateCounter> =
        window.use_state(cx, |_window, _cx| UseStateCounter { count: 0 });

    let count = state.read(cx).count;

    div()
        .child(format!("Count: {count}"))
        .child(
            div()
                .id("increment")
                .child("+")
                .on_click({
                    let state = state.clone();
                    move |_, _, cx| {
                        state.update(cx, |s, cx| { s.count += 1; cx.notify(); });
                    }
                }),
        )
}
```

```
cargo run --example creating_components
```

### Async tasks

`cx.spawn` runs on the foreground thread and can update entities. `cx.background_spawn` runs off-thread for heavy work. Both return `Task<T>` — drop the task to cancel it, or call `.detach()` to let it run independently:

```rust
fn start_work(&mut self, cx: &mut Context<Self>) {
    self.loading = true;
    cx.notify();

    // Heavy computation off the UI thread
    cx.spawn(async move |this, cx| {
        let result = cx.background_spawn(async {
            expensive_computation()
        }).await;

        // Update UI back on the foreground thread
        this.update(cx, |this, cx| {
            this.result = Some(result);
            this.loading = false;
            cx.notify();
        }).ok();
    })
    .detach();
}
```

```
cargo run --example async_tasks
```

### Animations

Wrap any element with `.with_animation` to animate style properties over time:

```rust
use gpui::{Animation, AnimationExt as _, Transformation, ease_in_out, bounce};
use std::time::Duration;

div()
    .size_16()
    .bg(colors.accent)
    .with_animation(
        "spin",
        Animation::new(Duration::from_secs(2)).repeat(),
        |style, delta| style.with_transform(Transformation::rotate(percentage(delta))),
    )

svg()
    .path(icon_path)
    .with_animation(
        "bounce",
        Animation::new(Duration::from_millis(800)).repeat(),
        move |svg, delta| {
            let y = bounce(ease_in_out(delta)) * 20.0;
            svg.with_transform(Transformation::translate(point(px(0.), px(-y))))
        },
    )
```

```
cargo run --example animation
```

### Custom drawing

Use the `canvas` element for direct wgpu-backed painting, or `PathBuilder` for vector shapes:

```rust
canvas(
    |_bounds, _window, _cx| {},   // prepaint
    |bounds, _state, window, _cx| {
        window.paint_quad(fill(
            Bounds { origin: bounds.origin, size: size(px(60.), px(40.)) },
            colors.accent,
        ));
    },
)
.w_full()
.h_32()
```

```
cargo run --example custom_drawing
```

### WgpuSurface — embedding 3D content

`WgpuSurface` lets you embed a raw wgpu render pass directly inside your UI tree. This is the unique capability that makes WGPUI suitable as a shell for 3D applications:

```rust
use gpui::{wgpu_surface, WgpuSurfaceHandle};

impl Render for My3DView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let handle = self.surface_handle.clone();
        div()
            .size_full()
            .child(
                wgpu_surface(move |surface: WgpuSurfaceHandle| {
                    handle.render(surface); // your wgpu draw calls here
                })
                .size_full(),
            )
    }
}
```

```
cargo run --example wgpu_surface_basic
cargo run --example wgpu_surface       # full Helio scene renderer integration
```

### Text

```rust
div()
    .text_xl()
    .font_weight(FontWeight::BOLD)
    .text_color(colors.text)
    .child("Bold headline")

// Inline styled text with highlights
div().child(
    StyledText::new("Error: file not found")
        .with_highlights(&[
            (0..5, HighlightStyle { color: Some(colors.error), ..Default::default() }),
        ])
)
```

```
cargo run --example text
```

### Actions and keybindings

Actions are zero-sized (or data-carrying) structs dispatched by keyboard shortcuts or code:

```rust
actions!(my_app, [Quit, Save, OpenFile]);

// Register a keymap
cx.bind_keys([
    KeyBinding::new("cmd-q", Quit, None),
    KeyBinding::new("cmd-s", Save, None),
]);

// Handle in an element
div()
    .on_action(cx.listener(|this, _: &Save, _window, cx| {
        this.save(cx);
    }))

// Dispatch programmatically
window.dispatch_action(Save.boxed_clone(), cx);
```

---

## Examples index

### Learn (start here)

| Example | What it shows | Command |
|---|---|---|
| `hello_world` | Minimal app, basic layout and colors | `cargo run --example hello_world` |
| `layout` | Flexbox rows/columns, grid, common patterns | `cargo run --example layout` |
| `styling` | Hover/active/focus states, conditional styles, theming | `cargo run --example styling` |
| `interactive_elements` | Click, double-click, hover, drag events | `cargo run --example interactive_elements` |
| `creating_components` | `use_state`, `RenderOnce`, `Render` side by side | `cargo run --example creating_components` |
| `async_tasks` | Foreground tasks, background tasks, progress updates | `cargo run --example async_tasks` |
| `animation` | `with_animation`, easing, transforms | `cargo run --example animation` |
| `custom_drawing` | `canvas`, `PathBuilder`, `paint_quad` | `cargo run --example custom_drawing` |
| `text` | Font sizes/weights, alignment, overflow, styled text | `cargo run --example text` |
| `wgpu_surface_basic` | Minimal wgpu render pass inside UI | `cargo run --example wgpu_surface_basic` |
| `wgpu_surface` | Helio 3D scene renderer embedded in UI | `cargo run --example wgpu_surface` |

### Legacy (reference)

| Example | Command |
|---|---|
| `scrollable` | `cargo run --example scrollable` |
| `uniform_list` | `cargo run --example uniform_list` |
| `input` | `cargo run --example input` |
| `tree` | `cargo run --example tree` |
| `gif_viewer` | `cargo run --example gif_viewer` |
| `image_loading` | `cargo run --example image_loading` |
| `focus_visible` | `cargo run --example focus_visible` |
| `tab_stop` | `cargo run --example tab_stop` |
| `gradient` | `cargo run --example gradient` |
| `window` | `cargo run --example window` |
| `window_shadow` | `cargo run --example window_shadow` |
| `window_positioning` | `cargo run --example window_positioning` |
| `opacity` | `cargo run --example opacity` |
| `svg` | `cargo run --example svg` |

### Benchmarks

```
cargo run --example data_table --release
cargo run --example paths_bench --release
cargo run --example pattern --release
cargo run --example shadow --release
```

---

## Testing

The `test-support` feature provides `TestAppContext`, a headless context for unit-testing views and entities without a real window:

```toml
[dev-dependencies]
gpui = { git = "https://github.com/gpui-ce/wgpui", features = ["test-support"] }
```

```rust
#[gpui::test]
fn test_counter(cx: &mut TestAppContext) {
    let counter = cx.new(|_| Counter { value: 0 });
    counter.update(cx, |c, cx| { c.value += 1; cx.notify(); });
    assert_eq!(counter.read(cx).value, 1);
}
```

---

## Building

```
cargo build
cargo check
./script/clippy     # or: ./script/clippy.ps1 on Windows
```

---

## License

Apache-2.0 — see [LICENSE-APACHE](LICENSE-APACHE).
