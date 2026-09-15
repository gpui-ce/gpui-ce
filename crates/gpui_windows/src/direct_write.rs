use std::{
    borrow::Cow,
    ffi::{c_uint, c_void},
    mem::ManuallyDrop,
    sync::Arc,
};

use anyhow::{Context as _, Result, ensure};
use collections::HashMap;
use gpui::{
    Bounds, DevicePixels, Font, FontId, FontMetrics, GlyphId, GlyphRenderMode, InlineLayout,
    InlineLayoutRequest, LineLayout, Pixels, PlatformTextSystem, Point, PreparedRasterStyle,
    RasterColorEffect, RasterStyleRequest, RasterizedGlyph, RenderGlyphParams, Rgba,
    SUBPIXEL_VARIANTS_X, Size, TextLayoutRequest, TextRenderingMode, bounds, point, size,
};
use gpui_parley::{
    BitmapFallbackGlyphRasterizer, ColorGlyphKind, FontDataBlob, GlyphRasterizer, ParleyTextSystem,
    RasterFace, SystemFonts,
};
use gpui_render::shaders::emoji_rasterization::GlyphLayerTextureParams;
use parking_lot::RwLock;
use wgsl_rs::std::vec4f;
use windows::{
    Win32::{
        Foundation::*,
        Graphics::{
            Direct3D::D3D_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP, Direct3D11::*, DirectWrite::*,
            Dxgi::Common::*,
        },
        UI::WindowsAndMessaging::*,
    },
    core::*,
};
use windows_numerics::Vector2;

use crate::*;

pub(crate) struct DirectWriteTextSystem {
    parley: ParleyTextSystem,
    renderer: Arc<RwLock<DirectWriteGlyphRenderer>>,
}

#[derive(Clone)]
struct SharedDirectWriteRenderer(Arc<RwLock<DirectWriteGlyphRenderer>>);

struct DirectWriteGlyphRenderer {
    components: DirectWriteComponents,
    variable_factory: Option<IDWriteFactory6>,
    rendering_params: IDWriteRenderingParams,
    gpu_state: Option<GPUState>,
    faces: NativeFaceCache<NativeFace>,
    sources: HashMap<u64, NativeSource>,
    system_subpixel_rendering: bool,
}

struct DirectWriteComponents {
    factory: IDWriteFactory5,
    in_memory_loader: IDWriteInMemoryFontFileLoader,
}

struct GPUState {
    device: ID3D11Device,
    device_context: ID3D11DeviceContext,
    sampler: Option<ID3D11SamplerState>,
    blend_state: ID3D11BlendState,
    vertex_shader: ID3D11VertexShader,
    pixel_shader: ID3D11PixelShader,
}

#[derive(Clone, Copy)]
struct NativeFontId(usize);

struct NativeFaceCache<F> {
    ids: HashMap<FontId, NativeFontId>,
    fonts: Vec<F>,
}

impl<F> Default for NativeFaceCache<F> {
    fn default() -> Self {
        Self {
            ids: HashMap::default(),
            fonts: Vec::new(),
        }
    }
}

