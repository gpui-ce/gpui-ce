use crate::{Bounds, Pixels};

/// The measurements of a typeface, in font units.
///
/// Get the values for a font with [`crate::TextSystem::resolve_font`] and the
/// `TextSystem` metric methods. The methods on this type convert the values to
/// pixels at a given font size. Use the font-unit fields directly only when
/// you need exact values.
#[derive(Clone, Copy, Debug)]
pub struct FontMetrics {
    /// The number of font units that make up the em square, the grid that font sizes are measured on.
    pub units_per_em: u32,

    /// The distance from the baseline up to the top of the tallest glyphs, in font units.
    pub ascent: f32,

    /// The distance from the baseline down to the bottom of the lowest glyphs, in font units.
    pub descent: f32,

    /// The extra space the font recommends between lines, in font units.
    pub line_gap: f32,

    /// The suggested vertical position of the underline, in font units.
    pub underline_position: f32,

    /// The suggested thickness of the underline, in font units.
    pub underline_thickness: f32,

    /// The height of a capital letter above the baseline, in font units.
    pub cap_height: f32,

    /// The height of a lowercase `x` above the baseline, in font units.
    pub x_height: f32,

    /// The outer limits of the area that the font covers, in font units.
    /// Corresponds to the xMin / xMax / yMin / yMax values in the OpenType `head` table.
    pub bounding_box: Bounds<f32>,
}

impl FontMetrics {
    /// Returns the distance from the baseline up to the tallest glyphs, in pixels at `font_size`.
    pub fn ascent(&self, font_size: Pixels) -> Pixels {
        Pixels((self.ascent / self.units_per_em as f32) * font_size.0)
    }

    /// Returns the distance from the baseline down to the lowest glyphs, in pixels at `font_size`.
    pub fn descent(&self, font_size: Pixels) -> Pixels {
        Pixels((self.descent / self.units_per_em as f32) * font_size.0)
    }

    /// Returns the recommended extra space between lines, in pixels at `font_size`.
    pub fn line_gap(&self, font_size: Pixels) -> Pixels {
        Pixels((self.line_gap / self.units_per_em as f32) * font_size.0)
    }

    /// Returns the suggested position of the underline, in pixels at `font_size`.
    pub fn underline_position(&self, font_size: Pixels) -> Pixels {
        Pixels((self.underline_position / self.units_per_em as f32) * font_size.0)
    }

    /// Returns the suggested thickness of the underline, in pixels at `font_size`.
    pub fn underline_thickness(&self, font_size: Pixels) -> Pixels {
        Pixels((self.underline_thickness / self.units_per_em as f32) * font_size.0)
    }

    /// Returns the height of a capital letter above the baseline, in pixels at `font_size`.
    pub fn cap_height(&self, font_size: Pixels) -> Pixels {
        Pixels((self.cap_height / self.units_per_em as f32) * font_size.0)
    }

    /// Returns the height of a lowercase `x` above the baseline, in pixels at `font_size`.
    pub fn x_height(&self, font_size: Pixels) -> Pixels {
        Pixels((self.x_height / self.units_per_em as f32) * font_size.0)
    }

    /// Returns the outer limits of the area that the font covers, in pixels at `font_size`.
    pub fn bounding_box(&self, font_size: Pixels) -> Bounds<Pixels> {
        (self.bounding_box / self.units_per_em as f32 * font_size.0).map(crate::px)
    }
}
