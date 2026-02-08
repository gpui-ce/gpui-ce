//! Overflow Fade Example
//!
//! Shows how `overflow_fade_y(...)` and `overflow_fade_x(...)` soften
//! clipped scroll edges.
//!
//! Run with:
//! `cargo run --example overflow_fade`

#[path = "../prelude.rs"]
mod example_prelude;

use example_prelude::init_example;
use gpui::{
    App, Application, Bounds, Colors, Context, FontWeight, Render, Window, WindowBounds,
    WindowOptions, div, prelude::*, px, size,
};

struct OverflowFadeExample;

fn chat_row(index: usize, colors: &Colors) -> impl IntoElement {
    let bg = if index % 2 == 0 {
        colors.surface
    } else {
        colors.surface_hover
    };

    div()
        .h_8()
        .px_3()
        .flex()
        .items_center()
        .rounded_md()
        .bg(bg)
        .text_sm()
        .text_color(colors.text)
        .child(format!(
            "Row {:02}  Overflow fade mask demo item",
            index + 1
        ))
}

fn vertical_panel(title: &'static str, use_fade: bool, colors: &Colors) -> impl IntoElement {
    let scroll_id: u32 = if use_fade { 1 } else { 0 };
    let scroll_area = div()
        .id(("vertical-scroll", scroll_id))
        .h(px(320.))
        .overflow_scroll()
        .rounded_lg()
        .bg(colors.background)
        .p_2()
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .children((0..40).map(|index| chat_row(index, colors))),
        );
    let scroll_area = if use_fade {
        scroll_area.overflow_fade_y(px(22.))
    } else {
        scroll_area
    };

    div()
        .w(px(320.))
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .text_sm()
                .font_weight(FontWeight::BOLD)
                .text_color(colors.text)
                .child(title),
        )
        .child(scroll_area)
        .child(
            div()
                .text_xs()
                .text_color(colors.text_muted)
                .child(if use_fade {
                    "overflow_fade_y(px(22.0)) enabled"
                } else {
                    "No overflow_fade_y"
                }),
        )
}

fn horizontal_chip(index: usize, colors: &Colors) -> impl IntoElement {
    div()
        .h_8()
        .flex_none()
        .px_3()
        .rounded_full()
        .border_1()
        .border_color(colors.border)
        .bg(colors.surface)
        .text_sm()
        .text_color(colors.text)
        .child(format!("Tag {:02}", index + 1))
}

fn horizontal_panel(colors: &Colors) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .text_sm()
                .font_weight(FontWeight::BOLD)
                .text_color(colors.text)
                .child("Horizontal overflow fade"),
        )
        .child(
            div()
                .id("horizontal-scroll")
                .h(px(76.))
                .overflow_scroll()
                .overflow_fade_x(px(30.))
                .rounded_lg()
                .bg(colors.background)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .px_2()
                        .h_full()
                        .children((0..50).map(|index| horizontal_chip(index, colors))),
                ),
        )
        .child(
            div()
                .text_xs()
                .text_color(colors.text_muted)
                .child("overflow_fade_x(px(30.0)) enabled"),
        )
}

impl Render for OverflowFadeExample {
    fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let colors = Colors::for_appearance(window);

        div()
            .id("app")
            .size_full()
            .p_6()
            .gap_6()
            .bg(colors.background)
            .overflow_scroll()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_xl()
                            .font_weight(FontWeight::BOLD)
                            .text_color(colors.text)
                            .child("Overflow Fade"),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(colors.text_muted)
                            .child("Compare no fade vs fade, then test horizontal fade."),
                    ),
            )
            .child(
                div()
                    .flex()
                    .gap_4()
                    .items_start()
                    .child(vertical_panel("Without edge fade", false, &colors))
                    .child(vertical_panel("With edge fade", true, &colors)),
            )
            .child(horizontal_panel(&colors))
    }
}

fn main() {
    Application::new().run(|cx: &mut App| {
        init_example(cx, "Overflow Fade");

        let bounds = Bounds::centered(None, size(px(980.), px(760.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_, cx| cx.new(|_| OverflowFadeExample),
        )
        .unwrap();
    });
}
