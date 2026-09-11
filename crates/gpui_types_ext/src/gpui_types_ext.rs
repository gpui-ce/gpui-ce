//! Palette-based color extension for gpui-ce.
//!
//! The upstream [`gpui_types::color`] API is the canonical color API re-exported
//! by `gpui`. This crate keeps gpui-ce's `palette` dependency as an extension:
//! the [`palette`] crate itself is re-exported, conversions between `palette`
//! and the upstream color types are provided, and gpui-ce's own extras
//! ([`ColorExt`], [`rgb_to_hsla`], [`SceneHsla`]) live here.

mod extras;
mod interop;
mod scene_color;

pub use extras::*;
pub use interop::*;
pub use palette;
pub use scene_color::*;
