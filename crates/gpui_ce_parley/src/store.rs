#[cfg(test)]
use gpui::{RasterColorEffect, px, rgba};

use anyhow::{Context as _, Result, bail, ensure};
use fontique::{Blob, Synthesis};
use gpui::{
    Bounds, FontId, FontMetrics, ForegroundDependency, GlyphId, GlyphRenderMode,
    PreparedRasterStyle, RasterStyleRequest, RasterizedGlyph, RasterizedGlyphFormat,
    RenderGlyphParams, SUBPIXEL_VARIANTS_X, SUBPIXEL_VARIANTS_Y, Size, TextRenderingMode, point,
    size,
};
use skrifa::{
    FontRef, MetadataProvider as _, Tag,
    bitmap::{BitmapFormat, BitmapStrikes},
    color::{
        Brush as ColorBrush, ColorGlyphFormat, ColorPainter, CompositeMode,
        Transform as ColorTransform,
    },
    instance::{Location, NormalizedCoord, Size as SkrifaSize},
    outline::{DrawSettings, OutlinePen},
    raw::TableProvider as _,
};
use std::collections::{HashMap, hash_map::Entry};
use swash::{
    CacheKey as SwashCacheKey, FontRef as SwashFontRef,
    scale::{Render, ScaleContext, Source, StrikeWith},
    zeno::{Angle, Format, Transform, Vector},
};

const CANONICAL_FONT_ID_BIT: usize = 1 << (usize::BITS - 1);

/// The identity assigned to one immutable font-data blob.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct SourceIdentity(u64);

impl SourceIdentity {
    /// Returns the stable identity carried by a Fontique blob.
    fn of(data: &Blob<u8>) -> Self {
        Self(data.id())
    }
}

/// One design-space variation coordinate for an exact font instance.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FontVariation {
    /// OpenType variation-axis tag.
    pub tag: Tag,
    /// Design-space value consumed by native font APIs.
    pub value: f32,
}

impl FontVariation {
    /// Creates a design-space variation coordinate from an OpenType axis tag.
    pub const fn new(tag: [u8; 4], value: f32) -> Self {
        Self {
            tag: Tag::new(&tag),
            value,
        }
    }
}

/// Synthetic styling attached to an exact font instance.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FontSynthesis {
    /// Whether the rasterizer should synthesize a heavier outline.
    pub embolden: bool,
    /// Synthetic clockwise skew in degrees.
    pub skew_degrees: Option<f32>,
}

impl From<Synthesis> for FontSynthesis {
    fn from(synthesis: Synthesis) -> Self {
        Self {
            embolden: synthesis.embolden(),
            skew_degrees: synthesis.skew(),
        }
    }
}

/// The immutable face and instance selected by Parley for a glyph.
#[derive(Clone, Copy)]
pub struct RasterFace<'a> {
    /// Canonical identity of the full face, variation, and synthesis combination.
    pub font_id: FontId,
    /// Stable identity shared by every face and instance backed by the same source bytes.
    pub source_id: u64,
    /// Owning handle for the original font or collection bytes.
    pub source: &'a Blob<u8>,
    /// Face index within a TTC or OTC collection.
    pub face_index: u32,
    /// Normalized variation coordinates in axis order.
    pub normalized_coords: &'a [NormalizedCoord],
    /// Design-space variation coordinates.
    pub variations: &'a [FontVariation],
    /// Synthetic styling selected by Fontique.
    pub synthesis: FontSynthesis,
    /// Whether the face advertises an OpenType color glyph table.
    pub has_color_glyphs: bool,
}

impl RasterFace<'_> {
    /// Returns the original font or collection bytes.
    pub fn data(&self) -> &[u8] {
        self.source.as_ref()
    }

    /// Returns whether every supplied variation coordinate uses its axis default.
    pub fn has_default_variations(&self) -> Result<bool> {
        let font = FontRef::from_index(self.data(), self.face_index)
            .context("cannot inspect variation axes in the selected face")?;

        Ok(variations_are_default(&font, self.variations))
    }

    fn requires_portable_bitmap_rasterization(&self) -> Result<bool> {
        let font = FontRef::from_index(self.data(), self.face_index)
            .context("cannot inspect color tables in the selected face")?;
        let has_table = |tag| font.table_data(Tag::new(tag)).is_some();

        Ok(has_table(b"CBDT")
            && has_table(b"CBLC")
            && ![b"COLR", b"sbix", b"SVG "].into_iter().any(has_table))
    }

    /// Returns the preferred color artwork format supported by the rasterizer.
    pub fn supported_color_glyph_kind(
        &self,
        glyph_id: GlyphId,
        supports: impl FnMut(ColorGlyphKind) -> bool,
    ) -> Result<Option<ColorGlyphKind>> {
        let font = FontRef::from_index(self.data(), self.face_index)
            .context("cannot inspect color glyph data in the selected face")?;

        Ok(first_supported_color_kind(
            ColorGlyphClassifier::new(font).available_kinds(glyph_id),
            supports,
        ))
    }
}

/// The native artwork format for a color glyph.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ColorGlyphKind {
    /// OpenType COLRv0 layers.
    ColrV0,
    /// An OpenType COLRv1 paint graph.
    ColrV1,
    /// A CBDT bitmap strike.
    Cbdt,
    /// An Apple sbix bitmap strike.
    Sbix,
    /// An SVG document embedded in the font.
    Svg,
}

/// A platform glyph rasterizer used after Parley has selected and shaped an exact face.
pub trait GlyphRasterizer: Send {
    /// Returns whether this rasterizer can preserve the glyph's native color artwork.
    fn supports_color_glyph(&self, _kind: ColorGlyphKind) -> bool {
        true
    }

    /// Reduces a scene request to the settings which alter cached raster pixels.
    fn prepare_style(&self, request: RasterStyleRequest) -> PreparedRasterStyle;

    /// Rasterizes one glyph from the exact face selected during shaping.
    fn rasterize(
        &mut self,
        face: RasterFace<'_>,
        params: &RenderGlyphParams,
    ) -> Result<RasterizedGlyph>;

    /// Returns the platform's default mode for ordinary text.
    fn recommended_mode(&self) -> TextRenderingMode {
        TextRenderingMode::Subpixel
    }
}

/// Synthetic outline changes which affect raster output.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct SynthesisKey {
    embolden: bool,
    skew_bits: Option<u32>,
}

impl From<Synthesis> for SynthesisKey {
    fn from(synthesis: Synthesis) -> Self {
        Self {
            embolden: synthesis.embolden(),
            skew_bits: synthesis.skew().map(f32::to_bits),
        }
    }
}

