#![allow(
    // We don't use syntax sugar where it's not necessary.
    clippy::match_like_matches_macro,
    // Redundant matching is more explicit.
    clippy::redundant_pattern_matching,
    // Explicit lifetimes are often easier to reason about.
    clippy::needless_lifetimes,
    // No need for defaults in the internal types.
    clippy::new_without_default,
    // Matches are good and extendable, no need to make an exception here.
    clippy::single_match,
    // Push commands are more regular than macros.
    clippy::vec_init_then_push,
    // This is the land of unsafe.
    clippy::missing_safety_doc,
)]
#![warn(
    trivial_numeric_casts,
    unused_extern_crates,
    //TODO: re-enable. Currently doesn't like "mem::size_of" on newer Rust
    //unused_qualifications,
    // We don't match on a reference, unless required.
    clippy::pattern_type_mismatch,
)]
#![allow(unused_unsafe)]
#![allow(mismatched_lifetime_syntaxes)]

pub use naga::{back::PipelineConstants, StorageAccess, VectorSize};
pub type Transform = mint::RowMatrix3x4<f32>;

pub const IDENTITY_TRANSFORM: Transform = mint::RowMatrix3x4 {
    x: mint::Vector4 {
        x: 1.0,
        y: 0.0,
        z: 0.0,
        w: 0.0,
    },
    y: mint::Vector4 {
        x: 0.0,
        y: 1.0,
        z: 0.0,
        w: 0.0,
    },
    z: mint::Vector4 {
        x: 0.0,
        y: 0.0,
        z: 1.0,
        w: 0.0,
    },
};

pub mod derive;
#[cfg_attr(
    all(not(vulkan), not(gles), any(target_os = "ios", target_os = "macos")),
    path = "metal/mod.rs"
)]
#[cfg_attr(
    all(
        not(gles),
        any(
            vulkan,
            windows,
            target_os = "linux",
            target_os = "android",
            target_os = "freebsd"
        )
    ),
    path = "vulkan/mod.rs"
)]
#[cfg_attr(any(gles, target_arch = "wasm32"), path = "gles/mod.rs")]
mod hal;
mod shader;
pub mod traits;
pub mod util;
pub mod limits {
    /// Max number of passes inside a command encoder.
    pub const PASS_COUNT: usize = 100;
    /// Max plain data size for a pipeline.
    pub const PLAIN_DATA_SIZE: u32 = 256;
    /// Max number of resources in a bind group.
    pub const RESOURCES_IN_GROUP: u32 = 8;
    /// Min storage buffer alignment.
    pub const STORAGE_BUFFER_ALIGNMENT: u64 = 16;
    /// Min acceleration structure scratch buffer alignment.
    pub const ACCELERATION_STRUCTURE_SCRATCH_ALIGNMENT: u64 = 256;
}

pub use hal::*;

#[cfg(target_arch = "wasm32")]
pub const CANVAS_ID: &str = "blade";

use std::{fmt, num::NonZeroU32};

#[derive(Clone, Debug, Default)]
pub struct ContextDesc {
    /// Ability to present contents to a window.
    pub presentation: bool,
    /// Enable validation of the GAPI, shaders,
    /// and insert crash markers into command buffers.
    pub validation: bool,
    /// Enable GPU timing of all passes.
    pub timing: bool,
    /// Enable capture support with GAPI tools.
    pub capture: bool,
    /// Enable GAPI overlay.
    pub overlay: bool,
    /// Force selection of a specific Device ID, unless 0.
    pub device_id: u32,
}

#[derive(Debug)]
pub enum NotSupportedError {
    Platform(PlatformError),
    NoSupportedDeviceFound,
    PlatformNotSupported,
}

