use std::sync::{Arc, Mutex};

use blade_graphics::{self as gpu, ShaderBindable as _};
use blade_util::{BufferBelt, BufferBeltDescriptor};

use crate::{
    CustomAddressMode, CustomBindingKind, CustomBindingValue, CustomBufferDesc, CustomBufferId,
    CustomBufferSource, CustomDrawRegistry, CustomFilterMode, CustomPipelineDesc, CustomPipelineId,
    CustomPrimitiveTopology, CustomSamplerDesc, CustomSamplerId, CustomTextureDesc,
    CustomTextureFormat, CustomTextureId, CustomVertexFormat, Result,
};

pub(crate) struct BladeCustomDrawRegistry {
    gpu: Arc<gpu::Context>,
    surface_info: gpu::SurfaceInfo,
    pipelines: Mutex<Vec<Option<BladeCustomPipeline>>>,
    buffers: Mutex<Vec<Option<BladeCustomBuffer>>>,
    textures: Mutex<Vec<Option<BladeCustomTexture>>>,
    samplers: Mutex<Vec<Option<gpu::Sampler>>>,
    upload_belt: Mutex<BufferBelt>,
    pending_uploads: Mutex<Vec<PendingUpload>>,
}

struct BladeCustomPipeline {
    pipeline: gpu::RenderPipeline,
    bindings: Vec<CustomBindingKind>,
}

struct BladeCustomBuffer {
    buffer: gpu::Buffer,
    size: u64,
}

struct BladeCustomTexture {
    texture: gpu::Texture,
    view: gpu::TextureView,
    width: u32,
    height: u32,
    bytes_per_pixel: u32,
}

struct PendingUpload {
    texture_id: CustomTextureId,
    data: gpu::BufferPiece,
}

