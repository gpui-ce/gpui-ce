mod command;
mod pipeline;
#[cfg_attr(not(target_arch = "wasm32"), path = "egl.rs")]
#[cfg_attr(target_arch = "wasm32", path = "web.rs")]
mod platform;
mod resource;

use std::{marker::PhantomData, mem, ops::Range};

type BindTarget = u32;
const DEBUG_ID: u32 = 0;
const MAX_TIMEOUT: u64 = 1_000_000_000; // MAX_CLIENT_WAIT_TIMEOUT_WEBGL;
const MAX_QUERIES: usize = crate::limits::PASS_COUNT + 1;

pub use platform::PlatformError;

bitflags::bitflags! {
    struct Capabilities: u32 {
        const BUFFER_STORAGE = 1 << 0;
        const DRAW_BUFFERS_INDEXED = 1 << 1;
        const DISJOINT_TIMER_QUERY = 1 << 2;
    }
}

#[derive(Clone, Debug)]
struct Limits {
    uniform_buffer_alignment: u32,
}

#[derive(Debug, Default)]
struct Toggles {
    scoping: bool,
    timing: bool,
}

pub struct Context {
    platform: platform::PlatformContext,
    capabilities: Capabilities,
    toggles: Toggles,
    limits: Limits,
    device_information: crate::DeviceInformation,
}

pub struct Surface {
    platform: platform::PlatformSurface,
    renderbuf: glow::Renderbuffer,
    framebuf: glow::Framebuffer,
}

#[derive(Clone, Copy, Debug, Hash, PartialEq)]
pub struct Buffer {
    raw: glow::Buffer,
    size: u64,
    data: *mut u8,
}

unsafe impl Send for Buffer {}
unsafe impl Sync for Buffer {}

impl Buffer {
    pub fn data(&self) -> *mut u8 {
        self.data
    }
}

#[derive(Clone, Copy, Debug, Hash, PartialEq)]
enum TextureInner {
    Renderbuffer {
        raw: glow::Renderbuffer,
    },
    Texture {
        raw: glow::Texture,
        target: BindTarget,
    },
}

impl TextureInner {
    fn as_native(&self) -> (glow::Texture, BindTarget) {
        match *self {
            Self::Renderbuffer { .. } => {
                panic!("Unexpected renderbuffer");
            }
            Self::Texture { raw, target } => (raw, target),
        }
    }
}

#[derive(Clone, Copy, Debug, Hash, PartialEq)]
pub struct Texture {
    inner: TextureInner,
    target_size: [u16; 2],
    format: crate::TextureFormat,
}

#[derive(Clone, Copy, Debug, Hash, PartialEq)]
pub struct TextureView {
    inner: TextureInner,
    target_size: [u16; 2],
    aspects: crate::TexelAspects,
}

#[derive(Clone, Copy, Debug, Hash, PartialEq)]
pub struct Sampler {
    raw: glow::Sampler,
}

#[derive(Clone, Copy, Debug, Hash, PartialEq)]
pub struct AccelerationStructure {}

type SlotList = Vec<u32>;

struct ShaderDataMapping {
    targets: Box<[SlotList]>,
}

struct VertexAttributeInfo {
    attrib: crate::VertexAttribute,
    buffer_index: u32,
    stride: i32,
    instanced: bool,
    location: u32,
}

struct PipelineInner {
    program: glow::Program,
    group_mappings: Box<[ShaderDataMapping]>,
    vertex_attribute_infos: Box<[VertexAttributeInfo]>,
    color_targets: Box<[(Option<crate::BlendState>, crate::ColorWrites)]>,
}

pub struct ComputePipeline {
    inner: PipelineInner,
    wg_size: [u32; 3],
}

impl ComputePipeline {
    pub fn get_workgroup_size(&self) -> [u32; 3] {
        self.wg_size
    }
}

pub struct RenderPipeline {
    inner: PipelineInner,
    topology: crate::PrimitiveTopology,
}

#[derive(Debug)]
pub struct Frame {
    platform: platform::PlatformFrame,
    texture: Texture,
}

impl Frame {
    pub fn texture(&self) -> Texture {
        self.texture
    }

    pub fn texture_view(&self) -> TextureView {
        TextureView {
            inner: self.texture.inner,
            target_size: self.texture.target_size,
            aspects: crate::TexelAspects::COLOR,
        }
    }
}

#[derive(Clone, Debug)]
struct BufferPart {
    raw: glow::Buffer,
    offset: u64,
}
impl From<crate::BufferPiece> for BufferPart {
    fn from(piece: crate::BufferPiece) -> Self {
        Self {
            raw: piece.buffer.raw,
            offset: piece.offset,
        }
    }
}

