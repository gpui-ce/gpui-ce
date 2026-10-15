use palette::{IntoColor, OklabHue, Oklcha, RgbHue};
use schemars::{JsonSchema, json_schema};
use serde::{Deserialize, Serialize};
use std::fmt::{self, Display, Formatter};

// Re-exported for api maintenance
pub use palette::{Hsla, rgb::Rgba};

/// Creates an opaque [`Rgba`] color from a `0xRRGGBB` hex number.
///
/// The highest byte of `hex` is ignored, and the alpha is `1.0`. Use [`rgba`] for a color
/// with alpha.
///
/// # Examples
///
/// ```
/// use gpui::rgb;
///
/// assert_eq!(rgb(0xffffff).alpha, 1.0);
/// ```
pub fn rgb(hex: u32) -> Rgba {
    let [_, r, g, b] = hex.to_be_bytes().map(|b| (b as f32) / 255.0);
    Rgba {
        color: palette::rgb::Rgb::new(r, g, b),
        alpha: 1.0,
    }
}

/// Creates an [`Rgba`] color from a `0xRRGGBBAA` hex number.
///
/// # Examples
///
/// ```
/// use gpui::rgba;
///
/// let half_red = rgba(0xff000080);
/// assert!((half_red.alpha - 128.0 / 255.0).abs() < 1e-6);
/// ```
pub fn rgba(hex: u32) -> Rgba {
    let [r, g, b, a] = hex.to_be_bytes().map(|b| (b as f32) / 255.0);
    Rgba::new(r, g, b, a)
}

/// Converts an sRGB [`Rgba`] color to [`Hsla`], keeping the alpha.
///
/// Use it at an API boundary that stores [`Hsla`], such as [`Styled::bg`](crate::Styled::bg),
/// when the value you have is [`Rgba`].
pub fn rgb_to_hsla(color: Rgba) -> Hsla {
    color.into_color()
}

/// Converts an [`Hsla`] color to sRGB [`Rgba`], keeping the alpha.
pub fn hsla_to_rgba(color: Hsla) -> Rgba {
    color.into_color()
}

/// Converts one premultiplied RGBA pixel to straight-alpha BGRA, in place.
///
/// The function reads and writes only the first four bytes of `color`. Bytes after the
/// first pixel are left unchanged.
///
/// # Panics
///
/// Panics if `color` has fewer than four bytes.
pub fn swap_rgba_pa_to_bgra(color: &mut [u8]) {
    color.swap(0, 2);
    if color[3] > 0 {
        let a = color[3] as f32 / 255.;
        color[0] = (color[0] as f32 / a) as u8;
        color[1] = (color[1] as f32 / a) as u8;
        color[2] = (color[2] as f32 / a) as u8;
    }
}

/// Creates an [`Hsla`] color from hue, saturation, lightness and alpha.
///
/// Every argument is a fraction from `0.0` to `1.0`; values outside that range are clamped.
/// The hue is a fraction of one turn, not degrees: `0.0` is red and `1.0 / 3.0` is green.
/// If you have a hue in degrees, use [`oklcha`] or convert it before calling this function.
///
/// # Examples
///
/// ```
/// use gpui::{hsla, red};
///
/// assert_eq!(hsla(0.0, 1.0, 0.5, 1.0), red());
/// ```
pub const fn hsla(h: f32, s: f32, l: f32, a: f32) -> Hsla {
    Hsla {
        color: palette::Hsl::new_const(
            // `RgbHue` stores degrees, so the 0..1 fraction of the circle needs
            // scaling to 0..360 before it's wrapped.
            RgbHue::new(h.clamp(0., 1.) * 360.),
            s.clamp(0., 1.),
            l.clamp(0., 1.),
        ),
        alpha: a.clamp(0., 1.),
    }
}

/// Creates an [`Oklcha`](palette::Oklcha) color from lightness, chroma, hue and alpha.
///
/// Unlike [`hsla`], the arguments are not clamped. The hue is in degrees. Use this for
/// perceptually even colors, such as palettes that keep the same lightness across hues.
pub fn oklcha<T>(lightness: T, chroma: T, hue: impl Into<OklabHue<T>>, alpha: T) -> Oklcha<T> {
    Oklcha::new(lightness, chroma, hue, alpha)
}

