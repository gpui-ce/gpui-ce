use std::sync::Arc;

use anyhow::anyhow;

use crate::{Bounds, ContentMask, Pixels, Result, ScaledPixels};

/// Identifier for a registered custom GPU pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CustomPipelineId(pub(crate) u32);

/// Identifier for a registered custom compute pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CustomComputePipelineId(pub(crate) u32);

/// Identifier for a registered custom buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CustomBufferId(pub(crate) u32);

/// Identifier for a registered custom texture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CustomTextureId(pub(crate) u32);

/// Identifier for a registered custom sampler.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CustomSamplerId(pub(crate) u32);

/// Identifier for a registered custom depth target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CustomDepthTargetId(pub(crate) u32);

/// Primitive topology for custom pipelines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CustomPrimitiveTopology {
    /// Points.
    PointList,
    /// Independent line segments.
    LineList,
    /// Connected line segments.
    LineStrip,
    /// Independent triangles.
    TriangleList,
    /// Connected triangle strip.
    TriangleStrip,
}

/// Front face winding order for custom pipelines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CustomFrontFace {
    /// Counter-clockwise front face.
    #[default]
    Ccw,
    /// Clockwise front face.
    Cw,
}

/// Face culling mode for custom pipelines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CustomCullMode {
    /// Disable face culling.
    #[default]
    None,
    /// Cull front-facing triangles.
    Front,
    /// Cull back-facing triangles.
    Back,
}

/// Blend mode for custom pipelines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CustomBlendMode {
    /// Backend default blend configuration.
    #[default]
    Default,
    /// Disable blending (opaque output).
    Opaque,
    /// Standard alpha blending.
    Alpha,
    /// Premultiplied alpha blending.
    PremultipliedAlpha,
}

/// Depth compare function for custom pipelines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CustomDepthCompare {
    /// Always passes.
    Always,
    /// Passes when the new depth is less.
    Less,
    /// Passes when the new depth is less than or equal.
    #[default]
    LessEqual,
    /// Passes when the new depth is greater.
    Greater,
    /// Passes when the new depth is greater than or equal.
    GreaterEqual,
}

/// Depth formats supported for custom render targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CustomDepthFormat {
    /// 32-bit float depth.
    Depth32Float,
}

/// Depth state for custom pipelines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CustomDepthState {
    /// Depth format.
    pub format: CustomDepthFormat,
    /// Depth compare function.
    pub compare: CustomDepthCompare,
    /// Whether depth writes are enabled.
    pub write_enabled: bool,
}

impl Default for CustomDepthState {
    fn default() -> Self {
        Self {
            format: CustomDepthFormat::Depth32Float,
            compare: CustomDepthCompare::LessEqual,
            write_enabled: true,
        }
    }
}

/// Fixed-function pipeline state for custom pipelines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CustomPipelineState {
    /// Blend mode for the primary color attachment.
    pub blend: CustomBlendMode,
    /// Face culling mode.
    pub cull_mode: CustomCullMode,
    /// Front face winding.
    pub front_face: CustomFrontFace,
    /// Optional depth state.
    pub depth: Option<CustomDepthState>,
    /// Number of MSAA samples for this pipeline.
    pub sample_count: u32,
}

impl Default for CustomPipelineState {
    fn default() -> Self {
        Self {
            blend: CustomBlendMode::Default,
            cull_mode: CustomCullMode::None,
            front_face: CustomFrontFace::Ccw,
            depth: None,
            sample_count: 1,
        }
    }
}

/// Vertex attribute formats supported by custom pipelines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CustomVertexFormat {
    /// Single 32-bit float.
    F32,
    /// 2x 32-bit float.
    F32Vec2,
    /// 3x 32-bit float.
    F32Vec3,
    /// 4x 32-bit float.
    F32Vec4,
    /// Single 32-bit unsigned int.
    U32,
    /// 2x 32-bit unsigned int.
    U32Vec2,
    /// 3x 32-bit unsigned int.
    U32Vec3,
    /// 4x 32-bit unsigned int.
    U32Vec4,
    /// Single 32-bit signed int.
    I32,
    /// 2x 32-bit signed int.
    I32Vec2,
    /// 3x 32-bit signed int.
    I32Vec3,
    /// 4x 32-bit signed int.
    I32Vec4,
}

/// Index formats supported by custom pipelines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CustomIndexFormat {
    /// Unsigned 16-bit indices.
    U16,
    /// Unsigned 32-bit indices.
    U32,
}

/// Fixed set of attribute names for custom vertex layouts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CustomVertexAttributeName {
    /// Attribute slot a0.
    A0,
    /// Attribute slot a1.
    A1,
    /// Attribute slot a2.
    A2,
    /// Attribute slot a3.
    A3,
    /// Attribute slot a4.
    A4,
    /// Attribute slot a5.
    A5,
    /// Attribute slot a6.
    A6,
    /// Attribute slot a7.
    A7,
}

