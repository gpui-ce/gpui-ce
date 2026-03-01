use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use anyhow::anyhow;
use metal::{self, MTLResourceOptions};

use crate::{
    CustomAddressMode, CustomBindingDesc, CustomBindingKind, CustomBindingName, CustomBindingSlot,
    CustomBlendMode, CustomBufferDesc, CustomBufferId, CustomComputePipelineDesc,
    CustomComputePipelineId, CustomCullMode, CustomDepthCompare, CustomDepthFormat,
    CustomDepthState, CustomDepthTargetDesc, CustomDepthTargetId, CustomDrawRegistry,
    CustomFilterMode, CustomFrontFace, CustomPipelineDesc, CustomPipelineId, CustomPipelineState,
    CustomPrimitiveTopology, CustomPushConstantsDesc, CustomRenderTargetDesc, CustomSamplerDesc,
    CustomSamplerId, CustomTextureDesc, CustomTextureDimension, CustomTextureFormat,
    CustomTextureId, CustomTextureUpdate, CustomTextureUsage, CustomVertexAttribute,
    CustomVertexAttributeName, CustomVertexFetch, CustomVertexFormat, Result,
};

pub(crate) struct MetalCustomDrawRegistry {
    device: metal::Device,
    pixel_format: metal::MTLPixelFormat,
    pipelines: Mutex<Vec<Option<MetalCustomPipeline>>>,
    compute_pipelines: Mutex<Vec<Option<MetalCustomComputePipeline>>>,
    pipeline_cache: Mutex<HashMap<PipelineCacheKey, CustomPipelineId>>,
    buffers: Mutex<Vec<Option<MetalCustomBuffer>>>,
    textures: Mutex<Vec<Option<MetalCustomTexture>>>,
    depth_targets: Mutex<Vec<Option<MetalCustomDepthTarget>>>,
    samplers: Mutex<Vec<Option<metal::SamplerState>>>,
}

unsafe impl Send for MetalCustomDrawRegistry {}
unsafe impl Sync for MetalCustomDrawRegistry {}

pub(crate) struct MetalCustomPipeline {
    pub(crate) pipeline_state: metal::RenderPipelineState,
    pub(crate) bindings: Vec<CustomBindingKind>,
    pub(crate) argument_buffers: Vec<Option<ArgumentBufferBinding>>,
    pub(crate) primitive: metal::MTLPrimitiveType,
    pub(crate) cull_mode: metal::MTLCullMode,
    pub(crate) front_face: metal::MTLWinding,
    pub(crate) color_format: metal::MTLPixelFormat,
    pub(crate) depth_format: Option<metal::MTLPixelFormat>,
    pub(crate) depth_state: Option<metal::DepthStencilState>,
    pub(crate) vertex_fetch_count: usize,
    pub(crate) buffer_binding_base: u64,
}

pub(crate) struct MetalCustomComputePipeline {
    pub(crate) pipeline_state: metal::ComputePipelineState,
    pub(crate) bindings: Vec<CustomBindingKind>,
    pub(crate) argument_buffers: Vec<Option<ArgumentBufferBinding>>,
    pub(crate) workgroup_size: [u32; 3],
    pub(crate) buffer_binding_base: u64,
}

pub(crate) struct ArgumentBufferBinding {
    pub(crate) encoder: metal::ArgumentEncoder,
}

#[derive(Hash, Eq, PartialEq)]
enum PipelineSourceKey {
    Wgsl(String),
    Msl(String),
}

#[derive(Hash, Eq, PartialEq)]
struct PipelineCacheKey {
    source: PipelineSourceKey,
    vertex_entry: String,
    fragment_entry: String,
    primitive: u8,
    color_format: u64,
    state: PipelineStateKey,
    vertex_fetches: Vec<VertexFetchKey>,
    push_constants: Option<u32>,
    bindings: Vec<BindingKey>,
}

#[derive(Hash, Eq, PartialEq)]
struct VertexFetchKey {
    stride: u32,
    instanced: bool,
    attributes: Vec<VertexAttributeKey>,
}

#[derive(Hash, Eq, PartialEq)]
struct VertexAttributeKey {
    name: u8,
    offset: u32,
    format: u8,
    location: Option<u32>,
}

#[derive(Hash, Eq, PartialEq)]
struct BindingKey {
    name: u8,
    kind: BindingKindKey,
    slot: Option<CustomBindingSlot>,
}

#[derive(Hash, Eq, PartialEq)]
struct BindingKindKey {
    kind: u8,
    size: u32,
    count: u32,
}

struct PushConstantsInfo {
    name: &'static str,
    size: u32,
    slot: CustomBindingSlot,
}

#[derive(Hash, Eq, PartialEq)]
struct PipelineStateKey {
    blend: u8,
    cull_mode: u8,
    front_face: u8,
    depth_format: u8,
    depth_compare: u8,
    depth_write: u8,
}

struct MetalCustomBuffer {
    buffer: metal::Buffer,
    size: u64,
}

pub(crate) struct MetalBufferSnapshot {
    pub(crate) buffer: metal::Buffer,
    pub(crate) size: u64,
}

pub(crate) struct MetalCustomTexture {
    pub(crate) texture: metal::Texture,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) array_layer_count: u32,
    pub(crate) mip_level_count: u32,
    pub(crate) bytes_per_pixel: u32,
    pub(crate) format: metal::MTLPixelFormat,
    pub(crate) is_render_target: bool,
    pub(crate) clear_color: [f32; 4],
}

pub(crate) struct MetalCustomDepthTarget {
    pub(crate) texture: metal::Texture,
    pub(crate) format: metal::MTLPixelFormat,
    pub(crate) clear_depth: f64,
}