/// Opaque black in [`Hsla`].
pub const fn black() -> Hsla {
    Hsla::new_const(RgbHue::new(0.), 0., 0., 1.)
}

/// Black with zero alpha in [`Hsla`].
pub const fn transparent_black() -> Hsla {
    Hsla::new_const(RgbHue::new(0.), 0., 0., 0.)
}

/// White with zero alpha in [`Hsla`].
pub const fn transparent_white() -> Hsla {
    Hsla::new_const(RgbHue::new(0.), 0., 1., 0.)
}

/// A grey in [`Hsla`] with the given lightness and opacity.
///
/// Both arguments are fractions from `0.0` to `1.0`. This constructor does not clamp them, so
/// callers must keep them in range.
pub const fn opaque_grey(lightness: f32, opacity: f32) -> Hsla {
    Hsla::new_const(RgbHue::new(0.), 0., lightness, opacity)
}

/// Opaque white in [`Hsla`].
pub const fn white() -> Hsla {
    Hsla::new_const(RgbHue::new(0.), 0., 1., 1.)
}

/// Opaque red in [`Hsla`], at hue `0.0`.
pub const fn red() -> Hsla {
    Hsla::new_const(RgbHue::new(0.), 1., 0.5, 1.)
}

/// Opaque blue in [`Hsla`], at hue 240 degrees.
pub const fn blue() -> Hsla {
    Hsla::new_const(RgbHue::new(240.), 1., 0.5, 1.)
}

/// A dark, opaque green in [`Hsla`], at hue 120 degrees and lightness 0.25.
pub const fn green() -> Hsla {
    Hsla::new_const(RgbHue::new(120.), 1., 0.25, 1.)
}

/// Opaque yellow in [`Hsla`], at hue 60 degrees.
pub const fn yellow() -> Hsla {
    Hsla::new_const(RgbHue::new(60.), 1., 0.5, 1.)
}

/// Generates the JsonSchema for palette::Hsla
pub fn hsla_schemar(_generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
    // https://github.com/Ogeon/palette/blob/9aa1ac21a7da60db398e8c044a43dbf3fdaf4855/palette/src/hsl.rs#L633-L636
    // https://github.com/Ogeon/palette/blob/9aa1ac21a7da60db398e8c044a43dbf3fdaf4855/palette/src/alpha/alpha.rs#L1197
    json_schema!({
        "type": "object",
        "properties": {
          "hue": {
              "type": "number",
              "format": "float"
          },
          "saturation": {
              "type": "number",
              "format": "float"
          },
          "lightness": {
              "type": "number",
              "format": "float"
          },
          "alpha": {
              "type": "number",
              "format": "float"
          },
        }
    })
}

/// Generates the JsonSchema for palette::Rgba
pub fn rgba_schemar(_generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
    // https://github.com/Ogeon/palette/blob/9aa1ac21a7da60db398e8c044a43dbf3fdaf4855/palette/src/rgb/rgb.rs#L1690
    // https://github.com/Ogeon/palette/blob/9aa1ac21a7da60db398e8c044a43dbf3fdaf4855/palette/src/alpha/alpha.rs#L1197
    json_schema!({
        "type": "object",
        "properties": {
          "red": {
              "type": "number",
              "format": "float"
          },
          "green": {
              "type": "number",
              "format": "float"
          },
          "blue": {
              "type": "number",
              "format": "float"
          },
          "alpha": {
              "type": "number",
              "format": "float"
          },
        }
    })
}

/// Helpers for changing the alpha of a color.
///
/// Implemented for [`Rgba`] and [`Hsla`]. The factors in these methods are clamped to
/// `0.0..=1.0`.
pub trait ColorExt {
    /// Composites `other` over this color with Porter–Duff source-over.
    fn blend(&self, other: &Self) -> Self
    where
        Self: Sized;
    /// Reduces the alpha of this color in place by `factor`.
    ///
    /// A factor of `0.0` leaves the color unchanged, and `1.0` makes it fully transparent.
    /// Values outside `0.0..=1.0` are clamped.
    fn fade_out(&mut self, factor: f32);
    /// Returns a copy of this color with its alpha multiplied by `factor`.
    ///
    /// If the color is opaque, the result has alpha `factor`. Use this for colors whose
    /// opacity comes from data, such as a color chosen at runtime. Values outside
    /// `0.0..=1.0` are clamped.
    ///
    /// # Examples
    ///
    /// ```
    /// use gpui::ColorExt;
    ///
    /// let faded = gpui::red().opacity(0.5);
    /// assert_eq!(faded.alpha, 0.5);
    /// ```
    ///
    /// Multiplying a color that already has alpha gives a result that is less opaque:
    ///
    /// ```
    /// use gpui::{ColorExt, hsla};
    ///
    /// let color = hsla(0.7, 1.0, 0.5, 0.7);
    /// let faded = color.opacity(0.16);
    /// assert!((faded.alpha - 0.112).abs() < 1e-6);
    /// ```
    fn opacity(&self, factor: f32) -> Self
    where
        Self: Sized;
}
impl ColorExt for Rgba {
    fn blend(&self, other: &Self) -> Self {
        use palette::blend::Compose;
        other.over(*self)
    }

