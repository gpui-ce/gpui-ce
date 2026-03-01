use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use blade_graphics::{self as gpu, ShaderBindable as _};
use blade_util::{BufferBelt, BufferBeltDescriptor};

use crate::{
    CustomAddressMode, CustomBindingKind, CustomBindingValue, CustomBlendMode, CustomBufferDesc,
    CustomBufferId, CustomBufferSource, CustomCullMode, CustomDrawRegistry, CustomFilterMode,
    CustomFrontFace, CustomPipelineDesc, CustomPipelineId, CustomPrimitiveTopology,
    CustomSamplerDesc, CustomSamplerId, CustomTextureDesc, CustomTextureFormat, CustomTextureId,
    CustomVertexFormat, Result,
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
    bindings: Vec<Vec<CustomBindingKind>>,
    binding_indices: Vec<Vec<Option<usize>>>,
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
        F: FnOnce(&gpu::RenderPipeline, &[Vec<CustomBindingKind>], &[Vec<Option<usize>>]) -> R,
    {
        let pipelines = self.pipelines.lock().unwrap();
        let entry = pipelines.get(id.0 as usize)?.as_ref()?;
        Some(f(&entry.pipeline, &entry.bindings, &entry.binding_indices))
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
        if let Some((index, slot)) = slots
            .iter_mut()
            .enumerate()
            .find(|(_, slot)| slot.is_none())
        {
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
            blend: blade_blend_state(desc.state.blend, self.surface_info.alpha),
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
                        location: attr.location,
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

        let mut group_entries: BTreeMap<
            u32,
            Vec<Option<(&'static str, gpu::ShaderBinding, CustomBindingKind, usize)>>,
        > = BTreeMap::new();
        for (flat_index, binding) in desc.bindings.iter().enumerate() {
            let slot = binding.slot.unwrap_or(crate::CustomBindingSlot {
                group: 0,
                binding: binding.name.index(),
            });
            let name = binding.name.as_str();
            let shader_binding = match binding.kind {
                CustomBindingKind::Buffer => gpu::ShaderBinding::Buffer,
                CustomBindingKind::Texture => gpu::ShaderBinding::Texture,
                CustomBindingKind::Sampler => gpu::ShaderBinding::Sampler,
                CustomBindingKind::Uniform { size } => gpu::ShaderBinding::Plain { size },
            };
            let group = group_entries.entry(slot.group).or_default();
            let binding_index = slot.binding as usize;
            if group.len() <= binding_index {
                group.resize(binding_index + 1, None);
            }
            group[binding_index] = Some((name, shader_binding, binding.kind, flat_index));
        }

        let mut binding_layouts: Vec<gpu::ShaderDataLayout> = Vec::new();
        let mut binding_kinds_by_group: Vec<Vec<CustomBindingKind>> = Vec::new();
        let mut binding_indices_by_group: Vec<Vec<Option<usize>>> = Vec::new();
        if !desc.bindings.is_empty() {
            let max_group = *group_entries.keys().max().unwrap_or(&0);
            for group in 0..=max_group {
                if let Some(entries) = group_entries.get(&group) {
                    let mut layout = gpu::ShaderDataLayout::default();
                    let mut kinds = Vec::with_capacity(entries.len());
                    let mut indices = Vec::with_capacity(entries.len());
                    for (binding_index, entry) in entries.iter().enumerate() {
                        match entry {
                            Some((name, shader_binding, kind, flat_index)) => {
                                layout.bindings.push((name, *shader_binding));
                                kinds.push(*kind);
                                indices.push(Some(*flat_index));
                            }
                            None => {
                                let placeholder = Box::leak(
                                    format!("__gpui_unused_b{}_{}", group, binding_index)
                                        .into_boxed_str(),
                                );
                                layout
                                    .bindings
                                    .push((placeholder, gpu::ShaderBinding::Buffer));
                                kinds.push(CustomBindingKind::Buffer);
                                indices.push(None);
                            }
                        }
                    }
                    binding_layouts.push(layout);
                    binding_kinds_by_group.push(kinds);
                    binding_indices_by_group.push(indices);
                } else {
                    binding_layouts.push(gpu::ShaderDataLayout::default());
                    binding_kinds_by_group.push(Vec::new());
                    binding_indices_by_group.push(Vec::new());
                }
            }
        }

        let data_layouts: Vec<&gpu::ShaderDataLayout> = binding_layouts.iter().collect();

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
                front_face: blade_front_face(desc.state.front_face),
                cull_mode: blade_cull_mode(desc.state.cull_mode),
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
                bindings: binding_kinds_by_group,
                binding_indices: binding_indices_by_group,
            },
        );
        Ok(CustomPipelineId(id))
    }

    fn create_pipeline_msl(
        &self,
        _desc: CustomPipelineDesc,
        _msl_source: String,
    ) -> Result<CustomPipelineId> {
        Err(anyhow::anyhow!(
            "precompiled MSL is only supported on the Metal backend"
        ))
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

fn blade_blend_state(
    blend: CustomBlendMode,
    alpha_mode: gpu::AlphaMode,
) -> Option<gpu::BlendState> {
    match blend {
        CustomBlendMode::Default => Some(match alpha_mode {
            gpu::AlphaMode::Ignored => gpu::BlendState::ALPHA_BLENDING,
            gpu::AlphaMode::PreMultiplied => gpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING,
            gpu::AlphaMode::PostMultiplied => gpu::BlendState::ALPHA_BLENDING,
        }),
        CustomBlendMode::Opaque => None,
        CustomBlendMode::Alpha => Some(gpu::BlendState::ALPHA_BLENDING),
        CustomBlendMode::PremultipliedAlpha => Some(gpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
    }
}

fn blade_front_face(face: CustomFrontFace) -> gpu::FrontFace {
    match face {
        CustomFrontFace::Ccw => gpu::FrontFace::Ccw,
        CustomFrontFace::Cw => gpu::FrontFace::Cw,
    }
}

fn blade_cull_mode(mode: CustomCullMode) -> Option<gpu::Face> {
    match mode {
        CustomCullMode::None => None,
        CustomCullMode::Front => Some(gpu::Face::Front),
        CustomCullMode::Back => Some(gpu::Face::Back),
    }
}

pub(crate) struct CustomBindings<'a> {
    pub(crate) bindings: &'a [CustomBindingValue],
    pub(crate) binding_kinds: &'a [CustomBindingKind],
    pub(crate) binding_indices: &'a [Option<usize>],
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
        let count = self.binding_kinds.len().min(self.binding_indices.len());
        for i in 0..count {
            let Some(binding_index) = self.binding_indices.get(i).and_then(|slot| *slot) else {
                continue;
            };
            let Some(binding_value) = self.bindings.get(binding_index) else {
                continue;
            };
            match (&self.binding_kinds[i], binding_value) {
                (CustomBindingKind::Buffer, CustomBindingValue::Buffer(source)) => match source {
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
                },
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