impl BladeCustomDrawRegistry {
    pub(crate) fn new(gpu: Arc<gpu::Context>, surface_info: gpu::SurfaceInfo) -> Self {
        Self {
            gpu,
            surface_info,
            pipelines: Mutex::new(Vec::new()),
            buffers: Mutex::new(Vec::new()),
            textures: Mutex::new(Vec::new()),
            samplers: Mutex::new(Vec::new()),
            upload_belt: Mutex::new(BufferBelt::new(BufferBeltDescriptor {
                memory: gpu::Memory::Upload,
                min_chunk_size: 0x10000,
                alignment: 64,
            })),
            pending_uploads: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn destroy(&self) {
        let mut pipelines = self.pipelines.lock().unwrap();
        for pipeline in pipelines.iter_mut().flatten() {
            self.gpu.destroy_render_pipeline(&mut pipeline.pipeline);
        }
        pipelines.clear();
        let mut buffers = self.buffers.lock().unwrap();
        for buffer in buffers.iter_mut().flatten() {
            self.gpu.destroy_buffer(buffer.buffer);
        }
        buffers.clear();
        let mut textures = self.textures.lock().unwrap();
        for texture in textures.iter_mut().flatten() {
            self.gpu.destroy_texture_view(texture.view);
            self.gpu.destroy_texture(texture.texture);
        }
        textures.clear();
        let mut samplers = self.samplers.lock().unwrap();
        for sampler in samplers.iter_mut().flatten() {
            self.gpu.destroy_sampler(*sampler);
        }
        samplers.clear();
        self.upload_belt.lock().unwrap().destroy(&self.gpu);
        self.pending_uploads.lock().unwrap().clear();
    }

    pub(crate) fn with_pipeline<F, R>(&self, id: CustomPipelineId, f: F) -> Option<R>
    where
        F: FnOnce(&gpu::RenderPipeline, &[CustomBindingKind]) -> R,
    {
        let pipelines = self.pipelines.lock().unwrap();
        let entry = pipelines.get(id.0 as usize)?.as_ref()?;
        Some(f(&entry.pipeline, &entry.bindings))
    }

    pub(crate) fn get_buffer(&self, id: CustomBufferId) -> Option<gpu::Buffer> {
        let buffers = self.buffers.lock().unwrap();
        buffers
            .get(id.0 as usize)
            .and_then(|slot| slot.as_ref())
            .map(|entry| entry.buffer)
    }

    pub(crate) fn buffers_snapshot(&self) -> Vec<Option<gpu::Buffer>> {
        self.buffers
            .lock()
            .unwrap()
            .iter()
            .map(|slot| slot.as_ref().map(|entry| entry.buffer))
            .collect()
    }

    pub(crate) fn textures_snapshot(&self) -> Vec<Option<gpu::TextureView>> {
        self.textures
            .lock()
            .unwrap()
            .iter()
            .map(|slot| slot.as_ref().map(|texture| texture.view))
            .collect()
    }

    pub(crate) fn samplers_snapshot(&self) -> Vec<Option<gpu::Sampler>> {
        self.samplers.lock().unwrap().clone()
    }

    pub(crate) fn before_frame(&self, encoder: &mut gpu::CommandEncoder) {
        let uploads = {
            let mut pending = self.pending_uploads.lock().unwrap();
            std::mem::take(&mut *pending)
        };
        if uploads.is_empty() {
            return;
        }
        for upload in uploads {
            let textures = self.textures.lock().unwrap();
            let Some(texture) = textures
                .get(upload.texture_id.0 as usize)
                .and_then(|slot| slot.as_ref())
            else {
                continue;
            };
            encoder.init_texture(texture.texture);
            let mut transfers = encoder.transfer("custom_draw_texture");
            transfers.copy_buffer_to_texture(
                upload.data,
                texture.width * texture.bytes_per_pixel,
                gpu::TexturePiece {
                    texture: texture.texture,
                    mip_level: 0,
                    array_layer: 0,
                    origin: [0, 0, 0],
                },
                gpu::Extent {
                    width: texture.width,
                    height: texture.height,
                    depth: 1,
                },
            );
        }
    }

    pub(crate) fn after_frame(&self, sync_point: &gpu::SyncPoint) {
        self.upload_belt.lock().unwrap().flush(sync_point);
    }

    fn alloc_slot<T>(slots: &mut Vec<Option<T>>, value: T) -> u32 {
        if let Some((index, slot)) = slots.iter_mut().enumerate().find(|(_, slot)| slot.is_none()) {
            *slot = Some(value);
            return index as u32;
        }
        slots.push(Some(value));
        (slots.len() - 1) as u32
    }
}

impl CustomDrawRegistry for BladeCustomDrawRegistry {
    fn create_pipeline(&self, desc: CustomPipelineDesc) -> Result<CustomPipelineId> {
        let shader = self.gpu.create_shader(gpu::ShaderDesc {
            source: &desc.shader_source,
        });

        let color_targets = &[gpu::ColorTargetState {
            format: self.surface_info.format,
            blend: Some(match self.surface_info.alpha {
                gpu::AlphaMode::Ignored => gpu::BlendState::ALPHA_BLENDING,
                gpu::AlphaMode::PreMultiplied => gpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING,
                gpu::AlphaMode::PostMultiplied => gpu::BlendState::ALPHA_BLENDING,
            }),
            write_mask: gpu::ColorWrites::default(),
        }];

        let mut layouts: Vec<gpu::VertexLayout> = Vec::new();
        let mut instanced_flags: Vec<bool> = Vec::new();
        for fetch in &desc.vertex_fetches {
            let mut attributes = Vec::with_capacity(fetch.layout.attributes.len());
            for attr in &fetch.layout.attributes {
                let name: &'static str = attr.name.as_str();
                attributes.push((
                    name,
                    gpu::VertexAttribute {
                        offset: attr.offset,
                        format: match attr.format {
                            CustomVertexFormat::F32 => gpu::VertexFormat::F32,
                            CustomVertexFormat::F32Vec2 => gpu::VertexFormat::F32Vec2,
                            CustomVertexFormat::F32Vec3 => gpu::VertexFormat::F32Vec3,
                            CustomVertexFormat::F32Vec4 => gpu::VertexFormat::F32Vec4,
                            CustomVertexFormat::U32 => gpu::VertexFormat::U32,
                            CustomVertexFormat::U32Vec2 => gpu::VertexFormat::U32Vec2,
                            CustomVertexFormat::U32Vec3 => gpu::VertexFormat::U32Vec3,
                            CustomVertexFormat::U32Vec4 => gpu::VertexFormat::U32Vec4,
                            CustomVertexFormat::I32 => gpu::VertexFormat::I32,
                            CustomVertexFormat::I32Vec2 => gpu::VertexFormat::I32Vec2,
                            CustomVertexFormat::I32Vec3 => gpu::VertexFormat::I32Vec3,
                            CustomVertexFormat::I32Vec4 => gpu::VertexFormat::I32Vec4,
                        },
                    },
                ));
            }
            layouts.push(gpu::VertexLayout {
                attributes,
                stride: fetch.layout.stride,
            });
            instanced_flags.push(fetch.instanced);
        }
        let mut fetches: Vec<gpu::VertexFetchState<'_>> = Vec::with_capacity(layouts.len());
        for (layout, instanced) in layouts.iter().zip(instanced_flags.iter()) {
            fetches.push(gpu::VertexFetchState {
                layout,
                instanced: *instanced,
            });
        }

        let mut binding_layout = gpu::ShaderDataLayout::default();
        let mut binding_kinds = Vec::new();
        for binding in &desc.bindings {
            let name = binding.name.as_str();
            let shader_binding = match binding.kind {
                CustomBindingKind::Buffer => gpu::ShaderBinding::Buffer,
                CustomBindingKind::Texture => gpu::ShaderBinding::Texture,
                CustomBindingKind::Sampler => gpu::ShaderBinding::Sampler,
                CustomBindingKind::Uniform { size } => {
                    gpu::ShaderBinding::Plain { size }
                }
            };
            binding_layout.bindings.push((name, shader_binding));
            binding_kinds.push(binding.kind);
        }

        let data_layouts: Vec<&gpu::ShaderDataLayout> = if desc.bindings.is_empty() {
            Vec::new()
        } else {
            vec![&binding_layout]
        };

        let pipeline = self.gpu.create_render_pipeline(gpu::RenderPipelineDesc {
            name: &desc.name,
            data_layouts: data_layouts.as_slice(),
            vertex: shader.at(&desc.vertex_entry),
            vertex_fetches: &fetches,
            primitive: gpu::PrimitiveState {
                topology: match desc.primitive {
                    CustomPrimitiveTopology::PointList => gpu::PrimitiveTopology::PointList,
                    CustomPrimitiveTopology::LineList => gpu::PrimitiveTopology::LineList,
                    CustomPrimitiveTopology::LineStrip => gpu::PrimitiveTopology::LineStrip,
                    CustomPrimitiveTopology::TriangleList => gpu::PrimitiveTopology::TriangleList,
                    CustomPrimitiveTopology::TriangleStrip => gpu::PrimitiveTopology::TriangleStrip,
                },
                ..Default::default()
            },
            depth_stencil: None,
            fragment: Some(shader.at(&desc.fragment_entry)),
            color_targets,
            multisample_state: gpu::MultisampleState::default(),
        });

        let mut pipelines = self.pipelines.lock().unwrap();
        let id = Self::alloc_slot(
            &mut pipelines,
            BladeCustomPipeline {
                pipeline,
                bindings: binding_kinds,
            },
        );
        Ok(CustomPipelineId(id))
    }

    fn create_buffer(&self, desc: CustomBufferDesc) -> Result<CustomBufferId> {
        let size = desc.data.len() as u64;
        let buffer = self.gpu.create_buffer(gpu::BufferDesc {
            name: &desc.name,
            size: size.max(1),
            memory: gpu::Memory::Shared,
        });
        if !desc.data.is_empty() {
            let piece = gpu::BufferPiece::from(buffer);
            unsafe {
                std::ptr::copy_nonoverlapping(desc.data.as_ptr(), piece.data(), desc.data.len());
            }
        }
        let mut buffers = self.buffers.lock().unwrap();
        let id = Self::alloc_slot(
            &mut buffers,
            BladeCustomBuffer {
                buffer,
                size: size.max(1),
            },
        );
        Ok(CustomBufferId(id))
    }

    fn update_buffer(&self, id: CustomBufferId, data: Arc<[u8]>) -> Result<()> {
        let mut buffers = self.buffers.lock().unwrap();
        let Some(slot) = buffers.get_mut(id.0 as usize) else {
            return Err(anyhow::anyhow!("custom buffer id out of range"));
        };
        let Some(entry) = slot.as_mut() else {
            return Err(anyhow::anyhow!("custom buffer id out of range"));
        };

        let new_size = data.len() as u64;
        if new_size > entry.size {
            self.gpu.destroy_buffer(entry.buffer);
            let buffer = self.gpu.create_buffer(gpu::BufferDesc {
                name: "custom_draw_buffer",
                size: new_size.max(1),
                memory: gpu::Memory::Shared,
            });
            *entry = BladeCustomBuffer {
                buffer,
                size: new_size.max(1),
            };
        }

        if !data.is_empty() {
            let piece = gpu::BufferPiece::from(entry.buffer);
            unsafe {
                std::ptr::copy_nonoverlapping(data.as_ptr(), piece.data(), data.len());
            }
        }
        Ok(())
    }

    fn remove_buffer(&self, id: CustomBufferId) {
        let mut buffers = self.buffers.lock().unwrap();
        if let Some(slot) = buffers.get_mut(id.0 as usize) {
            if let Some(entry) = slot.take() {
                self.gpu.destroy_buffer(entry.buffer);
            }
        }
    }

    fn create_texture(&self, desc: CustomTextureDesc) -> Result<CustomTextureId> {
        let (format, bytes_per_pixel) = match desc.format {
            CustomTextureFormat::Rgba8Unorm => (gpu::TextureFormat::Rgba8Unorm, 4),
            CustomTextureFormat::Bgra8Unorm => (gpu::TextureFormat::Bgra8Unorm, 4),
        };
        let raw = self.gpu.create_texture(gpu::TextureDesc {
            name: &desc.name,
            format,
            size: gpu::Extent {
                width: desc.width,
                height: desc.height,
                depth: 1,
            },
            array_layer_count: 1,
            mip_level_count: 1,
            sample_count: 1,
            dimension: gpu::TextureDimension::D2,
            usage: gpu::TextureUsage::COPY | gpu::TextureUsage::RESOURCE,
            external: None,
        });
        let view = self.gpu.create_texture_view(
            raw,
            gpu::TextureViewDesc {
                name: &desc.name,
                format,
                dimension: gpu::ViewDimension::D2,
                subresources: &Default::default(),
            },
        );
        let texture = BladeCustomTexture {
            texture: raw,
            view,
            width: desc.width,
            height: desc.height,
            bytes_per_pixel,
        };
        let mut textures = self.textures.lock().unwrap();
        let id = Self::alloc_slot(&mut textures, texture);
        drop(textures);
        self.update_texture(CustomTextureId(id), desc.data)?;
        Ok(CustomTextureId(id))
    }

    fn update_texture(&self, id: CustomTextureId, data: Arc<[u8]>) -> Result<()> {
        if data.is_empty() {
            return Err(anyhow::anyhow!("custom texture data is empty"));
        }
        let mut upload_belt = self.upload_belt.lock().unwrap();
        let piece = upload_belt.alloc_bytes(&data, &self.gpu);
        let mut pending = self.pending_uploads.lock().unwrap();
        pending.push(PendingUpload {
            texture_id: id,
            data: piece,
        });
        Ok(())
    }

    fn remove_texture(&self, id: CustomTextureId) {
        let mut textures = self.textures.lock().unwrap();
        if let Some(slot) = textures.get_mut(id.0 as usize) {
            if let Some(texture) = slot.take() {
                self.gpu.destroy_texture_view(texture.view);
                self.gpu.destroy_texture(texture.texture);
            }
        }
    }

    fn create_sampler(&self, desc: CustomSamplerDesc) -> Result<CustomSamplerId> {
        let sampler = self.gpu.create_sampler(gpu::SamplerDesc {
            name: &desc.name,
            mag_filter: match desc.mag_filter {
                CustomFilterMode::Nearest => gpu::FilterMode::Nearest,
                CustomFilterMode::Linear => gpu::FilterMode::Linear,
            },
            min_filter: match desc.min_filter {
                CustomFilterMode::Nearest => gpu::FilterMode::Nearest,
                CustomFilterMode::Linear => gpu::FilterMode::Linear,
            },
            mipmap_filter: match desc.mipmap_filter {
                CustomFilterMode::Nearest => gpu::FilterMode::Nearest,
                CustomFilterMode::Linear => gpu::FilterMode::Linear,
            },
            address_modes: [
                match desc.address_modes[0] {
                    CustomAddressMode::ClampToEdge => gpu::AddressMode::ClampToEdge,
                    CustomAddressMode::Repeat => gpu::AddressMode::Repeat,
                },
                match desc.address_modes[1] {
                    CustomAddressMode::ClampToEdge => gpu::AddressMode::ClampToEdge,
                    CustomAddressMode::Repeat => gpu::AddressMode::Repeat,
                },
                match desc.address_modes[2] {
                    CustomAddressMode::ClampToEdge => gpu::AddressMode::ClampToEdge,
                    CustomAddressMode::Repeat => gpu::AddressMode::Repeat,
                },
            ],
            ..Default::default()
        });
        let mut samplers = self.samplers.lock().unwrap();
        let id = Self::alloc_slot(&mut samplers, sampler);
        Ok(CustomSamplerId(id))
    }

    fn remove_sampler(&self, id: CustomSamplerId) {
        let mut samplers = self.samplers.lock().unwrap();
        if let Some(slot) = samplers.get_mut(id.0 as usize) {
            if let Some(sampler) = slot.take() {
                self.gpu.destroy_sampler(sampler);
            }
        }
    }
}

pub(crate) struct CustomBindings<'a> {
    pub(crate) bindings: &'a [CustomBindingValue],
    pub(crate) binding_kinds: &'a [CustomBindingKind],
    pub(crate) buffers: &'a [Option<gpu::Buffer>],
    pub(crate) textures: &'a [Option<gpu::TextureView>],
    pub(crate) samplers: &'a [Option<gpu::Sampler>],
    pub(crate) instance_belt: *mut blade_util::BufferBelt,
    pub(crate) gpu: *const gpu::Context,
}

impl gpu::ShaderData for CustomBindings<'_> {
    fn layout() -> gpu::ShaderDataLayout {
        gpu::ShaderDataLayout::default()
    }

