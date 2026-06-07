//! Backdrop Blur Showcase
//!
//! Demonstrates backdrop blur (frosted glass effect) using `backdrop_blur(...)` alongside `opacity(...)`.
//! Run with:
//!   cargo run --example blur_showcase

#[path = "../prelude.rs"]
mod example_prelude;

use example_prelude::init_example;
use gpui::{
    App, Application, Bounds, Context, Render, Window, WindowBounds, WindowOptions, div,
    prelude::*, px, rgb, rgba, size,
};

struct BlurShowcase;

fn color_strip() -> impl IntoElement {
    div()
        .flex()
        .h_20()
        .rounded_lg()
        .overflow_hidden()
        .child(div().flex_1().bg(rgb(0xff5d73)))
        .child(div().flex_1().bg(rgb(0xffc857)))
        .child(div().flex_1().bg(rgb(0x56cfe1)))
        .child(div().flex_1().bg(rgb(0x80ed99)))
        .child(div().flex_1().bg(rgb(0x6d77ff)))
}

fn blur_card(
    id: &'static str,
    title: &'static str,
    subtitle: &'static str,
    blur_radius: f32,
    opacity: f32,
) -> impl IntoElement {
    div()
        .id(id)
        .flex()
        .flex_col()
        .gap_2()
        .p_4()
        .rounded_lg()
        .bg(rgb(0x2a2a2a))
        .opacity(opacity)
        .backdrop_blur(blur_radius)
        .border_1()
        .border_color(rgb(0x444444))
        .child(
            div()
                .text_sm()
                .font_weight(gpui::FontWeight::BOLD)
                .child(title),
        )
        .child(div().text_xs().text_color(rgb(0x999999)).child(subtitle))
}

/// A frosted-glass card with no background tint at all — fully transparent, so only the
/// blurred backdrop (and its rounded-rect mask/border) is visible. This isolates the
/// `backdrop-filter` effect from any `bg(...)` compositing.
fn glass_card(
    id: &'static str,
    title: &'static str,
    subtitle: &'static str,
    blur_radius: f32,
) -> impl IntoElement {
    div()
        .id(id)
        .flex()
        .flex_col()
        .gap_2()
        .p_4()
        .rounded_lg()
        .bg(rgba(0x00000000))
        .backdrop_blur(blur_radius)
        .border_1()
        .border_color(rgb(0xffffff))
        .child(
            div()
                .text_sm()
                .font_weight(gpui::FontWeight::BOLD)
                .child(title),
        )
        .child(div().text_xs().text_color(rgb(0xdddddd)).child(subtitle))
}

fn section_heading(text: &'static str) -> impl IntoElement {
    div()
        .pt_2()
        .text_base()
        .font_weight(gpui::FontWeight::BOLD)
        .child(text)
}

fn radial_background_1() -> impl IntoElement {
    div().h_20().rounded_lg().bg(gpui::radial_gradient(
        0.5,
        0.5,
        0.8,
        0.8,
        gpui::gradient_color_stop(rgb(0xff5d73), 0.0),
        gpui::gradient_color_stop(rgb(0x6d77ff), 1.0),
    ))
}

fn radial_background_2() -> impl IntoElement {
    div().h_20().rounded_lg().bg(gpui::radial_gradient(
        0.7,
        0.32,
        0.9,
        0.6,
        gpui::gradient_color_stop(rgb(0x80ed99), 0.0),
        gpui::gradient_color_stop(rgb(0x000000), 1.0),
    ))
}

fn demo_row(
    id: &'static str,
    heading: &'static str,
    blur_radius: f32,
    opacity: f32,
) -> impl IntoElement {
    div()
        .id(id)
        .flex()
        .flex_col()
        .gap_2()
        .child(div().text_xs().text_color(rgb(0x999999)).child(heading))
        .child(
            div().relative().child(color_strip()).child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .right_0()
                    .bottom_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .p_4()
                    .child(blur_card(
                        "blur-card",
                        "Frosted Glass",
                        "Backdrop blur + opacity",
                        blur_radius,
                        opacity,
                    ))
                    .w_full(),
            ),
        )
}

/// A row showing a fully transparent `glass_card` (no `bg`/tint) over the color strip, so the
/// blur is the only thing distinguishing the card from its background.
fn glass_demo_row(id: &'static str, heading: &'static str, blur_radius: f32) -> impl IntoElement {
    div()
        .id(id)
        .flex()
        .flex_col()
        .gap_2()
        .child(div().text_xs().text_color(rgb(0x999999)).child(heading))
        .child(
            div().relative().child(color_strip()).child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .right_0()
                    .bottom_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .p_4()
                    .child(glass_card(
                        "glass-card",
                        "No Background",
                        "backdrop_blur with transparent bg",
                        blur_radius,
                    ))
                    .w_full(),
            ),
        )
}

/// A row contrasting a sharp color strip with a copy of itself blurred as a single group via
/// `.blur(radius)` — CSS `filter: blur(...)`. Unlike `backdrop_blur`, this filters the
/// element's *own* rendered content (and its children), not whatever is behind it.
fn content_blur_demo_row(
    id: &'static str,
    heading: &'static str,
    blur_radius: f32,
) -> impl IntoElement {
    div()
        .id(id)
        .flex()
        .flex_col()
        .gap_2()
        .child(div().text_xs().text_color(rgb(0x999999)).child(heading))
        .child(
            div()
                .flex()
                .gap_4()
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(div().text_xs().text_color(rgb(0x666666)).child("Sharp"))
                        .child(color_strip()),
                )
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(0x666666))
                                .child(format!("blur({blur_radius})")),
                        )
                        .child(div().blur(px(blur_radius)).child(color_strip())),
                ),
        )
}

