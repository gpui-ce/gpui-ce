#[cfg(not(target_family = "wasm"))]
use std::rc::Rc;
use std::sync::Arc;

use gpui::Scene;

#[cfg(not(target_family = "wasm"))]
use crate::{CompositorGpuHint, NativeBackend, SoftwareAdapterPolicy, WgpuDeviceRequirements};
use crate::{WgpuAtlas, WgpuContext};

#[cfg(not(target_family = "wasm"))]
use super::GpuContext;
use super::{WgpuRenderer, WgpuSurfaceConfig};

#[cfg(not(target_family = "wasm"))]
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};

impl WgpuRenderer {
    /// Creates a renderer whose surface and GPU context follow the native window lifetime.
    #[cfg(not(target_family = "wasm"))]
    pub fn new<W>(
        gpu_context: GpuContext,
        window: &W,
        config: WgpuSurfaceConfig,
        compositor_gpu: Option<CompositorGpuHint>,
        extra_requirements: Option<WgpuDeviceRequirements>,
    ) -> anyhow::Result<Self>
    where
        W: HasWindowHandle + HasDisplayHandle + std::fmt::Debug + Send + Sync + Clone + 'static,
    {
        let window_handle = window
            .window_handle()
            .map_err(|error| anyhow::anyhow!("failed to get window handle: {error}"))?;

        let mut context_slot = gpu_context.borrow_mut();
        let (context, surface) = match context_slot.as_mut() {
            Some(context) => {
                let surface = create_surface(&context.instance, window_handle.as_raw())?;
                context.check_compatible_with_surface(&surface)?;
                (context, surface)
            }
            None => {
                let (context, surface) = initialize_context_and_surface(
                    window,
                    window_handle.as_raw(),
                    compositor_gpu,
                    SoftwareAdapterPolicy::Allow,
                    extra_requirements.as_ref(),
                )?;
                (context_slot.insert(context), surface)
            }
        };
        let atlas = Arc::new(WgpuAtlas::from_context(context));
        Self::new_internal(
            Some(Rc::clone(&gpu_context)),
            context,
            Some(surface),
            config,
            compositor_gpu,
            extra_requirements,
            atlas,
        )
    }

    #[cfg(target_family = "wasm")]
    #[allow(clippy::arc_with_non_send_sync)]
    pub fn new_from_surface(
        context: &WgpuContext,
        surface: wgpu::Surface<'static>,
        config: WgpuSurfaceConfig,
    ) -> anyhow::Result<Self> {
        Self::new_internal(
            None,
            context,
            Some(surface),
            config,
            None,
            None,
            Arc::new(WgpuAtlas::from_context(context)),
        )
    }

    /// Acquires, renders, and presents one native or canvas surface frame.
    pub fn draw(&mut self, scene: &Scene) -> bool {
        if !self.target.is_configured() {
            return false;
        }
        let frame = match self
            .resources()
            .surface
            .as_ref()
            .expect("draw requires a configured surface")
            .get_current_texture()
        {
            wgpu::CurrentSurfaceTexture::Success(frame) => frame,
            wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
                drop(frame);
                self.reconfigure_surface();
                return false;
            }
            wgpu::CurrentSurfaceTexture::Lost | wgpu::CurrentSurfaceTexture::Outdated => {
                self.reconfigure_surface();
                return false;
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                self.target.request_redraw();
                return false;
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                self.target.request_redraw();
                *self.faults.pending_error.lock().unwrap() =
                    Some("surface texture validation error".to_string());
                return false;
            }
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let rendered = self.render_to_view(scene, &view);
        if rendered {
            frame.present();
        }
        rendered
    }

    fn reconfigure_surface(&mut self) {
        self.target.request_redraw();
        let config = self.target.configuration().clone();
        let resources = self.resources_mut();
        resources
            .surface
            .as_ref()
            .expect("draw requires a configured surface")
            .configure(&resources.device, &config);
    }

    /// Keeps device resources alive while detaching a destroyed native surface.
    pub fn unconfigure_surface(&mut self) {
        self.target.set_configured(false);
        if let Some(resources) = self.resources.as_mut() {
            resources.invalidate_intermediate_textures();
        }
    }

    #[cfg(not(target_family = "wasm"))]
    pub fn replace_surface<W: HasWindowHandle>(
        &mut self,
        window: &W,
        config: WgpuSurfaceConfig,
        instance: &wgpu::Instance,
    ) -> anyhow::Result<()> {
        let window_handle = window
            .window_handle()
            .map_err(|error| anyhow::anyhow!("failed to get window handle: {error}"))?;
        let surface = create_surface(instance, window_handle.as_raw())?;
        // A replacement surface can expose different present modes (for example when a
        // window moves between displays or Wayland/X11 surfaces). Query the new surface,
        // rather than reusing capabilities from the old target.
        let supported_present_modes = {
            let context_slot = self.context.as_ref().ok_or_else(|| {
                anyhow::anyhow!("native surface replacement requires a GPU context")
            })?;
            let context_slot = context_slot.borrow();
            let context = context_slot.as_ref().ok_or_else(|| {
                anyhow::anyhow!("native surface replacement requires an initialized GPU context")
            })?;
            let capabilities = surface.get_capabilities(&context.adapter);
            if capabilities.formats.is_empty() {
                let info = context.adapter.get_info();
                anyhow::bail!(
                    "Adapter {:?} (backend={:?}, device={:#06x}) is not compatible with the display surface for this window.",
                    info.name,
                    info.backend,
                    info.device,
                );
            }
            capabilities.present_modes
        };
        if self.target.apply(config, &supported_present_modes) {
            self.rebuild_pipelines();
        }
        let target_config = self.target.configuration().clone();

        let resources = self
            .resources
            .as_mut()
            .expect("GPU resources not available");
        surface.configure(&resources.device, &target_config);
        resources.surface = Some(surface);
        resources.invalidate_intermediate_textures();
        self.target.set_configured(true);
        Ok(())
    }

