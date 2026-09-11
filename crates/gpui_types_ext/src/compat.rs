//! Backwards-compatibility shims for the pre-migration gpui-ce color API.
//!
//! Before the upstream color types became canonical, these items lived directly
//! in the `gpui` crate's `color` module. They are kept here, re-exported from
//! `gpui`, so that downstream code written against the older API keeps
//! compiling.

use gpui_types::color::{Background, BackgroundTag, Hsla, LinearColorStop, Rgba};

/// Renderer-facing color alias kept for backwards compatibility.
///
/// The scene now uses the upstream [`Hsla`] directly; this is the same type.
pub type SceneHsla = Hsla;

/// What a [`Background`] paints, decoded from its packed representation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BackgroundKind {
    /// A flat color.
    Solid(Hsla),
    /// A linear gradient between two color stops.
    LinearGradient {
        /// The gradient line's angle in degrees, `0.0` pointing up, increasing clockwise.
        angle: f32,
        /// The two ends of the gradient.
        stops: [LinearColorStop; 2],
    },
    /// A diagonal stripe pattern.
    PatternSlash {
        /// The stripe color.
        color: Hsla,
        /// The stripe width, in logical pixels.
        width: f32,
        /// The gap between stripes, in logical pixels.
        interval: f32,
    },
    /// Alternating squares of one color and full transparency.
    Checkerboard {
        /// The color of one set of squares. The other set is fully transparent.
        color: Hsla,
        /// The width and height of each square, in logical pixels.
        size: f32,
    },
}

/// Extension methods on [`Background`] that previously lived on the type itself.
pub trait BackgroundExt {
    /// Returns the decoded form of this background.
    fn kind(&self) -> BackgroundKind;
}

impl BackgroundExt for Background {
    fn kind(&self) -> BackgroundKind {
        match self.tag {
            BackgroundTag::Solid => BackgroundKind::Solid(self.solid),
            BackgroundTag::LinearGradient => BackgroundKind::LinearGradient {
                angle: self.gradient_angle_or_pattern_height,
                stops: self.colors,
            },
            BackgroundTag::PatternSlash => {
                const PATTERN_COMPONENT_SCALE: f32 = u8::MAX as f32;
                const PATTERN_PACKING_RADIX: f32 = u16::MAX as f32;

                // `pattern_slash` packs both values into one exactly
                // representable f32; floor + rem_euclid inverts it, since
                // truncation and `%` give the wrong entry.
                let packed = self.gradient_angle_or_pattern_height;
                BackgroundKind::PatternSlash {
                    color: self.solid,
                    width: (packed / PATTERN_PACKING_RADIX).floor() / PATTERN_COMPONENT_SCALE,
                    interval: (packed.rem_euclid(PATTERN_PACKING_RADIX)) / PATTERN_COMPONENT_SCALE,
                }
            }
            BackgroundTag::Checkerboard => BackgroundKind::Checkerboard {
                color: self.solid,
                size: self.gradient_angle_or_pattern_height,
            },
        }
    }
}

/// Constructs an [`Oklcha`](palette::Oklcha) object from plain values.
pub fn oklcha<T>(
    lightness: T,
    chroma: T,
    hue: impl Into<palette::OklabHue<T>>,
    alpha: T,
) -> palette::Oklcha<T> {
    palette::Oklcha::new(lightness, chroma, hue, alpha)
}

/// Generates the JsonSchema for [`Hsla`].
pub fn hsla_schemar(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
    <Hsla as schemars::JsonSchema>::json_schema(generator)
}

/// Generates the JsonSchema for [`Rgba`].
pub fn rgba_schemar(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
    <Rgba as schemars::JsonSchema>::json_schema(generator)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_types::color::solid_background;

    #[test]
    fn scene_hsla_is_the_upstream_hsla() {
        let color: SceneHsla = Hsla {
            h: 0.25,
            s: 0.5,
            l: 0.5,
            a: 1.0,
        };
        let upstream: Hsla = color;
        assert_eq!(upstream.h, 0.25);
    }

    #[test]
    fn background_kind_decodes_a_solid_color() {
        let color = Hsla {
            h: 0.5,
            s: 0.5,
            l: 0.5,
            a: 1.0,
        };
        assert_eq!(solid_background(color).kind(), BackgroundKind::Solid(color));
    }

    #[test]
    fn oklcha_constructs_a_palette_color() {
        let color = oklcha(0.5, 0.1, palette::OklabHue::new(120.0_f32), 1.0);
        assert_eq!(color.alpha, 1.0);
    }
}
