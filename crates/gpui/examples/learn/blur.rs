//! Blur Filters Example
//!
//! Demonstrates the two CSS-style blur filters:
//!
//! 1. `backdrop_blur` — frosted glass: blurs whatever is rendered behind the element.
//!    Shown as a translucent panel, and again inside a `deferred()` popover (to prove the
//!    backdrop snapshot includes everything beneath an overlay, and that the overlay sorts on top).
//! 2. `blur` — content blur: blurs the element and its own children as a group.
//!    Shown with text and again with a row of colored chips.

use gpui::{
    App, Bounds, Context, Render, Window, WindowBounds, WindowOptions, deferred, div, point,
    prelude::*, px, rgb, rgba, size,
};

struct BlurExample;

/// A vivid, gap-free background so blur is obvious: a grid of saturated tiles filling the window.
fn busy_background() -> impl IntoElement {
    let palette = [
        0xef4444, 0xf97316, 0xeab308, 0x22c55e, 0x06b6d4, 0x3b82f6, 0x8b5cf6, 0xec4899,
    ];
    let mut next = 0usize;
    div()
        .absolute()
        .inset_0()
        .flex()
        .flex_col()
        .children((0..9).map(|_| {
            div().flex().flex_1().children(
                (0..8)
                    .map(|_| {
                        let hex = palette[next % palette.len()];
                        next += 1;
                        div()
                            .flex_1()
                            .h_full()
                            .bg(rgb(hex))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_color(rgb(0xffffff))
                            .text_xl()
                            .child("◆")
                    })
                    .collect::<Vec<_>>(),
            )
        }))
}

/// A translucent rounded panel that frosts the content behind it.
fn frosted_panel() -> impl IntoElement {
    div()
        .absolute()
        .left(px(60.))
        .top(px(120.))
        .w(px(360.))
        .h(px(200.))
        .rounded_xl()
        .bg(rgba(0xffffff30))
        .backdrop_blur(px(24.))
        .border_1()
        .border_color(rgba(0xffffff60))
        .flex()
        .items_center()
        .justify_center()
        .text_color(rgb(0x111111))
        .text_2xl()
        .child("backdrop_blur(24px)")
}

/// A popover painted via `deferred()` so it sits above everything; its backdrop blur must
/// still pick up the panel and background beneath it.
fn deferred_popover() -> impl IntoElement {
    deferred(
        div()
            .absolute()
            .left(px(260.))
            .top(px(260.))
            .w(px(300.))
            .h(px(150.))
            .rounded_lg()
            .bg(rgba(0x1e293b66))
            .backdrop_blur(px(12.))
            .border_1()
            .border_color(rgba(0xffffffaa))
            .flex()
            .items_center()
            .justify_center()
            .text_color(rgb(0xffffff))
            .text_xl()
            .child("deferred + backdrop_blur"),
    )
}

/// A self-blurred element (CSS `filter: blur`) — its own content is blurred as a group.
fn content_blurred() -> impl IntoElement {
    div()
        .absolute()
        .left(px(120.))
        .top(px(420.))
        .w(px(280.))
        .h(px(120.))
        .blur(px(5.))
        .bg(rgb(0x0f172a))
        .rounded_lg()
        .flex()
        .items_center()
        .justify_center()
        .text_color(rgb(0xfacc15))
        .text_3xl()
        .child("blur(5px) content")
}

/// A content-blurred element with richer content (a row of colored chips), so the `filter: blur`
/// effect — the element and its children blurred as one group — is clearly visible.
fn content_blurred_rich() -> impl IntoElement {
    div()
        .absolute()
        .left(px(120.))
        .top(px(580.))
        .w(px(280.))
        .h(px(120.))
        .blur(px(6.))
        .bg(rgb(0x1e293b))
        .rounded_lg()
        .flex()
        .items_center()
        .justify_center()
        .gap_3()
        .children([0xef4444, 0x22c55e, 0x3b82f6].into_iter().map(|hex| {
            div().w(px(48.)).h(px(48.)).rounded_md().bg(rgb(hex))
        }))
}

impl Render for BlurExample {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .relative()
            .size_full()
            .bg(rgb(0x000000))
            .child(busy_background())
            .child(frosted_panel())
            .child(content_blurred())
            .child(content_blurred_rich())
            .child(deferred_popover())
    }
}

fn main() {
    gpui_platform::application().run(|cx: &mut App| {
        cx.activate(true);
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();

        let bounds = Bounds {
            origin: point(px(100.), px(100.)),
            size: size(px(720.), px(760.)),
        };
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_, cx| cx.new(|_| BlurExample),
        )
        .expect("failed to open window");
    });
}