impl<F> NativeFaceCache<F> {
    fn get_or_insert(
        &mut self,
        face: RasterFace<'_>,
        load: impl FnOnce(RasterFace<'_>) -> Result<F>,
    ) -> Result<NativeFontId> {
        if let Some(font_id) = self.ids.get(&face.font_id) {
            return Ok(*font_id);
        }

        let native = load(face)?;
        let font_id = NativeFontId(self.fonts.len());
        self.fonts.push(native);
        self.ids.insert(face.font_id, font_id);

        Ok(font_id)
    }

    fn clear(&mut self) {
        self.ids.clear();
        self.fonts.clear();
    }
}

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

#[derive(Clone)]
struct NativeGlyphParams {
    font_id: NativeFontId,
    glyph_id: GlyphId,
    font_size: Pixels,
    subpixel_variant: Point<u8>,
    scale_factor: f32,
    is_emoji: bool,
    subpixel_rendering: bool,
    dilation: u8,
}

impl NativeGlyphParams {
    fn from_parley(font_id: NativeFontId, params: &RenderGlyphParams) -> Self {
        Self {
            font_id,
            glyph_id: params.glyph_id,
            font_size: params.font_size,
            subpixel_variant: params.subpixel_variant,
            scale_factor: params.scale_factor,
            is_emoji: params.raster_style.mode == GlyphRenderMode::Color,
            subpixel_rendering: params.raster_style.mode == GlyphRenderMode::Subpixel,
            dilation: match params.raster_style.color_effect {
                RasterColorEffect::Dilation(value) => value,
                _ => 0,
            },
        }
    }
}

impl GPUState {
    fn new(directx_devices: &DirectXDevices) -> Result<Self> {
        let device = directx_devices.device.clone();
        let device_context = directx_devices.device_context.clone();

        let blend_state = {
            let mut blend_state = None;
            let desc = D3D11_BLEND_DESC {
                AlphaToCoverageEnable: false.into(),
                IndependentBlendEnable: false.into(),
                RenderTarget: [
                    D3D11_RENDER_TARGET_BLEND_DESC {
                        BlendEnable: true.into(),
                        SrcBlend: D3D11_BLEND_ONE,
                        DestBlend: D3D11_BLEND_INV_SRC_ALPHA,
                        BlendOp: D3D11_BLEND_OP_ADD,
                        SrcBlendAlpha: D3D11_BLEND_ONE,
                        DestBlendAlpha: D3D11_BLEND_INV_SRC_ALPHA,
                        BlendOpAlpha: D3D11_BLEND_OP_ADD,
                        RenderTargetWriteMask: D3D11_COLOR_WRITE_ENABLE_ALL.0 as u8,
                    },
                    Default::default(),
                    Default::default(),
                    Default::default(),
                    Default::default(),
                    Default::default(),
                    Default::default(),
                    Default::default(),
                ],
            };
            unsafe { device.CreateBlendState(&desc, Some(&mut blend_state)) }?;
            blend_state.unwrap()
        };

        let sampler = {
            let mut sampler = None;
            let desc = D3D11_SAMPLER_DESC {
                Filter: D3D11_FILTER_MIN_MAG_MIP_POINT,
                AddressU: D3D11_TEXTURE_ADDRESS_BORDER,
                AddressV: D3D11_TEXTURE_ADDRESS_BORDER,
                AddressW: D3D11_TEXTURE_ADDRESS_BORDER,
                MipLODBias: 0.0,
                MaxAnisotropy: 1,
                ComparisonFunc: D3D11_COMPARISON_ALWAYS,
                BorderColor: [0.0, 0.0, 0.0, 0.0],
                MinLOD: 0.0,
                MaxLOD: 0.0,
            };
            unsafe { device.CreateSamplerState(&desc, Some(&mut sampler)) }?;
            sampler
        };

        let bytecode = shader_resources::ShaderModule::EmojiRasterization.bytecode()?;
        let vertex_shader = {
            let mut shader = None;
            unsafe { device.CreateVertexShader(bytecode.vertex, None, Some(&mut shader)) }?;
            shader.unwrap()
        };

        let pixel_shader = {
            let mut shader = None;
            unsafe { device.CreatePixelShader(bytecode.fragment, None, Some(&mut shader)) }?;
            shader.unwrap()
        };

        Ok(Self {
            device,
            device_context,
            sampler,
            blend_state,
            vertex_shader,
            pixel_shader,
        })
    }
}

impl DirectWriteTextSystem {
    pub(crate) fn new(directx_devices: &DirectXDevices) -> Result<Self> {
        Self::new_inner(Some(directx_devices))
    }

    pub(crate) fn new_headless() -> Result<Self> {
        Self::new_inner(None)
    }

    fn new_inner(directx_devices: Option<&DirectXDevices>) -> Result<Self> {
        let renderer = Arc::new(RwLock::new(DirectWriteGlyphRenderer::new(directx_devices)?));
        let parley = ParleyTextSystem::new_with_rasterizer(
            SystemFonts::Load,
            "Segoe UI",
            BitmapFallbackGlyphRasterizer::new(SharedDirectWriteRenderer(renderer.clone())),
        )
        .with_fallback_families(["Lilex", "IBM Plex Sans", "Arial"]);

        Ok(Self { parley, renderer })
    }

    pub(crate) fn handle_gpu_lost(&self, directx_devices: &DirectXDevices) -> Result<()> {
        self.renderer.write().handle_gpu_lost(directx_devices)
    }
}

impl PlatformTextSystem for DirectWriteTextSystem {
    fn add_fonts(&self, fonts: Vec<Cow<'static, [u8]>>) -> Result<()> {
        self.parley.add_fonts(fonts)
    }

    fn all_font_names(&self) -> Vec<String> {
        self.parley.all_font_names()
    }

    fn font_generation(&self) -> u64 {
        self.parley.font_generation()
    }

    fn font_id(&self, descriptor: &Font) -> Result<FontId> {
        self.parley.font_id(descriptor)
    }

    fn prewarm_fonts(&self, font_ids: &[FontId]) {
        self.parley.prewarm_fonts(font_ids);
    }

    fn font_metrics(&self, font_id: FontId) -> FontMetrics {
        self.parley.font_metrics(font_id)
    }

    fn typographic_bounds(&self, font_id: FontId, glyph_id: GlyphId) -> Result<Bounds<f32>> {
        self.parley.typographic_bounds(font_id, glyph_id)
    }

    fn advance(&self, font_id: FontId, glyph_id: GlyphId) -> Result<Size<f32>> {
        self.parley.advance(font_id, glyph_id)
    }

    fn glyph_for_char(&self, font_id: FontId, character: char) -> Option<GlyphId> {
        self.parley.glyph_for_char(font_id, character)
    }

    fn rasterize_glyph(&self, params: &RenderGlyphParams) -> Result<RasterizedGlyph> {
        self.parley.rasterize_glyph(params)
    }

    fn prepare_raster_style(&self, request: RasterStyleRequest) -> PreparedRasterStyle {
        self.parley.prepare_raster_style(request)
    }

    fn layout_text(&self, request: TextLayoutRequest<'_>) -> LineLayout {
        self.parley.layout_text(request)
    }

