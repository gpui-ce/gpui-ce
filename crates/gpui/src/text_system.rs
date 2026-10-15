#[cfg(test)]
use crate::{
    AtlasKey, AtlasTextureKind, PlatformAtlas, TestAtlas, TestTextSystem, hsla, point, size,
};

#[cfg(test)]
use std::{sync::atomic::AtomicUsize, sync::atomic::Ordering};

use crate::{
    Bounds, DevicePixels, Pixels, PlatformTextSystem, Point, ResolvedDirection, Result,
    SharedString, Size, StrikethroughStyle, TextAlign, TextRenderingMode, UnderlineStyle,
    UnicodeBidi, px,
};
use anyhow::{Context as _, anyhow};
use collections::FxHashMap;
use core::fmt;
use derive_more::{Add, Deref, FromStr, Sub};
use palette::Hsla;
use parking_lot::{RwLock, RwLockUpgradableReadGuard};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;
use std::{
    borrow::Cow,
    fmt::{Debug, Display, Formatter},
    hash::{Hash, Hasher},
    ops::Range,
    sync::Arc,
};

mod font_fallbacks;
mod font_features;
mod font_width;
mod line;
mod line_layout;

pub use font_fallbacks::*;
pub use font_features::*;
pub use font_width::*;
pub use line::*;
pub use line_layout::*;

/// UTF-8 text range containment checks.
pub trait TextRangeExt {
    /// Returns whether `range` is ordered, in bounds, and lies on character boundaries.
    fn contains_range(&self, range: &Range<usize>) -> bool;
}

impl TextRangeExt for str {
    fn contains_range(&self, range: &Range<usize>) -> bool {
        self.get(range.clone()).is_some()
    }
}

/// An opaque identifier for a font face that the [`TextSystem`] has resolved.
///
/// Get one with [`TextSystem::resolve_font`]. The id is valid for the lifetime of
/// the [`TextSystem`] that produced it and is used for metrics and glyph rasterization.
#[derive(Hash, PartialEq, Eq, Clone, Copy, Debug)]
#[repr(C)]
pub struct FontId(pub usize);

/// The number of horizontal subpixel positions each glyph is rasterized at.
///
/// Glyphs are rasterized at quarter-pixel offsets along the X axis so that
/// positions which fall between device pixels stay sharp.
pub const SUBPIXEL_VARIANTS_X: u8 = 4;

/// The number of vertical subpixel positions each glyph is rasterized at.
///
/// Glyphs are not offset vertically, so there is one variant along the Y axis.
pub const SUBPIXEL_VARIANTS_Y: u8 = 1;

/// The platform-independent font database and metrics cache, shared by all windows.
///
/// Get it from [`App::text_system`](crate::App::text_system) and use it to register fonts with
/// [`TextSystem::add_fonts`], to measure glyphs and to resolve [`Font`]s to
/// [`FontId`]s. To shape text for painting, use the [`WindowTextSystem`] of the
/// window, which also caches the layouts.
pub struct TextSystem {
    platform_text_system: Arc<dyn PlatformTextSystem>,
    font_ids_by_font: RwLock<FxHashMap<Font, Result<FontId>>>,
    font_metrics: RwLock<FxHashMap<FontId, FontMetrics>>,
}

impl TextSystem {
    /// Creates a text system that delegates font work to `platform_text_system`.
    ///
    /// GPUI creates the text system for each application. You don't need to call this
    /// yourself, except in tests that supply their own platform text system.
    pub fn new(platform_text_system: Arc<dyn PlatformTextSystem>) -> Self {
        TextSystem {
            platform_text_system,
            font_metrics: RwLock::default(),
            font_ids_by_font: RwLock::default(),
        }
    }

    /// Returns the font family names that this text system can resolve.
    ///
    /// Use these names as [`Font::family`] values. On the web, the list contains
    /// only the families registered with [`TextSystem::add_fonts`] and the
    /// `.SystemUIFont` alias, because the web has no system fonts.
    pub fn all_font_names(&self) -> Vec<String> {
        self.platform_text_system.all_font_names()
    }

    /// Registers font files so that [`Font`]s can resolve to them.
    ///
    /// Each entry is the bytes of one font file, such as a `.ttf` or `.otf`.
    /// Use `Cow::Borrowed` for bytes compiled into the binary with
    /// `include_bytes!`, and `Cow::Owned` for bytes loaded at runtime. The
    /// family names come from each font's name table, so pass those names to
    /// [`font`] or set [`Font::family`] to use them.
    ///
    /// The web starts with an empty font database, so add the fonts your
    /// application uses before you open a window.
    ///
    /// # Errors
    ///
    /// Returns an error if an entry contains no font face that the platform
    /// supports. The parley backend, used on macOS, Windows, Linux, and the web, checks
    /// every entry before it registers any, so a failed call leaves the set of
    /// fonts unchanged.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::TestApp;
    /// # use std::borrow::Cow;
    /// # let mut app = TestApp::new();
    /// let bytes = include_bytes!(concat!(
    ///     env!("CARGO_MANIFEST_DIR"),
    ///     "/../../assets/fonts/lilex/Lilex-Regular.ttf"
    /// ));
    /// app.text_system()
    ///     .add_fonts(vec![Cow::Borrowed(bytes.as_slice())])
    ///     .expect("Lilex is a valid font");
    /// ```
    ///
    /// See [text](crate::_documentation::guides::text) for how fonts are chosen.
    pub fn add_fonts(&self, fonts: Vec<Cow<'static, [u8]>>) -> Result<()> {
        // Serialize registration with cache misses so an in-flight lookup cannot
        // repopulate a stale miss after the newly registered fonts become available.
        let mut font_ids = self.font_ids_by_font.write();
        let result = self.platform_text_system.add_fonts(fonts);
        font_ids.clear();
        result
    }

    /// Resolves a font family and style to a cached [`FontId`].
    fn font_id(&self, font: &Font) -> Result<FontId> {
        fn clone_font_id_result(font_id: &Result<FontId>) -> Result<FontId> {
            match font_id {
                Ok(font_id) => Ok(*font_id),
                Err(err) => Err(anyhow!("{err}")),
            }
        }

        let font_id = self
            .font_ids_by_font
            .read()
            .get(font)
            .map(clone_font_id_result);
        if let Some(font_id) = font_id {
            font_id
        } else {
            let mut font_ids = self.font_ids_by_font.write();
            if let Some(font_id) = font_ids.get(font) {
                return clone_font_id_result(font_id);
            }
            let font_id = self.platform_text_system.font_id(font);
            font_ids.insert(font.clone(), clone_font_id_result(&font_id));
            font_id
        }
    }

    /// Resolves a [`Font`] to the [`FontId`] of the face that will be used to draw it.
    ///
    /// Resolution applies the family, weight, style, width, and fallbacks, and
    /// the result is cached. Use the returned id with the metric methods on this
    /// type, such as [`TextSystem::ascent`].
    ///
    /// # Panics
    ///
    /// Panics if the platform cannot resolve the font. Pass the family name of
    /// a font from [`TextSystem::all_font_names`] or one added with
    /// [`TextSystem::add_fonts`] to avoid this.
    pub fn resolve_font(&self, font: &Font) -> FontId {
        self.font_id(font)
            .unwrap_or_else(|error| panic!("failed to resolve font '{}': {error}", font.family))
    }

    /// Resolves each font and loads its platform caches ahead of time.
    ///
    /// Loading system fonts can be slow, so call this from a background
    /// executor before the first frame that uses the fonts. Fonts that are not
    /// prewarmed are loaded when they are first used.
    ///
    /// # Panics
    ///
    /// Panics if any font cannot be resolved, as [`TextSystem::resolve_font`] does.
    pub fn prewarm_fonts(&self, fonts: &[Font]) {
        let mut font_ids = SmallVec::<[FontId; 8]>::new();
        for font in fonts {
            let font_id = self.resolve_font(font);
            if !font_ids.contains(&font_id) {
                font_ids.push(font_id);
            }
        }
        self.platform_text_system.prewarm_fonts(&font_ids);
    }

