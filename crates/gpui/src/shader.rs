//! Fragment paints and custom geometry drawn in GPUI's scene order.
//!
//! [`Paint`] builds a fragment color from typed Rust [expressions](Expr).
//! Use it for element backgrounds, borders, or [`Window::paint_quad`](crate::Window::paint_quad):
//!
//! ```ignore
//! div().bg(paint(|px| {
//!     let ring = shape::circle(px.centered(), 40.0).abs() - 3.0;
//!     color(rgb(0xffd166)).clip(ring).over(color(rgb(0x0b0e1c)))
//! }))
//! ```
//!
//! [Shapes](shape), [noise], [`iterate`], and [`shape::raymarch`] compose into
//! one fragment program. Plain values become uniforms; changing them reuses
//! the program. Uniform expressions fold on the CPU using `wgsl_rs::std`.
//!
//! [`Shader`] imports a WGSL, GLSL, or `#[wgsl]` paint function.
//! [`Function`] imports typed helpers that compose with expressions;
//! [`wgsl_fn!`](crate::wgsl_fn) also supplies their Rust implementation for CPU evaluation.
//!
//! [`layer`] rebuilds a paint from pointer, hover, press, and time [`Input`].
//! [`Pipeline`] supplies custom vertex and fragment functions, drawn with
//! [`Window::paint_pipeline`](crate::Window::paint_pipeline).
//!
//! GPUI applies the element's geometry, clipping, and opacity. Renderers
//! without shader support use each paint's [`fallback`](Paint::fallback).

mod builtins;
mod compile;
mod expr;
mod function;
mod layer;
mod library;
mod paint;
mod pipeline;
mod value;

pub use compile::{CompiledPaint, Program, ShaderError};
pub use expr::{
    Bool, Expr, Scalar, Vec2, Vec3, Vec4, constant, iterate, iterate_until, vec2, vec3, vec4,
};
pub use function::{Arguments, CpuFunction, Function, Library, Shader, ShaderArguments, Signature};
pub use layer::{Input, Layer, layer};
pub use library::{noise, prelude, shape};
pub use paint::{Paint, Pixel, backdrop, color, paint, rgba};
pub use pipeline::{Instance, Pipeline, Topology};
pub use prelude::Fragment;
pub use value::{Arith, CpuValue, Float, Operand, Value, Vector, Widen};
pub use wgsl_rs::std::{Vec2f, Vec3f, Vec4f, vec2f, vec3f, vec4f};

#[cfg(test)]
mod tests;
