use anyhow::{Context as _, Result, bail};
use gpui::{
    Bounds, ColorGlyphClassifier, ColorGlyphKind, FontId, ForegroundDependency, GlyphId,
    GlyphRasterizer, GlyphRenderMode, PreparedRasterStyle, RasterColorEffect, RasterFace,
    RasterStyleRequest, RasterizedGlyph, RasterizedGlyphFormat, RenderGlyphParams,
    SUBPIXEL_VARIANTS_X, SUBPIXEL_VARIANTS_Y, TextRenderingMode, apply_pixel_opacity,
    multiply_alpha, point, size, swap_rgba_pa_to_bgra,
};
use skrifa::{
    FontRef, GlyphId as SkrifaGlyphId, MetadataProvider as _, Tag,
    bitmap::{BitmapFormat, BitmapStrikes},
    instance::Size as SkrifaSize,
    outline::{DrawSettings, OutlinePen},
    raw::TableProvider as _,
};
use std::collections::{HashMap, hash_map::Entry};
use swash::{
    FontRef as SwashFontRef,
    scale::{
        Render, ScaleContext, Source, StrikeWith,
        image::{Content, Image},
    },
    zeno::{Angle, Format, Transform, Vector},
};
use thiserror::Error;

#[derive(Debug, Error)]
enum SwashRasterError {
    #[error("invalid raster font size")]
    InvalidFontSize,

    #[error("invalid raster scale factor")]
    InvalidScaleFactor,

    #[error("invalid raster pixel size")]
    InvalidPixelSize,

    #[error("glyph ID {glyph_id} is outside the selected face's {glyph_count} glyphs")]
    InvalidGlyphId { glyph_id: u32, glyph_count: u32 },

    #[error("cannot parse the selected face's bitmap tables")]
    InvalidBitmapTables,
}

/// Swash raster state used by Linux, web, and explicit fallback construction.
#[derive(Default)]
pub struct SwashGlyphRasterizer {
    scale_context: ScaleContext,
}

