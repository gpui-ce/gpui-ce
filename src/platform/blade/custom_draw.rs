use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use blade_graphics::{self as gpu, ShaderBindable as _};
use blade_util::{BufferBelt, BufferBeltDescriptor};

use crate::{
    CustomAddressMode, CustomBindingDesc, CustomBindingKind, CustomBindingSlot, CustomBindingValue,
    CustomBlendMode, CustomBufferDesc, CustomBufferId, CustomBufferSource,
    CustomComputePipelineDesc, CustomComputePipelineId, CustomCullMode, CustomDepthCompare,
    CustomDepthFormat, CustomDepthTargetDesc, CustomDepthTargetId, CustomDrawRegistry,
    CustomDrawResourceStats, CustomFilterMode, CustomFrameDiagnostics, CustomFrontFace,
    CustomGpuFrameProfile, CustomPipelineDesc, CustomPipelineId, CustomPrimitiveTopology,
    CustomPushConstantsDesc, CustomRenderTargetDesc, CustomSamplerDesc, CustomSamplerId,
    CustomTextureBufferUpdate, CustomTextureDesc, CustomTextureDimension, CustomTextureFormat,
    CustomTextureId, CustomTextureUpdate, CustomTextureUsage, CustomVertexFormat, Result,
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
    pub(crate) color_formats: Vec<gpu::TextureFormat>,
    pub(crate) depth_format: Option<gpu::TextureFormat>,
    pub(crate) sample_count: u32,
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
    pub(crate) array_layer_count: u32,
    pub(crate) mip_level_count: u32,
    pub(crate) sample_count: u32,
    pub(crate) block_width: u32,
    pub(crate) block_height: u32,
    pub(crate) bytes_per_block: u32,
    pub(crate) format: gpu::TextureFormat,
    pub(crate) is_render_target: bool,
    pub(crate) clear_color: gpu::TextureColor,
    pub(crate) msaa_texture: Option<gpu::Texture>,
    pub(crate) msaa_view: Option<gpu::TextureView>,
}

pub(crate) struct BladeCustomDepthTarget {
    pub(crate) texture: gpu::Texture,
    pub(crate) view: gpu::TextureView,
    pub(crate) format: gpu::TextureFormat,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) sample_count: u32,
    pub(crate) clear_depth: f32,
}

enum PendingUploadSource {
    Staging(gpu::BufferPiece),
    Buffer { id: CustomBufferId, offset: u64 },
}

