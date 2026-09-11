//! Palette-based color API for gpui-ce, layered on top of `gpui_types`.
//!
//! `gpui_types` hosts the upstream color types (available as
//! [`gpui_types::color`]). This crate keeps gpui-ce's `palette`-based color API
//! and the renderer-facing [`SceneHsla`], and provides conversions between the
//! two families.
//!
//! `gpui` re-exports this crate, so `gpui::Hsla` continues to name
//! `palette::Hsla`, `gpui::SceneHsla` is available as before, and the palette
//! types are re-exported under [`palette`].

mod color;
mod interop;
mod scene_color;

pub use color::*;
pub use interop::*;
pub use palette;
pub use scene_color::*;