/// Nested content-filter groups: an outer card uses `.blur(...)` to filter itself *and* its
/// children — including an inner card that *also* applies its own `.blur(...)`. The renderer
/// must isolate each group into its own offscreen target and composite them in order.
fn nested_blur_demo() -> impl IntoElement {
    div()
        .id("nested-blur")
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .text_xs()
                .text_color(rgb(0x999999))
                .child("Nested filter groups: outer blur(4) wraps an inner blur(10)"),
        )
        .child(
            div()
                .flex()
                .gap_4()
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(div().text_xs().text_color(rgb(0x666666)).child("Unblurred"))
                        .child(
                            div()
                                .h(px(112.))
                                .rounded_lg()
                                .overflow_hidden()
                                .bg(rgb(0x333355))
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(color_strip().into_any_element())
                                .child(
                                    div()
                                        .absolute()
                                        .size_full()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .child(
                                            div()
                                                .p_3()
                                                .rounded_md()
                                                .bg(rgb(0xff5d73))
                                                .text_sm()
                                                .text_color(rgb(0xffffff))
                                                .child("inner"),
                                        ),
                                ),
                        ),
                )
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(0x666666))
                                .child("blur(4) ∘ blur(10)"),
                        )
                        .child(
                            div()
                                .blur(px(4.0))
                                .h(px(112.))
                                .rounded_lg()
                                .overflow_hidden()
                                .bg(rgb(0x333355))
                                .relative()
                                .child(color_strip())
                                .child(
                                    div()
                                        .absolute()
                                        .top_0()
                                        .left_0()
                                        .right_0()
                                        .bottom_0()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .child(
                                            div()
                                                .blur(px(10.0))
                                                .p_3()
                                                .rounded_md()
                                                .bg(rgb(0xff5d73))
                                                .text_sm()
                                                .text_color(rgb(0xffffff))
                                                .child("inner"),
                                        ),
                                ),
                        ),
                ),
        )
}

fn radial_demo_row(
    id: &'static str,
    heading: &'static str,
    blur_radius: f32,
    opacity: f32,
    background: impl IntoElement,
) -> impl IntoElement {
    div()
        .id(id)
        .flex()
        .flex_col()
        .gap_2()
        .child(div().text_xs().text_color(rgb(0x999999)).child(heading))
        .child(
            div().relative().child(background).child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .right_0()
                    .bottom_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .p_4()
                    .child(blur_card(
                        "blur-card",
                        "Frosted Glass",
                        "Backdrop blur + radial gradient",
                        blur_radius,
                        opacity,
                    ))
                    .w_full(),
            ),
        )
}

impl Render for BlurShowcase {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .p_6()
            .flex()
            .flex_col()
            .gap_5()
            .bg(rgb(0x1e1e1e))
            .child(div().text_lg().font_weight(gpui::FontWeight::BOLD).child("Backdrop Blur Showcase"))
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(0x999999))
                    .child("Compare rows: backdrop blur radius increases while opacity stays semi-transparent."),
            )
            .child(section_heading("backdrop_blur — tinted background"))
            .child(demo_row(
                "row-none",
                "backdrop_blur(0.0), opacity(0.82)",
                0.0,
                0.82,
            ))
            .child(demo_row(
                "row-soft",
                "backdrop_blur(6.0), opacity(0.82)",
                6.0,
                0.82,
            ))
            .child(demo_row(
                "row-medium",
                "backdrop_blur(12.0), opacity(0.82)",
                12.0,
                0.82,
            ))
            .child(demo_row(
                "row-strong",
                "backdrop_blur(18.0), opacity(0.82)",
                18.0,
                0.82,
            ))
            .child(section_heading("backdrop_blur — radial gradient backgrounds"))
            .child(radial_demo_row(
                "radial-1",
                "Radial Gradient #1",
                6.0,
                0.82,
                radial_background_1(),
            ))
            .child(radial_demo_row(
                "radial-2",
                "Radial Gradient #2",
                12.0,
                0.82,
                radial_background_2(),
            ))
            .child(section_heading(
                "backdrop_blur — fully transparent background (no tint)",
            ))
            .child(glass_demo_row(
                "glass-soft",
                "bg(transparent), backdrop_blur(8.0)",
                8.0,
            ))
            .child(glass_demo_row(
                "glass-strong",
                "bg(transparent), backdrop_blur(20.0)",
                20.0,
            ))
            .child(section_heading("filter: blur — blurring an element's own content"))
            .child(content_blur_demo_row("content-soft", "blur(4.0)", 4.0))
            .child(content_blur_demo_row("content-strong", "blur(12.0)", 12.0))
            .child(section_heading("filter: blur — nested filter groups"))
            .child(nested_blur_demo())
    }
}

fn main() {
    Application::new().run(|cx: &mut App| {
        init_example(cx, "Blur Showcase");

        let bounds = Bounds::centered(None, size(px(900.), px(760.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_, cx| cx.new(|_| BlurShowcase),
        )
        .expect("open window");
    });
}
