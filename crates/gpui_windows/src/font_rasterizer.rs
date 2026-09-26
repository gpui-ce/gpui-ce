#[cfg(test)]
use gpui::{GlyphId, PlatformTextSystem, font, rgba};

#[cfg(test)]
use gpui_parley::{ParleyTextSystem, SystemFonts};

#[cfg(test)]
use std::borrow::Cow;

use anyhow::{Context as _, Result, bail, ensure};
use gpui::{
    Bounds, DevicePixels, GlyphRenderMode, PreparedRasterStyle, RasterColorEffect,
    RasterStyleRequest, RasterizedGlyph, RasterizedGlyphFormat, RenderGlyphParams, Rgba8,
    SUBPIXEL_VARIANTS_X, SUBPIXEL_VARIANTS_Y, TextRenderingMode, point, size,
};
use gpui_parley::{ColorGlyphKind, GlyphRasterizer, RasterFace, SwashGlyphRasterizer};
use std::{
    collections::HashMap,
    error::Error,
    ffi::{c_uint, c_void},
    fmt,
    mem::ManuallyDrop,
};
use windows::{
    Win32::{
        Foundation::RECT,
        Graphics::DirectWrite::*,
        UI::WindowsAndMessaging::{
            FE_FONTSMOOTHINGCLEARTYPE, SPI_GETFONTSMOOTHING, SPI_GETFONTSMOOTHINGTYPE,
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW,
        },
    },
    core::{BOOL, Interface},
};
use windows_numerics::Vector2;

/// Uses DirectWrite where its required interfaces are available and retains the old OS range
/// through the portable rasterizer otherwise.
pub(crate) struct WindowsGlyphRasterizer {
    backend: WindowsRasterBackend,
    system_subpixel_rendering: bool,
}

enum WindowsRasterBackend {
    DirectWrite {
        rasterizer: DirectWriteGlyphRasterizer,
        fallback: SwashGlyphRasterizer,
    },
    Swash(SwashGlyphRasterizer),
}

#[derive(Debug)]
enum NativeRasterUnsupported {
    VariableAxesOnLegacyDirectWrite,
    BitmapColorGlyph,
    ColrV1Glyph,
}

impl fmt::Display for NativeRasterUnsupported {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::VariableAxesOnLegacyDirectWrite => {
                formatter.write_str("this DirectWrite version cannot instantiate variable axes")
            }
            Self::BitmapColorGlyph => formatter
                .write_str("the DirectWrite layer rasterizer does not handle bitmap glyphs"),
            Self::ColrV1Glyph => {
                formatter.write_str("the DirectWrite layer rasterizer does not handle COLRv1")
            }
        }
    }
}

impl Error for NativeRasterUnsupported {}

impl WindowsGlyphRasterizer {
    pub(crate) fn new() -> Self {
        let backend = match DirectWriteGlyphRasterizer::new() {
            Ok(rasterizer) => WindowsRasterBackend::DirectWrite {
                rasterizer,
                fallback: SwashGlyphRasterizer::default(),
            },
            Err(error) => {
                log::warn!("DirectWrite rasterization is unavailable; using Swash: {error:#}");
                WindowsRasterBackend::Swash(SwashGlyphRasterizer::default())
            }
        };

        Self {
            backend,
            system_subpixel_rendering: get_system_subpixel_rendering(),
        }
    }
}

impl GlyphRasterizer for WindowsGlyphRasterizer {
    fn supports_color_glyph(&self, kind: ColorGlyphKind) -> bool {
        matches!(kind, ColorGlyphKind::ColrV0 | ColorGlyphKind::Bitmap)
    }

    fn prepare_style(&self, request: RasterStyleRequest) -> PreparedRasterStyle {
        match &self.backend {
            WindowsRasterBackend::DirectWrite { rasterizer, .. } => {
                rasterizer.prepare_style(request)
            }
            WindowsRasterBackend::Swash(rasterizer) => rasterizer.prepare_style(request),
        }
    }