    fn fade_out(&mut self, factor: f32) {
        self.alpha *= 1.0 - factor.clamp(0., 1.);
    }

    fn opacity(&self, factor: f32) -> Self {
        let mut color = *self;
        color.alpha *= factor.clamp(0., 1.);
        color
    }
}
impl ColorExt for Hsla {
    fn blend(&self, other: &Self) -> Self {
        let this: Rgba = (*self).into_color();
        let other: Rgba = (*other).into_color();
        this.blend(&other).into_color()
    }

    fn fade_out(&mut self, factor: f32) {
        self.alpha *= 1.0 - factor.clamp(0., 1.);
    }

    fn opacity(&self, factor: f32) -> Self {
        let mut color = *self;
        color.alpha *= factor.clamp(0., 1.);
        color
    }
}

/// The storage-buffer discriminant for a [`Background`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[repr(u32)]
pub enum BackgroundTag {
    /// A flat color.
    Solid = 0,
    /// A two-stop linear gradient.
    LinearGradient = 1,
    /// Diagonal color stripes over transparency.
    PatternSlash = 2,
    /// Alternating colored and transparent squares.
    Checkerboard = 3,
}

/// A color space for color interpolation.
///
/// References:
/// - <https://developer.mozilla.org/en-US/docs/Web/CSS/color-interpolation-method>
/// - <https://www.w3.org/TR/css-color-4/#typedef-color-space>
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[repr(u32)]
pub enum ColorSpace {
    #[default]
    /// The sRGB color space.
    Srgb = 0,
    /// The Oklab color space.
    Oklab = 1,
}

impl Display for ColorSpace {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            ColorSpace::Srgb => write!(f, "sRGB"),
            ColorSpace::Oklab => write!(f, "Oklab"),
        }
    }
}

/// A fill for a shape: a solid color, a linear gradient, or a pattern.
///
/// Build a background with [`solid_background`], [`linear_gradient`], [`pattern_slash`] or
/// [`checkerboard`], or convert a color with `.into()`. Elements take a background through
/// [`Styled::bg`](crate::Styled::bg). Use [`Background::kind`] to inspect one.
///
/// # Examples
///
/// ```
/// use gpui::{Background, blue, linear_color_stop, linear_gradient, red};
///
/// let solid: Background = red().into();
/// assert!(solid.as_solid().is_some());
///
/// let gradient = linear_gradient(
///     90.,
///     linear_color_stop(red(), 0.),
///     linear_color_stop(blue(), 1.),
/// );
/// assert!(gradient.as_solid().is_none());
/// ```
#[derive(Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[repr(C)]
pub struct Background {
    pub(crate) tag: BackgroundTag,
    pub(crate) color_space: ColorSpace,
    pub(crate) solid: crate::SceneHsla,
    pub(crate) gradient_angle_or_pattern_height: f32,
    pub(crate) colors: [LinearColorStop; 2],
    /// Padding for alignment for repr(C) layout.
    pub(crate) padding: u32,
}

impl std::fmt::Debug for Background {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self.tag {
            BackgroundTag::Solid => write!(f, "Solid({:?})", self.solid),
            BackgroundTag::LinearGradient => write!(
                f,
                "LinearGradient({}, {:?}, {:?})",
                self.gradient_angle_or_pattern_height, self.colors[0], self.colors[1]
            ),
            BackgroundTag::PatternSlash => write!(
                f,
                "PatternSlash({:?}, {})",
                self.solid, self.gradient_angle_or_pattern_height
            ),
            BackgroundTag::Checkerboard => write!(
                f,
                "Checkerboard({:?}, {})",
                self.solid, self.gradient_angle_or_pattern_height
            ),
        }
    }
}

