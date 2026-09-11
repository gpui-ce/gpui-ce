//! Conversions between `palette` colors, [`crate::SceneHsla`], and the upstream
//! color types in [`gpui_types::color`].
//!
//! Direct `From` impls for the foreign/foreign pairs (`palette` <-> upstream)
//! are not permitted by Rust's orphan rules, so they are exposed as functions.

use crate::SceneHsla;

/// Converts an upstream [`gpui_types::color::Hsla`] into a [`palette::Hsla`].
pub fn hsla_to_palette(color: gpui_types::color::Hsla) -> palette::Hsla {
    SceneHsla::from(color).into()
}

/// Converts a [`palette::Hsla`] into an upstream [`gpui_types::color::Hsla`].
pub fn hsla_from_palette(color: palette::Hsla) -> gpui_types::color::Hsla {
    SceneHsla::from(color).into()
}

/// Converts an upstream [`gpui_types::color::Rgba`] into a [`palette::rgb::Rgba`].
pub fn rgba_to_palette(color: gpui_types::color::Rgba) -> palette::rgb::Rgba {
    palette::rgb::Rgba::new(color.r, color.g, color.b, color.a)
}

/// Converts a [`palette::rgb::Rgba`] into an upstream [`gpui_types::color::Rgba`].
pub fn rgba_from_palette(color: palette::rgb::Rgba) -> gpui_types::color::Rgba {
    gpui_types::color::Rgba {
        r: color.color.red,
        g: color.color.green,
        b: color.color.blue,
        a: color.alpha,
    }
}