    fn rasterize(
        &mut self,
        face: RasterFace<'_>,
        params: &RenderGlyphParams,
    ) -> Result<RasterizedGlyph> {
        match &mut self.backend {
            WindowsRasterBackend::DirectWrite {
                rasterizer,
                fallback,
            } => match rasterizer.rasterize(face, params) {
                Ok(glyph) => Ok(glyph),
                Err(error) if error.downcast_ref::<NativeRasterUnsupported>().is_some() => {
                    log::debug!("using Swash for an unsupported DirectWrite glyph: {error:#}");
                    fallback.rasterize(face, params)
                }
                Err(error) => Err(error),
            },
            WindowsRasterBackend::Swash(rasterizer) => rasterizer.rasterize(face, params),
        }
    }

    fn recommended_mode(&self) -> TextRenderingMode {
        if self.system_subpixel_rendering {
            TextRenderingMode::Subpixel
        } else {
            TextRenderingMode::Grayscale
        }
    }
}

/// DirectWrite rasterization for the exact face and instance selected by Parley.
pub(crate) struct DirectWriteGlyphRasterizer {
    factory: IDWriteFactory5,
    variable_factory: Option<IDWriteFactory6>,
    in_memory_loader: IDWriteInMemoryFontFileLoader,
    rendering_params: IDWriteRenderingParams,
    faces: HashMap<gpui::FontId, NativeFace>,
    color_rendering: ColorRenderingParams,
    system_subpixel_rendering: bool,
}

struct NativeFace {
    face: IDWriteFontFace3,
    _data: Box<[u8]>,
}

struct GlyphAnalysis {
    analysis: IDWriteGlyphRunAnalysis,
    bounds: RECT,
    texture_type: DWRITE_TEXTURE_TYPE,
}

struct ColorRenderingParams {
    gamma_ratios: [f32; 4],
    grayscale_enhanced_contrast: f32,
}

#[derive(Clone, Copy)]
struct LayerColor {
    red: f32,
    green: f32,
    blue: f32,
    alpha: f32,
}

