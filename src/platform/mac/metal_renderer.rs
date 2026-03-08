use super::{
    custom_draw::{
        ArgumentBufferBinding, MetalBufferSnapshot, MetalCustomComputePipeline,
        MetalCustomDrawRegistry, MetalCustomPipeline,
    },
    metal_atlas::MetalAtlas,
};
use crate::{
    AtlasTextureId, Background, Bounds, ContentMask, CustomBindingKind, CustomBindingValue,
    CustomBufferSource, CustomDraw, CustomFrameDiagnostics, CustomGpuFrameProfile,
    CustomIndexBuffer, CustomIndexFormat, CustomTextureId, DevicePixels, MonochromeSprite,
    PaintSurface, Path, Point, PolychromeSprite, PrimitiveBatch, Quad, ScaledPixels, Scene, Shadow,
    Size, Surface, Underline, point, size,
};
use anyhow::{Result, anyhow};
use block::ConcreteBlock;
use cocoa::{
    base::{NO, YES},
    foundation::{NSSize, NSUInteger},
    quartzcore::AutoresizingMask,
};

use core_foundation::base::TCFType;
use core_video::{
    metal_texture::CVMetalTextureGetTexture, metal_texture_cache::CVMetalTextureCache,
    pixel_buffer::kCVPixelFormatType_420YpCbCr8BiPlanarFullRange,
};
use foreign_types::{ForeignType, ForeignTypeRef};
use metal::{
    CAMetalLayer, CommandQueue, MTLPixelFormat, MTLResourceOptions, NSRange,
    RenderPassColorAttachmentDescriptorRef,
};
use objc::{self, msg_send, sel, sel_impl};
use parking_lot::Mutex;

use std::{cell::Cell, collections::BTreeMap, ffi::c_void, mem, ptr, sync::Arc, time::Instant};

// Exported to metal
pub(crate) type PointF = crate::Point<f32>;

#[cfg(not(feature = "runtime_shaders"))]
const SHADERS_METALLIB: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/shaders.metallib"));
#[cfg(feature = "runtime_shaders")]
const SHADERS_SOURCE_FILE: &str = include_str!(concat!(env!("OUT_DIR"), "/stitched_shaders.metal"));
// Use 4x MSAA, all devices support it.
// https://developer.apple.com/documentation/metal/mtldevice/1433355-supportstexturesamplecount
const PATH_SAMPLE_COUNT: u32 = 4;

pub type Context = Arc<Mutex<InstanceBufferPool>>;
pub type Renderer = MetalRenderer;

pub unsafe fn new_renderer(
    context: self::Context,
    _native_window: *mut c_void,
    _native_view: *mut c_void,
    _bounds: crate::Size<f32>,
    _transparent: bool,
) -> Renderer {
    MetalRenderer::new(context)
}

pub(crate) struct InstanceBufferPool {
    buffer_size: usize,
    buffers: Vec<metal::Buffer>,
}

impl Default for InstanceBufferPool {
    fn default() -> Self {
        Self {
            buffer_size: 2 * 1024 * 1024,
            buffers: Vec::new(),
        }
    }
}

pub(crate) struct InstanceBuffer {
    metal_buffer: metal::Buffer,
    size: usize,
}

impl InstanceBufferPool {
    pub(crate) fn reset(&mut self, buffer_size: usize) {
        self.buffer_size = buffer_size;
        self.buffers.clear();
    }

    pub(crate) fn acquire(&mut self, device: &metal::Device) -> InstanceBuffer {
        let buffer = self.buffers.pop().unwrap_or_else(|| {
            device.new_buffer(
                self.buffer_size as u64,
                MTLResourceOptions::StorageModeManaged,
            )
        });
        InstanceBuffer {
            metal_buffer: buffer,
            size: self.buffer_size,
        }
    }

    pub(crate) fn release(&mut self, buffer: InstanceBuffer) {
        if buffer.size == self.buffer_size {
            self.buffers.push(buffer.metal_buffer)
        }
    }
}

pub(crate) struct MetalRenderer {
    device: metal::Device,
    layer: metal::MetalLayer,
    presents_with_transaction: bool,
    command_queue: CommandQueue,
    paths_rasterization_pipeline_state: metal::RenderPipelineState,
    path_sprites_pipeline_state: metal::RenderPipelineState,
    shadows_pipeline_state: metal::RenderPipelineState,
    quads_pipeline_state: metal::RenderPipelineState,
    underlines_pipeline_state: metal::RenderPipelineState,
    monochrome_sprites_pipeline_state: metal::RenderPipelineState,
    polychrome_sprites_pipeline_state: metal::RenderPipelineState,
    surfaces_pipeline_state: metal::RenderPipelineState,
    unit_vertices: metal::Buffer,
    #[allow(clippy::arc_with_non_send_sync)]
    instance_buffer_pool: Arc<Mutex<InstanceBufferPool>>,
    sprite_atlas: Arc<MetalAtlas>,
    custom_draw: Arc<MetalCustomDrawRegistry>,
    core_video_texture_cache: core_video::metal_texture_cache::CVMetalTextureCache,
    path_intermediate_texture: Option<metal::Texture>,
    path_intermediate_msaa_texture: Option<metal::Texture>,
    path_sample_count: u32,
}

#[repr(C)]
pub struct PathRasterizationVertex {
    pub xy_position: Point<ScaledPixels>,
    pub st_position: Point<f32>,
    pub color: Background,
    pub bounds: Bounds<ScaledPixels>,
}

impl MetalRenderer {
    pub fn new(instance_buffer_pool: Arc<Mutex<InstanceBufferPool>>) -> Self {
        // Prefer low‐power integrated GPUs on Intel Mac. On Apple
        // Silicon, there is only ever one GPU, so this is equivalent to
        // `metal::Device::system_default()`.
        let device = if let Some(d) = metal::Device::all()
            .into_iter()
            .min_by_key(|d| (d.is_removable(), !d.is_low_power()))
        {
            d
        } else {
            // For some reason `all()` can return an empty list, see https://github.com/zed-industries/zed/issues/37689
            // In that case, we fall back to the system default device.
            log::error!(
                "Unable to enumerate Metal devices; attempting to use system default device"
            );
            metal::Device::system_default().unwrap_or_else(|| {
                log::error!("unable to access a compatible graphics device");
                std::process::exit(1);
            })
        };

        let layer = metal::MetalLayer::new();
        layer.set_device(&device);
        layer.set_pixel_format(MTLPixelFormat::BGRA8Unorm);
        layer.set_opaque(false);
        layer.set_maximum_drawable_count(3);
        unsafe {
            let _: () = msg_send![&*layer, setAllowsNextDrawableTimeout: NO];
            let _: () = msg_send![&*layer, setNeedsDisplayOnBoundsChange: YES];
            let _: () = msg_send![
                &*layer,
                setAutoresizingMask: AutoresizingMask::WIDTH_SIZABLE
                    | AutoresizingMask::HEIGHT_SIZABLE
            ];
        }
        #[cfg(feature = "runtime_shaders")]
        let library = device
            .new_library_with_source(&SHADERS_SOURCE_FILE, &metal::CompileOptions::new())
            .expect("error building metal library");
        #[cfg(not(feature = "runtime_shaders"))]
        let library = device
            .new_library_with_data(SHADERS_METALLIB)
            .expect("error building metal library");

        fn to_float2_bits(point: PointF) -> u64 {
            let mut output = point.y.to_bits() as u64;
            output <<= 32;
            output |= point.x.to_bits() as u64;
            output
        }

        let unit_vertices = [
            to_float2_bits(point(0., 0.)),
            to_float2_bits(point(1., 0.)),
            to_float2_bits(point(0., 1.)),
            to_float2_bits(point(0., 1.)),
            to_float2_bits(point(1., 0.)),
            to_float2_bits(point(1., 1.)),
        ];
        let unit_vertices = device.new_buffer_with_data(
            unit_vertices.as_ptr() as *const c_void,
            mem::size_of_val(&unit_vertices) as u64,
            MTLResourceOptions::StorageModeManaged,
        );

        let paths_rasterization_pipeline_state = build_path_rasterization_pipeline_state(
            &device,
            &library,
            "paths_rasterization",
            "path_rasterization_vertex",
            "path_rasterization_fragment",
            MTLPixelFormat::BGRA8Unorm,
            PATH_SAMPLE_COUNT,
        );
        let path_sprites_pipeline_state = build_path_sprite_pipeline_state(
            &device,
            &library,
            "path_sprites",
            "path_sprite_vertex",
            "path_sprite_fragment",
            MTLPixelFormat::BGRA8Unorm,
        );
        let shadows_pipeline_state = build_pipeline_state(
            &device,
            &library,
            "shadows",
            "shadow_vertex",
            "shadow_fragment",
            MTLPixelFormat::BGRA8Unorm,
        );
        let quads_pipeline_state = build_pipeline_state(
            &device,
            &library,
            "quads",
            "quad_vertex",
            "quad_fragment",
            MTLPixelFormat::BGRA8Unorm,
        );
        let underlines_pipeline_state = build_pipeline_state(
            &device,
            &library,
            "underlines",
            "underline_vertex",
            "underline_fragment",
            MTLPixelFormat::BGRA8Unorm,
        );
        let monochrome_sprites_pipeline_state = build_pipeline_state(
            &device,
            &library,
            "monochrome_sprites",
            "monochrome_sprite_vertex",
            "monochrome_sprite_fragment",
            MTLPixelFormat::BGRA8Unorm,
        );
        let polychrome_sprites_pipeline_state = build_pipeline_state(
            &device,
            &library,
            "polychrome_sprites",
            "polychrome_sprite_vertex",
            "polychrome_sprite_fragment",
            MTLPixelFormat::BGRA8Unorm,
        );
        let surfaces_pipeline_state = build_pipeline_state(
            &device,
            &library,
            "surfaces",
            "surface_vertex",
            "surface_fragment",
            MTLPixelFormat::BGRA8Unorm,
        );

        let command_queue = device.new_command_queue();
        let sprite_atlas = Arc::new(MetalAtlas::new(device.clone()));
        let custom_draw = Arc::new(MetalCustomDrawRegistry::new(
            device.clone(),
            MTLPixelFormat::BGRA8Unorm,
        ));
        let core_video_texture_cache =
            CVMetalTextureCache::new(None, device.clone(), None).unwrap();

        Self {
            device,
            layer,
            presents_with_transaction: false,
            command_queue,
            paths_rasterization_pipeline_state,
            path_sprites_pipeline_state,
            shadows_pipeline_state,
            quads_pipeline_state,
            underlines_pipeline_state,
            monochrome_sprites_pipeline_state,
            polychrome_sprites_pipeline_state,
            surfaces_pipeline_state,
            unit_vertices,
            instance_buffer_pool,
            sprite_atlas,
            custom_draw,
            core_video_texture_cache,
            path_intermediate_texture: None,
            path_intermediate_msaa_texture: None,
            path_sample_count: PATH_SAMPLE_COUNT,
        }
    }

    pub fn layer(&self) -> &metal::MetalLayerRef {
        &self.layer
    }

    pub fn layer_ptr(&self) -> *mut CAMetalLayer {
        self.layer.as_ptr()
    }

    pub fn sprite_atlas(&self) -> &Arc<MetalAtlas> {
        &self.sprite_atlas
    }

    pub fn custom_draw_registry(&self) -> Arc<dyn crate::CustomDrawRegistry> {
        self.custom_draw.clone()
    }

    pub fn set_presents_with_transaction(&mut self, presents_with_transaction: bool) {
        self.presents_with_transaction = presents_with_transaction;
        self.layer
            .set_presents_with_transaction(presents_with_transaction);
    }

    pub fn update_drawable_size(&mut self, size: Size<DevicePixels>) {
        let size = NSSize {
            width: size.width.0 as f64,
            height: size.height.0 as f64,
        };
        unsafe {
            let _: () = msg_send![
                self.layer(),
                setDrawableSize: size
            ];
        }
        let device_pixels_size = Size {
            width: DevicePixels(size.width as i32),
            height: DevicePixels(size.height as i32),
        };
        self.update_path_intermediate_textures(device_pixels_size);
    }

    fn update_path_intermediate_textures(&mut self, size: Size<DevicePixels>) {
        // We are uncertain when this happens, but sometimes size can be 0 here. Most likely before
        // the layout pass on window creation. Zero-sized texture creation causes SIGABRT.
        // https://github.com/zed-industries/zed/issues/36229
        if size.width.0 <= 0 || size.height.0 <= 0 {
            self.path_intermediate_texture = None;
            self.path_intermediate_msaa_texture = None;
            return;
        }

        let texture_descriptor = metal::TextureDescriptor::new();
        texture_descriptor.set_width(size.width.0 as u64);
        texture_descriptor.set_height(size.height.0 as u64);
        texture_descriptor.set_pixel_format(metal::MTLPixelFormat::BGRA8Unorm);
        texture_descriptor
            .set_usage(metal::MTLTextureUsage::RenderTarget | metal::MTLTextureUsage::ShaderRead);
        self.path_intermediate_texture = Some(self.device.new_texture(&texture_descriptor));

        if self.path_sample_count > 1 {
            let mut msaa_descriptor = texture_descriptor;
            msaa_descriptor.set_texture_type(metal::MTLTextureType::D2Multisample);
            msaa_descriptor.set_storage_mode(metal::MTLStorageMode::Private);
            msaa_descriptor.set_sample_count(self.path_sample_count as _);
            self.path_intermediate_msaa_texture = Some(self.device.new_texture(&msaa_descriptor));
        } else {
            self.path_intermediate_msaa_texture = None;
        }
    }

