//! A button that counts its clicks: state, an event handler, and `notify`.
//!
//! The view owns `count`. Clicking runs a listener that changes it and calls
//! `cx.notify()`, which tells GPUI to render the view again.
//!
//! Run it with `cargo run --example counter`.

use gpui::{
    App, Bounds, Context, QuitMode, Window, WindowBounds, WindowOptions, colors::Colors, div,
    prelude::*, px, size,
};

struct Counter {
    count: usize,
}

impl Render for Counter {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = Colors::for_appearance(window);

        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_4()
            .bg(colors.background)
            .text_color(colors.text)
            .child(div().text_3xl().child(self.count.to_string()))
            .child(
                div()
                    .id("increment")
                    .px_4()
                    .py_2()
                    .rounded_lg()
                    .bg(colors.selected)
                    .text_color(colors.selected_text)
                    .cursor_pointer()
                    .hover(|style| style.opacity(0.9))
                    .active(|style| style.opacity(0.8))
                    .child("Add one")
                    .on_click(cx.listener(|counter, _event, _window, cx| {
                        counter.count += 1;
                        cx.notify();
                    })),
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
    let bounds = Bounds::centered(None, size(px(360.), px(240.)), cx);
    cx.open_window(
        WindowOptions::default().window_bounds(WindowBounds::Windowed(bounds)),
        |_window, cx| cx.new(|_cx| Counter { count: 0 }),
    )
    .unwrap();
    cx.activate(true);
}
