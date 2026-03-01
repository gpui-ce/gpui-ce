use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use blade_graphics::{self as gpu, ShaderBindable as _};
use blade_util::{BufferBelt, BufferBeltDescriptor};

use crate::{
    CustomAddressMode, CustomBindingKind, CustomBindingValue, CustomBlendMode, CustomBufferDesc,
    CustomBufferId, CustomBufferSource, CustomComputePipelineDesc, CustomComputePipelineId,
    CustomCullMode, CustomDepthCompare, CustomDepthFormat, CustomDepthTargetDesc,
    CustomDepthTargetId, CustomDrawRegistry, CustomFilterMode, CustomFrontFace, CustomPipelineDesc,
    CustomPipelineId, CustomPrimitiveTopology, CustomRenderTargetDesc, CustomSamplerDesc,
    CustomSamplerId, CustomTextureDesc, CustomTextureFormat, CustomTextureId, CustomTextureUpdate,
    CustomTextureUsage, CustomVertexFormat, Result,
};

pub(crate) struct BladeCustomDrawRegistry {
    gpu: Arc<gpu::Context>,
    surface_info: gpu::SurfaceInfo,
    pipelines: Mutex<Vec<Option<BladeCustomPipeline>>>,
    compute_pipelines: Mutex<Vec<Option<BladeCustomComputePipeline>>>,
    buffers: Mutex<Vec<Option<BladeCustomBuffer>>>,
    textures: Mutex<Vec<Option<BladeCustomTexture>>>,
    depth_targets: Mutex<Vec<Option<BladeCustomDepthTarget>>>,
    samplers: Mutex<Vec<Option<gpu::Sampler>>>,
    upload_belt: Mutex<BufferBelt>,
    pending_uploads: Mutex<Vec<PendingUpload>>,
}

pub(crate) struct BladeCustomPipeline {
    pub(crate) pipeline: gpu::RenderPipeline,
    pub(crate) bindings: Vec<Vec<CustomBindingKind>>,
    pub(crate) binding_indices: Vec<Vec<Option<usize>>>,
    pub(crate) color_format: gpu::TextureFormat,
    pub(crate) depth_format: Option<gpu::TextureFormat>,
}

pub(crate) struct BladeCustomComputePipeline {
    pub(crate) pipeline: gpu::ComputePipeline,
    pub(crate) bindings: Vec<Vec<CustomBindingKind>>,
    pub(crate) binding_indices: Vec<Vec<Option<usize>>>,
}

struct BladeCustomBuffer {
    buffer: gpu::Buffer,
    size: u64,
}

pub(crate) struct BladeBufferSnapshot {
    pub(crate) buffer: gpu::Buffer,
    pub(crate) size: u64,
}

pub(crate) struct BladeCustomTexture {
    pub(crate) texture: gpu::Texture,
    pub(crate) view: gpu::TextureView,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) mip_level_count: u32,
    pub(crate) bytes_per_pixel: u32,
    pub(crate) format: gpu::TextureFormat,
    pub(crate) is_render_target: bool,
    pub(crate) clear_color: gpu::TextureColor,
}

pub(crate) struct BladeCustomDepthTarget {
    pub(crate) texture: gpu::Texture,
    pub(crate) view: gpu::TextureView,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) format: gpu::TextureFormat,
    pub(crate) clear_depth: f32,
}

struct PendingUpload {
    texture_id: CustomTextureId,
    data: gpu::BufferPiece,
    mip_level: u32,
    width: u32,
    height: u32,
    bytes_per_row: u32,
}

