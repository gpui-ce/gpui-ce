# gpui - Community Edition

A community fork of [GPUI](https://gpui.rs), Zed's GPU-accelerated UI framework.

### Hello, GPUI! (Community)

A basic GPUI application:

```rust
use gpui::{
    App, Application, Bounds, Context, SharedString, Window, WindowBounds, WindowOptions, div,
    prelude::*, px, rgb, size,
};

struct HelloWorld {
    /// A shared string is an immutable string that can be cheaply cloned in GPUI
    text: SharedString,
}

/// This is the trait that distinguishes "views" from other entities.
/// Views are `Entity`'s which `impl Render` and drawn to the screen.
impl Render for HelloWorld {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap_3()
            .bg(rgb(0x181818))
            .size(px(500.0))
            .justify_center()
            .items_center()
            .text_xl()
            .text_color(rgb(0xffffff))
            .child(format!("Hello, {}!", &self.text))
    }
}

fn main() {
    Application::new().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(500.), px(500.0)), cx);
        let window_opts = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            ..Default::default()
        };

        cx.open_window(window_opts, |_, cx| {
            cx.new(|_| HelloWorld {
                text: "World".into(),
            })
        })
        .unwrap();

        // Bring window into focus
        cx.activate(true);
    });
}
```

### Examples

To run the an example:

```bash
cargo run -p gpui --example hello_world
```