impl Eq for Background {}
impl Default for Background {
    /// Returns a solid background with zero alpha, which paints nothing.
    fn default() -> Self {
        Self {
            tag: BackgroundTag::Solid,
            solid: Hsla::default().into(),
            color_space: ColorSpace::default(),
            gradient_angle_or_pattern_height: 0.0,
            colors: [LinearColorStop::default(), LinearColorStop::default()],
            padding: 0,
        }
    }
}

/// Creates a background of diagonal stripes in `color`, over transparency.
///
/// `width` is the width of each stripe and `interval` is the gap between stripes, both in
/// logical pixels. Both are stored in steps of 1/255 pixel and packed into one `f32`. The
/// packing is exact only for widths up to about 1px and intervals up to 257px. Larger
/// values lose precision or overflow into the width.
pub fn pattern_slash(color: impl IntoColor<Hsla>, width: f32, interval: f32) -> Background {
    const PATTERN_COMPONENT_SCALE: f32 = u8::MAX as f32;
    const PATTERN_PACKING_RADIX: u32 = u16::MAX as u32;

    let width_scaled = (width * PATTERN_COMPONENT_SCALE) as u32;
    let interval_scaled = (interval * PATTERN_COMPONENT_SCALE) as u32;
    let height = (width_scaled * PATTERN_PACKING_RADIX + interval_scaled) as f32;

    Background {
        tag: BackgroundTag::PatternSlash,
        solid: color.into_color().into(),
        gradient_angle_or_pattern_height: height,
        ..Default::default()
    }
}

/// Creates a checkerboard of squares in `color`, alternating with transparency.
///
/// `size` is the width and height of each square in logical pixels.
pub fn checkerboard(color: impl IntoColor<Hsla>, size: f32) -> Background {
    Background {
        tag: BackgroundTag::Checkerboard,
        solid: color.into_color().into(),
        gradient_angle_or_pattern_height: size,
        ..Default::default()
    }
}

/// Creates a solid background of `color`.
///
/// This is the same as converting `color` with `.into()`.
pub fn solid_background(color: impl IntoColor<Hsla>) -> Background {
    Background {
        solid: color.into_color().into(),
        ..Default::default()
    }
}

/// Creates a linear gradient between two color stops.
///
/// `angle` is the direction of the gradient line in degrees, from `0.` (up) increasing
/// clockwise. The colors are interpolated in [`ColorSpace::Oklab`]. Call
/// [`Background::color_space`] to interpolate in sRGB instead.
///
/// # Examples
///
/// ```
/// use gpui::{ColorSpace, blue, linear_color_stop, linear_gradient, red};
///
/// let gradient = linear_gradient(
///     180.,
///     linear_color_stop(red(), 0.),
///     linear_color_stop(blue(), 1.),
/// );
/// assert_eq!(gradient.interpolation_space(), ColorSpace::Oklab);
/// ```
///
/// <https://developer.mozilla.org/en-US/docs/Web/CSS/gradient/linear-gradient>
pub fn linear_gradient(
    angle: f32,
    from: impl Into<LinearColorStop>,
    to: impl Into<LinearColorStop>,
) -> Background {
    Background {
        tag: BackgroundTag::LinearGradient,
        gradient_angle_or_pattern_height: angle,
        colors: [from.into(), to.into()],
        color_space: ColorSpace::Oklab,
        ..Default::default()
    }
}

/// A color stop in a linear gradient.
///
/// <https://developer.mozilla.org/en-US/docs/Web/CSS/gradient/linear-gradient#linear-color-stop>
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[repr(C)]
pub struct LinearColorStop {
    /// The color of the color stop.
    pub color: crate::SceneHsla,
    /// The percentage of the gradient, in the range 0.0 to 1.0.
    pub percentage: f32,
}

/// Creates a color stop of `color` at `percentage` along a gradient line.
///
/// `percentage` is in the range `0.0` (the start of the gradient) to `1.0` (the end).
pub fn linear_color_stop(color: impl IntoColor<Hsla>, percentage: f32) -> LinearColorStop {
    LinearColorStop {
        color: color.into_color().into(),
        percentage,
    }
}