impl BladeCustomDrawRegistry {
    pub(crate) fn new(gpu: Arc<gpu::Context>, surface_info: gpu::SurfaceInfo) -> Self {
        Self {
            gpu,
            surface_info,
            pipelines: Mutex::new(Vec::new()),
            compute_pipelines: Mutex::new(Vec::new()),
            buffers: Mutex::new(Vec::new()),
            textures: Mutex::new(Vec::new()),
            depth_targets: Mutex::new(Vec::new()),
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

        let mut compute_pipelines = self.compute_pipelines.lock().unwrap();
        for pipeline in compute_pipelines.iter_mut().flatten() {
            self.gpu.destroy_compute_pipeline(&mut pipeline.pipeline);
        }
        compute_pipelines.clear();
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
        let mut depth_targets = self.depth_targets.lock().unwrap();
        for depth_target in depth_targets.iter_mut().flatten() {
            self.gpu.destroy_texture_view(depth_target.view);
            self.gpu.destroy_texture(depth_target.texture);
        }
        depth_targets.clear();
        self.upload_belt.lock().unwrap().destroy(&self.gpu);
        self.pending_uploads.lock().unwrap().clear();
    }

    pub(crate) fn with_pipeline<F, R>(&self, id: CustomPipelineId, f: F) -> Option<R>
    where
        F: FnOnce(&BladeCustomPipeline) -> R,
    {
        let pipelines = self.pipelines.lock().unwrap();
        let entry = pipelines.get(id.0 as usize)?.as_ref()?;
        Some(f(entry))
    }

    pub(crate) fn with_compute_pipeline<F, R>(&self, id: CustomComputePipelineId, f: F) -> Option<R>
    where
        F: FnOnce(&BladeCustomComputePipeline) -> R,
    {
        let pipelines = self.compute_pipelines.lock().unwrap();
        let entry = pipelines.get(id.0 as usize)?.as_ref()?;
        Some(f(entry))
    }

    pub(crate) fn with_texture<F, R>(&self, id: CustomTextureId, f: F) -> Option<R>
    where
        F: FnOnce(&BladeCustomTexture) -> R,
    {
        let textures = self.textures.lock().unwrap();
        let entry = textures.get(id.0 as usize)?.as_ref()?;
        Some(f(entry))
    }

    pub(crate) fn with_depth_target<F, R>(&self, id: CustomDepthTargetId, f: F) -> Option<R>
    where
        F: FnOnce(&BladeCustomDepthTarget) -> R,
    {
        let depth_targets = self.depth_targets.lock().unwrap();
        let entry = depth_targets.get(id.0 as usize)?.as_ref()?;
        Some(f(entry))
    }

    pub(crate) fn buffers_snapshot(&self) -> Vec<Option<BladeBufferSnapshot>> {
        self.buffers
            .lock()
            .unwrap()
            .iter()
            .map(|slot| {
                slot.as_ref().map(|entry| BladeBufferSnapshot {
                    buffer: entry.buffer,
                    size: entry.size,
                })
            })
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
                upload.bytes_per_row,
                gpu::TexturePiece {
                    texture: texture.texture,
                    mip_level: upload.mip_level,
                    array_layer: 0,
                    origin: [0, 0, 0],
                },
                gpu::Extent {
                    width: upload.width,
                    height: upload.height,
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

        let color_format = desc
            .target_format
            .map(blade_color_format)
            .unwrap_or(self.surface_info.format);
        let color_targets = &[gpu::ColorTargetState {
            format: color_format,
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
                CustomBindingKind::Texture | CustomBindingKind::StorageTexture => {
                    gpu::ShaderBinding::Texture
                }
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

        let depth_format = desc
            .state
            .depth
            .map(|depth| blade_depth_format(depth.format));
        let depth_stencil = desc.state.depth.map(|depth| gpu::DepthStencilState {
            format: blade_depth_format(depth.format),
            depth_write_enabled: depth.write_enabled,
            depth_compare: blade_depth_compare(depth.compare),
            stencil: gpu::StencilState::default(),
            bias: gpu::DepthBiasState::default(),
        });

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
            depth_stencil,
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
                color_format,
                depth_format,
            },
        );
        Ok(CustomPipelineId(id))
    }

    fn create_compute_pipeline(
        &self,
        desc: CustomComputePipelineDesc,
    ) -> Result<CustomComputePipelineId> {
        let shader = self.gpu.create_shader(gpu::ShaderDesc {
            source: &desc.shader_source,
        });

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
                CustomBindingKind::Texture | CustomBindingKind::StorageTexture => {
                    gpu::ShaderBinding::Texture
                }
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

        let pipeline = self.gpu.create_compute_pipeline(gpu::ComputePipelineDesc {
            name: &desc.name,
            data_layouts: data_layouts.as_slice(),
            compute: shader.at(&desc.entry_point),
        });

        let mut pipelines = self.compute_pipelines.lock().unwrap();
        let id = Self::alloc_slot(
            &mut pipelines,
            BladeCustomComputePipeline {
                pipeline,
                bindings: binding_kinds_by_group,
                binding_indices: binding_indices_by_group,
            },
        );
        Ok(CustomComputePipelineId(id))
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
            CustomTextureFormat::Rgba8UnormSrgb => (gpu::TextureFormat::Rgba8UnormSrgb, 4),
            CustomTextureFormat::Bgra8UnormSrgb => (gpu::TextureFormat::Bgra8UnormSrgb, 4),
        };
        if desc.data.is_empty() {
            return Err(anyhow::anyhow!(
                "custom texture data must include at least one mip level"
            ));
        }
        let max_levels = max_mip_levels(desc.width, desc.height);
        if desc.data.len() as u32 > max_levels {
            return Err(anyhow::anyhow!(
                "custom texture mip level count {} exceeds maximum {}",
                desc.data.len(),
                max_levels
            ));
        }
        for (level, data) in desc.data.iter().enumerate() {
            let (width, height) = mip_level_size(desc.width, desc.height, level as u32);
            let expected_len = (width * height * bytes_per_pixel) as usize;
            if data.len() < expected_len {
                return Err(anyhow::anyhow!(
                    "custom texture mip level {} data is smaller than texture size",
                    level
                ));
            }
        }

        let is_sampled = desc.usage.contains(CustomTextureUsage::SAMPLED);
        let is_storage = desc.usage.contains(CustomTextureUsage::STORAGE);
        if !is_sampled && !is_storage {
            return Err(anyhow::anyhow!(
                "custom texture usage must include sampled or storage"
            ));
        }
        let mut usage = gpu::TextureUsage::COPY;
        if is_sampled {
            usage |= gpu::TextureUsage::RESOURCE;
        }
        if is_storage {
            usage |= gpu::TextureUsage::STORAGE;
        }

        let raw = self.gpu.create_texture(gpu::TextureDesc {
            name: &desc.name,
            format,
            size: gpu::Extent {
                width: desc.width,
                height: desc.height,
                depth: 1,
            },
            array_layer_count: 1,
            mip_level_count: desc.data.len() as u32,
            sample_count: 1,
            dimension: gpu::TextureDimension::D2,
            usage,
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
            mip_level_count: desc.data.len() as u32,
            bytes_per_pixel,
            format,
            is_render_target: false,
            clear_color: gpu::TextureColor::TransparentBlack,
        };
        let mut textures = self.textures.lock().unwrap();
        let id = Self::alloc_slot(&mut textures, texture);
        drop(textures);
        for (level, data) in desc.data.iter().enumerate() {
            self.update_texture(
                CustomTextureId(id),
                CustomTextureUpdate {
                    level: level as u32,
                    data: Arc::clone(data),
                },
            )?;
        }
        Ok(CustomTextureId(id))
    }

    fn create_render_target(&self, desc: CustomRenderTargetDesc) -> Result<CustomTextureId> {
        let (format, bytes_per_pixel) = match desc.format {
            CustomTextureFormat::Rgba8Unorm => (gpu::TextureFormat::Rgba8Unorm, 4),
            CustomTextureFormat::Bgra8Unorm => (gpu::TextureFormat::Bgra8Unorm, 4),
            CustomTextureFormat::Rgba8UnormSrgb => (gpu::TextureFormat::Rgba8UnormSrgb, 4),
            CustomTextureFormat::Bgra8UnormSrgb => (gpu::TextureFormat::Bgra8UnormSrgb, 4),
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
            usage: gpu::TextureUsage::RESOURCE | gpu::TextureUsage::TARGET,
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
            mip_level_count: 1,
            bytes_per_pixel,
            format,
            is_render_target: true,
            clear_color: blade_clear_color(desc.clear_color),
        };
        let mut textures = self.textures.lock().unwrap();
        let id = Self::alloc_slot(&mut textures, texture);
        Ok(CustomTextureId(id))
    }

    fn update_texture(&self, id: CustomTextureId, update: CustomTextureUpdate) -> Result<()> {
        if update.data.is_empty() {
            return Err(anyhow::anyhow!("custom texture data is empty"));
        }
        let (is_render_target, width, height, bytes_per_pixel, mip_level_count) = {
            let textures = self.textures.lock().unwrap();
            let Some(entry) = textures.get(id.0 as usize).and_then(|slot| slot.as_ref()) else {
                return Err(anyhow::anyhow!("custom texture id out of range"));
            };
            (
                entry.is_render_target,
                entry.width,
                entry.height,
                entry.bytes_per_pixel,
                entry.mip_level_count,
            )
        };
        if is_render_target {
            return Err(anyhow::anyhow!("custom render targets cannot be updated"));
        }
        if update.level >= mip_level_count {
            return Err(anyhow::anyhow!(
                "custom texture mip level {} out of range",
                update.level
            ));
        }
        let (level_width, level_height) = mip_level_size(width, height, update.level);
        let expected_len = (level_width * level_height * bytes_per_pixel) as usize;
        if update.data.len() < expected_len {
            return Err(anyhow::anyhow!(
                "custom texture data is smaller than texture size"
            ));
        }
        let mut upload_belt = self.upload_belt.lock().unwrap();
        let piece = upload_belt.alloc_bytes(&update.data, &self.gpu);
        let mut pending = self.pending_uploads.lock().unwrap();
        pending.push(PendingUpload {
            texture_id: id,
            data: piece,
            mip_level: update.level,
            width: level_width,
            height: level_height,
            bytes_per_row: level_width.saturating_mul(bytes_per_pixel),
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

    fn create_depth_target(&self, desc: CustomDepthTargetDesc) -> Result<CustomDepthTargetId> {
        let format = blade_depth_format(desc.format);
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
            usage: gpu::TextureUsage::TARGET,
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
        let target = BladeCustomDepthTarget {
            texture: raw,
            view,
            width: desc.width,
            height: desc.height,
            format,
            clear_depth: desc.clear_depth.unwrap_or(1.0),
        };
        let mut depth_targets = self.depth_targets.lock().unwrap();
        let id = Self::alloc_slot(&mut depth_targets, target);
        Ok(CustomDepthTargetId(id))
    }

    fn remove_depth_target(&self, id: CustomDepthTargetId) {
        let mut depth_targets = self.depth_targets.lock().unwrap();
        if let Some(slot) = depth_targets.get_mut(id.0 as usize) {
            if let Some(target) = slot.take() {
                self.gpu.destroy_texture_view(target.view);
                self.gpu.destroy_texture(target.texture);
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

fn blade_color_format(format: CustomTextureFormat) -> gpu::TextureFormat {
    match format {
        CustomTextureFormat::Rgba8Unorm => gpu::TextureFormat::Rgba8Unorm,
        CustomTextureFormat::Bgra8Unorm => gpu::TextureFormat::Bgra8Unorm,
        CustomTextureFormat::Rgba8UnormSrgb => gpu::TextureFormat::Rgba8UnormSrgb,
        CustomTextureFormat::Bgra8UnormSrgb => gpu::TextureFormat::Bgra8UnormSrgb,
    }
}

fn max_mip_levels(width: u32, height: u32) -> u32 {
    let mut levels = 1;
    let mut size = width.max(height);
    while size > 1 {
        size /= 2;
        levels += 1;
    }
    levels
}

fn mip_level_size(width: u32, height: u32, level: u32) -> (u32, u32) {
    let mut level_width = width.max(1);
    let mut level_height = height.max(1);
    for _ in 0..level {
        level_width = (level_width / 2).max(1);
        level_height = (level_height / 2).max(1);
    }
    (level_width, level_height)
}

fn blade_depth_format(format: CustomDepthFormat) -> gpu::TextureFormat {
    match format {
        CustomDepthFormat::Depth32Float => gpu::TextureFormat::Depth32Float,
    }
}

fn blade_depth_compare(compare: CustomDepthCompare) -> gpu::CompareFunction {
    match compare {
        CustomDepthCompare::Always => gpu::CompareFunction::Always,
        CustomDepthCompare::Less => gpu::CompareFunction::Less,
        CustomDepthCompare::LessEqual => gpu::CompareFunction::LessEqual,
        CustomDepthCompare::Greater => gpu::CompareFunction::Greater,
        CustomDepthCompare::GreaterEqual => gpu::CompareFunction::GreaterEqual,
    }
}

fn blade_clear_color(color: Option<[f32; 4]>) -> gpu::TextureColor {
    match color {
        Some([0.0, 0.0, 0.0, 0.0]) | None => gpu::TextureColor::TransparentBlack,
        Some([0.0, 0.0, 0.0, 1.0]) => gpu::TextureColor::OpaqueBlack,
        Some([1.0, 1.0, 1.0, 1.0]) => gpu::TextureColor::White,
        _ => gpu::TextureColor::TransparentBlack,
    }
}

pub(crate) struct CustomBindings<'a> {
    pub(crate) bindings: &'a [CustomBindingValue],
    pub(crate) binding_kinds: &'a [CustomBindingKind],
    pub(crate) binding_indices: &'a [Option<usize>],
    pub(crate) buffers: &'a [Option<BladeBufferSnapshot>],
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
                            let piece = gpu::BufferPiece::from(buffer.buffer);
                            piece.bind_to(&mut context, i as u32);
                        } else {
                            log::warn!("custom draw buffer {:?} missing", id.0);
                        }
                    }
                    CustomBufferSource::BufferSlice { id, offset, size } => {
                        if let Some(buffer) = self
                            .buffers
                            .get(id.0 as usize)
                            .and_then(|slot| slot.as_ref())
                        {
                            if *size == 0 {
                                log::warn!("custom draw buffer slice is empty");
                            } else if offset.saturating_add(*size) > buffer.size {
                                log::warn!("custom draw buffer slice out of range");
                            } else {
                                let piece = gpu::BufferPiece {
                                    buffer: buffer.buffer,
                                    offset: *offset,
                                };
                                piece.bind_to(&mut context, i as u32);
                            }
                        } else {
                            log::warn!("custom draw buffer {:?} missing", id.0);
                        }
                    }
                },
                (
                    CustomBindingKind::Texture | CustomBindingKind::StorageTexture,
                    CustomBindingValue::Texture(id),
                ) => {
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
                (
                    CustomBindingKind::Uniform {
                        size: expected_size,
                    },
                    CustomBindingValue::Uniform(source),
                ) => match source {
                    CustomBufferSource::Inline(data) => {
                        if data.len() != *expected_size as usize {
                            log::warn!(
                                "custom draw uniform size mismatch (expected {}, got {})",
                                expected_size,
                                data.len()
                            );
                        } else {
                            let piece = instance_belt.alloc_bytes(data, gpu);
                            piece.bind_to(&mut context, i as u32);
                        }
                    }
                    CustomBufferSource::Buffer(id) => {
                        if let Some(buffer) = self
                            .buffers
                            .get(id.0 as usize)
                            .and_then(|slot| slot.as_ref())
                        {
                            if buffer.size < *expected_size as u64 {
                                log::warn!(
                                    "custom draw uniform buffer too small (expected {}, got {})",
                                    expected_size,
                                    buffer.size
                                );
                            } else {
                                let piece = gpu::BufferPiece::from(buffer.buffer);
                                piece.bind_to(&mut context, i as u32);
                            }
                        } else {
                            log::warn!("custom draw uniform buffer {:?} missing", id.0);
                        }
                    }
                    CustomBufferSource::BufferSlice { id, offset, size } => {
                        if let Some(buffer) = self
                            .buffers
                            .get(id.0 as usize)
                            .and_then(|slot| slot.as_ref())
                        {
                            if *size == 0 {
                                log::warn!("custom draw uniform buffer slice is empty");
                            } else if offset.saturating_add(*size) > buffer.size {
                                log::warn!("custom draw uniform buffer slice out of range");
                            } else if *size < *expected_size as u64 {
                                log::warn!(
                                    "custom draw uniform buffer slice too small (expected {}, got {})",
                                    expected_size,
                                    size
                                );
                            } else {
                                let piece = gpu::BufferPiece {
                                    buffer: buffer.buffer,
                                    offset: *offset,
                                };
                                piece.bind_to(&mut context, i as u32);
                            }
                        } else {
                            log::warn!("custom draw uniform buffer {:?} missing", id.0);
                        }
                    }
                },
                _ => {}
            }
        }
    }
}
