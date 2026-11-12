use anyhow::{Context as _, Result, bail, ensure};
use gpui::{
    Bounds, ColorGlyphClassifier, ColorGlyphKind, FontId, ForegroundDependency, GlyphId,
    GlyphRasterizer, GlyphRenderMode, PreparedRasterStyle, RasterColorEffect, RasterFace,
    RasterStyleRequest, RasterizedGlyph, RasterizedGlyphFormat, RenderGlyphParams,
    SUBPIXEL_VARIANTS_X, SUBPIXEL_VARIANTS_Y, TextRenderingMode, point, size, swap_rgba_pa_to_bgra,
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
    CacheKey as SwashCacheKey, FontRef as SwashFontRef,
    scale::{
        Render, ScaleContext, Source, StrikeWith,
        image::{Content, Image},
    },
    zeno::{Angle, Format, Transform, Vector},
};

/// Swash raster state used by Linux, web, and explicit fallback construction.
#[derive(Default)]
pub struct SwashGlyphRasterizer {
    scale_context: ScaleContext,
    cache_keys: HashMap<FontId, SwashCacheKey>,
}

impl SwashGlyphRasterizer {
    fn render_glyph_image(
        &mut self,
        face: &RasterFace<'_>,
        params: &RenderGlyphParams,
    ) -> Result<Option<Image>> {
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

        let mut scaler = self
            .scale_context
            .builder(font_ref)
            .size(f32::from(params.font_size) * params.scale_factor)
            .normalized_coords(
                face.normalized_coords
                    .iter()
                    .map(|coordinate| coordinate.to_bits()),
            )
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

        if let RasterColorEffect::Preblend(color) = params.raster_style.color_effect {
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
            Content::Color => {
                let premultiplied = matches!(image.source, Source::ColorOutline(_));

                for pixel in image.data.chunks_exact_mut(4) {
                    if premultiplied {
                        swap_rgba_pa_to_bgra(pixel);
                    } else {
                        pixel.swap(0, 2);
                    }
                }

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
                        let alpha =
                            ((u16::from(coverage) * u16::from(foreground_alpha) + 127) / 255) as u8;

                        [blue, green, red, alpha]
                    })
                    .collect();

                (RasterizedGlyphFormat::BgraColor, pixels)
            }
            Content::Mask => (RasterizedGlyphFormat::AlphaMask, image.data),
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

    ensure!(
        glyph_id.0 < glyph_count,
        "glyph ID {} is outside the selected face's {glyph_count} glyphs",
        glyph_id.0
    );

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
    use gpui_fonts::{IBM_PLEX, IBM_PLEX_SEMIBOLD, NOTO_COLOR_EMOJI, SOURCE_SERIF};
    use gpui_parley::ParleyTextSystem;
    use gpui_parley::test_support::{
        OVERFLOW_CASES, OverflowParagraph, SCALE_FACTOR, first_group_color, logical_bounds,
        open_overflow_paragraph, quads, second_group_color, update_view,
    };
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