impl LinearColorStop {
    /// Returns a copy of this stop with its alpha multiplied by `factor`.
    ///
    /// The factor is clamped to `0.0..=1.0`, as with [`ColorExt::opacity`].
    pub fn opacity(&self, factor: f32) -> Self {
        let color: Hsla = self.color.into();
        Self {
            percentage: self.percentage,
            color: color.opacity(factor).into(),
        }
    }
}

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

impl Background {
    /// Returns the color if this is a solid background, and `None` otherwise.
    pub fn as_solid(&self) -> Option<Hsla> {
        if self.tag == BackgroundTag::Solid {
            Some(self.solid.into())
        } else {
            None
        }
    }

    /// Returns the decoded form of this background.
    pub fn kind(&self) -> BackgroundKind {
        match self.tag {
            BackgroundTag::Solid => BackgroundKind::Solid(self.solid.into()),
            BackgroundTag::LinearGradient => BackgroundKind::LinearGradient {
                angle: self.gradient_angle_or_pattern_height,
                stops: self.colors,
            },
            BackgroundTag::PatternSlash => {
                const PATTERN_COMPONENT_SCALE: f32 = u8::MAX as f32;
                const PATTERN_PACKING_RADIX: f32 = u16::MAX as f32;

                // `pattern_slash` packs both values into one exactly representable f32;
                // floor + rem_euclid inverts it, since truncation and `%` give the wrong entry.
                let packed = self.gradient_angle_or_pattern_height;
                BackgroundKind::PatternSlash {
                    color: self.solid.into(),
                    width: (packed / PATTERN_PACKING_RADIX).floor() / PATTERN_COMPONENT_SCALE,
                    interval: (packed.rem_euclid(PATTERN_PACKING_RADIX)) / PATTERN_COMPONENT_SCALE,
                }
            }
            BackgroundTag::Checkerboard => BackgroundKind::Checkerboard {
                color: self.solid.into(),
                size: self.gradient_angle_or_pattern_height,
            },
        }
    }

    /// Sets the color space that a gradient's colors are interpolated in.
    ///
    /// The default for [`linear_gradient`] is [`ColorSpace::Oklab`]. Solid colors and patterns
    /// don't interpolate, so this has no effect on them.
    ///
    /// <https://developer.mozilla.org/en-US/docs/Web/CSS/color-interpolation-method>
    pub fn color_space(mut self, color_space: ColorSpace) -> Self {
        self.color_space = color_space;
        self
    }

    /// The color space used to interpolate this background, set by [`Background::color_space`].
    pub fn interpolation_space(&self) -> ColorSpace {
        self.color_space
    }

    /// Returns a copy of this background with the alpha of every color multiplied by `factor`.
    ///
    /// The factor is clamped to `0.0..=1.0`. Gradient stops are scaled too.
    pub fn opacity(&self, factor: f32) -> Self {
        let mut background = *self;
        let solid: Hsla = background.solid.into();
        background.solid = solid.opacity(factor).into();
        background.colors = [
            self.colors[0].opacity(factor),
            self.colors[1].opacity(factor),
        ];
        background
    }

    /// Returns whether this background paints nothing.
    ///
    /// A solid, pattern or checkerboard background is transparent when its color has zero
    /// alpha. A gradient is transparent only when both of its stops have zero alpha.
    pub fn is_transparent(&self) -> bool {
        match self.tag {
            BackgroundTag::Solid => self.solid.a == 0.,
            BackgroundTag::LinearGradient => self.colors.iter().all(|c| c.color.a == 0.),
            BackgroundTag::PatternSlash => self.solid.a == 0.,
            BackgroundTag::Checkerboard => self.solid.a == 0.,
        }
    }
}