impl CustomVertexAttributeName {
    /// Returns the WGSL field name for this attribute slot.
    pub const fn as_str(self) -> &'static str {
        match self {
            CustomVertexAttributeName::A0 => "a0",
            CustomVertexAttributeName::A1 => "a1",
            CustomVertexAttributeName::A2 => "a2",
            CustomVertexAttributeName::A3 => "a3",
            CustomVertexAttributeName::A4 => "a4",
            CustomVertexAttributeName::A5 => "a5",
            CustomVertexAttributeName::A6 => "a6",
            CustomVertexAttributeName::A7 => "a7",
        }
    }
}

/// Vertex attribute definition for custom pipelines.
#[derive(Debug, Clone)]
pub struct CustomVertexAttribute {
    /// Attribute slot name (must match shader input field name).
    pub name: CustomVertexAttributeName,
    /// Byte offset from the start of the vertex.
    pub offset: u32,
    /// Attribute format.
    pub format: CustomVertexFormat,
    /// Optional explicit shader location for this attribute.
    pub location: Option<u32>,
}

/// Vertex buffer layout for custom pipelines.
#[derive(Debug, Clone)]
pub struct CustomVertexLayout {
    /// Byte stride of a single vertex.
    pub stride: u32,
    /// Vertex attributes in this buffer.
    pub attributes: Vec<CustomVertexAttribute>,
}

/// Vertex fetch configuration for custom pipelines.
#[derive(Debug, Clone)]
pub struct CustomVertexFetch {
    /// Layout of the vertex buffer.
    pub layout: CustomVertexLayout,
    /// Whether the buffer is instanced.
    pub instanced: bool,
}

/// Push constants descriptor for a custom pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CustomPushConstantsDesc {
    /// Push constant size in bytes (16-byte aligned).
    pub size: u32,
}

/// Pipeline description for custom GPU rendering.
#[derive(Debug, Clone)]
pub struct CustomPipelineDesc {
    /// Debug name for the pipeline.
    pub name: String,
    /// WGSL shader source.
    pub shader_source: String,
    /// Vertex entry point name.
    pub vertex_entry: String,
    /// Fragment entry point name.
    pub fragment_entry: String,
    /// Vertex buffer fetch definitions.
    pub vertex_fetches: Vec<CustomVertexFetch>,
    /// Primitive topology.
    pub primitive: CustomPrimitiveTopology,
    /// Color target formats (empty defaults to the surface format).
    pub color_targets: Vec<CustomTextureFormat>,
    /// Fixed-function pipeline state.
    pub state: CustomPipelineState,
    /// Optional push constants block.
    pub push_constants: Option<CustomPushConstantsDesc>,
    /// Optional shader bindings.
    pub bindings: Vec<CustomBindingDesc>,
}

/// Pipeline description for custom GPU compute.
#[derive(Debug, Clone)]
pub struct CustomComputePipelineDesc {
    /// Debug name for the pipeline.
    pub name: String,
    /// WGSL shader source.
    pub shader_source: String,
    /// Compute entry point name.
    pub entry_point: String,
    /// Optional push constants block.
    pub push_constants: Option<CustomPushConstantsDesc>,
    /// Optional shader bindings.
    pub bindings: Vec<CustomBindingDesc>,
}