    fn layout_line(
        system: &ParleyTextSystem,
        text: &str,
        font_size: Pixels,
        family: &str,
    ) -> LineLayout {
        system.layout_text(TextLayoutRequest {
            text,
            font_size,
            runs: &[TextRun {
                len: text.len(),
                font: font(family),
                ..Default::default()
            }],
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

    fn raster_face(source: &FontDataBlob<u8>) -> RasterFace<'_> {
        let font = FontRef::new(source.as_ref()).unwrap();

        let has_color_glyphs = [*b"CBDT", *b"sbix", *b"COLR", *b"SVG "]
            .into_iter()
            .any(|tag| font.table_data(Tag::new(&tag)).is_some());

        RasterFace {
            font_id: FontId(1),
            source_id: source.id(),
            source,
            face_index: 0,
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
                ..raster_face(&source)
            };

            for character in characters.chars() {
                let glyph_id = GlyphId(font.charmap().map(character).unwrap().to_u32());
                let params = raster_params(face.font_id, glyph_id, mode);

                if character == '😀' {
                    assert_eq!(
                        face.supported_color_glyph_kind(glyph_id, |kind| rasterizer
                            .supports_color_glyph(kind))
                            .unwrap(),
                        Some(ColorGlyphKind::Cbdt)
                    );
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

    #[test]
    fn portable_rasterization_uses_parleys_normalized_coordinates() {
        let source = FontDataBlob::from(SOURCE_SERIF.data.to_vec());
        let font = FontRef::new(source.as_ref()).unwrap();
        let default_coords = font.axes().location([] as [(Tag, f32); 0]);

        let default_face = RasterFace {
            normalized_coords: default_coords.coords(),
            ..raster_face(&source)
        };

        let optical_coords = font.axes().location([(Tag::new(b"opsz"), 72.0)]);

        let optical_face = RasterFace {
            font_id: FontId(2),
            normalized_coords: optical_coords.coords(),
            ..default_face
        };

        let glyph_id = GlyphId(font.charmap().map('A').unwrap().to_u32());

        let mut rasterizer = SwashGlyphRasterizer::default();
        let mut render = |face: RasterFace<'_>| {
            let params = RenderGlyphParams {
                font_size: px(48.0),
                ..raster_params(face.font_id, glyph_id, Grayscale)
            };

            rasterizer.rasterize(face, &params).unwrap()
        };

        assert_ne!(render(default_face).pixels, render(optical_face).pixels);
    }

    #[test]
    fn portable_color_outline_fallback_returns_a_tintable_mask() {
        let source = FontDataBlob::from(SOURCE_SERIF.data.to_vec());
        let font = FontRef::new(source.as_ref()).unwrap();
        let face = raster_face(&source);
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
    fn empty_glyphs_and_invalid_ids_preserve_subsequent_rasterization() {
        let system = test_system();

        for (fixture, visible, mode, expected_format) in [
            (IBM_PLEX, 'A', Grayscale, AlphaMask),
            (SOURCE_SERIF, 'A', Grayscale, AlphaMask),
            (IBM_PLEX, 'A', Subpixel, BgraSubpixelMask),
            (NOTO_COLOR_EMOJI, '😀', Color, BgraColor),
        ] {
            let case = format!("{} {mode:?}, size=24, scale=1", fixture.family);
            let font_id = system.font_id(&font(fixture.family)).unwrap();
            let glyph_id = system.glyph_for_char(font_id, visible).unwrap();
            let params = raster_params(font_id, glyph_id, mode);
            let face_font = FontRef::new(fixture.data).unwrap();

            let space = GlyphId(face_font.charmap().map(' ').unwrap().to_u32());
            let empty = system
                .rasterize_glyph(&raster_params(font_id, space, mode))
                .unwrap();

            assert_eq!(empty.size, Size::default(), "{case}");
            assert_eq!(empty.format, expected_format, "{case}");
            assert!(empty.pixels.is_empty(), "{case}");

            let invalid = GlyphId(u32::from(face_font.maxp().unwrap().num_glyphs()));
            let error = system
                .rasterize_glyph(&raster_params(font_id, invalid, mode))
                .unwrap_err();

            assert!(
                format!("{error:#}").contains("outside the selected face"),
                "{case}: {error:#}"
            );

            let recovered = system
                .rasterize_glyph(&params)
                .unwrap_or_else(|error| panic!("{case}: {error}"));

            assert_visible_raster(&recovered, expected_format, &case);
        }
    }

    #[test]
    fn bundled_emoji_sample_keeps_font_metrics_and_device_scaling() {
        const SAMPLE: &str = "Color emoji: 😀 🎉 🚀 💡 🔥 ✨";
        const EMOJI: [char; 6] = ['😀', '🎉', '🚀', '💡', '🔥', '✨'];

        let system = test_system();
        let noto_id = system.font_id(&font("Noto Color Emoji")).unwrap();

        for font_size in [px(16.0), px(24.0), px(32.0)] {
            let layout = layout_line(&system, SAMPLE, font_size, IBM_PLEX.family);

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

                let mut scale_one_bounds: Option<[f32; 4]> = None;

                for scale_factor in [1.0, 1.5, 2.0] {
                    let raster = system
                        .rasterize_glyph(&RenderGlyphParams {
                            font_size: fragment.font_size,
                            scale_factor,
                            raster_style,
                            ..raster_params(fragment.font_id, glyph.id, Color)
                        })
                        .unwrap();

                    let case = format!("{character}, size={font_size:?}, scale={scale_factor}");
                    assert_visible_raster(&raster, BgraColor, &case);

                    let logical_bounds = [
                        raster.bounds.origin.x.0 as f32 / scale_factor,
                        raster.bounds.origin.y.0 as f32 / scale_factor,
                        raster.size.width.0 as f32 / scale_factor,
                        raster.size.height.0 as f32 / scale_factor,
                    ];

                    if let Some(reference) = scale_one_bounds {
                        for (actual, expected) in logical_bounds.into_iter().zip(reference) {
                            assert!(
                                (actual - expected).abs() <= 1.0,
                                "{character} changed logical raster bounds from {reference:?} to {logical_bounds:?} at scale {scale_factor}"
                            );
                        }
                    } else {
                        scale_one_bounds = Some(logical_bounds);
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