impl<T: IntoColor<Hsla>> From<T> for Background {
    /// Converts a color into a solid [`Background`].
    fn from(value: T) -> Self {
        Self {
            tag: BackgroundTag::Solid,
            solid: value.into_color().into(),
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_blend_composites_the_overlay_over_the_base() {
        let assert_close = |actual: Rgba, expected: Rgba| {
            for (actual, expected) in [
                (actual.red, expected.red),
                (actual.green, expected.green),
                (actual.blue, expected.blue),
                (actual.alpha, expected.alpha),
            ] {
                assert!(
                    (actual - expected).abs() < 1e-6,
                    "{actual:?} != {expected:?}"
                );
            }
        };

        let base = Rgba::new(1.0, 0.0, 0.0, 1.0);
        assert_close(base.blend(&Rgba::new(0.0, 0.0, 1.0, 0.0)), base);
        assert_close(
            base.blend(&Rgba::new(0.0, 0.0, 1.0, 1.0)),
            Rgba::new(0.0, 0.0, 1.0, 1.0),
        );
        assert_close(
            base.blend(&Rgba::new(0.0, 0.0, 1.0, 0.25)),
            Rgba::new(0.75, 0.0, 0.25, 1.0),
        );
    }

    #[test]
    fn highlight_keeps_the_base_color_under_a_transparent_overlay() {
        let base = crate::TextStyle {
            color: red(),
            ..Default::default()
        };
        let transparent_blue = Hsla::new_const(RgbHue::new(240.), 1., 0.5, 0.);
        let highlighted = base
            .clone()
            .highlight(crate::HighlightStyle::color(transparent_blue));
        assert_eq!(highlighted.color, base.color);
    }

    #[test]
    fn test_background_solid() {
        let color: Hsla = rgba(0xff0099ff).into_color();
        let mut background = Background::from(color);
        assert_eq!(background.tag, BackgroundTag::Solid);
        assert_eq!(background.solid, color.into());

        assert_eq!(background.opacity(0.5).solid, color.opacity(0.5).into());
        assert!(!background.is_transparent());
        background.solid = hsla(0.0, 0.0, 0.0, 0.0).into();
        assert!(background.is_transparent());
    }

    #[test]
    fn test_background_linear_gradient() {
        let from = linear_color_stop(rgba(0xff0099ff), 0.0);
        let to = linear_color_stop(rgba(0x00ff99ff), 1.0);
        let background = linear_gradient(90.0, from, to);
        assert_eq!(background.tag, BackgroundTag::LinearGradient);
        assert_eq!(background.colors[0], from);
        assert_eq!(background.colors[1], to);

        assert_eq!(background.opacity(0.5).colors[0], from.opacity(0.5));
        assert_eq!(background.opacity(0.5).colors[1], to.opacity(0.5));
        assert!(!background.is_transparent());
        assert!(background.opacity(0.0).is_transparent());
    }

    #[test]
    fn test_background_kind() {
        let color: Hsla = rgba(0xff0099ff).into_color();
        assert_eq!(Background::from(color).kind(), BackgroundKind::Solid(color));

        let from = linear_color_stop(rgba(0xff0099ff), 0.0);
        let to = linear_color_stop(rgba(0x00ff99ff), 1.0);
        assert_eq!(
            linear_gradient(90.0, from, to).kind(),
            BackgroundKind::LinearGradient {
                angle: 90.0,
                stops: [from, to],
            }
        );

        assert_eq!(
            checkerboard(color, 12.0).kind(),
            BackgroundKind::Checkerboard { color, size: 12.0 }
        );
    }

    #[test]
    fn test_background_kind_unpacks_pattern_slash() {
        let color: Hsla = rgba(0xff0099ff).into_color();
        // Both values survive to the 1/255 the constructor quantizes them to.
        for (width, interval) in [(1.0, 3.0), (0.5, 0.25), (2.0, 10.0)] {
            let BackgroundKind::PatternSlash {
                width: got_width,
                interval: got_interval,
                ..
            } = pattern_slash(color, width, interval).kind()
            else {
                panic!("pattern_slash did not produce a PatternSlash");
            };
            assert!((got_width - width).abs() <= 1.0 / 255.0);
            assert!((got_interval - interval).abs() <= 1.0 / 255.0);
        }
    }

    #[test]
    fn test_rgba_alpha() {
        use palette::WithAlpha;
        let color = Rgba::<palette::Srgb>::new(0.2, 0.6, 1.0, 0.8);
        assert_eq!(color.with_alpha(0.25).alpha, 0.25);
        // NOTE: diverging from upstream, where Rgba::alpha clamps. palette does not clamp alpha
        assert_eq!(color.with_alpha(1.5).alpha, 1.5);
    }

    #[test]
    fn test_rgba_opacity() {
        use super::ColorExt;
        let color = Rgba::new(0.2, 0.6, 1.0, 0.8);
        assert!((color.opacity(0.5).alpha - 0.4).abs() < 1e-6);
        assert_eq!(color.opacity(2.0).alpha, 0.8);
    }
}
