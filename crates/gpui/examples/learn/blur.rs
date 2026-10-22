//! CSS-style backdrop and content blur.
//!
//! `backdrop_blur` samples behind a translucent panel and a `deferred()` popover,
//! checking that overlays sort on top and include all content beneath them.
//!
//! `blur` filters text and colored chips as a group. Nested blur filters the
//! inner subtree twice, testing group textures at each nesting level.
//!
//! Gap-free adjacent blocks on a dark parent compare per-block blur, which exposes the
//! parent at seams, with parent blur, which keeps the seams continuous.

use gpui::{
    App, Bounds, Context, Render, Window, WindowBounds, WindowOptions, deferred, div, point,
    prelude::*, px, rgb, rgba, size,
};

struct BlurExample;

/// Fills the window with saturated tiles to show blur.
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

/// A deferred popover whose backdrop blur must sample the panel and background below.
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

/// Blurs the element and its contents as one group, like CSS `filter: blur`.
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

/// Blurs a row of colored chips as one group.
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
        .children(
            [0xef4444, 0x22c55e, 0x3b82f6]
                .into_iter()
                .map(|hex| div().w(px(48.)).h(px(48.)).rounded_md().bg(rgb(hex))),
        )
}

/// Nested blur tests isolated group textures up to `MAX_FILTER_DEPTH`.
/// The inner block is filtered twice and should look softer than the outer text.
fn nested_content_blurred() -> impl IntoElement {
    div()
        .absolute()
        .left(px(740.))
        .top(px(80.))
        .w(px(290.))
        .h(px(220.))
        .blur(px(3.))
        .bg(rgb(0x1e293b))
        .rounded_xl()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_4()
        .text_color(rgb(0xe2e8f0))
        .text_xl()
        .child("outer blur(3px)")
        .child(
            div()
                .w(px(190.))
                .h(px(100.))
                .blur(px(8.))
                .bg(rgb(0xf59e0b))
                .rounded_lg()
                .flex()
                .items_center()
                .justify_center()
                .text_color(rgb(0x111111))
                .text_2xl()
                .child("inner blur(8px)"),
        )
}

const SEAM_COLORS: [u32; 4] = [0xef4444, 0x22c55e, 0x3b82f6, 0xeab308];

/// A numbered seam-test block. `blur_each` filters it independently. Otherwise,
/// only a parent filter can blur it.
///
/// Use square corners because GPUI's axis-aligned rectangular content masks
/// cannot clip blurred children to a rounded parent, which would
/// expose the background at corners and obscure the seam test.
fn seam_block(i: usize, hex: u32, blur_each: bool) -> impl IntoElement {
    let block = div().flex_1().h_full();
    let block = if blur_each { block.blur(px(5.)) } else { block };
    block
        .bg(rgb(hex))
        .flex()
        .items_center()
        .justify_center()
        .text_color(rgb(0xffffff))
        .text_2xl()
        .child(format!("{}", i + 1))
}

/// Gap-free adjacent blocks blurred independently over a dark parent, like CSS
/// `filter: blur()` on each sibling.
///
/// Each block fades to transparent at its edges and composites independently.
/// These edges expose the parent by roughly `α_left · α_right`,
/// peaking at about 25% on the seam.
fn adjacent_per_block_blur() -> impl IntoElement {
    div()
        .absolute()
        .left(px(740.))
        .top(px(350.))
        .w(px(290.))
        .h(px(110.))
        .bg(rgb(0x050505))
        .flex()
        .children(
            SEAM_COLORS
                .into_iter()
                .enumerate()
                .map(|(i, hex)| seam_block(i, hex, true)),
        )
}

/// Blur the parent to filter all four adjacent opaque blocks as one group.
///
/// The blocks touch, so the group's interior has no transparency. The blur
/// stays continuous across seams, and only the outer edge fades, like applying
/// CSS blur to the wrapper.
fn adjacent_group_blur() -> impl IntoElement {
    div()
        .absolute()
        .left(px(740.))
        .top(px(510.))
        .w(px(290.))
        .h(px(110.))
        .bg(rgb(0x050505))
        .blur(px(5.))
        .flex()
        .children(
            SEAM_COLORS
                .into_iter()
                .enumerate()
                .map(|(i, hex)| seam_block(i, hex, false)),
        )
}

/// Labels test sections against the tiled background.
fn caption(text: &'static str, left: f32, top: f32) -> impl IntoElement {
    div()
        .absolute()
        .left(px(left))
        .top(px(top))
        .px_2()
        .py_1()
        .rounded_md()
        .bg(rgba(0x000000cc))
        .text_color(rgb(0xffffff))
        .text_sm()
        .child(text)
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
            .child(nested_content_blurred())
            .child(adjacent_per_block_blur())
            .child(adjacent_group_blur())
            .child(caption("nested content blur", 740., 50.))
            .child(caption(
                "adjacent — blur each block (seams, = CSS)",
                740.,
                322.,
            ))
            .child(caption(
                "adjacent — blur the parent (one group, clean)",
                740.,
                482.,
            ))
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
            size: size(px(1060.), px(760.)),
        };
        cx.open_window(
            WindowOptions::new().window_bounds(Some(WindowBounds::Windowed(bounds))),
            |_, cx| cx.new(|_| BlurExample),
        )
        .expect("failed to open window");
    });
}
