//! Animation Example
//!
//! This example demonstrates animation capabilities in GPUI:
//!
//! 1. Basic animations with `with_animation`
//! 2. Easing functions - ease_in_out, bounce, linear
//! 3. Transformations - rotate, scale, translate
//! 4. Repeating and duration controls

#[path = "../prelude.rs"]
mod example_prelude;

use std::time::Duration;

use anyhow::Result;
use gpui::{
    Animation, AnimationExt as _, App, Application, AssetSource, Bounds, Colors, Context, Hsla,
    SharedString, Transformation, Window, WindowBounds, WindowOptions, bounce, div, ease_in_out,
    linear, percentage, prelude::*, px, size as gpui_size, svg,
};

struct Assets {}

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<std::borrow::Cow<'static, [u8]>>> {
        std::fs::read(path)
            .map(Into::into)
            .map_err(Into::into)
            .map(Some)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        Ok(std::fs::read_dir(path)?
            .filter_map(|entry| {
                Some(SharedString::from(
                    entry.ok()?.path().to_string_lossy().into_owned(),
                ))
            })
            .collect::<Vec<_>>())
    }
}

const ARROW_CIRCLE_SVG: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/examples/legacy/image/arrow_circle.svg"
);

struct AnimationExample;

impl Render for AnimationExample {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let colors = Colors::for_appearance(window);

        div()
            .id("main")
            .size_full()
            .p_6()
            .bg(colors.background)
            .overflow_scroll()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_6()
                    .max_w(px(600.))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_xl()
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .text_color(colors.text)
                                    .child("Animation Patterns"),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(colors.text_muted)
                                    .child("Animations, easing, and transformations in GPUI"),
                            ),
                    )
                    .child(section(
                        &colors,
                        "Rotation Animation",
                        rotation_example(&colors),
                    ))
                    .child(section(&colors, "Bounce Easing", bounce_example(&colors)))
                    .child(section(&colors, "Scale Animation", scale_example(&colors)))
                    .child(section(
                        &colors,
                        "Combined Animations",
                        combined_example(&colors),
                    )),
            )
    }
}

fn rotation_example(colors: &Colors) -> impl IntoElement {
    let text_muted = colors.text_muted;
    let accent = colors.accent;

    div()
        .flex()
        .flex_col()
        .gap_3()
        .child(
            div()
                .text_xs()
                .text_color(text_muted)
                .child("Continuous rotation with ease_in_out easing"),
        )
        .child(
            div().flex().items_center().justify_center().h_24().child(
                svg()
                    .size_16()
                    .overflow_hidden()
                    .path(ARROW_CIRCLE_SVG)
                    .text_color(accent)
                    .with_animation(
                        "rotation",
                        Animation::new(Duration::from_secs(2))
                            .repeat()
                            .with_easing(ease_in_out),
                        |svg, delta| {
                            svg.with_transformation(Transformation::rotate(percentage(delta)))
                        },
                    ),
            ),
        )
}

fn bounce_example(colors: &Colors) -> impl IntoElement {
    let text_muted = colors.text_muted;
    let success = colors.success;

    div()
        .flex()
        .flex_col()
        .gap_3()
        .child(
            div()
                .text_xs()
                .text_color(text_muted)
                .child("Bouncing rotation with bounce(ease_in_out)"),
        )
        .child(
            div().flex().items_center().justify_center().h_24().child(
                svg()
                    .size_16()
                    .overflow_hidden()
                    .path(ARROW_CIRCLE_SVG)
                    .text_color(success)
                    .with_animation(
                        "bounce_rotation",
                        Animation::new(Duration::from_secs(2))
                            .repeat()
                            .with_easing(bounce(ease_in_out)),
                        |svg, delta| {
                            svg.with_transformation(Transformation::rotate(percentage(delta)))
                        },
                    ),
            ),
        )
}

fn scale_example(colors: &Colors) -> impl IntoElement {
    let text_muted = colors.text_muted;
    let warning = colors.warning;

    div()
        .flex()
        .flex_col()
        .gap_3()
        .child(
            div()
                .text_xs()
                .text_color(text_muted)
                .child("Scale pulsing with linear easing"),
        )
        .child(
            div().flex().items_center().justify_center().h_24().child(
                svg()
                    .size_16()
                    .overflow_hidden()
                    .path(ARROW_CIRCLE_SVG)
                    .text_color(warning)
                    .with_animation(
                        "scale",
                        Animation::new(Duration::from_millis(1500))
                            .repeat()
                            .with_easing(bounce(linear)),
                        |svg, delta| {
                            let scale = 0.8 + (delta * 0.4);
                            svg.with_transformation(Transformation::scale(gpui_size(scale, scale)))
                        },
                    ),
            ),
        )
}

fn combined_example(colors: &Colors) -> impl IntoElement {
    let text_muted = colors.text_muted;
    let error = colors.error;

    div()
        .flex()
        .flex_col()
        .gap_3()
        .child(
            div()
                .text_xs()
                .text_color(text_muted)
                .child("Rotation + scale combined"),
        )
        .child(
            div().flex().items_center().justify_center().h_24().child(
                svg()
                    .size_16()
                    .overflow_hidden()
                    .path(ARROW_CIRCLE_SVG)
                    .text_color(error)
                    .with_animation(
                        "combined",
                        Animation::new(Duration::from_secs(3))
                            .repeat()
                            .with_easing(ease_in_out),
                        |svg, delta| {
                            let scale = 0.7 + (delta * 0.6);
                            svg.with_transformation(
                                Transformation::rotate(percentage(delta))
                                    .with_scaling(gpui_size(scale, scale)),
                            )
                        },
                    ),
            ),
        )
}

fn section(colors: &Colors, title: &'static str, content: impl IntoElement) -> impl IntoElement {
    let surface: Hsla = colors.surface.into();

    div()
        .flex()
        .flex_col()
        .gap_2()
        .p_4()
        .bg(surface.opacity(0.5))
        .rounded_lg()
        .child(
            div()
                .text_sm()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(colors.text)
                .child(title),
        )
        .child(content)
}

fn main() {
    Application::new()
        .with_assets(Assets {})
        .run(|cx: &mut App| {
            let bounds = Bounds::centered(None, gpui_size(px(500.), px(650.)), cx);
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    ..Default::default()
                },
                |_, cx| cx.new(|_| AnimationExample),
            )
            .expect("Failed to open window");

            example_prelude::init_example(cx, "Animation");
        });
}