    /// Returns the font's bounding box at `font_size` in logical pixels.
    ///
    /// The bounding box is the smallest rectangle that encloses every glyph in
    /// the font when they are all drawn at the same origin.
    pub fn bounding_box(&self, font_id: FontId, font_size: Pixels) -> Bounds<Pixels> {
        self.read_metrics(font_id, |metrics| metrics.bounding_box(font_size))
    }

    /// Returns the bounding rectangle of the glyph for `character` at `font_size` in logical pixels.
    ///
    /// # Errors
    ///
    /// Returns an error if the font has no glyph for `character` or the platform
    /// cannot read its bounds.
    pub fn typographic_bounds(
        &self,
        font_id: FontId,
        font_size: Pixels,
        character: char,
    ) -> Result<Bounds<Pixels>> {
        let glyph_id = self
            .platform_text_system
            .glyph_for_char(font_id, character)
            .with_context(|| format!("glyph not found for character '{character}'"))?;
        let bounds = self
            .platform_text_system
            .typographic_bounds(font_id, glyph_id)?;
        Ok(self.read_metrics(font_id, |metrics| {
            (bounds / metrics.units_per_em as f32 * font_size.0).map(px)
        }))
    }

    /// Returns the advance of `ch` at `font_size`: how far the pen moves after drawing it.
    ///
    /// The `width` of the result is the advance, which is the glyph's horizontal
    /// width in the font, not its ink width. Layout uses it to place the next character.
    ///
    /// # Errors
    ///
    /// Returns an error if the font has no glyph for `ch`.
    pub fn advance(&self, font_id: FontId, font_size: Pixels, ch: char) -> Result<Size<Pixels>> {
        let glyph_id = self
            .platform_text_system
            .glyph_for_char(font_id, ch)
            .with_context(|| format!("glyph not found for character '{ch}'"))?;
        let result = self.platform_text_system.advance(font_id, glyph_id)?
            / self.units_per_em(font_id) as f32;

        Ok(result * font_size)
    }

    /// Returns the number of font units in the font's em square.
    ///
    /// The em square is the grid that font sizes are measured on. Divide a
    /// font-unit measurement by this value to get a fraction of the font size.
    pub fn units_per_em(&self, font_id: FontId) -> u32 {
        self.read_metrics(font_id, |metrics| metrics.units_per_em)
    }

    /// Returns the distance from the baseline up to the top of the font's tallest glyphs, at `font_size`.
    pub fn ascent(&self, font_id: FontId, font_size: Pixels) -> Pixels {
        self.read_metrics(font_id, |metrics| metrics.ascent(font_size))
    }

    /// Returns the distance from the baseline down to the bottom of the font's lowest glyphs, at `font_size`.
    ///
    /// The value is positive. It is the distance below the baseline, not a
    /// signed coordinate.
    pub fn descent(&self, font_id: FontId, font_size: Pixels) -> Pixels {
        self.read_metrics(font_id, |metrics| metrics.descent(font_size))
    }

    /// Returns the height of lowercase `x` at `font_size`.
    pub fn x_height(&self, font_id: FontId, font_size: Pixels) -> Pixels {
        self.read_metrics(font_id, |metrics| metrics.x_height(font_size))
    }

    /// Returns the distance from the top of a line box of `line_height` to the baseline.
    ///
    /// The ascent and descent are centered in the line box, so extra
    /// `line_height` is split evenly above and below the glyphs. Use this to
    /// place text inside a box with a fixed height.
    pub fn baseline_offset(
        &self,
        font_id: FontId,
        font_size: Pixels,
        line_height: Pixels,
    ) -> Pixels {
        let ascent = self.ascent(font_id, font_size);
        let descent = self.descent(font_id, font_size);
        let padding_top = (line_height - ascent - descent) / 2.;
        padding_top + ascent
    }

    fn read_metrics<T>(&self, font_id: FontId, read: impl FnOnce(&FontMetrics) -> T) -> T {
        let lock = self.font_metrics.upgradable_read();

        if let Some(metrics) = lock.get(&font_id) {
            read(metrics)
        } else {
            let mut lock = RwLockUpgradableReadGuard::upgrade(lock);
            let metrics = lock
                .entry(font_id)
                .or_insert_with(|| self.platform_text_system.font_metrics(font_id));
            read(metrics)
        }
    }

    /// Rasterizes one glyph to pixels, and checks that the result is well formed.
    ///
    /// GPUI calls this when it draws a glyph that is not in the glyph atlas yet.
    /// Application code rarely needs it.
    ///
    /// # Errors
    ///
    /// Returns an error if the platform cannot rasterize the glyph, or if the
    /// pixel buffer does not match the size and format it reports. See
    /// [`RasterizedGlyph::validate`].
    pub fn rasterize_glyph(&self, params: &RenderGlyphParams) -> Result<ValidatedRasterizedGlyph> {
        let glyph = self.platform_text_system.rasterize_glyph(params)?;

        ValidatedRasterizedGlyph::new(glyph)
    }

    /// Normalizes a requested scene color and render mode into the settings which affect the
    /// cached glyph raster.
    pub(crate) fn prepare_raster_style(
        &self,
        font_id: FontId,
        glyph_id: GlyphId,
        scene_color: Hsla,
        requested_mode: GlyphRenderMode,
    ) -> PreparedRasterStyle {
        self.platform_text_system
            .prepare_raster_style(RasterStyleRequest {
                font_id,
                glyph_id,
                scene_color: crate::hsla_to_rgba(scene_color),
                requested_mode,
                foreground_dependency: ForegroundDependency::Full,
            })
    }

    /// Returns the text rendering mode recommended by the platform for the given font and size.
    /// The return value will never be [`TextRenderingMode::PlatformDefault`].
    pub(crate) fn recommended_rendering_mode(
        &self,
        font_id: FontId,
        font_size: Pixels,
    ) -> TextRenderingMode {
        self.platform_text_system
            .recommended_rendering_mode(font_id, font_size)
    }
}

/// The text layout subsystem for one window, with a cache of shaped lines.
///
/// Get it with [`Window::text_system`](crate::Window::text_system). It derefs to the shared [`TextSystem`],
/// so you can call font methods on it directly.
///
/// Layout methods such as [`WindowTextSystem::shape_line`] and
/// [`WindowTextSystem::shape_text`] cache their results. A layout is reused when
/// the same text, font size, and shaping settings are requested again. The cache
/// keeps a layout only while the next frame uses it, so a layout that a frame
/// doesn't request is dropped at the end of that frame. Changing only colors or
/// decorations reuses the glyphs and repaints them.
#[derive(Deref)]
pub struct WindowTextSystem {
    line_layout_cache: LineLayoutCache,
    #[deref]
    text_system: Arc<TextSystem>,
}

impl WindowTextSystem {
    /// Creates a window text system with an empty layout cache on top of `text_system`.
    ///
    /// GPUI creates one for each window. You don't need to call this yourself.
    pub fn new(text_system: Arc<TextSystem>) -> Self {
        Self {
            line_layout_cache: LineLayoutCache::new(text_system.platform_text_system.clone()),
            text_system,
        }
    }

    pub(crate) fn layout_index(&self) -> LineLayoutIndex {
        self.line_layout_cache.layout_index()
    }

    pub(crate) fn reuse_layouts(&self, index: Range<LineLayoutIndex>) {
        self.line_layout_cache.reuse_layouts(index)
    }

