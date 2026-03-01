use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use anyhow::anyhow;
use metal::{self, MTLResourceOptions};

use crate::{
    CustomAddressMode, CustomBindingDesc, CustomBindingKind, CustomBindingName, CustomBindingSlot,
    CustomBlendMode, CustomBufferDesc, CustomBufferId, CustomCullMode, CustomDepthCompare,
    CustomDepthFormat, CustomDepthState, CustomDepthTargetDesc, CustomDepthTargetId,
    CustomDrawRegistry, CustomFilterMode, CustomFrontFace, CustomPipelineDesc, CustomPipelineId,
    CustomPipelineState, CustomPrimitiveTopology, CustomRenderTargetDesc, CustomSamplerDesc,
    CustomSamplerId, CustomTextureDesc, CustomTextureFormat, CustomTextureId, CustomTextureUpdate,
    CustomVertexAttribute, CustomVertexAttributeName, CustomVertexFetch, CustomVertexFormat,
    Result,
};

pub(crate) struct MetalCustomDrawRegistry {
    device: metal::Device,
    pixel_format: metal::MTLPixelFormat,
    pipelines: Mutex<Vec<Option<MetalCustomPipeline>>>,
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
    pub(crate) primitive: metal::MTLPrimitiveType,
    pub(crate) cull_mode: metal::MTLCullMode,
    pub(crate) front_face: metal::MTLWinding,
    pub(crate) color_format: metal::MTLPixelFormat,
    pub(crate) depth_format: Option<metal::MTLPixelFormat>,
    pub(crate) depth_state: Option<metal::DepthStencilState>,
    pub(crate) vertex_fetch_count: usize,
    pub(crate) buffer_binding_base: u64,
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

        let mut module = naga::front::wgsl::parse_str(&desc.shader_source)
            .map_err(|err| anyhow!("WGSL parse failed: {err}"))?;
        let flags = naga::valid::ValidationFlags::all() ^ naga::valid::ValidationFlags::BINDINGS;
        let info = naga::valid::Validator::new(flags, naga::valid::Capabilities::empty())
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

        let attribute_locations = build_attribute_locations(&desc.vertex_fetches)?;
        assign_vertex_locations(&mut module, vertex_entry_index, &attribute_locations)?;

        let (bindings_by_name, bindings_by_slot) = build_binding_maps(&desc.bindings);
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
            let entry_point_resources =
                build_entry_point_resources(&desc.bindings, buffer_binding_base)?;
            let mut naga_options = naga::back::msl::Options::default();
            naga_options.lang_version = (1, 2);
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
        compile_options.set_language_version(metal::MTLLanguageVersion::V1_2);
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

        let binding_kinds = desc.bindings.iter().map(|binding| binding.kind).collect();
        let pipeline = MetalCustomPipeline {
            pipeline_state,
            bindings: binding_kinds,
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
        for (level, data) in desc.data.iter().enumerate() {
            let (width, height) = mip_level_size(desc.width, desc.height, level as u32);
            let expected_len = (width * height * bytes_per_pixel) as usize;
            if data.len() < expected_len {
                return Err(anyhow!(
                    "custom texture mip level {} data is smaller than texture size",
                    level
                ));
            }
        }

        let descriptor = metal::TextureDescriptor::new();
        descriptor.set_pixel_format(pixel_format);
        descriptor.set_width(desc.width as u64);
        descriptor.set_height(desc.height as u64);
        descriptor.set_mipmap_level_count(desc.data.len() as u64);
        descriptor.set_usage(metal::MTLTextureUsage::ShaderRead);

        let texture = self.device.new_texture(&descriptor);
        for (level, data) in desc.data.iter().enumerate() {
            let (width, height) = mip_level_size(desc.width, desc.height, level as u32);
            upload_texture_data(&texture, width, height, bytes_per_pixel, level as u64, data);
        }

        let mut textures = self.textures.lock().unwrap();
        let id = Self::alloc_slot(
            &mut textures,
            MetalCustomTexture {
                texture,
                width: desc.width,
                height: desc.height,
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
        let expected_len = (width * height * entry.bytes_per_pixel) as usize;
        if update.data.len() < expected_len {
            return Err(anyhow!("custom texture data is smaller than texture size"));
        }
        upload_texture_data(
            &entry.texture,
            width,
            height,
            entry.bytes_per_pixel,
            update.level as u64,
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
        CustomBindingKind::Buffer => BindingKindKey { kind: 0, size: 0 },
        CustomBindingKind::Texture => BindingKindKey { kind: 1, size: 0 },
        CustomBindingKind::Sampler => BindingKindKey { kind: 2, size: 0 },
        CustomBindingKind::Uniform { size } => BindingKindKey { kind: 3, size },
    }
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
        CustomBindingKind::Texture => match module.types[var.ty].inner {
            naga::TypeInner::Image { .. } => Ok(()),
            _ => Err(anyhow!(
                "binding '{}' in entry '{}' must be a texture",
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
            CustomBindingKind::Texture => naga::back::msl::BindTarget {
                texture: Some(binding_index),
                ..Default::default()
            },
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
    data: &[u8],
) {
    let region = metal::MTLRegion::new_2d(0, 0, width as u64, height as u64);
    texture.replace_region(
        region,
        mip_level,
        data.as_ptr() as *const _,
        (width * bytes_per_pixel) as u64,
    );
}
