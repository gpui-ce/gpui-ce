use crate::{FontFallbacks, FontFeatures, FontStyle, FontWeight, FontWidth};
use gpui_shared_string::SharedString;

/// An opaque identifier for a font face that the [`TextSystem`](crate::TextSystem) has resolved.
///
/// Get one with [`crate::TextSystem::resolve_font`]. The id is valid for the lifetime of
/// the [`TextSystem`](crate::TextSystem) that produced it and is used for metrics and glyph rasterization.
#[derive(Hash, PartialEq, Eq, Clone, Copy, Debug)]
#[repr(C)]
pub struct FontId(pub usize);

/// The face to use for text: a font family plus its weight, width, style, features, and fallbacks.
///
/// Build one with [`font`] and then adjust it with the builder methods, or set
/// the public fields directly. The [`TextSystem`](crate::TextSystem) caches lookups by `Font`, so
/// equal fonts share one lookup.
///
/// The default is the family `.SystemUIFont`, which the platform maps to its
/// system interface font. Use a named family when the look must be the same
/// on every platform.
///
/// # Examples
///
/// ```
/// # use gpui::{Font, FontStyle, FontWeight, font};
/// let heading = font("Lilex").bold();
/// assert_eq!(heading.weight, FontWeight::BOLD);
///
/// let quote = Font {
///     style: FontStyle::Italic,
///     ..font("Source Serif 4")
/// };
/// assert_eq!(quote.style, FontStyle::Italic);
/// ```
///
/// See [text](crate::_documentation::guides::text) for how fonts are chosen and
/// how to add your own.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct Font {
    /// The font family name, such as `"Lilex"`.
    ///
    /// The special name `.SystemUIFont` refers to the system interface font,
    /// which differs by platform. Names come from [`crate::TextSystem::all_font_names`].
    pub family: SharedString,

    /// The OpenType features to apply, such as ligatures or tabular figures.
    pub features: FontFeatures,

    /// Families to try, in order, when [`Font::family`] has no glyph for a character.
    pub fallbacks: Option<FontFallbacks>,

    /// The weight of the face. See [`FontWeight`].
    pub weight: FontWeight,

    /// The width of the face. Exhaustive font literals must supply this field.
    pub width: FontWidth,

    /// Whether to select the italic or oblique face. See [`FontStyle`].
    pub style: FontStyle,
}

impl Default for Font {
    /// Returns the `.SystemUIFont` family with normal weight, width, and style.
    fn default() -> Self {
        font(".SystemUIFont")
    }
}

/// Returns a [`Font`] for `family`, with normal weight, width, and style.
///
/// Pass a family name from [`crate::TextSystem::all_font_names`] or one you added with
/// [`crate::TextSystem::add_fonts`]. Chain [`Font::bold`] or [`Font::italic`] to change
/// the style.
///
/// # Examples
///
/// ```
/// # use gpui::{FontStyle, FontWeight, font};
/// let body = font("Lilex");
/// assert_eq!(body.family.as_ref(), "Lilex");
/// assert_eq!(body.weight, FontWeight::NORMAL);
/// assert_eq!(body.style, FontStyle::Normal);
/// ```
pub fn font(family: impl Into<SharedString>) -> Font {
    Font {
        family: family.into(),
        features: FontFeatures::default(),
        weight: FontWeight::default(),
        width: FontWidth::default(),
        style: FontStyle::default(),
        fallbacks: None,
    }
}

impl Font {
    /// Returns this font with the weight set to [`FontWeight::BOLD`].
    pub fn bold(mut self) -> Self {
        self.weight = FontWeight::BOLD;
        self
    }

    /// Returns this font with the style set to [`FontStyle::Italic`].
    pub fn italic(mut self) -> Self {
        self.style = FontStyle::Italic;
        self
    }
}