    fn layout_inline(&self, request: InlineLayoutRequest<'_>) -> InlineLayout {
        self.parley.layout_inline(request)
    }

    fn recommended_rendering_mode(&self, font_id: FontId, font_size: Pixels) -> TextRenderingMode {
        self.parley.recommended_rendering_mode(font_id, font_size)
    }
}

impl GlyphRasterizer for SharedDirectWriteRenderer {
    fn supports_color_glyph(&self, kind: ColorGlyphKind) -> bool {
        self.0.read().supports_color_glyph(kind)
    }

    fn prepare_style(&self, request: RasterStyleRequest) -> PreparedRasterStyle {
        self.0.read().prepare_style(request)
    }

    fn rasterize(
        &mut self,
        face: RasterFace<'_>,
        params: &RenderGlyphParams,
    ) -> Result<RasterizedGlyph> {
        self.0.write().rasterize(face, params)
    }

    fn recommended_mode(&self) -> TextRenderingMode {
        self.0.read().recommended_mode()
    }
}

impl DirectWriteGlyphRenderer {
    fn new(directx_devices: Option<&DirectXDevices>) -> Result<Self> {
        let factory: IDWriteFactory5 = unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED) }
            .context("creating the DirectWrite factory")?;
        let variable_factory = factory.cast().ok();
        let in_memory_loader = unsafe { factory.CreateInMemoryFontFileLoader() }
            .context("creating the DirectWrite in-memory font loader")?;
        unsafe { factory.RegisterFontFileLoader(&in_memory_loader) }
            .context("registering the DirectWrite in-memory font loader")?;
        let rendering_params = unsafe { factory.CreateRenderingParams() }
            .context("reading DirectWrite rendering parameters")?;

