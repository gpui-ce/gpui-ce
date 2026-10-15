//! The smallest GPUI program: a window with a styled greeting.
//!
//! Run it with `cargo run --example hello_world`.

use gpui::{
    App, Bounds, Context, QuitMode, Window, WindowBounds, WindowOptions, colors::Colors, div,
    prelude::*, px, size,
};

struct HelloWorld;

impl Render for HelloWorld {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let colors = Colors::for_appearance(window);

        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_2()
            .bg(colors.background)
            .text_color(colors.text)
            .child(
                div()
                    .text_3xl()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child("Hello, GPUI!"),
            )
            .child(
                div()
                    .text_color(colors.disabled)
                    .child("Edit this file and run it again."),
            )
    }
}

fn main() {
    gpui_platform::application()
        .with_quit_mode(QuitMode::LastWindowClosed)
        .run(run);
}

/// Opens the example window. The browser player and the example tests call this too.
pub fn run(cx: &mut App) {
    let bounds = Bounds::centered(None, size(px(480.), px(320.)), cx);
    cx.open_window(
        WindowOptions::default().window_bounds(WindowBounds::Windowed(bounds)),
        |_window, cx| cx.new(|_cx| HelloWorld),
    )
    .unwrap();
    cx.activate(true);
}
