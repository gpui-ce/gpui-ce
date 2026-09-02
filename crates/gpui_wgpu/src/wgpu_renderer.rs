use crate::{CompositorGpuHint, WgpuAtlas, WgpuContext, WgpuDeviceRequirements};
use gpui::{DevicePixels, GpuSpecs, Scene, Size};

#[cfg(test)]
use gpui::BackdropFilter;
#[cfg(all(test, feature = "test-support", not(target_family = "wasm")))]
use gpui::{Bounds, Point, Quad, ScaledPixels, Underline};
#[cfg(test)]
use gpui_render::blur::{BlurKernel, MAX_GAUSSIAN_SAMPLES_PER_SIDE};
#[cfg(test)]
use gpui_render::shaders::interface as shader_interface;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

mod buffers;
mod drawing;
mod filters;
mod frame;
#[cfg(all(feature = "test-support", not(target_family = "wasm")))]
mod headless;
mod path_types;
mod pipelines;
mod platform;
mod resources;
mod settings;
mod surfaces;
mod target;

#[cfg(all(feature = "test-support", not(target_family = "wasm")))]
pub use headless::WgpuHeadlessRenderer;
use pipelines::WgpuPipelines as ShaderPipelines;
use resources::{GlobalBufferLayout, ResourceMetadata, WgpuResources};
use settings::RenderingParameters;
pub use settings::{FontRasterizationSettings, SubpixelOrder, WgpuSurfaceConfig};
use target::RenderTarget;

/// Shared GPU context reference, used to coordinate device recovery across multiple windows.
pub type GpuContext = Rc<RefCell<Option<WgpuContext>>>;

struct GpuFaultState {
    pending_error: Arc<Mutex<Option<String>>>,
    consecutive_failed_frames: u32,
    device_lost: Arc<std::sync::atomic::AtomicBool>,
    #[cfg(not(target_family = "wasm"))]
    recovery_not_before: Option<std::time::Instant>,
}

pub struct WgpuRenderer {
    /// Shared GPU context for device recovery coordination (unused on WASM).
    #[allow(dead_code)]
    context: Option<GpuContext>,
    /// Compositor GPU hint for adapter selection (unused on WASM).
    #[allow(dead_code)]
    compositor_gpu: Option<CompositorGpuHint>,
    /// Application-requested extra wgpu features/limits, stored for device recovery.
    #[allow(dead_code)]
    extra_requirements: Option<WgpuDeviceRequirements>,
    resources: Option<WgpuResources>,
    target: RenderTarget,
    atlas: Arc<WgpuAtlas>,
    globals: GlobalBufferLayout,
    rendering_params: RenderingParameters,
    subpixel_order: SubpixelOrder,
    dual_source_blending: bool,
    adapter_info: wgpu::AdapterInfo,
    uploaded_globals: Option<frame::GlobalUniformState>,
    faults: GpuFaultState,
}

impl WgpuRenderer {
    fn resources(&self) -> &WgpuResources {
        self.resources
            .as_ref()
            .expect("GPU resources not available")
    }

    fn resources_mut(&mut self) -> &mut WgpuResources {
        self.resources
            .as_mut()
            .expect("GPU resources not available")
    }

    fn new_internal(
        gpu_context: Option<GpuContext>,
        context: &WgpuContext,
        surface: Option<wgpu::Surface<'static>>,
        config: WgpuSurfaceConfig,
        compositor_gpu: Option<CompositorGpuHint>,
        extra_requirements: Option<WgpuDeviceRequirements>,
        atlas: Arc<WgpuAtlas>,
    ) -> anyhow::Result<Self> {
        let target =
            RenderTarget::new(&context.adapter, &context.device, surface.as_ref(), config)?;
        if let Some(surface) = surface.as_ref() {
            surface.configure(&context.device, target.configuration());
        }

        let dual_source_blending = context.supports_dual_source_blending();
        let rendering_params = RenderingParameters::new(&context.adapter, target.format());
        let adapter_info = context.adapter.get_info();
        let (resources, metadata) = WgpuResources::new(
            context,
            surface,
            target.configuration(),
            &rendering_params,
            dual_source_blending,
        )?;
        let ResourceMetadata {
            globals,
            last_error,
        } = metadata;

        Ok(Self {
            context: gpu_context,
            compositor_gpu,
            extra_requirements,
            resources: Some(resources),
            target,
            atlas,
            globals,
            rendering_params,
            subpixel_order: SubpixelOrder::RedGreenBlue,
            dual_source_blending,
            adapter_info,
            uploaded_globals: None,
            faults: GpuFaultState {
                pending_error: last_error,
                consecutive_failed_frames: 0,
                device_lost: context.device_lost_flag(),
                #[cfg(not(target_family = "wasm"))]
                recovery_not_before: None,
            },
        })
    }