pub(crate) fn validate_custom_pipeline_desc(desc: &CustomPipelineDesc) -> Result<()> {
    use std::collections::{BTreeMap, BTreeSet, HashSet};

    if desc.vertex_entry.trim().is_empty() {
        return Err(anyhow!("custom draw vertex entry is empty"));
    }
    if desc.fragment_entry.trim().is_empty() {
        return Err(anyhow!("custom draw fragment entry is empty"));
    }

    if let Some(push_constants) = desc.push_constants {
        if push_constants.size == 0 || push_constants.size % 16 != 0 {
            return Err(anyhow!(
                "custom draw push constants size must be non-zero and 16-byte aligned (got {})",
                push_constants.size
            ));
        }
    }

    if desc.state.sample_count == 0
        || desc.state.sample_count > MAX_SAMPLE_COUNT
        || !desc.state.sample_count.is_power_of_two()
    {
        return Err(anyhow!(
            "custom draw sample count must be a power of two between 1 and {} (got {})",
            MAX_SAMPLE_COUNT,
            desc.state.sample_count
        ));
    }

    if desc.color_targets.len() > MAX_COLOR_TARGETS {
        return Err(anyhow!(
            "custom draw color target count must be at most {} (got {})",
            MAX_COLOR_TARGETS,
            desc.color_targets.len()
        ));
    }

    if desc
        .color_targets
        .iter()
        .any(|format| format.is_compressed())
    {
        return Err(anyhow!(
            "custom draw color targets must not use compressed formats"
        ));
    }

    for binding in &desc.bindings {
        match binding.kind {
            CustomBindingKind::Uniform { size } => {
                if size == 0 || size % 16 != 0 {
                    return Err(anyhow!(
                        "custom draw uniform size must be non-zero and 16-byte aligned (got {})",
                        size
                    ));
                }
            }
            CustomBindingKind::BufferArray { count }
            | CustomBindingKind::TextureArray { count }
            | CustomBindingKind::StorageTextureArray { count } => {
                if count == 0 || count > MAX_BINDING_ARRAY_COUNT {
                    return Err(anyhow!(
                        "custom draw binding array count must be between 1 and {} (got {})",
                        MAX_BINDING_ARRAY_COUNT,
                        count
                    ));
                }
            }
            _ => {}
        }
    }

    let mut seen_binding_names = HashSet::new();
    for binding in &desc.bindings {
        if !seen_binding_names.insert(binding.name) {
            return Err(anyhow!(
                "custom draw binding names must be unique (duplicate {})",
                binding.name.as_str()
            ));
        }
    }

    let mut seen_locations = BTreeSet::new();
    for fetch in &desc.vertex_fetches {
        for attr in &fetch.layout.attributes {
            if let Some(location) = attr.location {
                if !seen_locations.insert(location) {
                    return Err(anyhow!(
                        "custom draw vertex attribute locations must be unique (duplicate {})",
                        location
                    ));
                }
            }
        }
    }

    const MAX_BINDING_INDEX: u32 = 4096;
    let mut group_bindings: BTreeMap<u32, BTreeSet<u32>> = BTreeMap::new();
    for binding in &desc.bindings {
        let slot = binding.slot.unwrap_or(CustomBindingSlot {
            group: 0,
            binding: binding.name.index(),
        });
        if slot.binding > MAX_BINDING_INDEX {
            return Err(anyhow!(
                "custom draw binding index {} out of range (max {})",
                slot.binding,
                MAX_BINDING_INDEX
            ));
        }
        let group = group_bindings.entry(slot.group).or_default();
        if !group.insert(slot.binding) {
            return Err(anyhow!(
                "custom draw binding slots must be unique (group {}, binding {})",
                slot.group,
                slot.binding
            ));
        }
    }

    Ok(())
}

pub(crate) fn validate_custom_compute_pipeline_desc(
    desc: &CustomComputePipelineDesc,
) -> Result<()> {
    use std::collections::{BTreeMap, BTreeSet, HashSet};

    if desc.entry_point.trim().is_empty() {
        return Err(anyhow!("custom compute entry is empty"));
    }

    if let Some(push_constants) = desc.push_constants {
        if push_constants.size == 0 || push_constants.size % 16 != 0 {
            return Err(anyhow!(
                "custom compute push constants size must be non-zero and 16-byte aligned (got {})",
                push_constants.size
            ));
        }
    }

    for binding in &desc.bindings {
        match binding.kind {
            CustomBindingKind::Uniform { size } => {
                if size == 0 || size % 16 != 0 {
                    return Err(anyhow!(
                        "custom compute uniform size must be non-zero and 16-byte aligned (got {})",
                        size
                    ));
                }
            }
            CustomBindingKind::BufferArray { count }
            | CustomBindingKind::TextureArray { count }
            | CustomBindingKind::StorageTextureArray { count } => {
                if count == 0 || count > MAX_BINDING_ARRAY_COUNT {
                    return Err(anyhow!(
                        "custom compute binding array count must be between 1 and {} (got {})",
                        MAX_BINDING_ARRAY_COUNT,
                        count
                    ));
                }
            }
            _ => {}
        }
    }

    let mut seen_binding_names = HashSet::new();
    for binding in &desc.bindings {
        if !seen_binding_names.insert(binding.name) {
            return Err(anyhow!(
                "custom compute binding names must be unique (duplicate {})",
                binding.name.as_str()
            ));
        }
    }

    const MAX_BINDING_INDEX: u32 = 4096;
    let mut group_bindings: BTreeMap<u32, BTreeSet<u32>> = BTreeMap::new();
    for binding in &desc.bindings {
        let slot = binding.slot.unwrap_or(CustomBindingSlot {
            group: 0,
            binding: binding.name.index(),
        });
        if slot.binding > MAX_BINDING_INDEX {
            return Err(anyhow!(
                "custom compute binding index {} out of range (max {})",
                slot.binding,
                MAX_BINDING_INDEX
            ));
        }
        let group = group_bindings.entry(slot.group).or_default();
        if !group.insert(slot.binding) {
            return Err(anyhow!(
                "custom compute binding slots must be unique (group {}, binding {})",
                slot.group,
                slot.binding
            ));
        }
    }

    Ok(())
}

/// Fixed set of binding names for custom pipelines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CustomBindingName {
    /// Binding slot b0.
    B0,
    /// Binding slot b1.
    B1,
    /// Binding slot b2.
    B2,
    /// Binding slot b3.
    B3,
    /// Binding slot b4.
    B4,
    /// Binding slot b5.
    B5,
    /// Binding slot b6.
    B6,
    /// Binding slot b7.
    B7,
    /// Binding slot b8.
    B8,
    /// Binding slot b9.
    B9,
    /// Binding slot b10.
    B10,
    /// Binding slot b11.
    B11,
    /// Binding slot b12.
    B12,
    /// Binding slot b13.
    B13,
    /// Binding slot b14.
    B14,
    /// Binding slot b15.
    B15,
}