/// Full identity of a selected font instance.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct FontKey {
    source_identity: SourceIdentity,
    face_index: u32,
    normalized_coords: Vec<NormalizedCoord>,
    synthesis: SynthesisKey,
}

/// A concrete font face and variation instance selected for GPUI.
#[derive(Clone, Debug)]
pub(crate) struct LoadedFont {
    /// Immutable bytes containing the selected face.
    pub(crate) data: Blob<u8>,
    /// Face index within the font collection.
    pub(crate) index: u32,
    /// Normalized variation coordinates in axis order.
    pub(crate) normalized_coords: Vec<NormalizedCoord>,
    /// Design-space coordinates for native font APIs.
    pub(crate) variations: Vec<FontVariation>,
    /// Synthetic styling requested for the selected face.
    pub(crate) synthesis: Synthesis,
    /// Whether this face advertises color glyph data.
    pub(crate) has_color_glyphs: bool,
    source_identity: SourceIdentity,
}

/// Color glyph data parsed once for all glyphs in a shaped run.
pub(crate) struct ColorGlyphClassifier<'a> {
    colr: skrifa::color::ColorGlyphCollection<'a>,
    sbix: Option<skrifa::raw::tables::sbix::Sbix<'a>>,
    cbdt_strikes: Option<skrifa::bitmap::BitmapStrikes<'a>>,
    svg_ranges: Vec<(u32, u32)>,
}

