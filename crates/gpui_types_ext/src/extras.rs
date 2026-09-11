//! gpui-ce color extensions layered on the upstream `gpui_types::color` API.

use gpui_types::color::{Hsla, Rgba};

use crate::interop::{hsla_from_palette, rgba_from_palette};

/// Converts a color into the canonical [`Hsla`].
///
/// Implemented for both the upstream color types and the `palette` types
/// re-exported by this crate, so callers can pass either without boxing or
/// dynamic dispatch: the conversion is monomorphized and inlined. For a value
/// that is already [`Hsla`] this is the identity.
pub trait IntoHsla {
    /// Convert `self` into [`Hsla`].
    fn into_hsla(self) -> Hsla;
}

impl IntoHsla for Hsla {
    fn into_hsla(self) -> Hsla {
        self
    }
}

impl IntoHsla for Rgba {
    fn into_hsla(self) -> Hsla {
        self.into()
    }
}

impl IntoHsla for palette::Hsla {
    fn into_hsla(self) -> Hsla {
        hsla_from_palette(self)
    }
}

impl IntoHsla for palette::rgb::Rgba {
    fn into_hsla(self) -> Hsla {
        rgba_from_palette(self).into()
    }
}

impl<T: IntoHsla + Copy> IntoHsla for &T {
    fn into_hsla(self) -> Hsla {
        (*self).into_hsla()
    }
}

/// Wrapper methods to make alpha operations more convenient.
pub trait ColorExt {
    /// Performs a SrcAlpha x (1 - SrcAlpha) blend
    fn blend(&self, other: &Self) -> Self
    where
        Self: Sized;

    /// Fade out the color by a given factor. This factor should be between 0.0 and 1.0.
    /// Where 0.0 will leave the color unchanged, and 1.0 will completely fade out the color.
    fn fade_out(&mut self, factor: f32);

    /// Multiplies the alpha value of the color by a given factor and returns a new color.
    fn opacity(&self, factor: f32) -> Self
    where
        Self: Sized;

    /// Returns this color with its alpha channel replaced by `a`.
    ///
    /// Compatibility shim for `palette::WithAlpha::with_alpha`; equivalent to the
    /// upstream inherent `alpha` method.
    fn with_alpha(&self, a: f32) -> Self
    where
        Self: Sized;
}

impl ColorExt for Rgba {
    fn blend(&self, other: &Self) -> Self {
        Rgba::blend(self, *other)
    }

    fn fade_out(&mut self, factor: f32) {
        self.a *= 1.0 - factor.clamp(0., 1.);
    }

    fn opacity(&self, factor: f32) -> Self {
        Rgba::opacity(self, factor)
    }

    fn with_alpha(&self, a: f32) -> Self {
        Rgba::alpha(self, a)
    }
}

impl ColorExt for Hsla {
    fn blend(&self, other: &Self) -> Self {
        Hsla::blend(*self, *other)
    }

    fn fade_out(&mut self, factor: f32) {
        Hsla::fade_out(self, factor);
    }

    fn opacity(&self, factor: f32) -> Self {
        Hsla::opacity(self, factor)
    }

    fn with_alpha(&self, a: f32) -> Self {
        Hsla::alpha(self, a)
    }
}

/// Convert an [`Rgba`] color to [`Hsla`].
pub fn rgb_to_hsla(color: Rgba) -> Hsla {
    color.into()
}

/// Convert an [`Hsla`] color to [`Rgba`].
pub fn hsla_to_rgba(color: Hsla) -> Rgba {
    color.into()
}