    pub fn update_transparency(&self, _transparent: bool) {
        // todo(mac)?
    }

    pub fn destroy(&self) {
        // nothing to do
    }

    pub fn draw(&mut self, scene: &Scene) {
        let layer = self.layer.clone();
        let viewport_size = layer.drawable_size();
        let viewport_size: Size<DevicePixels> = size(
            (viewport_size.width.ceil() as i32).into(),
            (viewport_size.height.ceil() as i32).into(),
        );
        let drawable = if let Some(drawable) = layer.next_drawable() {
            drawable
        } else {
            log::error!(
                "failed to retrieve next drawable, drawable size: {:?}",
                viewport_size
            );
            return;
        };

        let frame_encode_start = Instant::now();
        let mut retry_count = 0u32;
        loop {
            let mut instance_buffer = self.instance_buffer_pool.lock().acquire(&self.device);
            let custom_gpu_profile = if self.custom_draw.gpu_profiling_enabled() {
                build_custom_gpu_profile(scene)
            } else {
                None
            };
            let custom_frame_diagnostics = if self.custom_draw.frame_diagnostics_enabled() {
                build_custom_frame_diagnostics(scene)
            } else {
                None
            };

            let command_buffer =
                self.draw_primitives(scene, &mut instance_buffer, drawable, viewport_size);

            match command_buffer {
                Ok(command_buffer) => {
                    let custom_draw = self.custom_draw.clone();
                    let instance_buffer_pool = self.instance_buffer_pool.clone();
                    let instance_buffer = Cell::new(Some(instance_buffer));
                    let cpu_encode_time_ns =
                        duration_as_u64_nanoseconds(frame_encode_start.elapsed());
                    let custom_frame_diagnostics =
                        custom_frame_diagnostics.map(|mut diagnostics| {
                            diagnostics.retry_count = retry_count;
                            diagnostics.cpu_encode_time_ns = cpu_encode_time_ns;
                            diagnostics
                        });
                    let submit_instant = Arc::new(std::sync::Mutex::new(None::<Instant>));
                    let scheduled_instant = Arc::new(std::sync::Mutex::new(None::<Instant>));

                    if custom_frame_diagnostics.is_some() {
                        let scheduled_instant = Arc::clone(&scheduled_instant);
                        let scheduled_block = ConcreteBlock::new(move |_| {
                            if let Ok(mut scheduled_value) = scheduled_instant.lock() {
                                if scheduled_value.is_none() {
                                    *scheduled_value = Some(Instant::now());
                                }
                            }
                        });
                        let scheduled_block = scheduled_block.copy();
                        command_buffer.add_scheduled_handler(&scheduled_block);
                    }

                    let completed_submit_instant = Arc::clone(&submit_instant);
                    let completed_scheduled_instant = Arc::clone(&scheduled_instant);
                    let block = ConcreteBlock::new(move |completed_command_buffer| {
                        if let Some(instance_buffer) = instance_buffer.take() {
                            instance_buffer_pool.lock().release(instance_buffer);
                        }

                        let gpu_time_ns =
                            metal_command_buffer_gpu_time_ns(completed_command_buffer);
                        if let Some(mut custom_gpu_profile) = custom_gpu_profile {
                            custom_gpu_profile.gpu_time_ns = gpu_time_ns;
                            custom_draw.record_gpu_profile(custom_gpu_profile);
                        }

                        if let Some(mut custom_frame_diagnostics) = custom_frame_diagnostics {
                            let completed_instant = Instant::now();
                            let submit_instant = completed_submit_instant
                                .lock()
                                .ok()
                                .and_then(|value| *value);
                            let scheduled_instant = completed_scheduled_instant
                                .lock()
                                .ok()
                                .and_then(|value| *value);

                            custom_frame_diagnostics.gpu_time_ns = gpu_time_ns;
                            custom_frame_diagnostics.submit_to_scheduled_ns = submit_instant
                                .and_then(|submit| {
                                    scheduled_instant.and_then(|scheduled| {
                                        scheduled
                                            .checked_duration_since(submit)
                                            .map(duration_as_u64_nanoseconds)
                                    })
                                });
                            custom_frame_diagnostics.submit_to_completed_ns = submit_instant
                                .and_then(|submit| {
                                    completed_instant
                                        .checked_duration_since(submit)
                                        .map(duration_as_u64_nanoseconds)
                                });
                            custom_frame_diagnostics.scheduled_to_completed_ns = scheduled_instant
                                .and_then(|scheduled| {
                                    completed_instant
                                        .checked_duration_since(scheduled)
                                        .map(duration_as_u64_nanoseconds)
                                });
                            custom_draw.record_frame_diagnostics(custom_frame_diagnostics);
                        }
                    });
                    let block = block.copy();
                    command_buffer.add_completed_handler(&block);

                    if custom_frame_diagnostics.is_some() {
                        if let Ok(mut submit_value) = submit_instant.lock() {
                            *submit_value = Some(Instant::now());
                        }
                    }

                    if self.presents_with_transaction {
                        command_buffer.commit();
                        command_buffer.wait_until_scheduled();
                        drawable.present();
                    } else {
                        command_buffer.present_drawable(drawable);
                        command_buffer.commit();
                    }
                    return;
                }
                Err(err) => {
                    retry_count = retry_count.saturating_add(1);
                    log::error!(
                        "failed to render: {}. retrying with larger instance buffer size",
                        err
                    );
                    let mut instance_buffer_pool = self.instance_buffer_pool.lock();
                    let buffer_size = instance_buffer_pool.buffer_size;
                    if buffer_size >= 256 * 1024 * 1024 {
                        log::error!("instance buffer size grew too large: {}", buffer_size);
                        break;
                    }
                    instance_buffer_pool.reset(buffer_size * 2);
                    log::info!(
                        "increased instance buffer size to {}",
                        instance_buffer_pool.buffer_size
                    );
                }
            }
        }
    }

    fn draw_primitives(
        &mut self,
        scene: &Scene,
        instance_buffer: &mut InstanceBuffer,
        drawable: &metal::MetalDrawableRef,
        viewport_size: Size<DevicePixels>,
    ) -> Result<metal::CommandBuffer> {
        let command_queue = self.command_queue.clone();
        let command_buffer = command_queue.new_command_buffer();
        let alpha = if self.layer.is_opaque() { 1. } else { 0. };
        let mut instance_offset = 0;

        self.dispatch_custom_computes(
            scene,
            command_buffer,
            instance_buffer,
            &mut instance_offset,
        )?;
        self.draw_custom_render_targets(
            scene,
            command_buffer,
            instance_buffer,
            &mut instance_offset,
        )?;

        let mut command_encoder = new_command_encoder(
            command_buffer,
            drawable,
            viewport_size,
            |color_attachment| {
                color_attachment.set_load_action(metal::MTLLoadAction::Clear);
                color_attachment.set_clear_color(metal::MTLClearColor::new(0., 0., 0., alpha));
            },
        );

        for batch in scene.batches() {
            let ok = match batch {
                PrimitiveBatch::Shadows(shadows) => self.draw_shadows(
                    shadows,
                    instance_buffer,
                    &mut instance_offset,
                    viewport_size,
                    command_encoder,
                ),
                PrimitiveBatch::Quads(quads) => self.draw_quads(
                    quads,
                    instance_buffer,
                    &mut instance_offset,
                    viewport_size,
                    command_encoder,
                ),
                PrimitiveBatch::Paths(paths) => {
                    command_encoder.end_encoding();

                    let did_draw = self.draw_paths_to_intermediate(
                        paths,
                        instance_buffer,
                        &mut instance_offset,
                        viewport_size,
                        command_buffer,
                    );

                    command_encoder = new_command_encoder(
                        command_buffer,
                        drawable,
                        viewport_size,
                        |color_attachment| {
                            color_attachment.set_load_action(metal::MTLLoadAction::Load);
                        },
                    );

                    if did_draw {
                        self.draw_paths_from_intermediate(
                            paths,
                            instance_buffer,
                            &mut instance_offset,
                            viewport_size,
                            command_encoder,
                        )
                    } else {
                        false
                    }
                }
                PrimitiveBatch::Underlines(underlines) => self.draw_underlines(
                    underlines,
                    instance_buffer,
                    &mut instance_offset,
                    viewport_size,
                    command_encoder,
                ),
                PrimitiveBatch::MonochromeSprites {
                    texture_id,
                    sprites,
                } => self.draw_monochrome_sprites(
                    texture_id,
                    sprites,
                    instance_buffer,
                    &mut instance_offset,
                    viewport_size,
                    command_encoder,
                ),
                PrimitiveBatch::PolychromeSprites {
                    texture_id,
                    sprites,
                } => self.draw_polychrome_sprites(
                    texture_id,
                    sprites,
                    instance_buffer,
                    &mut instance_offset,
                    viewport_size,
                    command_encoder,
                ),
                PrimitiveBatch::Surfaces(surfaces) => self.draw_surfaces(
                    surfaces,
                    instance_buffer,
                    &mut instance_offset,
                    viewport_size,
                    command_encoder,
                ),
                PrimitiveBatch::CustomDraws(draws) => self.draw_custom_draws(
                    draws,
                    instance_buffer,
                    &mut instance_offset,
                    command_encoder,
                ),
            };
            if !ok {
                command_encoder.end_encoding();
                anyhow::bail!(
                    "scene too large: {} paths, {} shadows, {} quads, {} underlines, {} mono, {} poly, {} surfaces",
                    scene.paths.len(),
                    scene.shadows.len(),
                    scene.quads.len(),
                    scene.underlines.len(),
                    scene.monochrome_sprites.len(),
                    scene.polychrome_sprites.len(),
                    scene.surfaces.len(),
                );
            }
        }

        command_encoder.end_encoding();

        instance_buffer.metal_buffer.did_modify_range(NSRange {
            location: 0,
            length: instance_offset as NSUInteger,
        });
        Ok(command_buffer.to_owned())
    }

    fn draw_paths_to_intermediate(
        &self,
        paths: &[Path<ScaledPixels>],
        instance_buffer: &mut InstanceBuffer,
        instance_offset: &mut usize,
        viewport_size: Size<DevicePixels>,
        command_buffer: &metal::CommandBufferRef,
    ) -> bool {
        if paths.is_empty() {
            return true;
        }
        let Some(intermediate_texture) = &self.path_intermediate_texture else {
            return false;
        };

        let render_pass_descriptor = metal::RenderPassDescriptor::new();
        let color_attachment = render_pass_descriptor
            .color_attachments()
            .object_at(0)
            .unwrap();
        color_attachment.set_load_action(metal::MTLLoadAction::Clear);
        color_attachment.set_clear_color(metal::MTLClearColor::new(0., 0., 0., 0.));

        if let Some(msaa_texture) = &self.path_intermediate_msaa_texture {
            color_attachment.set_texture(Some(msaa_texture));
            color_attachment.set_resolve_texture(Some(intermediate_texture));
            color_attachment.set_store_action(metal::MTLStoreAction::MultisampleResolve);
        } else {
            color_attachment.set_texture(Some(intermediate_texture));
            color_attachment.set_store_action(metal::MTLStoreAction::Store);
        }

        let command_encoder = command_buffer.new_render_command_encoder(render_pass_descriptor);
        command_encoder.set_render_pipeline_state(&self.paths_rasterization_pipeline_state);

        align_offset(instance_offset);
        let mut vertices = Vec::new();
        for path in paths {
            vertices.extend(path.vertices.iter().map(|v| PathRasterizationVertex {
                xy_position: v.xy_position,
                st_position: v.st_position,
                color: path.color,
                bounds: path.bounds.intersect(&path.content_mask.bounds),
            }));
        }
        let vertices_bytes_len = mem::size_of_val(vertices.as_slice());
        let next_offset = *instance_offset + vertices_bytes_len;
        if next_offset > instance_buffer.size {
            command_encoder.end_encoding();
            return false;
        }
        command_encoder.set_vertex_buffer(
            PathRasterizationInputIndex::Vertices as u64,
            Some(&instance_buffer.metal_buffer),
            *instance_offset as u64,
        );
        command_encoder.set_vertex_bytes(
            PathRasterizationInputIndex::ViewportSize as u64,
            mem::size_of_val(&viewport_size) as u64,
            &viewport_size as *const Size<DevicePixels> as *const _,
        );
        command_encoder.set_fragment_buffer(
            PathRasterizationInputIndex::Vertices as u64,
            Some(&instance_buffer.metal_buffer),
            *instance_offset as u64,
        );
        let buffer_contents =
            unsafe { (instance_buffer.metal_buffer.contents() as *mut u8).add(*instance_offset) };
        unsafe {
            ptr::copy_nonoverlapping(
                vertices.as_ptr() as *const u8,
                buffer_contents,
                vertices_bytes_len,
            );
        }
        command_encoder.draw_primitives(
            metal::MTLPrimitiveType::Triangle,
            0,
            vertices.len() as u64,
        );
        *instance_offset = next_offset;

        command_encoder.end_encoding();
        true
    }