        Ok(Self {
            components: DirectWriteComponents {
                factory,
                in_memory_loader,
            },
            variable_factory,
            rendering_params,
            gpu_state: directx_devices.map(GPUState::new).transpose()?,
            faces: NativeFaceCache::default(),
            sources: HashMap::default(),
            system_subpixel_rendering: get_system_subpixel_rendering(),
        })
    }

    fn load_face(&mut self, face: RasterFace<'_>) -> Result<NativeFontId> {
        let factory = &self.components.factory;
        let loader = &self.components.in_memory_loader;
        let variable_factory = self.variable_factory.as_ref();
        let sources = &mut self.sources;

        self.faces.get_or_insert(face, |face| {
            let use_default_axes = face.variations.is_empty()
                || (variable_factory.is_none() && face.has_default_variations()?);

            ensure!(
                use_default_axes || variable_factory.is_some(),
                "this DirectWrite version cannot instantiate the selected variable-font coordinates"
            );

            let source = match sources.entry(face.source_id) {
                std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
                std::collections::hash_map::Entry::Vacant(entry) => entry.insert(
                    NativeSource::new(factory, loader, face.source)
                        .context("DirectWrite could not retain the font source")?,
                ),
            };

            NativeFace::new(
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
            })
        })
    }

    fn create_glyph_run_analysis(
        &self,
        components: &DirectWriteComponents,
        params: &NativeGlyphParams,
    ) -> Result<IDWriteGlyphRunAnalysis> {
        let font = &self.faces.fonts[params.font_id.0];
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

        let antialias_mode = if params.subpixel_rendering {
            DWRITE_TEXT_ANTIALIAS_MODE_CLEARTYPE
        } else {
            DWRITE_TEXT_ANTIALIAS_MODE_GRAYSCALE
        };

        let glyph_analysis = unsafe {
            components.factory.CreateGlyphRunAnalysis(
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

    fn raster_bounds(
        &self,
        components: &DirectWriteComponents,
        params: &NativeGlyphParams,
    ) -> Result<Bounds<DevicePixels>> {
        let glyph_analysis = self.create_glyph_run_analysis(components, params)?;

        let texture_type = if params.subpixel_rendering {
            DWRITE_TEXTURE_CLEARTYPE_3x1
        } else {
            DWRITE_TEXTURE_ALIASED_1x1
        };

        let bounds = unsafe { glyph_analysis.GetAlphaTextureBounds(texture_type)? };

        if bounds.right <= bounds.left || bounds.bottom <= bounds.top {
            Ok(Bounds {
                origin: point(0.into(), 0.into()),
                size: size(0.into(), 0.into()),
            })
        } else {
            Ok(Bounds {
                origin: point(bounds.left.into(), bounds.top.into()),
                size: size(
                    (bounds.right - bounds.left).into(),
                    (bounds.bottom - bounds.top).into(),
                ),
            })
        }
    }

    fn rasterize_glyph(
        &self,
        components: &DirectWriteComponents,
        params: &NativeGlyphParams,
        glyph_bounds: Bounds<DevicePixels>,
    ) -> Result<(Size<DevicePixels>, Vec<u8>)> {
        if glyph_bounds.size.width.0 == 0 || glyph_bounds.size.height.0 == 0 {
            anyhow::bail!("glyph bounds are empty");
        }

        let bitmap_data = if params.is_emoji {
            if let Ok(color) = self.rasterize_color(components, params, glyph_bounds) {
                color
            } else {
                let monochrome = self.rasterize_monochrome(components, params, glyph_bounds)?;
                monochrome
                    .into_iter()
                    .flat_map(|pixel| [0, 0, 0, pixel])
                    .collect::<Vec<_>>()
            }
        } else {
            self.rasterize_monochrome(components, params, glyph_bounds)?
        };

        Ok((glyph_bounds.size, bitmap_data))
    }

    fn rasterize_monochrome(
        &self,
        components: &DirectWriteComponents,
        params: &NativeGlyphParams,
        glyph_bounds: Bounds<DevicePixels>,
    ) -> Result<Vec<u8>> {
        let glyph_analysis = self.create_glyph_run_analysis(components, params)?;
        if !params.subpixel_rendering {
            let mut bitmap_data =
                vec![0u8; glyph_bounds.size.width.0 as usize * glyph_bounds.size.height.0 as usize];
            unsafe {
                glyph_analysis.CreateAlphaTexture(
                    DWRITE_TEXTURE_ALIASED_1x1,
                    &RECT {
                        left: glyph_bounds.origin.x.0,
                        top: glyph_bounds.origin.y.0,
                        right: glyph_bounds.size.width.0 + glyph_bounds.origin.x.0,
                        bottom: glyph_bounds.size.height.0 + glyph_bounds.origin.y.0,
                    },
                    &mut bitmap_data,
                )?;
            }

            return Ok(bitmap_data);
        }

        let width = glyph_bounds.size.width.0 as usize;
        let height = glyph_bounds.size.height.0 as usize;
        let pixel_count = width * height;

        let mut bitmap_data = vec![0u8; pixel_count * 4];

        unsafe {
            glyph_analysis.CreateAlphaTexture(
                DWRITE_TEXTURE_CLEARTYPE_3x1,
                &RECT {
                    left: glyph_bounds.origin.x.0,
                    top: glyph_bounds.origin.y.0,
                    right: glyph_bounds.size.width.0 + glyph_bounds.origin.x.0,
                    bottom: glyph_bounds.size.height.0 + glyph_bounds.origin.y.0,
                },
                &mut bitmap_data[..pixel_count * 3],
            )?;
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

    fn rasterize_color(
        &self,
        components: &DirectWriteComponents,
        params: &NativeGlyphParams,
        glyph_bounds: Bounds<DevicePixels>,
    ) -> Result<Vec<u8>> {
        // INVARIANT: the code below drives the *shared* D3D11 immediate context
        // (`Map`/`Unmap`/`Draw`/`CopyResource`), which `DirectXRenderer` and `DirectXAtlas` also
        // touch. An immediate `ID3D11DeviceContext` is not thread-safe, so this must only run on
        // the main UI thread (which it always is; text rasterization never leaves that thread).
        let gpu_state = self
            .gpu_state
            .as_ref()
            .context("D3D11 color-glyph compositing is unavailable")?;
        let bitmap_size = glyph_bounds.size;
        let subpixel_shift = params
            .subpixel_variant
            .map(|v| v as f32 / SUBPIXEL_VARIANTS_X as f32);
        let baseline_origin_x = subpixel_shift.x / params.scale_factor;
        let baseline_origin_y = subpixel_shift.y / params.scale_factor;

        let transform = DWRITE_MATRIX {
            m11: params.scale_factor,
            m12: 0.0,
            m21: 0.0,
            m22: params.scale_factor,
            dx: 0.0,
            dy: 0.0,
        };

        let font = &self.faces.fonts[params.font_id.0];
        let glyph_id = [params.glyph_id.0 as u16];
        let advance = [glyph_bounds.size.width.0 as f32];
        let offset = [DWRITE_GLYPH_OFFSET {
            advanceOffset: -glyph_bounds.origin.x.0 as f32 / params.scale_factor,
            ascenderOffset: glyph_bounds.origin.y.0 as f32 / params.scale_factor,
        }];
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

        // todo: support formats other than COLR
        let color_enumerator = unsafe {
            components.factory.TranslateColorGlyphRun(
                Vector2::new(baseline_origin_x, baseline_origin_y),
                &glyph_run,
                None,
                DWRITE_GLYPH_IMAGE_FORMATS_COLR,
                DWRITE_MEASURING_MODE_NATURAL,
                Some(&transform),
                0,
            )
        }?;

        let mut glyph_layers = Vec::new();
        let mut alpha_data = Vec::new();
        loop {
            let color_run = unsafe { color_enumerator.GetCurrentRun() }?;
            let color_run = unsafe { &*color_run };
            let image_format = color_run.glyphImageFormat & !DWRITE_GLYPH_IMAGE_FORMATS_TRUETYPE;
            if image_format == DWRITE_GLYPH_IMAGE_FORMATS_COLR {
                let color_analysis = unsafe {
                    components.factory.CreateGlyphRunAnalysis(
                        &color_run.Base.glyphRun as *const _,
                        Some(&transform),
                        DWRITE_RENDERING_MODE1_NATURAL_SYMMETRIC,
                        DWRITE_MEASURING_MODE_NATURAL,
                        DWRITE_GRID_FIT_MODE_DEFAULT,
                        DWRITE_TEXT_ANTIALIAS_MODE_GRAYSCALE,
                        baseline_origin_x,
                        baseline_origin_y,
                    )
                }?;

                let color_bounds =
                    unsafe { color_analysis.GetAlphaTextureBounds(DWRITE_TEXTURE_ALIASED_1x1) }?;

                let color_size = size(
                    color_bounds.right - color_bounds.left,
                    color_bounds.bottom - color_bounds.top,
                );
                if color_size.width > 0 && color_size.height > 0 {
                    alpha_data.clear();
                    alpha_data.resize((color_size.width * color_size.height) as usize, 0);
                    unsafe {
                        color_analysis.CreateAlphaTexture(
                            DWRITE_TEXTURE_ALIASED_1x1,
                            &color_bounds,
                            &mut alpha_data,
                        )
                    }?;

                    let run_color = {
                        let run_color = color_run.Base.runColor;
                        Rgba::new(run_color.r, run_color.g, run_color.b, run_color.a)
                    };
                    let bounds = bounds(point(color_bounds.left, color_bounds.top), color_size);
                    glyph_layers.push(GlyphLayerTexture::new(
                        gpu_state,
                        run_color,
                        bounds,
                        &alpha_data,
                    )?);
                }
            }

            let has_next = unsafe { color_enumerator.MoveNext() }
                .map(|e| e.as_bool())
                .unwrap_or(false);
            if !has_next {
                break;
            }
        }

        let render_target_texture = {
            let mut texture = None;
            let desc = D3D11_TEXTURE2D_DESC {
                Width: bitmap_size.width.0 as u32,
                Height: bitmap_size.height.0 as u32,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                Usage: D3D11_USAGE_DEFAULT,
                BindFlags: D3D11_BIND_RENDER_TARGET.0 as u32,
                CPUAccessFlags: 0,
                MiscFlags: 0,
            };
            unsafe {
                gpu_state
                    .device
                    .CreateTexture2D(&desc, None, Some(&mut texture))
            }?;
            texture.unwrap()
        };

        let render_target_view = {
            let desc = D3D11_RENDER_TARGET_VIEW_DESC {
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                ViewDimension: D3D11_RTV_DIMENSION_TEXTURE2D,
                Anonymous: D3D11_RENDER_TARGET_VIEW_DESC_0 {
                    Texture2D: D3D11_TEX2D_RTV { MipSlice: 0 },
                },
            };
            let mut rtv = None;
            unsafe {
                gpu_state.device.CreateRenderTargetView(
                    &render_target_texture,
                    Some(&desc),
                    Some(&mut rtv),
                )
            }?;
            rtv
        };

        Self::composite_color_layers(
            gpu_state,
            &glyph_layers,
            bitmap_size,
            &render_target_texture,
            &render_target_view,
        )
    }

    fn composite_color_layers(
        gpu_state: &GPUState,
        glyph_layers: &[GlyphLayerTexture],
        bitmap_size: Size<DevicePixels>,
        render_target_texture: &ID3D11Texture2D,
        render_target_view: &Option<ID3D11RenderTargetView>,
    ) -> Result<Vec<u8>> {
        let params_buffer = {
            let desc = D3D11_BUFFER_DESC {
                ByteWidth: std::mem::size_of::<GlyphLayerTextureParams>().next_multiple_of(16)
                    as u32,
                Usage: D3D11_USAGE_DYNAMIC,
                BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32,
                CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
                MiscFlags: 0,
                StructureByteStride: 0,
            };

            let mut buffer = None;
            unsafe {
                gpu_state
                    .device
                    .CreateBuffer(&desc, None, Some(&mut buffer))
            }?;
            buffer
        };

        let staging_texture = {
            let mut texture = None;
            let desc = D3D11_TEXTURE2D_DESC {
                Width: bitmap_size.width.0 as u32,
                Height: bitmap_size.height.0 as u32,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                Usage: D3D11_USAGE_STAGING,
                BindFlags: 0,
                CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                MiscFlags: 0,
            };
            unsafe {
                gpu_state
                    .device
                    .CreateTexture2D(&desc, None, Some(&mut texture))
            }?;
            texture.unwrap()
        };

        let device_context = &gpu_state.device_context;
        unsafe { device_context.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP) };
        unsafe { device_context.VSSetShader(&gpu_state.vertex_shader, None) };
        unsafe { device_context.PSSetShader(&gpu_state.pixel_shader, None) };
        unsafe {
            device_context.VSSetConstantBuffers(0, Some(std::slice::from_ref(&params_buffer)))
        };
        unsafe {
            device_context.PSSetConstantBuffers(0, Some(std::slice::from_ref(&params_buffer)))
        };
        unsafe {
            device_context.OMSetRenderTargets(Some(std::slice::from_ref(render_target_view)), None)
        };
        unsafe {
            if let Some(render_target_view) = render_target_view.as_ref() {
                device_context.ClearRenderTargetView(render_target_view, &[0.0, 0.0, 0.0, 0.0]);
            }
        }
        unsafe { device_context.PSSetSamplers(2, Some(std::slice::from_ref(&gpu_state.sampler))) };
        unsafe { device_context.OMSetBlendState(&gpu_state.blend_state, None, 0xffffffff) };

        let crate::FontInfo {
            gamma_ratios,
            grayscale_enhanced_contrast,
            ..
        } = DirectXRenderer::get_font_info();

        for layer in glyph_layers {
            let params = GlyphLayerTextureParams {
                run_color: vec4f(
                    layer.run_color.red,
                    layer.run_color.green,
                    layer.run_color.blue,
                    layer.run_color.alpha,
                ),
                gamma_ratios: vec4f(
                    gamma_ratios[0],
                    gamma_ratios[1],
                    gamma_ratios[2],
                    gamma_ratios[3],
                ),
                grayscale_enhanced_contrast: *grayscale_enhanced_contrast,
            };
            unsafe {
                let mut dest = std::mem::zeroed();
                gpu_state.device_context.Map(
                    params_buffer.as_ref().unwrap(),
                    0,
                    D3D11_MAP_WRITE_DISCARD,
                    0,
                    Some(&mut dest),
                )?;
                std::ptr::copy_nonoverlapping(&params as *const _, dest.pData as *mut _, 1);
                gpu_state
                    .device_context
                    .Unmap(params_buffer.as_ref().unwrap(), 0);
            };

            let texture = [Some(layer.texture_view.clone())];
            unsafe { device_context.PSSetShaderResources(1, Some(&texture)) };

            let viewport = [D3D11_VIEWPORT {
                TopLeftX: layer.bounds.origin.x as f32,
                TopLeftY: layer.bounds.origin.y as f32,
                Width: layer.bounds.size.width as f32,
                Height: layer.bounds.size.height as f32,
                MinDepth: 0.0,
                MaxDepth: 1.0,
            }];
            unsafe { device_context.RSSetViewports(Some(&viewport)) };

            unsafe { device_context.Draw(4, 0) };
        }

        unsafe { device_context.CopyResource(&staging_texture, render_target_texture) };

        let mapped_data = {
            let mut mapped_data = D3D11_MAPPED_SUBRESOURCE::default();
            unsafe {
                device_context.Map(
                    &staging_texture,
                    0,
                    D3D11_MAP_READ,
                    0,
                    Some(&mut mapped_data),
                )
            }?;
            mapped_data
        };
        let mut rasterized =
            vec![0u8; (bitmap_size.width.0 as u32 * bitmap_size.height.0 as u32 * 4) as usize];

        for y in 0..bitmap_size.height.0 as usize {
            let width = bitmap_size.width.0 as usize;
            unsafe {
                std::ptr::copy_nonoverlapping::<u8>(
                    (mapped_data.pData as *const u8).byte_add(mapped_data.RowPitch as usize * y),
                    rasterized
                        .as_mut_ptr()
                        .byte_add(width * y * std::mem::size_of::<u32>()),
                    width * std::mem::size_of::<u32>(),
                )
            };
        }

        // Release the mapping now that the rows have been copied out; leaving `staging_texture`
        // mapped would leak the mapping and keep the resource pinned for later reuse.
        unsafe { device_context.Unmap(&staging_texture, 0) };

        // Convert from premultiplied to straight alpha
        for chunk in rasterized.chunks_exact_mut(4) {
            let b = chunk[0] as f32;
            let g = chunk[1] as f32;
            let r = chunk[2] as f32;
            let a = chunk[3] as f32;
            if a > 0.0 {
                let inv_a = 255.0 / a;
                chunk[0] = (b * inv_a).clamp(0.0, 255.0) as u8;
                chunk[1] = (g * inv_a).clamp(0.0, 255.0) as u8;
                chunk[2] = (r * inv_a).clamp(0.0, 255.0) as u8;
            }
        }

        Ok(rasterized)
    }

    fn handle_gpu_lost(&mut self, directx_devices: &DirectXDevices) -> Result<()> {
        try_to_recover_from_device_lost(|| {
            GPUState::new(directx_devices).context("recreating GPU state for DirectWrite")
        })
        .map(|gpu_state| self.gpu_state = Some(gpu_state))
    }
}

impl Drop for DirectWriteGlyphRenderer {
    fn drop(&mut self) {
        self.faces.clear();
        self.sources.clear();

        unsafe {
            let _ = self
                .components
                .factory
                .UnregisterFontFileLoader(&self.components.in_memory_loader);
        }
    }
}

impl GlyphRasterizer for DirectWriteGlyphRenderer {
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

        let native_id = self.load_face(face)?;
        let native_params = NativeGlyphParams::from_parley(native_id, params);
        debug_assert_eq!(native_params.dilation, 0);
        let bounds = self.raster_bounds(&self.components, &native_params)?;

        if bounds.size.width.0 == 0 || bounds.size.height.0 == 0 {
            return Ok(RasterizedGlyph::empty(format));
        }

        let (bitmap_size, pixels) =
            self.rasterize_glyph(&self.components, &native_params, bounds)?;

        Ok(RasterizedGlyph {
            bounds: Bounds {
                origin: bounds.origin,
                size: bitmap_size,
            },
            size: bitmap_size,
            format,
            pixels,
        })
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

struct GlyphLayerTexture {
    run_color: Rgba,
    bounds: Bounds<i32>,
    texture_view: ID3D11ShaderResourceView,
    // holding on to the texture to not RAII drop it
    _texture: ID3D11Texture2D,
}

impl GlyphLayerTexture {
    fn new(
        gpu_state: &GPUState,
        run_color: Rgba,
        bounds: Bounds<i32>,
        alpha_data: &[u8],
    ) -> Result<Self> {
        let texture_size = bounds.size;

        let desc = D3D11_TEXTURE2D_DESC {
            Width: texture_size.width as u32,
            Height: texture_size.height as u32,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_R8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };

        let texture = {
            let mut texture: Option<ID3D11Texture2D> = None;
            unsafe {
                gpu_state
                    .device
                    .CreateTexture2D(&desc, None, Some(&mut texture))?
            };
            texture.unwrap()
        };
        let texture_view = {
            let mut view: Option<ID3D11ShaderResourceView> = None;
            unsafe {
                gpu_state
                    .device
                    .CreateShaderResourceView(&texture, None, Some(&mut view))?
            };
            view.unwrap()
        };

        unsafe {
            gpu_state.device_context.UpdateSubresource(
                &texture,
                0,
                None,
                alpha_data.as_ptr() as _,
                texture_size.width as u32,
                0,
            )
        };

        Ok(GlyphLayerTexture {
            run_color,
            bounds,
            texture_view,
            _texture: texture,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{
        FontStyle, FontWeight, ForegroundDependency, RasterizedGlyphFormat, SUBPIXEL_VARIANTS_Y,
        font, px, rgba,
    };
    use gpui_parley::FontSynthesis;

    const IBM_PLEX: &[u8] =
        include_bytes!("../../../assets/fonts/ibm-plex-sans/IBMPlexSans-Regular.ttf");
    const IBM_PLEX_ITALIC: &[u8] =
        include_bytes!("../../../assets/fonts/ibm-plex-sans/IBMPlexSans-Italic.ttf");
    const SOURCE_SERIF: &[u8] =
        include_bytes!("../../../assets/fonts/source-serif-4/SourceSerif4[opsz,wght].ttf");
    #[test]
    fn fixed_fonts_cover_native_modes_instances_and_empty_glyphs() -> Result<()> {
        let system = DirectWriteTextSystem::new_headless()?;
        system.add_fonts(vec![Cow::Borrowed(IBM_PLEX), Cow::Borrowed(SOURCE_SERIF)])?;

        let regular_id = system.font_id(&font("IBM Plex Sans"))?;
        let regular_glyph = system
            .glyph_for_char(regular_id, 'A')
            .context("IBM Plex Sans has no A glyph")?;

        for scale_factor in [1.0, 1.5, 2.0] {
            let grayscale = rasterize(
                &system,
                regular_id,
                regular_glyph,
                GlyphRenderMode::Grayscale,
                point(0, 0),
                scale_factor,
            )?;
            assert_eq!(grayscale.format, RasterizedGlyphFormat::AlphaMask);
            grayscale.validate()?;

            for subpixel_x in 0..SUBPIXEL_VARIANTS_X {
                for subpixel_y in 0..SUBPIXEL_VARIANTS_Y {
                    let subpixel = rasterize(
                        &system,
                        regular_id,
                        regular_glyph,
                        GlyphRenderMode::Subpixel,
                        point(subpixel_x, subpixel_y),
                        scale_factor,
                    )?;
                    assert_eq!(subpixel.format, RasterizedGlyphFormat::BgraSubpixelMask);
                    subpixel.validate()?;
                }
            }
        }

        let space = system
            .glyph_for_char(regular_id, ' ')
            .context("IBM Plex Sans has no space glyph")?;
        let empty = rasterize(
            &system,
            regular_id,
            space,
            GlyphRenderMode::Grayscale,
            point(0, 0),
            1.0,
        )?;
        assert_eq!(empty.size, Size::default());
        assert!(empty.pixels.is_empty());

        let monochrome_color = rasterize(
            &system,
            regular_id,
            regular_glyph,
            GlyphRenderMode::Color,
            point(0, 0),
            1.0,
        )?;
        assert_eq!(monochrome_color.format, RasterizedGlyphFormat::BgraColor);
        assert!(
            monochrome_color
                .pixels
                .chunks_exact(4)
                .all(|pixel| pixel[..3] == [0, 0, 0])
        );
        monochrome_color.validate()?;

        let synthesized_id = system.font_id(&font("IBM Plex Sans").bold().italic())?;
        let synthesized_glyph = system
            .glyph_for_char(synthesized_id, 'A')
            .context("synthesized IBM Plex Sans has no A glyph")?;
        let synthesized = rasterize(
            &system,
            synthesized_id,
            synthesized_glyph,
            GlyphRenderMode::Grayscale,
            point(0, 0),
            2.0,
        )?;
        assert!(!synthesized.pixels.is_empty());
        synthesized.validate()?;

        let mut light = font("Source Serif 4");
        light.weight = FontWeight::LIGHT;
        let mut bold = light.clone();
        bold.weight = FontWeight::BOLD;
        bold.style = FontStyle::Normal;
        let light_id = system.font_id(&light)?;
        let bold_id = system.font_id(&bold)?;
        let light_glyph = system
            .glyph_for_char(light_id, 'A')
            .context("light Source Serif has no A glyph")?;
        let bold_glyph = system
            .glyph_for_char(bold_id, 'A')
            .context("bold Source Serif has no A glyph")?;
        let light = rasterize(
            &system,
            light_id,
            light_glyph,
            GlyphRenderMode::Grayscale,
            point(0, 0),
            2.0,
        )?;
        let bold = rasterize(
            &system,
            bold_id,
            bold_glyph,
            GlyphRenderMode::Grayscale,
            point(0, 0),
            2.0,
        )?;
        assert_ne!((light.bounds, light.pixels), (bold.bounds, bold.pixels));

        Ok(())
    }

    #[test]
    fn direct_write_loads_a_nonzero_collection_face() -> Result<()> {
        let collection = test_collection(&[SOURCE_SERIF, IBM_PLEX, IBM_PLEX_ITALIC]);
        let source = FontDataBlob::from(collection);
        let mut renderer = DirectWriteGlyphRenderer::new(None)?;
        let native_id = renderer.load_face(RasterFace {
            font_id: FontId(1),
            source_id: source.id(),
            source: &source,
            face_index: 1,
            normalized_coords: &[],
            variations: &[],
            synthesis: FontSynthesis::default(),
            has_color_glyphs: false,
        })?;
        let codepoint = 'A' as u32;
        let mut glyph_id = 0;
        unsafe {
            renderer.faces.fonts[native_id.0].face.GetGlyphIndices(
                &raw const codepoint,
                1,
                &raw mut glyph_id,
            )?;
        }
        assert_ne!(glyph_id, 0);

        Ok(())
    }

    #[test]
    fn native_color_survives_device_recovery() -> Result<()> {
        let devices = DirectXDevices::new()?;
        let system = DirectWriteTextSystem::new(&devices)?;
        let font_id = system.font_id(&font("Segoe UI Emoji"))?;
        let glyph_id = system
            .glyph_for_char(font_id, '😀')
            .context("Segoe UI Emoji has no grinning-face glyph")?;
        let before = rasterize(
            &system,
            font_id,
            glyph_id,
            GlyphRenderMode::Color,
            point(0, 0),
            2.0,
        )?;
        assert_eq!(before.format, RasterizedGlyphFormat::BgraColor);
        assert!(before.pixels.chunks_exact(4).any(|pixel| {
            pixel[3] > 128
                && (pixel[0].abs_diff(pixel[1]) > 20
                    || pixel[1].abs_diff(pixel[2]) > 20
                    || pixel[0].abs_diff(pixel[2]) > 20)
        }));
        before.validate()?;

        let replacement_devices = DirectXDevices::new()?;
        system.handle_gpu_lost(&replacement_devices)?;
        let after = rasterize(
            &system,
            font_id,
            glyph_id,
            GlyphRenderMode::Color,
            point(0, 0),
            2.0,
        )?;
        after.validate()?;
        assert_eq!(before.bounds, after.bounds);
        assert_eq!(before.format, after.format);
        assert_eq!(before.pixels, after.pixels);

        Ok(())
    }

    fn rasterize(
        system: &DirectWriteTextSystem,
        font_id: FontId,
        glyph_id: GlyphId,
        mode: GlyphRenderMode,
        subpixel_variant: Point<u8>,
        scale_factor: f32,
    ) -> Result<RasterizedGlyph> {
        let raster_style = system.prepare_raster_style(RasterStyleRequest {
            font_id,
            glyph_id,
            scene_color: rgba(0xffffffff),
            requested_mode: mode,
            foreground_dependency: ForegroundDependency::Full,
        });

        system.rasterize_glyph(&RenderGlyphParams {
            font_id,
            glyph_id,
            font_size: px(24.0),
            subpixel_variant,
            scale_factor,
            raster_style,
        })
    }

    fn test_collection(faces: &[&[u8]]) -> Vec<u8> {
        let header_len = 12 + faces.len() * 4;
        let mut collection = vec![0; header_len];
        collection[..4].copy_from_slice(b"ttcf");
        collection[4..8].copy_from_slice(&0x0001_0000u32.to_be_bytes());
        collection[8..12].copy_from_slice(&(faces.len() as u32).to_be_bytes());

        for (face_idx, face) in faces.iter().enumerate() {
            while collection.len() % 4 != 0 {
                collection.push(0);
            }

            let face_offset = collection.len();
            let offset_position = 12 + face_idx * 4;
            collection[offset_position..offset_position + 4]
                .copy_from_slice(&(face_offset as u32).to_be_bytes());
            collection.extend_from_slice(face);

            let table_count = read_u16(face, 4).expect("font has an SFNT table count") as usize;
            for table_idx in 0..table_count {
                let table_offset_position = face_offset + 12 + table_idx * 16 + 8;
                let table_offset = read_u32(&collection, table_offset_position)
                    .expect("font has an SFNT table offset");
                let collection_offset = table_offset + face_offset as u32;
                collection[table_offset_position..table_offset_position + 4]
                    .copy_from_slice(&collection_offset.to_be_bytes());
            }
        }

        collection
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
