use std::{
    collections::hash_map::Entry,
    ffi::{c_uint, c_void},
    marker::PhantomData,
    mem::ManuallyDrop,
    rc::Rc,
    sync::Arc,
};

use anyhow::{Context as _, Result, bail, ensure};
use collections::HashMap;
use gpui::{
    Bounds, ColorGlyphKind, DevicePixels, FontDataBlob, FontId, GlyphRasterizer, GlyphRenderMode,
    Pixels, PlatformTextSystem, PreparedRasterStyle, RasterColorEffect, RasterFace,
    RasterStyleRequest, RasterizedGlyph, RenderGlyphParams, Rgba8, SUBPIXEL_VARIANTS_X,
    SUBPIXEL_VARIANTS_Y, TextRenderingMode, apply_pixel_opacity, multiply_alpha, point, size,
};
use gpui_parley::ParleyTextSystem;
use windows::{
    Win32::{
        Foundation::*,
        Graphics::{
            Direct2D::{Common::*, *},
            DirectWrite::*,
            Dxgi::Common::*,
            Gdi::UnionRect,
            Imaging::*,
        },
        System::Com::*,
        UI::WindowsAndMessaging::*,
    },
    core::*,
};
use windows_numerics::{Matrix3x2, Vector2};

#[cfg(test)]
use crate::DirectXDevices;

struct NativeFace {
    face: IDWriteFontFace4,
}

struct NativeSource {
    file: IDWriteFontFile,
}

#[windows_core::implement()]
struct FontDataOwner {
    _data: FontDataBlob<u8>,
}

pub(crate) fn new_text_system() -> Result<Arc<dyn PlatformTextSystem>> {
    Ok(Arc::new(
        ParleyTextSystem::new(DirectWriteGlyphRasterizer::new()?)
            .with_system_fallback_family("Segoe UI")
            .with_fallback_families(["Segoe UI", "Lilex", "IBM Plex Sans", "Arial"])
            .with_system_fonts(),
    ))
}

struct DirectWriteGlyphRasterizer {
    factory: IDWriteFactory5,
    in_memory_loader: IDWriteInMemoryFontFileLoader,
    variable_factory: Option<IDWriteFactory6>,
    rendering_params: IDWriteRenderingParams,
    color_rendering_params: IDWriteRenderingParams3,
    faces: HashMap<FontId, NativeFace>,
    sources: HashMap<u64, NativeSource>,
    system_subpixel_rendering: bool,
}