impl CustomBindingName {
    /// Returns the WGSL variable name for this binding slot.
    pub const fn as_str(self) -> &'static str {
        match self {
            CustomBindingName::B0 => "b0",
            CustomBindingName::B1 => "b1",
            CustomBindingName::B2 => "b2",
            CustomBindingName::B3 => "b3",
            CustomBindingName::B4 => "b4",
            CustomBindingName::B5 => "b5",
            CustomBindingName::B6 => "b6",
            CustomBindingName::B7 => "b7",
            CustomBindingName::B8 => "b8",
            CustomBindingName::B9 => "b9",
            CustomBindingName::B10 => "b10",
            CustomBindingName::B11 => "b11",
            CustomBindingName::B12 => "b12",
            CustomBindingName::B13 => "b13",
            CustomBindingName::B14 => "b14",
            CustomBindingName::B15 => "b15",
        }
    }

    /// Returns the numeric binding index for this slot.
    pub const fn index(self) -> u32 {
        match self {
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
}

/// Maximum number of resources in a binding array.
pub(crate) const MAX_BINDING_ARRAY_COUNT: u32 = 16;
/// Maximum number of color targets in a render pipeline.
pub(crate) const MAX_COLOR_TARGETS: usize = 8;
/// Maximum supported MSAA sample count for custom pipelines.
pub(crate) const MAX_SAMPLE_COUNT: u32 = 8;

/// Binding kinds supported by custom pipelines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CustomBindingKind {
    /// Storage buffer binding.
    Buffer,
    /// Storage buffer binding array.
    BufferArray {
        /// Number of elements in the array.
        count: u32,
    },
    /// 2D sampled texture binding.
    Texture,
    /// 2D sampled texture binding array.
    TextureArray {
        /// Number of elements in the array.
        count: u32,
    },
    /// Storage texture binding (read/write).
    StorageTexture,
    /// Storage texture binding array.
    StorageTextureArray {
        /// Number of elements in the array.
        count: u32,
    },
    /// Sampler binding.
    Sampler,
    /// Uniform/constant buffer binding.
    /// Size is in bytes and should be 16-byte aligned.
    Uniform {
        /// Buffer size in bytes.
        size: u32,
    },
}

/// Binding definition for custom pipelines.
#[derive(Debug, Clone)]
pub struct CustomBindingDesc {
    /// Binding slot name.
    pub name: CustomBindingName,
    /// Binding kind.
    pub kind: CustomBindingKind,
    /// Optional explicit group/binding slot.
    pub slot: Option<CustomBindingSlot>,
}

/// Explicit bind group slot for a custom binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CustomBindingSlot {
    /// Bind group index.
    pub group: u32,
    /// Binding index within the group.
    pub binding: u32,
}

/// Helper for building 16-byte aligned uniform buffers.
#[derive(Debug, Default, Clone)]
pub struct CustomUniformBuilder {
    data: Vec<u8>,
}

impl CustomUniformBuilder {
    /// Create a new uniform builder.
    pub fn new() -> Self {
        Self { data: Vec::new() }
    }

    /// Append raw bytes to the uniform buffer.
    pub fn push_bytes(&mut self, bytes: &[u8]) -> &mut Self {
        self.data.extend_from_slice(bytes);
        self
    }

    /// Append a single f32.
    pub fn push_f32(&mut self, value: f32) -> &mut Self {
        self.data.extend_from_slice(&value.to_le_bytes());
        self
    }

    /// Append a vec2<f32>.
    pub fn push_vec2(&mut self, x: f32, y: f32) -> &mut Self {
        self.push_f32(x);
        self.push_f32(y);
        self
    }

    /// Append a vec4<f32>.
    pub fn push_vec4(&mut self, x: f32, y: f32, z: f32, w: f32) -> &mut Self {
        self.push_f32(x);
        self.push_f32(y);
        self.push_f32(z);
        self.push_f32(w);
        self
    }

    /// Append a 4x4 matrix (column-major) of f32 values.
    pub fn push_mat4(&mut self, values: [f32; 16]) -> &mut Self {
        for value in values {
            self.push_f32(value);
        }
        self
    }

