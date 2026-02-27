use std::sync::Arc;

use anyhow::anyhow;

use crate::{Bounds, ContentMask, Pixels, Result, ScaledPixels};

/// Identifier for a registered custom GPU pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CustomPipelineId(pub(crate) u32);

/// Identifier for a registered custom buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CustomBufferId(pub(crate) u32);

/// Identifier for a registered custom texture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CustomTextureId(pub(crate) u32);

/// Identifier for a registered custom sampler.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CustomSamplerId(pub(crate) u32);

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
    /// Optional shader bindings (buffers only for now).
    pub bindings: Vec<CustomBindingDesc>,
}

pub(crate) fn validate_custom_pipeline_desc(desc: &CustomPipelineDesc) -> Result<()> {
    if desc.vertex_entry.trim().is_empty() {
        return Err(anyhow!("custom draw vertex entry is empty"));
    }
    if desc.fragment_entry.trim().is_empty() {
        return Err(anyhow!("custom draw fragment entry is empty"));
    }

    for binding in &desc.bindings {
        if let CustomBindingKind::Uniform { size } = binding.kind {
            if size == 0 || size % 16 != 0 {
                return Err(anyhow!(
                    "custom draw uniform size must be non-zero and 16-byte aligned (got {})",
                    size
                ));
            }
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
}

impl CustomBindingName {
    /// Returns the WGSL variable name for this binding slot.
    pub const fn as_str(self) -> &'static str {
        match self {
            CustomBindingName::B0 => "b0",
            CustomBindingName::B1 => "b1",
            CustomBindingName::B2 => "b2",
            CustomBindingName::B3 => "b3",
        }
    }
}

/// Binding kinds supported by custom pipelines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CustomBindingKind {
    /// Storage buffer binding.
    Buffer,
    /// 2D texture binding.
    Texture,
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
        });
        assert!(validate_custom_pipeline_desc(&desc).is_err());
    }

    #[test]
    fn accepts_aligned_uniform_sizes() {
        let mut desc = base_desc();
        desc.bindings.push(CustomBindingDesc {
            name: CustomBindingName::B0,
            kind: CustomBindingKind::Uniform { size: 16 },
        });
        assert!(validate_custom_pipeline_desc(&desc).is_ok());
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

/// Source for a vertex buffer binding.
#[derive(Debug, Clone)]
pub enum CustomBufferSource {
    /// Buffer previously registered in the custom draw registry.
    Buffer(CustomBufferId),
    /// Inline buffer contents embedded in the draw call.
    Inline(Arc<[u8]>),
}

impl CustomBufferSource {
    pub(crate) fn hash(&self) -> u64 {
        match self {
            CustomBufferSource::Buffer(id) => (id.0 as u64).wrapping_mul(1099511628211),
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

/// Parameters for a custom draw call.
#[derive(Debug, Clone)]
pub struct CustomDrawParams {
    /// Bounds in window coordinates.
    pub bounds: Bounds<Pixels>,
    /// Pipeline to use.
    pub pipeline: CustomPipelineId,
    /// Vertex buffers bound for the draw.
    pub vertex_buffers: Vec<CustomVertexBuffer>,
    /// Number of vertices to draw.
    pub vertex_count: u32,
    /// Number of instances to draw.
    pub instance_count: u32,
    /// Optional shader bindings (buffers only for now).
    pub bindings: Vec<CustomBindingValue>,
}

/// Binding values for a custom draw call.
#[derive(Debug, Clone)]
pub enum CustomBindingValue {
    /// Storage buffer binding.
    Buffer(CustomBufferSource),
    /// Texture binding.
    Texture(CustomTextureId),
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
            CustomBindingValue::Texture(id) => {
                hash = hash.wrapping_mul(1099511628211);
                hash ^= 2;
                hash = hash.wrapping_mul(1099511628211);
                hash ^= id.0 as u64;
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
}

/// Texture description for custom GPU rendering.
#[derive(Debug, Clone)]
pub struct CustomTextureDesc {
    /// Debug name for the texture.
    pub name: String,
    /// Texture width in pixels.
    pub width: u32,
    /// Texture height in pixels.
    pub height: u32,
    /// Texture format.
    pub format: CustomTextureFormat,
    /// Initial texture contents.
    pub data: Arc<[u8]>,
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
    pub(crate) instance_count: u32,
    pub(crate) bindings: Vec<CustomBindingValue>,
    pub(crate) batch_key: CustomBatchKey,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct CustomBatchKey {
    pub(crate) pipeline: CustomPipelineId,
    pub(crate) bindings_hash: u64,
}

pub(crate) trait CustomDrawRegistry: Send + Sync {
    fn create_pipeline(&self, desc: CustomPipelineDesc) -> Result<CustomPipelineId>;
    fn create_buffer(&self, desc: CustomBufferDesc) -> Result<CustomBufferId>;
    fn update_buffer(&self, id: CustomBufferId, data: Arc<[u8]>) -> Result<()>;
    fn remove_buffer(&self, id: CustomBufferId);
    fn create_texture(&self, desc: CustomTextureDesc) -> Result<CustomTextureId>;
    fn update_texture(&self, id: CustomTextureId, data: Arc<[u8]>) -> Result<()>;
    fn remove_texture(&self, id: CustomTextureId);
    fn create_sampler(&self, desc: CustomSamplerDesc) -> Result<CustomSamplerId>;
    fn remove_sampler(&self, id: CustomSamplerId);
}
