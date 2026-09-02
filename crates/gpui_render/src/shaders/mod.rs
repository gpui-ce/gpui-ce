//! Rust-authored shader sources, split by ABI and rendering domain.

#![allow(dead_code, unused_assignments, unused_imports)]

pub mod common;
pub mod interface;

mod emoji;
mod filters;
mod linkage;
mod paths;
mod quads;
mod shadows;
mod sprites;

pub use emoji::emoji_rasterization;
pub use filters::{blur, surface};
pub use linkage::base;
pub use paths::{path, path_rasterization};
pub use quads::quad;
pub use shadows::shadow;
pub use sprites::{monochrome_sprite, polychrome_sprite, subpixel_sprite, underline};

mod host;

#[cfg(test)]
mod tests;