    /// Current size in bytes (before padding).
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// Returns true if empty.
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// Finalize the buffer, padding to 16-byte alignment.
    pub fn finish(mut self) -> Arc<[u8]> {
        let remainder = self.data.len() % 16;
        if remainder != 0 {
            let pad = 16 - remainder;
            self.data.extend(std::iter::repeat(0u8).take(pad));
        }
        Arc::from(self.data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_desc() -> CustomPipelineDesc {
        CustomPipelineDesc {
            name: "test_pipeline".to_string(),
            shader_source: "@vertex fn vs() -> @builtin(position) vec4<f32> { return vec4<f32>(0.0); }\n@fragment fn fs() -> @location(0) vec4<f32> { return vec4<f32>(1.0); }".to_string(),
            vertex_entry: "vs".to_string(),
            fragment_entry: "fs".to_string(),
            vertex_fetches: Vec::new(),
            primitive: CustomPrimitiveTopology::TriangleList,
            color_targets: Vec::new(),
            state: CustomPipelineState::default(),
            push_constants: None,
            bindings: Vec::new(),
        }
    }

    #[test]
    fn rejects_empty_entry_names() {
        let mut desc = base_desc();
        desc.vertex_entry = "   ".to_string();
        assert!(validate_custom_pipeline_desc(&desc).is_err());

        let mut desc = base_desc();
        desc.fragment_entry = "".to_string();
        assert!(validate_custom_pipeline_desc(&desc).is_err());
    }

    #[test]
    fn rejects_misaligned_uniform_sizes() {
        let mut desc = base_desc();
        desc.bindings.push(CustomBindingDesc {
            name: CustomBindingName::B0,
            kind: CustomBindingKind::Uniform { size: 12 },
            slot: None,
        });
        assert!(validate_custom_pipeline_desc(&desc).is_err());
    }

    #[test]
    fn accepts_aligned_uniform_sizes() {
        let mut desc = base_desc();
        desc.bindings.push(CustomBindingDesc {
            name: CustomBindingName::B0,
            kind: CustomBindingKind::Uniform { size: 16 },
            slot: None,
        });
        assert!(validate_custom_pipeline_desc(&desc).is_ok());
    }

    #[test]
    fn rejects_duplicate_vertex_locations() {
        let mut desc = base_desc();
        desc.vertex_fetches.push(CustomVertexFetch {
            layout: CustomVertexLayout {
                stride: 16,
                attributes: vec![
                    CustomVertexAttribute {
                        name: CustomVertexAttributeName::A0,
                        offset: 0,
                        format: CustomVertexFormat::F32Vec2,
                        location: Some(0),
                    },
                    CustomVertexAttribute {
                        name: CustomVertexAttributeName::A1,
                        offset: 8,
                        format: CustomVertexFormat::F32Vec2,
                        location: Some(0),
                    },
                ],
            },
            instanced: false,
        });
        assert!(validate_custom_pipeline_desc(&desc).is_err());
    }

    #[test]
    fn accepts_sparse_binding_slots() {
        let mut desc = base_desc();
        desc.bindings.push(CustomBindingDesc {
            name: CustomBindingName::B0,
            kind: CustomBindingKind::Uniform { size: 16 },
            slot: Some(CustomBindingSlot {
                group: 0,
                binding: 4,
            }),
        });
        desc.bindings.push(CustomBindingDesc {
            name: CustomBindingName::B1,
            kind: CustomBindingKind::Buffer,
            slot: Some(CustomBindingSlot {
                group: 0,
                binding: 0,
            }),
        });
        assert!(validate_custom_pipeline_desc(&desc).is_ok());
    }

    #[test]
    fn uniform_builder_pads_to_16_bytes() {
        let mut builder = CustomUniformBuilder::new();
        builder.push_f32(1.0).push_vec2(2.0, 3.0);
        let data = builder.finish();
        assert_eq!(data.len() % 16, 0);
    }
}

/// Buffer description for custom GPU rendering.
#[derive(Debug, Clone)]
pub struct CustomBufferDesc {
    /// Debug name for the buffer.
    pub name: String,
    /// Initial buffer contents.
    pub data: Arc<[u8]>,
}

/// Source for a custom buffer binding.
#[derive(Debug, Clone)]
pub enum CustomBufferSource {
    /// Buffer previously registered in the custom draw registry.
    Buffer(CustomBufferId),
    /// Slice of a registered buffer.
    BufferSlice {
        /// Buffer identifier.
        id: CustomBufferId,
        /// Byte offset into the buffer.
        offset: u64,
        /// Byte size of the slice.
        size: u64,
    },
    /// Inline buffer contents embedded in the draw call.
    Inline(Arc<[u8]>),
}

impl CustomBufferSource {
    pub(crate) fn hash(&self) -> u64 {
        match self {
            CustomBufferSource::Buffer(id) => (id.0 as u64).wrapping_mul(1099511628211),
            CustomBufferSource::BufferSlice { id, offset, size } => {
                let mut hash = (id.0 as u64).wrapping_mul(1099511628211);
                hash ^= *offset;
                hash = hash.wrapping_mul(1099511628211);
                hash ^= *size;
                hash
            }
            CustomBufferSource::Inline(data) => {
                let mut hash = 1469598103934665603u64;
                for byte in data.iter().take(64) {
                    hash ^= *byte as u64;
                    hash = hash.wrapping_mul(1099511628211);
                }
                hash ^ (data.len() as u64)
            }
        }
    }
}

/// Vertex buffer binding for a draw call.
#[derive(Debug, Clone)]
pub struct CustomVertexBuffer {
    /// Buffer source.
    pub source: CustomBufferSource,
}

/// Index buffer binding for a draw call.
#[derive(Debug, Clone)]
pub struct CustomIndexBuffer {
    /// Buffer source.
    pub source: CustomBufferSource,
    /// Index format.
    pub format: CustomIndexFormat,
}

/// Render target selection for custom draws.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CustomRenderTarget {
    /// Color target textures.
    pub colors: Vec<CustomTextureId>,
    /// Optional depth target.
    pub depth: Option<CustomDepthTargetId>,
}

impl CustomRenderTarget {
    pub(crate) fn hash(&self) -> u64 {
        let mut hash = 1469598103934665603u64;
        for color in &self.colors {
            hash = hash.wrapping_mul(1099511628211);
            hash ^= color.0 as u64;
        }
        if let Some(depth) = self.depth {
            hash = hash.wrapping_mul(1099511628211);
            hash ^= depth.0 as u64;
        }
        hash
    }
}

/// Parameters for a custom draw call.
#[derive(Debug, Clone)]
pub struct CustomDrawParams {
    /// Bounds in window coordinates.
    pub bounds: Bounds<Pixels>,
    /// Pipeline to use.
    pub pipeline: CustomPipelineId,
    /// Vertex buffers bound for the draw.
    pub vertex_buffers: Vec<CustomVertexBuffer>,
    /// Number of vertices to draw (non-indexed draws only).
    pub vertex_count: u32,
    /// Optional index buffer for indexed draws.
    pub index_buffer: Option<CustomIndexBuffer>,
    /// Number of indices to draw when using an index buffer.
    pub index_count: u32,
    /// Optional render target for offscreen passes.
    pub target: Option<CustomRenderTarget>,
    /// Number of instances to draw.
    pub instance_count: u32,
    /// Optional push constants data.
    pub push_constants: Option<Arc<[u8]>>,
    /// Optional shader bindings.
    pub bindings: Vec<CustomBindingValue>,
}

/// Parameters for a custom compute dispatch.
#[derive(Debug, Clone)]
pub struct CustomComputeDispatch {
    /// Compute pipeline to use.
    pub pipeline: CustomComputePipelineId,
    /// Optional push constants data.
    pub push_constants: Option<Arc<[u8]>>,
    /// Bindings for the dispatch.
    pub bindings: Vec<CustomBindingValue>,
    /// Workgroup counts for the dispatch.
    pub workgroup_count: [u32; 3],
}

/// Binding values for a custom draw call.
#[derive(Debug, Clone)]
pub enum CustomBindingValue {
    /// Storage buffer binding.
    Buffer(CustomBufferSource),
    /// Storage buffer binding array.
    BufferArray(Vec<CustomBufferSource>),
    /// Texture binding.
    Texture(CustomTextureId),
    /// Texture binding array.
    TextureArray(Vec<CustomTextureId>),
    /// Sampler binding.
    Sampler(CustomSamplerId),
    /// Uniform/constant buffer binding.
    Uniform(CustomBufferSource),
}

impl CustomBindingValue {
    pub(crate) fn hash(&self) -> u64 {
        let mut hash = 1469598103934665603u64;
        match self {
            CustomBindingValue::Buffer(source) => {
                hash = hash.wrapping_mul(1099511628211);
                hash ^= 1;
                hash = hash.wrapping_mul(1099511628211);
                hash ^= source.hash();
            }
            CustomBindingValue::BufferArray(sources) => {
                hash = hash.wrapping_mul(1099511628211);
                hash ^= 5;
                hash = hash.wrapping_mul(1099511628211);
                hash ^= sources.len() as u64;
                for source in sources {
                    hash = hash.wrapping_mul(1099511628211);
                    hash ^= source.hash();
                }
            }
            CustomBindingValue::Texture(id) => {
                hash = hash.wrapping_mul(1099511628211);
                hash ^= 2;
                hash = hash.wrapping_mul(1099511628211);
                hash ^= id.0 as u64;
            }
            CustomBindingValue::TextureArray(ids) => {
                hash = hash.wrapping_mul(1099511628211);
                hash ^= 6;
                hash = hash.wrapping_mul(1099511628211);
                hash ^= ids.len() as u64;
                for id in ids {
                    hash = hash.wrapping_mul(1099511628211);
                    hash ^= id.0 as u64;
                }
            }
            CustomBindingValue::Sampler(id) => {
                hash = hash.wrapping_mul(1099511628211);
                hash ^= 3;
                hash = hash.wrapping_mul(1099511628211);
                hash ^= id.0 as u64;
            }
            CustomBindingValue::Uniform(source) => {
                hash = hash.wrapping_mul(1099511628211);
                hash ^= 4;
                hash = hash.wrapping_mul(1099511628211);
                hash ^= source.hash();
            }
        }
        hash
    }
}

/// Texture formats supported by custom draw.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CustomTextureFormat {
    /// RGBA8 unorm.
    Rgba8Unorm,
    /// BGRA8 unorm.
    Bgra8Unorm,
    /// RGBA8 unorm sRGB.
    Rgba8UnormSrgb,
    /// BGRA8 unorm sRGB.
    Bgra8UnormSrgb,
    /// BC1/DXT1 RGBA.
    Bc1Unorm,
    /// BC1/DXT1 RGBA sRGB.
    Bc1UnormSrgb,
    /// BC3/DXT5 RGBA.
    Bc3Unorm,
    /// BC3/DXT5 RGBA sRGB.
    Bc3UnormSrgb,
    /// BC7 RGBA.
    Bc7Unorm,
    /// BC7 RGBA sRGB.
    Bc7UnormSrgb,
    /// ETC2 RGB8.
    Etc2Rgb8Unorm,
    /// ETC2 RGB8 sRGB.
    Etc2Rgb8UnormSrgb,
    /// ETC2 RGBA8.
    Etc2Rgba8Unorm,
    /// ETC2 RGBA8 sRGB.
    Etc2Rgba8UnormSrgb,
    /// ASTC 4x4 LDR.
    Astc4x4Unorm,
    /// ASTC 4x4 LDR sRGB.
    Astc4x4UnormSrgb,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CustomTexelBlockInfo {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) bytes: u32,
}