impl From<PlatformError> for NotSupportedError {
    fn from(error: PlatformError) -> Self {
        Self::Platform(error)
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Capabilities {
    /// Which shader stages support ray queries
    pub ray_query: ShaderVisibility,
    /// Bit mask of supported MSAA sample counts.
    pub sample_count_mask: u32,
}

#[derive(Clone, Debug, Default)]
pub struct DeviceInformation {
    /// If this is something like llvmpipe, not a real GPU
    pub is_software_emulated: bool,
    /// The name of the GPU device
    pub device_name: String,
    /// The driver used to talk to the GPU
    pub driver_name: String,
    /// Further information about the driver
    pub driver_info: String,
}

impl Context {
    pub fn create_surface_configured<
        I: raw_window_handle::HasWindowHandle + raw_window_handle::HasDisplayHandle,
    >(
        &self,
        window: &I,
        config: SurfaceConfig,
    ) -> Result<Surface, NotSupportedError> {
        let mut surface = self.create_surface(window)?;
        self.reconfigure_surface(&mut surface, config);
        Ok(surface)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Memory {
    /// Device-local memory. Fast for GPU operations.
    Device,
    /// Shared CPU-GPU memory. Not so fast for GPU.
    Shared,
    /// Upload memory. Can only be transferred on GPU.
    Upload,
    /// External memory
    External(ExternalMemorySource),
}

/// If the source contains None it will export it otherwise it will import the value in Some
//TODO add D3D11, D3D12 and metal support
#[derive(Clone, Copy, Debug, PartialEq, Hash)]
pub enum ExternalMemorySource {
    #[cfg(target_os = "windows")]
    Win32(Option<isize>),

    #[cfg(target_os = "windows")]
    Win32KMT(Option<isize>),

    #[cfg(not(target_os = "windows"))]
    Fd(Option<i32>),

    #[cfg(target_os = "linux")]
    Dma(Option<i32>),

    /// Is a pointer cast to usize, reason being it otherwise can't be used as sync and send, you should manage memory access to this yourself
    HostAllocation(usize),
}

impl Memory {
    pub fn is_host_visible(&self) -> bool {
        match *self {
            Self::Shared
            | Self::Upload
            | Self::External(ExternalMemorySource::HostAllocation(_)) => true,
            Self::Device | Self::External(_) => false,
        }
    }
}

#[derive(Debug)]
pub struct BufferDesc<'a> {
    pub name: &'a str,
    pub size: u64,
    pub memory: Memory,
}

#[derive(Clone, Copy, Debug)]
pub struct BufferPiece {
    pub buffer: Buffer,
    pub offset: u64,
}

impl From<Buffer> for BufferPiece {
    fn from(buffer: Buffer) -> Self {
        Self { buffer, offset: 0 }
    }
}

impl BufferPiece {
    pub fn data(&self) -> *mut u8 {
        let base = self.buffer.data();
        assert!(!base.is_null());
        unsafe { base.offset(self.offset as isize) }
    }
}

impl Buffer {
    pub fn at(self, offset: u64) -> BufferPiece {
        BufferPiece {
            buffer: self,
            offset,
        }
    }
}

pub type ResourceIndex = u32;
/// An array of resources to be used with shader bindings.
/// The generic argument tells the maximum number of resources.
pub struct ResourceArray<T, const N: ResourceIndex> {
    data: Vec<T>,
    free_list: Vec<ResourceIndex>,
}
impl<T, const N: ResourceIndex> ResourceArray<T, N> {
    pub fn new() -> Self {
        Self {
            data: Vec::with_capacity(N as usize),
            free_list: Vec::new(),
        }
    }
    pub fn alloc(&mut self, value: T) -> ResourceIndex {
        if let Some(index) = self.free_list.pop() {
            self.data[index as usize] = value;
            index
        } else {
            let index = self.data.len() as u32;
            assert!(index < N);
            self.data.push(value);
            index
        }
    }
    pub fn free(&mut self, index: ResourceIndex) {
        self.free_list.push(index);
    }
    pub fn clear(&mut self) {
        self.data.clear();
        self.free_list.clear();
    }
}
impl<T, const N: ResourceIndex> std::ops::Index<ResourceIndex> for ResourceArray<T, N> {
    type Output = T;
    fn index(&self, index: ResourceIndex) -> &T {
        &self.data[index as usize]
    }
}
impl<T, const N: ResourceIndex> std::ops::IndexMut<ResourceIndex> for ResourceArray<T, N> {
    fn index_mut(&mut self, index: ResourceIndex) -> &mut T {
        &mut self.data[index as usize]
    }
}
pub type BufferArray<const N: ResourceIndex> = ResourceArray<BufferPiece, N>;
pub type TextureArray<const N: ResourceIndex> = ResourceArray<TextureView, N>;

#[derive(Clone, Copy, Debug)]
pub struct TexturePiece {
    pub texture: Texture,
    pub mip_level: u32,
    pub array_layer: u32,
    pub origin: [u32; 3],
}

impl From<Texture> for TexturePiece {
    fn from(texture: Texture) -> Self {
        Self {
            texture,
            mip_level: 0,
            array_layer: 0,
            origin: [0; 3],
        }
    }
}

#[non_exhaustive]
#[derive(Clone, Copy, Debug, Hash, Eq, Ord, PartialEq, PartialOrd)]
pub enum TextureFormat {
    // color
    R8Unorm,
    Rg8Unorm,
    Rg8Snorm,
    Rgba8Unorm,
    Rgba8UnormSrgb,
    Bgra8Unorm,
    Bgra8UnormSrgb,
    Rgba8Snorm,
    R16Float,
    Rg16Float,
    Rgba16Float,
    R32Float,
    Rg32Float,
    Rgba32Float,
    R32Uint,
    Rg32Uint,
    Rgba32Uint,
    // depth and stencil
    Depth32Float,
    Depth32FloatStencil8Uint,
    Stencil8Uint,
    // S3TC block compression
    Bc1Unorm,
    Bc1UnormSrgb,
    Bc2Unorm,
    Bc2UnormSrgb,
    Bc3Unorm,
    Bc3UnormSrgb,
    Bc4Unorm,
    Bc4Snorm,
    Bc5Unorm,
    Bc5Snorm,
    Bc6hUfloat,
    Bc6hFloat,
    Bc7Unorm,
    Bc7UnormSrgb,
    // packed 32-bit
    Rgb10a2Unorm,
    Rg11b10Ufloat,
    Rgb9e5Ufloat,
}

#[derive(Clone, Copy, Debug)]
pub struct TexelBlockInfo {
    pub dimensions: (u8, u8),
    pub size: u8,
}

bitflags::bitflags! {
    #[derive(Copy, Clone, Debug, Default, Hash, Eq, PartialEq)]
    pub struct TexelAspects: u8 {
        const COLOR = 0x1;
        const DEPTH = 0x2;
        const STENCIL = 0x4;
    }
}

/// Dimensionality of a texture.
#[derive(Copy, Clone, Debug, Hash, Eq, PartialEq)]
pub enum TextureDimension {
    /// 1D texture
    D1,
    /// 2D texture
    D2,
    /// 3D texture
    D3,
}

#[derive(Copy, Clone, Debug, Hash, Eq, PartialEq)]
pub enum ViewDimension {
    D1,
    D1Array,
    D2,
    D2Array,
    Cube,
    CubeArray,
    D3,
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Hash, Eq, PartialEq)]
pub struct Extent {
    pub width: u32,
    pub height: u32,
    pub depth: u32,
}
impl Default for Extent {
    fn default() -> Self {
        Self {
            width: 1,
            height: 1,
            depth: 1,
        }
    }
}
impl fmt::Display for Extent {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}x{}x{}", self.width, self.height, self.depth)
    }
}