    fn fill(&self, mut context: gpu::PipelineContext) {
        // SAFETY: CustomBindings is created during rendering with exclusive access to the
        // renderer's instance_belt and gpu context. fill() runs synchronously on the render
        // thread, so these raw pointers remain valid for the duration of the call.
        let (instance_belt, gpu) = unsafe { (&mut *self.instance_belt, &*self.gpu) };
        let count = self.bindings.len().min(self.binding_kinds.len());
        for i in 0..count {
            match (&self.binding_kinds[i], &self.bindings[i]) {
                (CustomBindingKind::Buffer, CustomBindingValue::Buffer(source)) => {
                    match source {
                        CustomBufferSource::Inline(data) => {
                            let piece = instance_belt.alloc_bytes(data, gpu);
                            piece.bind_to(&mut context, i as u32);
                        }
                        CustomBufferSource::Buffer(id) => {
                            if let Some(buffer) = self
                                .buffers
                                .get(id.0 as usize)
                                .and_then(|slot| slot.as_ref())
                            {
                                let piece = gpu::BufferPiece::from(*buffer);
                                piece.bind_to(&mut context, i as u32);
                            }
                        }
                    }
                }
                (CustomBindingKind::Texture, CustomBindingValue::Texture(id)) => {
                    if let Some(view) = self
                        .textures
                        .get(id.0 as usize)
                        .and_then(|slot| slot.as_ref())
                    {
                        view.bind_to(&mut context, i as u32);
                    }
                }
                (CustomBindingKind::Sampler, CustomBindingValue::Sampler(id)) => {
                    if let Some(sampler) = self
                        .samplers
                        .get(id.0 as usize)
                        .and_then(|slot| slot.as_ref())
                    {
                        sampler.bind_to(&mut context, i as u32);
                    }
                }
                (CustomBindingKind::Uniform { .. }, CustomBindingValue::Uniform(source)) => {
                    match source {
                        CustomBufferSource::Inline(data) => {
                            let piece = instance_belt.alloc_bytes(data, gpu);
                            piece.bind_to(&mut context, i as u32);
                        }
                        CustomBufferSource::Buffer(id) => {
                            if let Some(buffer) = self
                                .buffers
                                .get(id.0 as usize)
                                .and_then(|slot| slot.as_ref())
                            {
                                let piece = gpu::BufferPiece::from(*buffer);
                                piece.bind_to(&mut context, i as u32);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
}