    fn draw_shadows(
        &self,
        shadows: &[Shadow],
        instance_buffer: &mut InstanceBuffer,
        instance_offset: &mut usize,
        viewport_size: Size<DevicePixels>,
        command_encoder: &metal::RenderCommandEncoderRef,
    ) -> bool {
        if shadows.is_empty() {
            return true;
        }
        align_offset(instance_offset);

        command_encoder.set_render_pipeline_state(&self.shadows_pipeline_state);
        command_encoder.set_vertex_buffer(
            ShadowInputIndex::Vertices as u64,
            Some(&self.unit_vertices),
            0,
        );
        command_encoder.set_vertex_buffer(
            ShadowInputIndex::Shadows as u64,
            Some(&instance_buffer.metal_buffer),
            *instance_offset as u64,
        );
        command_encoder.set_fragment_buffer(
            ShadowInputIndex::Shadows as u64,
            Some(&instance_buffer.metal_buffer),
            *instance_offset as u64,
        );

        command_encoder.set_vertex_bytes(
            ShadowInputIndex::ViewportSize as u64,
            mem::size_of_val(&viewport_size) as u64,
            &viewport_size as *const Size<DevicePixels> as *const _,
        );

        let shadow_bytes_len = mem::size_of_val(shadows);
        let buffer_contents =
            unsafe { (instance_buffer.metal_buffer.contents() as *mut u8).add(*instance_offset) };

        let next_offset = *instance_offset + shadow_bytes_len;
        if next_offset > instance_buffer.size {
            return false;
        }

        unsafe {
            ptr::copy_nonoverlapping(
                shadows.as_ptr() as *const u8,
                buffer_contents,
                shadow_bytes_len,
            );
        }

        command_encoder.draw_primitives_instanced(
            metal::MTLPrimitiveType::Triangle,
            0,
            6,
            shadows.len() as u64,
        );
        *instance_offset = next_offset;
        true
    }

    fn draw_quads(
        &self,
        quads: &[Quad],
        instance_buffer: &mut InstanceBuffer,
        instance_offset: &mut usize,
        viewport_size: Size<DevicePixels>,
        command_encoder: &metal::RenderCommandEncoderRef,
    ) -> bool {
        if quads.is_empty() {
            return true;
        }
        align_offset(instance_offset);

        command_encoder.set_render_pipeline_state(&self.quads_pipeline_state);
        command_encoder.set_vertex_buffer(
            QuadInputIndex::Vertices as u64,
            Some(&self.unit_vertices),
            0,
        );
        command_encoder.set_vertex_buffer(
            QuadInputIndex::Quads as u64,
            Some(&instance_buffer.metal_buffer),
            *instance_offset as u64,
        );
        command_encoder.set_fragment_buffer(
            QuadInputIndex::Quads as u64,
            Some(&instance_buffer.metal_buffer),
            *instance_offset as u64,
        );

        command_encoder.set_vertex_bytes(
            QuadInputIndex::ViewportSize as u64,
            mem::size_of_val(&viewport_size) as u64,
            &viewport_size as *const Size<DevicePixels> as *const _,
        );

        let quad_bytes_len = mem::size_of_val(quads);
        let buffer_contents =
            unsafe { (instance_buffer.metal_buffer.contents() as *mut u8).add(*instance_offset) };

        let next_offset = *instance_offset + quad_bytes_len;
        if next_offset > instance_buffer.size {
            return false;
        }

        unsafe {
            ptr::copy_nonoverlapping(quads.as_ptr() as *const u8, buffer_contents, quad_bytes_len);
        }

        command_encoder.draw_primitives_instanced(
            metal::MTLPrimitiveType::Triangle,
            0,
            6,
            quads.len() as u64,
        );
        *instance_offset = next_offset;
        true
    }

    fn draw_paths_from_intermediate(
        &self,
        paths: &[Path<ScaledPixels>],
        instance_buffer: &mut InstanceBuffer,
        instance_offset: &mut usize,
        viewport_size: Size<DevicePixels>,
        command_encoder: &metal::RenderCommandEncoderRef,
    ) -> bool {
        let Some(first_path) = paths.first() else {
            return true;
        };

        let Some(ref intermediate_texture) = self.path_intermediate_texture else {
            return false;
        };

        command_encoder.set_render_pipeline_state(&self.path_sprites_pipeline_state);
        command_encoder.set_vertex_buffer(
            SpriteInputIndex::Vertices as u64,
            Some(&self.unit_vertices),
            0,
        );
        command_encoder.set_vertex_bytes(
            SpriteInputIndex::ViewportSize as u64,
            mem::size_of_val(&viewport_size) as u64,
            &viewport_size as *const Size<DevicePixels> as *const _,
        );

        command_encoder.set_fragment_texture(
            SpriteInputIndex::AtlasTexture as u64,
            Some(intermediate_texture),
        );

        // When copying paths from the intermediate texture to the drawable,
        // each pixel must only be copied once, in case of transparent paths.
        //
        // If all paths have the same draw order, then their bounds are all
        // disjoint, so we can copy each path's bounds individually. If this
        // batch combines different draw orders, we perform a single copy
        // for a minimal spanning rect.
        let sprites;
        if paths.last().unwrap().order == first_path.order {
            sprites = paths
                .iter()
                .map(|path| PathSprite {
                    bounds: path.clipped_bounds(),
                })
                .collect();
        } else {
            let mut bounds = first_path.clipped_bounds();
            for path in paths.iter().skip(1) {
                bounds = bounds.union(&path.clipped_bounds());
            }
            sprites = vec![PathSprite { bounds }];
        }

        align_offset(instance_offset);
        let sprite_bytes_len = mem::size_of_val(sprites.as_slice());
        let next_offset = *instance_offset + sprite_bytes_len;
        if next_offset > instance_buffer.size {
            return false;
        }

        command_encoder.set_vertex_buffer(
            SpriteInputIndex::Sprites as u64,
            Some(&instance_buffer.metal_buffer),
            *instance_offset as u64,
        );

        let buffer_contents =
            unsafe { (instance_buffer.metal_buffer.contents() as *mut u8).add(*instance_offset) };
        unsafe {
            ptr::copy_nonoverlapping(
                sprites.as_ptr() as *const u8,
                buffer_contents,
                sprite_bytes_len,
            );
        }

        command_encoder.draw_primitives_instanced(
            metal::MTLPrimitiveType::Triangle,
            0,
            6,
            sprites.len() as u64,
        );
        *instance_offset = next_offset;

        true
    }

    fn draw_underlines(
        &self,
        underlines: &[Underline],
        instance_buffer: &mut InstanceBuffer,
        instance_offset: &mut usize,
        viewport_size: Size<DevicePixels>,
        command_encoder: &metal::RenderCommandEncoderRef,
    ) -> bool {
        if underlines.is_empty() {
            return true;
        }
        align_offset(instance_offset);

        command_encoder.set_render_pipeline_state(&self.underlines_pipeline_state);
        command_encoder.set_vertex_buffer(
            UnderlineInputIndex::Vertices as u64,
            Some(&self.unit_vertices),
            0,
        );
        command_encoder.set_vertex_buffer(
            UnderlineInputIndex::Underlines as u64,
            Some(&instance_buffer.metal_buffer),
            *instance_offset as u64,
        );
        command_encoder.set_fragment_buffer(
            UnderlineInputIndex::Underlines as u64,
            Some(&instance_buffer.metal_buffer),
            *instance_offset as u64,
        );

        command_encoder.set_vertex_bytes(
            UnderlineInputIndex::ViewportSize as u64,
            mem::size_of_val(&viewport_size) as u64,
            &viewport_size as *const Size<DevicePixels> as *const _,
        );

        let underline_bytes_len = mem::size_of_val(underlines);
        let buffer_contents =
            unsafe { (instance_buffer.metal_buffer.contents() as *mut u8).add(*instance_offset) };

        let next_offset = *instance_offset + underline_bytes_len;
        if next_offset > instance_buffer.size {
            return false;
        }

        unsafe {
            ptr::copy_nonoverlapping(
                underlines.as_ptr() as *const u8,
                buffer_contents,
                underline_bytes_len,
            );
        }

        command_encoder.draw_primitives_instanced(
            metal::MTLPrimitiveType::Triangle,
            0,
            6,
            underlines.len() as u64,
        );
        *instance_offset = next_offset;
        true
    }

    fn draw_monochrome_sprites(
        &self,
        texture_id: AtlasTextureId,
        sprites: &[MonochromeSprite],
        instance_buffer: &mut InstanceBuffer,
        instance_offset: &mut usize,
        viewport_size: Size<DevicePixels>,
        command_encoder: &metal::RenderCommandEncoderRef,
    ) -> bool {
        if sprites.is_empty() {
            return true;
        }
        align_offset(instance_offset);

        let sprite_bytes_len = mem::size_of_val(sprites);
        let buffer_contents =
            unsafe { (instance_buffer.metal_buffer.contents() as *mut u8).add(*instance_offset) };

        let next_offset = *instance_offset + sprite_bytes_len;
        if next_offset > instance_buffer.size {
            return false;
        }

        let texture = self.sprite_atlas.metal_texture(texture_id);
        let texture_size = size(
            DevicePixels(texture.width() as i32),
            DevicePixels(texture.height() as i32),
        );
        command_encoder.set_render_pipeline_state(&self.monochrome_sprites_pipeline_state);
        command_encoder.set_vertex_buffer(
            SpriteInputIndex::Vertices as u64,
            Some(&self.unit_vertices),
            0,
        );
        command_encoder.set_vertex_buffer(
            SpriteInputIndex::Sprites as u64,
            Some(&instance_buffer.metal_buffer),
            *instance_offset as u64,
        );
        command_encoder.set_vertex_bytes(
            SpriteInputIndex::ViewportSize as u64,
            mem::size_of_val(&viewport_size) as u64,
            &viewport_size as *const Size<DevicePixels> as *const _,
        );
        command_encoder.set_vertex_bytes(
            SpriteInputIndex::AtlasTextureSize as u64,
            mem::size_of_val(&texture_size) as u64,
            &texture_size as *const Size<DevicePixels> as *const _,
        );
        command_encoder.set_fragment_buffer(
            SpriteInputIndex::Sprites as u64,
            Some(&instance_buffer.metal_buffer),
            *instance_offset as u64,
        );
        command_encoder.set_fragment_texture(SpriteInputIndex::AtlasTexture as u64, Some(&texture));

        unsafe {
            ptr::copy_nonoverlapping(
                sprites.as_ptr() as *const u8,
                buffer_contents,
                sprite_bytes_len,
            );
        }

        command_encoder.draw_primitives_instanced(
            metal::MTLPrimitiveType::Triangle,
            0,
            6,
            sprites.len() as u64,
        );
        *instance_offset = next_offset;
        true
    }

    fn draw_polychrome_sprites(
        &self,
        texture_id: AtlasTextureId,
        sprites: &[PolychromeSprite],
        instance_buffer: &mut InstanceBuffer,
        instance_offset: &mut usize,
        viewport_size: Size<DevicePixels>,
        command_encoder: &metal::RenderCommandEncoderRef,
    ) -> bool {
        if sprites.is_empty() {
            return true;
        }
        align_offset(instance_offset);

        let texture = self.sprite_atlas.metal_texture(texture_id);
        let texture_size = size(
            DevicePixels(texture.width() as i32),
            DevicePixels(texture.height() as i32),
        );
        command_encoder.set_render_pipeline_state(&self.polychrome_sprites_pipeline_state);
        command_encoder.set_vertex_buffer(
            SpriteInputIndex::Vertices as u64,
            Some(&self.unit_vertices),
            0,
        );
        command_encoder.set_vertex_buffer(
            SpriteInputIndex::Sprites as u64,
            Some(&instance_buffer.metal_buffer),
            *instance_offset as u64,
        );
        command_encoder.set_vertex_bytes(
            SpriteInputIndex::ViewportSize as u64,
            mem::size_of_val(&viewport_size) as u64,
            &viewport_size as *const Size<DevicePixels> as *const _,
        );
        command_encoder.set_vertex_bytes(
            SpriteInputIndex::AtlasTextureSize as u64,
            mem::size_of_val(&texture_size) as u64,
            &texture_size as *const Size<DevicePixels> as *const _,
        );
        command_encoder.set_fragment_buffer(
            SpriteInputIndex::Sprites as u64,
            Some(&instance_buffer.metal_buffer),
            *instance_offset as u64,
        );
        command_encoder.set_fragment_texture(SpriteInputIndex::AtlasTexture as u64, Some(&texture));

        let sprite_bytes_len = mem::size_of_val(sprites);
        let buffer_contents =
            unsafe { (instance_buffer.metal_buffer.contents() as *mut u8).add(*instance_offset) };

        let next_offset = *instance_offset + sprite_bytes_len;
        if next_offset > instance_buffer.size {
            return false;
        }

        unsafe {
            ptr::copy_nonoverlapping(
                sprites.as_ptr() as *const u8,
                buffer_contents,
                sprite_bytes_len,
            );
        }

        command_encoder.draw_primitives_instanced(
            metal::MTLPrimitiveType::Triangle,
            0,
            6,
            sprites.len() as u64,
        );
        *instance_offset = next_offset;
        true
    }