impl Extent {
    pub fn max_mip_levels(&self) -> u32 {
        self.width
            .max(self.height)
            .max(self.depth)
            .next_power_of_two()
            .trailing_zeros()
    }
    pub fn at_mip_level(&self, level: u32) -> Self {
        Self {
            width: (self.width >> level).max(1),
            height: (self.height >> level).max(1),
            depth: (self.depth >> level).max(1),
        }
    }
}

bitflags::bitflags! {
    #[derive(Copy, Clone, Debug, Default, Hash, Eq, PartialEq)]
    pub struct TextureUsage: u32 {
        const COPY = 1 << 0;
        const TARGET = 1 << 1;
        const RESOURCE = 1 << 2;
        const STORAGE = 1 << 3;
    }
}

#[derive(Debug)]
pub struct TextureDesc<'a> {
    pub name: &'a str,
    pub format: TextureFormat,
    pub size: Extent,
    pub array_layer_count: u32,
    pub mip_level_count: u32,
    pub sample_count: u32,
    pub dimension: TextureDimension,
    pub usage: TextureUsage,
    pub external: Option<ExternalMemorySource>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TextureSubresources {
    pub base_mip_level: u32,
    pub mip_level_count: Option<NonZeroU32>,
    pub base_array_layer: u32,
    pub array_layer_count: Option<NonZeroU32>,
}

