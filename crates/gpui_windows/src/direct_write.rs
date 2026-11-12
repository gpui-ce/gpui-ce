use std::{
    collections::hash_map::Entry,
    ffi::{c_uint, c_void},
    marker::PhantomData,
    mem::ManuallyDrop,
    rc::Rc,
    sync::Arc,
};

use anyhow::{Context as _, Result, ensure};
use collections::HashMap;
use gpui::{
    Bounds, ColorGlyphKind, DevicePixels, FontDataBlob, FontId, GlyphRasterizer, GlyphRenderMode,
    Pixels, PlatformTextSystem, PreparedRasterStyle, RasterColorEffect, RasterFace,
    RasterStyleRequest, RasterizedGlyph, RenderGlyphParams, Rgba8, SUBPIXEL_VARIANTS_X,
    SUBPIXEL_VARIANTS_Y, TextRenderingMode, apply_pixel_opacity, multiply_alpha, point, size,
};
use gpui_parley::ParleyTextSystem;
use gpui_render::BitmapFallbackGlyphRasterizer;
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
    face: IDWriteFontFace3,
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
        ParleyTextSystem::new(BitmapFallbackGlyphRasterizer::new(
            DirectWriteGlyphRasterizer::new()?,
        ))
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
            fontFace: ManuallyDrop::new(Some(unsafe { std::ptr::read(&***font.face) })),
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
            fontFace: ManuallyDrop::new(Some(unsafe { std::ptr::read(&***font.face) })),
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
        kind == ColorGlyphKind::ColrV0
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
            params.scale_factor.is_finite() && params.scale_factor > 0.0,
            "invalid raster scale factor"
        );

        let format = params.raster_style.mode.rasterized_format();

        if params.font_size == Pixels::ZERO {
            return Ok(RasterizedGlyph::empty(format));
        }

        self.load_face(face)?;

        debug_assert!(
            !matches!(params.raster_style.color_effect, RasterColorEffect::Dilation(value) if value != 0)
        );

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

        let native_face = if use_default_axes {
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

        Ok(Self { face: native_face })
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
        Point, RasterizedGlyphFormat, Rgba, Size, font, px, rgba,
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
    fn color_layer_bounds_cover_empty_and_smaller_base_outlines() -> Result<()> {
        let (system, font_id) = color_test_system()?;
        let color_data = color_font_data(&*system, font_id);

        for character in ['i', ' '] {
            let glyph_id = system.glyph_for_char(font_id, character).unwrap();

            let rasterizer = test_rasterizer(font_id, &color_data, 0)?;

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
                let glyph_id = system.glyph_for_char(font_id, 'B').unwrap();
                let mut params =
                    raster_params(&*system, font_id, glyph_id, Color, rgba(0x20c080ff));

                params.subpixel_variant = point(1, 1);
                params.scale_factor = 1.5;

                for iteration in 0..3 {
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

        for (alpha, expected_alpha) in [(255, 128), (128, 64), (64, 32)] {
            let pixels = read_color_bitmap(&bitmap, 2, 2, alpha)?;
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

        assert_eq!(read_color_bitmap(&bitmap, 2, 2, 0)?, vec![0; 16]);

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

        let glyph_analysis = rasterizer.create_glyph_run_analysis(params)?;

        let base_bounds =
            unsafe { glyph_analysis.GetAlphaTextureBounds(DWRITE_TEXTURE_ALIASED_1x1)? };

        if character == ' ' {
            assert_eq!(base_bounds.right, base_bounds.left);
            assert_eq!(base_bounds.bottom, base_bounds.top);
        } else {
            assert!(actual.size.width.0 > base_bounds.right - base_bounds.left);
        }

        let padded = rasterizer.rasterize_color(params, padded_bounds)?;
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