    pub fn destroy(&mut self) {
        self.resources.take();
    }

    pub fn device_lost(&self) -> bool {
        self.faults
            .device_lost
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn needs_redraw(&mut self) -> bool {
        self.target.take_redraw_request()
    }

    #[cfg(not(target_family = "wasm"))]
    pub fn recover<W>(&mut self, window: &W) -> anyhow::Result<()>
    where
        W: HasWindowHandle + HasDisplayHandle + std::fmt::Debug + Send + Sync + Clone + 'static,
    {
        let gpu_context = self.context.as_ref().expect("recover requires gpu_context");
        let needs_new_context = gpu_context
            .borrow()
            .as_ref()
            .is_none_or(WgpuContext::device_lost);
        if needs_new_context {
            let now = std::time::Instant::now();
            match self.faults.recovery_not_before {
                None => {
                    self.faults.recovery_not_before =
                        Some(now + std::time::Duration::from_millis(350));
                    anyhow::bail!("waiting for the GPU driver to stabilize before recovery");
                }
                Some(not_before) if now < not_before => {
                    anyhow::bail!("waiting for the GPU driver to stabilize before recovery");
                }
                Some(_) => self.faults.recovery_not_before = None,
            }
        } else {
            self.faults.recovery_not_before = None;
        }

        let window_handle = window
            .window_handle()
            .map_err(|error| anyhow::anyhow!("failed to get window handle: {error}"))?;
        let surface = if needs_new_context {
            log::warn!("GPU device lost, recreating context...");
            self.resources = None;
            *gpu_context.borrow_mut() = None;
            let (new_context, surface) = match initialize_context_and_surface(
                window,
                window_handle.as_raw(),
                self.compositor_gpu,
                SoftwareAdapterPolicy::Reject,
                self.extra_requirements.as_ref(),
            ) {
                Ok(result) => result,
                Err(error) => {
                    self.faults.recovery_not_before =
                        Some(std::time::Instant::now() + std::time::Duration::from_millis(350));
                    return Err(error);
                }
            };
            *gpu_context.borrow_mut() = Some(new_context);
            surface
        } else {
            let context_slot = gpu_context.borrow();
            let instance = &context_slot
                .as_ref()
                .expect("a recovered context must exist")
                .instance;
            create_surface(instance, window_handle.as_raw())?
        };

        let config = self.target.recovery_config();
        let gpu_context = Rc::clone(gpu_context);
        let context_slot = gpu_context.borrow();
        let context = context_slot.as_ref().expect("context should exist");
        self.resources = None;
        self.atlas.handle_device_lost(context);

        let font_rasterization = self.rendering_params.font_rasterization;
        let subpixel_order = self.subpixel_order;
        let mut recovered = Self::new_internal(
            Some(Rc::clone(&gpu_context)),
            context,
            Some(surface),
            config,
            self.compositor_gpu,
            self.extra_requirements.clone(),
            Arc::clone(&self.atlas),
        )?;
        recovered.set_font_rasterization_settings(font_rasterization);
        recovered.set_subpixel_order(subpixel_order);
        *self = recovered;
        log::info!("GPU recovery complete");
        Ok(())
    }

    #[cfg(target_os = "macos")]
    pub fn set_presents_with_transaction(&self, enabled: bool) {
        let Some(surface) = self
            .resources
            .as_ref()
            .and_then(|resources| resources.surface.as_ref())
        else {
            return;
        };
        if let Some(surface) = unsafe { surface.as_hal::<wgpu::hal::api::Metal>() } {
            surface
                .render_layer()
                .lock()
                .setPresentsWithTransaction(enabled);
        }
    }
}

#[cfg(not(target_family = "wasm"))]
fn initialize_context_and_surface<W>(
    window: &W,
    raw_window_handle: raw_window_handle::RawWindowHandle,
    compositor_gpu: Option<CompositorGpuHint>,
    adapter_policy: SoftwareAdapterPolicy,
    extra_requirements: Option<&WgpuDeviceRequirements>,
) -> anyhow::Result<(WgpuContext, wgpu::Surface<'static>)>
where
    W: HasDisplayHandle + std::fmt::Debug + Clone + Send + Sync + 'static,
{
    NativeBackend::try_in_preference_order("a GPU context for the window", |backend| {
        let instance = backend.instance(Some(Box::new(window.clone())));
        let surface = create_surface(&instance.raw, raw_window_handle)?;
        let context = WgpuContext::new_with_adapter_policy(
            instance,
            &surface,
            compositor_gpu,
            adapter_policy,
            extra_requirements,
        )?;
        Ok((context, surface))
    })
}

#[cfg(not(target_family = "wasm"))]
fn create_surface(
    instance: &wgpu::Instance,
    raw_window_handle: raw_window_handle::RawWindowHandle,
) -> anyhow::Result<wgpu::Surface<'static>> {
    unsafe {
        instance
            .create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                raw_display_handle: None,
                raw_window_handle,
            })
            .map_err(|error| anyhow::anyhow!("failed to create surface: {error}"))
    }
}