impl CustomTextureFormat {
    pub(crate) const fn block_info(self) -> CustomTexelBlockInfo {
        match self {
            CustomTextureFormat::Rgba8Unorm
            | CustomTextureFormat::Bgra8Unorm
            | CustomTextureFormat::Rgba8UnormSrgb
            | CustomTextureFormat::Bgra8UnormSrgb => CustomTexelBlockInfo {
                width: 1,
                height: 1,
                bytes: 4,
            },
            CustomTextureFormat::Bc1Unorm
            | CustomTextureFormat::Bc1UnormSrgb
            | CustomTextureFormat::Etc2Rgb8Unorm
            | CustomTextureFormat::Etc2Rgb8UnormSrgb => CustomTexelBlockInfo {
                width: 4,
                height: 4,
                bytes: 8,
            },
            CustomTextureFormat::Bc3Unorm
            | CustomTextureFormat::Bc3UnormSrgb
            | CustomTextureFormat::Bc7Unorm
            | CustomTextureFormat::Bc7UnormSrgb
            | CustomTextureFormat::Etc2Rgba8Unorm
            | CustomTextureFormat::Etc2Rgba8UnormSrgb
            | CustomTextureFormat::Astc4x4Unorm
            | CustomTextureFormat::Astc4x4UnormSrgb => CustomTexelBlockInfo {
                width: 4,
                height: 4,
                bytes: 16,
            },
        }
    }