impl MetalCustomDrawRegistry {
    pub(crate) fn new(device: metal::Device, pixel_format: metal::MTLPixelFormat) -> Self {
        Self {
            device,
            pixel_format,
            pipelines: Mutex::new(Vec::new()),
            compute_pipelines: Mutex::new(Vec::new()),
            pipeline_cache: Mutex::new(HashMap::new()),
            buffers: Mutex::new(Vec::new()),
            textures: Mutex::new(Vec::new()),
            depth_targets: Mutex::new(Vec::new()),
            samplers: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn with_pipeline<F, R>(&self, id: CustomPipelineId, f: F) -> Option<R>
    where
        F: FnOnce(&MetalCustomPipeline) -> R,
    {
        let pipelines = self.pipelines.lock().unwrap();
        let entry = pipelines.get(id.0 as usize)?.as_ref()?;
        Some(f(entry))
    }

    pub(crate) fn with_compute_pipeline<F, R>(&self, id: CustomComputePipelineId, f: F) -> Option<R>
    where
        F: FnOnce(&MetalCustomComputePipeline) -> R,
    {
        let pipelines = self.compute_pipelines.lock().unwrap();
        let entry = pipelines.get(id.0 as usize)?.as_ref()?;
        Some(f(entry))
    }

    pub(crate) fn with_texture<F, R>(&self, id: CustomTextureId, f: F) -> Option<R>
    where
        F: FnOnce(&MetalCustomTexture) -> R,
    {
        let textures = self.textures.lock().unwrap();
        let entry = textures.get(id.0 as usize)?.as_ref()?;
        Some(f(entry))
    }

    pub(crate) fn with_depth_target<F, R>(&self, id: CustomDepthTargetId, f: F) -> Option<R>
    where
        F: FnOnce(&MetalCustomDepthTarget) -> R,
    {
        let depth_targets = self.depth_targets.lock().unwrap();
        let entry = depth_targets.get(id.0 as usize)?.as_ref()?;
        Some(f(entry))
    }

    pub(crate) fn buffers_snapshot(&self) -> Vec<Option<MetalBufferSnapshot>> {
        self.buffers
            .lock()
            .unwrap()
            .iter()
            .map(|slot| {
                slot.as_ref().map(|entry| MetalBufferSnapshot {
                    buffer: entry.buffer.clone(),
                    size: entry.size,
                })
            })
            .collect()
    }

    pub(crate) fn textures_snapshot(&self) -> Vec<Option<metal::Texture>> {
        self.textures
            .lock()
            .unwrap()
            .iter()
            .map(|slot| slot.as_ref().map(|entry| entry.texture.clone()))
            .collect()
    }

    pub(crate) fn samplers_snapshot(&self) -> Vec<Option<metal::SamplerState>> {
        self.samplers
            .lock()
            .unwrap()
            .iter()
            .map(|slot| slot.as_ref().cloned())
            .collect()
    }

    pub(crate) fn surface_format(&self) -> metal::MTLPixelFormat {
        self.pixel_format
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

    fn create_pipeline_internal(
        &self,
        desc: CustomPipelineDesc,
        msl_source: Option<String>,
    ) -> Result<CustomPipelineId> {
        let source_key = match &msl_source {
            Some(source) => PipelineSourceKey::Msl(source.clone()),
            None => PipelineSourceKey::Wgsl(desc.shader_source.clone()),
        };
        let color_format = resolve_color_format(desc.target_format, self.pixel_format)?;
        let cache_key = pipeline_cache_key(&desc, source_key, color_format);
        if let Some(existing) = self.pipeline_cache.lock().unwrap().get(&cache_key).copied() {
            return Ok(existing);
        }

        let has_binding_arrays = desc.bindings.iter().any(|binding| {
            matches!(
                binding.kind,
                CustomBindingKind::BufferArray { .. }
                    | CustomBindingKind::TextureArray { .. }
                    | CustomBindingKind::StorageTextureArray { .. }
            )
        });
        let msl_lang_version = if has_binding_arrays { (2, 0) } else { (1, 2) };

        let mut module = naga::front::wgsl::parse_str(&desc.shader_source)
            .map_err(|err| anyhow!("WGSL parse failed: {err}"))?;
        let flags = naga::valid::ValidationFlags::all() ^ naga::valid::ValidationFlags::BINDINGS;
        let capabilities = naga_capabilities(&desc.bindings);
        let mut info = naga::valid::Validator::new(flags, capabilities)
            .validate(&module)
            .map_err(|err| anyhow!("WGSL validation failed: {err}"))?;

        let vertex_entry_index = module
            .entry_points
            .iter()
            .position(|entry| entry.name == desc.vertex_entry)
            .ok_or_else(|| anyhow!("vertex entry '{}' not found", desc.vertex_entry))?;
        let fragment_entry_index = module
            .entry_points
            .iter()
            .position(|entry| entry.name == desc.fragment_entry)
            .ok_or_else(|| anyhow!("fragment entry '{}' not found", desc.fragment_entry))?;

        let push_constants_slot = push_constants_slot(&desc.bindings);
        let push_constants = apply_push_constants(
            &mut module,
            &info,
            &[vertex_entry_index, fragment_entry_index],
            desc.push_constants,
            push_constants_slot,
        )?;
        if push_constants.is_some() {
            info = naga::valid::Validator::new(flags, capabilities)
                .validate(&module)
                .map_err(|err| anyhow!("WGSL validation failed: {err}"))?;
        }

        let attribute_locations = build_attribute_locations(&desc.vertex_fetches)?;
        assign_vertex_locations(&mut module, vertex_entry_index, &attribute_locations)?;

        let (mut bindings_by_name, bindings_by_slot) = build_binding_maps(&desc.bindings);
        if let Some(push_constants) = &push_constants {
            if bindings_by_name.contains_key(push_constants.name) {
                return Err(anyhow!(
                    "custom draw push constants name '{}' conflicts with a binding name",
                    push_constants.name
                ));
            }
            bindings_by_name.insert(
                push_constants.name,
                BindingInfo {
                    kind: CustomBindingKind::Uniform {
                        size: push_constants.size,
                    },
                    slot: push_constants.slot,
                },
            );
        }
        let vertex_entry_name = module.entry_points[vertex_entry_index].name.clone();
        let fragment_entry_name = module.entry_points[fragment_entry_index].name.clone();
        assign_resource_bindings(
            &mut module,
            &info,
            &vertex_entry_name,
            vertex_entry_index,
            &bindings_by_name,
            &bindings_by_slot,
        )?;
        assign_resource_bindings(
            &mut module,
            &info,
            &fragment_entry_name,
            fragment_entry_index,
            &bindings_by_name,
            &bindings_by_slot,
        )?;

        let binding_array_handles = collect_binding_array_handles(&module);
        let vertex_usage = info.get_entry_point(vertex_entry_index);
        let fragment_usage = info.get_entry_point(fragment_entry_index);

        let buffer_binding_base = u8::try_from(desc.vertex_fetches.len())
            .map_err(|_| anyhow!("custom draw supports up to {} vertex buffers", u8::MAX))?;

        let (msl_source, vertex_name, fragment_name) = if let Some(source) = msl_source {
            if source.trim().is_empty() {
                return Err(anyhow!("MSL source is empty"));
            }
            (
                source,
                desc.vertex_entry.clone(),
                desc.fragment_entry.clone(),
            )
        } else {
            let mut entry_point_resources =
                build_entry_point_resources(&desc.bindings, buffer_binding_base)?;
            if let Some(push_constants) = &push_constants {
                let binding_index = u8::try_from(desc.bindings.len())
                    .map_err(|_| anyhow!("custom draw binding index exceeds Metal slot limit"))?;
                let buffer_slot = buffer_binding_base
                    .checked_add(binding_index)
                    .ok_or_else(|| anyhow!("custom draw push constants slot overflow"))?;
                entry_point_resources.resources.insert(
                    naga::ResourceBinding {
                        group: push_constants.slot.group,
                        binding: push_constants.slot.binding,
                    },
                    naga::back::msl::BindTarget {
                        buffer: Some(buffer_slot),
                        ..Default::default()
                    },
                );
            }
            normalize_binding_array_address_space(&mut module);
            let mut naga_options = naga::back::msl::Options::default();
            naga_options.lang_version = msl_lang_version;
            naga_options.fake_missing_bindings = false;
            naga_options.zero_initialize_workgroup_memory = false;
            naga_options.force_loop_bounding = false;
            naga_options
                .per_entry_point_map
                .insert(desc.vertex_entry.clone(), entry_point_resources.clone());
            naga_options
                .per_entry_point_map
                .insert(desc.fragment_entry.clone(), entry_point_resources);

            let pipeline_options = naga::back::msl::PipelineOptions {
                allow_and_force_point_size: matches!(
                    desc.primitive,
                    CustomPrimitiveTopology::PointList
                ),
                vertex_pulling_transform: false,
                vertex_buffer_mappings: Vec::new(),
            };

            let (msl_source, translation) =
                naga::back::msl::write_string(&module, &info, &naga_options, &pipeline_options)
                    .map_err(|err| anyhow!("MSL translation failed: {err}"))?;

            let vertex_name = translation
                .entry_point_names
                .get(vertex_entry_index)
                .ok_or_else(|| anyhow!("missing translated vertex entry"))
                .and_then(|result| result.as_ref().map_err(|err| anyhow!("{err}")))?
                .to_string();
            let fragment_name = translation
                .entry_point_names
                .get(fragment_entry_index)
                .ok_or_else(|| anyhow!("missing translated fragment entry"))
                .and_then(|result| result.as_ref().map_err(|err| anyhow!("{err}")))?
                .to_string();

            (msl_source, vertex_name, fragment_name)
        };

        let compile_options = metal::CompileOptions::new();
        compile_options.set_language_version(if has_binding_arrays {
            metal::MTLLanguageVersion::V2_0
        } else {
            metal::MTLLanguageVersion::V1_2
        });
        let library = self
            .device
            .new_library_with_source(&msl_source, &compile_options)
            .map_err(|err| anyhow!("MSL compilation failed: {err}"))?;

        let vertex_fn = library
            .get_function(&vertex_name, None)
            .map_err(|err| anyhow!("vertex entry '{vertex_name}' not found: {err}"))?;
        let fragment_fn = library
            .get_function(&fragment_name, None)
            .map_err(|err| anyhow!("fragment entry '{fragment_name}' not found: {err}"))?;

        let mut argument_buffers =
            Vec::with_capacity(desc.bindings.len() + usize::from(push_constants.is_some()));
        for (index, binding) in desc.bindings.iter().enumerate() {
            let argument_buffer = match binding.kind {
                CustomBindingKind::BufferArray { .. }
                | CustomBindingKind::TextureArray { .. }
                | CustomBindingKind::StorageTextureArray { .. } => {
                    let slot = binding.slot.unwrap_or(CustomBindingSlot {
                        group: 0,
                        binding: binding.name.index(),
                    });
                    let handle = binding_array_handles
                        .get(&(slot.group, slot.binding))
                        .ok_or_else(|| {
                            anyhow!(
                                "binding array '{}' not found in shader",
                                binding.name.as_str()
                            )
                        })?;
                    let vertex_used = !vertex_usage[*handle].is_empty();
                    let fragment_used = !fragment_usage[*handle].is_empty();
                    let buffer_slot = u64::from(buffer_binding_base) + index as u64;
                    let encoder = if vertex_used {
                        vertex_fn.new_argument_encoder(buffer_slot as metal::NSUInteger)
                    } else if fragment_used {
                        fragment_fn.new_argument_encoder(buffer_slot as metal::NSUInteger)
                    } else {
                        return Err(anyhow!(
                            "binding array '{}' is not used by the shader",
                            binding.name.as_str()
                        ));
                    };
                    Some(ArgumentBufferBinding { encoder })
                }
                _ => None,
            };
            argument_buffers.push(argument_buffer);
        }
        if push_constants.is_some() {
            argument_buffers.push(None);
        }

        let vertex_descriptor =
            build_vertex_descriptor(&desc.vertex_fetches, &attribute_locations)?;

        let pipeline_descriptor = metal::RenderPipelineDescriptor::new();
        pipeline_descriptor.set_label(&desc.name);
        pipeline_descriptor.set_vertex_function(Some(vertex_fn.as_ref()));
        pipeline_descriptor.set_fragment_function(Some(fragment_fn.as_ref()));
        pipeline_descriptor.set_vertex_descriptor(Some(vertex_descriptor));

        let color_attachment = pipeline_descriptor
            .color_attachments()
            .object_at(0)
            .ok_or_else(|| anyhow!("missing color attachment"))?;
        apply_blend_state(color_attachment, color_format, desc.state.blend);

        let (depth_state, depth_format) = if let Some(depth_state) = desc.state.depth {
            let depth_format = metal_depth_format(depth_state.format);
            pipeline_descriptor.set_depth_attachment_pixel_format(depth_format);
            (
                Some(create_depth_state(&self.device, depth_state)),
                Some(depth_format),
            )
        } else {
            (None, None)
        };

        let pipeline_state = self
            .device
            .new_render_pipeline_state(&pipeline_descriptor)
            .map_err(|err| anyhow!("custom draw pipeline failed: {err}"))?;

        let mut binding_kinds: Vec<CustomBindingKind> =
            desc.bindings.iter().map(|binding| binding.kind).collect();
        if let Some(push_constants) = &push_constants {
            binding_kinds.push(CustomBindingKind::Uniform {
                size: push_constants.size,
            });
        }
        let pipeline = MetalCustomPipeline {
            pipeline_state,
            bindings: binding_kinds,
            argument_buffers,
            primitive: metal_primitive(desc.primitive),
            cull_mode: metal_cull_mode(desc.state.cull_mode),
            front_face: metal_front_face(desc.state.front_face),
            color_format,
            depth_format,
            depth_state,
            vertex_fetch_count: desc.vertex_fetches.len(),
            buffer_binding_base: buffer_binding_base as u64,
        };

        let mut pipelines = self.pipelines.lock().unwrap();
        let id = Self::alloc_slot(&mut pipelines, pipeline);
        let pipeline_id = CustomPipelineId(id);
        self.pipeline_cache
            .lock()
            .unwrap()
            .insert(cache_key, pipeline_id);
        Ok(pipeline_id)
    }

    fn create_compute_pipeline_internal(
        &self,
        desc: CustomComputePipelineDesc,
    ) -> Result<CustomComputePipelineId> {
        let has_binding_arrays = desc.bindings.iter().any(|binding| {
            matches!(
                binding.kind,
                CustomBindingKind::BufferArray { .. }
                    | CustomBindingKind::TextureArray { .. }
                    | CustomBindingKind::StorageTextureArray { .. }
            )
        });
        let msl_lang_version = if has_binding_arrays { (2, 0) } else { (1, 2) };

        let mut module = naga::front::wgsl::parse_str(&desc.shader_source)
            .map_err(|err| anyhow!("WGSL parse failed: {err}"))?;
        let flags = naga::valid::ValidationFlags::all() ^ naga::valid::ValidationFlags::BINDINGS;
        let capabilities = naga_capabilities(&desc.bindings);
        let mut info = naga::valid::Validator::new(flags, capabilities)
            .validate(&module)
            .map_err(|err| anyhow!("WGSL validation failed: {err}"))?;

        let entry_index = module
            .entry_points
            .iter()
            .position(|entry| entry.name == desc.entry_point)
            .ok_or_else(|| anyhow!("compute entry '{}' not found", desc.entry_point))?;
        let entry = &module.entry_points[entry_index];
        if entry.stage != naga::ShaderStage::Compute {
            return Err(anyhow!(
                "entry '{}' is not a compute shader",
                desc.entry_point
            ));
        }
        if let Some(overrides) = entry.workgroup_size_overrides.as_ref() {
            if overrides
                .iter()
                .any(|override_expr| override_expr.is_some())
            {
                return Err(anyhow!(
                    "custom compute pipelines do not support workgroup size overrides"
                ));
            }
        }
        let workgroup_size = entry.workgroup_size;

        let push_constants_slot = push_constants_slot(&desc.bindings);
        let push_constants = apply_push_constants(
            &mut module,
            &info,
            &[entry_index],
            desc.push_constants,
            push_constants_slot,
        )?;
        if push_constants.is_some() {
            info = naga::valid::Validator::new(flags, capabilities)
                .validate(&module)
                .map_err(|err| anyhow!("WGSL validation failed: {err}"))?;
        }

        let (mut bindings_by_name, bindings_by_slot) = build_binding_maps(&desc.bindings);
        if let Some(push_constants) = &push_constants {
            if bindings_by_name.contains_key(push_constants.name) {
                return Err(anyhow!(
                    "custom compute push constants name '{}' conflicts with a binding name",
                    push_constants.name
                ));
            }
            bindings_by_name.insert(
                push_constants.name,
                BindingInfo {
                    kind: CustomBindingKind::Uniform {
                        size: push_constants.size,
                    },
                    slot: push_constants.slot,
                },
            );
        }
        let entry_name = module.entry_points[entry_index].name.clone();
        assign_resource_bindings(
            &mut module,
            &info,
            &entry_name,
            entry_index,
            &bindings_by_name,
            &bindings_by_slot,
        )?;

        let binding_array_handles = collect_binding_array_handles(&module);
        let entry_usage = info.get_entry_point(entry_index);

        let mut entry_point_resources = build_entry_point_resources(&desc.bindings, 0)?;
        if let Some(push_constants) = &push_constants {
            let binding_index = u8::try_from(desc.bindings.len())
                .map_err(|_| anyhow!("custom compute binding index exceeds Metal slot limit"))?;
            let buffer_slot = binding_index;
            entry_point_resources.resources.insert(
                naga::ResourceBinding {
                    group: push_constants.slot.group,
                    binding: push_constants.slot.binding,
                },
                naga::back::msl::BindTarget {
                    buffer: Some(buffer_slot),
                    ..Default::default()
                },
            );
        }
        normalize_binding_array_address_space(&mut module);
        let mut naga_options = naga::back::msl::Options::default();
        naga_options.lang_version = msl_lang_version;
        naga_options.fake_missing_bindings = false;
        naga_options.zero_initialize_workgroup_memory = false;
        naga_options.force_loop_bounding = false;
        naga_options
            .per_entry_point_map
            .insert(desc.entry_point.clone(), entry_point_resources);

        let pipeline_options = naga::back::msl::PipelineOptions {
            allow_and_force_point_size: false,
            vertex_pulling_transform: false,
            vertex_buffer_mappings: Vec::new(),
        };

        let (msl_source, translation) =
            naga::back::msl::write_string(&module, &info, &naga_options, &pipeline_options)
                .map_err(|err| anyhow!("MSL translation failed: {err}"))?;

        let compute_name = translation
            .entry_point_names
            .get(entry_index)
            .ok_or_else(|| anyhow!("missing translated compute entry"))
            .and_then(|result| result.as_ref().map_err(|err| anyhow!("{err}")))?
            .to_string();

        let compile_options = metal::CompileOptions::new();
        compile_options.set_language_version(if has_binding_arrays {
            metal::MTLLanguageVersion::V2_0
        } else {
            metal::MTLLanguageVersion::V1_2
        });
        let library = self
            .device
            .new_library_with_source(&msl_source, &compile_options)
            .map_err(|err| anyhow!("MSL compilation failed: {err}"))?;

        let compute_fn = library
            .get_function(&compute_name, None)
            .map_err(|err| anyhow!("compute entry '{compute_name}' not found: {err}"))?;

        let mut argument_buffers =
            Vec::with_capacity(desc.bindings.len() + usize::from(push_constants.is_some()));
        for (index, binding) in desc.bindings.iter().enumerate() {
            let argument_buffer = match binding.kind {
                CustomBindingKind::BufferArray { .. }
                | CustomBindingKind::TextureArray { .. }
                | CustomBindingKind::StorageTextureArray { .. } => {
                    let slot = binding.slot.unwrap_or(CustomBindingSlot {
                        group: 0,
                        binding: binding.name.index(),
                    });
                    let handle = binding_array_handles
                        .get(&(slot.group, slot.binding))
                        .ok_or_else(|| {
                            anyhow!(
                                "binding array '{}' not found in shader",
                                binding.name.as_str()
                            )
                        })?;
                    if entry_usage[*handle].is_empty() {
                        return Err(anyhow!(
                            "binding array '{}' is not used by the shader",
                            binding.name.as_str()
                        ));
                    }
                    let buffer_slot = index as u64;
                    let encoder = compute_fn.new_argument_encoder(buffer_slot as metal::NSUInteger);
                    Some(ArgumentBufferBinding { encoder })
                }
                _ => None,
            };
            argument_buffers.push(argument_buffer);
        }
        if push_constants.is_some() {
            argument_buffers.push(None);
        }

        let pipeline_state = self
            .device
            .new_compute_pipeline_state_with_function(compute_fn.as_ref())
            .map_err(|err| anyhow!("custom compute pipeline failed: {err}"))?;

        let mut binding_kinds: Vec<CustomBindingKind> =
            desc.bindings.iter().map(|binding| binding.kind).collect();
        if let Some(push_constants) = &push_constants {
            binding_kinds.push(CustomBindingKind::Uniform {
                size: push_constants.size,
            });
        }
        let pipeline = MetalCustomComputePipeline {
            pipeline_state,
            bindings: binding_kinds,
            argument_buffers,
            workgroup_size,
            buffer_binding_base: 0,
        };

        let mut pipelines = self.compute_pipelines.lock().unwrap();
        let id = Self::alloc_slot(&mut pipelines, pipeline);
        Ok(CustomComputePipelineId(id))
    }
}

impl CustomDrawRegistry for MetalCustomDrawRegistry {
    fn create_pipeline(&self, desc: CustomPipelineDesc) -> Result<CustomPipelineId> {
        self.create_pipeline_internal(desc, None)
    }

    fn create_pipeline_msl(
        &self,
        desc: CustomPipelineDesc,
        msl_source: String,
    ) -> Result<CustomPipelineId> {
        self.create_pipeline_internal(desc, Some(msl_source))
    }

    fn create_compute_pipeline(
        &self,
        desc: CustomComputePipelineDesc,
    ) -> Result<CustomComputePipelineId> {
        self.create_compute_pipeline_internal(desc)
    }

    fn create_buffer(&self, desc: CustomBufferDesc) -> Result<CustomBufferId> {
        let size = desc.data.len() as u64;
        let buffer = self
            .device
            .new_buffer(size.max(1), MTLResourceOptions::StorageModeManaged);
        if !desc.data.is_empty() {
            unsafe {
                let destination = buffer.contents() as *mut u8;
                std::ptr::copy_nonoverlapping(desc.data.as_ptr(), destination, desc.data.len());
            }
            buffer.did_modify_range(metal::NSRange {
                location: 0,
                length: size as u64,
            });
        }
        let mut buffers = self.buffers.lock().unwrap();
        let id = Self::alloc_slot(
            &mut buffers,
            MetalCustomBuffer {
                buffer,
                size: size.max(1),
            },
        );
        Ok(CustomBufferId(id))
    }

    fn update_buffer(&self, id: CustomBufferId, data: Arc<[u8]>) -> Result<()> {
        let mut buffers = self.buffers.lock().unwrap();
        let Some(slot) = buffers.get_mut(id.0 as usize) else {
            return Err(anyhow!("custom buffer id out of range"));
        };
        let Some(entry) = slot.as_mut() else {
            return Err(anyhow!("custom buffer id out of range"));
        };

        let new_size = data.len() as u64;
        if new_size > entry.size {
            let buffer = self
                .device
                .new_buffer(new_size.max(1), MTLResourceOptions::StorageModeManaged);
            *entry = MetalCustomBuffer {
                buffer,
                size: new_size.max(1),
            };
        }

        if !data.is_empty() {
            unsafe {
                let destination = entry.buffer.contents() as *mut u8;
                std::ptr::copy_nonoverlapping(data.as_ptr(), destination, data.len());
            }
            entry.buffer.did_modify_range(metal::NSRange {
                location: 0,
                length: new_size as u64,
            });
        }
        Ok(())
    }

    fn remove_buffer(&self, id: CustomBufferId) {
        let mut buffers = self.buffers.lock().unwrap();
        if let Some(slot) = buffers.get_mut(id.0 as usize) {
            slot.take();
        }
    }

    fn create_texture(&self, desc: CustomTextureDesc) -> Result<CustomTextureId> {
        let (pixel_format, bytes_per_pixel) = metal_color_format(desc.format);

        if desc.data.is_empty() {
            return Err(anyhow!(
                "custom texture data must include at least one mip level"
            ));
        }
        let max_levels = max_mip_levels(desc.width, desc.height);
        if desc.data.len() as u32 > max_levels {
            return Err(anyhow!(
                "custom texture mip level count {} exceeds maximum {}",
                desc.data.len(),
                max_levels
            ));
        }
        let (texture_type, array_layer_count, array_length) = match desc.dimension {
            CustomTextureDimension::D2 => (metal::MTLTextureType::D2, 1, 1),
            CustomTextureDimension::D2Array { layers } => {
                if layers == 0 {
                    return Err(anyhow!("custom texture array layer count must be non-zero"));
                }
                (metal::MTLTextureType::D2Array, layers, layers)
            }
            CustomTextureDimension::Cube => {
                if desc.width != desc.height {
                    return Err(anyhow!("custom cube textures must be square"));
                }
                (metal::MTLTextureType::Cube, 6, 1)
            }
        };
        for (level, data) in desc.data.iter().enumerate() {
            let (width, height) = mip_level_size(desc.width, desc.height, level as u32);
            let expected_len =
                width as u64 * height as u64 * bytes_per_pixel as u64 * array_layer_count as u64;
            if data.len() < expected_len as usize {
                return Err(anyhow!(
                    "custom texture mip level {} data is smaller than texture size",
                    level
                ));
            }
        }

        let mut usage = metal::MTLTextureUsage::empty();
        if desc.usage.contains(CustomTextureUsage::SAMPLED) {
            usage |= metal::MTLTextureUsage::ShaderRead;
        }
        if desc.usage.contains(CustomTextureUsage::STORAGE) {
            usage |= metal::MTLTextureUsage::ShaderWrite;
        }
        if usage.is_empty() {
            return Err(anyhow!(
                "custom texture usage must include sampled or storage"
            ));
        }
        if desc.usage.contains(CustomTextureUsage::STORAGE) && desc.dimension.is_array() {
            return Err(anyhow!("custom storage textures must be 2D"));
        }

        let descriptor = metal::TextureDescriptor::new();
        descriptor.set_pixel_format(pixel_format);
        descriptor.set_texture_type(texture_type);
        descriptor.set_width(desc.width as u64);
        descriptor.set_height(desc.height as u64);
        descriptor.set_array_length(array_length as u64);
        descriptor.set_mipmap_level_count(desc.data.len() as u64);
        descriptor.set_usage(usage);

        let texture = self.device.new_texture(&descriptor);
        for (level, data) in desc.data.iter().enumerate() {
            let (width, height) = mip_level_size(desc.width, desc.height, level as u32);
            upload_texture_data(
                &texture,
                width,
                height,
                bytes_per_pixel,
                level as u64,
                array_layer_count,
                data,
            );
        }

        let mut textures = self.textures.lock().unwrap();
        let id = Self::alloc_slot(
            &mut textures,
            MetalCustomTexture {
                texture,
                width: desc.width,
                height: desc.height,
                array_layer_count,
                mip_level_count: desc.data.len() as u32,
                bytes_per_pixel,
                format: pixel_format,
                is_render_target: false,
                clear_color: [0.0; 4],
            },
        );
        Ok(CustomTextureId(id))
    }

    fn create_render_target(&self, desc: CustomRenderTargetDesc) -> Result<CustomTextureId> {
        let (pixel_format, bytes_per_pixel) = metal_color_format(desc.format);
        let descriptor = metal::TextureDescriptor::new();
        descriptor.set_pixel_format(pixel_format);
        descriptor.set_width(desc.width as u64);
        descriptor.set_height(desc.height as u64);
        descriptor.set_mipmap_level_count(1);
        descriptor
            .set_usage(metal::MTLTextureUsage::RenderTarget | metal::MTLTextureUsage::ShaderRead);
        descriptor.set_storage_mode(metal::MTLStorageMode::Private);

        let texture = self.device.new_texture(&descriptor);
        let clear_color = desc.clear_color.unwrap_or([0.0, 0.0, 0.0, 0.0]);

        let mut textures = self.textures.lock().unwrap();
        let id = Self::alloc_slot(
            &mut textures,
            MetalCustomTexture {
                texture,
                width: desc.width,
                height: desc.height,
                array_layer_count: 1,
                mip_level_count: 1,
                bytes_per_pixel,
                format: pixel_format,
                is_render_target: true,
                clear_color,
            },
        );
        Ok(CustomTextureId(id))
    }

    fn update_texture(&self, id: CustomTextureId, update: CustomTextureUpdate) -> Result<()> {
        let mut textures = self.textures.lock().unwrap();
        let Some(slot) = textures.get_mut(id.0 as usize) else {
            return Err(anyhow!("custom texture id out of range"));
        };
        let Some(entry) = slot.as_mut() else {
            return Err(anyhow!("custom texture id out of range"));
        };
        if entry.is_render_target {
            return Err(anyhow!("custom render targets cannot be updated"));
        }
        if update.level >= entry.mip_level_count {
            return Err(anyhow!(
                "custom texture mip level {} out of range",
                update.level
            ));
        }
        let (width, height) = mip_level_size(entry.width, entry.height, update.level);
        let expected_len = width as u64
            * height as u64
            * entry.bytes_per_pixel as u64
            * entry.array_layer_count as u64;
        if update.data.len() < expected_len as usize {
            return Err(anyhow!("custom texture data is smaller than texture size"));
        }
        upload_texture_data(
            &entry.texture,
            width,
            height,
            entry.bytes_per_pixel,
            update.level as u64,
            entry.array_layer_count,
            &update.data,
        );
        Ok(())
    }

    fn remove_texture(&self, id: CustomTextureId) {
        let mut textures = self.textures.lock().unwrap();
        if let Some(slot) = textures.get_mut(id.0 as usize) {
            slot.take();
        }
    }

    fn create_depth_target(&self, desc: CustomDepthTargetDesc) -> Result<CustomDepthTargetId> {
        let pixel_format = metal_depth_format(desc.format);
        let descriptor = metal::TextureDescriptor::new();
        descriptor.set_pixel_format(pixel_format);
        descriptor.set_width(desc.width as u64);
        descriptor.set_height(desc.height as u64);
        descriptor.set_usage(metal::MTLTextureUsage::RenderTarget);
        descriptor.set_storage_mode(metal::MTLStorageMode::Private);

        let texture = self.device.new_texture(&descriptor);
        let clear_depth = desc.clear_depth.unwrap_or(1.0) as f64;

        let mut depth_targets = self.depth_targets.lock().unwrap();
        let id = Self::alloc_slot(
            &mut depth_targets,
            MetalCustomDepthTarget {
                texture,
                format: pixel_format,
                clear_depth,
            },
        );
        Ok(CustomDepthTargetId(id))
    }

    fn remove_depth_target(&self, id: CustomDepthTargetId) {
        let mut depth_targets = self.depth_targets.lock().unwrap();
        if let Some(slot) = depth_targets.get_mut(id.0 as usize) {
            slot.take();
        }
    }

    fn create_sampler(&self, desc: CustomSamplerDesc) -> Result<CustomSamplerId> {
        let descriptor = metal::SamplerDescriptor::new();
        descriptor.set_mag_filter(map_min_mag_filter(desc.mag_filter));
        descriptor.set_min_filter(map_min_mag_filter(desc.min_filter));
        descriptor.set_mip_filter(map_mip_filter(desc.mipmap_filter));
        descriptor.set_address_mode_s(map_address_mode(desc.address_modes[0]));
        descriptor.set_address_mode_t(map_address_mode(desc.address_modes[1]));
        descriptor.set_address_mode_r(map_address_mode(desc.address_modes[2]));

        let sampler = self.device.new_sampler(&descriptor);
        let mut samplers = self.samplers.lock().unwrap();
        let id = Self::alloc_slot(&mut samplers, sampler);
        Ok(CustomSamplerId(id))
    }

    fn remove_sampler(&self, id: CustomSamplerId) {
        let mut samplers = self.samplers.lock().unwrap();
        if let Some(slot) = samplers.get_mut(id.0 as usize) {
            slot.take();
        }
    }
}

#[derive(Clone, Copy)]
struct BindingInfo {
    kind: CustomBindingKind,
    slot: CustomBindingSlot,
}

fn build_attribute_locations(
    vertex_fetches: &[CustomVertexFetch],
) -> Result<HashMap<&'static str, u32>> {
    let mut locations = HashMap::new();
    let mut used_locations = std::collections::BTreeSet::new();

    for fetch in vertex_fetches {
        for attribute in &fetch.layout.attributes {
            if let Some(location) = attribute.location {
                if !used_locations.insert(location) {
                    return Err(anyhow!(
                        "custom draw vertex attribute locations must be unique (duplicate {})",
                        location
                    ));
                }
                locations.insert(attribute.name.as_str(), location);
            }
        }
    }

    let mut next_location = 0u32;
    for fetch in vertex_fetches {
        for attribute in &fetch.layout.attributes {
            let name = attribute.name.as_str();
            if locations.contains_key(name) {
                continue;
            }
            while used_locations.contains(&next_location) {
                next_location += 1;
            }
            locations.insert(name, next_location);
            used_locations.insert(next_location);
            next_location += 1;
        }
    }

    Ok(locations)
}

fn build_binding_maps(
    bindings: &[CustomBindingDesc],
) -> (
    HashMap<&'static str, BindingInfo>,
    HashMap<(u32, u32), BindingInfo>,
) {
    let mut by_name = HashMap::new();
    let mut by_slot = HashMap::new();

    for binding in bindings {
        let slot = binding.slot.unwrap_or(CustomBindingSlot {
            group: 0,
            binding: binding.name.index(),
        });
        let info = BindingInfo {
            kind: binding.kind,
            slot,
        };
        by_name.insert(binding.name.as_str(), info);
        by_slot.insert((slot.group, slot.binding), info);
    }

    (by_name, by_slot)
}

fn collect_binding_array_handles(
    module: &naga::Module,
) -> HashMap<(u32, u32), naga::Handle<naga::GlobalVariable>> {
    let mut handles = HashMap::new();
    for (handle, var) in module.global_variables.iter() {
        let Some(binding) = var.binding else {
            continue;
        };
        if matches!(
            module.types[var.ty].inner,
            naga::TypeInner::BindingArray { .. }
        ) {
            handles.insert((binding.group, binding.binding), handle);
        }
    }
    handles
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

fn normalize_binding_array_address_space(module: &mut naga::Module) {
    for (_, var) in module.global_variables.iter_mut() {
        if matches!(
            module.types[var.ty].inner,
            naga::TypeInner::BindingArray { .. }
        ) {
            var.space = naga::AddressSpace::Handle;
        }
    }
}

fn pipeline_cache_key(
    desc: &CustomPipelineDesc,
    source: PipelineSourceKey,
    color_format: metal::MTLPixelFormat,
) -> PipelineCacheKey {
    PipelineCacheKey {
        source,
        vertex_entry: desc.vertex_entry.clone(),
        fragment_entry: desc.fragment_entry.clone(),
        primitive: primitive_key(desc.primitive),
        color_format: color_format as u64,
        state: pipeline_state_key(desc.state),
        vertex_fetches: desc.vertex_fetches.iter().map(vertex_fetch_key).collect(),
        push_constants: desc.push_constants.map(|push| push.size),
        bindings: desc.bindings.iter().map(binding_key).collect(),
    }
}

fn vertex_fetch_key(fetch: &CustomVertexFetch) -> VertexFetchKey {
    VertexFetchKey {
        stride: fetch.layout.stride,
        instanced: fetch.instanced,
        attributes: fetch
            .layout
            .attributes
            .iter()
            .map(vertex_attribute_key)
            .collect(),
    }
}

fn vertex_attribute_key(attribute: &CustomVertexAttribute) -> VertexAttributeKey {
    VertexAttributeKey {
        name: vertex_attribute_name_key(attribute.name),
        offset: attribute.offset,
        format: vertex_format_key(attribute.format),
        location: attribute.location,
    }
}

fn binding_key(binding: &CustomBindingDesc) -> BindingKey {
    BindingKey {
        name: binding_name_key(binding.name),
        kind: binding_kind_key(binding.kind),
        slot: binding.slot,
    }
}

fn binding_kind_key(kind: CustomBindingKind) -> BindingKindKey {
    match kind {
        CustomBindingKind::Buffer => BindingKindKey {
            kind: 0,
            size: 0,
            count: 0,
        },
        CustomBindingKind::Texture => BindingKindKey {
            kind: 1,
            size: 0,
            count: 0,
        },
        CustomBindingKind::StorageTexture => BindingKindKey {
            kind: 2,
            size: 0,
            count: 0,
        },
        CustomBindingKind::Sampler => BindingKindKey {
            kind: 3,
            size: 0,
            count: 0,
        },
        CustomBindingKind::Uniform { size } => BindingKindKey {
            kind: 4,
            size,
            count: 0,
        },
        CustomBindingKind::BufferArray { count } => BindingKindKey {
            kind: 5,
            size: 0,
            count,
        },
        CustomBindingKind::TextureArray { count } => BindingKindKey {
            kind: 6,
            size: 0,
            count,
        },
        CustomBindingKind::StorageTextureArray { count } => BindingKindKey {
            kind: 7,
            size: 0,
            count,
        },
    }
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

fn apply_push_constants(
    module: &mut naga::Module,
    info: &naga::valid::ModuleInfo,
    entry_indices: &[usize],
    push_constants: Option<CustomPushConstantsDesc>,
    slot: CustomBindingSlot,
) -> Result<Option<PushConstantsInfo>> {
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
            return Err(anyhow!(
                "custom draw shaders may declare at most one push constants block"
            ));
        }
        push_constant_handle = Some(handle);
    }

    let Some(handle) = push_constant_handle else {
        if push_constants.is_some() {
            return Err(anyhow!(
                "push constants were provided but the shader has no push constant block"
            ));
        }
        return Ok(None);
    };

    let push_constants = push_constants
        .ok_or_else(|| anyhow!("shader declares push constants but none were provided"))?;

    let mut layouter = naga::proc::Layouter::default();
    layouter
        .update(module.to_ctx())
        .map_err(|err| anyhow!("push constants layout failed: {err}"))?;
    let layout = &layouter[module.global_variables[handle].ty];
    if layout.size != push_constants.size {
        return Err(anyhow!(
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

    Ok(Some(PushConstantsInfo {
        name: Box::leak(name.into_boxed_str()),
        size: push_constants.size,
        slot,
    }))
}

fn vertex_attribute_name_key(name: CustomVertexAttributeName) -> u8 {
    match name {
        CustomVertexAttributeName::A0 => 0,
        CustomVertexAttributeName::A1 => 1,
        CustomVertexAttributeName::A2 => 2,
        CustomVertexAttributeName::A3 => 3,
        CustomVertexAttributeName::A4 => 4,
        CustomVertexAttributeName::A5 => 5,
        CustomVertexAttributeName::A6 => 6,
        CustomVertexAttributeName::A7 => 7,
    }
}

fn binding_name_key(name: CustomBindingName) -> u8 {
    match name {
        CustomBindingName::B0 => 0,
        CustomBindingName::B1 => 1,
        CustomBindingName::B2 => 2,
        CustomBindingName::B3 => 3,
        CustomBindingName::B4 => 4,
        CustomBindingName::B5 => 5,
        CustomBindingName::B6 => 6,
        CustomBindingName::B7 => 7,
        CustomBindingName::B8 => 8,
        CustomBindingName::B9 => 9,
        CustomBindingName::B10 => 10,
        CustomBindingName::B11 => 11,
        CustomBindingName::B12 => 12,
        CustomBindingName::B13 => 13,
        CustomBindingName::B14 => 14,
        CustomBindingName::B15 => 15,
    }
}

fn vertex_format_key(format: CustomVertexFormat) -> u8 {
    match format {
        CustomVertexFormat::F32 => 0,
        CustomVertexFormat::F32Vec2 => 1,
        CustomVertexFormat::F32Vec3 => 2,
        CustomVertexFormat::F32Vec4 => 3,
        CustomVertexFormat::U32 => 4,
        CustomVertexFormat::U32Vec2 => 5,
        CustomVertexFormat::U32Vec3 => 6,
        CustomVertexFormat::U32Vec4 => 7,
        CustomVertexFormat::I32 => 8,
        CustomVertexFormat::I32Vec2 => 9,
        CustomVertexFormat::I32Vec3 => 10,
        CustomVertexFormat::I32Vec4 => 11,
    }
}

fn primitive_key(primitive: CustomPrimitiveTopology) -> u8 {
    match primitive {
        CustomPrimitiveTopology::PointList => 0,
        CustomPrimitiveTopology::LineList => 1,
        CustomPrimitiveTopology::LineStrip => 2,
        CustomPrimitiveTopology::TriangleList => 3,
        CustomPrimitiveTopology::TriangleStrip => 4,
    }
}

fn pipeline_state_key(state: CustomPipelineState) -> PipelineStateKey {
    let (depth_format, depth_compare, depth_write) = match state.depth {
        Some(depth) => (
            depth_format_key(depth.format),
            depth_compare_key(depth.compare),
            depth_write_key(depth.write_enabled),
        ),
        None => (0, 0, 0),
    };
    PipelineStateKey {
        blend: blend_mode_key(state.blend),
        cull_mode: cull_mode_key(state.cull_mode),
        front_face: front_face_key(state.front_face),
        depth_format,
        depth_compare,
        depth_write,
    }
}

fn blend_mode_key(mode: CustomBlendMode) -> u8 {
    match mode {
        CustomBlendMode::Default => 0,
        CustomBlendMode::Opaque => 1,
        CustomBlendMode::Alpha => 2,
        CustomBlendMode::PremultipliedAlpha => 3,
    }
}

fn cull_mode_key(mode: CustomCullMode) -> u8 {
    match mode {
        CustomCullMode::None => 0,
        CustomCullMode::Front => 1,
        CustomCullMode::Back => 2,
    }
}

fn front_face_key(face: CustomFrontFace) -> u8 {
    match face {
        CustomFrontFace::Ccw => 0,
        CustomFrontFace::Cw => 1,
    }
}

fn depth_format_key(format: CustomDepthFormat) -> u8 {
    match format {
        CustomDepthFormat::Depth32Float => 1,
    }
}

fn depth_compare_key(compare: CustomDepthCompare) -> u8 {
    match compare {
        CustomDepthCompare::Always => 0,
        CustomDepthCompare::Less => 1,
        CustomDepthCompare::LessEqual => 2,
        CustomDepthCompare::Greater => 3,
        CustomDepthCompare::GreaterEqual => 4,
    }
}

fn depth_write_key(write_enabled: bool) -> u8 {
    if write_enabled { 1 } else { 0 }
}

fn assign_vertex_locations(
    module: &mut naga::Module,
    vertex_entry_index: usize,
    attribute_locations: &HashMap<&'static str, u32>,
) -> Result<()> {
    for (ep_index, entry_point) in module.entry_points.iter().enumerate() {
        if entry_point.stage != naga::ShaderStage::Vertex {
            continue;
        }
        for argument in entry_point.function.arguments.iter() {
            if argument.binding.is_some() {
                continue;
            }
            let mut ty = module.types[argument.ty].clone();
            let members = match ty.inner {
                naga::TypeInner::Struct {
                    ref mut members, ..
                } => members,
                _ => {
                    return Err(anyhow!(
                        "vertex entry '{}' input is not a struct",
                        entry_point.name
                    ));
                }
            };
            let mut modified = false;
            if ep_index == vertex_entry_index {
                for member in members.iter_mut() {
                    if member.binding.is_some() {
                        continue;
                    }
                    let name = member
                        .name
                        .as_deref()
                        .ok_or_else(|| anyhow!("vertex input member missing name"))?;
                    let location = attribute_locations
                        .get(name)
                        .ok_or_else(|| anyhow!("vertex input '{}' not provided in layout", name))?;
                    member.binding = Some(naga::Binding::Location {
                        location: *location,
                        interpolation: None,
                        sampling: None,
                        blend_src: None,
                    });
                    modified = true;
                }
            } else {
                let mut location = 0;
                for member in members.iter_mut() {
                    if member.binding.is_none() {
                        member.binding = Some(naga::Binding::Location {
                            location,
                            interpolation: None,
                            sampling: None,
                            blend_src: None,
                        });
                        location += 1;
                        modified = true;
                    }
                }
            }
            if modified {
                module.types.replace(argument.ty, ty);
            }
        }
    }
    Ok(())
}

fn assign_resource_bindings(
    module: &mut naga::Module,
    info: &naga::valid::ModuleInfo,
    entry_point_name: &str,
    entry_point_index: usize,
    bindings_by_name: &HashMap<&'static str, BindingInfo>,
    bindings_by_slot: &HashMap<(u32, u32), BindingInfo>,
) -> Result<()> {
    let ep_info = info.get_entry_point(entry_point_index);
    let mut updates = Vec::new();

    for (handle, var) in module.global_variables.iter() {
        if ep_info[handle].is_empty() {
            continue;
        }
        match var.space {
            naga::AddressSpace::Storage { .. }
            | naga::AddressSpace::Uniform
            | naga::AddressSpace::Handle => {}
            _ => continue,
        }
        let name = var.name.as_deref().unwrap_or("<unnamed>");
        let binding_info = if let Some(binding) = var.binding {
            *bindings_by_slot
                .get(&(binding.group, binding.binding))
                .ok_or_else(|| {
                    anyhow!(
                        "explicit binding @group({}) @binding({}) not declared for '{}'",
                        binding.group,
                        binding.binding,
                        name
                    )
                })?
        } else {
            *bindings_by_name
                .get(name)
                .ok_or_else(|| anyhow!("custom draw binding '{}' not declared in pipeline", name))?
        };
        validate_binding_kind(module, var, binding_info.kind, entry_point_name, name)?;
        if var.binding.is_none() {
            updates.push((handle, binding_info.slot));
        }
    }

    for (handle, slot) in updates {
        let var = module.global_variables.get_mut(handle);
        var.binding = Some(naga::ResourceBinding {
            group: slot.group,
            binding: slot.binding,
        });
    }

    Ok(())
}

fn validate_binding_kind(
    module: &naga::Module,
    var: &naga::GlobalVariable,
    binding_kind: CustomBindingKind,
    entry_point_name: &str,
    name: &str,
) -> Result<()> {
    match binding_kind {
        CustomBindingKind::BufferArray { count } => match module.types[var.ty].inner {
            naga::TypeInner::BindingArray { size, .. } => {
                validate_binding_array_size(size, count, entry_point_name, name)?;
                match var.space {
                    naga::AddressSpace::Storage { .. } => Ok(()),
                    _ => Err(anyhow!(
                        "binding '{}' in entry '{}' must be a storage buffer array",
                        name,
                        entry_point_name
                    )),
                }
            }
            _ => Err(anyhow!(
                "binding '{}' in entry '{}' must be a storage buffer array",
                name,
                entry_point_name
            )),
        },
        CustomBindingKind::TextureArray { count } => match module.types[var.ty].inner {
            naga::TypeInner::BindingArray { base, size } => match module.types[base].inner {
                naga::TypeInner::Image {
                    class: naga::ImageClass::Sampled { .. },
                    ..
                } => {
                    validate_binding_array_size(size, count, entry_point_name, name)?;
                    Ok(())
                }
                _ => Err(anyhow!(
                    "binding '{}' in entry '{}' must be a sampled texture array",
                    name,
                    entry_point_name
                )),
            },
            _ => Err(anyhow!(
                "binding '{}' in entry '{}' must be a sampled texture array",
                name,
                entry_point_name
            )),
        },
        CustomBindingKind::StorageTextureArray { count } => match module.types[var.ty].inner {
            naga::TypeInner::BindingArray { base, size } => match module.types[base].inner {
                naga::TypeInner::Image {
                    class: naga::ImageClass::Storage { .. },
                    ..
                } => {
                    validate_binding_array_size(size, count, entry_point_name, name)?;
                    Ok(())
                }
                _ => Err(anyhow!(
                    "binding '{}' in entry '{}' must be a storage texture array",
                    name,
                    entry_point_name
                )),
            },
            _ => Err(anyhow!(
                "binding '{}' in entry '{}' must be a storage texture array",
                name,
                entry_point_name
            )),
        },
        CustomBindingKind::Texture => match module.types[var.ty].inner {
            naga::TypeInner::Image {
                class: naga::ImageClass::Sampled { .. },
                ..
            } => Ok(()),
            _ => Err(anyhow!(
                "binding '{}' in entry '{}' must be a sampled texture",
                name,
                entry_point_name
            )),
        },
        CustomBindingKind::StorageTexture => match module.types[var.ty].inner {
            naga::TypeInner::Image {
                class: naga::ImageClass::Storage { .. },
                ..
            } => Ok(()),
            _ => Err(anyhow!(
                "binding '{}' in entry '{}' must be a storage texture",
                name,
                entry_point_name
            )),
        },
        CustomBindingKind::Sampler => match module.types[var.ty].inner {
            naga::TypeInner::Sampler { .. } => Ok(()),
            _ => Err(anyhow!(
                "binding '{}' in entry '{}' must be a sampler",
                name,
                entry_point_name
            )),
        },
        CustomBindingKind::Uniform { size } => {
            if var.space != naga::AddressSpace::Uniform {
                return Err(anyhow!(
                    "binding '{}' in entry '{}' must be uniform",
                    name,
                    entry_point_name
                ));
            }
            let mut layouter = naga::proc::Layouter::default();
            layouter
                .update(module.to_ctx())
                .map_err(|err| anyhow!("uniform layout failed: {err}"))?;
            let layout = &layouter[var.ty];
            if layout.size != size {
                return Err(anyhow!(
                    "binding '{}' size mismatch (expected {}, shader reports {})",
                    name,
                    size,
                    layout.size
                ));
            }
            Ok(())
        }
        CustomBindingKind::Buffer => match var.space {
            naga::AddressSpace::Storage { .. } => Ok(()),
            _ => Err(anyhow!(
                "binding '{}' in entry '{}' must be a storage buffer",
                name,
                entry_point_name
            )),
        },
    }
}

fn validate_binding_array_size(
    size: naga::ArraySize,
    expected: u32,
    entry_point_name: &str,
    name: &str,
) -> Result<()> {
    let actual = match size {
        naga::ArraySize::Constant(size) => size.get(),
        naga::ArraySize::Pending(_) => {
            return Err(anyhow!(
                "binding '{}' in entry '{}' must use a constant binding array length",
                name,
                entry_point_name
            ));
        }
        naga::ArraySize::Dynamic => {
            return Err(anyhow!(
                "binding '{}' in entry '{}' must not use a runtime-sized binding array",
                name,
                entry_point_name
            ));
        }
    };
    if actual != expected {
        return Err(anyhow!(
            "binding '{}' array length mismatch (expected {}, shader reports {})",
            name,
            expected,
            actual
        ));
    }
    Ok(())
}

fn build_entry_point_resources(
    bindings: &[CustomBindingDesc],
    buffer_binding_base: u8,
) -> Result<naga::back::msl::EntryPointResources> {
    let mut resources = naga::back::msl::EntryPointResources::default();
    for (index, binding) in bindings.iter().enumerate() {
        let binding_index = u8::try_from(index)
            .map_err(|_| anyhow!("custom draw binding index exceeds Metal slot limit"))?;
        let slot = binding.slot.unwrap_or(CustomBindingSlot {
            group: 0,
            binding: binding.name.index(),
        });
        let resource_binding = naga::ResourceBinding {
            group: slot.group,
            binding: slot.binding,
        };
        let buffer_slot = buffer_binding_base
            .checked_add(binding_index)
            .ok_or_else(|| anyhow!("custom draw buffer slot overflow"))?;
        let bind_target = match binding.kind {
            CustomBindingKind::BufferArray { .. } => naga::back::msl::BindTarget {
                buffer: Some(buffer_slot),
                mutable: true,
                ..Default::default()
            },
            CustomBindingKind::TextureArray { .. }
            | CustomBindingKind::StorageTextureArray { .. } => naga::back::msl::BindTarget {
                buffer: Some(buffer_slot),
                ..Default::default()
            },
            CustomBindingKind::Texture | CustomBindingKind::StorageTexture => {
                naga::back::msl::BindTarget {
                    texture: Some(binding_index),
                    ..Default::default()
                }
            }
            CustomBindingKind::Sampler => naga::back::msl::BindTarget {
                sampler: Some(naga::back::msl::BindSamplerTarget::Resource(binding_index)),
                ..Default::default()
            },
            CustomBindingKind::Buffer => naga::back::msl::BindTarget {
                buffer: Some(buffer_slot),
                mutable: true,
                ..Default::default()
            },
            CustomBindingKind::Uniform { .. } => naga::back::msl::BindTarget {
                buffer: Some(buffer_slot),
                ..Default::default()
            },
        };
        resources.resources.insert(resource_binding, bind_target);
    }
    Ok(resources)
}

fn build_vertex_descriptor<'a>(
    vertex_fetches: &'a [CustomVertexFetch],
    attribute_locations: &'a HashMap<&'static str, u32>,
) -> Result<&'a metal::VertexDescriptorRef> {
    let descriptor = metal::VertexDescriptor::new();
    for (buffer_index, fetch) in vertex_fetches.iter().enumerate() {
        let layout = descriptor
            .layouts()
            .object_at(buffer_index as u64)
            .ok_or_else(|| anyhow!("missing vertex buffer layout"))?;
        layout.set_stride(fetch.layout.stride as u64);
        layout.set_step_function(if fetch.instanced {
            metal::MTLVertexStepFunction::PerInstance
        } else {
            metal::MTLVertexStepFunction::PerVertex
        });
        layout.set_step_rate(1);

        for attribute in &fetch.layout.attributes {
            let location = attribute_locations
                .get(attribute.name.as_str())
                .ok_or_else(|| {
                    anyhow!(
                        "vertex attribute '{}' missing location",
                        attribute.name.as_str()
                    )
                })?;
            let attr_descriptor = descriptor
                .attributes()
                .object_at(*location as u64)
                .ok_or_else(|| anyhow!("missing vertex attribute descriptor"))?;
            attr_descriptor.set_format(metal_vertex_format(attribute.format));
            attr_descriptor.set_offset(attribute.offset as u64);
            attr_descriptor.set_buffer_index(buffer_index as u64);
        }
    }
    Ok(descriptor)
}

fn metal_vertex_format(format: CustomVertexFormat) -> metal::MTLVertexFormat {
    match format {
        CustomVertexFormat::F32 => metal::MTLVertexFormat::Float,
        CustomVertexFormat::F32Vec2 => metal::MTLVertexFormat::Float2,
        CustomVertexFormat::F32Vec3 => metal::MTLVertexFormat::Float3,
        CustomVertexFormat::F32Vec4 => metal::MTLVertexFormat::Float4,
        CustomVertexFormat::U32 => metal::MTLVertexFormat::UInt,
        CustomVertexFormat::U32Vec2 => metal::MTLVertexFormat::UInt2,
        CustomVertexFormat::U32Vec3 => metal::MTLVertexFormat::UInt3,
        CustomVertexFormat::U32Vec4 => metal::MTLVertexFormat::UInt4,
        CustomVertexFormat::I32 => metal::MTLVertexFormat::Int,
        CustomVertexFormat::I32Vec2 => metal::MTLVertexFormat::Int2,
        CustomVertexFormat::I32Vec3 => metal::MTLVertexFormat::Int3,
        CustomVertexFormat::I32Vec4 => metal::MTLVertexFormat::Int4,
    }
}

fn metal_color_format(format: CustomTextureFormat) -> (metal::MTLPixelFormat, u32) {
    match format {
        CustomTextureFormat::Rgba8Unorm => (metal::MTLPixelFormat::RGBA8Unorm, 4),
        CustomTextureFormat::Bgra8Unorm => (metal::MTLPixelFormat::BGRA8Unorm, 4),
        CustomTextureFormat::Rgba8UnormSrgb => (metal::MTLPixelFormat::RGBA8Unorm_sRGB, 4),
        CustomTextureFormat::Bgra8UnormSrgb => (metal::MTLPixelFormat::BGRA8Unorm_sRGB, 4),
    }
}

fn resolve_color_format(
    format: Option<CustomTextureFormat>,
    default_format: metal::MTLPixelFormat,
) -> Result<metal::MTLPixelFormat> {
    if let Some(format) = format {
        return Ok(metal_color_format(format).0);
    }
    Ok(default_format)
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

fn metal_depth_format(format: CustomDepthFormat) -> metal::MTLPixelFormat {
    match format {
        CustomDepthFormat::Depth32Float => metal::MTLPixelFormat::Depth32Float,
    }
}

fn metal_compare_function(compare: CustomDepthCompare) -> metal::MTLCompareFunction {
    match compare {
        CustomDepthCompare::Always => metal::MTLCompareFunction::Always,
        CustomDepthCompare::Less => metal::MTLCompareFunction::Less,
        CustomDepthCompare::LessEqual => metal::MTLCompareFunction::LessEqual,
        CustomDepthCompare::Greater => metal::MTLCompareFunction::Greater,
        CustomDepthCompare::GreaterEqual => metal::MTLCompareFunction::GreaterEqual,
    }
}

fn create_depth_state(
    device: &metal::DeviceRef,
    state: CustomDepthState,
) -> metal::DepthStencilState {
    let descriptor = metal::DepthStencilDescriptor::new();
    descriptor.set_depth_compare_function(metal_compare_function(state.compare));
    descriptor.set_depth_write_enabled(state.write_enabled);
    device.new_depth_stencil_state(&descriptor)
}

fn metal_primitive(primitive: CustomPrimitiveTopology) -> metal::MTLPrimitiveType {
    match primitive {
        CustomPrimitiveTopology::PointList => metal::MTLPrimitiveType::Point,
        CustomPrimitiveTopology::LineList => metal::MTLPrimitiveType::Line,
        CustomPrimitiveTopology::LineStrip => metal::MTLPrimitiveType::LineStrip,
        CustomPrimitiveTopology::TriangleList => metal::MTLPrimitiveType::Triangle,
        CustomPrimitiveTopology::TriangleStrip => metal::MTLPrimitiveType::TriangleStrip,
    }
}

fn metal_cull_mode(mode: CustomCullMode) -> metal::MTLCullMode {
    match mode {
        CustomCullMode::None => metal::MTLCullMode::None,
        CustomCullMode::Front => metal::MTLCullMode::Front,
        CustomCullMode::Back => metal::MTLCullMode::Back,
    }
}

fn metal_front_face(face: CustomFrontFace) -> metal::MTLWinding {
    match face {
        CustomFrontFace::Ccw => metal::MTLWinding::CounterClockwise,
        CustomFrontFace::Cw => metal::MTLWinding::Clockwise,
    }
}

fn apply_blend_state(
    color_attachment: &metal::RenderPipelineColorAttachmentDescriptorRef,
    pixel_format: metal::MTLPixelFormat,
    blend: CustomBlendMode,
) {
    color_attachment.set_pixel_format(pixel_format);
    match blend {
        CustomBlendMode::Opaque => {
            color_attachment.set_blending_enabled(false);
        }
        CustomBlendMode::Default | CustomBlendMode::Alpha => {
            color_attachment.set_blending_enabled(true);
            color_attachment.set_rgb_blend_operation(metal::MTLBlendOperation::Add);
            color_attachment.set_alpha_blend_operation(metal::MTLBlendOperation::Add);
            color_attachment.set_source_rgb_blend_factor(metal::MTLBlendFactor::SourceAlpha);
            color_attachment.set_source_alpha_blend_factor(metal::MTLBlendFactor::One);
            color_attachment
                .set_destination_rgb_blend_factor(metal::MTLBlendFactor::OneMinusSourceAlpha);
            color_attachment.set_destination_alpha_blend_factor(metal::MTLBlendFactor::One);
        }
        CustomBlendMode::PremultipliedAlpha => {
            color_attachment.set_blending_enabled(true);
            color_attachment.set_rgb_blend_operation(metal::MTLBlendOperation::Add);
            color_attachment.set_alpha_blend_operation(metal::MTLBlendOperation::Add);
            color_attachment.set_source_rgb_blend_factor(metal::MTLBlendFactor::One);
            color_attachment.set_source_alpha_blend_factor(metal::MTLBlendFactor::One);
            color_attachment
                .set_destination_rgb_blend_factor(metal::MTLBlendFactor::OneMinusSourceAlpha);
            color_attachment.set_destination_alpha_blend_factor(metal::MTLBlendFactor::One);
        }
    }
}

fn map_min_mag_filter(filter: CustomFilterMode) -> metal::MTLSamplerMinMagFilter {
    match filter {
        CustomFilterMode::Nearest => metal::MTLSamplerMinMagFilter::Nearest,
        CustomFilterMode::Linear => metal::MTLSamplerMinMagFilter::Linear,
    }
}

fn map_mip_filter(filter: CustomFilterMode) -> metal::MTLSamplerMipFilter {
    match filter {
        CustomFilterMode::Nearest => metal::MTLSamplerMipFilter::Nearest,
        CustomFilterMode::Linear => metal::MTLSamplerMipFilter::Linear,
    }
}

fn map_address_mode(mode: CustomAddressMode) -> metal::MTLSamplerAddressMode {
    match mode {
        CustomAddressMode::ClampToEdge => metal::MTLSamplerAddressMode::ClampToEdge,
        CustomAddressMode::Repeat => metal::MTLSamplerAddressMode::Repeat,
    }
}

fn upload_texture_data(
    texture: &metal::TextureRef,
    width: u32,
    height: u32,
    bytes_per_pixel: u32,
    mip_level: u64,
    array_layer_count: u32,
    data: &[u8],
) {
    let bytes_per_row = width * bytes_per_pixel;
    let layer_size = (bytes_per_row * height) as usize;
    let region = metal::MTLRegion::new_2d(0, 0, width as u64, height as u64);
    if array_layer_count == 1 {
        texture.replace_region(
            region,
            mip_level,
            data.as_ptr() as *const _,
            bytes_per_row as u64,
        );
        return;
    }

    for layer in 0..array_layer_count {
        let start = layer as usize * layer_size;
        let end = start + layer_size;
        texture.replace_region_in_slice(
            region,
            mip_level,
            layer as u64,
            data[start..end].as_ptr() as *const _,
            bytes_per_row as u64,
            layer_size as u64,
        );
    }
}
