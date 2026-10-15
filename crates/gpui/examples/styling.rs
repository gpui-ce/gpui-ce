//! A gallery of backgrounds, borders, corners, shadows, opacity, blur, and interactive states.
//!
//! Each tile demonstrates a method from `Styled`. Gradients use `linear_gradient`, and
//! `backdrop_blur` frosts content behind a translucent element. The last tile changes its style on
//! hover and press; those interactions require an `.id(..)`.
//!
//! Run it with `cargo run --example styling`.

use gpui::{
    App, Bounds, Context, Div, FontWeight, QuitMode, Window, WindowBounds, WindowOptions,
    colors::Colors, div, linear_color_stop, linear_gradient, prelude::*, px, relative, rems, rgb,
    rgba, size,
};

struct Gallery;

impl Render for Gallery {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let colors = Colors::for_appearance(window);
        let width = window.viewport_size().width;
        let columns: u16 = if width < px(640.) {
            2
        } else if width < px(1000.) {
            3
        } else {
            4
        };

        let swatch = || div().w(relative(0.9)).h(px(96.)).rounded_lg();
        let gradient = |from, to| {
            linear_gradient(
                135.,
                linear_color_stop(rgb(from), 0.),
                linear_color_stop(rgb(to), 1.),
            )
        };
        let samples = [
            ("Solid background", swatch().bg(rgb(0x2a9bb0))),
            ("Linear gradient", swatch().bg(gradient(0x8a4fd1, 0xd08a2e))),
            (
                "Border width and color",
                swatch().border_4().border_color(rgb(0x2457ca)),
            ),
            (
                "Dashed border",
                swatch()
                    .border_2()
                    .border_dashed()
                    .border_color(colors.disabled),
            ),
            (
                "Corner radius",
                swatch()
                    .bg(rgb(0x3a9d5d))
                    .rounded_2xl()
                    .rounded_smoothing(0.),
            ),
            (
                "Corner smoothing",
                swatch()
                    .bg(rgb(0x3a9d5d))
                    .rounded_2xl()
                    .rounded_smoothing(1.),
            ),
            ("Shadow small", swatch().bg(colors.background).shadow_sm()),
            ("Shadow medium", swatch().bg(colors.background).shadow_md()),
            (
                "Shadow extra large",
                swatch().bg(colors.background).shadow_xl(),
            ),
            ("Opacity 40%", swatch().bg(rgb(0xc7463f)).opacity(0.4)),
            (
                "Backdrop blur",
                frosted(swatch().bg(gradient(0x2457ca, 0xc7463f))),
            ),
        ];
        let hover_and_press = swatch()
            .id("interactive")
            .flex()
            .items_center()
            .justify_center()
            .bg(colors.selected)
            .text_color(colors.selected_text)
            .shadow_sm()
            .cursor_pointer()
            .hover(|style| style.bg(rgb(0x1d4aa8)).shadow_xl())
            .active(|style| style.opacity(0.8))
            .child("Try me");

        div()
            .id("gallery")
            .size_full()
            .overflow_y_scroll()
            .p(rems(1.5))
            .bg(colors.background)
            .text_color(colors.text)
            .child(
                div()
                    .grid()
                    .grid_cols(columns)
                    .gap_4()
                    .children(samples.map(|(label, sample)| tile(label, sample, &colors)))
                    .child(tile("Hover and press", hover_and_press, &colors)),
            )
    }
}

/// A labelled tile. The preview is centered in a card so every tile lines up.
fn tile(label: &'static str, preview: impl IntoElement, colors: &Colors) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .min_w_0()
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .h(px(128.))
                .rounded_xl()
                .bg(colors.container)
                .border_1()
                .border_color(colors.separator)
                .child(preview),
        )
        .child(div().text_sm().text_color(colors.disabled).child(label))
}

/// Puts two colored circles on `base`, then covers them with a translucent layer that blurs them.
fn frosted(base: Div) -> Div {
    let circle = |size: f32, color: u32| {
        div()
            .absolute()
            .size(px(size))
            .rounded_full()
            .bg(rgb(color))
    };
    base.relative()
        .overflow_hidden()
        .child(circle(56., 0xffd166).top(px(12.)).left(px(16.)))
        .child(circle(72., 0x3a9d5d).bottom(px(8.)).right(px(24.)))
        .child(
            div()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .bg(rgba(0xffffff59))
                .backdrop_blur(px(8.))
                .font_weight(FontWeight::SEMIBOLD)
                .child("Frosted"),
        )
}

fn main() {
    gpui_platform::application()
        .with_quit_mode(QuitMode::LastWindowClosed)
        .run(run);
}

/// Opens the example window. The browser player and the example tests call this too.
pub fn run(cx: &mut App) {
    let bounds = Bounds::centered(None, size(px(960.), px(720.)), cx);
    cx.open_window(
        WindowOptions::default().window_bounds(WindowBounds::Windowed(bounds)),
        |_window, cx| cx.new(|_cx| Gallery),
    )
    .unwrap();
    cx.activate(true);
}
