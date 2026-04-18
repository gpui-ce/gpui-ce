use std::sync::Arc;

use super::surface_registry::SurfaceRegistry;

pub struct WgpuContext {
    pub(super) adapter: wgpu::Adapter,
    pub(super) device: wgpu::Device,
    pub(super) queue: wgpu::Queue,
    pub(super) instance: wgpu::Instance,

    pub(super) globals_buffer: wgpu::Buffer,
    pub(super) quads_buffer: wgpu::Buffer,
    pub(super) shadows_buffer: wgpu::Buffer,
    pub(super) underlines_buffer: wgpu::Buffer,
    pub(super) mono_sprites_buffer: wgpu::Buffer,
    pub(super) poly_sprites_buffer: wgpu::Buffer,
    pub(super) color_adjustments_buffer: wgpu::Buffer,

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
            | wgpu::Features::SHADER_PRIMITIVE_INDEX
            | wgpu::Features::MULTI_DRAW_INDIRECT_COUNT;

        let adapters = pollster::block_on(instance.enumerate_adapters(wgpu::Backends::all()));
        let adapter = adapters
            .into_iter()
            .find(|adapter| adapter.features().contains(required_features))
            .ok_or_else(|| anyhow::anyhow!(
                "No adapter available with required features: {:?}",
                required_features
            ))?;

        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: None,
            required_features,
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
            quads_buffer,
            shadows_buffer,
            underlines_buffer,
            mono_sprites_buffer,
            poly_sprites_buffer,
            color_adjustments_buffer,

            surface_registry: Arc::new(SurfaceRegistry::new()),
        })
    }
}