impl ColorGlyphClassifier<'_> {
    fn new(font: FontRef<'_>) -> ColorGlyphClassifier<'_> {
        let svg_ranges = font
            .svg()
            .ok()
            .and_then(|svg| svg.svg_document_list().ok())
            .map(|documents| {
                documents
                    .document_records()
                    .iter()
                    .map(|record| {
                        (
                            record.start_glyph_id().to_u32(),
                            record.end_glyph_id().to_u32(),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        ColorGlyphClassifier {
            colr: font.color_glyphs(),
            sbix: font.sbix().ok(),
            cbdt_strikes: skrifa::bitmap::BitmapStrikes::with_format(
                &font,
                skrifa::bitmap::BitmapFormat::Cbdt,
            ),
            svg_ranges,
        }
    }

    /// Returns every artwork format available for this glyph in preference order.
    pub(crate) fn available_kinds(
        &self,
        glyph_id: GlyphId,
    ) -> impl Iterator<Item = ColorGlyphKind> {
        let skrifa_id = skrifa::GlyphId::new(glyph_id.0);
        let has_colr_v1 = self
            .colr
            .get_with_format(skrifa_id, skrifa::color::ColorGlyphFormat::ColrV1)
            .is_some();
        let has_colr_v0 = self
            .colr
            .get_with_format(skrifa_id, skrifa::color::ColorGlyphFormat::ColrV0)
            .is_some();

        let has_sbix_bitmap = self
            .sbix
            .as_ref()
            .is_some_and(|sbix| sbix_has_glyph(sbix, skrifa_id));
        let has_cbdt_bitmap = self
            .cbdt_strikes
            .as_ref()
            .is_some_and(|strikes| strikes.iter().any(|strike| strike.get(skrifa_id).is_some()));
        let has_svg = self
            .svg_ranges
            .iter()
            .any(|&(start, end)| (start..=end).contains(&glyph_id.0));

        [
            has_colr_v1.then_some(ColorGlyphKind::ColrV1),
            has_colr_v0.then_some(ColorGlyphKind::ColrV0),
            has_sbix_bitmap.then_some(ColorGlyphKind::Sbix),
            has_cbdt_bitmap.then_some(ColorGlyphKind::Cbdt),
            has_svg.then_some(ColorGlyphKind::Svg),
        ]
        .into_iter()
        .flatten()
    }

    fn foreground_dependency(
        &self,
        glyph_id: GlyphId,
        supports: impl FnMut(ColorGlyphKind) -> bool,
    ) -> ForegroundDependency {
        let Some(kind) = first_supported_color_kind(self.available_kinds(glyph_id), supports)
        else {
            return ForegroundDependency::Full;
        };

        match kind {
            ColorGlyphKind::Cbdt | ColorGlyphKind::Sbix => ForegroundDependency::AlphaOnly,
            ColorGlyphKind::ColrV0 if self.colr_v0_has_fixed_palette(glyph_id) => {
                ForegroundDependency::AlphaOnly
            }
            _ => ForegroundDependency::Full,
        }
    }

    fn colr_v0_has_fixed_palette(&self, glyph_id: GlyphId) -> bool {
        let Some(glyph) = self
            .colr
            .get_with_format(skrifa::GlyphId::new(glyph_id.0), ColorGlyphFormat::ColrV0)
        else {
            return false;
        };
        let mut painter = ForegroundTrackingPainter::default();

        glyph.paint(&Location::default(), &mut painter).is_ok() && !painter.uses_foreground
    }
}

#[derive(Default)]
struct ForegroundTrackingPainter {
    uses_foreground: bool,
}

impl ForegroundTrackingPainter {
    fn inspect_brush(&mut self, brush: ColorBrush<'_>) {
        let uses_foreground = match brush {
            ColorBrush::Solid { palette_index, .. } => palette_index == u16::MAX,
            ColorBrush::LinearGradient { color_stops, .. }
            | ColorBrush::RadialGradient { color_stops, .. }
            | ColorBrush::SweepGradient { color_stops, .. } => color_stops
                .iter()
                .any(|stop| stop.palette_index == u16::MAX),
        };
        self.uses_foreground |= uses_foreground;
    }
}

impl ColorPainter for ForegroundTrackingPainter {
    fn push_transform(&mut self, _transform: ColorTransform) {}

    fn pop_transform(&mut self) {}

    fn push_clip_glyph(&mut self, _glyph_id: skrifa::GlyphId) {}

    fn push_clip_box(&mut self, _clip_box: skrifa::raw::types::BoundingBox<f32>) {}

    fn pop_clip(&mut self) {}

    fn fill(&mut self, brush: ColorBrush<'_>) {
        self.inspect_brush(brush);
    }

    fn push_layer(&mut self, _composite_mode: CompositeMode) {}
}

fn sbix_has_glyph(sbix: &skrifa::raw::tables::sbix::Sbix<'_>, glyph_id: skrifa::GlyphId) -> bool {
    (0..sbix.strikes().len()).any(|strike_idx| {
        sbix.strikes()
            .get(strike_idx)
            .ok()
            .and_then(|strike| strike.glyph_data(glyph_id).ok())
            .flatten()
            .is_some()
    })
}

fn first_supported_color_kind(
    kinds: impl IntoIterator<Item = ColorGlyphKind>,
    mut supports: impl FnMut(ColorGlyphKind) -> bool,
) -> Option<ColorGlyphKind> {
    kinds.into_iter().find(|&kind| supports(kind))
}

impl LoadedFont {
    fn skrifa_ref(&self) -> Result<FontRef<'_>> {
        FontRef::from_index(self.data.as_ref(), self.index)
            .context("Skrifa could not parse the stored font face")
    }

    fn location(&self) -> Location {
        let mut location = Location::new(self.normalized_coords.len());
        location
            .coords_mut()
            .copy_from_slice(&self.normalized_coords);
        location
    }

    /// Builds a glyph-level view of the face's color artwork.
    pub(crate) fn color_glyphs(&self) -> Result<ColorGlyphClassifier<'_>> {
        let font = self.skrifa_ref()?;
        Ok(ColorGlyphClassifier::new(font))
    }

    pub(crate) fn foreground_dependency(
        &self,
        glyph_id: GlyphId,
        supports: impl FnMut(ColorGlyphKind) -> bool,
    ) -> Result<ForegroundDependency> {
        Ok(self
            .color_glyphs()?
            .foreground_dependency(glyph_id, supports))
    }

    /// Reads global metrics in font units from the canonical bytes.
    pub(crate) fn metrics(&self) -> Result<FontMetrics> {
        let font = self.skrifa_ref()?;
        let location = self.location();
        let metrics = font.metrics(SkrifaSize::unscaled(), &location);
        Ok(FontMetrics {
            units_per_em: metrics.units_per_em.into(),
            ascent: metrics.ascent,
            descent: -metrics.descent,
            line_gap: metrics.leading,
            underline_position: metrics.underline.map_or(0.0, |underline| underline.offset),
            underline_thickness: metrics
                .underline
                .map_or(0.0, |underline| underline.thickness),
            cap_height: metrics.cap_height.unwrap_or(metrics.ascent),
            x_height: metrics.x_height.unwrap_or(metrics.ascent),
            bounding_box: Bounds {
                origin: point(0.0, 0.0),
                size: size(
                    metrics.max_width.unwrap_or(0.0),
                    metrics.ascent - metrics.descent,
                ),
            },
        })
    }

    /// Maps a Unicode scalar to a nominal glyph using the canonical bytes.
    pub(crate) fn glyph_for_char(&self, character: char) -> Result<Option<GlyphId>> {
        Ok(self
            .skrifa_ref()?
            .charmap()
            .map(character)
            .map(|glyph| GlyphId(glyph.to_u32())))
    }

    /// Returns the unscaled advance for a glyph.
    pub(crate) fn advance(&self, glyph_id: GlyphId) -> Result<Size<f32>> {
        let font = self.skrifa_ref()?;
        let location = self.location();
        let metrics = font.glyph_metrics(SkrifaSize::unscaled(), &location);
        let glyph_id = skrifa::GlyphId::new(glyph_id.0);
        Ok(size(metrics.advance_width(glyph_id).unwrap_or(0.0), 0.0))
    }

    pub(crate) fn raster_face(&self, font_id: FontId) -> RasterFace<'_> {
        RasterFace {
            font_id,
            source_id: self.source_identity.0,
            source: &self.data,
            face_index: self.index,
            normalized_coords: &self.normalized_coords,
            variations: &self.variations,
            synthesis: self.synthesis.into(),
            has_color_glyphs: self.has_color_glyphs,
        }
    }

    pub(crate) fn data_identity(&self) -> u64 {
        self.source_identity.0
    }

    /// Returns the glyph's control bounds in font units.
    pub(crate) fn glyph_bounds(&self, glyph_id: GlyphId) -> Result<Bounds<f32>> {
        let font = self.skrifa_ref()?;
        let location = self.location();
        let bounds = font
            .glyph_metrics(SkrifaSize::unscaled(), &location)
            .bounds(skrifa::GlyphId::new(glyph_id.0));
        let Some(bounds) = bounds else {
            return Ok(Bounds::default());
        };

        Ok(Bounds {
            origin: point(bounds.x_min, bounds.y_min),
            size: size(bounds.x_max - bounds.x_min, bounds.y_max - bounds.y_min),
        })
    }
}

/// Canonical storage for concrete font faces and variation instances.
#[derive(Default)]
pub(crate) struct FontStore {
    fonts: Vec<LoadedFont>,
    ids_by_key: HashMap<FontKey, FontId>,
}

impl FontStore {
    /// Interns a face using Fontique's selected variation and synthesis settings.
    pub(crate) fn intern_synthesized(
        &mut self,
        data: Blob<u8>,
        idx: u32,
        synthesis: Synthesis,
    ) -> Result<FontId> {
        let normalized_coords = {
            let font = FontRef::from_index(data.as_ref(), idx)
                .context("cannot intern a font face Skrifa cannot parse")?;
            font.axes()
                .location(synthesis.variation_settings().iter().copied())
                .coords()
                .to_vec()
        };

        self.intern(data, idx, &normalized_coords, synthesis, &[])
    }

    /// Interns a selected font instance and returns its canonical GPUI ID.
    pub(crate) fn intern(
        &mut self,
        data: Blob<u8>,
        idx: u32,
        normalized_coords: &[NormalizedCoord],
        synthesis: Synthesis,
        shaping_variations: &[FontVariation],
    ) -> Result<FontId> {
        let font = FontRef::from_index(data.as_ref(), idx)
            .context("cannot intern a font face Skrifa cannot parse")?;
        let axis_count = font.axes().len();
        ensure!(
            normalized_coords.len() <= axis_count,
            "shaped location has more coordinates than the selected face"
        );
        let mut canonical_coords = normalized_coords.to_vec();
        canonical_coords.resize(axis_count, NormalizedCoord::default());
        let variations =
            verified_design_variations(&font, &canonical_coords, synthesis, shaping_variations)?;
        let key = FontKey {
            source_identity: SourceIdentity::of(&data),
            face_index: idx,
            normalized_coords: canonical_coords.clone(),
            synthesis: synthesis.into(),
        };

        if let Some(font_id) = self.ids_by_key.get(&key) {
            return Ok(*font_id);
        }

        if self.fonts.len() >= CANONICAL_FONT_ID_BIT {
            bail!("canonical font store exhausted its FontId namespace");
        }

        let font_id = FontId(CANONICAL_FONT_ID_BIT | self.fonts.len());
        let has_color_glyphs = [*b"CBDT", *b"sbix", *b"COLR", *b"SVG "]
            .into_iter()
            .any(|tag| font.table_data(Tag::new(&tag)).is_some());
        let source_identity = SourceIdentity::of(&data);
        self.fonts.push(LoadedFont {
            data,
            index: idx,
            normalized_coords: canonical_coords,
            variations,
            synthesis,
            has_color_glyphs,
            source_identity,
        });

        self.ids_by_key.insert(key, font_id);
        Ok(font_id)
    }

    /// Returns the stored font for a canonical ID.
    pub(crate) fn get(&self, font_id: FontId) -> Option<&LoadedFont> {
        canonical_index(font_id).and_then(|idx| self.fonts.get(idx))
    }
}

/// Rebuilds the design-space settings used by shaping, then verifies their normalized location.
fn verified_design_variations(
    font: &FontRef<'_>,
    normalized_coords: &[NormalizedCoord],
    synthesis: Synthesis,
    shaping_variations: &[FontVariation],
) -> Result<Vec<FontVariation>> {
    let axes = font.axes();
    let axis_records = axes.iter().collect::<Vec<_>>();
    let mut variations = axis_records
        .iter()
        .map(|axis| FontVariation {
            tag: axis.tag(),
            value: axis.default_value(),
        })
        .collect::<Vec<_>>();

    let mut apply = |tag: Tag, value: f32| {
        let Some(axis) = axis_records.iter().find(|axis| axis.tag() == tag) else {
            return;
        };

        variations[axis.index()].value = value.clamp(axis.min_value(), axis.max_value());
    };

    for &(tag, value) in synthesis.variation_settings() {
        apply(tag, value);
    }

    for variation in shaping_variations {
        apply(variation.tag, variation.value);
    }

    let location = axes.location(
        variations
            .iter()
            .map(|variation| (variation.tag, variation.value)),
    );
    ensure!(
        location.coords() == normalized_coords,
        "native font coordinates differ from the shaped instance: design {variations:?}, native {:?}, shaped {normalized_coords:?}",
        location.coords()
    );

    Ok(variations)
}

fn variations_are_default(font: &FontRef<'_>, variations: &[FontVariation]) -> bool {
    let axes = font.axes();

    variations.iter().all(|variation| {
        axes.iter()
            .any(|axis| axis.tag() == variation.tag && axis.default_value() == variation.value)
    })
}

fn canonical_index(font_id: FontId) -> Option<usize> {
    (font_id.0 & CANONICAL_FONT_ID_BIT != 0).then_some(font_id.0 & !CANONICAL_FONT_ID_BIT)
}

/// Swash raster state used by Linux, web, and explicit fallback construction.
pub struct SwashGlyphRasterizer {
    scale_context: ScaleContext,
    cache_keys: HashMap<FontId, SwashCacheKey>,
}

impl Default for SwashGlyphRasterizer {
    fn default() -> Self {
        Self {
            scale_context: ScaleContext::new(),
            cache_keys: HashMap::default(),
        }
    }
}

impl GlyphRasterizer for SwashGlyphRasterizer {
    fn supports_color_glyph(&self, kind: ColorGlyphKind) -> bool {
        matches!(
            kind,
            ColorGlyphKind::ColrV0 | ColorGlyphKind::Cbdt | ColorGlyphKind::Sbix
        )
    }

    fn prepare_style(&self, request: RasterStyleRequest) -> PreparedRasterStyle {
        if request.requested_mode == GlyphRenderMode::Color {
            PreparedRasterStyle::preblend(request)
        } else {
            PreparedRasterStyle::independent(request.requested_mode)
        }
    }

    fn rasterize(
        &mut self,
        face: RasterFace<'_>,
        params: &RenderGlyphParams,
    ) -> Result<RasterizedGlyph> {
        let format = params.raster_style.mode.rasterized_format();
        let rendered = self.render_glyph_image(&face, params)?;
        let Some(mut image) = rendered else {
            if glyph_is_intentionally_empty(&face, params.glyph_id)? {
                return Ok(RasterizedGlyph::empty(format));
            }

            bail!("unable to rasterize glyph {:?}", params.glyph_id);
        };

        if image.placement.width == 0 || image.placement.height == 0 {
            return Ok(RasterizedGlyph::empty(format));
        }

        let bounds = Bounds {
            origin: point(image.placement.left.into(), (-image.placement.top).into()),
            size: size(image.placement.width.into(), image.placement.height.into()),
        };

        let (format, pixels) = match image.content {
            swash::scale::image::Content::Color => {
                let premultiplied = matches!(image.source, Source::ColorOutline(_));
                for pixel in image.data.chunks_exact_mut(4) {
                    if premultiplied {
                        gpui::swap_rgba_pa_to_bgra(pixel);
                    } else {
                        pixel.swap(0, 2);
                    }
                }

                (RasterizedGlyphFormat::BgraColor, image.data)
            }
            swash::scale::image::Content::SubpixelMask => {
                convert_subpixel_mask_to_bgra(&mut image.data);

                (RasterizedGlyphFormat::BgraSubpixelMask, image.data)
            }
            swash::scale::image::Content::Mask
                if params.raster_style.mode == GlyphRenderMode::Subpixel =>
            {
                (
                    RasterizedGlyphFormat::BgraSubpixelMask,
                    image
                        .data
                        .iter()
                        .flat_map(|&alpha| [alpha, alpha, alpha, 0])
                        .collect(),
                )
            }
            swash::scale::image::Content::Mask
                if params.raster_style.mode == GlyphRenderMode::Color =>
            {
                if params.raster_style.foreground_dependency == ForegroundDependency::AlphaOnly {
                    return Ok(RasterizedGlyph {
                        bounds,
                        size: bounds.size,
                        format: RasterizedGlyphFormat::AlphaMask,
                        pixels: image.data,
                    });
                }

                let color = match params.raster_style.color_effect {
                    gpui::RasterColorEffect::Preblend(color) => color,
                    gpui::RasterColorEffect::Independent => gpui::Rgba8 {
                        red: 0,
                        green: 0,
                        blue: 0,
                        alpha: 255,
                    },
                    gpui::RasterColorEffect::Dilation(_) => {
                        bail!("color glyph rasterization cannot use a dilation style")
                    }
                };

                let pixels = image
                    .data
                    .into_iter()
                    .flat_map(|coverage| {
                        let alpha =
                            ((u16::from(coverage) * u16::from(color.alpha) + 127) / 255) as u8;
                        [color.blue, color.green, color.red, alpha]
                    })
                    .collect();
                (RasterizedGlyphFormat::BgraColor, pixels)
            }
            swash::scale::image::Content::Mask => (RasterizedGlyphFormat::AlphaMask, image.data),
        };

        Ok(RasterizedGlyph {
            bounds,
            size: bounds.size,
            format,
            pixels,
        })
    }
}

/// Uses Swash for CBDT-only faces and a platform rasterizer for every other face.
pub struct BitmapFallbackGlyphRasterizer<Native> {
    native: Native,
    portable: SwashGlyphRasterizer,
    portable_faces: HashMap<FontId, bool>,
}

impl<Native> BitmapFallbackGlyphRasterizer<Native> {
    /// Adds portable CBDT support to a platform glyph rasterizer.
    pub fn new(native: Native) -> Self {
        Self {
            native,
            portable: SwashGlyphRasterizer::default(),
            portable_faces: HashMap::default(),
        }
    }
}

impl<Native> GlyphRasterizer for BitmapFallbackGlyphRasterizer<Native>
where
    Native: GlyphRasterizer,
{
    fn supports_color_glyph(&self, kind: ColorGlyphKind) -> bool {
        kind == ColorGlyphKind::Cbdt || self.native.supports_color_glyph(kind)
    }

    fn prepare_style(&self, request: RasterStyleRequest) -> PreparedRasterStyle {
        self.native.prepare_style(request)
    }

    fn rasterize(
        &mut self,
        face: RasterFace<'_>,
        params: &RenderGlyphParams,
    ) -> Result<RasterizedGlyph> {
        let use_portable = match self.portable_faces.entry(face.font_id) {
            Entry::Occupied(entry) => *entry.get(),
            Entry::Vacant(entry) => *entry.insert(face.requires_portable_bitmap_rasterization()?),
        };

        if use_portable {
            return self.portable.rasterize(face, params);
        }

        self.native.rasterize(face, params)
    }

    fn recommended_mode(&self) -> TextRenderingMode {
        self.native.recommended_mode()
    }
}

#[derive(Default)]
struct DrawablePathPen {
    has_segments: bool,
}

impl OutlinePen for DrawablePathPen {
    fn move_to(&mut self, _x: f32, _y: f32) {}

    fn line_to(&mut self, _x: f32, _y: f32) {
        self.has_segments = true;
    }

    fn quad_to(&mut self, _control_x: f32, _control_y: f32, _x: f32, _y: f32) {
        self.has_segments = true;
    }

    fn curve_to(
        &mut self,
        _control_x0: f32,
        _control_y0: f32,
        _control_x1: f32,
        _control_y1: f32,
        _x: f32,
        _y: f32,
    ) {
        self.has_segments = true;
    }

    fn close(&mut self) {}
}

fn glyph_is_intentionally_empty(face: &RasterFace<'_>, glyph_id: GlyphId) -> Result<bool> {
    let font = FontRef::from_index(face.data(), face.face_index)
        .context("cannot inspect a glyph in the selected face")?;
    let glyph_count = u32::from(
        font.maxp()
            .context("cannot read the selected face's glyph count")?
            .num_glyphs(),
    );
    ensure!(
        glyph_id.0 < glyph_count,
        "glyph ID {} is outside the selected face's {glyph_count} glyphs",
        glyph_id.0
    );

    let skrifa_id = skrifa::GlyphId::new(glyph_id.0);
    let outlines = font.outline_glyphs();
    if outlines.format().is_some() {
        let outline = outlines
            .get(skrifa_id)
            .context("cannot parse the selected glyph's outline")?;
        let mut location = Location::new(face.normalized_coords.len());
        location
            .coords_mut()
            .copy_from_slice(face.normalized_coords);
        let mut pen = DrawablePathPen::default();
        outline
            .draw(
                DrawSettings::unhinted(SkrifaSize::unscaled(), &location),
                &mut pen,
            )
            .context("cannot inspect the selected glyph's outline")?;

        if pen.has_segments {
            return Ok(false);
        }
    }

    if ColorGlyphClassifier::new(font.clone())
        .available_kinds(glyph_id)
        .next()
        .is_some()
    {
        return Ok(false);
    }

    for format in [BitmapFormat::Sbix, BitmapFormat::Cbdt, BitmapFormat::Ebdt] {
        let has_tables = match format {
            BitmapFormat::Sbix => font.table_data(Tag::new(b"sbix")).is_some(),
            BitmapFormat::Cbdt => {
                font.table_data(Tag::new(b"CBDT")).is_some()
                    || font.table_data(Tag::new(b"CBLC")).is_some()
            }
            BitmapFormat::Ebdt => {
                font.table_data(Tag::new(b"EBDT")).is_some()
                    || font.table_data(Tag::new(b"EBLC")).is_some()
            }
        };
        let Some(strikes) = BitmapStrikes::with_format(&font, format) else {
            ensure!(
                !has_tables,
                "cannot parse the selected face's bitmap tables"
            );

            continue;
        };

        if strikes.iter().any(|strike| {
            strike
                .get(skrifa_id)
                .is_some_and(|bitmap| bitmap.width != 0 && bitmap.height != 0)
        }) {
            return Ok(false);
        }
    }

    Ok(true)
}

impl SwashGlyphRasterizer {
    fn render_glyph_image(
        &mut self,
        face: &RasterFace<'_>,
        params: &RenderGlyphParams,
    ) -> Result<Option<swash::scale::image::Image>> {
        ensure!(
            params.scale_factor.is_finite() && params.scale_factor > 0.0,
            "invalid raster scale factor"
        );
        let cache_key = *self
            .cache_keys
            .entry(face.font_id)
            .or_insert_with(SwashCacheKey::new);
        let mut font_ref = SwashFontRef::from_index(face.data(), face.face_index as usize)
            .context("Swash could not parse the stored font face")?;
        font_ref.key = cache_key;
        let subpixel_offset = subpixel_offset(params);
        let normalized_coords = face
            .normalized_coords
            .iter()
            .map(|coordinate| coordinate.to_bits())
            .collect::<Vec<_>>();
        let mut scaler = self
            .scale_context
            .builder(font_ref)
            .size(f32::from(params.font_size) * params.scale_factor)
            .normalized_coords(&normalized_coords)
            .hint(true)
            .build();
        let sources: &[Source] = if params.raster_style.mode == GlyphRenderMode::Color {
            &[
                Source::ColorOutline(0),
                Source::ColorBitmap(StrikeWith::BestFit),
                Source::Outline,
            ]
        } else {
            &[Source::Bitmap(StrikeWith::ExactSize), Source::Outline]
        };

        let mut renderer = Render::new(sources);

        if params.raster_style.mode == GlyphRenderMode::Subpixel {
            renderer.format(Format::subpixel_bgra());
        } else {
            renderer.format(Format::Alpha);
        }

        if let gpui::RasterColorEffect::Preblend(color) = params.raster_style.color_effect {
            renderer.default_color([color.red, color.green, color.blue, color.alpha]);
        }

        renderer.offset(subpixel_offset);

        if face.synthesis.embolden {
            renderer.embolden(f32::from(params.font_size) * params.scale_factor / 48.0);
        }

        if let Some(degrees) = face.synthesis.skew_degrees {
            renderer.transform(Some(Transform::skew(
                Angle::from_degrees(degrees),
                Angle::ZERO,
            )));
        }

        let glyph_id: u16 = params.glyph_id.0.try_into()?;
        Ok(renderer.render(&mut scaler, glyph_id))
    }
}

fn subpixel_offset(params: &RenderGlyphParams) -> Vector {
    Vector::new(
        params.subpixel_variant.x as f32 / SUBPIXEL_VARIANTS_X as f32 / params.scale_factor,
        params.subpixel_variant.y as f32 / SUBPIXEL_VARIANTS_Y as f32 / params.scale_factor,
    )
}

fn convert_subpixel_mask_to_bgra(pixels: &mut [u8]) {
    for pixel in pixels.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOURCE_SERIF: &[u8] =
        include_bytes!("../../../assets/fonts/source-serif-4/SourceSerif4[opsz,wght].ttf");
    const NOTO_COLOR_EMOJI: &[u8] =
        include_bytes!("../../../assets/fonts/noto-color-emoji/NotoColorEmoji.subset.ttf");

    #[derive(Default)]
    struct RecordingNativeRasterizer {
        raster_calls: usize,
    }

    impl GlyphRasterizer for RecordingNativeRasterizer {
        fn supports_color_glyph(&self, kind: ColorGlyphKind) -> bool {
            kind == ColorGlyphKind::ColrV0
        }

        fn prepare_style(&self, request: RasterStyleRequest) -> PreparedRasterStyle {
            PreparedRasterStyle::independent(request.requested_mode)
        }

        fn rasterize(
            &mut self,
            _face: RasterFace<'_>,
            params: &RenderGlyphParams,
        ) -> Result<RasterizedGlyph> {
            self.raster_calls += 1;

            Ok(RasterizedGlyph::empty(
                params.raster_style.mode.rasterized_format(),
            ))
        }
    }

    #[test]
    fn bitmap_fallback_routes_complete_cbdt_faces_and_preserves_native_faces() {
        let cbdt_source = Blob::from(NOTO_COLOR_EMOJI.to_vec());
        let cbdt_font = FontRef::new(cbdt_source.as_ref()).unwrap();
        let cbdt_face = RasterFace {
            font_id: FontId(1),
            source_id: cbdt_source.id(),
            source: &cbdt_source,
            face_index: 0,
            normalized_coords: &[],
            variations: &[],
            synthesis: FontSynthesis::default(),
            has_color_glyphs: true,
        };
        let mut rasterizer =
            BitmapFallbackGlyphRasterizer::new(RecordingNativeRasterizer::default());

        assert!(rasterizer.supports_color_glyph(ColorGlyphKind::Cbdt));
        assert!(rasterizer.supports_color_glyph(ColorGlyphKind::ColrV0));
        assert!(!rasterizer.supports_color_glyph(ColorGlyphKind::Sbix));

        let emoji = GlyphId(cbdt_font.charmap().map('😀').unwrap().to_u32());
        assert_eq!(
            cbdt_face
                .supported_color_glyph_kind(emoji, |kind| { rasterizer.supports_color_glyph(kind) })
                .unwrap(),
            Some(ColorGlyphKind::Cbdt)
        );
        let params = |glyph_id| RenderGlyphParams {
            font_id: cbdt_face.font_id,
            glyph_id,
            font_size: px(24.0),
            subpixel_variant: point(0, 0),
            scale_factor: 1.0,
            raster_style: PreparedRasterStyle::independent(GlyphRenderMode::Color),
        };
        let rendered = rasterizer.rasterize(cbdt_face, &params(emoji)).unwrap();
        assert!(rendered.pixels.chunks_exact(4).any(|pixel| pixel[3] != 0));

        let space = GlyphId(cbdt_font.charmap().map(' ').unwrap().to_u32());
        let empty = rasterizer.rasterize(cbdt_face, &params(space)).unwrap();
        assert_eq!(empty.size, Size::default());
        assert!(empty.pixels.is_empty());
        assert_eq!(rasterizer.native.raster_calls, 0);

        let native_source = Blob::from(SOURCE_SERIF.to_vec());
        let native_font = FontRef::new(native_source.as_ref()).unwrap();
        let normalized_coords = native_font
            .axes()
            .location(std::iter::empty::<(Tag, f32)>())
            .coords()
            .to_vec();
        let native_face = RasterFace {
            font_id: FontId(2),
            source_id: native_source.id(),
            source: &native_source,
            face_index: 0,
            normalized_coords: &normalized_coords,
            variations: &[],
            synthesis: FontSynthesis::default(),
            has_color_glyphs: false,
        };
        let letter = GlyphId(native_font.charmap().map('A').unwrap().to_u32());
        let native_params = RenderGlyphParams {
            font_id: native_face.font_id,
            glyph_id: letter,
            font_size: px(24.0),
            subpixel_variant: point(0, 0),
            scale_factor: 1.0,
            raster_style: PreparedRasterStyle::independent(GlyphRenderMode::Grayscale),
        };
        rasterizer.rasterize(native_face, &native_params).unwrap();
        assert_eq!(rasterizer.native.raster_calls, 1);
    }

    #[test]
    fn original_design_variations_match_the_shaped_location() {
        let font = FontRef::new(SOURCE_SERIF).unwrap();
        let axes = font.axes();
        let cases = [
            Vec::new(),
            vec![FontVariation::new(*b"wght", 340.0)],
            vec![
                FontVariation::new(*b"wght", 340.0),
                FontVariation::new(*b"opsz", 48.0),
            ],
        ];

        for settings in cases {
            let target = axes.location(
                settings
                    .iter()
                    .map(|variation| (variation.tag, variation.value)),
            );
            let variations =
                verified_design_variations(&font, target.coords(), Synthesis::default(), &settings)
                    .unwrap();
            let actual = axes.location(
                variations
                    .iter()
                    .map(|variation| (variation.tag, variation.value)),
            );
            assert_eq!(actual.coords(), target.coords());

            for setting in settings {
                assert_eq!(
                    variations
                        .iter()
                        .find(|variation| variation.tag == setting.tag)
                        .map(|variation| variation.value),
                    Some(setting.value)
                );
            }
        }
    }

    #[test]
    fn steep_avar_fixture_preserves_the_original_design_coordinate() {
        let mut data = SOURCE_SERIF.to_vec();
        let table_count = u16::from_be_bytes(data[4..6].try_into().unwrap()) as usize;
        let avar_offset = (0..table_count)
            .find_map(|table_idx| {
                let record = 12 + table_idx * 16;
                (&data[record..record + 4] == b"avar").then(|| {
                    u32::from_be_bytes(data[record + 8..record + 12].try_into().unwrap()) as usize
                })
            })
            .unwrap();
        assert_eq!(
            &data[avar_offset + 14..avar_offset + 18],
            &[0xe0, 0x00, 0xd7, 0x8e]
        );
        data[avar_offset + 16..avar_offset + 18].copy_from_slice(&(-15_565i16).to_be_bytes());

        let font = FontRef::new(&data).unwrap();
        let settings = [FontVariation::new(*b"wght", 340.0)];
        let shaped = font.axes().location([(Tag::new(b"wght"), 340.0)]);
        let variations =
            verified_design_variations(&font, shaped.coords(), Synthesis::default(), &settings)
                .unwrap();

        assert_eq!(
            variations
                .iter()
                .find(|variation| variation.tag == Tag::new(b"wght"))
                .unwrap()
                .value,
            340.0
        );
        assert_eq!(
            font.axes()
                .location(
                    variations
                        .iter()
                        .map(|variation| (variation.tag, variation.value))
                )
                .coords(),
            shaped.coords()
        );
    }

    #[test]
    fn mismatched_design_variations_are_rejected() {
        let font = FontRef::new(SOURCE_SERIF).unwrap();
        let target = font.axes().location([(Tag::new(b"wght"), 340.0)]);

        assert!(
            verified_design_variations(
                &font,
                target.coords(),
                Synthesis::default(),
                &[FontVariation::new(*b"wght", 341.0)],
            )
            .is_err()
        );
    }

    #[test]
    fn interning_deduplicates_only_equivalent_font_instances() {
        let data = Blob::from(SOURCE_SERIF.to_vec());
        let mut store = FontStore::default();
        let first = store
            .intern_synthesized(data.clone(), 0, Synthesis::default())
            .unwrap();
        let duplicate = store
            .intern_synthesized(data.clone(), 0, Synthesis::default())
            .unwrap();
        assert_eq!(first, duplicate);

        let font = FontRef::new(data.as_ref()).unwrap();
        let explicit_defaults = vec![NormalizedCoord::default(); font.axes().len()];
        let equivalent = store
            .intern(
                data.clone(),
                0,
                &explicit_defaults,
                Synthesis::default(),
                &[],
            )
            .unwrap();
        assert_eq!(first, equivalent);
        assert!(
            store
                .intern(
                    data.clone(),
                    0,
                    &explicit_defaults,
                    Synthesis::default(),
                    &[FontVariation::new(*b"wght", 340.0)],
                )
                .is_err()
        );

        let varied_settings = [FontVariation::new(*b"wght", 700.0)];
        let varied_location = font.axes().location(
            varied_settings
                .iter()
                .map(|variation| (variation.tag, variation.value)),
        );
        let varied = store
            .intern(
                data.clone(),
                0,
                varied_location.coords(),
                Synthesis::default(),
                &varied_settings,
            )
            .unwrap();
        assert_ne!(first, varied);

        let copied_source = store
            .intern_synthesized(Blob::from(SOURCE_SERIF.to_vec()), 0, Synthesis::default())
            .unwrap();
        assert_ne!(first, copied_source);
        assert!(store.get(first).is_some());
        assert!(store.get(FontId(0)).is_none());
    }

    #[test]
    fn portable_rasterization_preserves_current_color_and_legacy_subpixel_offsets() {
        let rasterizer = SwashGlyphRasterizer::default();
        let style = rasterizer.prepare_style(RasterStyleRequest {
            font_id: FontId(1),
            glyph_id: GlyphId(1),
            scene_color: rgba(0xe02010cc),
            requested_mode: GlyphRenderMode::Color,
            foreground_dependency: ForegroundDependency::Full,
        });

        assert_eq!(style.mode, GlyphRenderMode::Color);
        assert_eq!(
            style.color_effect,
            RasterColorEffect::Preblend(rgba(0xe02010cc).into())
        );

        for scale_factor in [1.0, 2.0] {
            let params = RenderGlyphParams {
                font_id: FontId(1),
                glyph_id: GlyphId(1),
                font_size: px(16.0),
                subpixel_variant: point(SUBPIXEL_VARIANTS_X - 1, 0),
                scale_factor,
                raster_style: style,
            };

            assert_eq!(
                subpixel_offset(&params),
                Vector::new(0.75 / scale_factor, 0.0)
            );
        }
    }

    #[test]
    fn portable_rasterization_uses_parleys_normalized_coordinates() {
        let source = Blob::from(SOURCE_SERIF.to_vec());
        let font = FontRef::new(source.as_ref()).unwrap();
        let default_coords = vec![NormalizedCoord::default(); font.axes().len()];
        let mut optical_coords = default_coords.clone();
        let optical_idx = font
            .axes()
            .iter()
            .position(|axis| axis.tag() == Tag::new(b"opsz"))
            .unwrap();
        optical_coords[optical_idx] =
            font.axes().location([(Tag::new(b"opsz"), 72.0)]).coords()[optical_idx];
        let variations =
            verified_design_variations(&font, &default_coords, Synthesis::default(), &[]).unwrap();
        let default_face = RasterFace {
            font_id: FontId(1),
            source_id: source.id(),
            source: &source,
            face_index: 0,
            normalized_coords: &default_coords,
            variations: &variations,
            synthesis: FontSynthesis::default(),
            has_color_glyphs: false,
        };
        let optical_face = RasterFace {
            font_id: FontId(2),
            normalized_coords: &optical_coords,
            ..default_face
        };
        let glyph_id = GlyphId(font.charmap().map('A').unwrap().to_u32());
        let params = RenderGlyphParams {
            font_id: default_face.font_id,
            glyph_id,
            font_size: px(48.0),
            subpixel_variant: point(0, 0),
            scale_factor: 1.0,
            raster_style: PreparedRasterStyle::independent(GlyphRenderMode::Grayscale),
        };
        let mut rasterizer = SwashGlyphRasterizer::default();
        let default = rasterizer.rasterize(default_face, &params).unwrap();
        let optical = rasterizer
            .rasterize(
                optical_face,
                &RenderGlyphParams {
                    font_id: optical_face.font_id,
                    ..params
                },
            )
            .unwrap();

        assert_ne!(optical.pixels, default.pixels);
    }

    #[test]
    fn portable_color_outline_fallback_returns_a_tintable_mask() {
        let source = Blob::from(SOURCE_SERIF.to_vec());
        let font = FontRef::new(source.as_ref()).unwrap();
        let normalized_coords = font
            .axes()
            .location(std::iter::empty::<(Tag, f32)>())
            .coords()
            .to_vec();
        let variations =
            verified_design_variations(&font, &normalized_coords, Synthesis::default(), &[])
                .unwrap();
        let face = RasterFace {
            font_id: FontId(1),
            source_id: source.id(),
            source: &source,
            face_index: 0,
            normalized_coords: &normalized_coords,
            variations: &variations,
            synthesis: FontSynthesis::default(),
            has_color_glyphs: false,
        };
        let glyph_id = GlyphId(font.charmap().map('A').unwrap().to_u32());
        let raster_style = PreparedRasterStyle::preblend(RasterStyleRequest {
            font_id: face.font_id,
            glyph_id,
            scene_color: rgba(0xe02010cc),
            requested_mode: GlyphRenderMode::Color,
            foreground_dependency: ForegroundDependency::AlphaOnly,
        });
        let params = RenderGlyphParams {
            font_id: face.font_id,
            glyph_id,
            font_size: px(24.0),
            subpixel_variant: point(0, 0),
            scale_factor: 1.0,
            raster_style,
        };

        let raster = SwashGlyphRasterizer::default()
            .rasterize(face, &params)
            .unwrap();
        assert_eq!(raster.format, RasterizedGlyphFormat::AlphaMask);
        assert!(raster.pixels.iter().any(|coverage| *coverage != 0));
    }

    #[test]
    fn portable_rasterization_separates_blank_glyphs_from_failures() {
        let source = Blob::from(SOURCE_SERIF.to_vec());
        let font = FontRef::new(source.as_ref()).unwrap();
        let normalized_coords = font
            .axes()
            .location(std::iter::empty::<(Tag, f32)>())
            .coords()
            .to_vec();
        let variations =
            verified_design_variations(&font, &normalized_coords, Synthesis::default(), &[])
                .unwrap();
        let face = RasterFace {
            font_id: FontId(1),
            source_id: source.id(),
            source: &source,
            face_index: 0,
            normalized_coords: &normalized_coords,
            variations: &variations,
            synthesis: FontSynthesis::default(),
            has_color_glyphs: false,
        };
        let params = |glyph_id| RenderGlyphParams {
            font_id: face.font_id,
            glyph_id,
            font_size: px(24.0),
            subpixel_variant: point(0, 0),
            scale_factor: 1.0,
            raster_style: PreparedRasterStyle::independent(GlyphRenderMode::Grayscale),
        };
        let mut rasterizer = SwashGlyphRasterizer::default();
        let space = GlyphId(font.charmap().map(' ').unwrap().to_u32());
        let empty = rasterizer.rasterize(face, &params(space)).unwrap();
        assert_eq!(empty.size, Size::default());
        assert_eq!(empty.format, RasterizedGlyphFormat::AlphaMask);
        assert!(empty.pixels.is_empty());

        let glyph_count = u32::from(font.maxp().unwrap().num_glyphs());
        let error = rasterizer
            .rasterize(face, &params(GlyphId(glyph_count)))
            .unwrap_err();
        assert!(error.to_string().contains("outside the selected face"));

        let letter = GlyphId(font.charmap().map('A').unwrap().to_u32());
        let recovered = rasterizer.rasterize(face, &params(letter)).unwrap();
        assert!(recovered.pixels.iter().any(|coverage| *coverage != 0));
    }

    #[test]
    fn portable_bitmap_font_blanks_are_valid_empty_glyphs() {
        let source = Blob::from(NOTO_COLOR_EMOJI.to_vec());
        let font = FontRef::new(source.as_ref()).unwrap();
        let face = RasterFace {
            font_id: FontId(1),
            source_id: source.id(),
            source: &source,
            face_index: 0,
            normalized_coords: &[],
            variations: &[],
            synthesis: FontSynthesis::default(),
            has_color_glyphs: true,
        };
        let space = GlyphId(font.charmap().map(' ').unwrap().to_u32());
        let params = RenderGlyphParams {
            font_id: face.font_id,
            glyph_id: space,
            font_size: px(24.0),
            subpixel_variant: point(0, 0),
            scale_factor: 1.0,
            raster_style: PreparedRasterStyle::independent(GlyphRenderMode::Color),
        };

        let empty = SwashGlyphRasterizer::default()
            .rasterize(face, &params)
            .unwrap();

        assert_eq!(empty.size, Size::default());
        assert_eq!(empty.format, RasterizedGlyphFormat::BgraColor);
        assert!(empty.pixels.is_empty());
    }

    #[test]
    fn supported_color_artwork_is_selected_when_a_preferred_format_is_unsupported() {
        let available = [ColorGlyphKind::ColrV1, ColorGlyphKind::ColrV0];
        let selected = first_supported_color_kind(available, |kind| kind == ColorGlyphKind::ColrV0);

        assert_eq!(selected, Some(ColorGlyphKind::ColrV0));
    }

    #[test]
    fn sbix_records_are_artwork_even_when_their_payload_is_a_native_reference() {
        for graphic_type in [*b"flip", *b"dupe"] {
            let mut table = Vec::new();
            table.extend_from_slice(&1u16.to_be_bytes());
            table.extend_from_slice(&1u16.to_be_bytes());
            table.extend_from_slice(&1u32.to_be_bytes());
            table.extend_from_slice(&12u32.to_be_bytes());
            table.extend_from_slice(&20u16.to_be_bytes());
            table.extend_from_slice(&72u16.to_be_bytes());
            table.extend_from_slice(&16u32.to_be_bytes());
            table.extend_from_slice(&16u32.to_be_bytes());
            table.extend_from_slice(&26u32.to_be_bytes());
            table.extend_from_slice(&0i16.to_be_bytes());
            table.extend_from_slice(&0i16.to_be_bytes());
            table.extend_from_slice(&graphic_type);
            table.extend_from_slice(&0u16.to_be_bytes());

            let sbix = skrifa::raw::tables::sbix::Sbix::read(skrifa::raw::FontData::new(&table), 2)
                .unwrap();

            assert!(!sbix_has_glyph(&sbix, skrifa::GlyphId::new(0)));
            assert!(sbix_has_glyph(&sbix, skrifa::GlyphId::new(1)));
        }
    }

    #[test]
    fn subpixel_coverage_is_converted_from_swash_to_atlas_channel_order() {
        let mut pixels = vec![204, 127, 51, 0];
        convert_subpixel_mask_to_bgra(&mut pixels);

        assert_eq!(pixels, [51, 127, 204, 0]);
    }
}