    pub fn update_drawable_size(&mut self, size: Size<DevicePixels>) {
        if !self.target.resize(size) {
            return;
        }
        let config = self.target.configuration().clone();
        let Some(resources) = self.resources.as_mut() else {
            return;
        };
        resources.invalidate_intermediate_textures();
        if let Some(surface) = resources.surface.as_ref() {
            surface.configure(&resources.device, &config);
        }
    }

    /// Selects the physical LCD component order used by subpixel glyph correction.
    pub fn set_subpixel_order(&mut self, order: SubpixelOrder) {
        self.subpixel_order = order;
    }

    /// Updates platform-specific text rasterization parameters.
    pub fn set_font_rasterization_settings(&mut self, settings: FontRasterizationSettings) {
        self.rendering_params.font_rasterization = settings;
        self.subpixel_order = settings.subpixel_order;
    }

    /// Compatibility wrapper for callers that still report the layout as a boolean.
    pub fn set_subpixel_layout(&mut self, is_bgr: bool) {
        self.set_subpixel_order(if is_bgr {
            SubpixelOrder::BlueGreenRed
        } else {
            SubpixelOrder::RedGreenBlue
        });
    }

    pub fn update_transparency(&mut self, transparent: bool) {
        if !self.target.set_transparent(transparent) {
            return;
        }
        let config = self.target.configuration().clone();
        if let Some(resources) = self.resources.as_ref() {
            if let Some(surface) = resources.surface.as_ref() {
                surface.configure(&resources.device, &config);
            }
        }
        self.rebuild_pipelines();
    }

    pub(super) fn rebuild_pipelines(&mut self) {
        let config = self.target.configuration();
        let Some(resources) = self.resources.as_mut() else {
            return;
        };
        resources.pipelines = ShaderPipelines::new(
            &resources.device,
            &resources.bind_group_layouts,
            config.format,
            config.alpha_mode,
            self.rendering_params.path_sample_count,
            self.dual_source_blending,
            resources.renderer_tier,
        );
    }

    #[allow(dead_code)]
    pub fn viewport_size(&self) -> Size<DevicePixels> {
        self.target.viewport_size()
    }

    pub fn sprite_atlas(&self) -> &Arc<WgpuAtlas> {
        &self.atlas
    }

    pub fn supports_dual_source_blending(&self) -> bool {
        self.dual_source_blending
    }

    pub fn gpu_context(&self) -> (Arc<wgpu::Device>, Arc<wgpu::Queue>) {
        let resources = self.resources();
        (resources.device.clone(), resources.queue.clone())
    }

    pub fn gpu_specs(&self) -> GpuSpecs {
        GpuSpecs {
            is_software_emulated: self.adapter_info.device_type == wgpu::DeviceType::Cpu,
            device_name: self.adapter_info.name.clone(),
            driver_name: self.adapter_info.driver.clone(),
            driver_info: self.adapter_info.driver_info.clone(),
        }
    }

    pub fn max_texture_size(&self) -> u32 {
        self.target.maximum_dimension()
    }

    /// Encodes and submits a complete scene into an arbitrary color target.
    /// Swapchain drawing, window capture, and headless rendering all use this path so
    /// platform integrations can't diverge in batching, filters, or pipeline state.
    fn render_to_view(&mut self, scene: &Scene, frame_view: &wgpu::TextureView) -> bool {
        frame::render_to_view(self, scene, frame_view, None).is_some()
    }

    #[cfg(all(feature = "test-support", not(target_family = "wasm")))]
    fn render_to_view_with_readback(
        &mut self,
        scene: &Scene,
        frame_view: &wgpu::TextureView,
        readback: frame::ReadbackCopy<'_>,
    ) -> Option<wgpu::SubmissionIndex> {
        frame::render_to_view(self, scene, frame_view, Some(readback))
    }
}