#[derive(Debug)]
pub struct TextureViewDesc<'a> {
    pub name: &'a str,
    pub format: TextureFormat,
    pub dimension: ViewDimension,
    pub subresources: &'a TextureSubresources,
}

bitflags::bitflags! {
    #[derive(Copy, Clone, Debug, Default, Hash, Eq, PartialEq)]
    pub struct ShaderVisibility: u32 {
        const COMPUTE = 1 << 0;
        const VERTEX = 1 << 1;
        const FRAGMENT = 1 << 2;
    }
}

/// How edges should be handled in texture addressing.
#[derive(Copy, Clone, Debug, Default, Hash, Eq, PartialEq)]
pub enum AddressMode {
    /// Clamp the value to the edge of the texture.
    #[default]
    ClampToEdge,
    /// Repeat the texture in a tiling fashion.
    Repeat,
    /// Repeat the texture, mirroring it every repeat.
    MirrorRepeat,
    /// Clamp the value to the border of the texture.
    ClampToBorder,
}

/// Texel mixing mode when sampling between texels.
#[derive(Copy, Clone, Debug, Default, Hash, Eq, PartialEq)]
pub enum FilterMode {
    /// Nearest neighbor sampling.
    #[default]
    Nearest,
    /// Linear Interpolation
    Linear,
}

/// Comparison function used for depth and stencil operations.
#[derive(Copy, Clone, Debug, Default, Hash, Eq, PartialEq)]
pub enum CompareFunction {
    /// Function never passes
    Never,
    /// Function passes if new value less than existing value
    Less,
    /// Function passes if new value is equal to existing value. When using
    /// this compare function, make sure to mark your Vertex Shader's `@builtin(position)`
    /// output as `@invariant` to prevent artifacting.
    Equal,
    /// Function passes if new value is less than or equal to existing value
    LessEqual,
    /// Function passes if new value is greater than existing value
    Greater,
    /// Function passes if new value is not equal to existing value. When using
    /// this compare function, make sure to mark your Vertex Shader's `@builtin(position)`
    /// output as `@invariant` to prevent artifacting.
    NotEqual,
    /// Function passes if new value is greater than or equal to existing value
    GreaterEqual,
    /// Function always passes
    #[default]
    Always,
}

#[derive(Clone, Copy, Debug)]
pub enum TextureColor {
    TransparentBlack,
    OpaqueBlack,
    White,
}

#[derive(Debug, Default)]
pub struct SamplerDesc<'a> {
    pub name: &'a str,
    pub address_modes: [AddressMode; 3],
    pub mag_filter: FilterMode,
    pub min_filter: FilterMode,
    pub mipmap_filter: FilterMode,
    pub lod_min_clamp: f32,
    pub lod_max_clamp: Option<f32>,
    pub compare: Option<CompareFunction>,
    pub anisotropy_clamp: u32,
    pub border_color: Option<TextureColor>,
}

#[derive(Debug)]
pub enum AccelerationStructureType {
    TopLevel,
    BottomLevel,
}

#[derive(Debug)]
pub struct AccelerationStructureDesc<'a> {
    pub name: &'a str,
    pub ty: AccelerationStructureType,
    pub size: u64,
}

#[non_exhaustive]
#[derive(Clone, Copy, Debug, Hash, Eq, Ord, PartialEq, PartialOrd)]
pub enum VertexFormat {
    F32,
    F32Vec2,
    F32Vec3,
    F32Vec4,
    U32,
    U32Vec2,
    U32Vec3,
    U32Vec4,
    I32,
    I32Vec2,
    I32Vec3,
    I32Vec4,
}

#[derive(Clone, Debug)]
pub struct AccelerationStructureMesh {
    pub vertex_data: BufferPiece,
    pub vertex_format: VertexFormat,
    pub vertex_stride: u32,
    pub vertex_count: u32,
    pub index_data: BufferPiece,
    pub index_type: Option<IndexType>,
    pub triangle_count: u32,
    pub transform_data: BufferPiece,
    pub is_opaque: bool,
}