impl DirectWriteGlyphRasterizer {
    fn new() -> Result<Self> {
        let factory: IDWriteFactory5 = unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED) }
            .context("creating the DirectWrite factory")?;
        let variable_factory = factory.cast().ok();
        let in_memory_loader = unsafe { factory.CreateInMemoryFontFileLoader() }
            .context("creating the DirectWrite in-memory font loader")?;
        unsafe { factory.RegisterFontFileLoader(&in_memory_loader) }
            .context("registering the DirectWrite in-memory font loader")?;
        let rendering_params = unsafe { factory.CreateRenderingParams() }
            .context("reading DirectWrite rendering parameters")?;

        let grayscale_params: IDWriteRenderingParams1 = rendering_params.cast()?;

        // Use the same mode for layer bounds and drawing. Direct2D's default can switch to
        // outline rendering at large sizes.
        let color_rendering_params = unsafe {
            factory.CreateCustomRenderingParams(
                rendering_params.GetGamma(),
                rendering_params.GetEnhancedContrast(),
                grayscale_params.GetGrayscaleEnhancedContrast(),
                rendering_params.GetClearTypeLevel(),
                rendering_params.GetPixelGeometry(),
                DWRITE_RENDERING_MODE1_NATURAL_SYMMETRIC,
                DWRITE_GRID_FIT_MODE_DEFAULT,
            )
        }
        .context("creating DirectWrite color rendering parameters")?;

        Ok(Self {
            factory,
            in_memory_loader,
            variable_factory,
            rendering_params,
            color_rendering_params,
            faces: HashMap::default(),
            sources: HashMap::default(),
            system_subpixel_rendering: get_system_subpixel_rendering(),
        })
    }

    fn load_face(&mut self, face: RasterFace<'_>) -> Result<()> {
        let Entry::Vacant(entry) = self.faces.entry(face.font_id) else {
            return Ok(());
        };
        let factory = &self.factory;
        let loader = &self.in_memory_loader;
        let variable_factory = self.variable_factory.as_ref();
        let use_default_axes = face.variations.is_empty()
            || (variable_factory.is_none() && face.has_default_variations()?);

        ensure!(
            use_default_axes || variable_factory.is_some(),
            "this DirectWrite version cannot instantiate the selected variable-font coordinates"
        );

        let source = match self.sources.entry(face.source_id) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => entry.insert(
                NativeSource::new(factory, loader, face.source)
                    .context("DirectWrite could not retain the font source")?,
            ),
        };
        let native = NativeFace::new(
            factory,
            variable_factory,
            &source.file,
            &face,
            use_default_axes,
        )
        .with_context(|| {
            format!(
                "DirectWrite could not create FontId {:?}, face index {}, variations {:?}",
                face.font_id, face.face_index, face.variations
            )
        })?;
        entry.insert(native);

        Ok(())
    }

    fn create_glyph_run_analysis(
        &self,
        params: &RenderGlyphParams,
    ) -> Result<IDWriteGlyphRunAnalysis> {
        let font = &self.faces[&params.font_id];
        let glyph_id = [params.glyph_id.0 as u16];
        let advance = [0.0];
        let offset = [DWRITE_GLYPH_OFFSET::default()];
        let glyph_run = DWRITE_GLYPH_RUN {
            fontFace: ManuallyDrop::new(Some(unsafe { std::ptr::read(&****font.face) })),
            fontEmSize: params.font_size.as_f32(),
            glyphCount: 1,
            glyphIndices: glyph_id.as_ptr(),
            glyphAdvances: advance.as_ptr(),
            glyphOffsets: offset.as_ptr(),
            isSideways: BOOL(0),
            bidiLevel: 0,
        };
        let transform = DWRITE_MATRIX {
            m11: params.scale_factor,
            m12: 0.0,
            m21: 0.0,
            m22: params.scale_factor,
            dx: 0.0,
            dy: 0.0,
        };
        let baseline_origin_x =
            params.subpixel_variant.x as f32 / SUBPIXEL_VARIANTS_X as f32 / params.scale_factor;
        let baseline_origin_y = params.subpixel_variant.y as f32
            / gpui::SUBPIXEL_VARIANTS_Y as f32
            / params.scale_factor;

        let mut rendering_mode = DWRITE_RENDERING_MODE1::default();
        let mut grid_fit_mode = DWRITE_GRID_FIT_MODE::default();
        unsafe {
            font.face.GetRecommendedRenderingMode(
                params.font_size.as_f32(),
                // Using 96 as scale is applied by the transform
                96.0,
                96.0,
                Some(&transform),
                false,
                DWRITE_OUTLINE_THRESHOLD_ANTIALIASED,
                DWRITE_MEASURING_MODE_NATURAL,
                Some(&self.rendering_params),
                &mut rendering_mode,
                &mut grid_fit_mode,
            )?;
        }
        let rendering_mode = match rendering_mode {
            DWRITE_RENDERING_MODE1_OUTLINE => DWRITE_RENDERING_MODE1_NATURAL_SYMMETRIC,
            m => m,
        };

        let antialias_mode = if params.raster_style.mode == GlyphRenderMode::Subpixel {
            DWRITE_TEXT_ANTIALIAS_MODE_CLEARTYPE
        } else {
            DWRITE_TEXT_ANTIALIAS_MODE_GRAYSCALE
        };

        let glyph_analysis = unsafe {
            self.factory.CreateGlyphRunAnalysis(
                &glyph_run,
                Some(&transform),
                rendering_mode,
                DWRITE_MEASURING_MODE_NATURAL,
                grid_fit_mode,
                antialias_mode,
                baseline_origin_x,
                baseline_origin_y,
            )
        }?;
        Ok(glyph_analysis)
    }

    fn rasterize_monochrome(
        &self,
        glyph_analysis: &IDWriteGlyphRunAnalysis,
        texture_type: DWRITE_TEXTURE_TYPE,
        native_bounds: RECT,
    ) -> Result<Vec<u8>> {
        let width = (native_bounds.right - native_bounds.left) as usize;
        let height = (native_bounds.bottom - native_bounds.top) as usize;
        let pixel_count = width * height;
        let subpixel = texture_type == DWRITE_TEXTURE_CLEARTYPE_3x1;
        let native_channels = if subpixel { 3 } else { 1 };
        let output_channels = if subpixel { 4 } else { 1 };
        let mut bitmap_data = vec![0u8; pixel_count * output_channels];

        unsafe {
            glyph_analysis.CreateAlphaTexture(
                texture_type,
                &native_bounds,
                &mut bitmap_data[..pixel_count * native_channels],
            )?;
        }

        if !subpixel {
            return Ok(bitmap_data);
        }

        // The output buffer expects RGBA data, so pad the alpha channel with zeros.
        for pixel_ix in (0..pixel_count).rev() {
            let src = pixel_ix * 3;
            let dst = pixel_ix * 4;
            (
                bitmap_data[dst + 2],
                bitmap_data[dst + 1],
                bitmap_data[dst],
                bitmap_data[dst + 3],
            ) = (
                bitmap_data[src],
                bitmap_data[src + 1],
                bitmap_data[src + 2],
                0,
            );
        }

        Ok(bitmap_data)
    }

    fn rasterize_glyph(
        &self,
        params: &RenderGlyphParams,
        glyph_analysis: &IDWriteGlyphRunAnalysis,
        texture_type: DWRITE_TEXTURE_TYPE,
        native_bounds: RECT,
    ) -> Result<RasterizedGlyph> {
        let glyph_bounds = if native_bounds.right <= native_bounds.left
            || native_bounds.bottom <= native_bounds.top
        {
            Bounds::default()
        } else {
            Bounds {
                origin: point(native_bounds.left.into(), native_bounds.top.into()),
                size: size(
                    (native_bounds.right - native_bounds.left).into(),
                    (native_bounds.bottom - native_bounds.top).into(),
                ),
            }
        };

        if params.raster_style.mode == GlyphRenderMode::Color {
            if let Ok(color) = self.rasterize_color(params, glyph_bounds) {
                return Ok(color);
            }
        }

        let format = params.raster_style.mode.rasterized_format();

        if glyph_bounds.size.width.0 == 0 || glyph_bounds.size.height.0 == 0 {
            return Ok(RasterizedGlyph::empty(format));
        }

        let mut pixels = self.rasterize_monochrome(glyph_analysis, texture_type, native_bounds)?;

        if params.raster_style.mode == GlyphRenderMode::Color {
            let foreground = raster_foreground(params.raster_style);

            if foreground.alpha == 0 {
                return Ok(RasterizedGlyph::empty(format));
            }

            pixels = pixels
                .into_iter()
                .flat_map(|coverage| {
                    let alpha = multiply_alpha(coverage, foreground.alpha);

                    if alpha == 0 {
                        [0; 4]
                    } else {
                        [foreground.blue, foreground.green, foreground.red, alpha]
                    }
                })
                .collect();
        }

        let rasterized = RasterizedGlyph {
            bounds: glyph_bounds,
            size: glyph_bounds.size,
            format,
            pixels,
        };
        rasterized.validate()?;

        Ok(rasterized)
    }

    fn translate_color_glyph(
        &self,
        params: &RenderGlyphParams,
    ) -> Result<IDWriteColorGlyphRunEnumerator1> {
        let font = &self.faces[&params.font_id];
        let glyph_id = [params.glyph_id.0 as u16];
        let advance = [0.0];
        let offset = [DWRITE_GLYPH_OFFSET::default()];
        let glyph_run = DWRITE_GLYPH_RUN {
            fontFace: ManuallyDrop::new(Some(unsafe { std::ptr::read(&****font.face) })),
            fontEmSize: params.font_size.as_f32(),
            glyphCount: 1,
            glyphIndices: glyph_id.as_ptr(),
            glyphAdvances: advance.as_ptr(),
            glyphOffsets: offset.as_ptr(),
            isSideways: BOOL(0),
            bidiLevel: 0,
        };
        let transform = color_glyph_transform(params.scale_factor);
        let baseline = Vector2::new(
            params.subpixel_variant.x as f32 / SUBPIXEL_VARIANTS_X as f32 / params.scale_factor,
            params.subpixel_variant.y as f32 / SUBPIXEL_VARIANTS_Y as f32 / params.scale_factor,
        );

        // Request COLRv0 translation into ordinary outline runs, including foreground layers.
        Ok(unsafe {
            self.factory.TranslateColorGlyphRun(
                baseline,
                &glyph_run,
                None,
                DWRITE_GLYPH_IMAGE_FORMATS_COLR
                    | DWRITE_GLYPH_IMAGE_FORMATS_TRUETYPE
                    | DWRITE_GLYPH_IMAGE_FORMATS_CFF,
                DWRITE_MEASURING_MODE_NATURAL,
                Some(&transform),
                0,
            )
        }?)
    }

    fn rasterize_color(
        &self,
        params: &RenderGlyphParams,
        glyph_bounds: Bounds<DevicePixels>,
    ) -> Result<RasterizedGlyph> {
        // Declare the apartment first so every local COM resource drops before it.
        let _apartment = ColorRasterApartment::new()?;
        let enumerator = self.translate_color_glyph(params)?;
        let transform = color_glyph_transform(params.scale_factor);
        let mut ink_bounds = RECT {
            left: glyph_bounds.origin.x.0,
            top: glyph_bounds.origin.y.0,
            right: glyph_bounds.origin.x.0 + glyph_bounds.size.width.0,
            bottom: glyph_bounds.origin.y.0 + glyph_bounds.size.height.0,
        };

        while unsafe { enumerator.MoveNext()? }.as_bool() {
            let color_run = unsafe { &*enumerator.GetCurrentRun()? };
            ensure!(
                (color_run.glyphImageFormat
                    & (DWRITE_GLYPH_IMAGE_FORMATS_TRUETYPE | DWRITE_GLYPH_IMAGE_FORMATS_CFF))
                    .0
                    != 0,
                "DirectWrite returned an unsupported color layer format"
            );
            let layer = &color_run.Base;
            let analysis = unsafe {
                self.factory.CreateGlyphRunAnalysis(
                    &layer.glyphRun,
                    Some(&transform),
                    self.color_rendering_params.GetRenderingMode1(),
                    DWRITE_MEASURING_MODE_NATURAL,
                    self.color_rendering_params.GetGridFitMode(),
                    DWRITE_TEXT_ANTIALIAS_MODE_GRAYSCALE,
                    layer.baselineOriginX,
                    layer.baselineOriginY,
                )?
            };
            let layer_bounds =
                unsafe { analysis.GetAlphaTextureBounds(DWRITE_TEXTURE_ALIASED_1x1)? };

            let previous_bounds = ink_bounds;

            let _nonempty = unsafe { UnionRect(&mut ink_bounds, &previous_bounds, &layer_bounds) };
        }

        let format = params.raster_style.mode.rasterized_format();

        if ink_bounds.right <= ink_bounds.left || ink_bounds.bottom <= ink_bounds.top {
            return Ok(RasterizedGlyph::empty(format));
        }

        let wic: IWICImagingFactory =
            unsafe { CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)? };
        let foreground = raster_foreground(params.raster_style);

        rasterize_color_bitmap(&wic, params.raster_style, ink_bounds, |target| {
            let brush = unsafe { target.CreateSolidColorBrush(&D2D1_COLOR_F::default(), None)? };

            // MoveNext invalidates native layer pointers, so enumerate again for drawing.
            let enumerator = self.translate_color_glyph(params)?;

            unsafe {
                target.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
                target.SetTextRenderingParams(&self.color_rendering_params);

                target.SetTransform(
                    &(Matrix3x2::scale(params.scale_factor, params.scale_factor)
                        * Matrix3x2::translation(
                            -(ink_bounds.left as f32),
                            -(ink_bounds.top as f32),
                        )),
                );
            }

            while unsafe { enumerator.MoveNext()? }.as_bool() {
                let color_run = unsafe { &*enumerator.GetCurrentRun()? };
                let layer = &color_run.Base;
                let color = if layer.paletteIndex == 0xffff {
                    D2D1_COLOR_F {
                        r: foreground.red as f32 / 255.0,
                        g: foreground.green as f32 / 255.0,
                        b: foreground.blue as f32 / 255.0,
                        a: 1.0,
                    }
                } else {
                    D2D1_COLOR_F {
                        r: layer.runColor.r,
                        g: layer.runColor.g,
                        b: layer.runColor.b,
                        a: layer.runColor.a,
                    }
                };

                unsafe {
                    brush.SetColor(&color);
                    target.DrawGlyphRun(
                        Vector2::new(layer.baselineOriginX, layer.baselineOriginY),
                        &layer.glyphRun,
                        &brush,
                        DWRITE_MEASURING_MODE_NATURAL,
                    );
                }
            }

            Ok(())
        })
    }

    fn rasterize_bitmap_glyph(&self, params: &RenderGlyphParams) -> Result<RasterizedGlyph> {
        self.with_bitmap_glyph(params, |wic, data, bitmap| {
            let destination = bitmap_glyph_rect(params, data)?;
            let ink_bounds = bitmap_ink_bounds(destination)?;

            draw_bitmap_glyph(wic, params, bitmap, destination, ink_bounds)
        })
    }

    fn with_bitmap_glyph(
        &self,
        params: &RenderGlyphParams,
        draw: impl FnOnce(
            &IWICImagingFactory,
            &DWRITE_GLYPH_IMAGE_DATA,
            &IWICBitmap,
        ) -> Result<RasterizedGlyph>,
    ) -> Result<RasterizedGlyph> {
        let _apartment = ColorRasterApartment::new()?;
        let face = &self.faces[&params.font_id].face;
        let glyph_id = u16::try_from(params.glyph_id.0).context("invalid bitmap glyph ID")?;

        ensure!(
            params.font_size.as_f32().is_finite() && params.font_size.as_f32() > 0.0,
            "invalid bitmap glyph font size"
        );

        ensure!(
            params.scale_factor.is_finite() && params.scale_factor > 0.0,
            "invalid bitmap glyph scale factor"
        );

        let pixels_per_em = params.font_size.as_f32() as f64 * params.scale_factor as f64;

        ensure!(
            pixels_per_em.is_finite() && pixels_per_em > 0.0 && pixels_per_em <= u32::MAX as f64,
            "invalid bitmap glyph size"
        );

        let (image, image_format) =
            load_bitmap_image(face, glyph_id, pixels_per_em.round().max(1.0) as u32)?;

        let wic: IWICImagingFactory =
            unsafe { CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)? };

        let bitmap = decode_bitmap_glyph(&wic, &image.data, image.bytes()?, image_format)?;

        draw(&wic, &image.data, &bitmap)
    }
}