    pub(crate) fn truncate_layouts(&self, index: LineLayoutIndex) {
        self.line_layout_cache.truncate_layouts(index)
    }

    /// Shapes one line of text, ready to paint.
    ///
    /// Use this for a single line of text that you paint with [`ShapedLine::paint`].
    /// For text that can wrap or has several paragraphs, use [`Self::shape_text`].
    /// `runs` styles the text, and the run lengths must add up to `text.len()`.
    /// Each run's [`TextRun::font`] selects the face used for its bytes.
    ///
    /// The result is cached for the frame, as described on [`WindowTextSystem`].
    ///
    /// # Panics
    ///
    /// In debug builds, panics if `text` contains a `\n`. Release builds don't
    /// check this, and the result for such text is not meaningful. Use
    /// [`Self::shape_text`] for multiple lines.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{EmptyView, TestApp, TextRun, font, hsla, px};
    /// # let mut app = TestApp::new();
    /// # let mut window = app.open_window(|_window, _cx| EmptyView);
    /// window.update(|_view, window, _cx| {
    ///     let run = TextRun {
    ///         len: "Hello".len(),
    ///         font: font("Lilex"),
    ///         color: hsla(0., 0., 0., 1.),
    ///         ..Default::default()
    ///     };
    ///     let line = window.text_system().shape_line("Hello".into(), px(16.), &[run]);
    ///     assert_eq!(line.len(), 5);
    /// });
    /// ```
    pub fn shape_line(
        &self,
        text: SharedString,
        font_size: Pixels,
        runs: &[TextRun],
    ) -> ShapedLine {
        debug_assert!(
            text.find('\n').is_none(),
            "text argument should not contain newlines"
        );

        let layout = self.layout_line(&text, font_size, runs);

        ShapedLine { layout, text }
    }

    /// Shapes text that may span several lines, ready to paint.
    ///
    /// This is the function to use for paragraphs. Each `\n` starts a new line.
    /// When `wrap_width` is `Some`, lines wrap at word boundaries to fit within
    /// that many logical pixels. A word wider than that is not split, so it
    /// overflows. `None` disables soft wrapping, and only `\n` breaks lines.
    /// With a wrap width, `line_clamp = Some(n)` limits soft wrapping: once the
    /// budget is exhausted, the rest of each paragraph occupies an unwrapped line.
    /// Hard paragraph breaks are preserved, so the total can exceed `n`.
    /// `Some(0)` behaves like `Some(1)` and disables soft wrapping; it does not hide text.
    ///
    /// `runs` styles the text. Each run gives the number of UTF-8 bytes it covers,
    /// and the runs must cover the whole string and end on character boundaries.
    /// The `Result` is currently always `Ok`.
    ///
    /// Shaping is cached as described on [`WindowTextSystem`]. To lay out text
    /// inside an element, use [`StyledText`](crate::StyledText) instead. See
    /// [text](crate::_documentation::guides::text) for an overview.
    ///
    /// # Examples
    ///
    /// This example supplies the platform's real text system. [`TestApp::new`](crate::TestApp::new)
    /// uses a simplified test shaper, which does not reproduce paragraph layout.
    ///
    /// ```standalone_crate
    /// # use gpui::{EmptyView, TestApp, TextRun, font, hsla, px};
    /// let text_system = gpui_platform::current_platform(true).text_system();
    /// let mut app = TestApp::with_text_system(text_system);
    /// # let mut window = app.open_window(|_window, _cx| EmptyView);
    /// window.update(|_view, window, _cx| {
    ///     let text = "first line\nsecond line";
    ///     let run = TextRun {
    ///         len: text.len(),
    ///         font: font(".SystemUIFont"),
    ///         color: hsla(0., 0., 0., 1.),
    ///         ..Default::default()
    ///     };
    ///     let shaped = window
    ///         .text_system()
    ///         .shape_text(text, px(16.), &[run.clone()], Some(px(200.)), None)
    ///         .unwrap();
    ///     assert_eq!(shaped.len(), text.len());
    ///
    ///     // A clamp cannot remove the hard break between these paragraphs.
    ///     let zero = window.text_system()
    ///         .shape_text(text, px(16.), &[run.clone()], Some(px(20.)), Some(0))
    ///         .unwrap();
    ///     let one = window.text_system()
    ///         .shape_text(text, px(16.), &[run], Some(px(20.)), Some(1))
    ///         .unwrap();
    ///     assert_eq!(zero.line_count(), 2);
    ///     assert_eq!(one.line_count(), 2);
    /// });
    /// ```
    pub fn shape_text<S: AsRef<str> + Into<SharedString>>(
        &self,
        text: S,
        font_size: Pixels,
        runs: &[TextRun],
        wrap_width: Option<Pixels>,
        line_clamp: Option<usize>,
    ) -> Result<ShapedText> {
        self.shape_text_with_options(
            text,
            font_size,
            runs,
            TextLayoutOptions {
                wrap_width,
                line_clamp,
                ..Default::default()
            },
        )
    }

    /// Shapes a text document with explicit paragraph direction, alignment, and bidi options.
    ///
    /// This is the lower-level form of [`Self::shape_text`]. Use it when text
    /// must align within a width different from its wrap width, or must follow
    /// a specific paragraph direction. The `Result` is currently always `Ok`.
    #[doc(hidden)]
    pub fn shape_text_with_options<S: AsRef<str> + Into<SharedString>>(
        &self,
        text: S,
        font_size: Pixels,
        runs: &[TextRun],
        options: TextLayoutOptions,
    ) -> Result<ShapedText> {
        let text = text.into();
        let layout = self
            .line_layout_cache
            .layout_text_with_options(&text, font_size, runs, options);

        Ok(ShapedText { layout, text })
    }

    /// Lays out text and atomic boxes together, the way an inline element flows in a paragraph.
    ///
    /// Each box is an element that sits between characters, such as an icon or
    /// a badge. Text wraps around the boxes. The result is cached like other
    /// layouts.
    pub fn layout_inline(&self, request: InlineLayoutRequest<'_>) -> Arc<InlineLayout> {
        self.line_layout_cache.layout_inline(request)
    }

    /// Lays out one line of text and returns the backend's [`LineLayout`].
    ///
    /// Most code should call [`Self::shape_line`] instead, which returns a
    /// [`ShapedLine`] that you can paint. Use this only when you need the raw
    /// layout. `text` must not contain newlines, and nothing checks this here.
    pub fn layout_line(&self, text: &str, font_size: Pixels, runs: &[TextRun]) -> Arc<LineLayout> {
        self.line_layout_cache
            .layout_line(&SharedString::new(text), font_size, runs)
    }

    pub(crate) fn finish_frame(&self) {
        self.line_layout_cache.finish_frame()
    }
}

/// The weight of a font, from [`FontWeight::THIN`] (100) to [`FontWeight::BLACK`] (900).
///
/// Weight selects how thick the strokes of a face are. [`FontWeight::NORMAL`]
/// (400) is the default, and [`FontWeight::BOLD`] (700) is what [`Font::bold`]
/// sets. The value is a plain `f32`, so you can use values between the named
/// constants when the font has faces for them.
///
/// # Examples
///
/// ```
/// # use gpui::{FontWeight, font};
/// let heading = font("Lilex").bold();
/// assert_eq!(heading.weight, FontWeight::BOLD);
/// assert_eq!(FontWeight::default(), FontWeight::NORMAL);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd, Serialize, Deserialize, Add, Sub, FromStr)]
#[serde(transparent)]
pub struct FontWeight(pub f32);

impl Display for FontWeight {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<f32> for FontWeight {
    fn from(weight: f32) -> Self {
        FontWeight(weight)
    }
}

impl Default for FontWeight {
    #[inline]
    fn default() -> FontWeight {
        FontWeight::NORMAL
    }
}

impl Hash for FontWeight {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_u32(u32::from_be_bytes(self.0.to_be_bytes()));
    }
}