#[derive(Clone, Debug)]
pub struct AccelerationStructureInstance {
    pub acceleration_structure_index: u32,
    pub transform: Transform,
    pub mask: u32,
    pub custom_index: u32,
}

impl Default for AccelerationStructureInstance {
    fn default() -> Self {
        Self {
            acceleration_structure_index: 0,
            transform: IDENTITY_TRANSFORM,
            mask: 0xFF,
            custom_index: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AccelerationStructureSizes {
    /// Size of the permanent GPU data
    pub data: u64,
    /// Size of the scratch space
    pub scratch: u64,
}

pub struct Shader {
    module: naga::Module,
    info: naga::valid::ModuleInfo,
    source: String,
}

#[derive(Clone, Copy)]
pub struct ShaderFunction<'a> {
    pub shader: &'a Shader,
    pub entry_point: &'a str,
    pub constants: &'a PipelineConstants,
}

impl ShaderFunction<'_> {
    fn entry_point_index(&self) -> usize {
        self.shader
            .module
            .entry_points
            .iter()
            .position(|ep| ep.name == self.entry_point)
            .expect("Entry point not found in the shader")
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ShaderBinding {
    Texture,
    TextureArray { count: u32 },
    Sampler,
    Buffer,
    BufferArray { count: u32 },
    AccelerationStructure,
    Plain { size: u32 },
}

pub trait ShaderBindable: Clone + Copy + derive::HasShaderBinding {
    fn bind_to(&self, context: &mut PipelineContext, index: u32);
}

#[derive(Debug)]
struct ShaderDataInfo {
    visibility: ShaderVisibility,
    binding_access: Box<[StorageAccess]>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ShaderDataLayout {
    pub bindings: Vec<(&'static str, ShaderBinding)>,
}
impl ShaderDataLayout {
    pub const EMPTY: &'static Self = &Self {
        bindings: Vec::new(),
    };

    fn to_info(&self) -> ShaderDataInfo {
        ShaderDataInfo {
            visibility: ShaderVisibility::empty(),
            binding_access: vec![StorageAccess::empty(); self.bindings.len()].into_boxed_slice(),
        }
    }
}

pub trait ShaderData {
    fn layout() -> ShaderDataLayout;
    fn fill(&self, context: PipelineContext);
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub struct VertexAttribute {
    pub offset: u32,
    pub format: VertexFormat,
    pub location: Option<u32>,
}

struct VertexAttributeMapping {
    location: u32,
    buffer_index: usize,
    attribute_index: usize,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct VertexLayout {
    pub attributes: Vec<(&'static str, VertexAttribute)>,
    pub stride: u32,
}

pub trait Vertex {
    fn layout() -> VertexLayout;
}

#[derive(Clone, Debug, PartialEq)]
pub struct VertexFetchState<'a> {
    pub layout: &'a VertexLayout,
    pub instanced: bool,
}

pub struct ShaderDesc<'a> {
    pub source: &'a str,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum CommandType {
    Transfer,
    Compute,
    #[default]
    General,
}

pub struct CommandEncoderDesc<'a> {
    pub name: &'a str,
    /// Number of buffers that this encoder needs to keep alive.
    /// For example, one buffer is being run on GPU while the
    /// other is being actively encoded, which makes 2.
    pub buffer_count: u32,
}

pub struct ComputePipelineDesc<'a> {
    pub name: &'a str,
    pub data_layouts: &'a [&'a ShaderDataLayout],
    pub compute: ShaderFunction<'a>,
}

/// Primitive type the input mesh is composed of.
#[derive(Copy, Clone, Debug, Default, Hash, Eq, PartialEq)]
pub enum PrimitiveTopology {
    /// Vertex data is a list of points. Each vertex is a new point.
    PointList,
    /// Vertex data is a list of lines. Each pair of vertices composes a new line.
    ///
    /// Vertices `0 1 2 3` create two lines `0 1` and `2 3`
    LineList,
    /// Vertex data is a strip of lines. Each set of two adjacent vertices form a line.
    ///
    /// Vertices `0 1 2 3` create three lines `0 1`, `1 2`, and `2 3`.
    LineStrip,
    /// Vertex data is a list of triangles. Each set of 3 vertices composes a new triangle.
    ///
    /// Vertices `0 1 2 3 4 5` create two triangles `0 1 2` and `3 4 5`
    #[default]
    TriangleList,
    /// Vertex data is a triangle strip. Each set of three adjacent vertices form a triangle.
    ///
    /// Vertices `0 1 2 3 4 5` creates four triangles `0 1 2`, `2 1 3`, `2 3 4`, and `4 3 5`
    TriangleStrip,
}

/// Vertex winding order which classifies the "front" face of a triangle.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum FrontFace {
    /// Triangles with vertices in counter clockwise order are considered the front face.
    ///
    /// This is the default with right handed coordinate spaces.
    #[default]
    Ccw,
    /// Triangles with vertices in clockwise order are considered the front face.
    ///
    /// This is the default with left handed coordinate spaces.
    Cw,
}

/// Face of a vertex.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Face {
    /// Front face
    Front,
    /// Back face
    Back,
}