struct PendingUpload {
    texture_id: CustomTextureId,
    source: PendingUploadSource,
    mip_level: u32,
    array_layer: u32,
    width: u32,
    height: u32,
    bytes_per_row: u32,
    bytes_per_image: u64,
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
            if let Some(msaa_view) = texture.msaa_view {
                self.gpu.destroy_texture_view(msaa_view);
            }
            if let Some(msaa_texture) = texture.msaa_texture {
                self.gpu.destroy_texture(msaa_texture);
            }
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
            let texture = {
                let textures = self.textures.lock().unwrap();
                let Some(texture) = textures
                    .get(upload.texture_id.0 as usize)
                    .and_then(|slot| slot.as_ref())
                else {
                    continue;
                };
                texture.texture
            };
            let data = match upload.source {
                PendingUploadSource::Staging(piece) => Some(piece),
                PendingUploadSource::Buffer { id, offset } => {
                    let buffers = self.buffers.lock().unwrap();
                    let Some(buffer) = buffers.get(id.0 as usize).and_then(|slot| slot.as_ref())
                    else {
                        log::warn!("custom draw buffer {:?} missing", id.0);
                        continue;
                    };
                    if offset.saturating_add(upload.bytes_per_image) > buffer.size {
                        log::warn!("custom draw buffer slice out of range");
                        continue;
                    }
                    Some(buffer.buffer.at(offset))
                }
            };
            let Some(data) = data else {
                continue;
            };
            encoder.init_texture(texture);
            let mut transfers = encoder.transfer("custom_draw_texture");
            transfers.copy_buffer_to_texture(
                data,
                upload.bytes_per_row,
                gpu::TexturePiece {
                    texture,
                    mip_level: upload.mip_level,
                    array_layer: upload.array_layer,
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
        let push_constants_slot = push_constants_slot(&desc.bindings);
        let (shader_source, push_constants) = prepare_blade_shader_source(
            &desc.shader_source,
            &[&desc.vertex_entry, &desc.fragment_entry],
            &desc.bindings,
            desc.push_constants,
            push_constants_slot,
        )?;
        let shader = self.gpu.create_shader(gpu::ShaderDesc {
            source: &shader_source,
        });

        let color_formats: Vec<gpu::TextureFormat> = if desc.color_targets.is_empty() {
            vec![self.surface_info.format]
        } else {
            desc.color_targets
                .iter()
                .copied()
                .map(blade_color_format)
                .collect::<Result<Vec<_>>>()?
        };
        let color_targets: Vec<gpu::ColorTargetState> = color_formats
            .iter()
            .copied()
            .map(|format| gpu::ColorTargetState {
                format,
                blend: blade_blend_state(desc.state.blend, self.surface_info.alpha),
                write_mask: gpu::ColorWrites::default(),
            })
            .collect();

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
                CustomBindingKind::BufferArray { count } => {
                    gpu::ShaderBinding::BufferArray { count }
                }
                CustomBindingKind::Texture | CustomBindingKind::StorageTexture => {
                    gpu::ShaderBinding::Texture
                }
                CustomBindingKind::TextureArray { count }
                | CustomBindingKind::StorageTextureArray { count } => {
                    gpu::ShaderBinding::TextureArray { count }
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

        if let Some(push_constants) = &push_constants {
            if desc
                .bindings
                .iter()
                .any(|binding| binding.name.as_str() == push_constants.name)
            {
                return Err(anyhow::anyhow!(
                    "custom draw push constants name '{}' conflicts with a binding name",
                    push_constants.name
                ));
            }
            let group = group_entries.entry(push_constants.slot.group).or_default();
            let binding_index = push_constants.slot.binding as usize;
            if group.len() <= binding_index {
                group.resize(binding_index + 1, None);
            }
            if group[binding_index].is_some() {
                return Err(anyhow::anyhow!(
                    "custom draw push constants slot conflicts with existing binding (group {}, binding {})",
                    push_constants.slot.group,
                    push_constants.slot.binding
                ));
            }
            group[binding_index] = Some((
                push_constants.name,
                gpu::ShaderBinding::Plain {
                    size: push_constants.size,
                },
                CustomBindingKind::Uniform {
                    size: push_constants.size,
                },
                desc.bindings.len(),
            ));
        }

        let mut binding_layouts: Vec<gpu::ShaderDataLayout> = Vec::new();
        let mut binding_kinds_by_group: Vec<Vec<CustomBindingKind>> = Vec::new();
        let mut binding_indices_by_group: Vec<Vec<Option<usize>>> = Vec::new();
        if !group_entries.is_empty() {
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
            color_targets: color_targets.as_slice(),
            multisample_state: gpu::MultisampleState {
                sample_count: desc.state.sample_count,
                ..Default::default()
            },
        });

        let mut pipelines = self.pipelines.lock().unwrap();
        let id = Self::alloc_slot(
            &mut pipelines,
            BladeCustomPipeline {
                pipeline,
                bindings: binding_kinds_by_group,
                binding_indices: binding_indices_by_group,
                color_formats,
                depth_format,
                sample_count: desc.state.sample_count,
            },
        );
        Ok(CustomPipelineId(id))
    }

    fn create_compute_pipeline(
        &self,
        desc: CustomComputePipelineDesc,
    ) -> Result<CustomComputePipelineId> {
        let push_constants_slot = push_constants_slot(&desc.bindings);
        let (shader_source, push_constants) = prepare_blade_shader_source(
            &desc.shader_source,
            &[&desc.entry_point],
            &desc.bindings,
            desc.push_constants,
            push_constants_slot,
        )?;
        let shader = self.gpu.create_shader(gpu::ShaderDesc {
            source: &shader_source,
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
                CustomBindingKind::BufferArray { count } => {
                    gpu::ShaderBinding::BufferArray { count }
                }
                CustomBindingKind::Texture | CustomBindingKind::StorageTexture => {
                    gpu::ShaderBinding::Texture
                }
                CustomBindingKind::TextureArray { count }
                | CustomBindingKind::StorageTextureArray { count } => {
                    gpu::ShaderBinding::TextureArray { count }
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

        if let Some(push_constants) = &push_constants {
            if desc
                .bindings
                .iter()
                .any(|binding| binding.name.as_str() == push_constants.name)
            {
                return Err(anyhow::anyhow!(
                    "custom compute push constants name '{}' conflicts with a binding name",
                    push_constants.name
                ));
            }
            let group = group_entries.entry(push_constants.slot.group).or_default();
            let binding_index = push_constants.slot.binding as usize;
            if group.len() <= binding_index {
                group.resize(binding_index + 1, None);
            }
            if group[binding_index].is_some() {
                return Err(anyhow::anyhow!(
                    "custom compute push constants slot conflicts with existing binding (group {}, binding {})",
                    push_constants.slot.group,
                    push_constants.slot.binding
                ));
            }
            group[binding_index] = Some((
                push_constants.name,
                gpu::ShaderBinding::Plain {
                    size: push_constants.size,
                },
                CustomBindingKind::Uniform {
                    size: push_constants.size,
                },
                desc.bindings.len(),
            ));
        }

        let mut binding_layouts: Vec<gpu::ShaderDataLayout> = Vec::new();
        let mut binding_kinds_by_group: Vec<Vec<CustomBindingKind>> = Vec::new();
        let mut binding_indices_by_group: Vec<Vec<Option<usize>>> = Vec::new();
        if !group_entries.is_empty() {
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

    fn create_pipeline_metallib(
        &self,
        _desc: CustomPipelineDesc,
        _metallib_data: Arc<[u8]>,
    ) -> Result<CustomPipelineId> {
        Err(anyhow::anyhow!(
            "precompiled Metal libraries are only supported on the Metal backend"
        ))
    }

    fn set_pipeline_cache_path(&self, path: Option<std::path::PathBuf>) -> Result<()> {
        if path.is_some() {
            return Err(anyhow::anyhow!(
                "custom draw pipeline cache is only supported on the Metal backend"
            ));
        }
        Ok(())
    }

    fn set_gpu_profiling_enabled(&self, _enabled: bool) -> Result<()> {
        Ok(())
    }

    fn take_last_gpu_profile(&self) -> Option<CustomGpuFrameProfile> {
        None
    }

    fn set_frame_diagnostics_enabled(&self, _enabled: bool) -> Result<()> {
        Ok(())
    }

    fn take_last_frame_diagnostics(&self) -> Option<CustomFrameDiagnostics> {
        None
    }

    fn resource_stats(&self) -> CustomDrawResourceStats {
        let pipeline_count = self
            .pipelines
            .lock()
            .unwrap()
            .iter()
            .filter(|entry| entry.is_some())
            .count() as u32;
        let compute_pipeline_count = self
            .compute_pipelines
            .lock()
            .unwrap()
            .iter()
            .filter(|entry| entry.is_some())
            .count() as u32;

        let (buffer_count, buffer_bytes) = self
            .buffers
            .lock()
            .unwrap()
            .iter()
            .filter_map(|entry| entry.as_ref())
            .fold((0u32, 0u64), |(count, bytes), entry| {
                (count + 1, bytes.saturating_add(entry.size))
            });

        let (texture_count, texture_bytes, render_target_count) = self
            .textures
            .lock()
            .unwrap()
            .iter()
            .filter_map(|entry| entry.as_ref())
            .fold(
                (0u32, 0u64, 0u32),
                |(count, bytes, render_targets), entry| {
                    let mut texture_bytes = bytes.saturating_add(texture_mip_chain_estimate_bytes(
                        entry.width,
                        entry.height,
                        entry.array_layer_count,
                        entry.mip_level_count,
                        entry.block_width,
                        entry.block_height,
                        entry.bytes_per_block,
                    ));
                    if entry.msaa_texture.is_some() && entry.sample_count > 1 {
                        texture_bytes = texture_bytes.saturating_add(
                            texture_level_estimate_bytes(
                                entry.width,
                                entry.height,
                                entry.array_layer_count,
                                entry.block_width,
                                entry.block_height,
                                entry.bytes_per_block,
                            )
                            .saturating_mul(entry.sample_count as u64),
                        );
                    }
                    (
                        count + 1,
                        texture_bytes,
                        render_targets + u32::from(entry.is_render_target),
                    )
                },
            );

        let (depth_target_count, depth_target_bytes) = self
            .depth_targets
            .lock()
            .unwrap()
            .iter()
            .filter_map(|entry| entry.as_ref())
            .fold((0u32, 0u64), |(count, bytes), entry| {
                (
                    count + 1,
                    bytes.saturating_add(depth_target_estimate_bytes(
                        entry.width,
                        entry.height,
                        entry.sample_count,
                    )),
                )
            });

        let sampler_count = self
            .samplers
            .lock()
            .unwrap()
            .iter()
            .filter(|entry| entry.is_some())
            .count() as u32;

        CustomDrawResourceStats {
            pipeline_count,
            compute_pipeline_count,
            buffer_count,
            buffer_bytes,
            texture_count,
            texture_bytes,
            render_target_count,
            depth_target_count,
            depth_target_bytes,
            sampler_count,
        }
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
        let format = blade_color_format(desc.format)?;
        let block_info = desc.format.block_info();
        let block_width = block_info.width;
        let block_height = block_info.height;
        let bytes_per_block = block_info.bytes;
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
        let (view_dimension, array_layer_count) = match desc.dimension {
            CustomTextureDimension::D2 => (gpu::ViewDimension::D2, 1),
            CustomTextureDimension::D2Array { layers } => {
                if layers == 0 {
                    return Err(anyhow::anyhow!(
                        "custom texture array layer count must be non-zero"
                    ));
                }
                (gpu::ViewDimension::D2Array, layers)
            }
            CustomTextureDimension::Cube => {
                if desc.width != desc.height {
                    return Err(anyhow::anyhow!("custom cube textures must be square"));
                }
                (gpu::ViewDimension::Cube, 6)
            }
        };
        for (level, data) in desc.data.iter().enumerate() {
            let (width, height) = mip_level_size(desc.width, desc.height, level as u32);
            let blocks_w = width.div_ceil(block_width);
            let blocks_h = height.div_ceil(block_height);
            let expected_len = blocks_w as u64
                * blocks_h as u64
                * bytes_per_block as u64
                * array_layer_count as u64;
            if data.len() < expected_len as usize {
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
        if is_storage && desc.dimension.is_array() {
            return Err(anyhow::anyhow!("custom storage textures must be 2D"));
        }
        if is_storage && desc.format.is_compressed() {
            return Err(anyhow::anyhow!(
                "custom storage textures must not use compressed formats"
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
            array_layer_count,
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
                dimension: view_dimension,
                subresources: &Default::default(),
            },
        );
        let texture = BladeCustomTexture {
            texture: raw,
            view,
            width: desc.width,
            height: desc.height,
            array_layer_count,
            mip_level_count: desc.data.len() as u32,
            sample_count: 1,
            block_width,
            block_height,
            bytes_per_block,
            format,
            is_render_target: false,
            clear_color: gpu::TextureColor::TransparentBlack,
            msaa_texture: None,
            msaa_view: None,
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
                    bytes_per_row: None,
                },
            )?;
        }
        Ok(CustomTextureId(id))
    }

    fn create_render_target(&self, desc: CustomRenderTargetDesc) -> Result<CustomTextureId> {
        if desc.format.is_compressed() {
            return Err(anyhow::anyhow!(
                "custom render targets must not use compressed formats"
            ));
        }
        let format = blade_color_format(desc.format)?;
        let block_info = desc.format.block_info();
        if desc.sample_count == 0
            || desc.sample_count > crate::MAX_SAMPLE_COUNT
            || !desc.sample_count.is_power_of_two()
        {
            return Err(anyhow::anyhow!(
                "custom draw render target sample count must be a power of two between 1 and {} (got {})",
                crate::MAX_SAMPLE_COUNT,
                desc.sample_count
            ));
        }

        let resolve_texture = self.gpu.create_texture(gpu::TextureDesc {
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
        let resolve_view = self.gpu.create_texture_view(
            resolve_texture,
            gpu::TextureViewDesc {
                name: &desc.name,
                format,
                dimension: gpu::ViewDimension::D2,
                subresources: &Default::default(),
            },
        );

        let (msaa_texture, msaa_view) = if desc.sample_count > 1 {
            let msaa_name = format!("{}_msaa", desc.name.as_str());
            let msaa_texture = self.gpu.create_texture(gpu::TextureDesc {
                name: msaa_name.as_str(),
                format,
                size: gpu::Extent {
                    width: desc.width,
                    height: desc.height,
                    depth: 1,
                },
                array_layer_count: 1,
                mip_level_count: 1,
                sample_count: desc.sample_count,
                dimension: gpu::TextureDimension::D2,
                usage: gpu::TextureUsage::TARGET,
                external: None,
            });
            let msaa_view = self.gpu.create_texture_view(
                msaa_texture,
                gpu::TextureViewDesc {
                    name: msaa_name.as_str(),
                    format,
                    dimension: gpu::ViewDimension::D2,
                    subresources: &Default::default(),
                },
            );
            (Some(msaa_texture), Some(msaa_view))
        } else {
            (None, None)
        };

        let texture = BladeCustomTexture {
            texture: resolve_texture,
            view: resolve_view,
            width: desc.width,
            height: desc.height,
            array_layer_count: 1,
            mip_level_count: 1,
            sample_count: desc.sample_count,
            block_width: block_info.width,
            block_height: block_info.height,
            bytes_per_block: block_info.bytes,
            format,
            is_render_target: true,
            clear_color: blade_clear_color(desc.clear_color),
            msaa_texture,
            msaa_view,
        };
        let mut textures = self.textures.lock().unwrap();
        let id = Self::alloc_slot(&mut textures, texture);
        Ok(CustomTextureId(id))
    }

    fn update_texture(&self, id: CustomTextureId, update: CustomTextureUpdate) -> Result<()> {
        if update.data.is_empty() {
            return Err(anyhow::anyhow!("custom texture data is empty"));
        }
        let (
            is_render_target,
            width,
            height,
            block_width,
            block_height,
            bytes_per_block,
            mip_level_count,
            array_layer_count,
        ) = {
            let textures = self.textures.lock().unwrap();
            let Some(entry) = textures.get(id.0 as usize).and_then(|slot| slot.as_ref()) else {
                return Err(anyhow::anyhow!("custom texture id out of range"));
            };
            (
                entry.is_render_target,
                entry.width,
                entry.height,
                entry.block_width,
                entry.block_height,
                entry.bytes_per_block,
                entry.mip_level_count,
                entry.array_layer_count,
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
        let blocks_w = level_width.div_ceil(block_width);
        let blocks_h = level_height.div_ceil(block_height);
        let packed_bytes_per_row = blocks_w * bytes_per_block;
        let bytes_per_row = update.bytes_per_row.unwrap_or(packed_bytes_per_row);
        if bytes_per_row < packed_bytes_per_row {
            return Err(anyhow::anyhow!(
                "custom texture bytes per row {} is smaller than packed row size {}",
                bytes_per_row,
                packed_bytes_per_row
            ));
        }
        if !bytes_per_row.is_multiple_of(bytes_per_block) {
            return Err(anyhow::anyhow!(
                "custom texture bytes per row {} is not a multiple of texel block size {}",
                bytes_per_row,
                bytes_per_block
            ));
        }
        let bytes_per_image = bytes_per_row as u64 * blocks_h as u64;
        let expected_len = bytes_per_image * array_layer_count as u64;
        if update.data.len() < expected_len as usize {
            return Err(anyhow::anyhow!(
                "custom texture data is smaller than texture size"
            ));
        }
        let bytes_per_image_usize = bytes_per_image as usize;
        let mut upload_belt = self.upload_belt.lock().unwrap();
        let mut pending = self.pending_uploads.lock().unwrap();
        for layer in 0..array_layer_count {
            let start = layer as usize * bytes_per_image_usize;
            let end = start + bytes_per_image_usize;
            let piece = upload_belt.alloc_bytes(&update.data[start..end], &self.gpu);
            pending.push(PendingUpload {
                texture_id: id,
                source: PendingUploadSource::Staging(piece),
                mip_level: update.level,
                array_layer: layer,
                width: level_width,
                height: level_height,
                bytes_per_row,
                bytes_per_image,
            });
        }
        Ok(())
    }

    fn update_texture_from_buffer(
        &self,
        id: CustomTextureId,
        update: CustomTextureBufferUpdate,
    ) -> Result<()> {
        let CustomTextureBufferUpdate {
            level,
            buffer,
            bytes_per_row: bytes_per_row_override,
        } = update;
        let (
            is_render_target,
            width,
            height,
            block_width,
            block_height,
            bytes_per_block,
            mip_level_count,
            array_layer_count,
        ) = {
            let textures = self.textures.lock().unwrap();
            let Some(entry) = textures.get(id.0 as usize).and_then(|slot| slot.as_ref()) else {
                return Err(anyhow::anyhow!("custom texture id out of range"));
            };
            (
                entry.is_render_target,
                entry.width,
                entry.height,
                entry.block_width,
                entry.block_height,
                entry.bytes_per_block,
                entry.mip_level_count,
                entry.array_layer_count,
            )
        };
        if is_render_target {
            return Err(anyhow::anyhow!("custom render targets cannot be updated"));
        }
        if level >= mip_level_count {
            return Err(anyhow::anyhow!(
                "custom texture mip level {} out of range",
                level
            ));
        }
        let (level_width, level_height) = mip_level_size(width, height, level);
        let blocks_w = level_width.div_ceil(block_width);
        let blocks_h = level_height.div_ceil(block_height);
        let packed_bytes_per_row = blocks_w * bytes_per_block;
        let bytes_per_row = bytes_per_row_override.unwrap_or(packed_bytes_per_row);
        if bytes_per_row < packed_bytes_per_row {
            return Err(anyhow::anyhow!(
                "custom texture bytes per row {} is smaller than packed row size {}",
                bytes_per_row,
                packed_bytes_per_row
            ));
        }
        if !bytes_per_row.is_multiple_of(bytes_per_block) {
            return Err(anyhow::anyhow!(
                "custom texture bytes per row {} is not a multiple of texel block size {}",
                bytes_per_row,
                bytes_per_block
            ));
        }
        let bytes_per_image = bytes_per_row as u64 * blocks_h as u64;
        let expected_len = bytes_per_image * array_layer_count as u64;
        let (buffer_id, buffer_offset, buffer_size) = {
            let buffers = self.buffers.lock().unwrap();
            match buffer {
                CustomBufferSource::Buffer(id) => {
                    let Some(entry) = buffers.get(id.0 as usize).and_then(|slot| slot.as_ref())
                    else {
                        return Err(anyhow::anyhow!("custom buffer id out of range"));
                    };
                    (id, 0, entry.size)
                }
                CustomBufferSource::BufferSlice { id, offset, size } => {
                    let Some(entry) = buffers.get(id.0 as usize).and_then(|slot| slot.as_ref())
                    else {
                        return Err(anyhow::anyhow!("custom buffer id out of range"));
                    };
                    if size == 0 {
                        return Err(anyhow::anyhow!("custom texture buffer slice is empty"));
                    }
                    if offset.saturating_add(size) > entry.size {
                        return Err(anyhow::anyhow!("custom texture buffer slice out of range"));
                    }
                    (id, offset, size)
                }
                CustomBufferSource::Inline(_) => {
                    return Err(anyhow::anyhow!(
                        "custom texture buffer updates require a buffer source"
                    ));
                }
            }
        };
        if expected_len > buffer_size {
            return Err(anyhow::anyhow!(
                "custom texture buffer data is smaller than texture size"
            ));
        }
        let mut pending = self.pending_uploads.lock().unwrap();
        for layer in 0..array_layer_count {
            let offset = buffer_offset + bytes_per_image * layer as u64;
            pending.push(PendingUpload {
                texture_id: id,
                source: PendingUploadSource::Buffer {
                    id: buffer_id,
                    offset,
                },
                mip_level: level,
                array_layer: layer,
                width: level_width,
                height: level_height,
                bytes_per_row,
                bytes_per_image,
            });
        }
        Ok(())
    }

    fn remove_texture(&self, id: CustomTextureId) {
        let mut textures = self.textures.lock().unwrap();
        if let Some(slot) = textures.get_mut(id.0 as usize) {
            if let Some(texture) = slot.take() {
                self.gpu.destroy_texture_view(texture.view);
                self.gpu.destroy_texture(texture.texture);
                if let Some(msaa_view) = texture.msaa_view {
                    self.gpu.destroy_texture_view(msaa_view);
                }
                if let Some(msaa_texture) = texture.msaa_texture {
                    self.gpu.destroy_texture(msaa_texture);
                }
            }
        }
    }

    fn create_depth_target(&self, desc: CustomDepthTargetDesc) -> Result<CustomDepthTargetId> {
        let format = blade_depth_format(desc.format);
        if desc.sample_count == 0
            || desc.sample_count > crate::MAX_SAMPLE_COUNT
            || !desc.sample_count.is_power_of_two()
        {
            return Err(anyhow::anyhow!(
                "custom draw depth target sample count must be a power of two between 1 and {} (got {})",
                crate::MAX_SAMPLE_COUNT,
                desc.sample_count
            ));
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
            mip_level_count: 1,
            sample_count: desc.sample_count,
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
            format,
            width: desc.width,
            height: desc.height,
            sample_count: desc.sample_count,
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

struct PushConstantsInfo {
    name: &'static str,
    size: u32,
    slot: CustomBindingSlot,
}

fn push_constants_slot(bindings: &[CustomBindingDesc]) -> CustomBindingSlot {
    let mut max_group = 0u32;
    for binding in bindings {
        let slot = binding.slot.unwrap_or(CustomBindingSlot {
            group: 0,
            binding: binding.name.index(),
        });
        max_group = max_group.max(slot.group);
    }
    CustomBindingSlot {
        group: max_group.saturating_add(1),
        binding: 0,
    }
}

fn naga_capabilities(bindings: &[CustomBindingDesc]) -> naga::valid::Capabilities {
    let mut capabilities = naga::valid::Capabilities::empty();
    for binding in bindings {
        match binding.kind {
            CustomBindingKind::BufferArray { .. } | CustomBindingKind::TextureArray { .. } => {
                capabilities |=
                    naga::valid::Capabilities::SAMPLED_TEXTURE_AND_STORAGE_BUFFER_ARRAY_NON_UNIFORM_INDEXING;
            }
            CustomBindingKind::StorageTextureArray { .. } => {
                capabilities |=
                    naga::valid::Capabilities::STORAGE_TEXTURE_ARRAY_NON_UNIFORM_INDEXING;
            }
            _ => {}
        }
    }
    capabilities
}

fn prepare_blade_shader_source(
    source: &str,
    entry_points: &[&str],
    bindings: &[CustomBindingDesc],
    push_constants: Option<CustomPushConstantsDesc>,
    push_constants_slot: CustomBindingSlot,
) -> Result<(String, Option<PushConstantsInfo>)> {
    let mut module = naga::front::wgsl::parse_str(source)
        .map_err(|err| anyhow::anyhow!("WGSL parse failed: {err}"))?;
    let flags = naga::valid::ValidationFlags::all() ^ naga::valid::ValidationFlags::BINDINGS;
    let capabilities = naga_capabilities(bindings);
    let mut validator = naga::valid::Validator::new(flags, capabilities);
    let info = validator
        .validate(&module)
        .map_err(|err| anyhow::anyhow!("WGSL validation failed: {err}"))?;

    let mut entry_indices = Vec::with_capacity(entry_points.len());
    for entry_point in entry_points {
        let index = module
            .entry_points
            .iter()
            .position(|entry| entry.name == *entry_point)
            .ok_or_else(|| anyhow::anyhow!("entry '{}' not found", entry_point))?;
        entry_indices.push(index);
    }

    let mut push_constant_handle = None;
    for (handle, var) in module.global_variables.iter() {
        if var.space != naga::AddressSpace::PushConstant {
            continue;
        }
        let used = entry_indices.iter().any(|index| {
            let ep_info = info.get_entry_point(*index);
            !ep_info[handle].is_empty()
        });
        if !used {
            continue;
        }
        if push_constant_handle.is_some() {
            return Err(anyhow::anyhow!(
                "custom draw shaders may declare at most one push constants block"
            ));
        }
        push_constant_handle = Some(handle);
    }

    let Some(handle) = push_constant_handle else {
        if push_constants.is_some() {
            return Err(anyhow::anyhow!(
                "push constants were provided but the shader has no push constant block"
            ));
        }
        return Ok((source.to_string(), None));
    };

    let push_constants = push_constants
        .ok_or_else(|| anyhow::anyhow!("shader declares push constants but none were provided"))?;

    let mut layouter = naga::proc::Layouter::default();
    layouter
        .update(module.to_ctx())
        .map_err(|err| anyhow::anyhow!("push constants layout failed: {err}"))?;
    let layout = &layouter[module.global_variables[handle].ty];
    if layout.size != push_constants.size {
        return Err(anyhow::anyhow!(
            "push constants size mismatch (expected {}, shader reports {})",
            push_constants.size,
            layout.size
        ));
    }

    let var = module.global_variables.get_mut(handle);
    var.space = naga::AddressSpace::Uniform;
    var.binding = None;
    if var.name.is_none() {
        var.name = Some("push_constants".to_string());
    }
    let name = var
        .name
        .clone()
        .unwrap_or_else(|| "push_constants".to_string());

    let info = validator
        .validate(&module)
        .map_err(|err| anyhow::anyhow!("WGSL validation failed: {err}"))?;
    let source =
        naga::back::wgsl::write_string(&module, &info, naga::back::wgsl::WriterFlags::empty())
            .map_err(|err| anyhow::anyhow!("WGSL serialization failed: {err}"))?;

    Ok((
        source,
        Some(PushConstantsInfo {
            name: Box::leak(name.into_boxed_str()),
            size: push_constants.size,
            slot: push_constants_slot,
        }),
    ))
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

fn blade_color_format(format: CustomTextureFormat) -> Result<gpu::TextureFormat> {
    match format {
        CustomTextureFormat::Rgba8Unorm => Ok(gpu::TextureFormat::Rgba8Unorm),
        CustomTextureFormat::Bgra8Unorm => Ok(gpu::TextureFormat::Bgra8Unorm),
        CustomTextureFormat::Rgba8UnormSrgb => Ok(gpu::TextureFormat::Rgba8UnormSrgb),
        CustomTextureFormat::Bgra8UnormSrgb => Ok(gpu::TextureFormat::Bgra8UnormSrgb),
        CustomTextureFormat::Bc1Unorm => Ok(gpu::TextureFormat::Bc1Unorm),
        CustomTextureFormat::Bc1UnormSrgb => Ok(gpu::TextureFormat::Bc1UnormSrgb),
        CustomTextureFormat::Bc3Unorm => Ok(gpu::TextureFormat::Bc3Unorm),
        CustomTextureFormat::Bc3UnormSrgb => Ok(gpu::TextureFormat::Bc3UnormSrgb),
        CustomTextureFormat::Bc7Unorm => Ok(gpu::TextureFormat::Bc7Unorm),
        CustomTextureFormat::Bc7UnormSrgb => Ok(gpu::TextureFormat::Bc7UnormSrgb),
        CustomTextureFormat::Etc2Rgb8Unorm
        | CustomTextureFormat::Etc2Rgb8UnormSrgb
        | CustomTextureFormat::Etc2Rgba8Unorm
        | CustomTextureFormat::Etc2Rgba8UnormSrgb
        | CustomTextureFormat::Astc4x4Unorm
        | CustomTextureFormat::Astc4x4UnormSrgb => Err(anyhow::anyhow!(
            "custom texture format {:?} is not supported by Blade",
            format
        )),
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

fn texture_level_estimate_bytes(
    width: u32,
    height: u32,
    array_layer_count: u32,
    block_width: u32,
    block_height: u32,
    bytes_per_block: u32,
) -> u64 {
    let blocks_w = width.div_ceil(block_width);
    let blocks_h = height.div_ceil(block_height);
    (blocks_w as u64)
        .saturating_mul(blocks_h as u64)
        .saturating_mul(bytes_per_block as u64)
        .saturating_mul(array_layer_count as u64)
}

fn texture_mip_chain_estimate_bytes(
    width: u32,
    height: u32,
    array_layer_count: u32,
    mip_level_count: u32,
    block_width: u32,
    block_height: u32,
    bytes_per_block: u32,
) -> u64 {
    let mut total_bytes = 0u64;
    for level in 0..mip_level_count {
        let (level_width, level_height) = mip_level_size(width, height, level);
        total_bytes = total_bytes.saturating_add(texture_level_estimate_bytes(
            level_width,
            level_height,
            array_layer_count,
            block_width,
            block_height,
            bytes_per_block,
        ));
    }
    total_bytes
}

fn depth_target_estimate_bytes(width: u32, height: u32, sample_count: u32) -> u64 {
    (width.max(1) as u64)
        .saturating_mul(height.max(1) as u64)
        .saturating_mul(4)
        .saturating_mul(sample_count.max(1) as u64)
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
                    CustomBindingKind::BufferArray { count },
                    CustomBindingValue::BufferArray(sources),
                ) => {
                    if sources.len() != *count as usize {
                        log::warn!(
                            "custom draw buffer array length mismatch (expected {}, got {})",
                            count,
                            sources.len()
                        );
                        continue;
                    }
                    let mut pieces = Vec::with_capacity(*count as usize);
                    let mut missing = false;
                    for source in sources {
                        if let Some(piece) =
                            resolve_buffer_piece(source, self.buffers, instance_belt, gpu)
                        {
                            pieces.push(piece);
                        } else {
                            missing = true;
                            break;
                        }
                    }
                    if missing {
                        continue;
                    }
                    bind_buffer_array(&mut context, i as u32, *count, &pieces);
                }
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
                (
                    CustomBindingKind::TextureArray { count }
                    | CustomBindingKind::StorageTextureArray { count },
                    CustomBindingValue::TextureArray(ids),
                ) => {
                    if ids.len() != *count as usize {
                        log::warn!(
                            "custom draw texture array length mismatch (expected {}, got {})",
                            count,
                            ids.len()
                        );
                        continue;
                    }
                    let mut views = Vec::with_capacity(*count as usize);
                    let mut missing = false;
                    for id in ids {
                        if let Some(view) = self
                            .textures
                            .get(id.0 as usize)
                            .and_then(|slot| slot.as_ref())
                        {
                            views.push(*view);
                        } else {
                            log::warn!("custom draw texture {:?} missing", id.0);
                            missing = true;
                            break;
                        }
                    }
                    if missing {
                        continue;
                    }
                    bind_texture_array(&mut context, i as u32, *count, &views);
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

fn resolve_buffer_piece(
    source: &CustomBufferSource,
    buffers: &[Option<BladeBufferSnapshot>],
    instance_belt: &mut BufferBelt,
    gpu: &gpu::Context,
) -> Option<gpu::BufferPiece> {
    match source {
        CustomBufferSource::Inline(data) => Some(instance_belt.alloc_bytes(data, gpu)),
        CustomBufferSource::Buffer(id) => {
            if let Some(buffer) = buffers.get(id.0 as usize).and_then(|slot| slot.as_ref()) {
                Some(gpu::BufferPiece::from(buffer.buffer))
            } else {
                log::warn!("custom draw buffer {:?} missing", id.0);
                None
            }
        }
        CustomBufferSource::BufferSlice { id, offset, size } => {
            if let Some(buffer) = buffers.get(id.0 as usize).and_then(|slot| slot.as_ref()) {
                if *size == 0 {
                    log::warn!("custom draw buffer slice is empty");
                    None
                } else if offset.saturating_add(*size) > buffer.size {
                    log::warn!("custom draw buffer slice out of range");
                    None
                } else {
                    Some(gpu::BufferPiece {
                        buffer: buffer.buffer,
                        offset: *offset,
                    })
                }
            } else {
                log::warn!("custom draw buffer {:?} missing", id.0);
                None
            }
        }
    }
}

fn bind_buffer_array(
    context: &mut gpu::PipelineContext,
    index: u32,
    count: u32,
    pieces: &[gpu::BufferPiece],
) {
    match count {
        1 => bind_buffer_array_const::<1>(context, index, pieces),
        2 => bind_buffer_array_const::<2>(context, index, pieces),
        3 => bind_buffer_array_const::<3>(context, index, pieces),
        4 => bind_buffer_array_const::<4>(context, index, pieces),
        5 => bind_buffer_array_const::<5>(context, index, pieces),
        6 => bind_buffer_array_const::<6>(context, index, pieces),
        7 => bind_buffer_array_const::<7>(context, index, pieces),
        8 => bind_buffer_array_const::<8>(context, index, pieces),
        9 => bind_buffer_array_const::<9>(context, index, pieces),
        10 => bind_buffer_array_const::<10>(context, index, pieces),
        11 => bind_buffer_array_const::<11>(context, index, pieces),
        12 => bind_buffer_array_const::<12>(context, index, pieces),
        13 => bind_buffer_array_const::<13>(context, index, pieces),
        14 => bind_buffer_array_const::<14>(context, index, pieces),
        15 => bind_buffer_array_const::<15>(context, index, pieces),
        16 => bind_buffer_array_const::<16>(context, index, pieces),
        _ => log::warn!("custom draw buffer array count {} unsupported", count),
    }
}

fn bind_buffer_array_const<const N: gpu::ResourceIndex>(
    context: &mut gpu::PipelineContext,
    index: u32,
    pieces: &[gpu::BufferPiece],
) {
    let mut array = gpu::BufferArray::<N>::new();
    for piece in pieces.iter().take(N as usize) {
        array.alloc(*piece);
    }
    (&array).bind_to(context, index);
}

fn bind_texture_array(
    context: &mut gpu::PipelineContext,
    index: u32,
    count: u32,
    views: &[gpu::TextureView],
) {
    match count {
        1 => bind_texture_array_const::<1>(context, index, views),
        2 => bind_texture_array_const::<2>(context, index, views),
        3 => bind_texture_array_const::<3>(context, index, views),
        4 => bind_texture_array_const::<4>(context, index, views),
        5 => bind_texture_array_const::<5>(context, index, views),
        6 => bind_texture_array_const::<6>(context, index, views),
        7 => bind_texture_array_const::<7>(context, index, views),
        8 => bind_texture_array_const::<8>(context, index, views),
        9 => bind_texture_array_const::<9>(context, index, views),
        10 => bind_texture_array_const::<10>(context, index, views),
        11 => bind_texture_array_const::<11>(context, index, views),
        12 => bind_texture_array_const::<12>(context, index, views),
        13 => bind_texture_array_const::<13>(context, index, views),
        14 => bind_texture_array_const::<14>(context, index, views),
        15 => bind_texture_array_const::<15>(context, index, views),
        16 => bind_texture_array_const::<16>(context, index, views),
        _ => log::warn!("custom draw texture array count {} unsupported", count),
    }
}

fn bind_texture_array_const<const N: gpu::ResourceIndex>(
    context: &mut gpu::PipelineContext,
    index: u32,
    views: &[gpu::TextureView],
) {
    let mut array = gpu::TextureArray::<N>::new();
    for view in views.iter().take(N as usize) {
        array.alloc(*view);
    }
    (&array).bind_to(context, index);
}