    pub(crate) const fn is_compressed(self) -> bool {
        let info = self.block_info();
        info.width > 1 || info.height > 1
    }
}

/// Texture dimensions supported by custom draw.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CustomTextureDimension {
    /// 2D texture.
    #[default]
    D2,
    /// 2D texture array.
    D2Array {
        /// Number of array layers.
        layers: u32,
    },
    /// Cube texture (6 layers; width and height must match).
    Cube,
}

impl CustomTextureDimension {
    /// Number of array layers for this dimension.
    pub const fn array_layers(self) -> u32 {
        match self {
            CustomTextureDimension::D2 => 1,
            CustomTextureDimension::D2Array { layers } => layers,
            CustomTextureDimension::Cube => 6,
        }
    }

    /// Returns true if this dimension is a cube texture.
    pub const fn is_cube(self) -> bool {
        matches!(self, CustomTextureDimension::Cube)
    }

    /// Returns true if this dimension includes multiple array layers.
    pub const fn is_array(self) -> bool {
        matches!(
            self,
            CustomTextureDimension::D2Array { .. } | CustomTextureDimension::Cube
        )
    }
}

bitflags::bitflags! {
    /// Usage flags for custom textures.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct CustomTextureUsage: u8 {
        /// Texture can be sampled in shaders.
        const SAMPLED = 1 << 0;
        /// Texture can be written via storage texture bindings.
        const STORAGE = 1 << 1;
    }
}

impl Default for CustomTextureUsage {
    fn default() -> Self {
        Self::SAMPLED
    }
}

/// Texture description for custom GPU rendering.
#[derive(Debug, Clone)]
pub struct CustomTextureDesc {
    /// Debug name for the texture.
    pub name: String,
    /// Texture dimension.
    pub dimension: CustomTextureDimension,
    /// Texture width in pixels.
    pub width: u32,
    /// Texture height in pixels.
    pub height: u32,
    /// Texture format.
    pub format: CustomTextureFormat,
    /// Texture usage flags.
    pub usage: CustomTextureUsage,
    /// Initial texture contents for each mip level (level 0 first).
    ///
    /// For array or cube textures, each mip level contains all layers packed
    /// sequentially. For block-compressed formats, each mip level is packed by
    /// block in row-major order.
    pub data: Vec<Arc<[u8]>>,
}

/// Texture update for a specific mip level.
#[derive(Debug, Clone)]
pub struct CustomTextureUpdate {
    /// Mip level to update (0 = base level).
    pub level: u32,
    /// Texture data for the mip level.
    ///
    /// For array or cube textures, the data must include all layers packed
    /// sequentially. For block-compressed formats, each mip level is packed by
    /// block in row-major order.
    pub data: Arc<[u8]>,
}

