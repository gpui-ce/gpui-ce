# gpui - Community Edition

A community fork of [GPUI](https://gpui.rs), Zed's GPU-accelerated UI framework.

## Usage

```toml
[dependencies]
gpui = { package = "gpui-ce", version = "0.3" }
gpui_platform = { git = "https://github.com/gpui-ce/gpui-ce" }

# for test support...
[dev-dependencies]
gpui = { package = "gpui-ce", version = "0.3", features = ["test-support"] }
```

Or use the git version:

```toml
gpui = { package = "gpui", git = "https://github.com/gpui-ce/gpui-ce" }
```

Then use `gpui::{import}` as normal.

## FAQ

#### How does the project compare to other forks in the ecosystem?
Other efforts (namely WGPUI) are actively maintained, but have diverged quite a bit from mainline usage. They typically serve the interests of the projects that they're used within, leading to a diverse yet fragmented ecosystem. GPUI-CE strives to focus on the general use-case first, and over time, grow in the facilities to support the same outside adaptations through a single consistent API.

#### What is the long-term goal of GPUI-CE?
We'd like to be a premiere Rust GUI library! For the time being, we're working incrementally, in an effort to better understand the codebase and where is the right direction to take it, so we're okay being limited by mainline Zed. We will not stay this way forever! The spirit of the project is independence, so "limited" is loose, and we have and will continue to add features that mainline will never have. We will make your contribution work :)

If you'd like to join discussions and help us forge an path forward, please join the discord.

#### Can I use GPUI-CE with gpui-component?
100% Because we're a drop-in for GPUI, any component library or surrounding project should work 1:1 through the use of a [patch block](https://doc.rust-lang.org/cargo/reference/overriding-dependencies.html). DO NOTE: We track the latest upstream -- if there's breaking changes and the library you're pulling in hasn't updated yet, gpui-ce cannot help you. Otherwise, we treat any mismatches as bugs.

Example:
```toml
# If they're using the crates release
[patch.crates-io]
gpui = { git = "https://github.com/gpui-ce/gpui-ce", package = "gpui-ce" }

# If they're using the git remote
[patch."https://github.com/zed-industries/zed.git"]
gpui = { git = "https://github.com/gpui-ce/gpui-ce }
```

#### Is there a community I could... join?
For sure! Join the [discord](https://discord.gg/WYEmCKuv)


## Welcome to GPUI!

GPUI is a hybrid immediate and retained mode, GPU accelerated, UI framework for Rust, designed to support a wide variety of applications.

### Quick Start

Here's a complete example to get you started:

```rust
use gpui::*;
use gpui_platform_gpui_unofficial;
use std::f32::consts::TAU;

actions!(app, [Quit]);
actions!(Counter, [Increment]);

const RIPPLE_DURATION_FRAMES: f32 = 22.0;
const TITLEBAR_HEIGHT: f32 = 36.0;

// ── RippleElement ────────────────────────────────────────────────────────────

struct RippleElement {
    t: f32,
}

impl Element for RippleElement {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size = Size {
            width: relative(1.).into(),
            height: relative(1.).into(),
        };
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _state: &mut (),
        _window: &mut Window,
        _cx: &mut App,
    ) {
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut (),
        _prepaint: &mut (),
        window: &mut Window,
        _cx: &mut App,
    ) {
        if self.t <= 0.0 {
            return;
        }

        let t = self.t;
        let scale = 1.0 - (1.0 - t).powi(3);
        let alpha = (1.0 - t).powf(1.8);

        let center = bounds.center();
        let max_r = bounds.size.width.min(bounds.size.height) / 2.0;
        let radius = max_r * scale;
        let stroke_width = px(max_r.as_f32() * 0.28 * (1.0 - t * 0.7));

        let segments = 128usize;
        let mut builder = PathBuilder::stroke(stroke_width);
        builder.move_to(point(center.x + radius, center.y));
        for i in 1..=segments {
            let angle = (i as f32 / segments as f32) * TAU;
            builder.line_to(point(
                center.x + radius * angle.cos(),
                center.y + radius * angle.sin(),
            ));
        }

        if let Ok(path) = builder.build() {
            window.paint_path(path, hsla(0.6, 0.9, 0.88, alpha));
        }
    }
}

impl IntoElement for RippleElement {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

// ── Counter ──────────────────────────────────────────────────────────────────

struct Counter {
    value: i32,
    focus_handle: FocusHandle,
    ripple_t: f32,
    ripple_active: bool,
}

impl Counter {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        focus_handle.focus(window, cx);
        Self {
            value: 0,
            focus_handle,
            ripple_t: 0.0,
            ripple_active: false,
        }
    }

    fn increment(&mut self, _: &Increment, _window: &mut Window, cx: &mut Context<Self>) {
        self.value += 1;
        self.ripple_t = 0.001;
        self.ripple_active = true;
        cx.notify();
    }

    fn advance_ripple(&mut self, cx: &mut Context<Self>) {
        if !self.ripple_active {
            return;
        }
        self.ripple_t += 1.0 / RIPPLE_DURATION_FRAMES;
        if self.ripple_t >= 1.0 {
            self.ripple_t = 0.0;
            self.ripple_active = false;
        }
        cx.notify();
    }
}

impl Render for Counter {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.ripple_active {
            let entity = cx.entity().clone();
            window.on_next_frame(move |_window, cx| {
                entity.update(cx, |this, cx| this.advance_ripple(cx));
            });
        }

        let ripple_t = self.ripple_t;
        let value = self.value;
        let button_size = px(160.0);

        div()
            .id("counter-root")
            .key_context("Counter")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Counter::increment))
            .on_action(|_: &Quit, _window, cx| cx.quit())
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0x0f0f0f))
            // ── Titlebar ──────────────────────────────────────────────────
            // The drag region is registered via .window_control_area() on the
            // div itself — no custom element needed.
            .child(
                div()
                    .id("titlebar")
                    .w_full()
                    .h(px(TITLEBAR_HEIGHT))
                    .flex()
                    .items_center()
                    .flex_shrink_0()
                    // Marks this div's bounds as the window drag region
                    .window_control_area(WindowControlArea::Drag)
                    // Title — centred
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .justify_center()
                            .text_color(rgba(0xffffff55))
                            .text_size(px(13.0))
                            .child("Counter"),
                    )
                    // Close button — right side
                    // Rendered after the drag region so its hitbox wins on click
                    .child(
                        div()
                            .id("close-btn")
                            .w(px(TITLEBAR_HEIGHT))
                            .h(px(TITLEBAR_HEIGHT))
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .text_color(rgba(0xffffff33))
                            .text_size(px(14.0))
                            .child("✕")
                            // Override: mark this as a non-drag area so clicks
                            // don't get swallowed by the parent drag region
                            .window_control_area(WindowControlArea::Close)
                            .on_click(|_event, window, _cx| {
                                window.remove_window();
                            }),
                    ),
            )
            // ── Counter content ───────────────────────────────────────────
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap(px(24.0))
                    .child(
                        div()
                            .id("ripple-button")
                            .w(button_size)
                            .h(button_size)
                            .rounded_full()
                            .border(px(2.0))
                            .border_color(rgb(0x3b82f6))
                            .bg(rgb(0x1d4ed8))
                            .cursor_pointer()
                            .overflow_hidden()
                            .relative()
                            .on_click(cx.listener(|this, _event, window, cx| {
                                this.increment(&Increment, window, cx);
                            }))
                            .child(
                                div()
                                    .absolute()
                                    .inset_0()
                                    .child(RippleElement { t: ripple_t }),
                            )
                            .child(
                                div()
                                    .absolute()
                                    .inset_0()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .text_color(rgb(0xffffff))
                                    .text_size(px(48.0))
                                    .font_weight(FontWeight::BOLD)
                                    .child(format!("{}", value)),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(12.0))
                            .child(kbd_hint("space", "increment"))
                            .child(
                                div()
                                    .text_color(rgba(0xffffff18))
                                    .text_size(px(11.0))
                                    .child("·"),
                            )
                            .child(kbd_hint("q", "quit")),
                    ),
            )
    }
}

fn kbd_hint(key: &'static str, label: &'static str) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap(px(6.0))
        .child(
            div()
                .px(px(8.0))
                .py(px(3.0))
                .rounded(px(5.0))
                .border(px(1.0))
                .border_color(rgba(0xffffff18))
                .bg(rgba(0xffffff0a))
                .text_color(rgba(0xffffff55))
                .text_size(px(11.0))
                .child(key),
        )
        .child(
            div()
                .text_color(rgba(0xffffff28))
                .text_size(px(11.0))
                .child(label),
        )
}

// ── Main ─────────────────────────────────────────────────────────────────────

fn main() {
    gpui_platform_gpui_unofficial::application().run(|cx: &mut App| {
        cx.bind_keys([
            KeyBinding::new("space", Increment, Some("Counter")),
            KeyBinding::new("q", Quit, Some("Counter")),
        ]);

        let bounds = Bounds::centered(None, size(px(360.), px(420.)), cx);

        let _ = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some(SharedString::from("Counter")),
                    appears_transparent: false,
                    traffic_light_position: None,
                }),
                window_background: WindowBackgroundAppearance::Opaque,
                is_resizable: false,
                window_decorations: Some(WindowDecorations::Client),
                ..Default::default()
            },
            move |window, cx| cx.new(|cx| Counter::new(window, cx)),
        );
    });
}

```

### Dependencies

#### macOS

GPUI uses Metal for rendering on macOS. You'll need:

- Install [Xcode](https://apps.apple.com/us/app/xcode/id497799835?mt=12) from the macOS App Store or [Apple Developer](https://developer.apple.com/download/all/) website
- Install Xcode command line tools:
  ```sh
  xcode-select --install
  ```
- Point to your Xcode installation:
  ```sh
  sudo xcode-select --switch /Applications/Xcode.app/Contents/Developer
  ```

## The Big Picture

GPUI provides three complementary layers that work together:

- **`Entity<T>`** - A smart pointer (like `Rc`) that provides a strongly-typed reference to data managed by GPUI's storage system, allowing you to read or update that data through a context.

- **Views** - `Entity`s that can render themselves by implementing the `Render` trait. Most of your UI will be built with views, using declarative elements like `div()`.

- **Elements** - The building blocks of rendering. Views return trees of elements, which GPUI turns into pixels. Need custom layout or performance optimization? Drop down to the element layer.

In practice, you'll typically:
1. Create an `Entity` to hold your state
2. Make it a `View` by implementing `Render`
3. Use `div()` and other elements to describe your UI
4. Drop down to custom elements only when needed

Key methods on `Entity<T>` include:
- `read(&self, cx: &App)` - Read the data
- `update(&self, cx: &mut App, f)` - Mutate the data
- `entity_id(&self)` - Get the unique identifier
- `downgrade(&self)` - Get a non-retaining `WeakEntity`

See the `AppContext` trait, `Render` trait, and `Element` trait to learn more.

## Cross-platform Support

### Platform Matrix

| Platform | Windowing | Graphics | Text |
|----------|-----------|----------|------|
| Windows | Win32 | DirectX 11 | DirectWrite |
| macOS | Cocoa/AppKit | Metal | Core Text |
| Linux (X11) | X11 (via x11rb) | wgpu (Vulkan/GL) | cosmic-text |
| Linux (Wayland) | Wayland (via wayland-client) | wgpu (Vulkan/GL) | cosmic-text |
| Web | Web APIs | wgpu (WebGPU/WebGL) | cosmic-text |
| Test | Virtual | Virtual | Virtual |

### Graphics Backends

- **Windows**: DirectX 11 (via `windows` crate, Direct3D11 APIs)
- **macOS**: Metal (via `metal` crate, `MetalRenderer` struct)
- **Linux**: wgpu (via `gpui_wgpu` crate)
- **Web**: wgpu (WebGPU/WebGL backends)
- **Test**: Virtual

### Text Rendering

- **Windows**: DirectWrite
- **macOS**: Core Text
- **Linux/Web**: cosmic-text (RustyBuzz for shaping, fontdb for management)

### Feature Flags

From `crates/gpui_platform/Cargo.toml`:

| Feature | Purpose |
|---------|---------|
| `x11` | Enable X11 support on Linux |
| `wayland` | Enable Wayland support on Linux |
| `font-kit` | Font support for macOS |
| `test-support` | Enable virtual test platform |
| `screen-capture` | Enable screen capture (Windows/Linux) |
| `runtime_shaders` | Runtime shader compilation for macOS |

### Platform-Specific Details

- **macOS**: Native Metal via `metal` crate with `MetalRenderer` managing devices, command queues, and pipeline states
- **Windows**: Native DirectX 11 via `windows` crate with Direct3D11 APIs
- **Linux**: wgpu-based renderer supporting Vulkan/GL backends for both Wayland and X11
- **Web**: wgpu configured for WebGPU and WebGL

## Other Resources

- **Actions**: User-defined structs for converting keystrokes into logical operations. See `action` module.
- **Platform services**: Methods like `quit the app` or `open a URL` on `app::App`
- **Async executor**: Integrated with the platform's event loop. See `executor` module
- **Testing**: `[gpui::test]` macro and `TestAppContext` for simulating platform input. See `app::test_context` and `test` modules

Currently, the best way to learn about these APIs is to read the Zed source code or ask questions in the [Zed Discord](https://zed.dev/community-links).
