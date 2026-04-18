use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};

use refineable::Refineable as _;

use crate::{
    App, Bounds, Element, ElementId, GlobalElementId, InspectorElementId, IntoElement, LayoutId,
    MouseButton, Pixels, Style, StyleRefinement, Styled, Window,
    platform::cross::surface_registry::{SurfaceId, SurfaceRegistry},
};

/// Inner state shared across clones of `WgpuSurfaceHandle`.
/// When the last clone is dropped, the surface is removed from the registry.
struct WgpuSurfaceHandleInner {
    surface_id: SurfaceId,
    registry: Arc<SurfaceRegistry>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    present_trigger: Arc<dyn Fn() + Send + Sync>,
    /// Optional direct handle to the winit window.  Having an `Arc` lets
    /// us call `request_redraw()` from another thread without touching the
    /// event bus.
    winit_window: Option<Arc<winit::window::Window>>,
    size: Mutex<(u32, u32)>,
    pending_resize: Mutex<Option<(u32, u32)>>,
    deferred_resize: Mutex<Option<(u32, u32)>>,
    is_resizing: AtomicBool,
    format: wgpu::TextureFormat,
}

impl Drop for WgpuSurfaceHandleInner {
    fn drop(&mut self) {
        self.registry.remove(self.surface_id);
    }
}

/// A handle to a triple-buffered WGPU surface that perfectly emulates a Winit window.
///
/// External render threads use this to render continuously at their own pace:
/// 1. Get the back buffer with [`back_buffer_view()`](Self::back_buffer_view)
/// 2. Render to it
/// 3. Call [`present()`](Self::present) to swap buffers (non-blocking)
/// 4. Repeat immediately - no waiting required
///
/// The GPUI compositor runs independently, sampling the latest frame whenever it renders.
/// Frame drops and repeats are handled gracefully.
///
/// All rendering stays on the GPU — buffer swaps are atomic pointer swaps (no copy),
/// and the renderer samples textures directly in the shader.
///
/// This handle is `Clone + Send + Sync`.
#[derive(Clone)]
pub struct WgpuSurfaceHandle {
    inner: Arc<WgpuSurfaceHandleInner>,
}

impl WgpuSurfaceHandle {
    pub(crate) fn new(
        device: wgpu::Device,
        queue: wgpu::Queue,
        surface_id: SurfaceId,
        registry: Arc<SurfaceRegistry>,
        present_trigger: Arc<dyn Fn() + Send + Sync>,
        winit_window: Option<Arc<winit::window::Window>>,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
    ) -> Self {
        Self {
            inner: Arc::new(WgpuSurfaceHandleInner {
                surface_id,
                registry,
                device,
                queue,
                present_trigger,
                winit_window,
                size: Mutex::new((width, height)),
                pending_resize: Mutex::new(None),
                deferred_resize: Mutex::new(None),
                is_resizing: AtomicBool::new(false),
                format,
            }),
        }
    }

    /// The wgpu `Device` for creating GPU resources and command encoders.
    pub fn device(&self) -> &wgpu::Device {
        &self.inner.device
    }

    /// The wgpu `Queue` for submitting command buffers.
    pub fn queue(&self) -> &wgpu::Queue {
        &self.inner.queue
    }

    /// Get a `TextureView` of the back buffer for use as a render target.
    /// Render into this, then call [`present()`](Self::present).
    pub fn back_buffer_view(&self) -> Option<wgpu::TextureView> {
        self.inner.registry.back_view(self.inner.surface_id)
    }

    /// Atomically obtain the back buffer view _and_ its pixel dimensions.
    /// This avoids races where the surface is resized between separate calls
    /// to `back_buffer_view()` and `.size()`.
    pub fn back_view_with_size(&self) -> Option<(wgpu::TextureView, (u32, u32))> {
        self.inner
            .registry
            .lock_and_get_back_with_size(self.inner.surface_id)
    }

    /// Present the rendered frame with GPU synchronization (recommended).
    ///
    /// This atomically swaps the rendering and ready buffers, making your newly
    /// rendered frame available to the compositor, and triggers a window redraw request.
    ///
    /// The `submission_index` parameter (returned by `queue.submit()`) allows the
    /// compositor to poll the GPU and ensure rendering is complete before sampling
    /// the texture. This prevents visual artifacts from reading incomplete frames.
    ///
    /// **Returns immediately** - external threads can continue rendering the next
    /// frame without waiting for the compositor. This is the key difference from
    /// traditional blocking present models.
    ///
    /// # Example
    /// ```no_run
    /// // Render to the back buffer
    /// let view = surface.back_buffer_view()?;
    /// // ... encode commands ...
    /// let submission_idx = queue.submit([encoder.finish()]);
    /// drop(view);
    ///
    /// // Present with GPU sync
    /// surface.present_synced(submission_idx);
    /// ```
    pub fn present_synced(&self, submission_index: wgpu::SubmissionIndex) {
        // Atomically swap rendering ↔ ready buffers with GPU sync
        self.inner
            .registry
            .swap_rendering_ready(self.inner.surface_id, submission_index);

        // Track that this surface has new content to be composited
        self.inner
            .registry
            .set_redraw_pending(self.inner.surface_id);

        if let Some(winit) = &self.inner.winit_window {
            winit.request_redraw();
        } else {
            (self.inner.present_trigger)();
        }

        // Return immediately - no blocking
    }