impl Eq for FontWeight {}

impl FontWeight {
    /// Thin weight (100), the thinnest value.
    pub const THIN: FontWeight = FontWeight(100.0);
    /// Extra light weight (200).
    pub const EXTRA_LIGHT: FontWeight = FontWeight(200.0);
    /// Light weight (300).
    pub const LIGHT: FontWeight = FontWeight(300.0);
    /// Normal (400).
    pub const NORMAL: FontWeight = FontWeight(400.0);
    /// Medium weight (500, higher than normal).
    pub const MEDIUM: FontWeight = FontWeight(500.0);
    /// Semibold weight (600).
    pub const SEMIBOLD: FontWeight = FontWeight(600.0);
    /// Bold weight (700).
    pub const BOLD: FontWeight = FontWeight(700.0);
    /// Extra-bold weight (800).
    pub const EXTRA_BOLD: FontWeight = FontWeight(800.0);
    /// Black weight (900), the thickest value.
    pub const BLACK: FontWeight = FontWeight(900.0);

    /// All of the font weights, in order from thinnest to thickest.
    pub const ALL: [FontWeight; 9] = [
        Self::THIN,
        Self::EXTRA_LIGHT,
        Self::LIGHT,
        Self::NORMAL,
        Self::MEDIUM,
        Self::SEMIBOLD,
        Self::BOLD,
        Self::EXTRA_BOLD,
        Self::BLACK,
    ];
}

impl schemars::JsonSchema for FontWeight {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "FontWeight".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        use schemars::json_schema;
        json_schema!({
            "type": "number",
            "minimum": Self::THIN,
            "maximum": Self::BLACK,
            "default": Self::default(),
            "description": "Font weight value between 100 (thin) and 900 (black)"
        })
    }
}

/// Selects an italic or oblique face when the font family has one.
///
/// Use [`Font::italic`] to set [`FontStyle::Italic`].
#[derive(Clone, Copy, Eq, PartialEq, Debug, Hash, Default, Serialize, Deserialize, JsonSchema)]
pub enum FontStyle {
    /// The upright face of the family.
    #[default]
    Normal,
    /// The italic face, which is usually a cursive design.
    Italic,
    /// A sloped version of the upright face.
    Oblique,
}

impl Display for FontStyle {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        Debug::fmt(self, f)
    }
}

/// A stretch of text that shares one font and style.
///
/// Text layout takes a slice of runs whose `len` values add up to the length of
/// the text. Each run styles the bytes that follow the runs before it. Runs
/// control how text is shaped and painted, so changing a run's `color`,
/// `background_color`, `underline`, or `strikethrough` does not reshape the text.
///
/// # Examples
///
/// ```
/// # use gpui::{TextRun, font, hsla};
/// let text = "Hello world";
/// let runs = [
///     TextRun {
///         len: 6,
///         font: font("Lilex"),
///         color: hsla(0., 0., 0., 1.),
///         ..Default::default()
///     },
///     TextRun {
///         len: 5,
///         font: font("Lilex").bold(),
///         color: hsla(0., 0., 0., 1.),
///         ..Default::default()
///     },
/// ];
/// assert_eq!(runs.iter().map(|run| run.len).sum::<usize>(), text.len());
/// ```
#[derive(Clone, Debug, PartialEq, Default)]
pub struct TextRun {
    /// The number of UTF-8 bytes this run covers.
    pub len: usize,
    /// The font to use for this run.
    pub font: Font,
    /// The color of the glyphs.
    pub color: Hsla,
    /// The background color behind the glyphs, or `None` for no background.
    pub background_color: Option<Hsla>,
    /// The underline drawn under the glyphs, or `None` for no underline.
    pub underline: Option<UnderlineStyle>,
    /// The strikethrough drawn through the glyphs, or `None` for none.
    pub strikethrough: Option<StrikethroughStyle>,
    /// The extra space added between glyphs, in logical pixels, or `None` for no extra space.
    pub letter_spacing: Option<Pixels>,
}

/// The paragraph base-direction policy supplied to a text backend.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum ParagraphDirection {
    /// Determine paragraph direction from its source text.
    #[default]
    Auto,
    /// Use an explicit left-to-right paragraph base direction.
    LeftToRight,
    /// Use an explicit right-to-left paragraph base direction.
    RightToLeft,
}

impl From<ResolvedDirection> for ParagraphDirection {
    fn from(direction: ResolvedDirection) -> Self {
        match direction {
            ResolvedDirection::LeftToRight => Self::LeftToRight,
            ResolvedDirection::RightToLeft => Self::RightToLeft,
        }
    }
}

/// A directional scope within an inline text document.
#[doc(hidden)]
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct InlineBidiScope {
    /// UTF-8 source range covered by this scope.
    pub range: Range<usize>,
    /// Resolved direction used by the scope.
    pub direction: ResolvedDirection,
    /// The CSS `unicode-bidi` behavior for the scope.
    pub unicode_bidi: UnicodeBidi,
}

/// Direction, wrapping, and alignment options for a complete text document.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct TextLayoutOptions {
    /// Optional soft-wrap width.
    pub wrap_width: Option<Pixels>,
    /// Soft-wrap line budget. Hard paragraph breaks are preserved; `Some(0)` behaves
    /// like `Some(1)`. See [`WindowTextSystem::shape_text`].
    pub line_clamp: Option<usize>,
    /// Width used for horizontal alignment, independently of wrapping.
    pub alignment_width: Option<Pixels>,
    /// Horizontal text alignment.
    pub text_align: TextAlign,
    /// Paragraph base-direction policy.
    pub direction: ParagraphDirection,
    /// Bidirectional behavior applied to the complete document.
    pub unicode_bidi: UnicodeBidi,
}

/// Complete input for one backend-owned text document layout.
#[derive(Clone, Copy, Debug)]
pub struct TextLayoutRequest<'a> {
    /// UTF-8 source text.
    pub text: &'a str,
    /// Font size shared by all style runs.
    pub font_size: Pixels,
    /// Complete shaping and paint styles covering `text`.
    pub runs: &'a [TextRun],
    /// Wrapping, alignment, and bidirectional layout options.
    pub options: TextLayoutOptions,
}

/// An atomic element inserted at a UTF-8 boundary in an inline document.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct InlineBoxRequest {
    /// Identifier returned with the positioned box.
    pub id: u64,
    /// UTF-8 byte index at which to insert the box.
    pub index: usize,
    /// Measured size of the element.
    pub size: Size<Pixels>,
    /// Vertical alignment within the line containing this element.
    pub vertical_align: crate::VerticalAlign,
}

/// Resolved font size and line height for part of an inline document.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct InlineTextStyle {
    /// UTF-8 range receiving these resolved metrics.
    pub range: Range<usize>,
    /// Font size for this range.
    pub font_size: Pixels,
    /// Absolute line height for this range.
    pub line_height: Pixels,
}

/// Complete input for a document containing text and element boxes.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct InlineLayoutRequest<'a> {
    /// UTF-8 source text.
    pub text: &'a str,
    /// Complete shaping and paint styles covering `text`.
    pub runs: &'a [TextRun],
    /// Resolved font sizes and line heights within the document.
    pub text_styles: &'a [InlineTextStyle],
    /// Atomic boxes inserted into the text.
    pub boxes: &'a [InlineBoxRequest],
    /// Base font size.
    pub font_size: Pixels,
    /// Requested height of an ordinary text row.
    pub line_height: Pixels,
    /// Metrics for the inline container's base font.
    pub text_metrics: InlineTextMetrics,
    /// Wrapping, alignment, and bidirectional layout options.
    pub options: TextLayoutOptions,
    /// Directional scopes established by nested inline elements.
    pub bidi_scopes: &'a [InlineBidiScope],
}

