//! The WGPU renderer behind the DirectX renderer's interface, for the `wgpu`
//! feature. Applications can then share the window's device through
//! `gpui_wgpu::WgpuContextHandle` and composite their own WGPU textures
//! (with gpui's `custom-gpu` feature).

use std::sync::Arc;

use gpui::{GpuSpecs, Scene, Size, WindowBackgroundAppearance};
use gpui_wgpu::{
    GpuContext, WgpuAtlas, WgpuContextHandle, WgpuDeviceRequirements, WgpuRenderer,
    WgpuSurfaceConfig, wgpu,
};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::GetClientRect;

/// The device every window shares, and what the application requires of it.
#[derive(Clone, Default)]
pub struct Context {
    gpu: GpuContext,
    requirements: std::cell::RefCell<Option<WgpuDeviceRequirements>>,
}

impl Context {
    pub fn set_requirements(&self, requirements: WgpuDeviceRequirements) {
        *self.requirements.borrow_mut() = Some(requirements);
    }

    fn requirements(&self) -> Option<WgpuDeviceRequirements> {
        self.requirements.borrow().clone()
    }
}

pub type Renderer = WindowsWgpuRenderer;

pub fn new_renderer(hwnd: HWND, context: &Context) -> anyhow::Result<Renderer> {
    WindowsWgpuRenderer::new(hwnd, context)
}

fn raw_window_handle_from_hwnd(hwnd: HWND) -> raw_window_handle::RawWindowHandle {
    let mut handle = raw_window_handle::Win32WindowHandle::new(
        std::num::NonZeroIsize::new(hwnd.0 as isize)
            .expect("an HWND is never the null handle"),
    );
    // The instance handle is not needed for wgpu surface creation.
    handle.hinstance = None;
    raw_window_handle::RawWindowHandle::Win32(handle)
}

fn client_size(hwnd: HWND) -> Size<gpui::DevicePixels> {
    let mut rect = Default::default();
    let ok = unsafe { GetClientRect(hwnd, &mut rect) };
    if ok.is_err() {
        return Size {
            width: gpui::DevicePixels(1),
            height: gpui::DevicePixels(1),
        };
    }
    Size {
        width: gpui::DevicePixels((rect.right - rect.left).max(1)),
        height: gpui::DevicePixels((rect.bottom - rect.top).max(1)),
    }
}

pub struct WindowsWgpuRenderer {
    renderer: WgpuRenderer,
    hwnd: HWND,
    raw_window_handle: raw_window_handle::RawWindowHandle,
    size: Size<gpui::DevicePixels>,
    transparent: bool,
}

impl WindowsWgpuRenderer {
    pub fn new(hwnd: HWND, context: &Context) -> anyhow::Result<Self> {
        let raw_window_handle = raw_window_handle_from_hwnd(hwnd);
        let size = client_size(hwnd);
        let renderer = WgpuRenderer::new_for_raw_window_handle(
            context.gpu.clone(),
            raw_window_handle,
            WgpuSurfaceConfig {
                size,
                transparent: false,
                preferred_present_mode: None,
            },
            context.requirements(),
        )?;
        Ok(Self {
            renderer,
            hwnd,
            raw_window_handle,
            size,
            transparent: false,
        })
    }

    pub fn sprite_atlas(&self) -> Arc<dyn gpui::PlatformAtlas> {
        self.renderer.sprite_atlas().clone()
    }

    /// Called on DPI/display changes; the surface is reconfigured on the next
    /// draw from the window's client rect.
    pub fn resize(&mut self, size: Size<gpui::DevicePixels>) -> anyhow::Result<()> {
        if size != self.size {
            self.size = size;
            self.renderer.update_drawable_size(size);
        }
        Ok(())
    }

    /// The platform broadcasts device loss; the WGPU renderer self-heals on
    /// the next draw, so this only attempts an immediate recovery.
    pub fn handle_device_lost(
        &mut self,
        _directx_devices: &crate::DirectXDevices,
    ) -> anyhow::Result<()> {
        if let Err(error) = self
            .renderer
            .recover_raw_window_handle(self.raw_window_handle)
        {
            log::warn!("immediate GPU recovery deferred: {error}");
        }
        Ok(())
    }

    /// DirectX-specific bookkeeping; the WGPU renderer has no skipped-draw
    /// state to clear.
    pub fn mark_drawable(&mut self) {}

    pub fn update_drawable_size(&mut self, size: Size<gpui::DevicePixels>) {
        self.renderer.update_drawable_size(size);
    }

    /// The surface's alpha mode decides how DWM composites the swapchain, so
    /// transparency changes go through WGPU rather than window styles.
    pub fn update_transparency(&mut self, transparent: bool) {
        self.renderer.update_transparency(transparent);
    }

    pub fn destroy(&mut self) {
        self.renderer.destroy();
    }

    /// Draws `scene`, recovering the device first if it was lost. Sizes the
    /// surface to the window's client rect, which Windows changes without
    /// telling the renderer.
    pub fn draw(
        &mut self,
        scene: &Scene,
        background_appearance: WindowBackgroundAppearance,
    ) -> anyhow::Result<()> {
        let transparent = background_appearance != WindowBackgroundAppearance::Opaque;
        if transparent != self.transparent {
            self.transparent = transparent;
            self.renderer.update_transparency(transparent);
        }
        let size = client_size(self.hwnd);
        if size != self.size {
            self.size = size;
            self.renderer.update_drawable_size(size);
        }
        if self.renderer.device_lost() {
            if let Err(error) = self
                .renderer
                .recover_raw_window_handle(self.raw_window_handle)
            {
                log::warn!("GPU recovery failed, will retry on next frame: {error}");
            }
            if self.renderer.device_lost() {
                return Ok(());
            }
        }
        self.renderer.draw(scene);
        Ok(())
    }

    pub fn gpu_specs(&self) -> anyhow::Result<GpuSpecs> {
        Ok(self.renderer.gpu_specs())
    }

    pub fn gpu_context(&self) -> (Arc<wgpu::Device>, Arc<wgpu::Queue>) {
        self.renderer.gpu_context()
    }

    pub fn device_lost(&self) -> bool {
        self.renderer.device_lost()
    }

    pub fn gpu_context_info(&self) -> Option<WgpuContextHandle> {
        self.renderer.gpu_context_info()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn render_to_image(
        &mut self,
        _scene: &Scene,
        _background_appearance: WindowBackgroundAppearance,
    ) -> anyhow::Result<image::RgbaImage> {
        anyhow::bail!("render_to_image is not implemented for the Windows WGPU renderer yet")
    }
}

impl Drop for WindowsWgpuRenderer {
    fn drop(&mut self) {
        self.renderer.destroy();
    }
}