    /// Present the rendered frame without GPU synchronization (deprecated).
    ///
    /// **DEPRECATED**: Use [`present_synced()`](Self::present_synced) instead for proper
    /// GPU synchronization. This method may cause visual artifacts if the compositor
    /// samples the texture before GPU rendering is complete.
    ///
    /// This method exists for backward compatibility only.
    #[deprecated(note = "Use present_synced() for proper GPU synchronization")]
    pub fn present(&self) {
        // Atomically swap rendering ↔ ready buffers (no GPU sync)
        self.inner
            .registry
            .swap_rendering_ready_no_sync(self.inner.surface_id);

        // Track that this surface has new content to be composited
        self.inner
            .registry
            .set_redraw_pending(self.inner.surface_id);

        if let Some(winit) = &self.inner.winit_window {
            winit.request_redraw();
        } else {
            (self.inner.present_trigger)();
        }

        // Return immediately - no blocking
    }

    /// Silently swap the rendered buffer to the ready slot without triggering any
    /// redraw request or setting `redraw_pending`.
    ///
    /// Use this when the GPUI draw cycle (via `window.request_animation_frame()`) drives
    /// the compositor instead of the fast-blit path.  The compositor will pick up the
    /// latest ready buffer the next time it paints the `WgpuSurface` element.
    pub fn swap_buffers(&self) {
        self.inner
            .registry
            .swap_rendering_ready_no_sync(self.inner.surface_id);
    }

    /// Current size in device pixels.
    pub fn size(&self) -> (u32, u32) {
        *self.inner.size.lock().unwrap()
    }

    /// The texture format used by this surface's buffers.
    pub fn format(&self) -> wgpu::TextureFormat {
        self.inner.format
    }

    /// Returns true if a resize is pending, deferred, or currently in progress.
    pub fn is_resize_pending(&self) -> bool {
        if self.inner.is_resizing.load(Ordering::Acquire) {
            return true;
        }
        if self.inner.pending_resize.lock().unwrap().is_some() {
            return true;
        }
        self.inner.deferred_resize.lock().unwrap().is_some()
    }

    /// Defer a resize until a later time, without starting texture reallocation yet.
    pub fn defer_resize(&self, width: u32, height: u32) {
        let current_size = self.size();
        if current_size == (width, height) {
            return;
        }
        let mut deferred = self.inner.deferred_resize.lock().unwrap();
        if deferred.map_or(false, |pending_size| pending_size == (width, height)) {
            return;
        }
        *deferred = Some((width, height));
    }

    /// Take any deferred resize request, returning the target size.
    pub fn take_deferred_resize(&self) -> Option<(u32, u32)> {
        self.inner.deferred_resize.lock().unwrap().take()
    }

    /// The `SurfaceId` for this handle (used internally by the element).
    pub(crate) fn id(&self) -> SurfaceId {
        self.inner.surface_id
    }

    /// Resize the surface's triple buffers. Called by the element when bounds change.
    pub(crate) fn resize(&self, width: u32, height: u32) {
        let mut size = self.inner.size.lock().unwrap();
        if size.0 == width && size.1 == height {
            return;
        }
        self.inner
            .registry
            .resize(&self.inner.device, self.inner.surface_id, width, height);
        *size = (width, height);
    }

    /// Schedule the surface resize in a background thread.
    ///
    /// The current surface contents remain available while the new textures are
    /// allocated, preventing UI stalls during panel resize.
    pub fn request_resize(&self, width: u32, height: u32) {
        let current_size = self.size();
        if current_size == (width, height) {
            return;
        }

        {
            let mut pending = self.inner.pending_resize.lock().unwrap();
            if pending.map_or(false, |pending_size| pending_size == (width, height)) {
                return;
            }
            *pending = Some((width, height));
        }

        if self
            .inner
            .is_resizing
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            let inner = self.inner.clone();
            std::thread::spawn(move || {
                let registry = inner.registry.clone();
                let device = inner.device.clone();
                let surface_id = inner.surface_id;

                loop {
                    let target = {
                        let mut pending = inner.pending_resize.lock().unwrap();
                        pending.take()
                    };

                    let (width, height) = match target {
                        Some(size) => size,
                        None => {
                            inner.is_resizing.store(false, Ordering::Release);
                            if inner.pending_resize.lock().unwrap().is_some() {
                                // A new resize request arrived after we last checked.
                                if inner.is_resizing.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire).is_ok() {
                                    continue;
                                }
                            }
                            break;
                        }
                    };

                    while !registry.resize(&device, surface_id, width, height) {
                        std::thread::sleep(std::time::Duration::from_millis(8));
                    }

                    let mut size = inner.size.lock().unwrap();
                    *size = (width, height);
                }
            });
        }
    }
}

