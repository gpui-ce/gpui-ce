use crate::{
    App, Bounds, Element, ElementId, GlobalElementId, InspectorElementId, IntoElement, LayoutId,
    ObjectFit, Pixels, Style, StyleRefinement, Styled, Window,
};
use refineable::Refineable;
use std::sync::Arc;

/// Universal GPU texture handle for zero-copy rendering.
///
/// At the fundamental level, all GPU textures are just RGBA8 bytes in memory.
/// This handle is a platform-agnostic reference to that memory - whether it's:
/// - Windows: DirectX shared resource handle (NT handle)
/// - macOS: Metal IOSurface handle
/// - Linux: Vulkan external memory handle (dma-buf FD)
///
/// The key insight: these are all different OS-level ways to reference the
/// SAME underlying GPU memory with the SAME RGBA8 byte format.
#[derive(Clone, Debug)]
pub struct GpuTextureHandle {
    /// Platform-native handle to the shared GPU texture memory
    /// - Windows: NT handle (isize)
    /// - macOS: IOSurface ID (isize)
    /// - Linux: dma-buf file descriptor (isize)
    pub native_handle: isize,

    /// Width of the texture in pixels
    pub width: u32,

    /// Height of the texture in pixels
    pub height: u32,

    /// Texture format (typically RGBA8, universal across all platforms)
    pub format: GpuTextureFormat,
}

/// GPU texture format - universal across all platforms
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GpuTextureFormat {
    /// 8-bit RGBA (4 bytes per pixel) - most common
    RGBA8,
    /// 8-bit BGRA (4 bytes per pixel) - some platforms prefer this
    BGRA8,
    /// 16-bit float RGBA (8 bytes per pixel) - HDR
    RGBA16F,
}

impl GpuTextureHandle {
    /// Create a new GPU texture handle with RGBA8 format (default)
    pub fn new(native_handle: isize, width: u32, height: u32) -> Self {
        Self {
            native_handle,
            width,
            height,
            format: GpuTextureFormat::RGBA8,
        }
    }

    /// Create a new GPU texture handle with a specific format
    pub fn new_with_format(
        native_handle: isize,
        width: u32,
        height: u32,
        format: GpuTextureFormat,
    ) -> Self {
        Self {
            native_handle,
            width,
            height,
            format,
        }
    }

    /// Get the size in bytes of a single pixel for this format
    pub fn bytes_per_pixel(&self) -> u32 {
        match self.format {
            GpuTextureFormat::RGBA8 => 4,
            GpuTextureFormat::BGRA8 => 4,
            GpuTextureFormat::RGBA16F => 8,
        }
    }

    /// Get the total size in bytes of the texture
    pub fn size_in_bytes(&self) -> usize {
        (self.width * self.height * self.bytes_per_pixel()) as usize
    }
}

unsafe impl Send for GpuTextureHandle {}
unsafe impl Sync for GpuTextureHandle {}

/// Double-buffered GPU texture source for flicker-free rendering.
/// One buffer is written by the producer while the other is read by GPUI.
#[derive(Clone)]
pub struct GpuCanvasSource {
    /// Current active buffer index (0 or 1)
    active_buffer: Arc<std::sync::atomic::AtomicUsize>,
    /// The two shared GPU texture handles
    buffers: [GpuTextureHandle; 2],
}

impl GpuCanvasSource {
    /// Create a new double-buffered GPU canvas source.
    pub fn new(buffer0: GpuTextureHandle, buffer1: GpuTextureHandle) -> Self {
        Self {
            active_buffer: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            buffers: [buffer0, buffer1],
        }
    }

    /// Get the currently active buffer for reading.
    pub fn active_buffer(&self) -> &GpuTextureHandle {
        let index = self
            .active_buffer
            .load(std::sync::atomic::Ordering::Acquire);
        &self.buffers[index % 2]
    }

    /// Swap to the other buffer (call this from the producer thread after rendering).
    pub fn swap_buffers(&self) {
        self.active_buffer
            .fetch_xor(1, std::sync::atomic::Ordering::Release);
    }

    /// Set the active buffer index directly (0 or 1).
    pub fn set_active_buffer(&self, index: usize) {
        self.active_buffer
            .store(index % 2, std::sync::atomic::Ordering::Release);
    }
}

/// A GPU canvas element for zero-copy rendering of external GPU content.
///
/// This element displays GPU textures shared from another rendering context
/// (e.g., DX12, Vulkan, Metal) without any CPU copies. It uses double-buffering
/// to avoid tearing and allows the producer to render independently.
///
/// # Example
/// ```ignore
/// gpu_canvas(source.clone())
///     .object_fit(ObjectFit::Cover)
///     .w_full()
///     .h_full()
/// ```
pub struct GpuCanvas {
    source: GpuCanvasSource,
    object_fit: ObjectFit,
    style: StyleRefinement,
}

/// Create a new GPU canvas element with the given texture source.
pub fn gpu_canvas(source: GpuCanvasSource) -> GpuCanvas {
    GpuCanvas {
        source,
        object_fit: ObjectFit::Contain,
        style: Default::default(),
    }
}

impl GpuCanvas {
    /// Set how the GPU texture should fit within the element bounds.
    pub fn object_fit(mut self, object_fit: ObjectFit) -> Self {
        self.object_fit = object_fit;
        self
    }
}

impl Element for GpuCanvas {
    type RequestLayoutState = ();
    type PrepaintState = GpuTextureHandle;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.refine(&self.style);
        let layout_id = window.request_layout(style, [], cx);
        (layout_id, ())
    }

    fn prepaint(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Self::PrepaintState {
        self.source.active_buffer().clone()
    }

    fn paint(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        _cx: &mut App,
    ) {
        window.paint_gpu_texture(bounds, prepaint.clone(), self.object_fit.clone());
    }
}

impl IntoElement for GpuCanvas {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Styled for GpuCanvas {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}