    fn draw_surfaces(
        &mut self,
        surfaces: &[PaintSurface],
        instance_buffer: &mut InstanceBuffer,
        instance_offset: &mut usize,
        viewport_size: Size<DevicePixels>,
        command_encoder: &metal::RenderCommandEncoderRef,
    ) -> bool {
        command_encoder.set_render_pipeline_state(&self.surfaces_pipeline_state);
        command_encoder.set_vertex_buffer(
            SurfaceInputIndex::Vertices as u64,
            Some(&self.unit_vertices),
            0,
        );
        command_encoder.set_vertex_bytes(
            SurfaceInputIndex::ViewportSize as u64,
            mem::size_of_val(&viewport_size) as u64,
            &viewport_size as *const Size<DevicePixels> as *const _,
        );

        for surface in surfaces {
            let texture_size = size(
                DevicePixels::from(surface.image_buffer.get_width() as i32),
                DevicePixels::from(surface.image_buffer.get_height() as i32),
            );

            assert_eq!(
                surface.image_buffer.get_pixel_format(),
                kCVPixelFormatType_420YpCbCr8BiPlanarFullRange
            );

            let y_texture = self
                .core_video_texture_cache
                .create_texture_from_image(
                    surface.image_buffer.as_concrete_TypeRef(),
                    None,
                    MTLPixelFormat::R8Unorm,
                    surface.image_buffer.get_width_of_plane(0),
                    surface.image_buffer.get_height_of_plane(0),
                    0,
                )
                .unwrap();
            let cb_cr_texture = self
                .core_video_texture_cache
                .create_texture_from_image(
                    surface.image_buffer.as_concrete_TypeRef(),
                    None,
                    MTLPixelFormat::RG8Unorm,
                    surface.image_buffer.get_width_of_plane(1),
                    surface.image_buffer.get_height_of_plane(1),
                    1,
                )
                .unwrap();

            align_offset(instance_offset);
            let next_offset = *instance_offset + mem::size_of::<Surface>();
            if next_offset > instance_buffer.size {
                return false;
            }

            command_encoder.set_vertex_buffer(
                SurfaceInputIndex::Surfaces as u64,
                Some(&instance_buffer.metal_buffer),
                *instance_offset as u64,
            );
            command_encoder.set_vertex_bytes(
                SurfaceInputIndex::TextureSize as u64,
                mem::size_of_val(&texture_size) as u64,
                &texture_size as *const Size<DevicePixels> as *const _,
            );
            // let y_texture = y_texture.get_texture().unwrap().
            command_encoder.set_fragment_texture(SurfaceInputIndex::YTexture as u64, unsafe {
                let texture = CVMetalTextureGetTexture(y_texture.as_concrete_TypeRef());
                Some(metal::TextureRef::from_ptr(texture as *mut _))
            });
            command_encoder.set_fragment_texture(SurfaceInputIndex::CbCrTexture as u64, unsafe {
                let texture = CVMetalTextureGetTexture(cb_cr_texture.as_concrete_TypeRef());
                Some(metal::TextureRef::from_ptr(texture as *mut _))
            });

            unsafe {
                let buffer_contents = (instance_buffer.metal_buffer.contents() as *mut u8)
                    .add(*instance_offset)
                    as *mut SurfaceBounds;
                ptr::write(
                    buffer_contents,
                    SurfaceBounds {
                        bounds: surface.bounds,
                        content_mask: surface.content_mask.clone(),
                    },
                );
            }

            command_encoder.draw_primitives(metal::MTLPrimitiveType::Triangle, 0, 6);
            *instance_offset = next_offset;
        }
        true
    }

    fn draw_custom_render_targets(
        &mut self,
        scene: &Scene,
        command_buffer: &metal::CommandBufferRef,
        instance_buffer: &mut InstanceBuffer,
        instance_offset: &mut usize,
    ) -> Result<()> {
        struct RenderTargetInfo {
            texture: metal::Texture,
            msaa_texture: Option<metal::Texture>,
            width: u32,
            height: u32,
            format: metal::MTLPixelFormat,
            clear_color: [f32; 4],
            is_render_target: bool,
            sample_count: u32,
        }

        struct DepthTargetInfo {
            texture: metal::Texture,
            format: metal::MTLPixelFormat,
            clear_depth: f64,
            width: u32,
            height: u32,
            sample_count: u32,
        }

        let mut draws_by_target = BTreeMap::new();
        for draw in scene.custom_draws.iter() {
            let Some(target) = draw.target.as_ref() else {
                continue;
            };
            let colors: Vec<u32> = target.colors.iter().map(|color| color.0).collect();
            draws_by_target
                .entry((colors, target.depth.map(|depth| depth.0)))
                .or_insert_with(Vec::new)
                .push(draw);
        }
        if draws_by_target.is_empty() {
            return Ok(());
        }

        let buffers_snapshot = self.custom_draw.buffers_snapshot();
        let textures_snapshot = self.custom_draw.textures_snapshot();
        let samplers_snapshot = self.custom_draw.samplers_snapshot();

        'render_target: for (_, draws) in draws_by_target {
            let Some(target) = draws.first().and_then(|draw| draw.target.as_ref()) else {
                continue;
            };
            let mut color_targets = Vec::with_capacity(target.colors.len());
            for color_id in &target.colors {
                let Some(color_target) =
                    self.custom_draw
                        .with_texture(*color_id, |entry| RenderTargetInfo {
                            texture: entry.texture.clone(),
                            msaa_texture: entry.msaa_texture.clone(),
                            width: entry.width,
                            height: entry.height,
                            format: entry.format,
                            clear_color: entry.clear_color,
                            is_render_target: entry.is_render_target,
                            sample_count: entry.sample_count,
                        })
                else {
                    log::warn!("custom render target {:?} missing", color_id.0);
                    continue 'render_target;
                };
                if !color_target.is_render_target {
                    log::warn!("custom draw target {:?} is not a render target", color_id.0);
                    continue 'render_target;
                }
                if color_target.sample_count > 1 && color_target.msaa_texture.is_none() {
                    log::warn!("custom draw target {:?} missing MSAA texture", color_id.0);
                    continue 'render_target;
                }
                color_targets.push(color_target);
            }
            let Some(first_target) = color_targets.first() else {
                continue;
            };
            for target_info in &color_targets[1..] {
                if target_info.width != first_target.width
                    || target_info.height != first_target.height
                {
                    log::warn!("custom render targets must match in size");
                    continue 'render_target;
                }
                if target_info.sample_count != first_target.sample_count {
                    log::warn!("custom render targets must match in sample count");
                    continue 'render_target;
                }
            }

            let depth_target = if let Some(depth_id) = target.depth {
                match self
                    .custom_draw
                    .with_depth_target(depth_id, |entry| DepthTargetInfo {
                        texture: entry.texture.clone(),
                        format: entry.format,
                        clear_depth: entry.clear_depth,
                        width: entry.width,
                        height: entry.height,
                        sample_count: entry.sample_count,
                    }) {
                    Some(target) => Some(target),
                    None => {
                        log::warn!("custom depth target {:?} missing", depth_id.0);
                        continue 'render_target;
                    }
                }
            } else {
                None
            };

            if let Some(depth_target) = depth_target.as_ref() {
                if depth_target.width != first_target.width
                    || depth_target.height != first_target.height
                {
                    log::warn!("custom depth target size mismatch");
                    continue 'render_target;
                }
                if depth_target.sample_count != first_target.sample_count {
                    log::warn!("custom depth target sample count mismatch");
                    continue 'render_target;
                }
            }

            let render_pass_descriptor = metal::RenderPassDescriptor::new();
            let color_attachments = render_pass_descriptor.color_attachments();
            for (index, color_target) in color_targets.iter().enumerate() {
                let Some(color_attachment) = color_attachments.object_at(index as u64) else {
                    log::warn!("custom draw color attachment {} missing", index);
                    continue 'render_target;
                };
                color_attachment.set_load_action(metal::MTLLoadAction::Clear);
                if let Some(msaa_texture) = color_target.msaa_texture.as_ref() {
                    color_attachment.set_texture(Some(msaa_texture));
                    color_attachment.set_resolve_texture(Some(&color_target.texture));
                    color_attachment.set_store_action(metal::MTLStoreAction::MultisampleResolve);
                } else {
                    color_attachment.set_texture(Some(&color_target.texture));
                    color_attachment.set_store_action(metal::MTLStoreAction::Store);
                }
                color_attachment.set_clear_color(metal::MTLClearColor::new(
                    color_target.clear_color[0] as f64,
                    color_target.clear_color[1] as f64,
                    color_target.clear_color[2] as f64,
                    color_target.clear_color[3] as f64,
                ));
            }

            if let Some(depth_target) = depth_target.as_ref() {
                let Some(depth_attachment) = render_pass_descriptor.depth_attachment() else {
                    log::warn!("custom draw depth attachment missing");
                    continue 'render_target;
                };
                depth_attachment.set_texture(Some(&depth_target.texture));
                depth_attachment.set_load_action(metal::MTLLoadAction::Clear);
                depth_attachment.set_store_action(metal::MTLStoreAction::Store);
                depth_attachment.set_clear_depth(depth_target.clear_depth);
            }

            let command_encoder = command_buffer.new_render_command_encoder(render_pass_descriptor);
            command_encoder.set_viewport(metal::MTLViewport {
                originX: 0.0,
                originY: 0.0,
                width: first_target.width as f64,
                height: first_target.height as f64,
                znear: 0.0,
                zfar: 1.0,
            });

            let color_formats: Vec<metal::MTLPixelFormat> =
                color_targets.iter().map(|target| target.format).collect();
            let outcome = self.draw_custom_draws_for_target(
                &draws,
                instance_buffer,
                instance_offset,
                command_encoder,
                color_formats.as_slice(),
                first_target.sample_count,
                depth_target.as_ref().map(|target| target.format),
                &buffers_snapshot,
                &textures_snapshot,
                &samplers_snapshot,
            );
            command_encoder.end_encoding();

            if matches!(outcome, CustomDrawBindOutcome::OutOfSpace) {
                return Err(anyhow!("custom draw out of space"));
            }
        }