impl Drop for DirectWriteGlyphRasterizer {
    fn drop(&mut self) {
        self.faces.clear();
        self.sources.clear();

        unsafe {
            let _ = self
                .factory
                .UnregisterFontFileLoader(&self.in_memory_loader);
        }
    }
}

impl GlyphRasterizer for DirectWriteGlyphRasterizer {
    fn supports_color_glyph(&self, kind: ColorGlyphKind) -> bool {
        matches!(kind, ColorGlyphKind::ColrV0 | ColorGlyphKind::Cbdt)
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
        ensure!(
            params.font_size.as_f32().is_finite() && params.font_size.as_f32() >= 0.0,
            "invalid raster font size"
        );

        ensure!(
            params.scale_factor.is_finite() && params.scale_factor > 0.0,
            "invalid raster scale factor"
        );

        let format = params.raster_style.mode.rasterized_format();

        if params.font_size == Pixels::ZERO {
            return Ok(RasterizedGlyph::empty(format));
        }

        let is_cbdt = self.requested_color_glyph_kind(&face, params)? == Some(ColorGlyphKind::Cbdt);

        self.load_face(face)?;

        ensure!(
            params.glyph_id.0 < unsafe { self.faces[&params.font_id].face.GetGlyphCount() } as u32,
            "invalid glyph ID"
        );

        debug_assert!(
            !matches!(params.raster_style.color_effect, RasterColorEffect::Dilation(value) if value != 0)
        );

        if is_cbdt {
            return self.rasterize_bitmap_glyph(params);
        }

        let glyph_analysis = self.create_glyph_run_analysis(params)?;
        let texture_type = if params.raster_style.mode == GlyphRenderMode::Subpixel {
            DWRITE_TEXTURE_CLEARTYPE_3x1
        } else {
            DWRITE_TEXTURE_ALIASED_1x1
        };
        let native_bounds = unsafe { glyph_analysis.GetAlphaTextureBounds(texture_type)? };

        self.rasterize_glyph(params, &glyph_analysis, texture_type, native_bounds)
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
        file: &IDWriteFontFile,
        face: &RasterFace<'_>,
        use_default_axes: bool,
    ) -> Result<Self> {
        let mut simulations = DWRITE_FONT_SIMULATIONS_NONE;

        if face.synthesis.embolden {
            simulations |= DWRITE_FONT_SIMULATIONS_BOLD;
        }

        if face.synthesis.skew_degrees.is_some() {
            simulations |= DWRITE_FONT_SIMULATIONS_OBLIQUE;
        }

        let native_face: IDWriteFontFace3 = if use_default_axes {
            let reference =
                unsafe { factory.CreateFontFaceReference(file, face.face_index, simulations) }?;

            unsafe { reference.CreateFontFace() }?
        } else {
            let variable_factory = variable_factory
                .context("this DirectWrite version cannot instantiate variable-font coordinates")?;
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
                    file,
                    face.face_index,
                    simulations,
                    &variations,
                )
            }?;
            let variable_face = unsafe { reference.CreateFontFace() }?;

            variable_face.cast()?
        };

        Ok(Self {
            face: native_face.cast()?,
        })
    }
}

impl NativeSource {
    fn new(
        factory: &IDWriteFactory5,
        loader: &IDWriteInMemoryFontFileLoader,
        source: &FontDataBlob<u8>,
    ) -> Result<Self> {
        let bytes = source.as_ref();
        let data_len =
            u32::try_from(bytes.len()).context("font data exceeds DirectWrite limits")?;
        let owner: windows::core::IUnknown = FontDataOwner {
            _data: source.clone(),
        }
        .into();
        let file = unsafe {
            loader.CreateInMemoryFontFileReference(factory, bytes.as_ptr().cast(), data_len, &owner)
        }?;

        Ok(Self { file })
    }
}

struct GlyphImageData<'a> {
    face: &'a IDWriteFontFace4,
    data: DWRITE_GLYPH_IMAGE_DATA,
    context: Option<*mut c_void>,
}

impl<'a> GlyphImageData<'a> {
    fn new(
        face: &'a IDWriteFontFace4,
        glyph_id: u16,
        pixels_per_em: u32,
        image_format: DWRITE_GLYPH_IMAGE_FORMATS,
    ) -> Result<Option<Self>> {
        let mut data = DWRITE_GLYPH_IMAGE_DATA::default();
        let mut context = std::ptr::null_mut();

        unsafe {
            face.GetGlyphImageData(
                glyph_id,
                pixels_per_em,
                image_format,
                &mut data,
                &mut context,
            )
        }
        .context("retrieving DirectWrite bitmap glyph data")?;

        // Empty results have no mapping. Retain ownership independently of the image metadata.
        let image = Self {
            face,
            data,
            context: (data.imageDataSize != 0).then_some(context),
        };

        if image.data.imageDataSize == 0 {
            return Ok(None);
        }

        Ok(Some(image))
    }

    fn bytes(&self) -> Result<&[u8]> {
        ensure!(
            !self.data.imageData.is_null()
                && self.data.imageDataSize > 0
                && self.data.imageDataSize as u64 <= isize::MAX as u64,
            "DirectWrite returned an invalid bitmap glyph buffer"
        );

        // The guard borrows the face and releases the mapping only after this slice expires.
        Ok(unsafe {
            std::slice::from_raw_parts(self.data.imageData.cast(), self.data.imageDataSize as usize)
        })
    }
}

impl Drop for GlyphImageData<'_> {
    fn drop(&mut self) {
        if let Some(context) = self.context {
            unsafe { self.face.ReleaseGlyphImageData(context) };
        }
    }
}

fn load_bitmap_image(
    face: &IDWriteFontFace4,
    glyph_id: u16,
    pixels_per_em: u32,
) -> Result<(GlyphImageData<'_>, DWRITE_GLYPH_IMAGE_FORMATS)> {
    ensure!(
        glyph_id < unsafe { face.GetGlyphCount() },
        "invalid bitmap glyph ID"
    );

    ensure!(pixels_per_em > 0, "invalid bitmap glyph pixels per em");

    // Search every strike. A bitmap need not exist at the requested pixels per em.
    let formats = unsafe { face.GetGlyphImageFormats(glyph_id, 0, u32::MAX) }
        .context("querying DirectWrite bitmap glyph formats")?;

    for image_format in [
        DWRITE_GLYPH_IMAGE_FORMATS_PNG,
        DWRITE_GLYPH_IMAGE_FORMATS_PREMULTIPLIED_B8G8R8A8,
    ] {
        if (formats & image_format).0 == 0 {
            continue;
        }

        let Some(image) = GlyphImageData::new(face, glyph_id, pixels_per_em, image_format)? else {
            continue;
        };

        return Ok((image, image_format));
    }

    bail!("DirectWrite returned no image data for the selected CBDT artwork")
}

fn rasterize_color_bitmap(
    wic: &IWICImagingFactory,
    style: PreparedRasterStyle,
    bounds: RECT,
    draw: impl FnOnce(&ID2D1RenderTarget) -> Result<()>,
) -> Result<RasterizedGlyph> {
    let width = bounds
        .right
        .checked_sub(bounds.left)
        .context("color glyph width overflow")?;

    let height = bounds
        .bottom
        .checked_sub(bounds.top)
        .context("color glyph height overflow")?;

    color_bitmap_byte_count(width, height)?;

    let bitmap = unsafe {
        wic.CreateBitmap(
            width as u32,
            height as u32,
            &GUID_WICPixelFormat32bppPBGRA,
            WICBitmapCacheOnLoad,
        )?
    };

    let factory: ID2D1Factory =
        unsafe { D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)? };

    let properties = D2D1_RENDER_TARGET_PROPERTIES {
        r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
        pixelFormat: D2D1_PIXEL_FORMAT {
            format: DXGI_FORMAT_B8G8R8A8_UNORM,
            alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
        },
        dpiX: 96.0,
        dpiY: 96.0,
        ..Default::default()
    };

    let target = unsafe { factory.CreateWicBitmapRenderTarget(&bitmap, &properties)? };

    unsafe {
        target.SetTransform(&Matrix3x2::translation(
            -(bounds.left as f32),
            -(bounds.top as f32),
        ));

        target.BeginDraw();
        target.Clear(Some(&D2D1_COLOR_F::default()));
    }

    let draw_result = draw(&target);
    let end_result = unsafe { target.EndDraw(None, None) };
    draw_result?;
    end_result.context("drawing Direct2D color glyph")?;
    drop(target);

    let pixels = read_color_bitmap(&bitmap, width, height, raster_foreground(style).alpha)?;
    let format = style.mode.rasterized_format();

    if pixels.chunks_exact(4).all(|pixel| pixel[3] == 0) {
        return Ok(RasterizedGlyph::empty(format));
    }

    let bitmap_size = size(width.into(), height.into());

    let rasterized = RasterizedGlyph {
        bounds: Bounds {
            origin: point(bounds.left.into(), bounds.top.into()),
            size: bitmap_size,
        },
        size: bitmap_size,
        format,
        pixels,
    };

    rasterized.validate()?;

    Ok(rasterized)
}

