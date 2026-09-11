//! Palette-based color extension for gpui-ce.
//!
//! The upstream [`gpui_types::color`] API is the canonical color API re-exported
//! by `gpui`. This crate keeps gpui-ce's `palette` dependency as an extension:
//! the [`palette`] crate itself is re-exported, and conversions between `palette`
//! and the upstream color types, along with gpui-ce's own extras ([`ColorExt`],
//! [`IntoHsla`], [`rgb_to_hsla`]) and the pre-migration back-compat shims live
//! here.

mod compat;
mod extras;
mod interop;

pub use compat::*;
pub use extras::*;
pub use interop::*;
pub use palette;