        Ok(())
    }

    fn dispatch_custom_computes(
        &mut self,
        scene: &Scene,
        command_buffer: &metal::CommandBufferRef,
        instance_buffer: &mut InstanceBuffer,
        instance_offset: &mut usize,
    ) -> Result<()> {
        if scene.custom_computes.is_empty() {
            return Ok(());
        }

        let buffers_snapshot = self.custom_draw.buffers_snapshot();
        let textures_snapshot = self.custom_draw.textures_snapshot();
        let samplers_snapshot = self.custom_draw.samplers_snapshot();

        let command_encoder = command_buffer.new_compute_command_encoder();
        for compute in scene.custom_computes.iter() {
            if compute.workgroup_count.iter().any(|count| *count == 0) {
                continue;
            }
            let Some(outcome) =
                self.custom_draw
                    .with_compute_pipeline(compute.pipeline, |pipeline| {
                        command_encoder.set_compute_pipeline_state(&pipeline.pipeline_state);
                        match self.bind_custom_compute_resources(
                            command_encoder,
                            pipeline,
                            &compute.bindings,
                            &buffers_snapshot,
                            &textures_snapshot,
                            &samplers_snapshot,
                            instance_buffer,
                            instance_offset,
                        ) {
                            CustomDrawBindOutcome::Ready => {}
                            other => return other,
                        }

                        let groups = metal::MTLSize {
                            width: compute.workgroup_count[0] as u64,
                            height: compute.workgroup_count[1] as u64,
                            depth: compute.workgroup_count[2] as u64,
                        };
                        let threads_per_group = metal::MTLSize {
                            width: pipeline.workgroup_size[0] as u64,
                            height: pipeline.workgroup_size[1] as u64,
                            depth: pipeline.workgroup_size[2] as u64,
                        };
                        command_encoder.dispatch_thread_groups(groups, threads_per_group);
                        CustomDrawBindOutcome::Ready
                    })
            else {
                log::warn!("custom compute pipeline {:?} not found", compute.pipeline.0);
                continue;
            };

            if matches!(outcome, CustomDrawBindOutcome::OutOfSpace) {
                command_encoder.end_encoding();
                return Err(anyhow!("custom compute out of space"));
            }
        }
        command_encoder.end_encoding();
        Ok(())
    }

    fn draw_custom_draws(
        &mut self,
        draws: &[CustomDraw],
        instance_buffer: &mut InstanceBuffer,
        instance_offset: &mut usize,
        command_encoder: &metal::RenderCommandEncoderRef,
    ) -> bool {
        let draws: Vec<&CustomDraw> = draws.iter().filter(|draw| draw.target.is_none()).collect();
        if draws.is_empty() {
            return true;
        }

        let buffers_snapshot = self.custom_draw.buffers_snapshot();
        let textures_snapshot = self.custom_draw.textures_snapshot();
        let samplers_snapshot = self.custom_draw.samplers_snapshot();

        let color_formats = [self.custom_draw.surface_format()];
        let outcome = self.draw_custom_draws_for_target(
            &draws,
            instance_buffer,
            instance_offset,
            command_encoder,
            color_formats.as_slice(),
            1,
            None,
            &buffers_snapshot,
            &textures_snapshot,
            &samplers_snapshot,
        );

        !matches!(outcome, CustomDrawBindOutcome::OutOfSpace)
    }

    fn draw_custom_draws_for_target(
        &mut self,
        draws: &[&CustomDraw],
        instance_buffer: &mut InstanceBuffer,
        instance_offset: &mut usize,
        command_encoder: &metal::RenderCommandEncoderRef,
        color_formats: &[metal::MTLPixelFormat],
        sample_count: u32,
        depth_format: Option<metal::MTLPixelFormat>,
        buffers_snapshot: &[Option<MetalBufferSnapshot>],
        textures_snapshot: &[Option<metal::Texture>],
        samplers_snapshot: &[Option<metal::SamplerState>],
    ) -> CustomDrawBindOutcome {
        let mut index = 0;
        while index < draws.len() {
            let batch_key = draws[index].batch_key;
            let mut end = index + 1;
            while end < draws.len() && draws[end].batch_key == batch_key {
                end += 1;
            }

            let batch = &draws[index..end];
            let pipeline_id = batch[0].pipeline;
            let bindings = &batch[0].bindings;

            let Some(outcome) = self.custom_draw.with_pipeline(pipeline_id, |pipeline| {
                if pipeline.color_formats.len() != color_formats.len() {
                    log::warn!(
                        "custom draw pipeline {:?} expects {} color targets, got {}",
                        pipeline_id.0,
                        pipeline.color_formats.len(),
                        color_formats.len()
                    );
                    return CustomDrawBindOutcome::SkipBatch;
                }
                for (expected, actual) in pipeline.color_formats.iter().zip(color_formats.iter()) {
                    if *expected != *actual {
                        log::warn!(
                            "custom draw pipeline {:?} color format mismatch",
                            pipeline_id.0
                        );
                        return CustomDrawBindOutcome::SkipBatch;
                    }
                }
                if pipeline.sample_count != sample_count {
                    log::warn!(
                        "custom draw pipeline {:?} sample count mismatch",
                        pipeline_id.0
                    );
                    return CustomDrawBindOutcome::SkipBatch;
                }
                if let Some(pipeline_depth_format) = pipeline.depth_format {
                    if Some(pipeline_depth_format) != depth_format {
                        log::warn!(
                            "custom draw pipeline {:?} depth format mismatch",
                            pipeline_id.0
                        );
                        return CustomDrawBindOutcome::SkipBatch;
                    }
                }
                if pipeline.depth_format.is_some() {
                    if let Some(depth_state) = pipeline.depth_state.as_ref() {
                        command_encoder.set_depth_stencil_state(depth_state);
                    }
                }

                command_encoder.set_render_pipeline_state(&pipeline.pipeline_state);
                command_encoder.set_cull_mode(pipeline.cull_mode);
                command_encoder.set_front_facing_winding(pipeline.front_face);

                match self.bind_custom_resources(
                    command_encoder,
                    pipeline,
                    bindings,
                    buffers_snapshot,
                    textures_snapshot,
                    samplers_snapshot,
                    instance_buffer,
                    instance_offset,
                ) {
                    CustomDrawBindOutcome::Ready => {}
                    other => return other,
                }

                for draw in batch {
                    if draw.instance_count == 0 {
                        continue;
                    }
                    if draw.vertex_buffers.len() < pipeline.vertex_fetch_count {
                        log::warn!(
                            "custom draw missing vertex buffers (expected {}, got {})",
                            pipeline.vertex_fetch_count,
                            draw.vertex_buffers.len()
                        );
                        continue;
                    }

                    let mut vertex_binding_outcome = CustomDrawBindOutcome::Ready;
                    for (buffer_index, buffer) in draw
                        .vertex_buffers
                        .iter()
                        .enumerate()
                        .take(pipeline.vertex_fetch_count)
                    {
                        match self.bind_vertex_buffer(
                            command_encoder,
                            buffer_index,
                            &buffer.source,
                            buffers_snapshot,
                            instance_buffer,
                            instance_offset,
                        ) {
                            CustomDrawBindOutcome::Ready => {}
                            other => {
                                vertex_binding_outcome = other;
                                break;
                            }
                        }
                    }

                    match vertex_binding_outcome {
                        CustomDrawBindOutcome::Ready => {
                            if let Some(index_buffer) = &draw.index_buffer {
                                if draw.index_count == 0 {
                                    continue;
                                }
                                match self.bind_index_buffer(
                                    index_buffer,
                                    draw.index_count,
                                    buffers_snapshot,
                                    instance_buffer,
                                    instance_offset,
                                ) {
                                    IndexBufferBindOutcome::Ready(binding) => {
                                        command_encoder.draw_indexed_primitives_instanced(
                                            pipeline.primitive,
                                            draw.index_count as u64,
                                            metal_index_type(index_buffer.format),
                                            binding.buffer.as_ref(),
                                            binding.offset,
                                            draw.instance_count as u64,
                                        );
                                    }
                                    IndexBufferBindOutcome::SkipBatch => continue,
                                    IndexBufferBindOutcome::OutOfSpace => {
                                        return CustomDrawBindOutcome::OutOfSpace;
                                    }
                                }
                            } else {
                                if draw.vertex_count == 0 {
                                    continue;
                                }
                                command_encoder.draw_primitives_instanced(
                                    pipeline.primitive,
                                    0,
                                    draw.vertex_count as u64,
                                    draw.instance_count as u64,
                                );
                            }
                        }
                        CustomDrawBindOutcome::SkipBatch => continue,
                        CustomDrawBindOutcome::OutOfSpace => {
                            return CustomDrawBindOutcome::OutOfSpace;
                        }
                    }
                }

                CustomDrawBindOutcome::Ready
            }) else {
                log::warn!("custom draw pipeline {:?} not found", pipeline_id.0);
                index = end;
                continue;
            };

            if matches!(outcome, CustomDrawBindOutcome::OutOfSpace) {
                return CustomDrawBindOutcome::OutOfSpace;
            }

            index = end;
        }

        CustomDrawBindOutcome::Ready
    }

    fn prepare_argument_buffer(
        &self,
        argument_binding: &ArgumentBufferBinding,
        instance_buffer: &mut InstanceBuffer,
        instance_offset: &mut usize,
    ) -> std::result::Result<u64, InlineAllocationError> {
        let encoded_length = argument_binding.encoder.encoded_length() as usize;
        let alignment = argument_binding.encoder.alignment() as usize;
        let offset =
            allocate_inline_storage(instance_buffer, instance_offset, encoded_length, alignment)?;
        argument_binding
            .encoder
            .set_argument_buffer(&instance_buffer.metal_buffer, offset as metal::NSUInteger);
        Ok(offset)
    }

    fn bind_render_buffer_array(
        &self,
        command_encoder: &metal::RenderCommandEncoderRef,
        argument_binding: &ArgumentBufferBinding,
        buffer_slot: u64,
        sources: &[CustomBufferSource],
        buffers: &[Option<MetalBufferSnapshot>],
        instance_buffer: &mut InstanceBuffer,
        instance_offset: &mut usize,
    ) -> CustomDrawBindOutcome {
        let argument_offset = match self.prepare_argument_buffer(
            argument_binding,
            instance_buffer,
            instance_offset,
        ) {
            Ok(offset) => offset,
            Err(InlineAllocationError::EmptyData) => {
                log::warn!("custom draw binding array argument buffer is empty");
                return CustomDrawBindOutcome::SkipBatch;
            }
            Err(InlineAllocationError::OutOfSpace) => {
                return CustomDrawBindOutcome::OutOfSpace;
            }
        };

        for (array_index, source) in sources.iter().enumerate() {
            let array_index = array_index as metal::NSUInteger;
            match source {
                CustomBufferSource::Inline(data) => {
                    match allocate_inline_bytes(instance_buffer, instance_offset, data) {
                        Ok(offset) => {
                            argument_binding.encoder.set_buffer(
                                array_index,
                                &instance_buffer.metal_buffer,
                                offset,
                            );
                        }
                        Err(InlineAllocationError::EmptyData) => {
                            log::warn!("custom draw inline buffer array element is empty");
                            return CustomDrawBindOutcome::SkipBatch;
                        }
                        Err(InlineAllocationError::OutOfSpace) => {
                            return CustomDrawBindOutcome::OutOfSpace;
                        }
                    }
                }
                CustomBufferSource::Buffer(id) => {
                    let Some(buffer) = buffers.get(id.0 as usize).and_then(|slot| slot.as_ref())
                    else {
                        log::warn!("custom draw buffer {:?} missing", id.0);
                        return CustomDrawBindOutcome::SkipBatch;
                    };
                    argument_binding
                        .encoder
                        .set_buffer(array_index, &buffer.buffer, 0);
                }
                CustomBufferSource::BufferSlice { id, offset, size } => {
                    let Some(buffer) = buffers.get(id.0 as usize).and_then(|slot| slot.as_ref())
                    else {
                        log::warn!("custom draw buffer {:?} missing", id.0);
                        return CustomDrawBindOutcome::SkipBatch;
                    };
                    if *size == 0 {
                        log::warn!("custom draw buffer slice is empty");
                        return CustomDrawBindOutcome::SkipBatch;
                    }
                    if offset.saturating_add(*size) > buffer.size {
                        log::warn!("custom draw buffer slice out of range");
                        return CustomDrawBindOutcome::SkipBatch;
                    }
                    argument_binding.encoder.set_buffer(
                        array_index,
                        &buffer.buffer,
                        *offset as metal::NSUInteger,
                    );
                }
            }
        }

        command_encoder.set_vertex_buffer(
            buffer_slot,
            Some(&instance_buffer.metal_buffer),
            argument_offset,
        );
        command_encoder.set_fragment_buffer(
            buffer_slot,
            Some(&instance_buffer.metal_buffer),
            argument_offset,
        );

        CustomDrawBindOutcome::Ready
    }

    fn bind_render_texture_array(
        &self,
        command_encoder: &metal::RenderCommandEncoderRef,
        argument_binding: &ArgumentBufferBinding,
        buffer_slot: u64,
        ids: &[CustomTextureId],
        textures: &[Option<metal::Texture>],
        instance_buffer: &mut InstanceBuffer,
        instance_offset: &mut usize,
    ) -> CustomDrawBindOutcome {
        let argument_offset = match self.prepare_argument_buffer(
            argument_binding,
            instance_buffer,
            instance_offset,
        ) {
            Ok(offset) => offset,
            Err(InlineAllocationError::EmptyData) => {
                log::warn!("custom draw binding array argument buffer is empty");
                return CustomDrawBindOutcome::SkipBatch;
            }
            Err(InlineAllocationError::OutOfSpace) => {
                return CustomDrawBindOutcome::OutOfSpace;
            }
        };

        for (array_index, id) in ids.iter().enumerate() {
            let Some(slot) = textures.get(id.0 as usize) else {
                log::warn!("custom draw texture {:?} missing", id.0);
                return CustomDrawBindOutcome::SkipBatch;
            };
            let Some(texture) = slot.as_ref() else {
                log::warn!("custom draw texture {:?} missing", id.0);
                return CustomDrawBindOutcome::SkipBatch;
            };
            argument_binding
                .encoder
                .set_texture(array_index as metal::NSUInteger, texture);
        }

        command_encoder.set_vertex_buffer(
            buffer_slot,
            Some(&instance_buffer.metal_buffer),
            argument_offset,
        );
        command_encoder.set_fragment_buffer(
            buffer_slot,
            Some(&instance_buffer.metal_buffer),
            argument_offset,
        );

        CustomDrawBindOutcome::Ready
    }

    fn bind_compute_buffer_array(
        &self,
        command_encoder: &metal::ComputeCommandEncoderRef,
        argument_binding: &ArgumentBufferBinding,
        buffer_slot: u64,
        sources: &[CustomBufferSource],
        buffers: &[Option<MetalBufferSnapshot>],
        instance_buffer: &mut InstanceBuffer,
        instance_offset: &mut usize,
    ) -> CustomDrawBindOutcome {
        let argument_offset = match self.prepare_argument_buffer(
            argument_binding,
            instance_buffer,
            instance_offset,
        ) {
            Ok(offset) => offset,
            Err(InlineAllocationError::EmptyData) => {
                log::warn!("custom compute binding array argument buffer is empty");
                return CustomDrawBindOutcome::SkipBatch;
            }
            Err(InlineAllocationError::OutOfSpace) => {
                return CustomDrawBindOutcome::OutOfSpace;
            }
        };

        for (array_index, source) in sources.iter().enumerate() {
            let array_index = array_index as metal::NSUInteger;
            match source {
                CustomBufferSource::Inline(data) => {
                    match allocate_inline_bytes(instance_buffer, instance_offset, data) {
                        Ok(offset) => {
                            argument_binding.encoder.set_buffer(
                                array_index,
                                &instance_buffer.metal_buffer,
                                offset,
                            );
                        }
                        Err(InlineAllocationError::EmptyData) => {
                            log::warn!("custom compute inline buffer array element is empty");
                            return CustomDrawBindOutcome::SkipBatch;
                        }
                        Err(InlineAllocationError::OutOfSpace) => {
                            return CustomDrawBindOutcome::OutOfSpace;
                        }
                    }
                }
                CustomBufferSource::Buffer(id) => {
                    let Some(buffer) = buffers.get(id.0 as usize).and_then(|slot| slot.as_ref())
                    else {
                        log::warn!("custom compute buffer {:?} missing", id.0);
                        return CustomDrawBindOutcome::SkipBatch;
                    };
                    argument_binding
                        .encoder
                        .set_buffer(array_index, &buffer.buffer, 0);
                }
                CustomBufferSource::BufferSlice { id, offset, size } => {
                    let Some(buffer) = buffers.get(id.0 as usize).and_then(|slot| slot.as_ref())
                    else {
                        log::warn!("custom compute buffer {:?} missing", id.0);
                        return CustomDrawBindOutcome::SkipBatch;
                    };
                    if *size == 0 {
                        log::warn!("custom compute buffer slice is empty");
                        return CustomDrawBindOutcome::SkipBatch;
                    }
                    if offset.saturating_add(*size) > buffer.size {
                        log::warn!("custom compute buffer slice out of range");
                        return CustomDrawBindOutcome::SkipBatch;
                    }
                    argument_binding.encoder.set_buffer(
                        array_index,
                        &buffer.buffer,
                        *offset as metal::NSUInteger,
                    );
                }
            }
        }

        command_encoder.set_buffer(
            buffer_slot,
            Some(&instance_buffer.metal_buffer),
            argument_offset,
        );

        CustomDrawBindOutcome::Ready
    }

    fn bind_compute_texture_array(
        &self,
        command_encoder: &metal::ComputeCommandEncoderRef,
        argument_binding: &ArgumentBufferBinding,
        buffer_slot: u64,
        ids: &[CustomTextureId],
        textures: &[Option<metal::Texture>],
        instance_buffer: &mut InstanceBuffer,
        instance_offset: &mut usize,
    ) -> CustomDrawBindOutcome {
        let argument_offset = match self.prepare_argument_buffer(
            argument_binding,
            instance_buffer,
            instance_offset,
        ) {
            Ok(offset) => offset,
            Err(InlineAllocationError::EmptyData) => {
                log::warn!("custom compute binding array argument buffer is empty");
                return CustomDrawBindOutcome::SkipBatch;
            }
            Err(InlineAllocationError::OutOfSpace) => {
                return CustomDrawBindOutcome::OutOfSpace;
            }
        };

        for (array_index, id) in ids.iter().enumerate() {
            let Some(slot) = textures.get(id.0 as usize) else {
                log::warn!("custom compute texture {:?} missing", id.0);
                return CustomDrawBindOutcome::SkipBatch;
            };
            let Some(texture) = slot.as_ref() else {
                log::warn!("custom compute texture {:?} missing", id.0);
                return CustomDrawBindOutcome::SkipBatch;
            };
            argument_binding
                .encoder
                .set_texture(array_index as metal::NSUInteger, texture);
        }

        command_encoder.set_buffer(
            buffer_slot,
            Some(&instance_buffer.metal_buffer),
            argument_offset,
        );

        CustomDrawBindOutcome::Ready
    }

    fn bind_custom_resources(
        &self,
        command_encoder: &metal::RenderCommandEncoderRef,
        pipeline: &MetalCustomPipeline,
        bindings: &[CustomBindingValue],
        buffers: &[Option<MetalBufferSnapshot>],
        textures: &[Option<metal::Texture>],
        samplers: &[Option<metal::SamplerState>],
        instance_buffer: &mut InstanceBuffer,
        instance_offset: &mut usize,
    ) -> CustomDrawBindOutcome {
        if bindings.len() < pipeline.bindings.len() {
            log::warn!(
                "custom draw bindings missing (expected {}, got {})",
                pipeline.bindings.len(),
                bindings.len()
            );
        }

        for (index, kind) in pipeline.bindings.iter().enumerate() {
            let Some(binding) = bindings.get(index) else {
                match kind {
                    CustomBindingKind::Texture | CustomBindingKind::StorageTexture => {
                        command_encoder.set_vertex_texture(index as u64, None);
                        command_encoder.set_fragment_texture(index as u64, None);
                    }
                    CustomBindingKind::Sampler => {
                        command_encoder.set_vertex_sampler_state(index as u64, None);
                        command_encoder.set_fragment_sampler_state(index as u64, None);
                    }
                    _ => {}
                }
                continue;
            };
            let binding_index = index as u64;
            match (kind, binding) {
                (CustomBindingKind::Buffer, CustomBindingValue::Buffer(source)) => {
                    let buffer_slot = pipeline.buffer_binding_base + binding_index;
                    match self.bind_buffer_source(
                        command_encoder,
                        buffer_slot,
                        source,
                        buffers,
                        instance_buffer,
                        instance_offset,
                        None,
                    ) {
                        CustomDrawBindOutcome::Ready => {}
                        other => return other,
                    }
                }
                (
                    CustomBindingKind::BufferArray { count },
                    CustomBindingValue::BufferArray(sources),
                ) => {
                    if sources.len() != *count as usize {
                        log::warn!(
                            "custom draw buffer array length mismatch (expected {}, got {})",
                            count,
                            sources.len()
                        );
                        return CustomDrawBindOutcome::SkipBatch;
                    }
                    let Some(argument_binding) = pipeline
                        .argument_buffers
                        .get(index)
                        .and_then(|entry| entry.as_ref())
                    else {
                        log::warn!(
                            "custom draw binding array encoder missing at slot {}",
                            index
                        );
                        return CustomDrawBindOutcome::SkipBatch;
                    };
                    let buffer_slot = pipeline.buffer_binding_base + binding_index;
                    match self.bind_render_buffer_array(
                        command_encoder,
                        argument_binding,
                        buffer_slot,
                        sources,
                        buffers,
                        instance_buffer,
                        instance_offset,
                    ) {
                        CustomDrawBindOutcome::Ready => {}
                        other => return other,
                    }
                }
                (
                    CustomBindingKind::TextureArray { count }
                    | CustomBindingKind::StorageTextureArray { count },
                    CustomBindingValue::TextureArray(ids),
                ) => {
                    if ids.len() != *count as usize {
                        log::warn!(
                            "custom draw texture array length mismatch (expected {}, got {})",
                            count,
                            ids.len()
                        );
                        return CustomDrawBindOutcome::SkipBatch;
                    }
                    let Some(argument_binding) = pipeline
                        .argument_buffers
                        .get(index)
                        .and_then(|entry| entry.as_ref())
                    else {
                        log::warn!(
                            "custom draw binding array encoder missing at slot {}",
                            index
                        );
                        return CustomDrawBindOutcome::SkipBatch;
                    };
                    let buffer_slot = pipeline.buffer_binding_base + binding_index;
                    match self.bind_render_texture_array(
                        command_encoder,
                        argument_binding,
                        buffer_slot,
                        ids,
                        textures,
                        instance_buffer,
                        instance_offset,
                    ) {
                        CustomDrawBindOutcome::Ready => {}
                        other => return other,
                    }
                }
                (CustomBindingKind::Uniform { size }, CustomBindingValue::Uniform(source)) => {
                    let buffer_slot = pipeline.buffer_binding_base + binding_index;
                    match self.bind_buffer_source(
                        command_encoder,
                        buffer_slot,
                        source,
                        buffers,
                        instance_buffer,
                        instance_offset,
                        Some(*size as usize),
                    ) {
                        CustomDrawBindOutcome::Ready => {}
                        other => return other,
                    }
                }
                (
                    CustomBindingKind::Texture | CustomBindingKind::StorageTexture,
                    CustomBindingValue::Texture(id),
                ) => {
                    let Some(texture) = textures.get(id.0 as usize).and_then(|slot| slot.as_ref())
                    else {
                        log::warn!("custom draw texture {:?} missing", id.0);
                        return CustomDrawBindOutcome::SkipBatch;
                    };
                    command_encoder.set_vertex_texture(binding_index, Some(texture));
                    command_encoder.set_fragment_texture(binding_index, Some(texture));
                }
                (CustomBindingKind::Sampler, CustomBindingValue::Sampler(id)) => {
                    let Some(sampler) = samplers.get(id.0 as usize).and_then(|slot| slot.as_ref())
                    else {
                        log::warn!("custom draw sampler {:?} missing", id.0);
                        return CustomDrawBindOutcome::SkipBatch;
                    };
                    command_encoder.set_vertex_sampler_state(binding_index, Some(sampler));
                    command_encoder.set_fragment_sampler_state(binding_index, Some(sampler));
                }
                _ => {
                    log::warn!("custom draw binding mismatch at slot {}", index);
                    return CustomDrawBindOutcome::SkipBatch;
                }
            }
        }

        CustomDrawBindOutcome::Ready
    }

    fn bind_custom_compute_resources(
        &self,
        command_encoder: &metal::ComputeCommandEncoderRef,
        pipeline: &MetalCustomComputePipeline,
        bindings: &[CustomBindingValue],
        buffers: &[Option<MetalBufferSnapshot>],
        textures: &[Option<metal::Texture>],
        samplers: &[Option<metal::SamplerState>],
        instance_buffer: &mut InstanceBuffer,
        instance_offset: &mut usize,
    ) -> CustomDrawBindOutcome {
        if bindings.len() < pipeline.bindings.len() {
            log::warn!(
                "custom compute bindings missing (expected {}, got {})",
                pipeline.bindings.len(),
                bindings.len()
            );
        }

        for (index, kind) in pipeline.bindings.iter().enumerate() {
            let Some(binding) = bindings.get(index) else {
                match kind {
                    CustomBindingKind::Texture | CustomBindingKind::StorageTexture => {
                        command_encoder.set_texture(index as u64, None);
                    }
                    CustomBindingKind::Sampler => {
                        command_encoder.set_sampler_state(index as u64, None);
                    }
                    _ => {}
                }
                continue;
            };
            let binding_index = index as u64;
            match (kind, binding) {
                (CustomBindingKind::Buffer, CustomBindingValue::Buffer(source)) => {
                    let buffer_slot = pipeline.buffer_binding_base + binding_index;
                    match self.bind_compute_buffer_source(
                        command_encoder,
                        buffer_slot,
                        source,
                        buffers,
                        instance_buffer,
                        instance_offset,
                        None,
                    ) {
                        CustomDrawBindOutcome::Ready => {}
                        other => return other,
                    }
                }
                (
                    CustomBindingKind::BufferArray { count },
                    CustomBindingValue::BufferArray(sources),
                ) => {
                    if sources.len() != *count as usize {
                        log::warn!(
                            "custom compute buffer array length mismatch (expected {}, got {})",
                            count,
                            sources.len()
                        );
                        return CustomDrawBindOutcome::SkipBatch;
                    }
                    let Some(argument_binding) = pipeline
                        .argument_buffers
                        .get(index)
                        .and_then(|entry| entry.as_ref())
                    else {
                        log::warn!(
                            "custom compute binding array encoder missing at slot {}",
                            index
                        );
                        return CustomDrawBindOutcome::SkipBatch;
                    };
                    let buffer_slot = pipeline.buffer_binding_base + binding_index;
                    match self.bind_compute_buffer_array(
                        command_encoder,
                        argument_binding,
                        buffer_slot,
                        sources,
                        buffers,
                        instance_buffer,
                        instance_offset,
                    ) {
                        CustomDrawBindOutcome::Ready => {}
                        other => return other,
                    }
                }
                (
                    CustomBindingKind::TextureArray { count }
                    | CustomBindingKind::StorageTextureArray { count },
                    CustomBindingValue::TextureArray(ids),
                ) => {
                    if ids.len() != *count as usize {
                        log::warn!(
                            "custom compute texture array length mismatch (expected {}, got {})",
                            count,
                            ids.len()
                        );
                        return CustomDrawBindOutcome::SkipBatch;
                    }
                    let Some(argument_binding) = pipeline
                        .argument_buffers
                        .get(index)
                        .and_then(|entry| entry.as_ref())
                    else {
                        log::warn!(
                            "custom compute binding array encoder missing at slot {}",
                            index
                        );
                        return CustomDrawBindOutcome::SkipBatch;
                    };
                    let buffer_slot = pipeline.buffer_binding_base + binding_index;
                    match self.bind_compute_texture_array(
                        command_encoder,
                        argument_binding,
                        buffer_slot,
                        ids,
                        textures,
                        instance_buffer,
                        instance_offset,
                    ) {
                        CustomDrawBindOutcome::Ready => {}
                        other => return other,
                    }
                }
                (CustomBindingKind::Uniform { size }, CustomBindingValue::Uniform(source)) => {
                    let buffer_slot = pipeline.buffer_binding_base + binding_index;
                    match self.bind_compute_buffer_source(
                        command_encoder,
                        buffer_slot,
                        source,
                        buffers,
                        instance_buffer,
                        instance_offset,
                        Some(*size as usize),
                    ) {
                        CustomDrawBindOutcome::Ready => {}
                        other => return other,
                    }
                }
                (
                    CustomBindingKind::Texture | CustomBindingKind::StorageTexture,
                    CustomBindingValue::Texture(id),
                ) => {
                    let Some(texture) = textures.get(id.0 as usize).and_then(|slot| slot.as_ref())
                    else {
                        log::warn!("custom compute texture {:?} missing", id.0);
                        return CustomDrawBindOutcome::SkipBatch;
                    };
                    command_encoder.set_texture(binding_index, Some(texture));
                }
                (CustomBindingKind::Sampler, CustomBindingValue::Sampler(id)) => {
                    let Some(sampler) = samplers.get(id.0 as usize).and_then(|slot| slot.as_ref())
                    else {
                        log::warn!("custom compute sampler {:?} missing", id.0);
                        return CustomDrawBindOutcome::SkipBatch;
                    };
                    command_encoder.set_sampler_state(binding_index, Some(sampler));
                }
                _ => {
                    log::warn!("custom compute binding mismatch at slot {}", index);
                    return CustomDrawBindOutcome::SkipBatch;
                }
            }
        }

        CustomDrawBindOutcome::Ready
    }

    fn bind_vertex_buffer(
        &self,
        command_encoder: &metal::RenderCommandEncoderRef,
        buffer_index: usize,
        source: &CustomBufferSource,
        buffers: &[Option<MetalBufferSnapshot>],
        instance_buffer: &mut InstanceBuffer,
        instance_offset: &mut usize,
    ) -> CustomDrawBindOutcome {
        match source {
            CustomBufferSource::Inline(data) => {
                match allocate_inline_bytes(instance_buffer, instance_offset, data) {
                    Ok(offset) => {
                        command_encoder.set_vertex_buffer(
                            buffer_index as u64,
                            Some(&instance_buffer.metal_buffer),
                            offset,
                        );
                        CustomDrawBindOutcome::Ready
                    }
                    Err(InlineAllocationError::EmptyData) => {
                        log::warn!("custom draw inline vertex buffer is empty");
                        CustomDrawBindOutcome::SkipBatch
                    }
                    Err(InlineAllocationError::OutOfSpace) => CustomDrawBindOutcome::OutOfSpace,
                }
            }
            CustomBufferSource::Buffer(id) => {
                let Some(buffer) = buffers.get(id.0 as usize).and_then(|slot| slot.as_ref()) else {
                    log::warn!("custom draw vertex buffer {:?} missing", id.0);
                    return CustomDrawBindOutcome::SkipBatch;
                };
                command_encoder.set_vertex_buffer(buffer_index as u64, Some(&buffer.buffer), 0);
                CustomDrawBindOutcome::Ready
            }
            CustomBufferSource::BufferSlice { id, offset, size } => {
                let Some(buffer) = buffers.get(id.0 as usize).and_then(|slot| slot.as_ref()) else {
                    log::warn!("custom draw vertex buffer {:?} missing", id.0);
                    return CustomDrawBindOutcome::SkipBatch;
                };
                if *size == 0 {
                    log::warn!("custom draw vertex buffer slice is empty");
                    return CustomDrawBindOutcome::SkipBatch;
                }
                if offset.saturating_add(*size) > buffer.size {
                    log::warn!("custom draw vertex buffer slice out of range");
                    return CustomDrawBindOutcome::SkipBatch;
                }
                command_encoder.set_vertex_buffer(
                    buffer_index as u64,
                    Some(&buffer.buffer),
                    *offset,
                );
                CustomDrawBindOutcome::Ready
            }
        }
    }

    fn bind_compute_buffer_source(
        &self,
        command_encoder: &metal::ComputeCommandEncoderRef,
        buffer_slot: u64,
        source: &CustomBufferSource,
        buffers: &[Option<MetalBufferSnapshot>],
        instance_buffer: &mut InstanceBuffer,
        instance_offset: &mut usize,
        expected_size: Option<usize>,
    ) -> CustomDrawBindOutcome {
        match source {
            CustomBufferSource::Inline(data) => {
                if let Some(expected_size) = expected_size {
                    if data.len() != expected_size {
                        log::warn!(
                            "custom compute uniform size mismatch (expected {}, got {})",
                            expected_size,
                            data.len()
                        );
                        return CustomDrawBindOutcome::SkipBatch;
                    }
                }
                match allocate_inline_bytes(instance_buffer, instance_offset, data) {
                    Ok(offset) => {
                        command_encoder.set_buffer(
                            buffer_slot,
                            Some(&instance_buffer.metal_buffer),
                            offset,
                        );
                        CustomDrawBindOutcome::Ready
                    }
                    Err(InlineAllocationError::EmptyData) => {
                        log::warn!("custom compute inline buffer is empty");
                        CustomDrawBindOutcome::SkipBatch
                    }
                    Err(InlineAllocationError::OutOfSpace) => CustomDrawBindOutcome::OutOfSpace,
                }
            }
            CustomBufferSource::Buffer(id) => {
                let Some(buffer) = buffers.get(id.0 as usize).and_then(|slot| slot.as_ref()) else {
                    log::warn!("custom compute buffer {:?} missing", id.0);
                    return CustomDrawBindOutcome::SkipBatch;
                };
                if let Some(expected_size) = expected_size {
                    if buffer.size < expected_size as u64 {
                        log::warn!(
                            "custom compute uniform buffer too small (expected at least {}, got {})",
                            expected_size,
                            buffer.size
                        );
                        return CustomDrawBindOutcome::SkipBatch;
                    }
                }
                command_encoder.set_buffer(buffer_slot, Some(&buffer.buffer), 0);
                CustomDrawBindOutcome::Ready
            }
            CustomBufferSource::BufferSlice { id, offset, size } => {
                let Some(buffer) = buffers.get(id.0 as usize).and_then(|slot| slot.as_ref()) else {
                    log::warn!("custom compute buffer {:?} missing", id.0);
                    return CustomDrawBindOutcome::SkipBatch;
                };
                if *size == 0 {
                    log::warn!("custom compute buffer slice is empty");
                    return CustomDrawBindOutcome::SkipBatch;
                }
                if offset.saturating_add(*size) > buffer.size {
                    log::warn!("custom compute buffer slice out of range");
                    return CustomDrawBindOutcome::SkipBatch;
                }
                if let Some(expected_size) = expected_size {
                    if *size < expected_size as u64 {
                        log::warn!(
                            "custom compute uniform buffer slice too small (expected at least {}, got {})",
                            expected_size,
                            size
                        );
                        return CustomDrawBindOutcome::SkipBatch;
                    }
                }
                command_encoder.set_buffer(buffer_slot, Some(&buffer.buffer), *offset);
                CustomDrawBindOutcome::Ready
            }
        }
    }

    fn bind_index_buffer(
        &self,
        index_buffer: &CustomIndexBuffer,
        index_count: u32,
        buffers: &[Option<MetalBufferSnapshot>],
        instance_buffer: &mut InstanceBuffer,
        instance_offset: &mut usize,
    ) -> IndexBufferBindOutcome {
        let expected_len = index_count as usize * index_format_size(index_buffer.format);
        match &index_buffer.source {
            CustomBufferSource::Inline(data) => {
                if expected_len > 0 && data.len() < expected_len {
                    log::warn!(
                        "custom draw index buffer too small (expected at least {}, got {})",
                        expected_len,
                        data.len()
                    );
                    return IndexBufferBindOutcome::SkipBatch;
                }
                match allocate_inline_bytes(instance_buffer, instance_offset, data) {
                    Ok(offset) => IndexBufferBindOutcome::Ready(IndexBufferBinding {
                        buffer: instance_buffer.metal_buffer.clone(),
                        offset,
                    }),
                    Err(InlineAllocationError::EmptyData) => {
                        log::warn!("custom draw inline index buffer is empty");
                        IndexBufferBindOutcome::SkipBatch
                    }
                    Err(InlineAllocationError::OutOfSpace) => IndexBufferBindOutcome::OutOfSpace,
                }
            }
            CustomBufferSource::Buffer(id) => {
                let Some(buffer) = buffers.get(id.0 as usize).and_then(|slot| slot.as_ref()) else {
                    log::warn!("custom draw index buffer {:?} missing", id.0);
                    return IndexBufferBindOutcome::SkipBatch;
                };
                if expected_len > 0 && buffer.size < expected_len as u64 {
                    log::warn!(
                        "custom draw index buffer too small (expected at least {}, got {})",
                        expected_len,
                        buffer.size
                    );
                    return IndexBufferBindOutcome::SkipBatch;
                }
                IndexBufferBindOutcome::Ready(IndexBufferBinding {
                    buffer: buffer.buffer.clone(),
                    offset: 0,
                })
            }
            CustomBufferSource::BufferSlice { id, offset, size } => {
                let Some(buffer) = buffers.get(id.0 as usize).and_then(|slot| slot.as_ref()) else {
                    log::warn!("custom draw index buffer {:?} missing", id.0);
                    return IndexBufferBindOutcome::SkipBatch;
                };
                if *size == 0 {
                    log::warn!("custom draw index buffer slice is empty");
                    return IndexBufferBindOutcome::SkipBatch;
                }
                if offset.saturating_add(*size) > buffer.size {
                    log::warn!("custom draw index buffer slice out of range");
                    return IndexBufferBindOutcome::SkipBatch;
                }
                if expected_len > 0 && *size < expected_len as u64 {
                    log::warn!(
                        "custom draw index buffer slice too small (expected at least {}, got {})",
                        expected_len,
                        size
                    );
                    return IndexBufferBindOutcome::SkipBatch;
                }
                IndexBufferBindOutcome::Ready(IndexBufferBinding {
                    buffer: buffer.buffer.clone(),
                    offset: *offset,
                })
            }
        }
    }

    fn bind_buffer_source(
        &self,
        command_encoder: &metal::RenderCommandEncoderRef,
        buffer_slot: u64,
        source: &CustomBufferSource,
        buffers: &[Option<MetalBufferSnapshot>],
        instance_buffer: &mut InstanceBuffer,
        instance_offset: &mut usize,
        expected_size: Option<usize>,
    ) -> CustomDrawBindOutcome {
        match source {
            CustomBufferSource::Inline(data) => {
                if let Some(expected_size) = expected_size {
                    if data.len() != expected_size {
                        log::warn!(
                            "custom draw uniform size mismatch (expected {}, got {})",
                            expected_size,
                            data.len()
                        );
                        return CustomDrawBindOutcome::SkipBatch;
                    }
                }
                match allocate_inline_bytes(instance_buffer, instance_offset, data) {
                    Ok(offset) => {
                        command_encoder.set_vertex_buffer(
                            buffer_slot,
                            Some(&instance_buffer.metal_buffer),
                            offset,
                        );
                        command_encoder.set_fragment_buffer(
                            buffer_slot,
                            Some(&instance_buffer.metal_buffer),
                            offset,
                        );
                        CustomDrawBindOutcome::Ready
                    }
                    Err(InlineAllocationError::EmptyData) => {
                        log::warn!("custom draw inline buffer is empty");
                        CustomDrawBindOutcome::SkipBatch
                    }
                    Err(InlineAllocationError::OutOfSpace) => CustomDrawBindOutcome::OutOfSpace,
                }
            }
            CustomBufferSource::Buffer(id) => {
                let Some(buffer) = buffers.get(id.0 as usize).and_then(|slot| slot.as_ref()) else {
                    log::warn!("custom draw buffer {:?} missing", id.0);
                    return CustomDrawBindOutcome::SkipBatch;
                };
                if let Some(expected_size) = expected_size {
                    if buffer.size < expected_size as u64 {
                        log::warn!(
                            "custom draw uniform buffer too small (expected at least {}, got {})",
                            expected_size,
                            buffer.size
                        );
                        return CustomDrawBindOutcome::SkipBatch;
                    }
                }
                command_encoder.set_vertex_buffer(buffer_slot, Some(&buffer.buffer), 0);
                command_encoder.set_fragment_buffer(buffer_slot, Some(&buffer.buffer), 0);
                CustomDrawBindOutcome::Ready
            }
            CustomBufferSource::BufferSlice { id, offset, size } => {
                let Some(buffer) = buffers.get(id.0 as usize).and_then(|slot| slot.as_ref()) else {
                    log::warn!("custom draw buffer {:?} missing", id.0);
                    return CustomDrawBindOutcome::SkipBatch;
                };
                if *size == 0 {
                    log::warn!("custom draw buffer slice is empty");
                    return CustomDrawBindOutcome::SkipBatch;
                }
                if offset.saturating_add(*size) > buffer.size {
                    log::warn!("custom draw buffer slice out of range");
                    return CustomDrawBindOutcome::SkipBatch;
                }
                if let Some(expected_size) = expected_size {
                    if *size < expected_size as u64 {
                        log::warn!(
                            "custom draw uniform buffer slice too small (expected at least {}, got {})",
                            expected_size,
                            size
                        );
                        return CustomDrawBindOutcome::SkipBatch;
                    }
                }
                command_encoder.set_vertex_buffer(buffer_slot, Some(&buffer.buffer), *offset);
                command_encoder.set_fragment_buffer(buffer_slot, Some(&buffer.buffer), *offset);
                CustomDrawBindOutcome::Ready
            }
        }
    }
}