impl DirectWriteGlyphRasterizer {
    pub(crate) fn new() -> Result<Self> {
        let factory: IDWriteFactory5 = unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED) }
            .context("creating the DirectWrite factory")?;
        let variable_factory = factory.cast().ok();
        let in_memory_loader = unsafe { factory.CreateInMemoryFontFileLoader() }
            .context("creating the DirectWrite in-memory font loader")?;
        unsafe { factory.RegisterFontFileLoader(&in_memory_loader) }
            .context("registering the DirectWrite in-memory font loader")?;
        let rendering_params = unsafe { factory.CreateRenderingParams() }
            .context("reading DirectWrite rendering parameters")?;
        let grayscale_rendering_params: IDWriteRenderingParams1 = rendering_params
            .cast()
            .context("reading DirectWrite grayscale rendering parameters")?;
        let color_rendering = ColorRenderingParams {
            gamma_ratios: gpui::get_gamma_correction_ratios(unsafe {
                grayscale_rendering_params.GetGamma()
            }),
            grayscale_enhanced_contrast: unsafe {
                grayscale_rendering_params.GetGrayscaleEnhancedContrast()
            },
        };

        Ok(Self {
            factory,
            variable_factory,
            in_memory_loader,
            rendering_params,
            faces: HashMap::default(),
            color_rendering,
            system_subpixel_rendering: get_system_subpixel_rendering(),
        })
    }

    fn native_face(&mut self, face: &RasterFace<'_>) -> Result<IDWriteFontFace3> {
        if !face.variations.is_empty() && self.variable_factory.is_none() {
            return Err(NativeRasterUnsupported::VariableAxesOnLegacyDirectWrite.into());
        }

        match self.faces.entry(face.font_id) {
            std::collections::hash_map::Entry::Occupied(entry) => Ok(entry.get().face.clone()),
            std::collections::hash_map::Entry::Vacant(entry) => {
                let native = NativeFace::new(
                    &self.factory,
                    self.variable_factory.as_ref(),
                    &self.in_memory_loader,
                    face,
                )
                .with_context(|| {
                    format!(
                        "DirectWrite could not create FontId {:?}, face index {}, variations {:?}",
                        face.font_id, face.face_index, face.variations
                    )
                })?;

                Ok(entry.insert(native).face.clone())
            }
        }
    }

    fn create_glyph_analysis(
        &self,
        font_face: &IDWriteFontFace3,
        params: &RenderGlyphParams,
        mode: GlyphRenderMode,
    ) -> Result<GlyphAnalysis> {
        let glyph_id =
            [u16::try_from(params.glyph_id.0).context("DirectWrite glyph IDs are 16-bit")?];
        let advances = [0.0];
        let offsets = [DWRITE_GLYPH_OFFSET::default()];
        let base_face: IDWriteFontFace = font_face.cast()?;
        let glyph_run = DWRITE_GLYPH_RUN {
            fontFace: ManuallyDrop::new(Some(unsafe { std::ptr::read(&base_face) })),
            fontEmSize: f32::from(params.font_size),
            glyphCount: 1,
            glyphIndices: glyph_id.as_ptr(),
            glyphAdvances: advances.as_ptr(),
            glyphOffsets: offsets.as_ptr(),
            isSideways: BOOL(0),
            bidiLevel: 0,
        };

        let transform = raster_transform(params.scale_factor);
        let baseline = baseline_origin(params);
        let mut rendering_mode = DWRITE_RENDERING_MODE1::default();
        let mut grid_fit_mode = DWRITE_GRID_FIT_MODE::default();
        unsafe {
            font_face.GetRecommendedRenderingMode(
                f32::from(params.font_size),
                96.0,
                96.0,
                Some(&transform),
                false,
                DWRITE_OUTLINE_THRESHOLD_ANTIALIASED,
                DWRITE_MEASURING_MODE_NATURAL,
                &self.rendering_params,
                &mut rendering_mode,
                &mut grid_fit_mode,
            )?;
        }

        if rendering_mode == DWRITE_RENDERING_MODE1_OUTLINE {
            rendering_mode = DWRITE_RENDERING_MODE1_NATURAL_SYMMETRIC;
        }

        let (antialias_mode, texture_type) = if mode == GlyphRenderMode::Subpixel {
            (
                DWRITE_TEXT_ANTIALIAS_MODE_CLEARTYPE,
                DWRITE_TEXTURE_CLEARTYPE_3x1,
            )
        } else {
            (
                DWRITE_TEXT_ANTIALIAS_MODE_GRAYSCALE,
                DWRITE_TEXTURE_ALIASED_1x1,
            )
        };

        let analysis = unsafe {
            self.factory.CreateGlyphRunAnalysis(
                &glyph_run,
                Some(&transform),
                rendering_mode,
                DWRITE_MEASURING_MODE_NATURAL,
                grid_fit_mode,
                antialias_mode,
                baseline.X,
                baseline.Y,
            )
        }?;

        let bounds = unsafe { analysis.GetAlphaTextureBounds(texture_type) }?;

        Ok(GlyphAnalysis {
            analysis,
            bounds,
            texture_type,
        })
    }

    fn rasterize_mask(
        &self,
        font_face: &IDWriteFontFace3,
        params: &RenderGlyphParams,
        mode: GlyphRenderMode,
    ) -> Result<RasterizedGlyph> {
        let glyph = self.create_glyph_analysis(font_face, params, mode)?;
        let Some((bounds, width, height)) = convert_bounds(glyph.bounds)? else {
            return Ok(RasterizedGlyph::empty(mode.rasterized_format()));
        };

        let pixel_count = width as usize * height as usize;

        if mode != GlyphRenderMode::Subpixel {
            let mut pixels = vec![0; pixel_count];
            unsafe {
                glyph.analysis.CreateAlphaTexture(
                    DWRITE_TEXTURE_ALIASED_1x1,
                    &glyph.bounds,
                    &mut pixels,
                )?;
            }

            return Ok(RasterizedGlyph {
                bounds,
                size: size(DevicePixels(width), DevicePixels(height)),
                format: RasterizedGlyphFormat::AlphaMask,
                pixels,
            });
        }

        let mut pixels = vec![0; pixel_count * 4];
        unsafe {
            glyph.analysis.CreateAlphaTexture(
                glyph.texture_type,
                &glyph.bounds,
                &mut pixels[..pixel_count * 3],
            )?;
        }

        for pixel_idx in (0..pixel_count).rev() {
            let source = pixel_idx * 3;
            let target = pixel_idx * 4;
            let red = pixels[source];
            let green = pixels[source + 1];
            let blue = pixels[source + 2];
            pixels[target..target + 4].copy_from_slice(&[blue, green, red, 0]);
        }

        Ok(RasterizedGlyph {
            bounds,
            size: size(DevicePixels(width), DevicePixels(height)),
            format: RasterizedGlyphFormat::BgraSubpixelMask,
            pixels,
        })
    }

    fn rasterize_colr(
        &self,
        font_face: &IDWriteFontFace3,
        params: &RenderGlyphParams,
    ) -> Result<RasterizedGlyph> {
        let current_color = prepared_color(params.raster_style)?;
        let glyph_id = [u16::try_from(params.glyph_id.0)?];
        let advances = [0.0];
        let offsets = [DWRITE_GLYPH_OFFSET::default()];
        let base_face: IDWriteFontFace = font_face.cast()?;
        let glyph_run = DWRITE_GLYPH_RUN {
            fontFace: ManuallyDrop::new(Some(unsafe { std::ptr::read(&base_face) })),
            fontEmSize: f32::from(params.font_size),
            glyphCount: 1,
            glyphIndices: glyph_id.as_ptr(),
            glyphAdvances: advances.as_ptr(),
            glyphOffsets: offsets.as_ptr(),
            isSideways: BOOL(0),
            bidiLevel: 0,
        };

        let transform = raster_transform(params.scale_factor);
        let baseline = baseline_origin(params);
        let enumerate = || unsafe {
            self.factory.TranslateColorGlyphRun(
                baseline,
                &glyph_run,
                None,
                DWRITE_GLYPH_IMAGE_FORMATS_COLR,
                DWRITE_MEASURING_MODE_NATURAL,
                Some(&transform),
                0,
            )
        };

        let enumerator = enumerate()?;
        let mut raster_bounds: Option<RECT> = None;
        while unsafe { enumerator.MoveNext() }?.as_bool() {
            let run = unsafe { &*enumerator.GetCurrentRun()? };

            if run.glyphImageFormat & DWRITE_GLYPH_IMAGE_FORMATS_COLR
                == DWRITE_GLYPH_IMAGE_FORMATS_NONE
            {
                continue;
            }

            let analysis = unsafe {
                self.factory.CreateGlyphRunAnalysis(
                    &run.Base.glyphRun,
                    Some(&transform),
                    DWRITE_RENDERING_MODE1_NATURAL_SYMMETRIC,
                    run.measuringMode,
                    DWRITE_GRID_FIT_MODE_DEFAULT,
                    DWRITE_TEXT_ANTIALIAS_MODE_GRAYSCALE,
                    run.Base.baselineOriginX,
                    run.Base.baselineOriginY,
                )
            }?;

            let layer_bounds =
                unsafe { analysis.GetAlphaTextureBounds(DWRITE_TEXTURE_ALIASED_1x1) }?;

            if convert_bounds(layer_bounds)?.is_none() {
                continue;
            }

            raster_bounds = Some(match raster_bounds {
                Some(bounds) => RECT {
                    left: bounds.left.min(layer_bounds.left),
                    top: bounds.top.min(layer_bounds.top),
                    right: bounds.right.max(layer_bounds.right),
                    bottom: bounds.bottom.max(layer_bounds.bottom),
                },
                None => layer_bounds,
            });
        }

        let Some(raster_bounds) = raster_bounds else {
            return Ok(RasterizedGlyph::empty(RasterizedGlyphFormat::BgraColor));
        };

        let Some((bounds, width, height)) = convert_bounds(raster_bounds)? else {
            unreachable!("color layer bounds were validated above");
        };

        let mut premultiplied = vec![[0.0f32; 4]; width as usize * height as usize];
        let enumerator = enumerate()?;
        while unsafe { enumerator.MoveNext() }?.as_bool() {
            let run = unsafe { &*enumerator.GetCurrentRun()? };

            if run.glyphImageFormat & DWRITE_GLYPH_IMAGE_FORMATS_COLR
                == DWRITE_GLYPH_IMAGE_FORMATS_NONE
            {
                continue;
            }

            let layer_analysis = unsafe {
                self.factory.CreateGlyphRunAnalysis(
                    &run.Base.glyphRun,
                    Some(&transform),
                    DWRITE_RENDERING_MODE1_NATURAL_SYMMETRIC,
                    run.measuringMode,
                    DWRITE_GRID_FIT_MODE_DEFAULT,
                    DWRITE_TEXT_ANTIALIAS_MODE_GRAYSCALE,
                    run.Base.baselineOriginX,
                    run.Base.baselineOriginY,
                )
            }?;

            let layer_bounds =
                unsafe { layer_analysis.GetAlphaTextureBounds(DWRITE_TEXTURE_ALIASED_1x1) }?;

            let Some((_, layer_width, layer_height)) = convert_bounds(layer_bounds)? else {
                continue;
            };

            let mut coverage = vec![0; layer_width as usize * layer_height as usize];
            unsafe {
                layer_analysis.CreateAlphaTexture(
                    DWRITE_TEXTURE_ALIASED_1x1,
                    &layer_bounds,
                    &mut coverage,
                )?;
            }

            let color = layer_color(run, current_color);
            for layer_y in 0..layer_height {
                let target_y = layer_bounds.top - raster_bounds.top + layer_y;

                if !(0..height).contains(&target_y) {
                    continue;
                }

                for layer_x in 0..layer_width {
                    let target_x = layer_bounds.left - raster_bounds.left + layer_x;

                    if !(0..width).contains(&target_x) {
                        continue;
                    }

                    let source_idx = (layer_y as usize * layer_width as usize) + layer_x as usize;
                    let target_idx = target_y as usize * width as usize + target_x as usize;
                    let corrected = corrected_coverage(
                        f32::from(coverage[source_idx]) / 255.0,
                        color,
                        &self.color_rendering,
                    );
                    composite_color(&mut premultiplied[target_idx], color, corrected);
                }
            }
        }

        let mut pixels = Vec::with_capacity(premultiplied.len() * 4);
        for pixel in premultiplied {
            let alpha = pixel[3].clamp(0.0, 1.0);

            if alpha == 0.0 {
                pixels.extend_from_slice(&[0, 0, 0, 0]);
                continue;
            }

            pixels.extend_from_slice(&[
                float_channel(pixel[2] / alpha),
                float_channel(pixel[1] / alpha),
                float_channel(pixel[0] / alpha),
                float_channel(alpha),
            ]);
        }

        Ok(RasterizedGlyph {
            bounds,
            size: size(DevicePixels(width), DevicePixels(height)),
            format: RasterizedGlyphFormat::BgraColor,
            pixels,
        })
    }

    fn rasterize_native_monochrome_color(
        &self,
        glyph: GlyphAnalysis,
        bounds: Bounds<DevicePixels>,
        width: i32,
        height: i32,
        color: Rgba8,
    ) -> Result<RasterizedGlyph> {
        let pixel_count = width as usize * height as usize;
        let mut coverage = vec![0; pixel_count];
        unsafe {
            glyph.analysis.CreateAlphaTexture(
                DWRITE_TEXTURE_ALIASED_1x1,
                &glyph.bounds,
                &mut coverage,
            )?;
        }

        let mut pixels = Vec::with_capacity(pixel_count * 4);
        for alpha in coverage {
            let alpha = multiply_u8(alpha, color.alpha);
            pixels.extend_from_slice(&[color.blue, color.green, color.red, alpha]);
        }

        Ok(RasterizedGlyph {
            bounds,
            size: size(DevicePixels(width), DevicePixels(height)),
            format: RasterizedGlyphFormat::BgraColor,
            pixels,
        })
    }
}