fn color_bitmap_byte_count(width: i32, height: i32) -> Result<usize> {
    ensure!(width > 0 && height > 0, "color bitmap dimensions are empty");

    let byte_count = (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(4))
        .context("color glyph byte count overflow")?;

    ensure!(
        u32::try_from(byte_count).is_ok() && byte_count <= isize::MAX as usize,
        "color glyph exceeds WIC's buffer limit"
    );

    Ok(byte_count)
}

fn decode_bitmap_glyph(
    wic: &IWICImagingFactory,
    data: &DWRITE_GLYPH_IMAGE_DATA,
    bytes: &[u8],
    image_format: DWRITE_GLYPH_IMAGE_FORMATS,
) -> Result<IWICBitmap> {
    let width = i32::try_from(data.pixelSize.width).context("bitmap glyph width overflow")?;
    let height = i32::try_from(data.pixelSize.height).context("bitmap glyph height overflow")?;
    let byte_count = color_bitmap_byte_count(width, height)?;

    if image_format == DWRITE_GLYPH_IMAGE_FORMATS_PREMULTIPLIED_B8G8R8A8 {
        ensure!(
            bytes.len() == byte_count,
            "invalid BGRA bitmap glyph buffer size"
        );

        return Ok(unsafe {
            wic.CreateBitmapFromMemory(
                width as u32,
                height as u32,
                &GUID_WICPixelFormat32bppPBGRA,
                width as u32 * 4,
                bytes,
            )?
        });
    }

    ensure!(
        image_format == DWRITE_GLYPH_IMAGE_FORMATS_PNG,
        "unsupported bitmap glyph format"
    );

    ensure!(
        u32::try_from(bytes.len()).is_ok(),
        "PNG bitmap glyph exceeds WIC's buffer limit"
    );

    let stream = unsafe { wic.CreateStream()? };
    unsafe { stream.InitializeFromMemory(bytes)? };

    let decoder = unsafe {
        wic.CreateDecoderFromStream(&stream, std::ptr::null(), WICDecodeMetadataCacheOnLoad)
    }
    .context("decoding PNG bitmap glyph")?;

    let frame = unsafe { decoder.GetFrame(0)? };
    let mut decoded_width = 0;
    let mut decoded_height = 0;
    unsafe { frame.GetSize(&mut decoded_width, &mut decoded_height)? };

    ensure!(
        decoded_width == data.pixelSize.width && decoded_height == data.pixelSize.height,
        "PNG bitmap glyph dimensions differ from DirectWrite image data"
    );

    let converter = unsafe { WICConvertBitmapSource(&GUID_WICPixelFormat32bppPBGRA, &frame)? };

    // Materialize the pixels before the stream's borrowed input can expire.
    Ok(unsafe { wic.CreateBitmapFromSource(&converter, WICBitmapCacheOnLoad)? })
}

fn bitmap_glyph_rect(
    params: &RenderGlyphParams,
    data: &DWRITE_GLYPH_IMAGE_DATA,
) -> Result<D2D_RECT_F> {
    ensure!(data.pixelsPerEm > 0, "bitmap glyph has no strike size");

    let scale =
        params.font_size.as_f32() as f64 * params.scale_factor as f64 / data.pixelsPerEm as f64;

    let left = params.subpixel_variant.x as f64 / SUBPIXEL_VARIANTS_X as f64
        - data.horizontalLeftOrigin.x as f64 * scale;

    let top = params.subpixel_variant.y as f64 / SUBPIXEL_VARIANTS_Y as f64
        - data.horizontalLeftOrigin.y as f64 * scale;

    ensure!(
        scale.is_finite() && scale > 0.0,
        "invalid bitmap glyph scale"
    );

    Ok(D2D_RECT_F {
        left: left as f32,
        top: top as f32,
        right: (left + data.pixelSize.width as f64 * scale) as f32,
        bottom: (top + data.pixelSize.height as f64 * scale) as f32,
    })
}

fn bitmap_ink_bounds(rect: D2D_RECT_F) -> Result<RECT> {
    let edges = [
        rect.left.floor(),
        rect.top.floor(),
        rect.right.ceil(),
        rect.bottom.ceil(),
    ];

    ensure!(
        edges.iter().all(|edge| edge.is_finite()
            && *edge as f64 >= i32::MIN as f64
            && (*edge as f64) <= i32::MAX as f64),
        "bitmap glyph bounds overflow"
    );

    Ok(RECT {
        left: edges[0] as i32,
        top: edges[1] as i32,
        right: edges[2] as i32,
        bottom: edges[3] as i32,
    })
}

