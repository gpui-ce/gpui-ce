//! Palette-based color extension for gpui-ce.
//!
//! The upstream [`gpui_types::color`] API is the canonical color API re-exported
//! by `gpui`. This crate keeps gpui-ce's `palette` dependency as an extension:
//! the [`palette`] crate itself is re-exported, and conversions between `palette`
//! and the upstream color types, along with gpui-ce's own extras ([`ColorExt`],
//! [`IntoHsla`], [`rgb_to_hsla`]), live here.

mod extras;
mod interop;

pub use extras::*;
pub use interop::*;
pub use palette;