impl CustomTextureUpdate {
    /// Convenience for updating the base mip level.
    pub fn base_level(data: Arc<[u8]>) -> Self {
        Self { level: 0, data }
    }
}

/// Offscreen render target description.
#[derive(Debug, Clone)]
pub struct CustomRenderTargetDesc {
    /// Debug name for the target.
    pub name: String,
    /// Target width in pixels.
    pub width: u32,
    /// Target height in pixels.
    pub height: u32,
    /// Target format.
    pub format: CustomTextureFormat,
    /// Number of MSAA samples for the target.
    pub sample_count: u32,
    /// Optional clear color for each frame (defaults to transparent black).
    pub clear_color: Option<[f32; 4]>,
}

/// Offscreen depth target description.
#[derive(Debug, Clone)]
pub struct CustomDepthTargetDesc {
    /// Debug name for the target.
    pub name: String,
    /// Target width in pixels.
    pub width: u32,
    /// Target height in pixels.
    pub height: u32,
    /// Depth format.
    pub format: CustomDepthFormat,
    /// Number of MSAA samples for the target.
    pub sample_count: u32,
    /// Optional clear depth value (defaults to 1.0).
    pub clear_depth: Option<f32>,
}

/// Sampler filter modes supported by custom draw.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CustomFilterMode {
    /// Nearest sampling.
    Nearest,
    /// Linear sampling.
    Linear,
}

/// Sampler address modes supported by custom draw.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CustomAddressMode {
    /// Clamp to edge.
    ClampToEdge,
    /// Repeat.
    Repeat,
}

/// Sampler description for custom GPU rendering.
#[derive(Debug, Clone)]
pub struct CustomSamplerDesc {
    /// Debug name for the sampler.
    pub name: String,
    /// Minification filter.
    pub min_filter: CustomFilterMode,
    /// Magnification filter.
    pub mag_filter: CustomFilterMode,
    /// Mipmap filter.
    pub mipmap_filter: CustomFilterMode,
    /// Address modes for U/V/W.
    pub address_modes: [CustomAddressMode; 3],
}

#[derive(Debug, Clone)]
#[cfg_attr(not(feature = "macos-blade"), allow(dead_code))]
pub(crate) struct CustomDraw {
    pub(crate) order: u32,
    pub(crate) bounds: Bounds<ScaledPixels>,
    pub(crate) content_mask: ContentMask<ScaledPixels>,
    pub(crate) pipeline: CustomPipelineId,
    pub(crate) vertex_buffers: Vec<CustomVertexBuffer>,
    pub(crate) vertex_count: u32,
    pub(crate) index_buffer: Option<CustomIndexBuffer>,
    pub(crate) index_count: u32,
    pub(crate) target: Option<CustomRenderTarget>,
    pub(crate) instance_count: u32,
    pub(crate) bindings: Vec<CustomBindingValue>,
    pub(crate) batch_key: CustomBatchKey,
}

#[derive(Debug, Clone)]
#[cfg_attr(not(feature = "macos-blade"), allow(dead_code))]
pub(crate) struct CustomCompute {
    pub(crate) pipeline: CustomComputePipelineId,
    pub(crate) bindings: Vec<CustomBindingValue>,
    pub(crate) workgroup_count: [u32; 3],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct CustomBatchKey {
    pub(crate) pipeline: CustomPipelineId,
    pub(crate) target_hash: u64,
    pub(crate) bindings_hash: u64,
}

pub(crate) trait CustomDrawRegistry: Send + Sync {
    fn create_pipeline(&self, desc: CustomPipelineDesc) -> Result<CustomPipelineId>;
    fn create_pipeline_msl(
        &self,
        desc: CustomPipelineDesc,
        msl_source: String,
    ) -> Result<CustomPipelineId>;
    fn create_compute_pipeline(
        &self,
        desc: CustomComputePipelineDesc,
    ) -> Result<CustomComputePipelineId>;
    fn create_buffer(&self, desc: CustomBufferDesc) -> Result<CustomBufferId>;
    fn update_buffer(&self, id: CustomBufferId, data: Arc<[u8]>) -> Result<()>;
    fn remove_buffer(&self, id: CustomBufferId);
    fn create_texture(&self, desc: CustomTextureDesc) -> Result<CustomTextureId>;
    fn create_render_target(&self, desc: CustomRenderTargetDesc) -> Result<CustomTextureId>;
    fn update_texture(&self, id: CustomTextureId, update: CustomTextureUpdate) -> Result<()>;
    fn remove_texture(&self, id: CustomTextureId);
    fn create_depth_target(&self, desc: CustomDepthTargetDesc) -> Result<CustomDepthTargetId>;
    fn remove_depth_target(&self, id: CustomDepthTargetId);
    fn create_sampler(&self, desc: CustomSamplerDesc) -> Result<CustomSamplerId>;
    fn remove_sampler(&self, id: CustomSamplerId);
}