struct IndexBufferBinding {
    buffer: metal::Buffer,
    offset: u64,
}

enum IndexBufferBindOutcome {
    Ready(IndexBufferBinding),
    SkipBatch,
    OutOfSpace,
}

enum CustomDrawBindOutcome {
    Ready,
    SkipBatch,
    OutOfSpace,
}

enum InlineAllocationError {
    OutOfSpace,
    EmptyData,
}

fn allocate_inline_bytes(
    instance_buffer: &mut InstanceBuffer,
    instance_offset: &mut usize,
    data: &[u8],
) -> std::result::Result<u64, InlineAllocationError> {
    if data.is_empty() {
        return Err(InlineAllocationError::EmptyData);
    }

    align_offset(instance_offset);
    let start = *instance_offset;
    let next = start + data.len();
    if next > instance_buffer.size {
        return Err(InlineAllocationError::OutOfSpace);
    }

    unsafe {
        let destination = (instance_buffer.metal_buffer.contents() as *mut u8).add(start);
        ptr::copy_nonoverlapping(data.as_ptr(), destination, data.len());
    }
    *instance_offset = next;

    Ok(start as u64)
}

fn allocate_inline_storage(
    instance_buffer: &mut InstanceBuffer,
    instance_offset: &mut usize,
    size: usize,
    alignment: usize,
) -> std::result::Result<u64, InlineAllocationError> {
    if size == 0 {
        return Err(InlineAllocationError::EmptyData);
    }

    align_offset_to(instance_offset, alignment);
    let start = *instance_offset;
    let next = start + size;
    if next > instance_buffer.size {
        return Err(InlineAllocationError::OutOfSpace);
    }

    unsafe {
        let destination = (instance_buffer.metal_buffer.contents() as *mut u8).add(start);
        ptr::write_bytes(destination, 0, size);
    }
    *instance_offset = next;

    Ok(start as u64)
}