impl Drop for DirectWriteGlyphRasterizer {
    fn drop(&mut self) {
        self.faces.clear();
        unsafe {
            let _ = self
                .factory
                .UnregisterFontFileLoader(&self.in_memory_loader);
        }
    }
}

impl GlyphRasterizer for DirectWriteGlyphRasterizer {
    fn supports_color_glyph(&self, kind: ColorGlyphKind) -> bool {
        kind == ColorGlyphKind::ColrV0
    }

    fn prepare_style(&self, request: RasterStyleRequest) -> PreparedRasterStyle {
        if request.requested_mode == GlyphRenderMode::Color {
            PreparedRasterStyle {
                mode: GlyphRenderMode::Color,
                color_effect: RasterColorEffect::Preblend(request.scene_color.into()),
            }
        } else {
            PreparedRasterStyle::independent(request.requested_mode)
        }
    }

    fn rasterize(
        &mut self,
        face: RasterFace<'_>,
        params: &RenderGlyphParams,
    ) -> Result<RasterizedGlyph> {
        ensure!(
            params.scale_factor.is_finite() && params.scale_factor > 0.0,
            "invalid raster scale factor"
        );
        let color_kind = if params.raster_style.mode == GlyphRenderMode::Color {
            face.color_glyph_kind(params.glyph_id)?
        } else {
            None
        };

        if color_kind == Some(ColorGlyphKind::Bitmap) {
            return Err(NativeRasterUnsupported::BitmapColorGlyph.into());
        }

        if color_kind == Some(ColorGlyphKind::ColrV1) {
            return Err(NativeRasterUnsupported::ColrV1Glyph.into());
        }

        let font_face = self.native_face(&face)?;
        match color_kind {
            Some(ColorGlyphKind::ColrV0) => self.rasterize_colr(&font_face, params),
            Some(ColorGlyphKind::Svg) | None
                if params.raster_style.mode == GlyphRenderMode::Color =>
            {
                let color = prepared_color(params.raster_style)?;
                let glyph =
                    self.create_glyph_analysis(&font_face, params, GlyphRenderMode::Grayscale)?;
                let Some((bounds, width, height)) = convert_bounds(glyph.bounds)? else {
                    return Ok(RasterizedGlyph::empty(RasterizedGlyphFormat::BgraColor));
                };

                self.rasterize_native_monochrome_color(glyph, bounds, width, height, color)
            }
            _ => self.rasterize_mask(&font_face, params, params.raster_style.mode),
        }
    }