impl Eq for TextRun {}

impl Hash for TextRun {
    fn hash<H: Hasher>(&self, state: &mut H) {
        fn hash_float<H: Hasher>(value: f32, state: &mut H) {
            if value == 0.0 {
                0.0f32.to_bits().hash(state);
            } else {
                value.to_bits().hash(state);
            }
        }

        fn hash_color<H: Hasher>(color: Hsla, state: &mut H) {
            let color = crate::hsla_to_rgba(color);
            hash_float(color.color.red, state);
            hash_float(color.color.green, state);
            hash_float(color.color.blue, state);
            hash_float(color.alpha, state);
        }

        self.len.hash(state);
        self.font.hash(state);
        hash_color(self.color, state);
        if let Some(color) = self.background_color {
            hash_color(color, state);
        }
        self.background_color.is_some().hash(state);
        self.underline.is_some().hash(state);
        if let Some(underline) = self.underline {
            hash_float(underline.thickness.into(), state);
            underline.color.is_some().hash(state);
            if let Some(color) = underline.color {
                hash_color(color, state);
            }
            underline.wavy.hash(state);
        }
        self.strikethrough.is_some().hash(state);
        if let Some(strikethrough) = self.strikethrough {
            hash_float(strikethrough.thickness.into(), state);
            strikethrough.color.is_some().hash(state);
            if let Some(color) = strikethrough.color {
                hash_color(color, state);
            }
        }
        self.letter_spacing.is_some().hash(state);
        if let Some(letter_spacing) = self.letter_spacing {
            hash_float(letter_spacing.into(), state);
        }
    }
}

impl From<&TextRun> for PaintStyle {
    fn from(run: &TextRun) -> Self {
        Self {
            color: run.color,
            background_color: run.background_color,
            underline: run.underline,
            strikethrough: run.strikethrough,
        }
    }
}

/// An identifier for a specific glyph, as returned by [`WindowTextSystem::layout_line`].
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
#[repr(C)]
pub struct GlyphId(pub u32);

/// Parameters for rendering a glyph, used as cache keys for glyph atlas entries.
///
/// This struct identifies a specific glyph rendering configuration including
/// font, size, subpixel positioning, and scale factor. It's used to look up
/// cached pixels, placement metadata, and sprite atlas entries.
#[derive(Clone, Debug, PartialEq)]
#[expect(missing_docs)]
pub struct RenderGlyphParams {
    pub font_id: FontId,
    pub glyph_id: GlyphId,
    pub font_size: Pixels,
    pub subpixel_variant: Point<u8>,
    pub scale_factor: f32,
    pub raster_style: PreparedRasterStyle,
}

/// The kind of glyph image requested from a rasterizer.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum GlyphRenderMode {
    /// A color-independent, one-channel coverage mask.
    Grayscale,
    /// A color-independent, three-channel subpixel coverage mask.
    Subpixel,
    /// A color glyph whose pixels include their final RGB values.
    Color,
}

impl GlyphRenderMode {
    /// Returns the byte format required for this render mode.
    pub const fn rasterized_format(self) -> RasterizedGlyphFormat {
        match self {
            Self::Grayscale => RasterizedGlyphFormat::AlphaMask,
            Self::Subpixel => RasterizedGlyphFormat::BgraSubpixelMask,
            Self::Color => RasterizedGlyphFormat::BgraColor,
        }
    }
}

/// Eight-bit sRGB color stored in a raster cache key when a native raster path needs it.
pub type Rgba8 = palette::Srgba<u8>;

/// Converts a scene color to the bytes used by a raster cache key.
pub trait RasterColorExt {
    /// Clamps and rounds each channel to the nearest byte.
    fn quantize_raster_color(self) -> Rgba8;
}

impl RasterColorExt for crate::Rgba {
    fn quantize_raster_color(self) -> Rgba8 {
        fn channel(value: f32) -> u8 {
            (value.clamp(0.0, 1.0) * 255.0).round() as u8
        }

        Rgba8::new(
            channel(self.red),
            channel(self.green),
            channel(self.blue),
            channel(self.alpha),
        )
    }
}

/// A scene request before a platform rasterizer normalizes its cache-relevant settings.
#[derive(Clone, Copy, Debug)]
pub struct RasterStyleRequest {
    /// The canonical font instance containing the glyph.
    pub font_id: FontId,
    /// The glyph whose artwork will be rasterized.
    pub glyph_id: GlyphId,
    /// The exact application color. Rasterizers may only retain a normalized derivative of it.
    pub scene_color: crate::Rgba,
    /// The requested kind of glyph image.
    pub requested_mode: GlyphRenderMode,
    /// Which foreground channels can alter the selected artwork.
    pub foreground_dependency: ForegroundDependency,
}

impl RasterStyleRequest {
    /// Quantizes the foreground after removing channels unused by the artwork.
    pub fn normalized_color(self) -> Rgba8 {
        let color = self.scene_color.quantize_raster_color();
        if self.foreground_dependency == ForegroundDependency::AlphaOnly {
            return Rgba8::new(0, 0, 0, color.alpha);
        }

        color
    }
}

/// Describes which foreground channels can change rasterized color artwork.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ForegroundDependency {
    /// The artwork or its fallback may use the complete foreground color.
    Full,
    /// Fixed RGB artwork uses only the foreground alpha.
    AlphaOnly,
}

/// The cache-relevant raster settings chosen by a platform rasterizer.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PreparedRasterStyle {
    /// The normalized render mode.
    pub mode: GlyphRenderMode,
    /// Any normalized color effect baked into coverage or color pixels.
    pub color_effect: RasterColorEffect,
    /// Which foreground channels can alter this raster or its fallback.
    pub foreground_dependency: ForegroundDependency,
}

impl PreparedRasterStyle {
    /// Creates a color-independent raster style.
    pub const fn independent(mode: GlyphRenderMode) -> Self {
        Self {
            mode,
            color_effect: RasterColorEffect::Independent,
            foreground_dependency: ForegroundDependency::Full,
        }
    }

    /// Creates a native preblended style from the request's relevant color channels.
    pub fn preblend(request: RasterStyleRequest) -> Self {
        Self {
            mode: request.requested_mode,
            color_effect: RasterColorEffect::Preblend(request.normalized_color()),
            foreground_dependency: request.foreground_dependency,
        }
    }
}

/// The part of the requested color, if any, which changes rasterized pixels.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RasterColorEffect {
    /// Coverage and color pixels do not depend on the scene color.
    Independent,
    /// CoreGraphics' five-level font-smoothing dilation.
    Dilation(u8),
    /// A quantized color consumed by a native preblending or `currentColor` path.
    Preblend(Rgba8),
}

impl Hash for RasterColorEffect {
    fn hash<H: Hasher>(&self, state: &mut H) {
        core::mem::discriminant(self).hash(state);

        match self {
            Self::Independent => {}
            Self::Dilation(dilation) => dilation.hash(state),
            Self::Preblend(color) => <[u8; 4]>::from(*color).hash(state),
        }
    }
}

/// The byte layout supplied by a glyph rasterizer.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum RasterizedGlyphFormat {
    /// One byte of coverage per pixel.
    AlphaMask,
    /// Four bytes per pixel in blue, green, red, unused order.
    BgraSubpixelMask,
    /// Four bytes per pixel in blue, green, red, straight-alpha order.
    BgraColor,
}

/// A glyph raster produced by a platform rasterizer.
#[derive(Clone, Debug)]
pub struct RasterizedGlyph {
    /// Placement relative to the glyph's baseline origin.
    pub bounds: Bounds<DevicePixels>,
    /// Pixel dimensions of the supplied buffer.
    pub size: Size<DevicePixels>,
    /// Byte layout of `pixels`.
    pub format: RasterizedGlyphFormat,
    /// Pixel bytes. Color pixels use straight alpha.
    pub pixels: Vec<u8>,
}