fn index_format_size(format: CustomIndexFormat) -> usize {
    match format {
        CustomIndexFormat::U16 => 2,
        CustomIndexFormat::U32 => 4,
    }
}

fn metal_index_type(format: CustomIndexFormat) -> metal::MTLIndexType {
    match format {
        CustomIndexFormat::U16 => metal::MTLIndexType::UInt16,
        CustomIndexFormat::U32 => metal::MTLIndexType::UInt32,
    }
}

fn new_command_encoder<'a>(
    command_buffer: &'a metal::CommandBufferRef,
    drawable: &'a metal::MetalDrawableRef,
    viewport_size: Size<DevicePixels>,
    configure_color_attachment: impl Fn(&RenderPassColorAttachmentDescriptorRef),
) -> &'a metal::RenderCommandEncoderRef {
    let render_pass_descriptor = metal::RenderPassDescriptor::new();
    let color_attachment = render_pass_descriptor
        .color_attachments()
        .object_at(0)
        .unwrap();
    color_attachment.set_texture(Some(drawable.texture()));
    color_attachment.set_store_action(metal::MTLStoreAction::Store);
    configure_color_attachment(color_attachment);

    let command_encoder = command_buffer.new_render_command_encoder(render_pass_descriptor);
    command_encoder.set_viewport(metal::MTLViewport {
        originX: 0.0,
        originY: 0.0,
        width: i32::from(viewport_size.width) as f64,
        height: i32::from(viewport_size.height) as f64,
        znear: 0.0,
        zfar: 1.0,
    });
    command_encoder
}

