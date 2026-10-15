//! A clock pinned to the bottom edge of the screen as a Wayland layer-shell surface.
//!
//! `WindowKind::LayerShell` places the window on a compositor layer instead of a normal
//! toplevel. `LayerShellOptions` picks the anchor edges and margins. The view re-renders
//! twice a second to keep the clock current.
//!
//! Runs on Linux with the `wayland` feature only. Elsewhere `main` panics with an explanation.
//!
//! Run it with `cargo run --example layer_shell --features wayland`.

#[cfg(all(target_os = "linux", feature = "wayland"))]
fn main() {
    linux::main();
}

#[cfg(not(all(target_os = "linux", feature = "wayland")))]
fn main() {
    panic!("This example requires Linux and the `wayland` feature.");
}

#[cfg(all(target_os = "linux", feature = "wayland"))]
mod linux {
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use gpui::{
        App, Bounds, Context, FontWeight, LinuxWindowBackground, QuitMode, Size, Window,
        WindowBounds, WindowKind, WindowOptions, div, layer_shell::*, point, prelude::*, px, rems,
        rgba, white,
    };

    struct Clock;

    impl Clock {
        fn new(cx: &mut Context<Self>) -> Self {
            cx.spawn(async move |this, cx| {
                loop {
                    if this.update(cx, |_, cx| cx.notify()).is_err() {
                        break;
                    }
                    cx.background_executor()
                        .timer(Duration::from_millis(500))
                        .await;
                }
            })
            .detach();
            Clock
        }
    }

    impl Render for Clock {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            let seconds = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();

            div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_size(rems(4.5))
                .font_weight(FontWeight::EXTRA_BOLD)
                .text_color(white())
                .bg(rgba(0x00000044))
                .rounded_xl()
                .child(format!(
                    "{:02}:{:02}:{:02}",
                    (seconds / 3600) % 24,
                    (seconds / 60) % 60,
                    seconds % 60
                ))
        }
    }

    pub fn main() {
        gpui_platform::application()
            .with_quit_mode(QuitMode::LastWindowClosed)
            .run(run);
    }

    /// Opens the layer-shell clock. The browser player and the example tests call this too.
    pub fn run(cx: &mut App) {
        let options = WindowOptions::new()
            .titlebar(None)
            .window_bounds(Some(WindowBounds::Windowed(Bounds {
                origin: point(px(0.), px(0.)),
                size: Size::new(px(500.), px(200.)),
            })))
            .app_id(Some("gpui-layer-shell-example".to_string()))
            .kind(WindowKind::LayerShell(LayerShellOptions {
                namespace: "gpui".to_string(),
                anchor: Anchor::LEFT | Anchor::RIGHT | Anchor::BOTTOM,
                margin: Some((px(0.), px(0.), px(40.), px(0.))),
                keyboard_interactivity: KeyboardInteractivity::None,
                ..Default::default()
            }))
            .linux_window_background(LinuxWindowBackground::Transparent);
        cx.open_window(options, |_, cx| cx.new(Clock::new)).unwrap();
    }
}
