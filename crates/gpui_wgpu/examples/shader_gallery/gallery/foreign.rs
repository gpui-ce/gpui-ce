//! WGSL files and Rust functions imported into paints.

use gpui::{
    rgb,
    shader::{Function, Paint, Shader, Vec2f, color, paint},
};
use std::sync::LazyLock;

/// Import the file's `paint` function; parameters are checked at compilation.
static PLASMA: LazyLock<Shader> =
    LazyLock::new(|| Shader::wgsl(include_str!("../plasma.wgsl")).expect("plasma.wgsl"));

/// One Rust implementation supplies GPU code and CPU evaluation.
#[wgsl_rs::wgsl]
#[allow(missing_docs)]
pub mod ripples {
    use wgsl_rs::std::*;

    pub fn ripple(point: Vec2f, time: f32) -> f32 {
        let distance = length(point);
        let wave = sin(distance * 0.2 - time * 4.0) * 0.5 + 0.5;
        wave * exp(-distance * 0.015)
    }
}

static RIPPLE: LazyLock<Function<fn(Vec2f, f32) -> f32>> =
    LazyLock::new(|| gpui::wgsl_fn!(ripples::ripple).expect("ripple"));

pub fn plasma(time: f32) -> Paint {
    // Raw WGSL needs an explicit fallback for renderers without shader support.
    PLASMA.with(time).fallback(rgb(0x7b5cff))
}

pub fn ripple(time: f32) -> Paint {
    paint(|px| {
        let wave = RIPPLE.call((px.centered(), time));
        color(rgb(0x0b1d3a)).mix(color(rgb(0x5ad1ff)), wave)
    })
}