    fn recommended_mode(&self) -> TextRenderingMode {
        if self.system_subpixel_rendering {
            TextRenderingMode::Subpixel
        } else {
            TextRenderingMode::Grayscale
        }
    }
}

impl NativeFace {
    fn new(
        factory: &IDWriteFactory5,
        variable_factory: Option<&IDWriteFactory6>,
        loader: &IDWriteInMemoryFontFileLoader,
        face: &RasterFace<'_>,
    ) -> Result<Self> {
        let data: Box<[u8]> = face.data.into();
        let data_len = u32::try_from(data.len()).context("font data exceeds DirectWrite limits")?;
        let file = unsafe {
            loader.CreateInMemoryFontFileReference(
                factory,
                data.as_ptr().cast(),
                data_len,
                None::<&windows::core::IUnknown>,
            )
        }?;

        let mut simulations = DWRITE_FONT_SIMULATIONS_NONE;

        if face.synthesis.embolden {
            simulations |= DWRITE_FONT_SIMULATIONS_BOLD;
        }

        if face.synthesis.skew_degrees.is_some() {
            simulations |= DWRITE_FONT_SIMULATIONS_OBLIQUE;
        }

        let native_face = if face.variations.is_empty() {
            let reference =
                unsafe { factory.CreateFontFaceReference(&file, face.face_index, simulations) }?;

            unsafe { reference.CreateFontFace() }?
        } else {
            let variable_factory =
                variable_factory.ok_or(NativeRasterUnsupported::VariableAxesOnLegacyDirectWrite)?;
            let variations = face
                .variations
                .iter()
                .map(|variation| DWRITE_FONT_AXIS_VALUE {
                    axisTag: DWRITE_FONT_AXIS_TAG(u32::from_le_bytes(variation.tag.to_be_bytes())),
                    value: variation.value,
                })
                .collect::<Vec<_>>();
            let reference = unsafe {
                variable_factory.CreateFontFaceReference(
                    &file,
                    face.face_index,
                    simulations,
                    &variations,
                )
            }?;

            let variable_face = unsafe { reference.CreateFontFace() }?;

            variable_face.cast()?
        };

        Ok(Self {
            face: native_face,
            _data: data,
        })
    }
}