#[derive(Clone, Debug)]
struct TexturePart {
    raw: glow::Texture,
    target: BindTarget,
    format: crate::TextureFormat,
    mip_level: u32,
    array_layer: u32,
    origin: [u32; 3],
}
impl From<crate::TexturePiece> for TexturePart {
    fn from(piece: crate::TexturePiece) -> Self {
        let (raw, target) = piece.texture.inner.as_native();
        Self {
            raw,
            target,
            format: piece.texture.format,
            mip_level: piece.mip_level,
            array_layer: piece.array_layer,
            origin: piece.origin,
        }
    }
}

#[derive(Clone, Debug)]
struct ImageBinding {
    raw: glow::Texture,
    mip_level: u32,
    array_layer: Option<u32>,
    access: u32,
    format: u32,
}

#[derive(Clone, Copy, Debug)]
enum ColorType {
    Float,
    Uint,
    Sint,
}

#[derive(Debug)]
enum Command {
    Draw {
        topology: u32,
        start_vertex: u32,
        vertex_count: u32,
        instance_count: u32,
    },
    DrawIndexed {
        topology: u32,
        index_buf: BufferPart,
        index_type: u32,
        index_count: u32,
        base_vertex: i32,
        instance_count: u32,
    },
    DrawIndirect {
        topology: u32,
        indirect_buf: BufferPart,
    },
    DrawIndexedIndirect {
        topology: u32,
        raw_index_buf: glow::Buffer,
        index_type: u32,
        indirect_buf: BufferPart,
    },
    Dispatch([u32; 3]),
    DispatchIndirect {
        indirect_buf: BufferPart,
    },
    FillBuffer {
        dst: BufferPart,
        size: u64,
        value: u8,
    },
    CopyBufferToBuffer {
        src: BufferPart,
        dst: BufferPart,
        size: u64,
    },
    CopyTextureToTexture {
        src: TexturePart,
        dst: TexturePart,
        size: crate::Extent,
    },
    CopyBufferToTexture {
        src: BufferPart,
        dst: TexturePart,
        bytes_per_row: u32,
        size: crate::Extent,
    },
    CopyTextureToBuffer {
        src: TexturePart,
        dst: BufferPart,
        bytes_per_row: u32,
        size: crate::Extent,
    },
    ResetFramebuffer,
    BlitFramebuffer {
        from: TextureView,
        to: TextureView,
    },
    BindAttachment {
        attachment: u32,
        view: TextureView,
    },
    InvalidateAttachment(u32),
    SetDrawColorBuffers(u8),
    SetAllColorTargets(Option<crate::BlendState>, crate::ColorWrites),
    SetSingleColorTarget(u32, Option<crate::BlendState>, crate::ColorWrites),
    ClearColor {
        draw_buffer: u32,
        color: crate::TextureColor,
        ty: ColorType,
    },
    ClearDepthStencil {
        depth: Option<f32>,
        stencil: Option<u32>,
    },
    Barrier,
    SetViewport(crate::Viewport),
    SetScissor(crate::ScissorRect),
    SetStencilFunc {
        face: u32,
        function: u32,
        reference: u32,
        read_mask: u32,
    },
    SetStencilOps {
        face: u32,
        write_mask: u32,
        //ops: crate::StencilOps,
    },
    //SetDepth(DepthState),
    //SetDepthBias(wgt::DepthBiasState),
    //ConfigureDepthStencil(crate::FormatAspects),
    SetProgram(glow::Program),
    UnsetProgram,
    //SetPrimitive(PrimitiveState),
    SetBlendConstant([f32; 4]),
    SetColorTarget {
        draw_buffer_index: Option<u32>,
        //desc: ColorTargetDesc,
    },
    BindUniform {
        slot: u32,
        offset: u32,
        size: u32,
    },
    BindVertex {
        buffer: glow::Buffer,
    },
    BindBuffer {
        target: BindTarget,
        slot: u32,
        buffer: BufferPart,
        size: u32,
    },
    SetVertexAttribute {
        index: u32,
        format: crate::VertexFormat,
        offset: i32,
        stride: i32,
        instanced: bool,
    },
    DisableVertexAttributes {
        count: u32,
    },
    BindSampler {
        slot: u32,
        sampler: glow::Sampler,
    },
    BindTexture {
        slot: u32,
        texture: glow::Texture,
        target: BindTarget,
    },
    BindImage {
        slot: u32,
        binding: ImageBinding,
    },
    ResetAllSamplers,
    QueryCounter {
        query: glow::Query,
    },
    PushScope {
        name_range: Range<usize>,
    },
    PopScope,
}