#[derive(Clone, Debug, Default)]
pub struct PrimitiveState {
    /// The primitive topology used to interpret vertices.
    pub topology: PrimitiveTopology,
    /// The face to consider the front for the purpose of culling and stencil operations.
    pub front_face: FrontFace,
    /// The face culling mode.
    pub cull_mode: Option<Face>,
    /// If set to true, the polygon depth is not clipped to 0-1 before rasterization.
    pub unclipped_depth: bool,
    /// If true, only the primitive edges are rasterized..
    pub wireframe: bool,
}

/// Operation to perform on the stencil value.
#[derive(Copy, Clone, Debug, Default, Hash, Eq, PartialEq)]
pub enum StencilOperation {
    /// Keep stencil value unchanged.
    #[default]
    Keep,
    /// Set stencil value to zero.
    Zero,
    /// Replace stencil value with value provided.
    Replace,
    /// Bitwise inverts stencil value.
    Invert,
    /// Increments stencil value by one, clamping on overflow.
    IncrementClamp,
    /// Decrements stencil value by one, clamping on underflow.
    DecrementClamp,
    /// Increments stencil value by one, wrapping on overflow.
    IncrementWrap,
    /// Decrements stencil value by one, wrapping on underflow.
    DecrementWrap,
}

/// Describes stencil state in a render pipeline.
///
/// If you are not using stencil state, set this to [`StencilFaceState::IGNORE`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct StencilFaceState {
    /// Comparison function that determines if the fail_op or pass_op is used on the stencil buffer.
    pub compare: CompareFunction,
    /// Operation that is preformed when stencil test fails.
    pub fail_op: StencilOperation,
    /// Operation that is performed when depth test fails but stencil test succeeds.
    pub depth_fail_op: StencilOperation,
    /// Operation that is performed when stencil test success.
    pub pass_op: StencilOperation,
}

impl StencilFaceState {
    /// Ignore the stencil state for the face.
    pub const IGNORE: Self = StencilFaceState {
        compare: CompareFunction::Always,
        fail_op: StencilOperation::Keep,
        depth_fail_op: StencilOperation::Keep,
        pass_op: StencilOperation::Keep,
    };
}

impl Default for StencilFaceState {
    fn default() -> Self {
        Self::IGNORE
    }
}