fn convert_bounds(bounds: RECT) -> Result<Option<(Bounds<DevicePixels>, i32, i32)>> {
    if bounds.right <= bounds.left || bounds.bottom <= bounds.top {
        return Ok(None);
    }

    let width = bounds
        .right
        .checked_sub(bounds.left)
        .context("DirectWrite glyph width overflow")?;
    let height = bounds
        .bottom
        .checked_sub(bounds.top)
        .context("DirectWrite glyph height overflow")?;
    Ok(Some((
        Bounds {
            origin: point(DevicePixels(bounds.left), DevicePixels(bounds.top)),
            size: size(DevicePixels(width), DevicePixels(height)),
        },
        width,
        height,
    )))
}

fn raster_transform(scale_factor: f32) -> DWRITE_MATRIX {
    DWRITE_MATRIX {
        m11: scale_factor,
        m12: 0.0,
        m21: 0.0,
        m22: scale_factor,
        dx: 0.0,
        dy: 0.0,
    }
}

fn baseline_origin(params: &RenderGlyphParams) -> Vector2 {
    Vector2::new(
        f32::from(params.subpixel_variant.x) / SUBPIXEL_VARIANTS_X as f32 / params.scale_factor,
        f32::from(params.subpixel_variant.y) / SUBPIXEL_VARIANTS_Y as f32 / params.scale_factor,
    )
}