struct TimingData {
    pass_names: Vec<String>,
    queries: Box<[glow::Query]>,
}

pub struct CommandEncoder {
    name: String,
    commands: Vec<Command>,
    plain_data: Vec<u8>,
    string_data: Vec<u8>,
    needs_scopes: bool,
    present_frames: Vec<platform::PlatformFrame>,
    limits: Limits,
    timing_datas: Option<Box<[TimingData]>>,
    timings: crate::Timings,
}

enum PassKind {
    Transfer,
    Compute,
    Render,
}

pub struct PassEncoder<'a, P> {
    commands: &'a mut Vec<Command>,
    plain_data: &'a mut Vec<u8>,
    kind: PassKind,
    invalidate_attachments: Vec<u32>,
    pipeline: PhantomData<P>,
    limits: &'a Limits,
    has_scope: bool,
}

pub type ComputeCommandEncoder<'a> = PassEncoder<'a, ComputePipeline>;
pub type RenderCommandEncoder<'a> = PassEncoder<'a, RenderPipeline>;

pub struct PipelineEncoder<'a> {
    commands: &'a mut Vec<Command>,
    plain_data: &'a mut Vec<u8>,
    group_mappings: &'a [ShaderDataMapping],
    topology: u32,
    limits: &'a Limits,
    vertex_attributes: &'a [VertexAttributeInfo],
}

impl Drop for PipelineEncoder<'_> {
    fn drop(&mut self) {
        if !self.vertex_attributes.is_empty() {
            let count = self.vertex_attributes.len() as u32;
            self.commands
                .push(Command::DisableVertexAttributes { count });
        }
    }
}

pub struct PipelineContext<'a> {
    commands: &'a mut Vec<Command>,
    plain_data: &'a mut Vec<u8>,
    targets: &'a [SlotList],
    limits: &'a Limits,
}

#[derive(Clone, Debug)]
pub struct SyncPoint {
    fence: glow::Fence,
}
//TODO: destructor

struct ExecutionContext {
    framebuf: glow::Framebuffer,
    plain_buffer: glow::Buffer,
    string_data: Box<[u8]>,
}

impl Context {
    pub fn capabilities(&self) -> crate::Capabilities {
        crate::Capabilities {
            ray_query: crate::ShaderVisibility::empty(),
            sample_count_mask: 0x1 | 0x4, //TODO: accurate info
        }
    }

    pub fn device_information(&self) -> &crate::DeviceInformation {
        &self.device_information
    }
}

#[hidden_trait::expose]
impl crate::traits::CommandDevice for Context {
    type CommandEncoder = CommandEncoder;
    type SyncPoint = SyncPoint;

    fn create_command_encoder(&self, desc: super::CommandEncoderDesc) -> CommandEncoder {
        use glow::HasContext as _;

        let timing_datas = if self.toggles.timing {
            let gl = self.lock();
            let mut array = Vec::new();
            // Allocating one extra set of timers because we are resolving them
            // in submit() as opposed to start().
            for _ in 0..desc.buffer_count + 1 {
                array.push(TimingData {
                    pass_names: Vec::new(),
                    queries: (0..MAX_QUERIES)
                        .map(|_| unsafe { gl.create_query().unwrap() })
                        .collect(),
                });
            }
            Some(array.into_boxed_slice())
        } else {
            None
        };
        CommandEncoder {
            name: desc.name.to_string(),
            commands: Vec::new(),
            plain_data: Vec::new(),
            string_data: Vec::new(),
            needs_scopes: self.toggles.scoping,
            present_frames: Vec::new(),
            limits: self.limits.clone(),
            timing_datas,
            timings: Default::default(),
        }
    }

    fn destroy_command_encoder(&self, encoder: &mut CommandEncoder) {
        use glow::HasContext as _;

        if let Some(timing_datas) = encoder.timing_datas.take() {
            let gl = self.lock();
            for td in timing_datas {
                for query in td.queries {
                    unsafe { gl.delete_query(query) };
                }
            }
        }
    }

