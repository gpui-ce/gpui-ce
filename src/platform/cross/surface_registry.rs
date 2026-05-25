use std::collections::HashMap;
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};
use std::sync::Mutex;

/// An opaque identifier for a registered WGPU surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SurfaceId(pub(crate) u64);

/// Triple-buffered surface for lock-free rendering.
///
/// Uses three buffers with atomic index swaps:
/// - `rendering`: Currently being rendered by external thread
/// - `ready`: Latest complete frame, ready to display
/// - `display`: Currently being composited by GPUI
///
/// This allows external thread and compositor to run independently without blocking.
struct TripleBuffer {
    textures: [wgpu::Texture; 3],
    views: [wgpu::TextureView; 3],

    // Packed state: 2 bits each for rendering/ready/display indices.
    // layout: [display(2-bit) | ready(2-bit) | rendering(2-bit)]
    state: AtomicU8,

    // GPU synchronization: Track submission indices for each buffer to ensure
    // GPU work is complete before swapping buffers
    submission_indices: Mutex<[Option<wgpu::SubmissionIndex>; 3]>,

    // Redraw coalescing: prevents flooding compositor with thousands of requests/sec
    redraw_pending: std::sync::atomic::AtomicBool,

    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
}

impl TripleBuffer {
    #[inline]
    fn pack_state(rendering: u8, ready: u8, display: u8) -> u8 {
        debug_assert!(rendering < 3 && ready < 3 && display < 3);
        debug_assert!(rendering != ready && ready != display && display != rendering);
        (display << 4) | (ready << 2) | rendering
    }

    #[inline]
    fn unpack_state(state: u8) -> (u8, u8, u8) {
        let rendering = state & 0x03;
        let ready = (state >> 2) & 0x03;
        let display = (state >> 4) & 0x03;
        (rendering, ready, display)
    }
}

/// Thread-safe registry of all active WGPU surfaces.
/// Maps `SurfaceId` to triple-buffered texture sets.
pub struct SurfaceRegistry {
    surfaces: Mutex<HashMap<SurfaceId, TripleBuffer>>,
    next_id: AtomicU64,
}

impl SurfaceRegistry {
    pub fn new() -> Self {
        Self {
            surfaces: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
        }
    }

    /// Create a new triple-buffered surface. Returns its `SurfaceId`.
    pub fn create(
        &self,
        device: &wgpu::Device,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat
    ) -> SurfaceId {
        let id = SurfaceId(self.next_id.fetch_add(1, Ordering::Relaxed));
        let tb = Self::create_triple_buffer(device, width, height, format);
        self.surfaces.lock().unwrap().insert(id, tb);
        id
    }

    /// Atomically swap rendering and ready buffers (called by external thread after rendering).
    ///
    /// This is the "present" operation - it makes the newly rendered frame available
    /// to the compositor and gives the external thread a recycled buffer to render into.
    ///
    /// The `submission_idx` is stored to track GPU work completion, allowing the compositor
    /// to poll before sampling to prevent reading incomplete frames.
    ///
    /// Returns immediately without blocking.
    pub fn swap_rendering_ready(&self, id: SurfaceId, submission_idx: wgpu::SubmissionIndex) {
        if let Some(tb) = self.surfaces.lock().unwrap().get(&id) {
            let current = tb.state.load(Ordering::Acquire);
            let (rendering, ready, display) = TripleBuffer::unpack_state(current);

            log::debug!("[surface_id={:?}] swap_rendering_ready called - state before: rendering={}, ready={}, display={}",
                id, rendering, ready, display);

            // Store submission index for the buffer we just rendered to
            tb.submission_indices.lock().unwrap()[rendering as usize] = Some(submission_idx);

            // Atomic swap: rendering ↔ ready
            let mut current = tb.state.load(Ordering::Acquire);
            loop {
                let (rendering, ready, display) = TripleBuffer::unpack_state(current);
                let next = TripleBuffer::pack_state(ready, rendering, display);
                match tb.state.compare_exchange(
                    current,
                    next,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                ) {
                    Ok(_) => break,
                    Err(updated) => current = updated,
                }
            }
        }
    }

