#[cfg(test)]
use crate::{AtlasKey, AtlasTextureKind, TestTextSystem, hsla, point, size};

#[cfg(test)]
use std::{sync::atomic::AtomicUsize, sync::atomic::Ordering};

use crate::{
    Bounds, DevicePixels, Pixels, PlatformTextSystem, Point, Result, SharedString, Size,
    StrikethroughStyle, TextAlign, TextRenderingMode, UnderlineStyle, px,
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
mod line;
mod line_layout;

pub use font_fallbacks::*;
pub use font_features::*;
pub use line::*;
pub use line_layout::*;

/// An opaque identifier for a specific font.
#[derive(Hash, PartialEq, Eq, Clone, Copy, Debug)]
#[repr(C)]
pub struct FontId(pub usize);

/// Number of subpixel glyph variants along the X axis.
pub const SUBPIXEL_VARIANTS_X: u8 = 4;

/// Number of subpixel glyph variants along the Y axis.
pub const SUBPIXEL_VARIANTS_Y: u8 = 1;

/// The GPUI text rendering sub system.
pub struct TextSystem {
    platform_text_system: Arc<dyn PlatformTextSystem>,
    font_ids_by_font: RwLock<FxHashMap<Font, Result<FontId>>>,
    font_metrics: RwLock<FxHashMap<FontId, FontMetrics>>,
    raster_metadata: RwLock<FxHashMap<RenderGlyphParams, RasterizedGlyphMetadata>>,
}

impl TextSystem {
    /// Create a new TextSystem with the given platform text system.
    pub fn new(platform_text_system: Arc<dyn PlatformTextSystem>) -> Self {
        TextSystem {
            platform_text_system,
            font_metrics: RwLock::default(),
            raster_metadata: RwLock::default(),
            font_ids_by_font: RwLock::default(),
        }
    }

    /// Get a list of all available font names from the operating system.
    pub fn all_font_names(&self) -> Vec<String> {
        self.platform_text_system.all_font_names()
    }

    /// Add a font's data to the text system.
    pub fn add_fonts(&self, fonts: Vec<Cow<'static, [u8]>>) -> Result<()> {
        // Serialize registration with cache misses so an in-flight lookup cannot
        // repopulate a stale miss after the newly registered fonts become available.
        let mut font_ids = self.font_ids_by_font.write();
        let result = self.platform_text_system.add_fonts(fonts);
        font_ids.clear();
        result
    }

    /// Get the FontId for the configure font family and style.
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

    /// Resolves the specified font using backend-owned aliases and fallbacks.
    ///
    /// # Panics
    ///
    /// Panics if the backend cannot resolve the font.
    pub fn resolve_font(&self, font: &Font) -> FontId {
        self.font_id(font)
            .unwrap_or_else(|error| panic!("failed to resolve font '{}': {error}", font.family))
    }

    /// Prewarm any system font caches needed to shape text.
    ///
    /// This may be expensive, so callers should generally invoke it on a
    /// background executor. Missing entries are still populated on demand by
    /// the normal shaping path.
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

    /// Get the bounding box for the given font and font size.
    /// A font's bounding box is the smallest rectangle that could enclose all glyphs
    /// in the font. superimposed over one another.
    pub fn bounding_box(&self, font_id: FontId, font_size: Pixels) -> Bounds<Pixels> {
        self.read_metrics(font_id, |metrics| metrics.bounding_box(font_size))
    }

    /// Get the typographic bounds for the given character, in the given font and size.
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

    /// Get the advance width for the given character, in the given font and size.
    pub fn advance(&self, font_id: FontId, font_size: Pixels, ch: char) -> Result<Size<Pixels>> {
        let glyph_id = self
            .platform_text_system
            .glyph_for_char(font_id, ch)
            .with_context(|| format!("glyph not found for character '{ch}'"))?;
        let result = self.platform_text_system.advance(font_id, glyph_id)?
            / self.units_per_em(font_id) as f32;

        Ok(result * font_size)
    }

    /// Get the number of font size units per 'em square',
    /// Per MDN: "an abstract square whose height is the intended distance between
    /// lines of type in the same type size"
    pub fn units_per_em(&self, font_id: FontId) -> u32 {
        self.read_metrics(font_id, |metrics| metrics.units_per_em)
    }

    /// Get the recommended distance from the baseline for the given font
    pub fn ascent(&self, font_id: FontId, font_size: Pixels) -> Pixels {
        self.read_metrics(font_id, |metrics| metrics.ascent(font_size))
    }

    /// Get the recommended distance below the baseline for the given font,
    /// in single spaced text.
    pub fn descent(&self, font_id: FontId, font_size: Pixels) -> Pixels {
        self.read_metrics(font_id, |metrics| metrics.descent(font_size))
    }

    /// Get the x-height for the given font and font size.
    pub fn x_height(&self, font_id: FontId, font_size: Pixels) -> Pixels {
        self.read_metrics(font_id, |metrics| metrics.x_height(font_size))
    }

    /// Get the recommended baseline offset for the given font and line height.
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

    /// Rasterizes a glyph and records only the metadata needed on later atlas hits.
    pub fn rasterize_glyph(&self, params: &RenderGlyphParams) -> Result<RasterizedGlyph> {
        let glyph = self.platform_text_system.rasterize_glyph(params)?;
        glyph.validate()?;

        let metadata = glyph.metadata();
        let cached = self.raster_metadata.upgradable_read();

        if let Some(previous) = cached.get(params) {
            anyhow::ensure!(
                *previous == metadata,
                "glyph raster metadata changed for the same render parameters"
            );
        } else {
            let mut cached = RwLockUpgradableReadGuard::upgrade(cached);
            cached.insert(params.clone(), metadata);
        }

        Ok(glyph)
    }

    pub(crate) fn raster_metadata(
        &self,
        params: &RenderGlyphParams,
    ) -> Option<RasterizedGlyphMetadata> {
        self.raster_metadata.read().get(params).copied()
    }

    /// Normalizes a requested scene color and render mode into the settings which affect the
    /// cached glyph raster.
    pub(crate) fn prepare_raster_style(
        &self,
        scene_color: Hsla,
        requested_mode: GlyphRenderMode,
    ) -> PreparedRasterStyle {
        self.platform_text_system
            .prepare_raster_style(RasterStyleRequest {
                scene_color: crate::hsla_to_rgba(scene_color),
                requested_mode,
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

/// The GPUI text layout subsystem.
#[derive(Deref)]
pub struct WindowTextSystem {
    line_layout_cache: LineLayoutCache,
    #[deref]
    text_system: Arc<TextSystem>,
}

impl WindowTextSystem {
    /// Create a new WindowTextSystem with the given TextSystem.
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

    /// Shape the given line, at the given font_size, for painting to the screen.
    /// Subsets of the line can be styled independently with the `runs` parameter.
    ///
    /// Note that this method can only shape a single line of text. It will panic
    /// if the text contains newlines. If you need to shape multiple lines of text,
    /// use [`Self::shape_text`] instead.
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

    /// Shape a multi line string of text, at the given font_size, for painting to the screen.
    /// Subsets of the text can be styled independently with the `runs` parameter,
    /// where each run gives the number of UTF-8 bytes that it styles. Runs must cover the
    /// complete string and end on UTF-8 character boundaries.
    ///
    /// If `wrap_width` is provided, the line breaks will be adjusted to fit within the given width.
    ///
    /// The backend receives the complete string. Hard breaks, trailing empty lines, bidi
    /// paragraphs, wrapping, and `line_clamp` therefore share one layout model.
    pub fn shape_text<S: AsRef<str> + Into<SharedString>>(
        &self,
        text: S,
        font_size: Pixels,
        runs: &[TextRun],
        wrap_width: Option<Pixels>,
        line_clamp: Option<usize>,
    ) -> Result<WrappedLine> {
        let text = text.into();
        let layout = self
            .line_layout_cache
            .layout_wrapped_line(&text, font_size, runs, wrap_width, line_clamp);

        Ok(WrappedLine { layout, text })
    }

    /// Layout text and atomic element boxes in one inline formatting context.
    pub fn layout_inline(&self, request: InlineLayoutRequest<'_>) -> InlineLayout {
        self.text_system.platform_text_system.layout_inline(request)
    }

    /// Layout the given line of text, at the given font_size.
    /// Subsets of the line can be styled independently with the `runs` parameter.
    /// Generally, you should prefer to use [`Self::shape_line`] instead, which
    /// can be painted directly.
    pub fn layout_line(&self, text: &str, font_size: Pixels, runs: &[TextRun]) -> Arc<LineLayout> {
        self.line_layout_cache
            .layout_line(&SharedString::new(text), font_size, runs)
    }

    pub(crate) fn finish_frame(&self) {
        self.line_layout_cache.finish_frame()
    }
}

/// The degree of blackness or stroke thickness of a font. This value ranges from 100.0 to 900.0,
/// with 400.0 as normal.
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

/// Allows italic or oblique faces to be selected.
#[derive(Clone, Copy, Eq, PartialEq, Debug, Hash, Default, Serialize, Deserialize, JsonSchema)]
pub enum FontStyle {
    /// A face that is neither italic not obliqued.
    #[default]
    Normal,
    /// A form that is generally cursive in nature.
    Italic,
    /// A typically-sloped version of the regular face.
    Oblique,
}

impl Display for FontStyle {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        Debug::fmt(self, f)
    }
}

/// A styled run of text, for use in [`crate::TextLayout`].
#[derive(Clone, Debug, PartialEq, Default)]
pub struct TextRun {
    /// A number of utf8 bytes
    pub len: usize,
    /// The font to use for this run.
    pub font: Font,
    /// The color
    pub color: Hsla,
    /// The background color (if any)
    pub background_color: Option<Hsla>,
    /// The underline style (if any)
    pub underline: Option<UnderlineStyle>,
    /// The strikethrough style (if any)
    pub strikethrough: Option<StrikethroughStyle>,
    /// Letter spacing applied between glyphs, in pixels.
    pub letter_spacing: Option<Pixels>,
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
    /// Optional soft-wrap width.
    pub wrap_width: Option<Pixels>,
    /// Optional maximum number of visual rows.
    pub line_clamp: Option<usize>,
}

/// An atomic element inserted at a UTF-8 boundary in an inline document.
#[derive(Clone, Copy, Debug, PartialEq)]
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
#[derive(Clone, Debug)]
pub struct InlineTextStyle {
    /// UTF-8 range receiving these resolved metrics.
    pub range: Range<usize>,
    /// Font size for this range.
    pub font_size: Pixels,
    /// Absolute line height for this range.
    pub line_height: Pixels,
}

/// Complete input for a document containing text and element boxes.
#[derive(Clone, Copy, Debug)]
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
    /// Optional soft-wrap width.
    pub wrap_width: Option<Pixels>,
    /// Optional maximum number of visual rows.
    pub line_clamp: Option<usize>,
    /// Horizontal alignment within `wrap_width`.
    pub text_align: TextAlign,
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

/// Parameters for rendering a glyph, used as cache keys for raster bounds.
///
/// This struct identifies a specific glyph rendering configuration including
/// font, size, subpixel positioning, and scale factor. It's used to look up
/// cached raster bounds and sprite atlas entries.
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
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Rgba8 {
    /// Red channel.
    pub red: u8,
    /// Green channel.
    pub green: u8,
    /// Blue channel.
    pub blue: u8,
    /// Alpha channel.
    pub alpha: u8,
}

impl From<crate::Rgba> for Rgba8 {
    fn from(color: crate::Rgba) -> Self {
        fn channel(value: f32) -> u8 {
            (value.clamp(0.0, 1.0) * 255.0).round() as u8
        }

        Self {
            red: channel(color.red),
            green: channel(color.green),
            blue: channel(color.blue),
            alpha: channel(color.alpha),
        }
    }
}

/// A scene request before a platform rasterizer normalizes its cache-relevant settings.
#[derive(Clone, Copy, Debug)]
pub struct RasterStyleRequest {
    /// The exact application color. Rasterizers may only retain a normalized derivative of it.
    pub scene_color: crate::Rgba,
    /// The requested kind of glyph image.
    pub requested_mode: GlyphRenderMode,
}

/// The cache-relevant raster settings chosen by a platform rasterizer.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PreparedRasterStyle {
    /// The normalized render mode.
    pub mode: GlyphRenderMode,
    /// Any normalized color effect baked into coverage or color pixels.
    pub color_effect: RasterColorEffect,
}

impl PreparedRasterStyle {
    /// Creates a color-independent raster style.
    pub const fn independent(mode: GlyphRenderMode) -> Self {
        Self {
            mode,
            color_effect: RasterColorEffect::Independent,
        }
    }
}

/// The part of the requested color, if any, which changes rasterized pixels.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum RasterColorEffect {
    /// Coverage and color pixels do not depend on the scene color.
    Independent,
    /// CoreGraphics' five-level font-smoothing dilation.
    Dilation(u8),
    /// A quantized color consumed by a native preblending or `currentColor` path.
    Preblend(Rgba8),
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

/// A glyph raster ready for insertion into a renderer atlas.
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RasterizedGlyphMetadata {
    pub(crate) bounds: Bounds<DevicePixels>,
    pub(crate) format: RasterizedGlyphFormat,
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

    pub(crate) fn metadata(&self) -> RasterizedGlyphMetadata {
        RasterizedGlyphMetadata {
            bounds: self.bounds,
            format: self.format,
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

/// The configuration details for identifying a specific font.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct Font {
    /// The font family name.
    ///
    /// The special name ".SystemUIFont" is used to identify the system UI font, which varies based on platform.
    pub family: SharedString,

    /// The font features to use.
    pub features: FontFeatures,

    /// The fallbacks fonts to use.
    pub fallbacks: Option<FontFallbacks>,

    /// The font weight.
    pub weight: FontWeight,

    /// The font style.
    pub style: FontStyle,
}

impl Default for Font {
    fn default() -> Self {
        font(".SystemUIFont")
    }
}

/// Get a [`Font`] for a given name.
pub fn font(family: impl Into<SharedString>) -> Font {
    Font {
        family: family.into(),
        features: FontFeatures::default(),
        weight: FontWeight::default(),
        style: FontStyle::default(),
        fallbacks: None,
    }
}

impl Font {
    /// Set this Font to be bold
    pub fn bold(mut self) -> Self {
        self.weight = FontWeight::BOLD;
        self
    }

    /// Set this Font to be italic
    pub fn italic(mut self) -> Self {
        self.style = FontStyle::Italic;
        self
    }
}

/// A struct for storing font metrics.
/// It is used to define the measurements of a typeface.
#[derive(Clone, Copy, Debug)]
pub struct FontMetrics {
    /// The number of font units that make up the "em square",
    /// a scalable grid for determining the size of a typeface.
    pub units_per_em: u32,

    /// The vertical distance from the baseline of the font to the top of the glyph covers.
    pub ascent: f32,

    /// The vertical distance from the baseline of the font to the bottom of the glyph covers.
    pub descent: f32,

    /// The recommended additional space to add between lines of type.
    pub line_gap: f32,

    /// The suggested position of the underline.
    pub underline_position: f32,

    /// The suggested thickness of the underline.
    pub underline_thickness: f32,

    /// The height of a capital letter measured from the baseline of the font.
    pub cap_height: f32,

    /// The height of a lowercase x.
    pub x_height: f32,

    /// The outer limits of the area that the font covers.
    /// Corresponds to the xMin / xMax / yMin / yMax values in the OpenType `head` table
    pub bounding_box: Bounds<f32>,
}

impl FontMetrics {
    /// Returns the vertical distance from the baseline of the font to the top of the glyph covers in pixels.
    pub fn ascent(&self, font_size: Pixels) -> Pixels {
        Pixels((self.ascent / self.units_per_em as f32) * font_size.0)
    }

    /// Returns the vertical distance from the baseline of the font to the bottom of the glyph covers in pixels.
    pub fn descent(&self, font_size: Pixels) -> Pixels {
        Pixels((self.descent / self.units_per_em as f32) * font_size.0)
    }

    /// Returns the recommended additional space to add between lines of type in pixels.
    pub fn line_gap(&self, font_size: Pixels) -> Pixels {
        Pixels((self.line_gap / self.units_per_em as f32) * font_size.0)
    }

    /// Returns the suggested position of the underline in pixels.
    pub fn underline_position(&self, font_size: Pixels) -> Pixels {
        Pixels((self.underline_position / self.units_per_em as f32) * font_size.0)
    }

    /// Returns the suggested thickness of the underline in pixels.
    pub fn underline_thickness(&self, font_size: Pixels) -> Pixels {
        Pixels((self.underline_thickness / self.units_per_em as f32) * font_size.0)
    }

    /// Returns the height of a capital letter measured from the baseline of the font in pixels.
    pub fn cap_height(&self, font_size: Pixels) -> Pixels {
        Pixels((self.cap_height / self.units_per_em as f32) * font_size.0)
    }

    /// Returns the height of a lowercase x in pixels.
    pub fn x_height(&self, font_size: Pixels) -> Pixels {
        Pixels((self.x_height / self.units_per_em as f32) * font_size.0)
    }

    /// Returns the outer limits of the area that the font covers in pixels.
    pub fn bounding_box(&self, font_size: Pixels) -> Bounds<Pixels> {
        (self.bounding_box / self.units_per_em as f32 * font_size.0).map(px)
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
        let backend = Arc::new(SequencedRasterizer::default());
        let text_system = TextSystem::new(backend.clone());
        let style_a =
            text_system.prepare_raster_style(hsla(0.0, 0.0, 0.20, 1.0), GlyphRenderMode::Grayscale);
        let style_b =
            text_system.prepare_raster_style(hsla(0.0, 0.0, 0.24, 1.0), GlyphRenderMode::Grayscale);
        let style_c =
            text_system.prepare_raster_style(hsla(0.0, 0.0, 0.30, 1.0), GlyphRenderMode::Grayscale);
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
        assert_eq!(
            text_system.raster_metadata(&first),
            Some(first_raster.metadata())
        );
        assert_eq!(
            text_system.raster_metadata(&third),
            Some(distinct_raster.metadata())
        );

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
    fn only_successful_valid_native_rasters_record_metadata() {
        let backend = Arc::new(SequencedRasterizer {
            attempts: AtomicUsize::new(0),
            fail_until_valid: true,
        });

        let text_system = TextSystem::new(backend.clone());
        let params = params(PreparedRasterStyle::independent(GlyphRenderMode::Grayscale));

        assert!(text_system.rasterize_glyph(&params).is_err());
        assert!(text_system.raster_metadata(&params).is_none());
        assert!(text_system.rasterize_glyph(&params).is_err());
        assert!(text_system.raster_metadata(&params).is_none());
        let first_success = text_system.rasterize_glyph(&params).unwrap();
        let second_success = text_system.rasterize_glyph(&params).unwrap();

        assert_eq!(first_success.pixels, [0x7f]);
        assert_eq!(second_success.pixels, [0x7f]);
        assert_eq!(
            text_system.raster_metadata(&params),
            Some(first_success.metadata())
        );
        assert_eq!(backend.attempts.load(Ordering::SeqCst), 4);
    }

    #[derive(Default)]
    struct SequencedRasterizer {
        attempts: AtomicUsize,
        fail_until_valid: bool,
    }

    impl PlatformTextSystem for SequencedRasterizer {
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

            if !self.fail_until_valid {
                return Ok(RasterizedGlyph {
                    bounds: Bounds {
                        origin: Point::default(),
                        size: size(DevicePixels(1), DevicePixels(1)),
                    },
                    size: size(DevicePixels(1), DevicePixels(1)),
                    format: RasterizedGlyphFormat::AlphaMask,
                    pixels: vec![0x7f],
                });
            }

            match attempt {
                0 => Err(anyhow!("transient native failure")),
                1 => Ok(RasterizedGlyph {
                    bounds: Bounds {
                        origin: Point::default(),
                        size: size(DevicePixels(1), DevicePixels(1)),
                    },
                    size: size(DevicePixels(1), DevicePixels(1)),
                    format: RasterizedGlyphFormat::AlphaMask,
                    pixels: Vec::new(),
                }),
                _ => Ok(RasterizedGlyph {
                    bounds: Bounds {
                        origin: Point::default(),
                        size: size(DevicePixels(1), DevicePixels(1)),
                    },
                    size: size(DevicePixels(1), DevicePixels(1)),
                    format: RasterizedGlyphFormat::AlphaMask,
                    pixels: vec![0x7f],
                }),
            }
        }

        fn prepare_raster_style(&self, request: RasterStyleRequest) -> PreparedRasterStyle {
            PreparedRasterStyle {
                mode: request.requested_mode,
                color_effect: RasterColorEffect::Dilation(
                    (request.scene_color.red * 4.0).floor() as u8
                ),
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
