//! Gallery layout and animation scheduling.

mod foreign;
mod interactive;
mod paints;
mod sparks;

use foreign::{plasma, ripple};
use gpui::{
    Context, IntoElement, ParentElement, Render, Styled, Window, div, px, rgb,
    shader::{self, Input, Paint},
    white,
};
use interactive::{spotlight, torus};
use paints::*;
use std::time::Instant;

const CARD: f32 = 200.0;
const INK: u32 = 0x0e1120;

pub struct Gallery {
    start: Instant,
    frozen: Option<f32>,
}

impl Gallery {
    pub fn new() -> Self {
        Self {
            start: Instant::now(),
            frozen: None,
        }
    }

    /// Freeze time for reproducible headless frames.
    #[cfg(feature = "test-support")]
    pub fn frozen(time: f32) -> Self {
        Self {
            start: Instant::now(),
            frozen: Some(time),
        }
    }
}

fn card(title: &'static str, surface: impl IntoElement) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(surface)
        .child(div().text_sm().text_color(rgb(0xaab3d0)).child(title))
}

fn surface() -> gpui::Div {
    div().size(px(CARD)).rounded(px(20.0)).overflow_hidden()
}

/// A card whose paint follows the pointer and time.
fn layer(
    id: &'static str,
    frozen: Option<f32>,
    paint: impl Fn(&Input) -> Paint + 'static,
) -> shader::Layer {
    let layer = shader::layer(id, move |input| {
        let input = Input {
            time: frozen.unwrap_or(input.time),
            ..*input
        };
        paint(&input)
    })
    .size(px(CARD))
    .rounded(px(20.0));
    if frozen.is_none() {
        layer.animate()
    } else {
        layer
    }
}

impl Render for Gallery {
    fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let time = self
            .frozen
            .unwrap_or_else(|| self.start.elapsed().as_secs_f32());
        if self.frozen.is_none() {
            window.request_animation_frame();
        }
        let status = if window.supports_shader_paint() {
            "Running on the GPU."
        } else {
            "This renderer cannot run shaders: paints show their fallbacks."
        };

        div()
            .size_full()
            .bg(rgb(INK))
            .p_6()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .child(div().text_xl().text_color(white()).child("Shader paints"))
                    .child(div().text_sm().text_color(rgb(0x6f7898)).child(status)),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_4()
                    .child(card("Expressions", surface().bg(waves(time))))
                    .child(card("Signed distances", surface().bg(orbits(time))))
                    .child(card("Noise", surface().bg(aurora(time))))
                    .child(card("Loops", surface().bg(clouds(time))))
                    .child(card("Remapped", surface().bg(swirl(time))))
                    .child(card("A .wgsl file", surface().bg(plasma(time))))
                    .child(card("A #[wgsl] module", surface().bg(ripple(time))))
                    .child(card("Raymarched 3D", layer("torus", self.frozen, torus)))
                    .child(card(
                        "Borders",
                        surface()
                            .border(px(6.0))
                            .border_color(rainbow(time))
                            .bg(rgb(0x1a1f3a)),
                    ))
                    .child(card(
                        "Backdrop",
                        surface().relative().bg(stripes(time)).child(
                            div()
                                .absolute()
                                .top(px(40.0))
                                .left(px(40.0))
                                .size(px(120.0))
                                .rounded(px(60.0))
                                .bg(lens()),
                        ),
                    ))
                    .child(card("Pointer", layer("spotlight", self.frozen, spotlight)))
                    .child(card(
                        "A custom pipeline",
                        surface().bg(rgb(0x060914)).child(sparks::sparks(time)),
                    )),
            )
    }
}