    fn submit(&self, encoder: &mut CommandEncoder) -> SyncPoint {
        use glow::HasContext as _;

        let fence = {
            let gl = self.lock();
            encoder.finish(&gl);

            let push_group = !encoder.name.is_empty() && gl.supports_debug();
            let ec = unsafe {
                if push_group {
                    gl.push_debug_group(glow::DEBUG_SOURCE_APPLICATION, DEBUG_ID, &encoder.name);
                }
                let framebuf = gl.create_framebuffer().unwrap();
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(framebuf));
                let plain_buffer = gl.create_buffer().unwrap();
                if !encoder.plain_data.is_empty() {
                    log::trace!("Allocating plain data of size {}", encoder.plain_data.len());
                    gl.bind_buffer(glow::UNIFORM_BUFFER, Some(plain_buffer));
                    gl.buffer_data_u8_slice(
                        glow::UNIFORM_BUFFER,
                        &encoder.plain_data,
                        glow::STATIC_DRAW,
                    );
                }
                ExecutionContext {
                    framebuf,
                    plain_buffer,
                    string_data: mem::take(&mut encoder.string_data).into_boxed_slice(),
                }
            };
            for command in encoder.commands.iter() {
                log::trace!("{:?}", command);
                unsafe { command.execute(&*gl, &ec) };
            }
            unsafe {
                gl.delete_framebuffer(ec.framebuf);
                gl.delete_buffer(ec.plain_buffer);
                if push_group {
                    gl.pop_debug_group();
                }
                gl.fence_sync(glow::SYNC_GPU_COMMANDS_COMPLETE, 0).unwrap()
            }
        };
        for frame in encoder.present_frames.drain(..) {
            self.platform.present(frame);
        }
        SyncPoint { fence }
    }

    fn wait_for(&self, sp: &SyncPoint, timeout_ms: u32) -> bool {
        use glow::HasContext as _;

        let gl = self.lock();
        let timeout_ns = if timeout_ms == !0 {
            !0
        } else {
            timeout_ms as u64 * 1_000_000
        };
        //TODO: https://github.com/grovesNL/glow/issues/287
        let timeout_ns_i32 = timeout_ns.min(MAX_TIMEOUT) as i32;

        let status =
            unsafe { gl.client_wait_sync(sp.fence, glow::SYNC_FLUSH_COMMANDS_BIT, timeout_ns_i32) };
        match status {
            glow::ALREADY_SIGNALED | glow::CONDITION_SATISFIED => true,
            _ => false,
        }
    }
}

// Align the size up to 16 bytes, as expected by GL.
fn round_up_uniform_size(size: u32) -> u32 {
    if size & 0xF != 0 {
        (size | 0xF) + 1
    } else {
        size
    }
}

struct FormatInfo {
    internal: u32,
    external: u32,
    data_type: u32,
}