impl RasterizedGlyph {
    /// Returns a successful empty glyph in the requested format.
    pub fn empty(format: RasterizedGlyphFormat) -> Self {
        Self {
            bounds: Bounds::default(),
            size: Size::default(),
            format,
            pixels: Vec::new(),
        }
    }

    /// Checks the buffer dimensions and byte count promised by `format`.
    pub fn validate(&self) -> Result<()> {
        anyhow::ensure!(
            self.bounds.size == self.size,
            "glyph raster bounds size {:?} does not match buffer size {:?}",
            self.bounds.size,
            self.size
        );

        let width: usize = self
            .size
            .width
            .0
            .try_into()
            .map_err(|_| anyhow::anyhow!("glyph raster width is negative"))?;

        let height: usize = self
            .size
            .height
            .0
            .try_into()
            .map_err(|_| anyhow::anyhow!("glyph raster height is negative"))?;

        if width == 0 || height == 0 {
            anyhow::ensure!(
                width == 0 && height == 0 && self.pixels.is_empty(),
                "empty glyph raster must have zero size and no pixels"
            );
            return Ok(());
        }

        let bytes_per_pixel = match self.format {
            RasterizedGlyphFormat::AlphaMask => 1,
            RasterizedGlyphFormat::BgraSubpixelMask | RasterizedGlyphFormat::BgraColor => 4,
        };
        let expected_len = width
            .checked_mul(height)
            .and_then(|pixels| pixels.checked_mul(bytes_per_pixel))
            .ok_or_else(|| anyhow::anyhow!("glyph raster byte count overflow"))?;

        anyhow::ensure!(
            self.pixels.len() == expected_len,
            "glyph raster format {:?} requires {expected_len} bytes for {}x{}, got {}",
            self.format,
            width,
            height,
            self.pixels.len()
        );

        Ok(())
    }
}

/// A glyph raster whose placement and pixel buffer have passed validation.
#[derive(Clone, Debug, Deref)]
pub struct ValidatedRasterizedGlyph(RasterizedGlyph);

impl ValidatedRasterizedGlyph {
    /// Validates a glyph raster before it can be used by an atlas builder.
    pub fn new(glyph: RasterizedGlyph) -> Result<Self> {
        glyph.validate()?;

        Ok(Self(glyph))
    }
}

/// A cached glyph's GPU tile and the placement metadata for its uploaded pixels.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GlyphAtlasEntry {
    /// The GPU tile, or `None` for a successfully empty glyph.
    pub tile: Option<crate::AtlasTile>,
    /// Placement relative to the glyph's baseline origin.
    pub bounds: Bounds<DevicePixels>,
    /// Byte layout of the cached pixels.
    pub format: RasterizedGlyphFormat,
}

impl Eq for RenderGlyphParams {}

impl Hash for RenderGlyphParams {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.font_id.0.hash(state);
        self.glyph_id.0.hash(state);
        self.font_size.0.to_bits().hash(state);
        self.subpixel_variant.hash(state);
        self.scale_factor.to_bits().hash(state);
        self.raster_style.hash(state);
    }
}

/// The face to use for text: a font family plus its weight, width, style, features, and fallbacks.
///
/// Build one with [`font`] and then adjust it with the builder methods, or set
/// the public fields directly. The [`TextSystem`] caches lookups by `Font`, so
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
    /// which differs by platform. Names come from [`TextSystem::all_font_names`].
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
/// Pass a family name from [`TextSystem::all_font_names`] or one you added with
/// [`TextSystem::add_fonts`]. Chain [`Font::bold`] or [`Font::italic`] to change
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

/// The measurements of a typeface, in font units.
///
/// Get the values for a font with [`TextSystem::resolve_font`] and the
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
        (self.bounding_box / self.units_per_em as f32 * font_size.0).map(px)
    }
}

#[cfg(test)]
mod text_range_tests {
    use super::*;

    #[test]
    fn contained_ranges_are_valid_utf8_slices() {
        let text = "aé日";
        #[allow(
            clippy::reversed_empty_ranges,
            reason = "The reversed range is intentional invalid input for contains_range."
        )]
        let cases = [
            ("valid ASCII", 0..1, true),
            ("valid multibyte", 1..3, true),
            ("empty", 1..1, true),
            ("out of bounds", 0..text.len() + 1, false),
            ("reversed", 3..1, false),
            ("split UTF-8 code point", 2..3, false),
        ];

        for (name, range, expected) in cases {
            assert_eq!(text.contains_range(&range), expected, "{name}");
        }
    }
}

#[cfg(test)]
mod layout_cache_tests {
    use super::*;

    fn window_text_system() -> WindowTextSystem {
        WindowTextSystem::new(Arc::new(TextSystem::new(Arc::new(TestTextSystem))))
    }

    #[test]
    fn paint_changes_reuse_shaped_geometry() {
        let system = window_text_system();
        let text = SharedString::from("paint cache");
        let mut run = TextRun {
            len: text.len(),
            color: hsla(0.0, 0.8, 0.4, 1.0),
            ..Default::default()
        };
        let first = system
            .shape_text(text.clone(), px(16.0), &[run.clone()], None, None)
            .unwrap();

        run.color = hsla(0.6, 0.8, 0.4, 1.0);
        run.underline = Some(crate::UnderlineStyle {
            thickness: px(1.5),
            color: None,
            wavy: false,
        });
        let second = system
            .shape_text(text, px(16.0), std::slice::from_ref(&run), None, None)
            .unwrap();

        assert!(Arc::ptr_eq(&first.platform_layout, &second.platform_layout));
        assert_eq!(second.paint_fragments[0].style, PaintStyle::from(&run));
        assert!(second.paint_fragments[0].underline_offset.is_some());
    }

    #[test]
    fn inline_layouts_reuse_all_valid_widths_across_frames() {
        let system = window_text_system();
        let text = "cached inline paragraph";
        let runs = [TextRun {
            len: text.len(),
            ..Default::default()
        }];
        let request = |wrap_width| InlineLayoutRequest {
            text,
            runs: &runs,
            text_styles: &[],
            boxes: &[],
            font_size: px(16.0),
            line_height: px(20.0),
            text_metrics: InlineTextMetrics::default(),
            options: TextLayoutOptions {
                wrap_width: Some(wrap_width),
                text_align: TextAlign::Left,
                ..Default::default()
            },
            bidi_scopes: &[],
        };

        let wide = system.layout_inline(request(px(200.0)));
        let narrow = system.layout_inline(request(px(80.0)));
        assert!(!Arc::ptr_eq(&wide, &narrow));

        let painted_runs = [TextRun {
            color: hsla(0.5, 0.8, 0.4, 1.0),
            ..runs[0].clone()
        }];
        let changed_paint = system.layout_inline(InlineLayoutRequest {
            runs: &painted_runs,
            ..request(px(200.0))
        });
        let changed_size = system.layout_inline(InlineLayoutRequest {
            font_size: px(18.0),
            ..request(px(200.0))
        });
        assert!(!Arc::ptr_eq(&wide, &changed_paint));
        assert!(!Arc::ptr_eq(&wide, &changed_size));
        assert!(Arc::ptr_eq(
            &wide,
            &system.layout_inline(request(px(200.0)))
        ));

        system.finish_frame();
        assert!(Arc::ptr_eq(
            &wide,
            &system.layout_inline(request(px(200.0)))
        ));
        assert!(Arc::ptr_eq(
            &narrow,
            &system.layout_inline(request(px(80.0)))
        ));
    }
}

#[cfg(test)]
mod raster_contract_tests {
    use super::*;