/// State of the stencil operation (fixed-pipeline stage).
///
/// For use in [`DepthStencilState`].
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct StencilState {
    /// Front face mode.
    pub front: StencilFaceState,
    /// Back face mode.
    pub back: StencilFaceState,
    /// Stencil values are AND'd with this mask when reading and writing from the stencil buffer. Only low 8 bits are used.
    pub read_mask: u32,
    /// Stencil values are AND'd with this mask when writing to the stencil buffer. Only low 8 bits are used.
    pub write_mask: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DepthBiasState {
    /// Constant depth biasing factor, in basic units of the depth format.
    pub constant: i32,
    /// Slope depth biasing factor.
    pub slope_scale: f32,
    /// Depth bias clamp value (absolute).
    pub clamp: f32,
}

/// Describes the depth/stencil state in a render pipeline.
#[derive(Clone, Debug)]
pub struct DepthStencilState {
    /// Format of the depth/stencil texture view.
    pub format: TextureFormat,
    /// If disabled, depth will not be written to.
    pub depth_write_enabled: bool,
    /// Comparison function used to compare depth values in the depth test.
    pub depth_compare: CompareFunction,
    /// Stencil state.
    pub stencil: StencilState,
    /// Depth bias state.
    pub bias: DepthBiasState,
}

/// Alpha blend factor.
#[derive(Copy, Clone, Debug, Hash, Eq, PartialEq)]
pub enum BlendFactor {
    /// 0.0
    Zero,
    /// 1.0
    One,
    /// S.component
    Src,
    /// 1.0 - S.component
    OneMinusSrc,
    /// S.alpha
    SrcAlpha,
    /// 1.0 - S.alpha
    OneMinusSrcAlpha,
    /// D.component
    Dst,
    /// 1.0 - D.component
    OneMinusDst,
    /// D.alpha
    DstAlpha,
    /// 1.0 - D.alpha
    OneMinusDstAlpha,
    /// min(S.alpha, 1.0 - D.alpha)
    SrcAlphaSaturated,
    /// Constant
    Constant,
    /// 1.0 - Constant
    OneMinusConstant,
}

/// Alpha blend operation.
#[derive(Copy, Clone, Debug, Default, Hash, Eq, PartialEq)]
pub enum BlendOperation {
    /// Src + Dst
    #[default]
    Add,
    /// Src - Dst
    Subtract,
    /// Dst - Src
    ReverseSubtract,
    /// min(Src, Dst)
    Min,
    /// max(Src, Dst)
    Max,
}

/// Describes a blend component of a [`BlendState`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BlendComponent {
    /// Multiplier for the source, which is produced by the fragment shader.
    pub src_factor: BlendFactor,
    /// Multiplier for the destination, which is stored in the target.
    pub dst_factor: BlendFactor,
    /// The binary operation applied to the source and destination,
    /// multiplied by their respective factors.
    pub operation: BlendOperation,
}

impl BlendComponent {
    /// Default blending state that replaces destination with the source.
    pub const REPLACE: Self = Self {
        src_factor: BlendFactor::One,
        dst_factor: BlendFactor::Zero,
        operation: BlendOperation::Add,
    };

    /// Blend state of (1 * src) + ((1 - src_alpha) * dst)
    pub const OVER: Self = Self {
        src_factor: BlendFactor::One,
        dst_factor: BlendFactor::OneMinusSrcAlpha,
        operation: BlendOperation::Add,
    };

    /// Blend state of src + dst
    pub const ADDITIVE: Self = Self {
        src_factor: BlendFactor::One,
        dst_factor: BlendFactor::One,
        operation: BlendOperation::Add,
    };
}

impl Default for BlendComponent {
    fn default() -> Self {
        Self::REPLACE
    }
}

/// Describe the blend state of a render pipeline,
/// within [`ColorTargetState`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BlendState {
    /// Color equation.
    pub color: BlendComponent,
    /// Alpha equation.
    pub alpha: BlendComponent,
}

impl BlendState {
    /// Blend mode that does no color blending, just overwrites the output with the contents of the shader.
    pub const REPLACE: Self = Self {
        color: BlendComponent::REPLACE,
        alpha: BlendComponent::REPLACE,
    };

    /// Blend mode that does standard alpha blending with non-premultiplied alpha.
    pub const ALPHA_BLENDING: Self = Self {
        color: BlendComponent {
            src_factor: BlendFactor::SrcAlpha,
            dst_factor: BlendFactor::OneMinusSrcAlpha,
            operation: BlendOperation::Add,
        },
        alpha: BlendComponent::OVER,
    };

    /// Blend mode that does standard alpha blending with premultiplied alpha.
    pub const PREMULTIPLIED_ALPHA_BLENDING: Self = Self {
        color: BlendComponent::OVER,
        alpha: BlendComponent::OVER,
    };

    /// Blend mode that just adds the value.
    pub const ADDITIVE: Self = Self {
        color: BlendComponent::ADDITIVE,
        alpha: BlendComponent::ADDITIVE,
    };
}