fn describe_texture_format(format: crate::TextureFormat) -> FormatInfo {
    use crate::TextureFormat as Tf;
    let (internal, external, data_type) = match format {
        Tf::R8Unorm => (glow::R8, glow::RED, glow::UNSIGNED_BYTE),
        Tf::Rg8Unorm => (glow::RG8, glow::RG, glow::UNSIGNED_BYTE),
        Tf::Rg8Snorm => (glow::RG8, glow::RG, glow::BYTE),
        Tf::Rgba8Unorm => (glow::RGBA8, glow::RGBA, glow::UNSIGNED_BYTE),
        Tf::Rgba8UnormSrgb => (glow::SRGB8_ALPHA8, glow::RGBA, glow::UNSIGNED_BYTE),
        Tf::Bgra8Unorm => (glow::RGBA8, glow::BGRA, glow::UNSIGNED_BYTE),
        Tf::Bgra8UnormSrgb => (glow::SRGB8_ALPHA8, glow::BGRA, glow::UNSIGNED_BYTE),
        Tf::Rgba8Snorm => (glow::RGBA8, glow::RGBA, glow::BYTE),
        Tf::R16Float => (glow::R16F, glow::RED, glow::HALF_FLOAT),
        Tf::Rg16Float => (glow::RG16F, glow::RG, glow::HALF_FLOAT),
        Tf::Rgba16Float => (glow::RGBA16F, glow::RGBA, glow::HALF_FLOAT),
        Tf::R32Float => (glow::R32F, glow::RED, glow::FLOAT),
        Tf::Rg32Float => (glow::RG32F, glow::RG, glow::FLOAT),
        Tf::Rgba32Float => (glow::RGBA32F, glow::RGBA, glow::FLOAT),
        Tf::R32Uint => (glow::R32UI, glow::RED, glow::UNSIGNED_INT),
        Tf::Rg32Uint => (glow::RG32UI, glow::RG, glow::UNSIGNED_INT),
        Tf::Rgba32Uint => (glow::RGBA32UI, glow::RGBA, glow::UNSIGNED_INT),
        Tf::Depth32Float => (glow::DEPTH_COMPONENT32F, glow::DEPTH_COMPONENT, glow::FLOAT),
        Tf::Depth32FloatStencil8Uint => (
            glow::DEPTH32F_STENCIL8,
            glow::DEPTH_STENCIL,
            glow::FLOAT_32_UNSIGNED_INT_24_8_REV,
        ),
        Tf::Stencil8Uint => (
            glow::STENCIL_INDEX8,
            glow::STENCIL_INDEX,
            glow::UNSIGNED_BYTE,
        ),
        Tf::Bc1Unorm => (glow::COMPRESSED_RGBA_S3TC_DXT1_EXT, glow::RGBA, 0),
        Tf::Bc1UnormSrgb => (glow::COMPRESSED_SRGB_ALPHA_S3TC_DXT1_EXT, glow::RGBA, 0),
        Tf::Bc2Unorm => (glow::COMPRESSED_RGBA_S3TC_DXT3_EXT, glow::RGBA, 0),
        Tf::Bc2UnormSrgb => (glow::COMPRESSED_SRGB_ALPHA_S3TC_DXT3_EXT, glow::RGBA, 0),
        Tf::Bc3Unorm => (glow::COMPRESSED_RGBA_S3TC_DXT5_EXT, glow::RGBA, 0),
        Tf::Bc3UnormSrgb => (glow::COMPRESSED_SRGB_ALPHA_S3TC_DXT5_EXT, glow::RGBA, 0),
        Tf::Bc4Unorm => (glow::COMPRESSED_RED_RGTC1, glow::RED, 0),
        Tf::Bc4Snorm => (glow::COMPRESSED_SIGNED_RED_RGTC1, glow::RED, 0),
        Tf::Bc5Unorm => (glow::COMPRESSED_RG_RGTC2, glow::RG, 0),
        Tf::Bc5Snorm => (glow::COMPRESSED_SIGNED_RG_RGTC2, glow::RG, 0),
        Tf::Bc6hUfloat => (glow::COMPRESSED_RGB_BPTC_UNSIGNED_FLOAT, glow::RGB, 0),
        Tf::Bc6hFloat => (glow::COMPRESSED_RGB_BPTC_SIGNED_FLOAT, glow::RGB, 0),
        Tf::Bc7Unorm => (glow::COMPRESSED_RGBA_BPTC_UNORM, glow::RGBA, 0),
        Tf::Bc7UnormSrgb => (glow::COMPRESSED_SRGB_ALPHA_BPTC_UNORM, glow::RGBA, 0),
        Tf::Rgb10a2Unorm => (
            glow::RGB10_A2,
            glow::RGBA,
            glow::UNSIGNED_INT_2_10_10_10_REV,
        ),
        Tf::Rg11b10Ufloat => (
            glow::R11F_G11F_B10F,
            glow::RGB,
            glow::UNSIGNED_INT_10F_11F_11F_REV,
        ),
        Tf::Rgb9e5Ufloat => (glow::RGB9_E5, glow::RGB, glow::UNSIGNED_INT_5_9_9_9_REV),
    };
    FormatInfo {
        internal,
        external,
        data_type,
    }
}

fn map_compare_func(fun: crate::CompareFunction) -> u32 {
    use crate::CompareFunction as Cf;
    match fun {
        Cf::Never => glow::NEVER,
        Cf::Less => glow::LESS,
        Cf::LessEqual => glow::LEQUAL,
        Cf::Equal => glow::EQUAL,
        Cf::GreaterEqual => glow::GEQUAL,
        Cf::Greater => glow::GREATER,
        Cf::NotEqual => glow::NOTEQUAL,
        Cf::Always => glow::ALWAYS,
    }
}

unsafe fn present_blit(gl: &glow::Context, source: glow::Framebuffer, size: crate::Extent) {
    use glow::HasContext as _;

    gl.disable(glow::SCISSOR_TEST);
    gl.color_mask(true, true, true, true);
    gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, None);
    gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(source));
    // Note: the Y-flipping here. GL's presentation is not flipped,
    // but main rendering is. Therefore, we Y-flip the output positions
    // in the shader, and also this blit.
    // Note2: we could avoid doing both and get correct rendering for the main window
    // but then other render targets would be screwed.
    gl.blit_framebuffer(
        0,
        size.height as i32,
        size.width as i32,
        0,
        0,
        0,
        size.width as i32,
        size.height as i32,
        glow::COLOR_BUFFER_BIT,
        glow::NEAREST,
    );
    gl.bind_framebuffer(glow::READ_FRAMEBUFFER, None);
}
