//! Typed fragment expressions, CPU evaluation, and cached WGSL compilation.
//!
//! [`Paint`] composes expressions into one fragment program. Plain values
//! become uniforms; changing them reuses the program. [`Shader`] and
//! [`Function`] import WGSL, GLSL, or `#[wgsl]` functions. [`wgsl_fn!`](crate::wgsl_fn)
//! also supplies the Rust implementation for CPU evaluation and folding.

mod builtins;
mod compile;
mod expr;
mod function;
mod library;
mod paint;
mod pipeline;
mod value;

pub use compile::{CompiledPaint, Program, ShaderError};
pub use expr::{
    Bool, Expr, Scalar, Vec2, Vec3, Vec4, constant, iterate, iterate_until, vec2, vec3, vec4,
};
pub use function::{Arguments, CpuFunction, Function, Library, Shader, ShaderArguments, Signature};
pub use library::{noise, prelude, shape};
pub use paint::{Paint, Pixel, backdrop, color, paint, rgba};
pub use pipeline::{Instance, Pipeline, Topology};
pub use prelude::Fragment;
pub use value::{Arith, CpuValue, Float, Operand, Value, Vector, Widen};
pub use wgsl_rs::std::{Vec2f, Vec3f, Vec4f, vec2f, vec3f, vec4f};

#[cfg(test)]
mod tests;
