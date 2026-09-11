//! Conversions between `palette` colors and the upstream color types in
//! [`gpui_types::color`].
//!
//! Direct `From` impls for this foreign/foreign pair are not permitted by Rust's
//! orphan rules, so they are exposed as functions.

use gpui_types::color::Hsla;

/// Converts an upstream [`Hsla`] into a [`palette::Hsla`].
pub fn hsla_to_palette(color: Hsla) -> palette::Hsla {
    palette::Hsla::new(color.h * 360.0, color.s, color.l, color.a)
}

/// Converts a [`palette::Hsla`] into an upstream [`Hsla`].
pub fn hsla_from_palette(color: palette::Hsla) -> Hsla {
    Hsla {
        h: color.hue.into_positive_degrees() / 360.0,
        s: color.saturation,
        l: color.lightness,
        a: color.alpha,
    }
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
