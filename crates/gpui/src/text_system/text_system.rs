use crate::{
    Bounds, Font, FontId, FontMetrics, ForegroundDependency, GlyphId, GlyphRenderMode, Pixels,
    PlatformTextSystem, PreparedRasterStyle, RasterStyleRequest, RenderGlyphParams, Result, Size,
    TextRenderingMode, ValidatedRasterizedGlyph, px,
};
use anyhow::{Context as _, anyhow};
use collections::FxHashMap;
use palette::Hsla;
use parking_lot::{RwLock, RwLockUpgradableReadGuard};
use smallvec::SmallVec;
use std::{borrow::Cow, sync::Arc};

/// The platform-independent font database and metrics cache, shared by all windows.
///
/// Get it from [`App::text_system`](crate::App::text_system) and use it to register fonts with
/// [`TextSystem::add_fonts`], to measure glyphs and to resolve [`Font`]s to
/// [`FontId`]s. To shape text for painting, use the [`WindowTextSystem`](crate::WindowTextSystem) of the
/// window, which also caches the layouts.
pub struct TextSystem {
    pub(super) platform_text_system: Arc<dyn PlatformTextSystem>,
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
    /// [`font`](crate::font) or set [`Font::family`] to use them.
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
    /// [`crate::RasterizedGlyph::validate`].
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