fn build_pipeline_state(
    device: &metal::DeviceRef,
    library: &metal::LibraryRef,
    label: &str,
    vertex_fn_name: &str,
    fragment_fn_name: &str,
    pixel_format: metal::MTLPixelFormat,
) -> metal::RenderPipelineState {
    let vertex_fn = library
        .get_function(vertex_fn_name, None)
        .expect("error locating vertex function");
    let fragment_fn = library
        .get_function(fragment_fn_name, None)
        .expect("error locating fragment function");

    let descriptor = metal::RenderPipelineDescriptor::new();
    descriptor.set_label(label);
    descriptor.set_vertex_function(Some(vertex_fn.as_ref()));
    descriptor.set_fragment_function(Some(fragment_fn.as_ref()));
    let color_attachment = descriptor.color_attachments().object_at(0).unwrap();
    color_attachment.set_pixel_format(pixel_format);
    color_attachment.set_blending_enabled(true);
    color_attachment.set_rgb_blend_operation(metal::MTLBlendOperation::Add);
    color_attachment.set_alpha_blend_operation(metal::MTLBlendOperation::Add);
    color_attachment.set_source_rgb_blend_factor(metal::MTLBlendFactor::SourceAlpha);
    color_attachment.set_source_alpha_blend_factor(metal::MTLBlendFactor::One);
    color_attachment.set_destination_rgb_blend_factor(metal::MTLBlendFactor::OneMinusSourceAlpha);
    color_attachment.set_destination_alpha_blend_factor(metal::MTLBlendFactor::One);

    device
        .new_render_pipeline_state(&descriptor)
        .expect("could not create render pipeline state")
}

fn build_path_sprite_pipeline_state(
    device: &metal::DeviceRef,
    library: &metal::LibraryRef,
    label: &str,
    vertex_fn_name: &str,
    fragment_fn_name: &str,
    pixel_format: metal::MTLPixelFormat,
) -> metal::RenderPipelineState {
    let vertex_fn = library
        .get_function(vertex_fn_name, None)
        .expect("error locating vertex function");
    let fragment_fn = library
        .get_function(fragment_fn_name, None)
        .expect("error locating fragment function");

    let descriptor = metal::RenderPipelineDescriptor::new();
    descriptor.set_label(label);
    descriptor.set_vertex_function(Some(vertex_fn.as_ref()));
    descriptor.set_fragment_function(Some(fragment_fn.as_ref()));
    let color_attachment = descriptor.color_attachments().object_at(0).unwrap();
    color_attachment.set_pixel_format(pixel_format);
    color_attachment.set_blending_enabled(true);
    color_attachment.set_rgb_blend_operation(metal::MTLBlendOperation::Add);
    color_attachment.set_alpha_blend_operation(metal::MTLBlendOperation::Add);
    color_attachment.set_source_rgb_blend_factor(metal::MTLBlendFactor::One);
    color_attachment.set_source_alpha_blend_factor(metal::MTLBlendFactor::One);
    color_attachment.set_destination_rgb_blend_factor(metal::MTLBlendFactor::OneMinusSourceAlpha);
    color_attachment.set_destination_alpha_blend_factor(metal::MTLBlendFactor::One);

    device
        .new_render_pipeline_state(&descriptor)
        .expect("could not create render pipeline state")
}

fn build_path_rasterization_pipeline_state(
    device: &metal::DeviceRef,
    library: &metal::LibraryRef,
    label: &str,
    vertex_fn_name: &str,
    fragment_fn_name: &str,
    pixel_format: metal::MTLPixelFormat,
    path_sample_count: u32,
) -> metal::RenderPipelineState {
    let vertex_fn = library
        .get_function(vertex_fn_name, None)
        .expect("error locating vertex function");
    let fragment_fn = library
        .get_function(fragment_fn_name, None)
        .expect("error locating fragment function");

    let descriptor = metal::RenderPipelineDescriptor::new();
    descriptor.set_label(label);
    descriptor.set_vertex_function(Some(vertex_fn.as_ref()));
    descriptor.set_fragment_function(Some(fragment_fn.as_ref()));
    if path_sample_count > 1 {
        descriptor.set_raster_sample_count(path_sample_count as _);
        descriptor.set_alpha_to_coverage_enabled(false);
    }
    let color_attachment = descriptor.color_attachments().object_at(0).unwrap();
    color_attachment.set_pixel_format(pixel_format);
    color_attachment.set_blending_enabled(true);
    color_attachment.set_rgb_blend_operation(metal::MTLBlendOperation::Add);
    color_attachment.set_alpha_blend_operation(metal::MTLBlendOperation::Add);
    color_attachment.set_source_rgb_blend_factor(metal::MTLBlendFactor::One);
    color_attachment.set_source_alpha_blend_factor(metal::MTLBlendFactor::One);
    color_attachment.set_destination_rgb_blend_factor(metal::MTLBlendFactor::OneMinusSourceAlpha);
    color_attachment.set_destination_alpha_blend_factor(metal::MTLBlendFactor::OneMinusSourceAlpha);

    device
        .new_render_pipeline_state(&descriptor)
        .expect("could not create render pipeline state")
}

fn build_custom_work_counts(scene: &Scene) -> Option<(u32, u32, u32, u32)> {
    if scene.custom_draws.is_empty() && scene.custom_computes.is_empty() {
        return None;
    }

    let mut has_window_custom_draw = false;
    let mut offscreen_target_hashes = std::collections::HashSet::new();
    for draw in scene.custom_draws.iter() {
        if draw.target.is_some() {
            offscreen_target_hashes.insert(draw.batch_key.target_hash);
        } else {
            has_window_custom_draw = true;
        }
    }

    let custom_render_pass_count =
        offscreen_target_hashes.len() as u32 + u32::from(has_window_custom_draw);
    let custom_compute_pass_count = u32::from(!scene.custom_computes.is_empty());

    Some((
        scene.custom_draws.len() as u32,
        scene.custom_computes.len() as u32,
        custom_render_pass_count,
        custom_compute_pass_count,
    ))
}

fn build_custom_gpu_profile(scene: &Scene) -> Option<CustomGpuFrameProfile> {
    let (
        custom_draw_count,
        custom_compute_count,
        custom_render_pass_count,
        custom_compute_pass_count,
    ) = build_custom_work_counts(scene)?;

    Some(CustomGpuFrameProfile {
        custom_draw_count,
        custom_compute_count,
        custom_render_pass_count,
        custom_compute_pass_count,
        gpu_time_ns: None,
    })
}

fn build_custom_frame_diagnostics(scene: &Scene) -> Option<CustomFrameDiagnostics> {
    let (
        custom_draw_count,
        custom_compute_count,
        custom_render_pass_count,
        custom_compute_pass_count,
    ) = build_custom_work_counts(scene)?;

    Some(CustomFrameDiagnostics {
        custom_draw_count,
        custom_compute_count,
        custom_render_pass_count,
        custom_compute_pass_count,
        retry_count: 0,
        cpu_encode_time_ns: 0,
        submit_to_scheduled_ns: None,
        submit_to_completed_ns: None,
        scheduled_to_completed_ns: None,
        gpu_time_ns: None,
    })
}

fn metal_command_buffer_gpu_time_ns(command_buffer: &metal::CommandBufferRef) -> Option<u64> {
    #[allow(clippy::disallowed_methods)]
    unsafe {
        let has_gpu_start_time: bool =
            msg_send![command_buffer, respondsToSelector: sel!(GPUStartTime)];
        let has_gpu_end_time: bool =
            msg_send![command_buffer, respondsToSelector: sel!(GPUEndTime)];
        if !has_gpu_start_time || !has_gpu_end_time {
            return None;
        }

        let gpu_start_time: f64 = msg_send![command_buffer, GPUStartTime];
        let gpu_end_time: f64 = msg_send![command_buffer, GPUEndTime];
        if gpu_start_time <= 0.0 || gpu_end_time < gpu_start_time {
            return None;
        }

        let gpu_time_ns = ((gpu_end_time - gpu_start_time) * 1_000_000_000.0).round();
        if !gpu_time_ns.is_finite() || gpu_time_ns < 0.0 {
            return None;
        }

        Some(gpu_time_ns as u64)
    }
}

fn duration_as_u64_nanoseconds(duration: std::time::Duration) -> u64 {
    duration.as_nanos().min(u64::MAX as u128) as u64
}

fn align_offset_to(offset: &mut usize, alignment: usize) {
    let alignment = alignment.max(256);
    *offset = (*offset).div_ceil(alignment) * alignment;
}

// Align to multiples of 256 make Metal happy.
fn align_offset(offset: &mut usize) {
    align_offset_to(offset, 256);
}

#[repr(C)]
enum ShadowInputIndex {
    Vertices = 0,
    Shadows = 1,
    ViewportSize = 2,
}

#[repr(C)]
enum QuadInputIndex {
    Vertices = 0,
    Quads = 1,
    ViewportSize = 2,
}

#[repr(C)]
enum UnderlineInputIndex {
    Vertices = 0,
    Underlines = 1,
    ViewportSize = 2,
}

#[repr(C)]
enum SpriteInputIndex {
    Vertices = 0,
    Sprites = 1,
    ViewportSize = 2,
    AtlasTextureSize = 3,
    AtlasTexture = 4,
}

#[repr(C)]
enum SurfaceInputIndex {
    Vertices = 0,
    Surfaces = 1,
    ViewportSize = 2,
    TextureSize = 3,
    YTexture = 4,
    CbCrTexture = 5,
}

#[repr(C)]
enum PathRasterizationInputIndex {
    Vertices = 0,
    ViewportSize = 1,
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct PathSprite {
    pub bounds: Bounds<ScaledPixels>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct SurfaceBounds {
    pub bounds: Bounds<ScaledPixels>,
    pub content_mask: ContentMask<ScaledPixels>,
}