/// Create a `WgpuSurface` element from an existing handle.
pub fn wgpu_surface(handle: WgpuSurfaceHandle) -> WgpuSurface {
    WgpuSurface {
        handle,
        style: StyleRefinement::default(),
        on_resize: None,
        defer_resize_until_mouse_up: false,
        was_window_resizing: false,
    }
}

/// An element that displays content rendered externally via WGPU.
///
/// Acts as a drop-in replacement for a Winit window - external render threads
/// can render continuously at their own pace while GPUI composites around them.
///
/// On the WGPU platform, the renderer composites the surface's display buffer
/// texture directly (GPU → GPU, no copies). On other platforms this renders
/// as a fallback colored box.
pub struct WgpuSurface {
    handle: WgpuSurfaceHandle,
    style: StyleRefinement,
    on_resize: Option<Box<dyn Fn(u32, u32, &WgpuSurfaceHandle) + 'static>>,
    defer_resize_until_mouse_up: bool,
    /// Tracks previous frame's window_resizing state to detect OS resize completion.
    was_window_resizing: bool,
}

impl WgpuSurface {
    /// Register a callback invoked when the element's layout bounds change.
    /// The surface textures are automatically resized; use this to recreate
    /// any external resources that depend on the size.
    pub fn on_resize(
        mut self,
        callback: impl Fn(u32, u32, &WgpuSurfaceHandle) + 'static,
    ) -> Self {
        self.on_resize = Some(Box::new(callback));
        self
    }

    /// Enable deferred resize until left mouse release.
    pub fn defer_resize_until_mouse_up(mut self, enabled: bool) -> Self {
        self.defer_resize_until_mouse_up = enabled;
        self
    }
}

impl Element for WgpuSurface {
    type RequestLayoutState = Style;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.refine(&self.style);
        let layout_id = window.request_layout(style.clone(), [], cx);
        (layout_id, style)
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        _cx: &mut App,
    ) -> Self::PrepaintState {
        // Compute pixel size accounting for scale factor
        let scale = window.scale_factor();
        let pixel_w = (bounds.size.width.0 * scale).round() as u32;
        let pixel_h = (bounds.size.height.0 * scale).round() as u32;

        let (cur_w, cur_h) = self.handle.size();
        let left_pressed = window.pressed_mouse_button() == Some(MouseButton::Left);
        let window_resizing = window.is_window_resizing();
        // True on the first frame after OS window resize ends — force-flush even if
        // the mouse button state didn't update (macOS may not deliver mouse-up for
        // resize-handle drags that the OS owns).
        let os_resize_just_ended = self.was_window_resizing && !window_resizing;
        self.was_window_resizing = window_resizing;

        if pixel_w != cur_w || pixel_h != cur_h {
            if self.defer_resize_until_mouse_up && (left_pressed || window_resizing) {
                self.handle.defer_resize(pixel_w, pixel_h);
            } else {
                self.handle.request_resize(pixel_w, pixel_h);
                if let Some(cb) = &self.on_resize {
                    cb(pixel_w, pixel_h, &self.handle);
                }
            }
        }

        // Flush deferred resize when:
        //   - internal panel drag: mouse released (!left_pressed) and no OS resize
        //   - OS window resize: size just stabilized (os_resize_just_ended), overrides mouse state
        if self.defer_resize_until_mouse_up && (os_resize_just_ended || (!left_pressed && !window_resizing)) {
            if let Some((pending_w, pending_h)) = self.handle.take_deferred_resize() {
                if (pending_w, pending_h) != (cur_w, cur_h) {
                    self.handle.request_resize(pending_w, pending_h);
                    if let Some(cb) = &self.on_resize {
                        cb(pending_w, pending_h, &self.handle);
                    }
                }
            }
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        style: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        style.paint(bounds, window, cx, |window, _cx| {
            window.paint_wgpu_surface(bounds, self.handle.id());
        });
    }
}

impl IntoElement for WgpuSurface {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Styled for WgpuSurface {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}