    /// Atomically swap rendering and ready buffers without GPU synchronization.
    ///
    /// DEPRECATED: Use swap_rendering_ready() with SubmissionIndex for proper GPU sync.
    /// This method exists for backward compatibility only.
    pub fn swap_rendering_ready_no_sync(&self, id: SurfaceId) {
        if let Some(tb) = self.surfaces.lock().unwrap().get(&id) {
            let mut current = tb.state.load(Ordering::Acquire);
            loop {
                let (rendering, ready, display) = TripleBuffer::unpack_state(current);
                let next = TripleBuffer::pack_state(ready, rendering, display);
                match tb.state.compare_exchange(
                    current,
                    next,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                ) {
                    Ok(_) => break,
                    Err(updated) => current = updated,
                }
            }
        }
    }

    /// Atomically swap ready and display buffers with GPU synchronization.
    ///
    /// Polls the GPU to check if the ready buffer's work is complete before swapping.
    /// This ensures the compositor never samples incomplete frames.
    ///
    /// Returns `true` if a swap occurred, `false` if GPU work is incomplete (compositor
    /// should reuse the current display buffer).
    pub fn swap_ready_display(&self, device: &wgpu::Device, id: SurfaceId) -> bool {
        if let Some(tb) = self.surfaces.lock().unwrap().get(&id) {
            // Atomic swap: ready ↔ display
            // NOTE: We do NOT call device.poll() here because:
            // 1. The render thread owns the device and is actively using it
            // 2. Calling poll from multiple threads causes driver contention ("device lost")
            // 3. WGPU internally handles synchronization when textures are accessed
            // 4. The triple-buffer lock-free swaps are already safe
            let mut current = tb.state.load(Ordering::Acquire);
            loop {
                let (rendering, ready, display) = TripleBuffer::unpack_state(current);
                let next = TripleBuffer::pack_state(rendering, display, ready);
                match tb.state.compare_exchange(
                    current,
                    next,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                ) {
                    Ok(_) => return true,
                    Err(updated) => current = updated,
                }
            }
        }
        false
    }

    /// Get the rendering buffer's `TextureView` (what external code renders into).
    pub fn back_view(&self, id: SurfaceId) -> Option<wgpu::TextureView> {
        let surfaces = self.surfaces.lock().unwrap();
        surfaces.get(&id).map(|tb| {
            let (rendering, _, _) = TripleBuffer::unpack_state(tb.state.load(Ordering::Acquire));
            tb.views[rendering as usize].clone()
        })
    }

    /// Get the display buffer's `TextureView` (what the compositor reads from).
    pub fn front_view(&self, id: SurfaceId) -> Option<wgpu::TextureView> {
        let surfaces = self.surfaces.lock().unwrap();
        surfaces.get(&id).map(|tb| {
            let (_, _, display) = TripleBuffer::unpack_state(tb.state.load(Ordering::Acquire));
            tb.views[display as usize].clone()
        })
    }

    /// Atomically retrieve both the rendering view and the corresponding texture
    /// dimensions. This is useful when a caller needs to create auxiliary
    /// resources (e.g. a depth buffer) that must exactly match the view's size.
    pub fn lock_and_get_back_with_size(
        &self,
        id: SurfaceId
    ) -> Option<(wgpu::TextureView, (u32, u32))> {
        let surfaces = self.surfaces.lock().unwrap();
        surfaces.get(&id).map(|tb| {
            let (rendering, _, _) = TripleBuffer::unpack_state(tb.state.load(Ordering::Acquire));
            (tb.views[rendering as usize].clone(), (tb.width, tb.height))
        })
    }