    fn params(style: PreparedRasterStyle) -> RenderGlyphParams {
        RenderGlyphParams {
            font_id: FontId(7),
            glyph_id: GlyphId(42),
            font_size: px(16.0),
            subpixel_variant: point(1, 0),
            scale_factor: 2.0,
            raster_style: style,
        }
    }

    #[test]
    fn raster_buffers_enforce_the_documented_format_contract() {
        let cases = [
            (
                "alpha mask",
                RasterizedGlyph {
                    bounds: Bounds {
                        origin: point(DevicePixels(-1), DevicePixels(-2)),
                        size: size(DevicePixels(2), DevicePixels(1)),
                    },
                    size: size(DevicePixels(2), DevicePixels(1)),
                    format: RasterizedGlyphFormat::AlphaMask,
                    pixels: vec![0, 255],
                },
                true,
            ),
            (
                "subpixel mask",
                RasterizedGlyph {
                    bounds: Bounds {
                        origin: point(DevicePixels(1), DevicePixels(-2)),
                        size: size(DevicePixels(1), DevicePixels(2)),
                    },
                    size: size(DevicePixels(1), DevicePixels(2)),
                    format: RasterizedGlyphFormat::BgraSubpixelMask,
                    pixels: vec![0; 8],
                },
                true,
            ),
            (
                "straight-alpha color",
                RasterizedGlyph {
                    bounds: Bounds {
                        origin: point(DevicePixels(0), DevicePixels(-1)),
                        size: size(DevicePixels(1), DevicePixels(1)),
                    },
                    size: size(DevicePixels(1), DevicePixels(1)),
                    format: RasterizedGlyphFormat::BgraColor,
                    pixels: vec![1, 2, 3, 4],
                },
                true,
            ),
            (
                "empty glyph",
                RasterizedGlyph::empty(RasterizedGlyphFormat::AlphaMask),
                true,
            ),
            (
                "wrong alpha stride",
                RasterizedGlyph {
                    bounds: Bounds {
                        origin: Point::default(),
                        size: size(DevicePixels(2), DevicePixels(1)),
                    },
                    size: size(DevicePixels(2), DevicePixels(1)),
                    format: RasterizedGlyphFormat::AlphaMask,
                    pixels: vec![0],
                },
                false,
            ),
            (
                "wrong BGRA stride",
                RasterizedGlyph {
                    bounds: Bounds {
                        origin: Point::default(),
                        size: size(DevicePixels(1), DevicePixels(1)),
                    },
                    size: size(DevicePixels(1), DevicePixels(1)),
                    format: RasterizedGlyphFormat::BgraColor,
                    pixels: vec![0; 3],
                },
                false,
            ),
            (
                "partially empty dimensions",
                RasterizedGlyph {
                    bounds: Bounds {
                        origin: Point::default(),
                        size: size(DevicePixels(0), DevicePixels(1)),
                    },
                    size: size(DevicePixels(0), DevicePixels(1)),
                    format: RasterizedGlyphFormat::AlphaMask,
                    pixels: Vec::new(),
                },
                false,
            ),
            (
                "negative dimensions",
                RasterizedGlyph {
                    bounds: Bounds {
                        origin: Point::default(),
                        size: size(DevicePixels(-1), DevicePixels(1)),
                    },
                    size: size(DevicePixels(-1), DevicePixels(1)),
                    format: RasterizedGlyphFormat::AlphaMask,
                    pixels: Vec::new(),
                },
                false,
            ),
            (
                "bounds and buffer size disagree",
                RasterizedGlyph {
                    bounds: Bounds {
                        origin: Point::default(),
                        size: size(DevicePixels(2), DevicePixels(1)),
                    },
                    size: size(DevicePixels(1), DevicePixels(1)),
                    format: RasterizedGlyphFormat::AlphaMask,
                    pixels: vec![255],
                },
                false,
            ),
        ];

        for (description, raster, valid) in cases {
            assert_eq!(raster.validate().is_ok(), valid, "{description}");
        }
    }

    #[test]
    fn prepared_style_and_format_drive_atlas_keys_without_caching_pixels() {
        let backend = Arc::new(ScriptedRasterizer::default());
        let text_system = TextSystem::new(backend.clone());
        let style_a = text_system.prepare_raster_style(
            FontId(7),
            GlyphId(42),
            hsla(0.0, 0.0, 0.20, 1.0),
            GlyphRenderMode::Grayscale,
        );
        let style_b = text_system.prepare_raster_style(
            FontId(7),
            GlyphId(42),
            hsla(0.0, 0.0, 0.24, 1.0),
            GlyphRenderMode::Grayscale,
        );
        let style_c = text_system.prepare_raster_style(
            FontId(7),
            GlyphId(42),
            hsla(0.0, 0.0, 0.30, 1.0),
            GlyphRenderMode::Grayscale,
        );
        assert_eq!(style_a, style_b, "nearby scene colors share a mask style");
        assert_ne!(
            style_a, style_c,
            "different dilation levels need distinct rasters"
        );

        let first = params(style_a);
        let second = params(style_b);
        let third = params(style_c);
        assert_eq!(first, second);
        let first_raster = text_system.rasterize_glyph(&first).unwrap();
        let reused_raster = text_system.rasterize_glyph(&second).unwrap();
        let distinct_raster = text_system.rasterize_glyph(&third).unwrap();
        assert_eq!(first_raster.pixels, reused_raster.pixels);
        assert_eq!(first_raster.pixels, distinct_raster.pixels);
        assert_eq!(backend.attempts.load(Ordering::SeqCst), 3);
        for (format, expected) in [
            (
                RasterizedGlyphFormat::AlphaMask,
                AtlasTextureKind::Monochrome,
            ),
            (
                RasterizedGlyphFormat::BgraSubpixelMask,
                AtlasTextureKind::Subpixel,
            ),
            (
                RasterizedGlyphFormat::BgraColor,
                AtlasTextureKind::Polychrome,
            ),
        ] {
            assert_eq!(
                AtlasKey::from((first.clone(), format)).texture_kind(),
                expected
            );
        }
    }

    #[test]
    fn atlas_entries_own_their_raster_metadata_and_retry_failures() {
        let mut malformed = raster(
            Point::default(),
            size(DevicePixels(1), DevicePixels(1)),
            RasterizedGlyphFormat::AlphaMask,
        );
        malformed.pixels.clear();

        let backend = Arc::new(ScriptedRasterizer {
            outcomes: vec![
                Err("transient native failure"),
                Ok(malformed),
                Ok(raster(
                    Point::default(),
                    size(DevicePixels(1), DevicePixels(1)),
                    RasterizedGlyphFormat::AlphaMask,
                )),
            ],
            ..Default::default()
        });
        let text_system = TextSystem::new(backend.clone());
        let params = params(PreparedRasterStyle::independent(GlyphRenderMode::Grayscale));
        let atlas = TestAtlas::new();

        assert!(
            atlas
                .get_or_insert_glyph_with(&params, &mut || { text_system.rasterize_glyph(&params) })
                .is_err()
        );
        assert!(
            atlas
                .get_or_insert_glyph_with(&params, &mut || { text_system.rasterize_glyph(&params) })
                .is_err()
        );
        let first_success = atlas
            .get_or_insert_glyph_with(&params, &mut || text_system.rasterize_glyph(&params))
            .unwrap();
        let second_success = atlas
            .get_or_insert_glyph_with(&params, &mut || text_system.rasterize_glyph(&params))
            .unwrap();

        assert_eq!(first_success, second_success);
        assert_eq!(
            first_success.bounds.size,
            size(DevicePixels(1), DevicePixels(1))
        );
        assert_eq!(first_success.format, RasterizedGlyphFormat::AlphaMask);
        assert!(first_success.tile.is_some());
        assert_eq!(backend.attempts.load(Ordering::SeqCst), 3);

        atlas.clear();
        atlas
            .get_or_insert_glyph_with(&params, &mut || text_system.rasterize_glyph(&params))
            .unwrap();
        assert_eq!(backend.attempts.load(Ordering::SeqCst), 4);
    }