fn begin_color_render_pass<'encoder>(
    encoder: &'encoder mut wgpu::CommandEncoder,
    label: &'encoder str,
    target: &'encoder wgpu::TextureView,
    load: wgpu::LoadOp<wgpu::Color>,
) -> wgpu::RenderPass<'encoder> {
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some(label),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: target,
            resolve_target: None,
            ops: wgpu::Operations {
                load,
                store: wgpu::StoreOp::Store,
            },
            depth_slice: None,
        })],
        depth_stencil_attachment: None,
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wgpu_renderer::filters::FrameUniformRequirements;

    #[test]
    fn rust_storage_types_match_shader_strides() {
        fn module_layout(source: &wgsl_rs::Source) -> (naga::Module, naga::proc::Layouter) {
            let source = source.wgsl_source().expect("shader must generate WGSL");
            let module =
                naga::front::wgsl::parse_str(&format!("enable dual_source_blending;\n{source}"))
                    .expect("generated shader must parse as WGSL");
            let mut layouter = naga::proc::Layouter::default();
            layouter
                .update(module.to_ctx())
                .expect("generated shader types must have valid layouts");
            (module, layouter)
        }

        let (base, base_layout) = module_layout(&gpui_render::shaders::base::WGSL_SOURCE);
        let (subpixel, subpixel_layout) =
            module_layout(&gpui_render::shaders::subpixel_sprite::WGSL_SOURCE);

        for abi in shader_interface::SCENE_STORAGE_ABI
            .iter()
            .chain(path_types::STORAGE_ABI)
        {
            let base_handle = base
                .types
                .iter()
                .find(|(_, ty)| ty.name.as_deref() == Some(abi.wgsl_type))
                .map(|(handle, _)| handle);
            let shader_stride = if let Some(handle) = base_handle {
                base_layout[handle].to_stride()
            } else {
                let (handle, _) = subpixel
                    .types
                    .iter()
                    .find(|(_, ty)| ty.name.as_deref() == Some(abi.wgsl_type))
                    .unwrap_or_else(|| panic!("missing WGSL type {}", abi.wgsl_type));
                subpixel_layout[handle].to_stride()
            };
            assert_eq!(
                shader_stride as usize, abi.rust_stride,
                "Rust and WGSL layouts differ for {}",
                abi.wgsl_type,
            );
        }
    }

    #[test]
    fn large_filter_scenes_reserve_unique_uniform_slots() {
        let mut scene = Scene::default();
        scene
            .backdrop_filters
            .resize_with(100, BackdropFilter::default);
        scene.finish();

        assert_eq!(
            frame::FrameRequirements::for_scene(
                &scene,
                super::buffers::InstanceTransport::StorageBuffer
            )
            .uniforms,
            FrameUniformRequirements {
                filter_count: 401,
                surface_count: 0,
            }
        );
    }

    #[test]
    fn large_blur_radii_expand_tap_spacing_instead_of_truncating_the_kernel() {
        let kernel = BlurKernel::for_radius(100.0).expect("positive radii have a blur kernel");
        assert_eq!(kernel.standard_deviation, 50.0);
        assert_eq!(kernel.sample_count, MAX_GAUSSIAN_SAMPLES_PER_SIDE);
        assert!(kernel.sample_step > 4.0);
    }

    #[cfg(any(target_os = "macos", target_os = "windows"))]
    #[test]
    fn desktop_blending_preserves_native_alpha_accumulation() {
        let straight = pipelines::desktop_scene_blend_state(wgpu::CompositeAlphaMode::Opaque);
        assert_eq!(straight.color.src_factor, wgpu::BlendFactor::SrcAlpha);
        assert_eq!(straight.alpha.dst_factor, wgpu::BlendFactor::One);

        let premultiplied =
            pipelines::desktop_scene_blend_state(wgpu::CompositeAlphaMode::PreMultiplied);
        assert_eq!(premultiplied.color.src_factor, wgpu::BlendFactor::One);
        assert_eq!(premultiplied.alpha.dst_factor, wgpu::BlendFactor::One);
    }

    #[cfg(all(feature = "test-support", not(target_family = "wasm")))]
    #[test]
    fn underline_opacity_is_applied_once() -> anyhow::Result<()> {
        let context = WgpuContext::new_headless(None)?;
        let size = Size {
            width: DevicePixels(3),
            height: DevicePixels(3),
        };
        let mut renderer = WgpuRenderer::new_headless(&context, size)?;
        let bounds = Bounds {
            origin: Point {
                x: ScaledPixels(0.0),
                y: ScaledPixels(0.0),
            },
            size: Size {
                width: ScaledPixels(3.0),
                height: ScaledPixels(3.0),
            },
        };
        let mut scene = Scene::default();
        // Forces a non-zero first-instance index in the mixed-type arena.
        scene.insert_primitive(Quad::default());
        scene.insert_primitive(Underline {
            order: 0,
            padding: 0,
            bounds,
            content_mask: gpui::ContentMask { bounds },
            color: gpui::hsla(0.0, 1.0, 0.5, 0.5).into(),
            thickness: ScaledPixels(1.0),
            wavy: false.into(),
        });
        scene.finish();

        let image = renderer.render_to_image(&scene)?;
        let center = image.get_pixel(1, 1).0;
        assert_eq!(
            center,
            [128, 0, 0, 255],
            "must match the retired Metal renderer's mixed primitive baseline"
        );
        Ok(())
    }

    #[cfg(all(feature = "test-support", not(target_family = "wasm")))]
    #[test]
    fn quad_backgrounds_match_legacy_metal_pixels() -> anyhow::Result<()> {
        const LEGACY: &[u8] = &[
            255, 0, 0, 255, 255, 0, 0, 255, 0, 0, 0, 255, 38, 110, 217, 255, 255, 0, 0, 255, 255,
            0, 0, 255, 38, 110, 217, 255, 0, 0, 0, 255, 10, 34, 4, 255, 57, 196, 22, 255, 193, 114,
            151, 255, 223, 154, 105, 255, 57, 196, 22, 255, 10, 34, 4, 255, 166, 61, 182, 255, 192,
            113, 151, 255,
        ];
        let bounds = |x, y| Bounds {
            origin: Point {
                x: ScaledPixels(x),
                y: ScaledPixels(y),
            },
            size: Size {
                width: ScaledPixels(2.0),
                height: ScaledPixels(2.0),
            },
        };
        let mut scene = Scene::default();
        for (order, (bounds, background)) in [
            (
                bounds(0.0, 0.0),
                gpui::solid_background(gpui::hsla(0.0, 1.0, 0.5, 1.0)),
            ),
            (
                bounds(2.0, 0.0),
                gpui::checkerboard(gpui::hsla(0.6, 0.7, 0.5, 1.0), 1.0),
            ),
            (
                bounds(0.0, 2.0),
                gpui::pattern_slash(gpui::hsla(0.3, 0.8, 0.5, 1.0), 1.0, 1.0),
            ),
            (
                bounds(2.0, 2.0),
                gpui::linear_gradient(
                    45.0,
                    gpui::linear_color_stop(gpui::hsla(0.8, 0.9, 0.4, 1.0), 0.0),
                    gpui::linear_color_stop(gpui::hsla(0.1, 0.8, 0.6, 1.0), 1.0),
                ),
            ),
        ]
        .into_iter()
        .enumerate()
        {
            scene.insert_primitive(Quad {
                order: order as u32,
                bounds,
                content_mask: gpui::ContentMask { bounds },
                background,
                ..Default::default()
            });
        }
        scene.finish();
        let context = WgpuContext::new_headless(None)?;
        let mut renderer = WgpuRenderer::new_headless(
            &context,
            Size {
                width: DevicePixels(4),
                height: DevicePixels(4),
            },
        )?;
        let actual = renderer.render_to_image(&scene)?;
        for (index, (actual, expected)) in actual.as_raw().iter().zip(LEGACY).enumerate() {
            assert_eq!(
                actual,
                expected,
                "legacy mismatch at pixel ({}, {}), channel {}",
                (index / 4) % 4,
                (index / 4) / 4,
                index % 4
            );
        }
        assert_eq!(actual.as_raw().len(), LEGACY.len());
        Ok(())
    }

    #[cfg(all(feature = "test-support", not(target_family = "wasm")))]
    #[test]
    fn cached_filter_bindings_accept_frame_local_dynamic_offsets() -> anyhow::Result<()> {
        let context = WgpuContext::new_headless(None)?;
        let size = Size {
            width: DevicePixels(4),
            height: DevicePixels(4),
        };
        let mut renderer = WgpuRenderer::new_headless(&context, size)?;
        let bounds = Bounds {
            origin: Point {
                x: ScaledPixels(0.0),
                y: ScaledPixels(0.0),
            },
            size: Size {
                width: ScaledPixels(4.0),
                height: ScaledPixels(4.0),
            },
        };
        let mut scene = Scene::default();
        scene.insert_primitive(BackdropFilter {
            bounds,
            content_mask: gpui::ContentMask { bounds },
            filters: smallvec::smallvec![gpui::ScaledFilter::Blur(ScaledPixels(1.0))],
            opacity: 1.0,
            ..BackdropFilter::default()
        });
        scene.finish();

        renderer.render_to_image(&scene)?;
        renderer.render_to_image(&scene)?;
        Ok(())
    }

    #[cfg(all(feature = "test-support", not(target_family = "wasm")))]
    #[test]
    fn odd_sized_blur_preserves_edge_symmetry() -> anyhow::Result<()> {
        let context = WgpuContext::new_headless(None)?;
        let size = Size {
            width: DevicePixels(5),
            height: DevicePixels(5),
        };
        let mut renderer = WgpuRenderer::new_headless(&context, size)?;
        let full_bounds = Bounds {
            origin: Point {
                x: ScaledPixels(0.0),
                y: ScaledPixels(0.0),
            },
            size: Size {
                width: ScaledPixels(5.0),
                height: ScaledPixels(5.0),
            },
        };
        let center_bounds = Bounds {
            origin: Point {
                x: ScaledPixels(2.0),
                y: ScaledPixels(2.0),
            },
            size: Size {
                width: ScaledPixels(1.0),
                height: ScaledPixels(1.0),
            },
        };
        let mut scene = Scene::default();
        scene.insert_primitive(Quad {
            order: 0,
            bounds: full_bounds,
            content_mask: gpui::ContentMask {
                bounds: full_bounds,
            },
            background: gpui::solid_background(gpui::hsla(0.0, 0.0, 0.0, 1.0)),
            ..Default::default()
        });
        scene.insert_primitive(Quad {
            order: 1,
            bounds: center_bounds,
            content_mask: gpui::ContentMask {
                bounds: center_bounds,
            },
            background: gpui::solid_background(gpui::hsla(0.0, 0.0, 1.0, 1.0)),
            ..Default::default()
        });
        scene.insert_primitive(BackdropFilter {
            order: 2,
            bounds: full_bounds,
            content_mask: gpui::ContentMask {
                bounds: full_bounds,
            },
            filters: smallvec::smallvec![gpui::ScaledFilter::Blur(ScaledPixels(2.0))],
            opacity: 1.0,
            ..Default::default()
        });
        scene.finish();

        let image = renderer.render_to_image(&scene)?;
        for y in 0..5 {
            for x in 0..5 {
                let actual = image.get_pixel(x, y).0;
                let horizontal = image.get_pixel(4 - x, y).0;
                let vertical = image.get_pixel(x, 4 - y).0;
                for channel in 0..4 {
                    assert!(
                        actual[channel].abs_diff(horizontal[channel]) <= 1,
                        "horizontal blur asymmetry at ({x}, {y}), channel {channel}: {actual:?} vs {horizontal:?}"
                    );
                    assert!(
                        actual[channel].abs_diff(vertical[channel]) <= 1,
                        "vertical blur asymmetry at ({x}, {y}), channel {channel}: {actual:?} vs {vertical:?}"
                    );
                }
            }
        }
        assert!(
            image.get_pixel(1, 2).0[0] > 0,
            "blur must spread from the center pixel"
        );
        Ok(())
    }

    #[cfg(all(feature = "test-support", not(target_family = "wasm")))]
    #[test]
    fn alternating_instance_batches_fit_the_exact_upload_arena() -> anyhow::Result<()> {
        let context = WgpuContext::new_headless(None)?;
        let size = Size {
            width: DevicePixels(1),
            height: DevicePixels(1),
        };
        let mut renderer = WgpuRenderer::new_headless(&context, size)?;
        let bounds = Bounds {
            origin: Point {
                x: ScaledPixels(0.0),
                y: ScaledPixels(0.0),
            },
            size: Size {
                width: ScaledPixels(1.0),
                height: ScaledPixels(1.0),
            },
        };
        let mut scene = Scene::default();
        for index in 0..512 {
            scene.insert_primitive(Quad {
                order: index * 2,
                bounds,
                content_mask: gpui::ContentMask { bounds },
                background: gpui::solid_background(gpui::hsla(0.0, 0.0, 0.0, 1.0)),
                ..Default::default()
            });
            scene.insert_primitive(Underline {
                order: index * 2 + 1,
                padding: 0,
                bounds,
                content_mask: gpui::ContentMask { bounds },
                color: gpui::hsla(0.0, 0.0, 1.0, 1.0).into(),
                thickness: ScaledPixels(1.0),
                wavy: false.into(),
            });
        }
        scene.finish();
        assert_eq!(
            scene
                .render_commands()
                .iter()
                .filter(|command| matches!(command, gpui::RenderCommand::Batch(_)))
                .count(),
            1024
        );

        renderer.render_to_image(&scene)?;
        Ok(())
    }
}