impl SwashGlyphRasterizer {
    fn render_glyph_image(
        &mut self,
        face: &RasterFace<'_>,
        params: &RenderGlyphParams,
    ) -> Result<Option<Image>> {
        let pixel_size = f32::from(params.font_size) * params.scale_factor;

        if !pixel_size.is_finite() || pixel_size <= 0.0 {
            return Err(SwashRasterError::InvalidPixelSize.into());
        }

        let is_cbdt = self.requested_color_glyph_kind(face, params)? == Some(ColorGlyphKind::Cbdt);

        let sources: &[Source] = if is_cbdt {
            &[Source::ColorBitmap(StrikeWith::BestFit), Source::Outline]
        } else if params.raster_style.mode == GlyphRenderMode::Color {
            &[
                Source::ColorOutline(0),
                Source::ColorBitmap(StrikeWith::BestFit),
                Source::Outline,
            ]
        } else {
            &[Source::Bitmap(StrikeWith::ExactSize), Source::Outline]
        };

        let font_ref = SwashFontRef::from_index(face.data(), face.face_index as usize)
            .context("Swash could not parse the stored font face")?;

        let subpixel_offset = subpixel_offset(params);

        let mut scaler = self
            .scale_context
            .builder_with_id(font_ref, [face.source_id, u64::from(face.face_index)])
            .size(pixel_size)
            .normalized_coords(
                face.normalized_coords
                    .iter()
                    .map(|coordinate| coordinate.to_bits()),
            )
            .hint(true)
            .build();

        let mut renderer = Render::new(sources);

        if params.raster_style.mode == GlyphRenderMode::Subpixel {
            renderer.format(Format::subpixel_bgra());
        } else {
            renderer.format(Format::Alpha);
        }

        if let RasterColorEffect::Preblend(color) = params.raster_style.color_effect {
            renderer.default_color([color.red, color.green, color.blue, color.alpha]);
        }

        renderer.offset(subpixel_offset);

        if face.synthesis.embolden {
            renderer.embolden(pixel_size / 48.0);
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
        let font_size = f32::from(params.font_size);

        if !font_size.is_finite() || font_size < 0.0 {
            return Err(SwashRasterError::InvalidFontSize.into());
        }

        if !params.scale_factor.is_finite() || params.scale_factor <= 0.0 {
            return Err(SwashRasterError::InvalidScaleFactor.into());
        }

        if font_size == 0.0 {
            return Ok(RasterizedGlyph::empty(format));
        }

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
            Content::Color => {
                convert_color_image_to_bgra(&mut image, params.raster_style)?;

                (RasterizedGlyphFormat::BgraColor, image.data)
            }
            Content::SubpixelMask => {
                convert_subpixel_mask_to_bgra(&mut image.data);

                (RasterizedGlyphFormat::BgraSubpixelMask, image.data)
            }
            Content::Mask if params.raster_style.mode == GlyphRenderMode::Subpixel => (
                RasterizedGlyphFormat::BgraSubpixelMask,
                image
                    .data
                    .iter()
                    .flat_map(|&alpha| [alpha, alpha, alpha, 0])
                    .collect(),
            ),
            Content::Mask
                if params.raster_style.mode == GlyphRenderMode::Color
                    && params.raster_style.foreground_dependency
                        != ForegroundDependency::AlphaOnly =>
            {
                let [red, green, blue, foreground_alpha] = match params.raster_style.color_effect {
                    RasterColorEffect::Preblend(color) => color.into(),
                    RasterColorEffect::Independent => [0, 0, 0, 255],
                    RasterColorEffect::Dilation(_) => {
                        bail!("color glyph rasterization cannot use a dilation style")
                    }
                };

                let pixels = image
                    .data
                    .into_iter()
                    .flat_map(|coverage| {
                        let alpha = multiply_alpha(coverage, foreground_alpha);

                        [blue, green, red, alpha]
                    })
                    .collect();

                (RasterizedGlyphFormat::BgraColor, pixels)
            }
            Content::Mask => (RasterizedGlyphFormat::AlphaMask, image.data),
        };

        let raster = RasterizedGlyph {
            bounds,
            size: bounds.size,
            format,
            pixels,
        };

        raster.validate()?;

        Ok(raster)
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
            Entry::Vacant(entry) => *entry.insert(requires_portable_bitmap_rasterization(&face)?),
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

fn subpixel_offset(params: &RenderGlyphParams) -> Vector {
    Vector::new(
        params.subpixel_variant.x as f32 / SUBPIXEL_VARIANTS_X as f32 / params.scale_factor,
        params.subpixel_variant.y as f32 / SUBPIXEL_VARIANTS_Y as f32 / params.scale_factor,
    )
}

fn convert_color_image_to_bgra(image: &mut Image, style: PreparedRasterStyle) -> Result<()> {
    let premultiplied = matches!(image.source, Source::ColorOutline(_));
    let is_bitmap = matches!(image.source, Source::ColorBitmap(_));

    let foreground_alpha = match style.color_effect {
        RasterColorEffect::Preblend(color) => color.alpha,
        RasterColorEffect::Dilation(_) if is_bitmap => {
            bail!("color bitmap rasterization cannot use a dilation style")
        }
        _ => 255,
    };

    for pixel in image.data.chunks_exact_mut(4) {
        if premultiplied {
            swap_rgba_pa_to_bgra(pixel);
        } else {
            pixel.swap(0, 2);
        }

        if is_bitmap {
            apply_pixel_opacity(pixel, foreground_alpha);
        }
    }

    Ok(())
}

fn convert_subpixel_mask_to_bgra(pixels: &mut [u8]) {
    for pixel in pixels.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
}

fn glyph_is_intentionally_empty(face: &RasterFace<'_>, glyph_id: GlyphId) -> Result<bool> {
    let font = FontRef::from_index(face.data(), face.face_index)
        .context("cannot inspect a glyph in the selected face")?;

    let glyph_count = u32::from(
        font.maxp()
            .context("cannot read the selected face's glyph count")?
            .num_glyphs(),
    );

    if glyph_id.0 >= glyph_count {
        return Err(SwashRasterError::InvalidGlyphId {
            glyph_id: glyph_id.0,
            glyph_count,
        }
        .into());
    }

    let skrifa_id = SkrifaGlyphId::new(glyph_id.0);
    let outlines = font.outline_glyphs();

    if outlines.format().is_some() {
        let outline = outlines
            .get(skrifa_id)
            .context("cannot parse the selected glyph's outline")?;

        let mut pen = DrawablePathPen::default();

        outline
            .draw(
                DrawSettings::unhinted(SkrifaSize::unscaled(), face.normalized_coords),
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
            if has_tables {
                return Err(SwashRasterError::InvalidBitmapTables.into());
            }

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

fn requires_portable_bitmap_rasterization(face: &RasterFace<'_>) -> Result<bool> {
    let font = FontRef::from_index(face.data(), face.face_index)
        .context("cannot inspect color tables in the selected face")?;

    let has_table = |tag| font.table_data(Tag::new(tag)).is_some();

    Ok(has_table(b"CBDT")
        && has_table(b"CBLC")
        && ![b"COLR", b"sbix", b"SVG "].into_iter().any(has_table))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{
        AppContext as _, Context, FontDataBlob, FontId, FontSynthesis,
        ForegroundDependency::{AlphaOnly, Full},
        GlyphRenderMode::{Color, Grayscale, Subpixel},
        HeadlessAppContext, IntoElement, LineLayout, PaintFragment, Pixels, PlatformTextSystem,
        RasterizedGlyphFormat::{AlphaMask, BgraColor, BgraSubpixelMask},
        Render, Rgba, Rgba8, ShapedGlyph, Size, Styled, TextLayoutOptions, TextLayoutRequest,
        TextRun, TruncateFrom, Window, div, font,
        prelude::*,
        px, rgba,
    };
    use gpui_fonts::{
        IBM_PLEX, IBM_PLEX_SEMIBOLD, NOTO_COLOR_EMOJI, SOURCE_SERIF, test_collection,
    };
    use gpui_parley::ParleyTextSystem;
    use gpui_parley::test_support::{
        OVERFLOW_CASES, OverflowParagraph, SCALE_FACTOR, first_group_color, logical_bounds,
        open_overflow_paragraph, quads, second_group_color, update_view,
    };
    use skrifa::{bitmap::BitmapData, raw::MinByteRange as _};
    use std::{borrow::Cow, sync::Arc};

    fn test_system() -> Arc<ParleyTextSystem> {
        let system = ParleyTextSystem::new(SwashGlyphRasterizer::default())
            .with_fallback_families([IBM_PLEX.family, NOTO_COLOR_EMOJI.family]);

        system
            .add_fonts(vec![
                Cow::Borrowed(IBM_PLEX.data),
                Cow::Borrowed(IBM_PLEX_SEMIBOLD.data),
                Cow::Borrowed(SOURCE_SERIF.data),
                Cow::Borrowed(NOTO_COLOR_EMOJI.data),
            ])
            .unwrap();

        Arc::new(system)
    }

    fn text_run(text: &str, family: &str) -> TextRun {
        TextRun {
            len: text.len(),
            font: font(family),
            ..Default::default()
        }
    }

    fn layout_line(
        system: &ParleyTextSystem,
        text: &str,
        font_size: Pixels,
        runs: &[TextRun],
    ) -> LineLayout {
        system.layout_text(TextLayoutRequest {
            text,
            font_size,
            runs,
            options: TextLayoutOptions::default(),
        })
    }

    fn shaped_glyphs(
        layout: &LineLayout,
    ) -> impl Iterator<Item = (&PaintFragment, usize, &ShapedGlyph)> {
        layout.paint_fragments.iter().flat_map(|fragment| {
            fragment
                .glyphs
                .iter()
                .enumerate()
                .map(move |(glyph_idx, glyph)| (fragment, glyph_idx, glyph))
        })
    }

    fn headless() -> HeadlessAppContext {
        HeadlessAppContext::new(test_system())
    }

    fn raster_face(source: &FontDataBlob<u8>, face_index: u32) -> RasterFace<'_> {
        let font = FontRef::from_index(source.as_ref(), face_index).unwrap();

        let has_color_glyphs = [*b"CBDT", *b"sbix", *b"COLR", *b"SVG "]
            .into_iter()
            .any(|tag| font.table_data(Tag::new(&tag)).is_some());

        RasterFace {
            font_id: FontId(1),
            source_id: source.id(),
            source,
            face_index,
            normalized_coords: &[],
            variations: &[],
            synthesis: FontSynthesis::default(),
            has_color_glyphs,
        }
    }

    fn raster_params(
        font_id: FontId,
        glyph_id: GlyphId,
        mode: GlyphRenderMode,
    ) -> RenderGlyphParams {
        RenderGlyphParams {
            font_id,
            glyph_id,
            font_size: px(24.0),
            subpixel_variant: point(0, 0),
            scale_factor: 1.0,
            raster_style: PreparedRasterStyle::independent(mode),
        }
    }

    fn color_request(font_id: FontId, glyph_id: GlyphId, scene_color: Rgba) -> RasterStyleRequest {
        RasterStyleRequest {
            font_id,
            glyph_id,
            scene_color,
            requested_mode: Color,
            foreground_dependency: ForegroundDependency::Full,
        }
    }

    fn assert_visible_raster(
        raster: &RasterizedGlyph,
        expected_format: RasterizedGlyphFormat,
        case: &str,
    ) {
        raster
            .validate()
            .unwrap_or_else(|error| panic!("{case}: {error}"));

        assert_eq!(raster.format, expected_format, "{case}");

        assert!(
            raster.size.width.0 > 0 && raster.size.height.0 > 0,
            "{case}"
        );

        let visible = match expected_format {
            AlphaMask => raster.pixels.iter().any(|&alpha| alpha > 0),
            BgraSubpixelMask => raster
                .pixels
                .chunks_exact(4)
                .any(|pixel| pixel[0] != pixel[1] || pixel[1] != pixel[2]),
            BgraColor => raster
                .pixels
                .chunks_exact(4)
                .any(|pixel| pixel[3] > 128 && (pixel[0] != pixel[1] || pixel[1] != pixel[2])),
        };

        assert!(
            visible,
            "{case}: missing visible {expected_format:?} pixels"
        );
    }

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
        let mut rasterizer =
            BitmapFallbackGlyphRasterizer::new(RecordingNativeRasterizer::default());

        assert!(rasterizer.supports_color_glyph(ColorGlyphKind::Cbdt));
        assert!(rasterizer.supports_color_glyph(ColorGlyphKind::ColrV0));
        assert!(!rasterizer.supports_color_glyph(ColorGlyphKind::Sbix));

        for (idx, (fixture, characters, mode, native_calls)) in [
            (NOTO_COLOR_EMOJI, "😀 ", Color, 0),
            (SOURCE_SERIF, "A", Grayscale, 1),
        ]
        .into_iter()
        .enumerate()
        {
            let source = FontDataBlob::from(fixture.data.to_vec());
            let font = FontRef::new(source.as_ref()).unwrap();
            let face = RasterFace {
                font_id: FontId(idx + 1),
                ..raster_face(&source, 0)
            };

            for character in characters.chars() {
                let glyph_id = GlyphId(font.charmap().map(character).unwrap().to_u32());
                let params = raster_params(face.font_id, glyph_id, mode);

                if character == '😀' {
                    assert_eq!(
                        rasterizer
                            .requested_color_glyph_kind(&face, &params)
                            .unwrap(),
                        Some(ColorGlyphKind::Cbdt)
                    );

                    for mode in [Grayscale, Subpixel] {
                        let request = raster_params(face.font_id, glyph_id, mode);

                        assert_eq!(
                            rasterizer
                                .requested_color_glyph_kind(&face, &request)
                                .unwrap(),
                            None
                        );
                    }
                }

                rasterizer.rasterize(face, &params).unwrap();

                assert_eq!(
                    rasterizer.native.raster_calls, native_calls,
                    "{} {character:?} {mode:?}",
                    fixture.family
                );
            }
        }
    }

    #[test]
    fn portable_rasterization_preserves_legacy_subpixel_offsets() {
        for scale_factor in [1.0, 2.0] {
            let params = RenderGlyphParams {
                subpixel_variant: point(SUBPIXEL_VARIANTS_X - 1, 0),
                scale_factor,
                ..raster_params(FontId(1), GlyphId(1), Color)
            };

            assert_eq!(
                subpixel_offset(&params),
                Vector::new(0.75 / scale_factor, 0.0)
            );
        }
    }

    fn assert_cached_raster_matches_fresh(
        rasterizer: &mut SwashGlyphRasterizer,
        face: RasterFace<'_>,
        params: &RenderGlyphParams,
    ) -> RasterizedGlyph {
        let case = format!(
            "source {}, face {}, FontId {:?}, coords {:?}, synthesis {:?}",
            face.source_id, face.face_index, face.font_id, face.normalized_coords, face.synthesis
        );

        let actual = rasterizer
            .rasterize(face, params)
            .unwrap_or_else(|error| panic!("{case}: {error}"));

        let expected = SwashGlyphRasterizer::default()
            .rasterize(face, params)
            .unwrap();

        assert_visible_raster(&actual, params.raster_style.mode.rasterized_format(), &case);
        assert_eq!(actual.format, expected.format, "{case}");
        assert_eq!(actual.bounds, expected.bounds, "{case}");
        assert_eq!(actual.size, expected.size, "{case}");
        assert_eq!(actual.pixels, expected.pixels, "{case}");

        actual
    }

    #[test]
    fn portable_cache_distinguishes_sources_and_collection_faces() {
        let plex = FontDataBlob::from(IBM_PLEX.data.to_vec());
        let serif = FontDataBlob::from(SOURCE_SERIF.data.to_vec());
        let emoji = FontDataBlob::from(NOTO_COLOR_EMOJI.data.to_vec());

        let collection = FontDataBlob::from(test_collection(&[
            IBM_PLEX.data,
            NOTO_COLOR_EMOJI.data,
            SOURCE_SERIF.data,
        ]));

        let faces = [
            (raster_face(&plex, 0), 'A', Grayscale),
            (raster_face(&emoji, 0), '😀', Color),
            (raster_face(&serif, 0), 'A', Grayscale),
            (raster_face(&collection, 0), 'A', Grayscale),
            (raster_face(&collection, 1), '😀', Color),
            (raster_face(&collection, 2), 'A', Grayscale),
        ];

        let mut rasterizer = SwashGlyphRasterizer::default();

        for font_id in [FontId(1), FontId(2)] {
            for (mut face, character, mode) in faces.into_iter().chain(faces.into_iter().rev()) {
                face.font_id = font_id;
                let font = FontRef::from_index(face.data(), face.face_index).unwrap();
                let glyph_id = GlyphId(font.charmap().map(character).unwrap().to_u32());
                let params = raster_params(font_id, glyph_id, mode);
                assert_cached_raster_matches_fresh(&mut rasterizer, face, &params);
            }
        }
    }

    #[test]
    fn portable_cache_preserves_normalized_coordinates_and_synthesis() {
        let source = FontDataBlob::from(SOURCE_SERIF.data.to_vec());
        let font = FontRef::new(source.as_ref()).unwrap();
        let default_coords = font.axes().location([] as [(Tag, f32); 0]);
        let optical_coords = font.axes().location([(Tag::new(b"opsz"), 72.0)]);
        let weight_coords = font.axes().location([(Tag::new(b"wght"), 700.0)]);

        let default_face = RasterFace {
            normalized_coords: default_coords.coords(),
            ..raster_face(&source, 0)
        };

        let instances = [
            RasterFace {
                normalized_coords: optical_coords.coords(),
                ..default_face
            },
            RasterFace {
                normalized_coords: weight_coords.coords(),
                ..default_face
            },
            RasterFace {
                synthesis: FontSynthesis {
                    embolden: true,
                    ..FontSynthesis::default()
                },
                ..default_face
            },
            RasterFace {
                synthesis: FontSynthesis {
                    skew_degrees: Some(14.0),
                    ..FontSynthesis::default()
                },
                ..default_face
            },
            RasterFace {
                normalized_coords: optical_coords.coords(),
                synthesis: FontSynthesis {
                    embolden: true,
                    skew_degrees: Some(14.0),
                },
                ..default_face
            },
        ];

        let glyph_id = GlyphId(font.charmap().map('A').unwrap().to_u32());
        let mut rasterizer = SwashGlyphRasterizer::default();

        let params = RenderGlyphParams {
            font_size: px(48.0),
            ..raster_params(default_face.font_id, glyph_id, Grayscale)
        };

        let original = assert_cached_raster_matches_fresh(&mut rasterizer, default_face, &params);

        for (idx, instance) in instances.into_iter().enumerate() {
            let face = RasterFace {
                font_id: FontId(idx + 2),
                ..instance
            };

            let instance_params = RenderGlyphParams {
                font_id: face.font_id,
                ..params.clone()
            };

            let changed =
                assert_cached_raster_matches_fresh(&mut rasterizer, face, &instance_params);

            assert_ne!(changed.pixels, original.pixels, "instance {idx}");

            let restored =
                assert_cached_raster_matches_fresh(&mut rasterizer, default_face, &params);

            assert_eq!(restored.bounds, original.bounds, "instance {idx}");
            assert_eq!(restored.pixels, original.pixels, "instance {idx}");
        }
    }

    #[test]
    fn portable_color_outline_fallback_returns_a_tintable_mask() {
        let source = FontDataBlob::from(SOURCE_SERIF.data.to_vec());
        let font = FontRef::new(source.as_ref()).unwrap();
        let face = raster_face(&source, 0);
        let glyph_id = GlyphId(font.charmap().map('A').unwrap().to_u32());
        let mut rasterizer = SwashGlyphRasterizer::default();

        let raster_style = rasterizer.prepare_style(RasterStyleRequest {
            foreground_dependency: AlphaOnly,
            ..color_request(face.font_id, glyph_id, rgba(0xe02010cc))
        });

        let params = RenderGlyphParams {
            raster_style,
            ..raster_params(face.font_id, glyph_id, Color)
        };

        let raster = rasterizer.rasterize(face, &params).unwrap();
        assert_visible_raster(&raster, AlphaMask, "tintable outline fallback");
    }

    #[test]
    fn subpixel_coverage_is_converted_from_swash_to_atlas_channel_order() {
        let mut pixels = vec![204, 127, 51, 0];
        convert_subpixel_mask_to_bgra(&mut pixels);

        assert_eq!(pixels, [51, 127, 204, 0]);
    }

    #[test]
    fn portable_rasterization_validates_requests_and_preserves_empty_glyphs() {
        for (data, mode, visible) in [
            (IBM_PLEX.data, Grayscale, 'A'),
            (IBM_PLEX.data, Subpixel, 'A'),
            (SOURCE_SERIF.data, Grayscale, 'A'),
            (NOTO_COLOR_EMOJI.data, Color, '😀'),
        ] {
            let name = format!("{mode:?} glyph {visible}");
            let source = FontDataBlob::from(data.to_vec());
            let font = FontRef::new(source.as_ref()).unwrap();
            let face = raster_face(&source, 0);

            let params = |glyph_id| raster_params(face.font_id, glyph_id, mode);
            let glyph = GlyphId(font.charmap().map(visible).unwrap().to_u32());
            let space = GlyphId(font.charmap().map(' ').unwrap().to_u32());
            let mut rasterizer = SwashGlyphRasterizer::default();

            for (glyph_id, font_size) in [(space, px(24.0)), (glyph, px(0.0))] {
                let mut request = params(glyph_id);
                request.font_size = font_size;
                let empty = rasterizer.rasterize(face, &request).unwrap();
                empty.validate().unwrap();
                assert_eq!(empty.size, Size::default(), "{name}");
                assert_eq!(empty.format, mode.rasterized_format(), "{name}");
                assert!(empty.pixels.is_empty(), "{name}");
            }

            let glyph_count = u32::from(font.maxp().unwrap().num_glyphs());

            let error = rasterizer
                .rasterize(face, &params(GlyphId(glyph_count)))
                .unwrap_err();

            assert!(
                matches!(
                    error.downcast_ref::<SwashRasterError>(),
                    Some(SwashRasterError::InvalidGlyphId {
                        glyph_id,
                        glyph_count: count,
                    }) if *glyph_id == glyph_count && *count == glyph_count
                ),
                "{name}: {error}"
            );

            for (font_size, scale_factor) in [
                (-1.0, 1.0),
                (f32::NAN, 1.0),
                (f32::INFINITY, 1.0),
                (0.0, 0.0),
                (24.0, 0.0),
                (24.0, -1.0),
                (24.0, f32::NAN),
                (24.0, f32::INFINITY),
                (f32::MAX, 2.0),
                (f32::MIN_POSITIVE, f32::MIN_POSITIVE),
            ] {
                let mut request = params(glyph);
                request.font_size = px(font_size);
                request.scale_factor = scale_factor;

                assert!(
                    rasterizer.rasterize(face, &request).is_err(),
                    "{name}: accepted font size {font_size} at scale {scale_factor}"
                );
            }

            let recovered = rasterizer.rasterize(face, &params(glyph)).unwrap();
            assert_visible_raster(&recovered, mode.rasterized_format(), &name);
        }
    }

    #[test]
    fn corrupt_color_bitmaps_fall_back_to_tintable_outlines_when_available() {
        for include_outlines in [false, true] {
            let source = FontDataBlob::from(damaged_emoji_font(include_outlines));
            let font = FontRef::new(source.as_ref()).unwrap();
            let face = raster_face(&source, 0);
            let glyph_id = GlyphId(font.charmap().map('😀').unwrap().to_u32());
            let mut rasterizer = SwashGlyphRasterizer::default();

            let raster_style = rasterizer.prepare_style(RasterStyleRequest {
                foreground_dependency: AlphaOnly,
                ..color_request(face.font_id, glyph_id, rgba(0xe0201080))
            });

            let params = RenderGlyphParams {
                raster_style,
                ..raster_params(face.font_id, glyph_id, Color)
            };

            assert_eq!(
                rasterizer
                    .requested_color_glyph_kind(&face, &params)
                    .unwrap(),
                Some(ColorGlyphKind::Cbdt)
            );

            let result = rasterizer.rasterize(face, &params);

            if include_outlines {
                let fallback = result.unwrap();
                assert_visible_raster(&fallback, AlphaMask, "corrupt bitmap outline fallback");

                let outline = rasterizer
                    .rasterize(face, &raster_params(face.font_id, glyph_id, Grayscale))
                    .unwrap();

                assert_eq!(fallback.bounds, outline.bounds);
                assert_eq!(fallback.pixels, outline.pixels);
            } else {
                assert!(result.is_err());
            }

            let glyph_id = GlyphId(font.charmap().map('🚀').unwrap().to_u32());

            let recovered = rasterizer
                .rasterize(face, &raster_params(face.font_id, glyph_id, Color))
                .unwrap();

            assert_visible_raster(&recovered, BgraColor, "valid bitmap after decode failure");
        }
    }

    fn damaged_emoji_font(include_outlines: bool) -> Vec<u8> {
        let mut data = NOTO_COLOR_EMOJI.data.to_vec();
        let font = FontRef::new(&data).unwrap();
        let glyph_id = font.charmap().map('😀').unwrap();

        let bitmap = BitmapStrikes::with_format(&font, BitmapFormat::Cbdt)
            .unwrap()
            .get(0)
            .unwrap()
            .get(glyph_id)
            .unwrap();

        let BitmapData::Png(payload) = bitmap.data else {
            panic!("expected a CBDT PNG payload");
        };

        let payload_offset = payload.as_ptr() as usize - data.as_ptr() as usize;
        data[payload_offset..payload_offset + 8].fill(0);

        if !include_outlines {
            return data;
        }

        let font = FontRef::new(&data).unwrap();
        let directory_end = font.table_directory().min_table_bytes().len();
        let glyph_count = usize::from(font.maxp().unwrap().num_glyphs());
        let mut records = data[12..directory_end].as_chunks::<16>().0.to_vec();
        let outline_font = FontRef::new(IBM_PLEX.data).unwrap();

        // Reuse real outlines to exercise recovery from a failed CBDT decode.
        for (old_tag, new_tag) in [(b"vhea", b"glyf"), (b"vmtx", b"loca")] {
            let record = records
                .iter_mut()
                .find(|record| &record[..4] == old_tag)
                .unwrap();

            let table = outline_font.table_data(Tag::new(new_tag)).unwrap();
            let table = table.as_bytes();

            let table = if new_tag == b"loca" {
                &table[..(glyph_count + 1) * 2]
            } else {
                table
            };

            data.resize(data.len().next_multiple_of(4), 0);
            record[..4].copy_from_slice(new_tag);
            record[8..12].copy_from_slice(&(data.len() as u32).to_be_bytes());
            record[12..].copy_from_slice(&(table.len() as u32).to_be_bytes());
            data.extend_from_slice(table);
        }

        records.sort();
        data[12..directory_end].copy_from_slice(records.as_flattened());

        data
    }

    #[test]
    fn fixed_color_bitmaps_ignore_foreground_rgb_but_preserve_alpha() {
        let system = test_system();
        let font_id = system.font_id(&font(NOTO_COLOR_EMOJI.family)).unwrap();
        let glyph_id = system.glyph_for_char(font_id, '😀').unwrap();

        let style = |color| system.prepare_raster_style(color_request(font_id, glyph_id, color));

        let mut params = raster_params(font_id, glyph_id, GlyphRenderMode::Color);
        params.scale_factor = 1.5;
        params.raster_style = style(rgba(0xffffffff));
        let opaque = system.rasterize_glyph(&params).unwrap();

        assert!(
            opaque
                .pixels
                .chunks_exact(4)
                .any(|pixel| pixel[3] > 0 && pixel[3] < 255)
        );

        for alpha in [0u8, 64, 128, 255] {
            let red = style(rgba(0xff000000 | u32::from(alpha)));
            let blue = style(rgba(0x0000ff00 | u32::from(alpha)));
            assert_eq!(red, blue);
            assert_eq!(red.foreground_dependency, ForegroundDependency::AlphaOnly);

            assert_eq!(
                red.color_effect,
                RasterColorEffect::Preblend(Rgba8::new(0, 0, 0, alpha))
            );

            params.raster_style = red;
            let raster = system.rasterize_glyph(&params).unwrap();
            raster.validate().unwrap();
            assert_eq!(raster.bounds, opaque.bounds);
            assert_eq!(raster.format, RasterizedGlyphFormat::BgraColor);

            for (actual, original) in raster
                .pixels
                .chunks_exact(4)
                .zip(opaque.pixels.chunks_exact(4))
            {
                let expected_alpha =
                    (f32::from(original[3]) * f32::from(alpha) / 255.0).round() as u8;

                let expected = if expected_alpha == 0 {
                    [0; 4]
                } else {
                    [original[0], original[1], original[2], expected_alpha]
                };

                assert_eq!(actual, expected, "foreground alpha {alpha}");
            }
        }

        params.raster_style.color_effect = RasterColorEffect::Dilation(1);
        assert!(system.rasterize_glyph(&params).is_err());
    }

    #[test]
    fn bundled_emoji_sample_keeps_font_metrics_and_device_scaling() {
        const SAMPLE: &str = "1 1 Color emoji: 😀 🎉 🚀 💡 🔥 ✨";
        const EMOJI: [char; 8] = ['1', '1', '😀', '🎉', '🚀', '💡', '🔥', '✨'];

        let system = test_system();
        let noto_id = system.font_id(&font("Noto Color Emoji")).unwrap();
        let source = FontDataBlob::from(NOTO_COLOR_EMOJI.data.to_vec());
        let font = FontRef::new(source.as_ref()).unwrap();

        let strike = BitmapStrikes::with_format(&font, BitmapFormat::Cbdt)
            .unwrap()
            .get(0)
            .unwrap();

        for font_size in [px(16.0), px(24.0), px(32.0), px(strike.ppem())] {
            let layout = layout_line(
                &system,
                SAMPLE,
                font_size,
                &[
                    text_run("1 1", NOTO_COLOR_EMOJI.family),
                    text_run(&SAMPLE[3..], IBM_PLEX.family),
                ],
            );

            let emoji_glyphs = shaped_glyphs(&layout)
                .filter(|(_, _, glyph)| glyph.is_emoji)
                .collect::<Vec<_>>();

            assert_eq!(emoji_glyphs.len(), EMOJI.len());
            assert_eq!(layout.font_size, font_size);

            for (character, (fragment, glyph_idx, glyph)) in EMOJI.into_iter().zip(emoji_glyphs) {
                assert_eq!(
                    fragment.font_id, noto_id,
                    "{character} selected another font"
                );

                assert_eq!(fragment.font_size, font_size);
                assert_eq!(glyph.position.y, Pixels::ZERO);

                let bitmap = strike.get(SkrifaGlyphId::new(glyph.id.0)).unwrap();

                assert_eq!(
                    glyph.id,
                    system.glyph_for_char(noto_id, character).unwrap(),
                    "{character} did not use its nominal bundled glyph"
                );

                let shaped_advance = fragment
                    .glyphs
                    .get(glyph_idx + 1)
                    .map(|next| next.position.x - glyph.position.x)
                    .unwrap_or(fragment.x_range.end - glyph.position.x);

                assert!(shaped_advance > Pixels::ZERO);

                let raster_style = system.prepare_raster_style(color_request(
                    fragment.font_id,
                    glyph.id,
                    rgba(0xffffffff),
                ));

                for scale_factor in [1.0, 1.5, 2.0] {
                    let params = RenderGlyphParams {
                        font_id: fragment.font_id,
                        glyph_id: glyph.id,
                        font_size: fragment.font_size,
                        subpixel_variant: point(0, 0),
                        scale_factor,
                        raster_style,
                    };

                    let raster = system.rasterize_glyph(&params).unwrap();

                    let case = format!("{character}, size={font_size:?}, scale={scale_factor}");
                    assert_visible_raster(&raster, BgraColor, &case);

                    let bitmap_scale = f32::from(font_size) * scale_factor / bitmap.ppem_y;

                    let expected = Bounds {
                        origin: point(bitmap.inner_bearing_x, -bitmap.inner_bearing_y),
                        size: size(bitmap.width as f32, bitmap.height as f32),
                    }
                    .map(|value| ((value * bitmap_scale) as i32).into());

                    assert_eq!(
                        raster.bounds, expected,
                        "{character} at scale {scale_factor}"
                    );

                    for subpixel_variant in [
                        point(1, 0),
                        point(SUBPIXEL_VARIANTS_X - 1, SUBPIXEL_VARIANTS_Y - 1),
                    ] {
                        let mut shifted = params.clone();
                        shifted.subpixel_variant = subpixel_variant;
                        let shifted = system.rasterize_glyph(&shifted).unwrap();

                        assert_eq!(shifted.bounds, raster.bounds);
                        assert_eq!(shifted.pixels, raster.pixels);
                    }
                }
            }
        }
    }

    #[test]
    fn color_styles_preserve_only_the_foreground_channels_used_by_the_artwork() {
        let system = test_system();

        for (fixture, character, dependency, expected_color) in [
            (SOURCE_SERIF, 'A', Full, Rgba8::new(224, 32, 16, 204)),
            (NOTO_COLOR_EMOJI, '😀', AlphaOnly, Rgba8::new(0, 0, 0, 204)),
        ] {
            let font_id = system.font_id(&font(fixture.family)).unwrap();
            let glyph_id = system.glyph_for_char(font_id, character).unwrap();

            let style =
                |color| system.prepare_raster_style(color_request(font_id, glyph_id, color));

            let red = style(rgba(0xe02010cc));
            let blue = style(rgba(0x1020e0cc));
            let translucent = style(rgba(0x1020e080));

            assert_eq!(red.mode, Color, "{}", fixture.family);
            assert_eq!(red.foreground_dependency, dependency, "{}", fixture.family);

            assert_eq!(
                red.color_effect,
                RasterColorEffect::Preblend(expected_color),
                "{}",
                fixture.family
            );

            assert_eq!(
                red == blue,
                dependency == AlphaOnly,
                "{}: {red:?}, {blue:?}",
                fixture.family
            );

            assert_ne!(red, translucent, "{}", fixture.family);
        }
    }

    struct MixedTypography;

    impl Render for MixedTypography {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            div().size_full().items_start().child(
                div()
                    .block()
                    .w(px(170.))
                    .text_size(px(17.))
                    .line_height(px(24.))
                    .child("Before ")
                    .child(
                        div()
                            .inline()
                            .text_size(px(30.))
                            .line_height(px(46.))
                            .text_color(first_group_color())
                            .text_bg(first_group_color())
                            .underline()
                            .child("Tall letters wrap ")
                            .child(
                                div()
                                    .inline()
                                    .text_size(px(14.))
                                    .line_height(px(20.))
                                    .text_color(second_group_color())
                                    .text_bg(second_group_color())
                                    .child("small tail"),
                            ),
                    )
                    .child(" after."),
            )
        }
    }

    #[test]
    fn mixed_font_sizes_colors_and_underlines_paint_in_their_wrapped_rows() {
        let mut cx = headless();

        let window = cx
            .open_window(size(px(320.), px(300.)), |window, cx| {
                window.set_scale_factor(SCALE_FACTOR);
                cx.new(|_| MixedTypography)
            })
            .unwrap();
        cx.run_until_parked();

        let mut glyph_heights = Vec::new();

        for color in [first_group_color(), second_group_color()] {
            let backgrounds = quads(&mut cx, window.into(), color);
            assert!(!backgrounds.is_empty());

            let underlines: Vec<_> = cx
                .underline_bounds(window.into(), color)
                .unwrap()
                .into_iter()
                .map(logical_bounds)
                .collect();
            assert_eq!(underlines.len(), backgrounds.len());

            for underline in underlines {
                assert!(
                    backgrounds
                        .iter()
                        .any(
                            |background| (underline.origin.x - background.origin.x).abs() < px(1.)
                                && underline.origin.y >= background.origin.y
                                && underline.bottom() <= background.bottom() + px(1.)
                        ),
                    "underline {underline:?} outside {backgrounds:?}"
                );
            }

            let glyphs: Vec<_> = cx
                .glyph_bounds(window.into(), color)
                .unwrap()
                .into_iter()
                .map(logical_bounds)
                .collect();
            assert!(!glyphs.is_empty());

            for glyph in &glyphs {
                assert!(
                    backgrounds
                        .iter()
                        .any(|background| background.contains(&glyph.center())),
                    "glyph {glyph:?} outside {backgrounds:?}"
                );
            }

            glyph_heights.push(
                glyphs
                    .iter()
                    .map(|bounds| bounds.size.height)
                    .fold(px(0.), Pixels::max),
            );
        }

        assert!(quads(&mut cx, window.into(), first_group_color()).len() >= 2);
        assert!(
            glyph_heights[0] > glyph_heights[1] * 1.5,
            "glyph painting must use each shaped run's font size"
        );
    }

    #[test]
    fn block_overflow_paints_glyphs_for_each_truncation_mode() {
        let mut cx = headless();

        for (direction, max_lines, width, scale) in OVERFLOW_CASES {
            let mut view = OverflowParagraph::new(direction);
            view.max_lines = max_lines;
            view.width = width;
            let (window, _painted) = open_overflow_paragraph(&mut cx, view, 360., scale);

            assert!(
                !cx.glyph_bounds(window.into(), first_group_color())
                    .unwrap()
                    .is_empty(),
                "{direction:?}, max_lines={max_lines:?}, width={width}, scale={scale}"
            );
        }
    }

    #[test]
    fn block_ellipsis_only_paints_glyphs_in_displayed_spans() {
        let mut cx = headless();
        let mut view = OverflowParagraph::new(TruncateFrom::Middle);
        view.styled = true;
        view.width = 260.;
        let (window, _painted) = open_overflow_paragraph(&mut cx, view, 360., SCALE_FACTOR);

        for width in [260., 100., 260.] {
            update_view(&mut cx, window, |view| view.width = width);
            let backgrounds = quads(&mut cx, window.into(), second_group_color());

            let glyphs = cx
                .glyph_bounds(window.into(), second_group_color())
                .unwrap();

            assert_eq!(glyphs.is_empty(), width == 100., "width={width}");

            for glyph in glyphs {
                assert!(
                    backgrounds
                        .iter()
                        .any(|bounds| bounds.contains(&logical_bounds(glyph).center())),
                    "width={width}: glyph {glyph:?} outside {backgrounds:?}"
                );
            }
        }
    }
}