bitflags::bitflags! {
    /// Color write mask. Disabled color channels will not be written to.
    #[repr(transparent)]
    #[derive(Copy, Clone, Debug, Hash, Eq, PartialEq)]
    pub struct ColorWrites: u32 {
        /// Enable red channel writes
        const RED = 1 << 0;
        /// Enable green channel writes
        const GREEN = 1 << 1;
        /// Enable blue channel writes
        const BLUE = 1 << 2;
        /// Enable alpha channel writes
        const ALPHA = 1 << 3;
        /// Enable red, green, and blue channel writes
        const COLOR = Self::RED.bits() | Self::GREEN.bits() | Self::BLUE.bits();
        /// Enable writes to all channels.
        const ALL = Self::RED.bits() | Self::GREEN.bits() | Self::BLUE.bits() | Self::ALPHA.bits();
    }
}

impl Default for ColorWrites {
    fn default() -> Self {
        Self::ALL
    }
}

/// Describes the color state of a render pipeline.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ColorTargetState {
    /// The [`TextureFormat`] of the image that this pipeline will render to.
    pub format: TextureFormat,
    /// The blending that is used for this pipeline.
    pub blend: Option<BlendState>,
    /// Mask which enables/disables writes to different color/alpha channel.
    pub write_mask: ColorWrites,
}

impl From<TextureFormat> for ColorTargetState {
    fn from(format: TextureFormat) -> Self {
        Self {
            format,
            blend: None,
            write_mask: ColorWrites::ALL,
        }
    }
}

pub struct RenderPipelineDesc<'a> {
    pub name: &'a str,
    pub data_layouts: &'a [&'a ShaderDataLayout],
    pub vertex: ShaderFunction<'a>,
    pub vertex_fetches: &'a [VertexFetchState<'a>],
    pub primitive: PrimitiveState,
    pub depth_stencil: Option<DepthStencilState>,
    pub fragment: Option<ShaderFunction<'a>>,
    pub color_targets: &'a [ColorTargetState],
    pub multisample_state: MultisampleState,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct MultisampleState {
    pub sample_count: u32,
    pub sample_mask: u64,
    pub alpha_to_coverage: bool,
}

impl Default for MultisampleState {
    fn default() -> Self {
        Self {
            sample_count: 1,
            sample_mask: !0,
            alpha_to_coverage: false,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum InitOp {
    Load,
    Clear(TextureColor),
    DontCare,
}

#[derive(Clone, Copy, Debug)]
pub enum FinishOp {
    Store,
    Discard,
    /// The texture specified here will be stored but it is undefined what
    /// happens to the original render target
    ResolveTo(TextureView),
    Ignore,
}

#[derive(Debug)]
pub struct RenderTarget {
    pub view: TextureView,
    pub init_op: InitOp,
    pub finish_op: FinishOp,
}

#[derive(Debug)]
pub struct RenderTargetSet<'a> {
    pub colors: &'a [RenderTarget],
    pub depth_stencil: Option<RenderTarget>,
}

/// Mechanism used to acquire frames and display them on screen.
#[derive(Copy, Clone, Debug, Default, Hash, Eq, PartialEq)]
pub enum DisplaySync {
    /// Block until the oldest frame is released.
    #[default]
    Block,
    /// Display the most recently presented frame.
    /// Falls back to `Tear` if unsupported.
    Recent,
    /// Tear the currently displayed frame when presenting a new one.
    Tear,
}

#[derive(Copy, Clone, Debug, Default, Hash, Eq, PartialEq)]
pub enum ColorSpace {
    #[default]
    Linear,
    Srgb,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SurfaceConfig {
    pub size: Extent,
    pub usage: TextureUsage,
    pub display_sync: DisplaySync,
    /// The color space that render output colors are expected to be in.
    ///
    /// This will affect the surface format returned by the `Context`.
    ///
    /// For example, if the display expects sRGB space and we render
    /// in `ColorSpace::Linear` space, the returned format will be sRGB.
    pub color_space: ColorSpace,
    pub transparent: bool,
    pub allow_exclusive_full_screen: bool,
}

#[derive(Copy, Clone, Debug, Default, Hash, Eq, PartialEq)]
pub enum AlphaMode {
    #[default]
    Ignored,
    PreMultiplied,
    PostMultiplied,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceInfo {
    pub format: TextureFormat,
    pub alpha: AlphaMode,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum IndexType {
    U16,
    U32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ScissorRect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Viewport {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub depth: std::ops::Range<f32>,
}

pub type Timings = Vec<(String, std::time::Duration)>;
