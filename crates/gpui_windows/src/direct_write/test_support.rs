use std::borrow::Cow;

use anyhow::Context as _;
use gpui::{
    FontDataBlob, FontId, FontSynthesis, ForegroundDependency, GlyphId, GlyphRenderMode,
    GlyphRenderMode::Color, PlatformTextSystem, PreparedRasterStyle, RasterFace,
    RasterStyleRequest, RenderGlyphParams, Result, Rgba, font, point, px, rgba,
};
use windows::Win32::{Foundation::RECT, Graphics::Imaging::IWICImagingFactory};

use super::{DirectWriteGlyphRasterizer, rasterize_color_bitmap};

pub(super) fn oversized_color_bounds(
    wic: &IWICImagingFactory,
    style: PreparedRasterStyle,
) -> Result<RECT> {
    let mut bounds = RECT {
        right: 1,
        bottom: 1,
        ..Default::default()
    };

    rasterize_color_bitmap(wic, style, bounds, |target| {
        bounds.right = unsafe { target.GetMaximumBitmapSize() }
            .checked_add(1)
            .context("bitmap limit overflow")?
            .try_into()?;

        Ok(())
    })?;

    Ok(bounds)
}

pub(super) fn bitmap_test_params(system: &dyn PlatformTextSystem) -> Result<RenderGlyphParams> {
    system.add_fonts(vec![Cow::Borrowed(gpui_fonts::NOTO_COLOR_EMOJI.data)])?;
    let font_id = system.font_id(&font("Noto Color Emoji"))?;
    let glyph_id = system.glyph_for_char(font_id, '😀').unwrap();

    Ok(raster_params(
        system,
        font_id,
        glyph_id,
        Color,
        rgba(0xffffffff),
    ))
}

pub(super) fn raster_params(
    system: &dyn PlatformTextSystem,
    font_id: FontId,
    glyph_id: GlyphId,
    mode: GlyphRenderMode,
    scene_color: Rgba,
) -> RenderGlyphParams {
    RenderGlyphParams {
        font_id,
        glyph_id,
        font_size: px(24.0),
        subpixel_variant: point(0, 0),
        scale_factor: 1.0,
        raster_style: system.prepare_raster_style(RasterStyleRequest {
            font_id,
            glyph_id,
            scene_color,
            requested_mode: mode,
            foreground_dependency: ForegroundDependency::Full,
        }),
    }
}

pub(super) fn test_rasterizer(
    font_id: FontId,
    data: &[u8],
    face_index: u32,
) -> Result<DirectWriteGlyphRasterizer> {
    let source = FontDataBlob::from(data.to_vec());
    let mut rasterizer = DirectWriteGlyphRasterizer::new()?;

    rasterizer.load_face(RasterFace {
        font_id,
        source_id: source.id(),
        source: &source,
        face_index,
        normalized_coords: &[],
        variations: &[],
        synthesis: FontSynthesis::default(),
        has_color_glyphs: false,
    })?;

    Ok(rasterizer)
}