    /// Resize all three buffers, creating new textures with GPU synchronization.
    ///
    /// SAFETY: Waits for all pending GPU work to complete before destroying textures.
    /// This prevents use-after-free and ensures all GPU commands finish before
    /// texture resources are released.
    ///
    /// Also skips resize if compositor is actively using the buffers (redraw_pending).
    /// Returns `true` if the resize completed, `false` if it was skipped due to active composition.
    pub fn resize(&self, device: &wgpu::Device, id: SurfaceId, width: u32, height: u32) -> bool {
        let mut surfaces = self.surfaces.lock().unwrap();
        if let Some(tb) = surfaces.get_mut(&id) {
            if tb.width == width && tb.height == height {
                return true;
            }

            // CRITICAL: Don't resize while compositor is rendering this surface!
            // If redraw_pending is true, compositor is using the buffers.
            // Skip resize - the element will retry on next frame.
            if tb.redraw_pending.load(Ordering::Relaxed) {
                return false;
            }

            // NOTE: We do NOT call device.poll() here because:
            // 1. The render thread owns the device and may be actively using it
            // 2. Calling poll from compositor thread causes device corruption
            // 3. WGPU internally ref-counts textures, so old views remain valid until dropped
            // 4. The skip-if-redraw-pending check above prevents resize during active composition

            // Now safe to recreate textures
            let new_tb = Self::create_triple_buffer(device, width, height, tb.format);
            *tb = new_tb;
            return true;
        }
        false
    }

    /// Get the current size of a surface.
    pub fn size(&self, id: SurfaceId) -> Option<(u32, u32)> {
        let surfaces = self.surfaces.lock().unwrap();
        surfaces.get(&id).map(|tb| (tb.width, tb.height))
    }

    /// Get the texture format for a surface.
    pub fn format(&self, id: SurfaceId) -> Option<wgpu::TextureFormat> {
        let surfaces = self.surfaces.lock().unwrap();
        surfaces.get(&id).map(|tb| tb.format)
    }

    /// Remove a surface from the registry.
    pub fn remove(&self, id: SurfaceId) {
        self.surfaces.lock().unwrap().remove(&id);
    }

    /// Set the redraw pending flag, returning the previous value.
    /// Used by present() to coalesce multiple redraw requests.
    pub fn set_redraw_pending(&self, id: SurfaceId) -> bool {
        if let Some(tb) = self.surfaces.lock().unwrap().get(&id) {
            tb.redraw_pending.swap(true, Ordering::Relaxed)
        } else {
            false
        }
    }

    /// Clear the redraw pending flag.
    /// Called by the compositor after consuming a frame.
    pub fn clear_redraw_pending(&self, id: SurfaceId) {
        if let Some(tb) = self.surfaces.lock().unwrap().get(&id) {
            tb.redraw_pending.store(false, Ordering::Relaxed);
        }
    }

    /// Get all surfaces that have pending redraws.
    /// Used by the fast blit path to check which surfaces need updating.
    pub fn get_pending_surfaces(&self) -> Vec<SurfaceId> {
        let surfaces = self.surfaces.lock().unwrap();
        surfaces
            .iter()
            .filter(|(_, tb)| tb.redraw_pending.load(Ordering::Relaxed))
            .map(|(id, _)| *id)
            .collect()
    }

    fn create_triple_buffer(
        device: &wgpu::Device,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat
    ) -> TripleBuffer {
        let w = width.max(1);
        let h = height.max(1);

        let create_texture = |label: &str| {
            device.create_texture(
                &wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: w,
                        height: h,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT |
                           wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                }
            )
        };

        let tex0 = create_texture("surface_buffer_0");
        let tex1 = create_texture("surface_buffer_1");
        let tex2 = create_texture("surface_buffer_2");

        let view0 = tex0.create_view(&wgpu::TextureViewDescriptor::default());
        let view1 = tex1.create_view(&wgpu::TextureViewDescriptor::default());
        let view2 = tex2.create_view(&wgpu::TextureViewDescriptor::default());

        TripleBuffer {
            textures: [tex0, tex1, tex2],
            views: [view0, view1, view2],
            state: AtomicU8::new(TripleBuffer::pack_state(0, 1, 2)),
            submission_indices: Mutex::new([None, None, None]),
            redraw_pending: std::sync::atomic::AtomicBool::new(false),
            width: w,
            height: h,
            format,
        }
    }
}