fn draw_bitmap_glyph(
    wic: &IWICImagingFactory,
    params: &RenderGlyphParams,
    bitmap: &IWICBitmap,
    destination: D2D_RECT_F,
    ink_bounds: RECT,
) -> Result<RasterizedGlyph> {
    rasterize_color_bitmap(wic, params.raster_style, ink_bounds, |target| {
        let image = unsafe { target.CreateBitmapFromWicBitmap(bitmap, None)? };

        unsafe {
            target.DrawBitmap(
                &image,
                Some(&destination),
                1.0,
                D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
                None,
            );
        }

        Ok(())
    })
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

struct ColorRasterApartment {
    initialized: bool,
    _thread: PhantomData<Rc<()>>,
}

impl ColorRasterApartment {
    fn new() -> Result<Self> {
        let status = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        let initialized = status != RPC_E_CHANGED_MODE;

        if initialized {
            // S_OK and S_FALSE both acquire a reference that this guard must release.
            status
                .ok()
                .context("initializing COM for color rasterization")?;
        }

        Ok(Self {
            initialized,
            _thread: PhantomData,
        })
    }
}

impl Drop for ColorRasterApartment {
    fn drop(&mut self) {
        if self.initialized {
            unsafe { CoUninitialize() };
        }
    }
}

fn color_glyph_transform(scale_factor: f32) -> DWRITE_MATRIX {
    DWRITE_MATRIX {
        m11: scale_factor,
        m12: 0.0,
        m21: 0.0,
        m22: scale_factor,
        dx: 0.0,
        dy: 0.0,
    }
}

fn raster_foreground(style: PreparedRasterStyle) -> Rgba8 {
    match style.color_effect {
        RasterColorEffect::Preblend(color) => color,
        _ => Rgba8::new(0, 0, 0, 255),
    }
}

fn read_color_bitmap(bitmap: &IWICBitmap, width: i32, height: i32, alpha: u8) -> Result<Vec<u8>> {
    let mut pixels = vec![0; color_bitmap_byte_count(width, height)?];

    unsafe {
        bitmap.CopyPixels(
            &WICRect {
                X: 0,
                Y: 0,
                Width: width,
                Height: height,
            },
            width as u32 * 4,
            &mut pixels,
        )?;
    }

    // Scaling premultiplied RGB and alpha together leaves straight RGB unchanged.
    // Retain the original alpha for division to avoid quantizing the scaled RGB twice.
    for pixel in pixels.chunks_exact_mut(4) {
        let coverage = pixel[3] as u16;
        apply_pixel_opacity(pixel, alpha);

        if pixel[3] == 0 {
            continue;
        }

        for channel in &mut pixel[..3] {
            *channel = ((*channel as u16 * 255 + coverage / 2) / coverage).min(255) as u8;
        }
    }

    Ok(pixels)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{
        Font, FontStyle, FontSynthesis, FontWeight, FontWidth, ForegroundDependency, GlyphId,
        GlyphRenderMode::{Color, Grayscale, Subpixel},
        Point, RasterizedGlyphFormat, Rgba, Size, TextLayoutRequest, TextRun, font, px, rgba,
    };
    use gpui_fonts::test_collection;
    use std::{borrow::Cow, collections::BTreeMap};

    #[test]
    fn fixed_fonts_cover_native_modes_instances_and_empty_glyphs() -> Result<()> {
        let system = new_text_system()?;
        system.add_fonts(vec![
            Cow::Borrowed(*gpui_fonts::IBM_PLEX),
            Cow::Borrowed(*gpui_fonts::SOURCE_SERIF),
            Cow::Borrowed(*gpui_fonts::NOTO_SANS),
        ])?;

        let regular_id = system.font_id(&font("IBM Plex Sans"))?;
        let regular_glyph = system
            .glyph_for_char(regular_id, 'A')
            .context("IBM Plex Sans has no A glyph")?;

        for scale_factor in [1.0, 1.5, 2.0] {
            for (mode, format) in [
                (Grayscale, RasterizedGlyphFormat::AlphaMask),
                (Subpixel, RasterizedGlyphFormat::BgraSubpixelMask),
            ] {
                let mut params =
                    raster_params(&*system, regular_id, regular_glyph, mode, rgba(0xffffffff));

                params.scale_factor = scale_factor;

                for offset in
                    subpixel_variants().filter(|offset| mode == Subpixel || *offset == point(0, 0))
                {
                    params.subpixel_variant = offset;
                    let rasterized = system.rasterize_glyph(&params)?;
                    assert_eq!(rasterized.format, format);
                    rasterized.validate()?;
                }
            }
        }

        let space = system
            .glyph_for_char(regular_id, ' ')
            .context("IBM Plex Sans has no space glyph")?;

        let params = raster_params(&*system, regular_id, space, Grayscale, rgba(0xffffffff));

        let empty = system.rasterize_glyph(&params)?;
        assert_eq!(empty.size, Size::default());
        assert!(empty.pixels.is_empty());

        let params = raster_params(&*system, regular_id, regular_glyph, Color, rgba(0xffffffff));

        let monochrome_color = system.rasterize_glyph(&params)?;
        assert_eq!(monochrome_color.format, RasterizedGlyphFormat::BgraColor);
        assert!(
            monochrome_color
                .pixels
                .chunks_exact(4)
                .filter(|pixel| pixel[3] != 0)
                .all(|pixel| pixel[..3] == [255, 255, 255])
        );
        monochrome_color.validate()?;

        for alpha in [0, 64, 128, 255] {
            let mut params = raster_params(
                &*system,
                regular_id,
                regular_glyph,
                Color,
                Rgba8::new(224, 32, 16, alpha).into(),
            );

            params.subpixel_variant = point(1, 1);
            params.scale_factor = 1.5;

            let rasterized = system.rasterize_glyph(&params)?;
            rasterized.validate()?;
            assert_eq!(rasterized.format, RasterizedGlyphFormat::BgraColor);

            assert!(rasterized.pixels.chunks_exact(4).all(|pixel| {
                pixel[3] <= alpha && (pixel[3] == 0 || pixel[..3] == [16, 32, 224])
            }));

            let rasterizer = test_rasterizer(regular_id, gpui_fonts::IBM_PLEX.data, 0)?;
            let error = rasterizer.translate_color_glyph(&params).unwrap_err();
            assert_eq!(
                error.downcast_ref::<windows::core::Error>().unwrap().code(),
                DWRITE_E_NOCOLOR
            );
        }

        let synthesized_id = system.font_id(&font("IBM Plex Sans").bold().italic())?;
        let synthesized_glyph = system
            .glyph_for_char(synthesized_id, 'A')
            .context("synthesized IBM Plex Sans has no A glyph")?;

        let mut params = raster_params(
            &*system,
            synthesized_id,
            synthesized_glyph,
            Grayscale,
            rgba(0xffffffff),
        );

        params.scale_factor = 2.0;

        let synthesized = system.rasterize_glyph(&params)?;
        assert!(!synthesized.pixels.is_empty());
        synthesized.validate()?;

        let mut light = font("Source Serif 4");
        light.weight = FontWeight::LIGHT;
        let mut bold = light.clone();
        bold.weight = FontWeight::BOLD;
        bold.style = FontStyle::Normal;
        let rasterize_font = |descriptor: &Font| {
            let font_id = system.font_id(descriptor)?;
            let glyph_id = system
                .glyph_for_char(font_id, 'A')
                .context("fixture has no A glyph")?;

            let mut params =
                raster_params(&*system, font_id, glyph_id, Grayscale, rgba(0xffffffff));

            params.scale_factor = 2.0;

            system.rasterize_glyph(&params)
        };

        for (first, second) in [
            (light, bold),
            (
                font("Noto Sans"),
                Font {
                    width: FontWidth::CONDENSED,
                    ..font("Noto Sans")
                },
            ),
        ] {
            let first = rasterize_font(&first)?;
            let second = rasterize_font(&second)?;
            first.validate()?;
            second.validate()?;
            assert_ne!((first.bounds, first.pixels), (second.bounds, second.pixels));
        }

        Ok(())
    }

    #[test]
    fn direct_write_loads_a_nonzero_collection_face() -> Result<()> {
        let collection = test_collection(&[
            *gpui_fonts::SOURCE_SERIF,
            *gpui_fonts::IBM_PLEX,
            *gpui_fonts::IBM_PLEX_ITALIC,
        ]);
        let rasterizer = test_rasterizer(FontId(1), &collection, 1)?;
        let codepoint = 'A' as u32;
        let mut glyph_id = 0;
        unsafe {
            rasterizer.faces[&FontId(1)].face.GetGlyphIndices(
                &raw const codepoint,
                1,
                &raw mut glyph_id,
            )?;
        }
        assert_ne!(glyph_id, 0);

        Ok(())
    }

    #[test]
    fn native_color_survives_device_replacement() -> Result<()> {
        let devices = DirectXDevices::new()?;
        let system = new_text_system()?;
        let font_id = system.font_id(&font("Segoe UI Emoji"))?;
        let glyph_id = system
            .glyph_for_char(font_id, '😀')
            .context("Segoe UI Emoji has no grinning-face glyph")?;

        let mut params = raster_params(&*system, font_id, glyph_id, Color, rgba(0xffffffff));

        params.scale_factor = 2.0;
        let before = system.rasterize_glyph(&params)?;
        assert_eq!(before.format, RasterizedGlyphFormat::BgraColor);
        assert!(before.pixels.chunks_exact(4).any(|pixel| {
            pixel[3] > 128
                && (pixel[0].abs_diff(pixel[1]) > 20
                    || pixel[1].abs_diff(pixel[2]) > 20
                    || pixel[0].abs_diff(pixel[2]) > 20)
        }));
        before.validate()?;

        drop(devices);
        let _replacement_devices = DirectXDevices::new()?;
        let after = system.rasterize_glyph(&params)?;
        after.validate()?;
        assert_eq!(before.bounds, after.bounds);
        assert_eq!(before.format, after.format);
        assert_eq!(before.pixels, after.pixels);

        Ok(())
    }

    #[test]
    fn headless_color_layers_preserve_palette_foreground_and_alpha() -> Result<()> {
        let (system, font_id) = color_test_system()?;

        let bitmap = bitmap_test_params(&*system)?;

        for (font_id, character) in [(font_id, 'A'), (font_id, 'B'), (bitmap.font_id, '😀')] {
            let glyph_id = system.glyph_for_char(font_id, character).unwrap();
            let mut red = raster_params(&*system, font_id, glyph_id, Color, rgba(0xe02010ff));
            red.scale_factor = 2.0;
            let mut green = raster_params(&*system, font_id, glyph_id, Color, rgba(0x20c080ff));
            green.scale_factor = 2.0;
            let red_pixels = system.rasterize_glyph(&red)?;
            let green_pixels = system.rasterize_glyph(&green)?;
            red_pixels.validate()?;
            green_pixels.validate()?;
            assert!(!red_pixels.pixels.is_empty());
            assert_eq!(red_pixels.format, RasterizedGlyphFormat::BgraColor);
            assert!(red_pixels.pixels.chunks_exact(4).any(|pixel| pixel[3] == 0));

            if character != '😀' {
                assert!(red_pixels.pixels.chunks_exact(4).any(|pixel| {
                    pixel[0] == 255 && pixel[1] == 0 && pixel[2] == 0 && pixel[3] == 128
                }));
            }

            if character != 'B' {
                assert_eq!(
                    red.raster_style.foreground_dependency,
                    ForegroundDependency::AlphaOnly
                );
                assert_eq!(red.raster_style, green.raster_style);
                assert_eq!(red_pixels.pixels, green_pixels.pixels);

                if character == 'A' {
                    assert!(red_pixels.pixels.chunks_exact(4).any(|pixel| {
                        pixel[3] == 255
                            && pixel[0].abs_diff(128) <= 2
                            && pixel[1] == 0
                            && pixel[2].abs_diff(127) <= 2
                    }));
                }
            } else {
                assert_eq!(
                    red.raster_style.foreground_dependency,
                    ForegroundDependency::Full
                );
                assert_ne!(red.raster_style, green.raster_style);
                assert_ne!(red_pixels.pixels, green_pixels.pixels);
                assert!(
                    red_pixels
                        .pixels
                        .chunks_exact(4)
                        .any(|pixel| { pixel[..3] == [16, 32, 224] && pixel[3] == 255 })
                );
            }

            for alpha in [0, 64, 128, 255] {
                let foreground = if character == 'B' {
                    Rgba8::new(224, 32, 16, alpha)
                } else {
                    Rgba8::new(32, 192, 128, alpha)
                };

                let mut params =
                    raster_params(&*system, font_id, glyph_id, Color, foreground.into());

                params.scale_factor = 2.0;
                let rasterized = system.rasterize_glyph(&params)?;
                rasterized.validate()?;

                assert_foreground_alpha(&rasterized, &red_pixels, alpha);
            }
        }

        Ok(())
    }

    #[test]
    fn color_bounds_cover_layers_and_resampled_bitmap_strikes() -> Result<()> {
        let (system, font_id) = color_test_system()?;
        let color_data = color_font_data(&*system, font_id);
        let bitmap = bitmap_test_params(&*system)?;

        for (font_id, character) in [(font_id, 'i'), (font_id, ' '), (bitmap.font_id, '😀')] {
            let glyph_id = system.glyph_for_char(font_id, character).unwrap();

            let data = if character == '😀' {
                gpui_fonts::NOTO_COLOR_EMOJI.data
            } else {
                &color_data
            };

            let rasterizer = test_rasterizer(font_id, data, 0)?;

            for (font_size, scale_factor) in [
                (12.0, 1.0),
                (24.0, 1.0),
                (24.0, 1.5),
                (24.0, 2.0),
                (54.5, 2.0),
                (160.0, 1.0),
                (160.0, 2.0),
            ] {
                for subpixel_variant in subpixel_variants().chain([point(1, 1)]) {
                    let mut params =
                        raster_params(&*system, font_id, glyph_id, Color, rgba(0xffffffff));

                    params.subpixel_variant = subpixel_variant;
                    params.scale_factor = scale_factor;

                    params.font_size = px(font_size);
                    assert_color_bounds(&*system, &rasterizer, character, &params)?;
                }
            }
        }

        Ok(())
    }

    #[test]
    fn color_rasterization_balances_uninitialized_sta_and_mta_apartments() -> Result<()> {
        for (apartment, concurrent_mta) in [
            (None, false),
            (None, true),
            (Some(COINIT_APARTMENTTHREADED), false),
            (Some(COINIT_MULTITHREADED), false),
        ] {
            // Keep another thread's MTA alive to exercise implicit membership deterministically.
            let _concurrent_apartment =
                concurrent_mta.then(ColorRasterApartment::new).transpose()?;

            std::thread::spawn(move || -> Result<()> {
                assert_eq!(explicit_calling_apartment()?, None);
                let caller = apartment
                    .map(|mode| -> Result<ColorRasterApartment> {
                        assert_eq!(unsafe { CoInitializeEx(None, mode) }, S_OK);

                        Ok(ColorRasterApartment {
                            initialized: true,
                            _thread: PhantomData,
                        })
                    })
                    .transpose()?;
                let before = explicit_calling_apartment()?;
                let (system, font_id) = color_test_system()?;
                let bitmap_params = bitmap_test_params(&*system)?;
                let rasterizer =
                    test_rasterizer(bitmap_params.font_id, gpui_fonts::NOTO_COLOR_EMOJI.data, 0)?;

                let glyph_id = system.glyph_for_char(font_id, 'B').unwrap();
                let mut params =
                    raster_params(&*system, font_id, glyph_id, Color, rgba(0x20c080ff));

                params.subpixel_variant = point(1, 1);
                params.scale_factor = 1.5;

                for iteration in 0..3 {
                    let bitmap = system.rasterize_glyph(&bitmap_params)?;
                    bitmap.validate()?;
                    assert!(!bitmap.pixels.is_empty());

                    assert_eq!(explicit_calling_apartment()?, before);

                    // Fail after retrieval and decoding to exercise the apartment and image guards.
                    assert!(
                        rasterizer
                            .with_bitmap_glyph(&bitmap_params, |_wic, _data, _bitmap| {
                                bail!("injected bitmap draw failure")
                            })
                            .is_err()
                    );

                    assert_eq!(explicit_calling_apartment()?, before);

                    let mut invalid_bitmap = bitmap_params.clone();
                    invalid_bitmap.glyph_id = GlyphId(u32::MAX);
                    assert!(rasterizer.rasterize_bitmap_glyph(&invalid_bitmap).is_err());
                    assert_eq!(explicit_calling_apartment()?, before);

                    let rasterized = system.rasterize_glyph(&params)?;
                    rasterized.validate()?;

                    assert!(
                        rasterized
                            .pixels
                            .chunks_exact(4)
                            .any(|pixel| pixel[..3] == [128, 192, 32])
                    );

                    assert_eq!(
                        explicit_calling_apartment()?,
                        before,
                        "apartment changed after raster {iteration}"
                    );
                }

                drop(system);
                drop(caller);
                assert_eq!(explicit_calling_apartment()?, None);

                Ok(())
            })
            .join()
            .expect("color raster thread panicked")?;
        }

        Ok(())
    }

    #[test]
    fn cbdt_preserves_sequences_and_empty_or_invalid_glyphs() -> Result<()> {
        let system = new_text_system()?;
        let mut params = bitmap_test_params(&*system)?;
        system.rasterize_glyph(&params)?;

        let rasterizer = test_rasterizer(params.font_id, gpui_fonts::NOTO_COLOR_EMOJI.data, 0)?;
        let rasterize_native =
            |params: &RenderGlyphParams| rasterizer.rasterize_bitmap_glyph(params);

        for text in ["👩‍💻", "👩🏽", "🇬🇧", "1️⃣"] {
            let runs = [TextRun {
                len: text.len(),
                font: font("Noto Color Emoji"),
                ..Default::default()
            }];

            let layout = system.layout_text(TextLayoutRequest {
                text,
                font_size: px(24.0),
                runs: &runs,
                options: Default::default(),
            });

            let fragments = &layout.paint_fragments;
            assert_eq!(fragments.len(), 1, "{text}");
            assert_eq!(fragments[0].font_id, params.font_id, "{text}");
            assert_eq!(fragments[0].glyphs.len(), 1, "{text}");
            let glyph_id = fragments[0].glyphs[0].id;
            params = raster_params(&*system, params.font_id, glyph_id, Color, rgba(0xffffffff));

            assert_eq!(
                params.raster_style.foreground_dependency,
                ForegroundDependency::AlphaOnly
            );

            let actual = system.rasterize_glyph(&params)?;
            actual.validate()?;

            assert!(
                actual.pixels.chunks_exact(4).any(|pixel| {
                    pixel[3] > 0 && (pixel[0] != pixel[1] || pixel[1] != pixel[2])
                }),
                "{text}"
            );

            let native = rasterize_native(&params)?;

            assert_eq!(actual.bounds, native.bounds, "{text}");
            assert_eq!(actual.format, native.format, "{text}");
            assert_eq!(actual.pixels, native.pixels, "{text}");
        }

        params.glyph_id = system.glyph_for_char(params.font_id, ' ').unwrap();

        let empty = system.rasterize_glyph(&params)?;
        empty.validate()?;
        assert!(empty.pixels.is_empty());
        assert!(rasterize_native(&params).is_err());

        for invalid in [u16::MAX as u32, u16::MAX as u32 + 1, u32::MAX] {
            params.glyph_id = GlyphId(invalid);
            assert!(system.rasterize_glyph(&params).is_err());

            assert!(rasterize_native(&params).is_err());
        }

        params.glyph_id = system.glyph_for_char(params.font_id, '😀').unwrap();
        params.font_size = Pixels::ZERO;
        assert!(system.rasterize_glyph(&params)?.pixels.is_empty());

        for invalid in [f32::NAN, f32::INFINITY, -1.0, f32::MAX] {
            params.font_size = px(invalid);

            assert!(system.rasterize_glyph(&params).is_err());
            assert!(rasterize_native(&params).is_err());
        }

        params.font_size = px(24.0);

        for invalid in [f32::NAN, f32::INFINITY, -1.0, 0.0] {
            params.scale_factor = invalid;

            assert!(system.rasterize_glyph(&params).is_err());
            assert!(rasterize_native(&params).is_err());
        }

        params.scale_factor = 1.0;
        assert!(!system.rasterize_glyph(&params)?.pixels.is_empty());

        Ok(())
    }

    #[test]
    fn cbdt_image_data_survives_retrieval_decode_and_target_errors() -> Result<()> {
        let _apartment = ColorRasterApartment::new()?;

        let wic: IWICImagingFactory =
            unsafe { CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)? };

        let system = new_text_system()?;
        let params = bitmap_test_params(&*system)?;
        system.rasterize_glyph(&params)?;
        let rasterizer = test_rasterizer(params.font_id, gpui_fonts::NOTO_COLOR_EMOJI.data, 0)?;
        let face = &rasterizer.faces[&params.font_id].face;
        let glyph_id = params.glyph_id.0 as u16;
        let png = DWRITE_GLYPH_IMAGE_FORMATS_PNG;

        assert!(
            GlyphImageData::new(
                face,
                glyph_id,
                24,
                png | DWRITE_GLYPH_IMAGE_FORMATS_PREMULTIPLIED_B8G8R8A8
            )
            .is_err()
        );

        assert!(GlyphImageData::new(face, glyph_id, 24, DWRITE_GLYPH_IMAGE_FORMATS_SVG)?.is_none());

        // Each failure unwinds the data guard before the next retrieval.
        let load = |length: usize| -> Result<_> {
            let image = GlyphImageData::new(face, glyph_id, 24, png)?
                .context("missing Noto PNG artwork")?;

            let bytes = image.bytes()?;

            let bitmap =
                decode_bitmap_glyph(&wic, &image.data, &bytes[..length.min(bytes.len())], png)?;

            Ok((image.data, bitmap))
        };

        for iteration in 0..3 {
            assert!(load(16).is_err());
            let (data, bitmap) = load(usize::MAX)?;
            assert!(decode_bitmap_glyph(&wic, &data, b"not a PNG", png).is_err());
            let mut invalid = data;
            invalid.pixelSize.width += 1;
            let image = GlyphImageData::new(face, glyph_id, 24, png)?.unwrap();
            assert!(decode_bitmap_glyph(&wic, &invalid, image.bytes()?, png).is_err());

            for (width, height) in [(0, 1), (1, 0), (u32::MAX, 1), (65536, 65536)] {
                invalid = data;
                invalid.pixelSize.width = width;
                invalid.pixelSize.height = height;

                assert!(decode_bitmap_glyph(&wic, &invalid, image.bytes()?, png).is_err());
            }

            let mut invalid_image = GlyphImageData::new(face, glyph_id, 24, png)?.unwrap();
            invalid_image.data.imageData = std::ptr::null();
            assert!(invalid_image.bytes().is_err());
            invalid_image.data.imageDataSize = 0;
            assert!(invalid_image.bytes().is_err());
            drop(invalid_image);

            invalid = data;
            invalid.pixelsPerEm = 0;
            assert!(bitmap_glyph_rect(&params, &invalid).is_err());

            for edge in [f32::NAN, f32::INFINITY, i32::MAX as f32] {
                assert!(
                    bitmap_ink_bounds(D2D_RECT_F {
                        right: edge,
                        bottom: 1.0,
                        ..Default::default()
                    })
                    .is_err()
                );
            }

            let destination = bitmap_glyph_rect(&params, &data)?;
            let bounds = bitmap_ink_bounds(destination)?;
            let failed_bounds = oversized_color_bounds(&wic, params.raster_style)?;
            assert!(draw_bitmap_glyph(&wic, &params, &bitmap, destination, failed_bounds).is_err());
            let actual = draw_bitmap_glyph(&wic, &params, &bitmap, destination, bounds)?;
            actual.validate()?;

            assert!(
                !actual.pixels.is_empty(),
                "glyph data lost after error {iteration}"
            );
        }

        Ok(())
    }

    #[test]
    fn native_color_target_failure_uses_monochrome_foreground() -> Result<()> {
        let _apartment = ColorRasterApartment::new()?;
        let wic: IWICImagingFactory =
            unsafe { CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)? };
        let (system, font_id) = color_test_system()?;
        let glyph_id = system.glyph_for_char(font_id, 'B').unwrap();
        let params = raster_params(&*system, font_id, glyph_id, Color, rgba(0xe0201080));
        system.rasterize_glyph(&params)?;

        let width = oversized_color_bounds(&wic, params.raster_style)?.right;

        let rasterizer = test_rasterizer(font_id, &color_font_data(&*system, font_id), 0)?;
        let glyph_analysis = rasterizer.create_glyph_run_analysis(&params)?;
        let oversized = Bounds {
            origin: point(0.into(), (-10).into()),
            size: size(width.into(), 1.into()),
        };
        assert!(rasterizer.rasterize_color(&params, oversized).is_err());

        let native_bounds = RECT {
            left: oversized.origin.x.0,
            top: oversized.origin.y.0,
            right: oversized.origin.x.0 + oversized.size.width.0,
            bottom: oversized.origin.y.0 + oversized.size.height.0,
        };
        let coverage = rasterizer.rasterize_monochrome(
            &glyph_analysis,
            DWRITE_TEXTURE_ALIASED_1x1,
            native_bounds,
        )?;
        let fallback = rasterizer.rasterize_glyph(
            &params,
            &glyph_analysis,
            DWRITE_TEXTURE_ALIASED_1x1,
            native_bounds,
        )?;
        fallback.validate()?;
        assert_eq!(fallback.format, RasterizedGlyphFormat::BgraColor);
        assert_eq!(fallback.bounds, oversized);
        assert!(fallback.pixels.chunks_exact(4).any(|pixel| pixel[3] != 0));

        for (pixel, coverage) in fallback.pixels.chunks_exact(4).zip(coverage) {
            assert_eq!(pixel[3], ((coverage as u16 * 128 + 127) / 255) as u8);

            if pixel[3] == 0 {
                assert_eq!(pixel, [0; 4]);
            } else {
                assert_eq!(pixel[..3], [16, 32, 224]);
            }
        }

        Ok(())
    }

    #[test]
    fn wic_readback_preserves_rows_and_converts_premultiplied_alpha() -> Result<()> {
        let _apartment = ColorRasterApartment::new()?;
        let factory: IWICImagingFactory =
            unsafe { CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)? };
        let source = [
            0, 0, 0, 0, 32, 64, 128, 128, 17, 18, 19, 20, 255, 0, 0, 255, 9, 8, 7, 0, 21, 22, 23,
            24,
        ];
        let bitmap = unsafe {
            factory.CreateBitmapFromMemory(2, 2, &GUID_WICPixelFormat32bppPBGRA, 12, &source)?
        };

        let data = DWRITE_GLYPH_IMAGE_DATA {
            pixelsPerEm: 2,
            pixelSize: D2D_SIZE_U {
                width: 2,
                height: 2,
            },
            horizontalLeftOrigin: POINT { x: 1, y: 2 },
            ..Default::default()
        };

        let bytes = [0, 0, 0, 0, 32, 64, 128, 128, 255, 0, 0, 255, 0, 0, 0, 0];

        let bgra = DWRITE_GLYPH_IMAGE_FORMATS_PREMULTIPLIED_B8G8R8A8;
        let decoded = decode_bitmap_glyph(&factory, &data, &bytes, bgra)?;

        for current in [&bitmap, &decoded] {
            for (alpha, expected_alpha) in [(255, 128), (128, 64), (64, 32)] {
                let pixels = read_color_bitmap(current, 2, 2, alpha)?;

                assert_eq!(
                    pixels,
                    [
                        [0; 4],
                        [64, 128, 255, expected_alpha],
                        [255, 0, 0, alpha],
                        [0; 4]
                    ]
                    .concat()
                );
            }

            assert_eq!(read_color_bitmap(current, 2, 2, 0)?, vec![0; 16]);
        }

        for invalid in [&bytes[..15], &bytes[..0]] {
            assert!(decode_bitmap_glyph(&factory, &data, invalid, bgra).is_err());
        }

        let system = new_text_system()?;
        let mut params = bitmap_test_params(&*system)?;

        params.font_size = px(4.0);

        let destination = bitmap_glyph_rect(&params, &data)?;
        let bounds = bitmap_ink_bounds(destination)?;

        assert_eq!(
            (bounds.left, bounds.top, bounds.right, bounds.bottom),
            (-2, -4, 2, 0)
        );

        let actual = draw_bitmap_glyph(&factory, &params, &decoded, destination, bounds)?;
        actual.validate()?;

        // Premultiplied interpolation keeps transparent neighbors from darkening the color.
        let edge = &actual.pixels[4..8];
        assert!(edge[3] > 0 && edge[3] < 128);
        assert!(edge[0].abs_diff(64) <= 8);
        assert!(edge[1].abs_diff(128) <= 8);
        assert!(edge[2] >= 248);

        Ok(())
    }

    fn oversized_color_bounds(
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

    fn bitmap_test_params(system: &dyn PlatformTextSystem) -> Result<RenderGlyphParams> {
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

    fn raster_params(
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

    fn explicit_calling_apartment() -> windows::core::Result<Option<(APTTYPE, APTTYPEQUALIFIER)>> {
        let mut apartment = APTTYPE::default();
        let mut qualifier = APTTYPEQUALIFIER::default();

        match unsafe { CoGetApartmentType(&mut apartment, &mut qualifier) } {
            Err(error) if error.code() == CO_E_NOTINITIALIZED => Ok(None),
            Err(error) => Err(error),
            // Other tests can keep the process MTA alive without initializing this thread.
            Ok(()) if apartment == APTTYPE_MTA && qualifier == APTTYPEQUALIFIER_IMPLICIT_MTA => {
                Ok(None)
            }
            Ok(()) => Ok(Some((apartment, qualifier))),
        }
    }

    fn subpixel_variants() -> impl Iterator<Item = Point<u8>> {
        (0..SUBPIXEL_VARIANTS_X).flat_map(|subpixel_x| {
            (0..SUBPIXEL_VARIANTS_Y).map(move |subpixel_y| point(subpixel_x, subpixel_y))
        })
    }

    fn assert_foreground_alpha(actual: &RasterizedGlyph, opaque: &RasterizedGlyph, alpha: u8) {
        // Native color rasters can be empty, while bitmap fallback retains its placement.
        if actual.pixels.is_empty() {
            assert_eq!(alpha, 0);
            assert_eq!(actual.size, Size::default());

            return;
        }

        assert_eq!(actual.bounds, opaque.bounds);
        assert_eq!(actual.size, opaque.size);
        assert_eq!(actual.pixels.len(), opaque.pixels.len());

        for (actual, opaque) in actual
            .pixels
            .chunks_exact(4)
            .zip(opaque.pixels.chunks_exact(4))
        {
            assert_eq!(
                actual[3],
                ((opaque[3] as u16 * alpha as u16 + 127) / 255) as u8
            );

            if actual[3] == 0 {
                assert_eq!(actual, [0; 4]);
            } else {
                assert_eq!(actual[..3], opaque[..3]);
            }
        }
    }

    fn assert_color_bounds(
        system: &dyn PlatformTextSystem,
        rasterizer: &DirectWriteGlyphRasterizer,
        character: char,
        params: &RenderGlyphParams,
    ) -> Result<()> {
        let actual = system.rasterize_glyph(params)?;
        actual.validate()?;
        assert!(!actual.pixels.is_empty());

        let padded_bounds = actual.bounds.dilate(DevicePixels(4));

        let padded = if character == '😀' {
            rasterizer.with_bitmap_glyph(params, |wic, data, bitmap| {
                let direct = rasterizer.rasterize_bitmap_glyph(params)?;

                assert_eq!(
                    (actual.bounds, actual.format, &actual.pixels),
                    (direct.bounds, direct.format, &direct.pixels)
                );

                assert_ne!(
                    data.pixelsPerEm, 24,
                    "fixture must exercise strike resampling"
                );

                assert!(actual.bounds.origin.y.0 < 0);

                assert!(
                    actual
                        .pixels
                        .chunks_exact(4)
                        .any(|pixel| pixel[3] > 0 && pixel[2] > pixel[0].saturating_add(32))
                );

                let scale =
                    params.font_size.as_f32() * params.scale_factor / data.pixelsPerEm as f32;

                assert!(
                    (actual.size.width.0 as f32 - data.pixelSize.width as f32 * scale).abs() < 2.0
                );

                draw_bitmap_glyph(
                    wic,
                    params,
                    bitmap,
                    bitmap_glyph_rect(params, data)?,
                    RECT {
                        left: padded_bounds.left().0,
                        top: padded_bounds.top().0,
                        right: padded_bounds.right().0,
                        bottom: padded_bounds.bottom().0,
                    },
                )
            })?
        } else {
            let glyph_analysis = rasterizer.create_glyph_run_analysis(params)?;

            let base_bounds =
                unsafe { glyph_analysis.GetAlphaTextureBounds(DWRITE_TEXTURE_ALIASED_1x1)? };

            if character == ' ' {
                assert_eq!(base_bounds.right, base_bounds.left);
                assert_eq!(base_bounds.bottom, base_bounds.top);
            } else {
                assert!(actual.size.width.0 > base_bounds.right - base_bounds.left);
            }

            rasterizer.rasterize_color(params, padded_bounds)?
        };

        padded.validate()?;
        assert_padded_color_matches(&actual, &padded);

        Ok(())
    }

    fn assert_padded_color_matches(actual: &RasterizedGlyph, padded: &RasterizedGlyph) {
        let actual_width = actual.size.width.0 as usize;
        let padded_width = padded.size.width.0 as usize;

        for (pixel_index, pixel) in padded.pixels.chunks_exact(4).enumerate() {
            let column = pixel_index % padded_width;
            let row = pixel_index / padded_width;
            let actual_x = padded.bounds.origin.x.0 + column as i32 - actual.bounds.origin.x.0;
            let actual_y = padded.bounds.origin.y.0 + row as i32 - actual.bounds.origin.y.0;

            if actual_x < 0
                || actual_y < 0
                || actual_x >= actual.size.width.0
                || actual_y >= actual.size.height.0
            {
                assert_eq!(pixel, [0; 4], "color ink extends outside raster bounds");

                continue;
            }

            let offset = (actual_y as usize * actual_width + actual_x as usize) * 4;
            assert_eq!(pixel, &actual.pixels[offset..offset + 4]);
        }
    }

    fn color_test_system() -> Result<(Arc<dyn PlatformTextSystem>, FontId)> {
        let outline_system = new_text_system()?;
        outline_system.add_fonts(vec![Cow::Borrowed(*gpui_fonts::IBM_PLEX)])?;
        let font_id = outline_system.font_id(&font("IBM Plex Sans"))?;
        let data = color_font_data(&*outline_system, font_id);

        let system = new_text_system()?;
        system.add_fonts(vec![Cow::Owned(data)])?;
        let font_id = system.font_id(&font("IBM Plex Sans"))?;

        Ok((system, font_id))
    }

    fn test_rasterizer(
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

    fn color_font_data(system: &dyn PlatformTextSystem, font_id: FontId) -> Vec<u8> {
        let glyphs = ['A', 'B', 'i', ' ', 'H', 'O']
            .map(|character| system.glyph_for_char(font_id, character).unwrap().0 as u16);

        test_color_font(glyphs)
    }

    fn test_color_font(glyphs: [u16; 6]) -> Vec<u8> {
        let [
            palette,
            foreground,
            small_base,
            empty_base,
            first_layer,
            second_layer,
        ] = glyphs;
        let mut base_records: [(u16, Vec<(u16, u16)>); 4] = [
            (palette, vec![(first_layer, 0), (second_layer, 1)]),
            (foreground, vec![(first_layer, 0xffff), (second_layer, 1)]),
            (small_base, vec![(first_layer, 0), (second_layer, 1)]),
            (empty_base, vec![(first_layer, 0)]),
        ];
        base_records.sort_by_key(|record| record.0);

        let mut colr = Vec::new();
        colr.extend_from_slice(&0u16.to_be_bytes());
        colr.extend_from_slice(&(base_records.len() as u16).to_be_bytes());
        colr.extend_from_slice(&14u32.to_be_bytes());
        colr.extend_from_slice(&(14 + base_records.len() as u32 * 6).to_be_bytes());
        colr.extend_from_slice(&7u16.to_be_bytes());

        let mut layer_index = 0u16;

        for (glyph_id, layers) in &base_records {
            colr.extend_from_slice(&glyph_id.to_be_bytes());
            colr.extend_from_slice(&layer_index.to_be_bytes());
            colr.extend_from_slice(&(layers.len() as u16).to_be_bytes());
            layer_index += layers.len() as u16;
        }

        for (_glyph_id, layers) in &base_records {
            for (glyph_id, palette_index) in layers {
                colr.extend_from_slice(&glyph_id.to_be_bytes());
                colr.extend_from_slice(&palette_index.to_be_bytes());
            }
        }

        // One opaque red and one half-transparent blue CPAL entry.
        let cpal = vec![
            0, 0, 0, 2, 0, 1, 0, 2, 0, 0, 0, 14, 0, 0, 0, 0, 255, 255, 255, 0, 0, 128,
        ];
        let table_count = read_u16(*gpui_fonts::IBM_PLEX, 4).unwrap() as usize;
        let mut tables = BTreeMap::new();

        for table_index in 0..table_count {
            let record = 12 + table_index * 16;
            let tag: [u8; 4] = gpui_fonts::IBM_PLEX[record..record + 4].try_into().unwrap();
            let offset = read_u32(*gpui_fonts::IBM_PLEX, record + 8).unwrap() as usize;
            let length = read_u32(*gpui_fonts::IBM_PLEX, record + 12).unwrap() as usize;
            tables.insert(tag, gpui_fonts::IBM_PLEX[offset..offset + length].to_vec());
        }

        tables.insert(*b"COLR", colr);
        tables.insert(*b"CPAL", cpal);
        tables.get_mut(b"head").unwrap()[8..12].fill(0);

        let table_count = tables.len() as u16;
        let search_range = (1u16 << table_count.ilog2()) * 16;
        let mut font = vec![0; 12 + tables.len() * 16];
        font[..4].copy_from_slice(&gpui_fonts::IBM_PLEX[..4]);
        font[4..6].copy_from_slice(&table_count.to_be_bytes());
        font[6..8].copy_from_slice(&search_range.to_be_bytes());
        font[8..10].copy_from_slice(&(table_count.ilog2() as u16).to_be_bytes());
        font[10..12].copy_from_slice(&(table_count * 16 - search_range).to_be_bytes());

        let mut head_offset = 0;

        for (table_index, (tag, data)) in tables.into_iter().enumerate() {
            let offset = font.len();
            let record = 12 + table_index * 16;
            font[record..record + 4].copy_from_slice(&tag);
            font[record + 4..record + 8].copy_from_slice(&sfnt_checksum(&data).to_be_bytes());
            font[record + 8..record + 12].copy_from_slice(&(offset as u32).to_be_bytes());
            font[record + 12..record + 16].copy_from_slice(&(data.len() as u32).to_be_bytes());

            if tag == *b"head" {
                head_offset = offset;
            }

            font.extend_from_slice(&data);

            while !font.len().is_multiple_of(4) {
                font.push(0);
            }
        }

        let adjustment = 0xb1b0_afbau32.wrapping_sub(sfnt_checksum(&font));
        font[head_offset + 8..head_offset + 12].copy_from_slice(&adjustment.to_be_bytes());

        font
    }

    fn sfnt_checksum(data: &[u8]) -> u32 {
        data.chunks(4).fold(0u32, |checksum, chunk| {
            let mut word = [0; 4];
            word[..chunk.len()].copy_from_slice(chunk);

            checksum.wrapping_add(u32::from_be_bytes(word))
        })
    }

    fn read_u16(data: &[u8], offset: usize) -> Option<u16> {
        Some(u16::from_be_bytes(
            data.get(offset..offset + 2)?.try_into().ok()?,
        ))
    }

    fn read_u32(data: &[u8], offset: usize) -> Option<u32> {
        Some(u32::from_be_bytes(
            data.get(offset..offset + 4)?.try_into().ok()?,
        ))
    }
}