fn prepared_color(style: PreparedRasterStyle) -> Result<Rgba8> {
    match style.color_effect {
        RasterColorEffect::Preblend(color) => Ok(color),
        _ => bail!("color glyph rasterization requires a prepared currentColor value"),
    }
}

fn layer_color(run: &DWRITE_COLOR_GLYPH_RUN1, current_color: Rgba8) -> LayerColor {
    if u32::from(run.Base.paletteIndex) == DWRITE_NO_PALETTE_INDEX {
        LayerColor {
            red: f32::from(current_color.red) / 255.0,
            green: f32::from(current_color.green) / 255.0,
            blue: f32::from(current_color.blue) / 255.0,
            alpha: f32::from(current_color.alpha) / 255.0,
        }
    } else {
        let color = run.Base.runColor;
        LayerColor {
            red: color.r,
            green: color.g,
            blue: color.b,
            alpha: color.a,
        }
    }
}

fn corrected_coverage(sample: f32, color: LayerColor, rendering: &ColorRenderingParams) -> f32 {
    let brightness = 0.30 * color.red + 0.59 * color.green + 0.11 * color.blue;
    let light_on_dark = (4.0 * (0.75 - brightness)).clamp(0.0, 1.0);
    let contrast = rendering.grayscale_enhanced_contrast * light_on_dark;
    let contrasted = sample * (contrast + 1.0) / (sample * contrast + 1.0);
    let ratios = rendering.gamma_ratios;
    let brightness_adjustment = ratios[0] * brightness + ratios[1];
    let correction = brightness_adjustment * contrasted + ratios[2] * brightness + ratios[3];
    (contrasted + contrasted * (1.0 - contrasted) * correction).clamp(0.0, 1.0)
}

fn composite_color(destination: &mut [f32; 4], color: LayerColor, coverage: f32) {
    let source_alpha = (coverage * color.alpha).clamp(0.0, 1.0);
    let inverse_alpha = 1.0 - source_alpha;
    destination[0] = color.red * source_alpha + destination[0] * inverse_alpha;
    destination[1] = color.green * source_alpha + destination[1] * inverse_alpha;
    destination[2] = color.blue * source_alpha + destination[2] * inverse_alpha;
    destination[3] = source_alpha + destination[3] * inverse_alpha;
}

fn float_channel(value: f32) -> u8 {
    (value * 255.0).round().clamp(0.0, 255.0) as u8
}

fn multiply_u8(left: u8, right: u8) -> u8 {
    ((u16::from(left) * u16::from(right) + 127) / 255) as u8
}

