use std::sync::{Arc, Mutex};

use super::surface_registry::SurfaceRegistry;

pub struct WgpuContext {
    pub(super) adapter: wgpu::Adapter,
    pub(super) device: wgpu::Device,
    pub(super) queue: wgpu::Queue,
    pub(super) instance: wgpu::Instance,

    pub(super) globals_buffer: wgpu::Buffer,
    pub(super) quads_buffer: Mutex<wgpu::Buffer>,
    pub(super) shadows_buffer: Mutex<wgpu::Buffer>,
    pub(super) underlines_buffer: Mutex<wgpu::Buffer>,
    pub(super) mono_sprites_buffer: Mutex<wgpu::Buffer>,
    pub(super) poly_sprites_buffer: Mutex<wgpu::Buffer>,
    pub(super) color_adjustments_buffer: wgpu::Buffer,
        pub(super) paths_vertices_buffer: Mutex<wgpu::Buffer>,

    pub(crate) surface_registry: Arc<SurfaceRegistry>,
}

impl WgpuContext {
    pub fn new() -> anyhow::Result<Self> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });

        let required_features = wgpu::Features::TIMESTAMP_QUERY
            | wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS
            | wgpu::Features::TEXTURE_BINDING_ARRAY
            | wgpu::Features::SAMPLED_TEXTURE_AND_STORAGE_BUFFER_ARRAY_NON_UNIFORM_INDEXING
            | wgpu::Features::SHADER_PRIMITIVE_INDEX;

        let adapters = pollster::block_on(instance.enumerate_adapters(wgpu::Backends::all()));

        // On macOS, MULTI_DRAW_INDIRECT_COUNT is optional — prefer adapters that expose it
        // but do not require it, since Metal may not advertise it on all hardware.
        // On all other platforms, require it outright for full indirect draw support.
        #[cfg(target_os = "macos")]
        let (adapter, device_features) = {
            let optional_features = wgpu::Features::MULTI_DRAW_INDIRECT_COUNT;
            let adapter = adapters
                .into_iter()
                .filter(|adapter| adapter.features().contains(required_features))
                .max_by_key(|adapter| adapter.features().contains(optional_features))
                .ok_or_else(|| anyhow::anyhow!(
                    "No adapter available with required features: {:?}",
                    required_features
                ))?;
            let device_features = if adapter.features().contains(optional_features) {
                required_features | optional_features
            } else {
                required_features
            };
            (adapter, device_features)
        };

        #[cfg(not(target_os = "macos"))]
        let (adapter, device_features) = {
            let required_features = required_features | wgpu::Features::MULTI_DRAW_INDIRECT_COUNT;
            let adapter = adapters
                .into_iter()
                .find(|adapter| adapter.features().contains(required_features))
                .ok_or_else(|| anyhow::anyhow!(
                    "No adapter available with required features: {:?}",
                    required_features
                ))?;
            (adapter, required_features)
        };

        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: None,
            required_features: device_features,
            required_limits: wgpu::Limits {
                max_binding_array_elements_per_shader_stage: 512,
                ..adapter.limits()
            },
            ..Default::default()
        }))?;

        let globals_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Globals Buffer"),
            // FIXME(mdeand): Hack
            size: 16 as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let quads_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Quads Buffer"),
            // TODO(mdeand): Determine appropriate size
            size: 1024 * 1024, // 1 MB buffer for quads, for now. (:
            usage: wgpu::BufferUsages::VERTEX
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });

        let mono_sprites_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Monosprites Buffer"),
            // TODO(mdeand): Determine appropriate size, or make resizable.
            size: 1024 * 1024,
            usage: wgpu::BufferUsages::VERTEX
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });

        let shadows_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Shadows Buffer"),
            size: 1024 * 1024,
            usage: wgpu::BufferUsages::VERTEX
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });

        let underlines_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Underlines Buffer"),
            size: 1024 * 1024,
            usage: wgpu::BufferUsages::VERTEX
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });

        let poly_sprites_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Poly Sprites Buffer"),
            size: 1024 * 1024,
            usage: wgpu::BufferUsages::VERTEX
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        
            let paths_vertices_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Path Vertices Buffer"),
                size: 4 * 1024 * 1024, // 4 MB – ~87 k vertices @ 48 bytes each
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });

        let color_adjustments_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Color Adjustments Buffer"),
            size: 1024 * 16, // TODO(mdeand): 16 KB buffer for color adjustments, for now. (:
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::UNIFORM,
            mapped_at_creation: false,
        });

        Ok(Self {
            adapter,
            device,
            queue,
            instance,

            globals_buffer,
            quads_buffer: Mutex::new(quads_buffer),
            shadows_buffer: Mutex::new(shadows_buffer),
            underlines_buffer: Mutex::new(underlines_buffer),
            mono_sprites_buffer: Mutex::new(mono_sprites_buffer),
            poly_sprites_buffer: Mutex::new(poly_sprites_buffer),
            color_adjustments_buffer,

                paths_vertices_buffer: Mutex::new(paths_vertices_buffer),
            surface_registry: Arc::new(SurfaceRegistry::new()),
        })
    }

}

/// Ensures a buffer is large enough to hold the required size.
/// If the buffer is too small, it will be recreated with the new size.
pub(super) fn ensure_buffer_size(
    device: &wgpu::Device,
    buffer: &RefCell<wgpu::Buffer>,
    required_size: u64,
    label: &str,
    usage: wgpu::BufferUsages,
) {
    let current_size = buffer.borrow().size();
    if current_size < required_size {
        // Recreate buffer with new size (add some headroom to avoid frequent reallocations)
        let new_size = (required_size * 3 / 2).max(required_size + 1024 * 1024);
        *buffer.borrow_mut() = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: new_size,
            usage,
            mapped_at_creation: false,
        });
    }
}