    #[test]
    fn separate_atlases_keep_the_metadata_for_their_own_pixels() {
        let backend = Arc::new(ScriptedRasterizer {
            outcomes: vec![
                Ok(raster(
                    point(DevicePixels(-2), DevicePixels(-7)),
                    size(DevicePixels(2), DevicePixels(3)),
                    RasterizedGlyphFormat::AlphaMask,
                )),
                Ok(raster(
                    point(DevicePixels(1), DevicePixels(-9)),
                    size(DevicePixels(4), DevicePixels(5)),
                    RasterizedGlyphFormat::BgraColor,
                )),
                Ok(raster(
                    point(DevicePixels(3), DevicePixels(-8)),
                    size(DevicePixels(3), DevicePixels(4)),
                    RasterizedGlyphFormat::BgraColor,
                )),
            ],
            ..Default::default()
        });
        let text_system = TextSystem::new(backend.clone());
        let params = params(PreparedRasterStyle::independent(GlyphRenderMode::Color));
        let first_atlas = TestAtlas::new();
        let second_atlas = TestAtlas::new();

        let silhouette = first_atlas
            .get_or_insert_glyph_with(&params, &mut || text_system.rasterize_glyph(&params))
            .unwrap();
        let silhouette_hit = first_atlas
            .get_or_insert_glyph_with(&params, &mut || text_system.rasterize_glyph(&params))
            .unwrap();
        assert_eq!(silhouette_hit, silhouette);
        assert_eq!(backend.attempts.load(Ordering::SeqCst), 1);

        let color = second_atlas
            .get_or_insert_glyph_with(&params, &mut || text_system.rasterize_glyph(&params))
            .unwrap();
        assert_ne!(color.bounds, silhouette.bounds);
        assert_ne!(color.format, silhouette.format);
        assert_eq!(backend.attempts.load(Ordering::SeqCst), 2);

        let original = first_atlas
            .get_or_insert_glyph_with(&params, &mut || text_system.rasterize_glyph(&params))
            .unwrap();
        assert_eq!(original, silhouette);
        assert_eq!(backend.attempts.load(Ordering::SeqCst), 2);

        first_atlas.clear();
        let recovered = first_atlas
            .get_or_insert_glyph_with(&params, &mut || text_system.rasterize_glyph(&params))
            .unwrap();
        assert_ne!(recovered.bounds, silhouette.bounds);
        assert_eq!(recovered.format, RasterizedGlyphFormat::BgraColor);
        assert_eq!(backend.attempts.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn successful_empty_glyphs_are_cached_without_a_tile() {
        let atlas = TestAtlas::new();
        let params = params(PreparedRasterStyle::independent(GlyphRenderMode::Grayscale));
        let mut builds = 0;
        let mut build = || {
            builds += 1;

            ValidatedRasterizedGlyph::new(RasterizedGlyph::empty(RasterizedGlyphFormat::AlphaMask))
        };

        let first = atlas.get_or_insert_glyph_with(&params, &mut build).unwrap();
        let second = atlas.get_or_insert_glyph_with(&params, &mut build).unwrap();
        assert_eq!(first, second);
        assert!(first.tile.is_none());
        assert_eq!(builds, 1);
    }

    #[test]
    fn custom_glyph_builders_validate_before_caching() {
        let atlas = TestAtlas::new();
        let params = params(PreparedRasterStyle::independent(GlyphRenderMode::Grayscale));
        let mut builds = 0;
        let mut build = || {
            builds += 1;
            let size = size(DevicePixels(1), DevicePixels(1));
            let pixels = if builds == 1 { Vec::new() } else { vec![255] };

            ValidatedRasterizedGlyph::new(RasterizedGlyph {
                bounds: Bounds {
                    origin: Point::default(),
                    size,
                },
                size,
                format: RasterizedGlyphFormat::AlphaMask,
                pixels,
            })
        };

        assert!(atlas.get_or_insert_glyph_with(&params, &mut build).is_err());

        let first = atlas.get_or_insert_glyph_with(&params, &mut build).unwrap();
        let second = atlas.get_or_insert_glyph_with(&params, &mut build).unwrap();
        assert_eq!(first, second);
        assert!(first.tile.is_some());
        assert_eq!(builds, 2);
    }

    fn raster(
        origin: Point<DevicePixels>,
        size: Size<DevicePixels>,
        format: RasterizedGlyphFormat,
    ) -> RasterizedGlyph {
        let bytes_per_pixel = if format == RasterizedGlyphFormat::AlphaMask {
            1
        } else {
            4
        };

        RasterizedGlyph {
            bounds: Bounds { origin, size },
            size,
            format,
            pixels: vec![0x7f; size.width.0 as usize * size.height.0 as usize * bytes_per_pixel],
        }
    }

    #[derive(Default)]
    struct ScriptedRasterizer {
        attempts: AtomicUsize,
        outcomes: Vec<Result<RasterizedGlyph, &'static str>>,
    }

    impl PlatformTextSystem for ScriptedRasterizer {
        fn add_fonts(&self, _fonts: Vec<Cow<'static, [u8]>>) -> Result<()> {
            Ok(())
        }

        fn all_font_names(&self) -> Vec<String> {
            Vec::new()
        }

        fn font_id(&self, descriptor: &Font) -> Result<FontId> {
            PlatformTextSystem::font_id(&TestTextSystem, descriptor)
        }

        fn font_metrics(&self, font_id: FontId) -> FontMetrics {
            PlatformTextSystem::font_metrics(&TestTextSystem, font_id)
        }

        fn typographic_bounds(&self, font_id: FontId, glyph_id: GlyphId) -> Result<Bounds<f32>> {
            PlatformTextSystem::typographic_bounds(&TestTextSystem, font_id, glyph_id)
        }

        fn advance(&self, font_id: FontId, glyph_id: GlyphId) -> Result<Size<f32>> {
            PlatformTextSystem::advance(&TestTextSystem, font_id, glyph_id)
        }

        fn glyph_for_char(&self, font_id: FontId, character: char) -> Option<GlyphId> {
            PlatformTextSystem::glyph_for_char(&TestTextSystem, font_id, character)
        }

        fn rasterize_glyph(&self, _params: &RenderGlyphParams) -> Result<RasterizedGlyph> {
            let attempt = self.attempts.fetch_add(1, Ordering::SeqCst);
            let outcome = self.outcomes.get(attempt).or_else(|| self.outcomes.last());

            match outcome {
                Some(Ok(raster)) => Ok(raster.clone()),
                Some(Err(message)) => Err(anyhow!("{message}")),
                None => Ok(raster(
                    Point::default(),
                    size(DevicePixels(1), DevicePixels(1)),
                    RasterizedGlyphFormat::AlphaMask,
                )),
            }
        }

        fn prepare_raster_style(&self, request: RasterStyleRequest) -> PreparedRasterStyle {
            PreparedRasterStyle {
                mode: request.requested_mode,
                color_effect: RasterColorEffect::Dilation(
                    (request.scene_color.red * 4.0).floor() as u8
                ),
                foreground_dependency: request.foreground_dependency,
            }
        }

        fn layout_text(&self, request: TextLayoutRequest<'_>) -> LineLayout {
            PlatformTextSystem::layout_text(&TestTextSystem, request)
        }

        fn layout_inline(&self, request: InlineLayoutRequest<'_>) -> InlineLayout {
            PlatformTextSystem::layout_inline(&TestTextSystem, request)
        }
    }
}