fn get_system_subpixel_rendering() -> bool {
    let mut smoothing_enabled = BOOL::default();
    let enabled_result = unsafe {
        SystemParametersInfoW(
            SPI_GETFONTSMOOTHING,
            0,
            Some((&mut smoothing_enabled as *mut BOOL).cast::<c_void>()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS::default(),
        )
    };

    let mut smoothing_type = c_uint::default();
    let type_result = unsafe {
        SystemParametersInfoW(
            SPI_GETFONTSMOOTHINGTYPE,
            0,
            Some((&mut smoothing_type as *mut c_uint).cast::<c_void>()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS::default(),
        )
    };

    enabled_result.is_ok()
        && type_result.is_ok()
        && smoothing_enabled.as_bool()
        && smoothing_type == FE_FONTSMOOTHINGCLEARTYPE
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOURCE_SERIF: &[u8] =
        include_bytes!("../../../assets/fonts/source-serif-4/SourceSerif4[opsz,wght].ttf");
    const NOTO_COLOR_EMOJI: &[u8] =
        include_bytes!("../../../assets/fonts/noto-color-emoji/NotoColorEmoji.subset.ttf");

    #[test]
    fn windows_rasterizer_covers_native_masks_current_color_and_color_fallbacks() {
        let system = ParleyTextSystem::new_with_rasterizer(
            SystemFonts::Skip,
            "Source Serif 4",
            WindowsGlyphRasterizer::new(),
        );
        system
            .add_fonts(vec![
                Cow::Borrowed(SOURCE_SERIF),
                Cow::Borrowed(NOTO_COLOR_EMOJI),
            ])
            .unwrap();
        let font_id = system
            .font_id(&font("Source Serif 4").bold().italic())
            .unwrap();

        let render = |font_id, glyph_id: GlyphId, mode, color, variant| {
            let raster_style = system.prepare_raster_style(RasterStyleRequest {
                scene_color: color,
                requested_mode: mode,
            });

            system
                .rasterize_glyph(&RenderGlyphParams {
                    font_id,
                    glyph_id,
                    font_size: gpui::px(24.0),
                    subpixel_variant: variant,
                    scale_factor: 2.0,
                    raster_style,
                })
                .unwrap()
        };

        let letter = system.glyph_for_char(font_id, 'A').unwrap();
        for (mode, format) in [
            (GlyphRenderMode::Grayscale, RasterizedGlyphFormat::AlphaMask),
            (
                GlyphRenderMode::Subpixel,
                RasterizedGlyphFormat::BgraSubpixelMask,
            ),
        ] {
            let raster = render(font_id, letter, mode, rgba(0x303030ff), point(3, 0));
            assert_eq!(raster.format, format);
            assert!(raster.bounds.origin.y.0 < 0);
            assert!(raster.size.width.0 > 0 && raster.size.height.0 > 0);
            raster.validate().unwrap();
        }

        let current_color = render(
            font_id,
            letter,
            GlyphRenderMode::Color,
            rgba(0xe02010cc),
            point(1, 0),
        );
        assert_eq!(current_color.format, RasterizedGlyphFormat::BgraColor);
        current_color.validate().unwrap();
        assert!(
            current_color
                .pixels
                .chunks_exact(4)
                .any(|pixel| { pixel[3] > 0 && pixel[2] > pixel[3] && pixel[2] > pixel[0] })
        );

        let space = render(
            font_id,
            system.glyph_for_char(font_id, ' ').unwrap(),
            GlyphRenderMode::Grayscale,
            rgba(0x000000ff),
            point(0, 0),
        );
        assert_eq!(space.size, gpui::Size::default());
        assert!(space.pixels.is_empty());

        let emoji_font = system.font_id(&font("Noto Color Emoji")).unwrap();
        let emoji = render(
            emoji_font,
            system.glyph_for_char(emoji_font, '😀').unwrap(),
            GlyphRenderMode::Color,
            rgba(0xffffffff),
            point(2, 0),
        );
        assert_eq!(emoji.format, RasterizedGlyphFormat::BgraColor);
        emoji.validate().unwrap();
        assert!(emoji.pixels.chunks_exact(4).any(|pixel| {
            pixel[3] > 128
                && (pixel[0].abs_diff(pixel[1]) > 20
                    || pixel[1].abs_diff(pixel[2]) > 20
                    || pixel[0].abs_diff(pixel[2]) > 20)
        }));
    }
}
