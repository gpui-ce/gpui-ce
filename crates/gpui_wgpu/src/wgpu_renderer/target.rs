use gpui::{DevicePixels, Size};

use super::WgpuSurfaceConfig;

/// Configuration and lifecycle state shared by window, canvas, and headless targets.
pub(super) struct RenderTarget {
    config: wgpu::SurfaceConfiguration,
    transparent_alpha_mode: wgpu::CompositeAlphaMode,
    opaque_alpha_mode: wgpu::CompositeAlphaMode,
    maximum_dimension: u32,
    configured: bool,
    needs_redraw: bool,
    clear_color: wgpu::Color,
}

impl RenderTarget {
    pub(super) fn new(
        adapter: &wgpu::Adapter,
        device: &wgpu::Device,
        surface: Option<&wgpu::Surface<'_>>,
        requested: WgpuSurfaceConfig,
    ) -> anyhow::Result<Self> {
        let (format, transparent_alpha_mode, opaque_alpha_mode, present_mode) =
            if let Some(surface) = surface {
                let capabilities = surface.get_capabilities(adapter);
                let format = [
                    wgpu::TextureFormat::Bgra8Unorm,
                    wgpu::TextureFormat::Rgba8Unorm,
                ]
                .into_iter()
                .find(|format| capabilities.formats.contains(format))
                .or_else(|| {
                    capabilities
                        .formats
                        .iter()
                        .find(|format| !format.is_srgb())
                        .copied()
                })
                .or_else(|| capabilities.formats.first().copied())
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "surface reports no texture formats for adapter {:?}",
                        adapter.get_info().name
                    )
                })?;
                let pick_alpha = |preferences: &[wgpu::CompositeAlphaMode]| {
                    preferences
                        .iter()
                        .find(|mode| capabilities.alpha_modes.contains(mode))
                        .copied()
                        .or_else(|| capabilities.alpha_modes.first().copied())
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "surface reports no alpha modes for adapter {:?}",
                                adapter.get_info().name
                            )
                        })
                };
                let transparent = pick_alpha(&[
                    wgpu::CompositeAlphaMode::PreMultiplied,
                    wgpu::CompositeAlphaMode::Inherit,
                ])?;
                let opaque = pick_alpha(&[
                    wgpu::CompositeAlphaMode::Opaque,
                    wgpu::CompositeAlphaMode::Inherit,
                ])?;
                let present_mode = requested
                    .preferred_present_mode
                    .filter(|mode| capabilities.present_modes.contains(mode))
                    .unwrap_or(wgpu::PresentMode::Fifo);
                (format, transparent, opaque, present_mode)
            } else {
                // Native desktop swapchains are BGRA; matching that keeps snapshots comparable.
                (
                    wgpu::TextureFormat::Bgra8Unorm,
                    wgpu::CompositeAlphaMode::PreMultiplied,
                    wgpu::CompositeAlphaMode::Opaque,
                    wgpu::PresentMode::Fifo,
                )
            };
        let maximum_dimension = device.limits().max_texture_dimension_2d;
        let (width, height) = clamped_size(requested.size, maximum_dimension);
        let alpha_mode = if requested.transparent {
            transparent_alpha_mode
        } else {
            opaque_alpha_mode
        };
        Ok(Self {
            config: wgpu::SurfaceConfiguration {
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                format,
                width,
                height,
                present_mode,
                desired_maximum_frame_latency: 2,
                alpha_mode,
                view_formats: Vec::new(),
            },
            transparent_alpha_mode,
            opaque_alpha_mode,
            maximum_dimension,
            configured: surface.is_some(),
            needs_redraw: false,
            clear_color: clear_color(requested.transparent),
        })
    }

    pub(super) fn configuration(&self) -> &wgpu::SurfaceConfiguration {
        &self.config
    }

    pub(super) fn resize(&mut self, size: Size<DevicePixels>) -> bool {
        let (width, height) = clamped_size(size, self.maximum_dimension);
        if (width, height) == (self.config.width, self.config.height) {
            return false;
        }
        self.config.width = width;
        self.config.height = height;
        true
    }

    /// Returns whether blend pipelines must be rebuilt.
    pub(super) fn set_transparent(&mut self, transparent: bool) -> bool {
        self.clear_color = clear_color(transparent);
        let alpha_mode = if transparent {
            self.transparent_alpha_mode
        } else {
            self.opaque_alpha_mode
        };
        if alpha_mode == self.config.alpha_mode {
            return false;
        }
        self.config.alpha_mode = alpha_mode;
        true
    }

    #[cfg(not(target_family = "wasm"))]
    pub(super) fn apply(&mut self, requested: WgpuSurfaceConfig) -> bool {
        self.resize(requested.size);
        if let Some(mode) = requested.preferred_present_mode {
            self.config.present_mode = mode;
        }
        self.set_transparent(requested.transparent)
    }

    #[cfg(not(target_family = "wasm"))]
    pub(super) fn recovery_config(&self) -> WgpuSurfaceConfig {
        WgpuSurfaceConfig {
            size: self.viewport_size(),
            transparent: self.config.alpha_mode != wgpu::CompositeAlphaMode::Opaque,
            preferred_present_mode: Some(self.config.present_mode),
        }
    }

    pub(super) fn viewport_size(&self) -> Size<DevicePixels> {
        Size {
            width: DevicePixels(self.config.width as i32),
            height: DevicePixels(self.config.height as i32),
        }
    }

    pub(super) fn format(&self) -> wgpu::TextureFormat {
        self.config.format
    }

    pub(super) fn width(&self) -> u32 {
        self.config.width
    }

    pub(super) fn height(&self) -> u32 {
        self.config.height
    }

    pub(super) fn alpha_mode(&self) -> wgpu::CompositeAlphaMode {
        self.config.alpha_mode
    }

    pub(super) fn maximum_dimension(&self) -> u32 {
        self.maximum_dimension
    }

    pub(super) fn clear_color(&self) -> wgpu::Color {
        self.clear_color
    }

    pub(super) fn is_configured(&self) -> bool {
        self.configured
    }

    pub(super) fn set_configured(&mut self, configured: bool) {
        self.configured = configured;
    }

    pub(super) fn request_redraw(&mut self) {
        self.needs_redraw = true;
    }

    pub(super) fn take_redraw_request(&mut self) -> bool {
        std::mem::take(&mut self.needs_redraw)
    }
}

fn clamped_size(size: Size<DevicePixels>, maximum: u32) -> (u32, u32) {
    let requested_width = size.width.0.max(1) as u32;
    let requested_height = size.height.0.max(1) as u32;
    let width = requested_width.min(maximum);
    let height = requested_height.min(maximum);
    if (width, height) != (requested_width, requested_height) {
        log::warn!(
            "requested target size ({requested_width}, {requested_height}) exceeds maximum texture dimension {maximum}; clamping to ({width}, {height})"
        );
    }
    (width, height)
}

fn clear_color(transparent: bool) -> wgpu::Color {
    if transparent {
        return wgpu::Color::TRANSPARENT;
    }
    #[cfg(target_os = "windows")]
    return wgpu::Color::WHITE;
    #[cfg(target_os = "macos")]
    return wgpu::Color::BLACK;
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    wgpu::Color::TRANSPARENT
}
