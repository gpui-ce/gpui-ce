use objc2::{rc::Retained, runtime::ProtocolObject};
use objc2_metal::{self as metal, MTLDevice};
use std::{
    marker::PhantomData,
    ptr,
    sync::{Arc, Mutex},
    thread, time,
};

mod command;
mod pipeline;
mod resource;
mod surface;

const MAX_TIMESTAMPS: usize = crate::limits::PASS_COUNT * 2;

pub type PlatformError = ();

pub struct Surface {
    view: Option<objc2::rc::Retained<objc2::runtime::NSObject>>,
    render_layer: Retained<objc2_quartz_core::CAMetalLayer>,
    info: crate::SurfaceInfo,
}

#[derive(Debug)]
pub struct Frame {
    drawable: Retained<ProtocolObject<dyn metal::MTLDrawable>>,
    texture: Retained<ProtocolObject<dyn metal::MTLTexture>>,
}

unsafe impl Send for Frame {}
unsafe impl Sync for Frame {}
impl Frame {
    pub fn texture(&self) -> Texture {
        Texture {
            raw: Retained::as_ptr(&self.texture) as *mut _,
        }
    }

    pub fn texture_view(&self) -> TextureView {
        TextureView {
            raw: Retained::as_ptr(&self.texture) as *mut _,
            aspects: crate::TexelAspects::COLOR,
        }
    }
}

#[derive(Debug, Clone)]
struct PrivateInfo {
    language_version: metal::MTLLanguageVersion,
    enable_debug_groups: bool,
    enable_dispatch_type: bool,
}

pub struct Context {
    device: Mutex<Retained<ProtocolObject<dyn metal::MTLDevice>>>,
    queue: Arc<Mutex<Retained<ProtocolObject<dyn metal::MTLCommandQueue>>>>,
    capture: Option<Retained<metal::MTLCaptureManager>>,
    timestamp_counter_set: Option<Retained<ProtocolObject<dyn metal::MTLCounterSet>>>,
    info: PrivateInfo,
    device_information: crate::DeviceInformation,
}

// needed for `capture` and `timestamp_counter_set`
unsafe impl Send for Context {}
unsafe impl Sync for Context {}

#[derive(Clone, Copy, Debug, Hash, PartialEq)]
pub struct Buffer {
    raw: *mut ProtocolObject<dyn metal::MTLBuffer>,
}

unsafe impl Send for Buffer {}
unsafe impl Sync for Buffer {}

impl Default for Buffer {
    fn default() -> Self {
        Self {
            raw: ptr::null_mut(),
        }
    }
}

impl Buffer {
    fn as_ref(&self) -> &ProtocolObject<dyn metal::MTLBuffer> {
        unsafe { &*self.raw }
    }

    pub fn data(&self) -> *mut u8 {
        use metal::MTLBuffer as _;
        self.as_ref().contents().as_ptr() as *mut u8
    }
}

#[derive(Clone, Copy, Debug, Hash, PartialEq)]
pub struct Texture {
    raw: *mut ProtocolObject<dyn metal::MTLTexture>,
}

unsafe impl Send for Texture {}
unsafe impl Sync for Texture {}

impl Default for Texture {
    fn default() -> Self {
        Self {
            raw: ptr::null_mut(),
        }
    }
}

impl Texture {
    fn as_ref(&self) -> &ProtocolObject<dyn metal::MTLTexture> {
        unsafe { &*self.raw }
    }
}

#[derive(Clone, Copy, Debug, Hash, PartialEq)]
pub struct TextureView {
    raw: *mut ProtocolObject<dyn metal::MTLTexture>,
    aspects: crate::TexelAspects,
}

unsafe impl Send for TextureView {}
unsafe impl Sync for TextureView {}

impl Default for TextureView {
    fn default() -> Self {
        Self {
            raw: ptr::null_mut(),
            aspects: crate::TexelAspects::COLOR,
        }
    }
}

impl TextureView {
    fn as_ref(&self) -> &ProtocolObject<dyn metal::MTLTexture> {
        unsafe { &*self.raw }
    }

    /// Create a TextureView from a raw Metal Texture.
    /// Does not keep a reference, need not being destoryed.
    pub fn from_metal_texture(
        raw: &Retained<ProtocolObject<dyn metal::MTLTexture>>,
        aspects: crate::TexelAspects,
    ) -> Self {
        Self {
            raw: Retained::into_raw(raw.clone()),
            aspects,
        }
    }
}

#[derive(Clone, Copy, Debug, Hash, PartialEq)]
pub struct Sampler {
    raw: *mut ProtocolObject<dyn metal::MTLSamplerState>,
}

unsafe impl Send for Sampler {}
unsafe impl Sync for Sampler {}

impl Default for Sampler {
    fn default() -> Self {
        Self {
            raw: ptr::null_mut(),
        }
    }
}

impl Sampler {
    fn as_ref(&self) -> &ProtocolObject<dyn metal::MTLSamplerState> {
        unsafe { &*self.raw }
    }
}

#[derive(Clone, Copy, Debug, Hash, PartialEq)]
pub struct AccelerationStructure {
    raw: *mut ProtocolObject<dyn metal::MTLAccelerationStructure>,
}

unsafe impl Send for AccelerationStructure {}
unsafe impl Sync for AccelerationStructure {}

impl Default for AccelerationStructure {
    fn default() -> Self {
        Self {
            raw: ptr::null_mut(),
        }
    }
}

impl AccelerationStructure {
    fn as_ref(&self) -> &ProtocolObject<dyn metal::MTLAccelerationStructure> {
        unsafe { &*self.raw }
    }
    fn as_retained(&self) -> Retained<ProtocolObject<dyn metal::MTLAccelerationStructure>> {
        unsafe { Retained::retain(self.raw).unwrap() }
    }
}

//TODO: make this copyable?
#[derive(Clone, Debug)]
pub struct SyncPoint {
    cmd_buf: Retained<ProtocolObject<dyn metal::MTLCommandBuffer>>,
}
// Safe because all mutability is externalized
unsafe impl Send for SyncPoint {}
unsafe impl Sync for SyncPoint {}

struct TimingData {
    pass_names: Vec<String>,
    sample_buffer: Retained<ProtocolObject<dyn metal::MTLCounterSampleBuffer>>,
}

type RawCommandBuffer = Retained<ProtocolObject<dyn metal::MTLCommandBuffer>>;
pub struct CommandEncoder {
    raw: Option<RawCommandBuffer>,
    name: String,
    queue: Arc<Mutex<Retained<ProtocolObject<dyn metal::MTLCommandQueue>>>>,
    enable_debug_groups: bool,
    enable_dispatch_type: bool,
    has_open_debug_group: bool,
    timing_datas: Option<Box<[TimingData]>>,
    timings: crate::Timings,
}

#[derive(Debug)]
struct ShaderDataMapping {
    visibility: crate::ShaderVisibility,
    targets: Box<[u32]>,
}

#[derive(Debug)]
struct PipelineLayout {
    group_mappings: Box<[ShaderDataMapping]>,
    group_infos: Box<[crate::ShaderDataInfo]>,
    sizes_buffer_slot: Option<u32>,
}

pub struct ComputePipeline {
    raw: Retained<ProtocolObject<dyn metal::MTLComputePipelineState>>,
    name: String,
    #[allow(dead_code)]
    lib: Retained<ProtocolObject<dyn metal::MTLLibrary>>,
    layout: PipelineLayout,
    wg_size: metal::MTLSize,
    wg_memory_sizes: Box<[u32]>,
}

unsafe impl Send for ComputePipeline {}
unsafe impl Sync for ComputePipeline {}
impl ComputePipeline {
    pub fn get_workgroup_size(&self) -> [u32; 3] {
        [
            self.wg_size.width as u32,
            self.wg_size.height as u32,
            self.wg_size.depth as u32,
        ]
    }
}

pub struct RenderPipeline {
    raw: Retained<ProtocolObject<dyn metal::MTLRenderPipelineState>>,
    name: String,
    #[allow(dead_code)]
    vs_lib: Retained<ProtocolObject<dyn metal::MTLLibrary>>,
    #[allow(dead_code)]
    fs_lib: Option<Retained<ProtocolObject<dyn metal::MTLLibrary>>>,
    layout: PipelineLayout,
    primitive_type: metal::MTLPrimitiveType,
    triangle_fill_mode: metal::MTLTriangleFillMode,
    front_winding: metal::MTLWinding,
    cull_mode: metal::MTLCullMode,
    depth_clip_mode: metal::MTLDepthClipMode,
    depth_stencil: Option<(
        Retained<ProtocolObject<dyn metal::MTLDepthStencilState>>,
        super::DepthBiasState,
    )>,
}

unsafe impl Send for RenderPipeline {}
unsafe impl Sync for RenderPipeline {}

pub struct TransferCommandEncoder<'a> {
    raw: Retained<ProtocolObject<dyn metal::MTLBlitCommandEncoder>>,
    phantom: PhantomData<&'a CommandEncoder>,
}

pub struct AccelerationStructureCommandEncoder<'a> {
    raw: Retained<ProtocolObject<dyn metal::MTLAccelerationStructureCommandEncoder>>,
    phantom: PhantomData<&'a CommandEncoder>,
}

pub struct ComputeCommandEncoder<'a> {
    raw: Retained<ProtocolObject<dyn metal::MTLComputeCommandEncoder>>,
    phantom: PhantomData<&'a CommandEncoder>,
}

pub struct RenderCommandEncoder<'a> {
    raw: Retained<ProtocolObject<dyn metal::MTLRenderCommandEncoder>>,
    phantom: PhantomData<&'a CommandEncoder>,
}

pub struct PipelineContext<'a> {
    //raw: metal::ArgumentEncoderRef,
    cs_encoder: Option<&'a ProtocolObject<dyn metal::MTLComputeCommandEncoder>>,
    vs_encoder: Option<&'a ProtocolObject<dyn metal::MTLRenderCommandEncoder>>,
    fs_encoder: Option<&'a ProtocolObject<dyn metal::MTLRenderCommandEncoder>>,
    targets: &'a [u32],
}

pub struct ComputePipelineContext<'a> {
    encoder: &'a ProtocolObject<dyn metal::MTLComputeCommandEncoder>,
    wg_size: metal::MTLSize,
    group_mappings: &'a [ShaderDataMapping],
}

pub struct RenderPipelineContext<'a> {
    encoder: &'a ProtocolObject<dyn metal::MTLRenderCommandEncoder>,
    primitive_type: metal::MTLPrimitiveType,
    group_mappings: &'a [ShaderDataMapping],
}

fn map_texture_format(format: crate::TextureFormat) -> metal::MTLPixelFormat {
    use crate::TextureFormat as Tf;
    use metal::MTLPixelFormat as Mpf;
    match format {
        Tf::R8Unorm => Mpf::R8Unorm,
        Tf::Rg8Unorm => Mpf::RG8Unorm,
        Tf::Rg8Snorm => Mpf::RG8Snorm,
        Tf::Rgba8Unorm => Mpf::RGBA8Unorm,
        Tf::Rgba8UnormSrgb => Mpf::RGBA8Unorm_sRGB,
        Tf::Bgra8Unorm => Mpf::BGRA8Unorm,
        Tf::Bgra8UnormSrgb => Mpf::BGRA8Unorm_sRGB,
        Tf::Rgba8Snorm => Mpf::RGBA8Snorm,
        Tf::R16Float => Mpf::R16Float,
        Tf::Rg16Float => Mpf::RG16Float,
        Tf::Rgba16Float => Mpf::RGBA16Float,
        Tf::R32Float => Mpf::R32Float,
        Tf::Rg32Float => Mpf::RG32Float,
        Tf::Rgba32Float => Mpf::RGBA32Float,
        Tf::R32Uint => Mpf::R32Uint,
        Tf::Rg32Uint => Mpf::RG32Uint,
        Tf::Rgba32Uint => Mpf::RGBA32Uint,
        Tf::Depth32Float => Mpf::Depth32Float,
        Tf::Depth32FloatStencil8Uint => Mpf::Depth32Float_Stencil8,
        Tf::Stencil8Uint => Mpf::Stencil8,
        Tf::Bc1Unorm => Mpf::BC1_RGBA,
        Tf::Bc1UnormSrgb => Mpf::BC1_RGBA_sRGB,
        Tf::Bc2Unorm => Mpf::BC2_RGBA,
        Tf::Bc2UnormSrgb => Mpf::BC2_RGBA_sRGB,
        Tf::Bc3Unorm => Mpf::BC3_RGBA,
        Tf::Bc3UnormSrgb => Mpf::BC3_RGBA_sRGB,
        Tf::Bc4Unorm => Mpf::BC4_RUnorm,
        Tf::Bc4Snorm => Mpf::BC4_RSnorm,
        Tf::Bc5Unorm => Mpf::BC5_RGUnorm,
        Tf::Bc5Snorm => Mpf::BC5_RGSnorm,
        Tf::Bc6hUfloat => Mpf::BC6H_RGBUfloat,
        Tf::Bc6hFloat => Mpf::BC6H_RGBFloat,
        Tf::Bc7Unorm => Mpf::BC7_RGBAUnorm,
        Tf::Bc7UnormSrgb => Mpf::BC7_RGBAUnorm_sRGB,
        Tf::Rgb10a2Unorm => Mpf::RGB10A2Unorm,
        Tf::Rg11b10Ufloat => Mpf::RG11B10Float,
        Tf::Rgb9e5Ufloat => Mpf::RGB9E5Float,
    }
}

fn map_compare_function(fun: crate::CompareFunction) -> metal::MTLCompareFunction {
    use crate::CompareFunction as Cf;
    use metal::MTLCompareFunction as Mcf;
    match fun {
        Cf::Never => Mcf::Never,
        Cf::Less => Mcf::Less,
        Cf::LessEqual => Mcf::LessEqual,
        Cf::Equal => Mcf::Equal,
        Cf::GreaterEqual => Mcf::GreaterEqual,
        Cf::Greater => Mcf::Greater,
        Cf::NotEqual => Mcf::NotEqual,
        Cf::Always => Mcf::Always,
    }
}

fn map_index_type(ty: crate::IndexType) -> metal::MTLIndexType {
    match ty {
        crate::IndexType::U16 => metal::MTLIndexType::UInt16,
        crate::IndexType::U32 => metal::MTLIndexType::UInt32,
    }
}

fn map_vertex_format(
    format: crate::VertexFormat,
) -> (metal::MTLVertexFormat, metal::MTLAttributeFormat) {
    match format {
        crate::VertexFormat::F32 => (
            metal::MTLVertexFormat::Float,
            metal::MTLAttributeFormat::Float,
        ),
        crate::VertexFormat::F32Vec2 => (
            metal::MTLVertexFormat::Float2,
            metal::MTLAttributeFormat::Float2,
        ),
        crate::VertexFormat::F32Vec3 => (
            metal::MTLVertexFormat::Float3,
            metal::MTLAttributeFormat::Float3,
        ),
        crate::VertexFormat::F32Vec4 => (
            metal::MTLVertexFormat::Float4,
            metal::MTLAttributeFormat::Float4,
        ),
        crate::VertexFormat::U32 => (
            metal::MTLVertexFormat::UInt,
            metal::MTLAttributeFormat::UInt,
        ),
        crate::VertexFormat::U32Vec2 => (
            metal::MTLVertexFormat::UInt2,
            metal::MTLAttributeFormat::UInt2,
        ),
        crate::VertexFormat::U32Vec3 => (
            metal::MTLVertexFormat::UInt3,
            metal::MTLAttributeFormat::UInt3,
        ),
        crate::VertexFormat::U32Vec4 => (
            metal::MTLVertexFormat::UInt4,
            metal::MTLAttributeFormat::UInt4,
        ),
        crate::VertexFormat::I32 => (metal::MTLVertexFormat::Int, metal::MTLAttributeFormat::Int),
        crate::VertexFormat::I32Vec2 => (
            metal::MTLVertexFormat::Int2,
            metal::MTLAttributeFormat::Int2,
        ),
        crate::VertexFormat::I32Vec3 => (
            metal::MTLVertexFormat::Int3,
            metal::MTLAttributeFormat::Int3,
        ),
        crate::VertexFormat::I32Vec4 => (
            metal::MTLVertexFormat::Int4,
            metal::MTLAttributeFormat::Int4,
        ),
    }
}

impl Context {
    pub unsafe fn init(desc: super::ContextDesc) -> Result<Self, super::NotSupportedError> {
        if desc.validation {
            std::env::set_var("METAL_DEVICE_WRAPPER_TYPE", "1");
        }
        if desc.overlay {
            std::env::set_var("MTL_HUD_ENABLED", "1");
        }
        if desc.device_id != 0 {
            log::warn!("Unable to filter devices by ID");
        }

        let device = metal::MTLCreateSystemDefaultDevice()
            .ok_or(super::NotSupportedError::NoSupportedDeviceFound)?;
        let queue = device.newCommandQueue().unwrap();

        let auto_capture_everything = false;
        let capture = if desc.capture && auto_capture_everything {
            use metal::MTLCaptureScope as _;
            objc2::rc::autoreleasepool(|_| {
                let capture_manager = metal::MTLCaptureManager::sharedCaptureManager();
                let default_capture_scope = capture_manager.newCaptureScopeWithDevice(&device);
                capture_manager.setDefaultCaptureScope(Some(&default_capture_scope));
                let capture_desc = metal::MTLCaptureDescriptor::new();
                capture_desc.set_capture_scope(&default_capture_scope);
                capture_manager
                    .startCaptureWithDescriptor_error(&capture_desc)
                    .unwrap();
                default_capture_scope.beginScope();
                Some(capture_manager.to_owned())
            })
        } else {
            None
        };
        let device_information = crate::DeviceInformation {
            is_software_emulated: false,
            device_name: device.name().to_string(),
            driver_name: "Metal".to_string(),
            driver_info: "".to_string(),
        };

        let mut timestamp_counter_set = None;
        if desc.timing {
            use metal::MTLCounterSet as _;
            if let Some(counter_sets) = device.counterSets() {
                for counter_set in counter_sets {
                    if counter_set.name().to_string() == "timestamp" {
                        timestamp_counter_set = Some(counter_set);
                    }
                }
            }
            if timestamp_counter_set.is_none() {
                log::warn!("Timing counters are not supported by the device");
            } else if !device
                .supportsCounterSampling(metal::MTLCounterSamplingPoint::AtStageBoundary)
            {
                log::warn!("Timing counters do not support stage boundary");
                timestamp_counter_set = None;
            }
        }

        Ok(Context {
            device: Mutex::new(device),
            queue: Arc::new(Mutex::new(queue)),
            capture,
            timestamp_counter_set,
            info: PrivateInfo {
                //TODO: determine based on OS version
                language_version: metal::MTLLanguageVersion::Version2_4,
                enable_debug_groups: desc.capture,
                enable_dispatch_type: true,
            },
            device_information,
        })
    }

    pub fn capabilities(&self) -> crate::Capabilities {
        use metal::MTLDevice as _;
        let device = self.device.lock().unwrap();

        crate::Capabilities {
            ray_query: if device.supportsFamily(metal::MTLGPUFamily::Apple6) {
                crate::ShaderVisibility::all()
            } else if device.supportsFamily(metal::MTLGPUFamily::Mac2)
                || device.supportsFamily(metal::MTLGPUFamily::Metal3)
            {
                crate::ShaderVisibility::COMPUTE
            } else {
                crate::ShaderVisibility::empty()
            },
            sample_count_mask: (0u32..7)
                .map(|v| 1 << v)
                .filter(|&count| device.supportsTextureSampleCount(count as _))
                .sum(),
        }
    }

    pub fn device_information(&self) -> &crate::DeviceInformation {
        &self.device_information
    }

    /// Get an MTLDevice of this context.
    /// This is platform specific API.
    pub fn metal_device(&self) -> Retained<ProtocolObject<dyn metal::MTLDevice>> {
        self.device.lock().unwrap().clone()
    }
}

#[hidden_trait::expose]
impl crate::traits::CommandDevice for Context {
    type CommandEncoder = CommandEncoder;
    type SyncPoint = SyncPoint;

    fn create_command_encoder(&self, desc: super::CommandEncoderDesc) -> CommandEncoder {
        use metal::MTLDevice as _;

        let timing_datas = if let Some(ref counter_set) = self.timestamp_counter_set {
            let mut array = Vec::with_capacity(desc.buffer_count as usize);
            let csb_desc = unsafe {
                let desc = metal::MTLCounterSampleBufferDescriptor::new();
                desc.setCounterSet(Some(counter_set));
                desc.setStorageMode(metal::MTLStorageMode::Shared);
                desc.setSampleCount(MAX_TIMESTAMPS);
                desc
            };
            for i in 0..desc.buffer_count {
                let label = format!("{}/counter{}", desc.name, i);
                let sample_buffer = unsafe {
                    csb_desc.setLabel(&objc2_foundation::NSString::from_str(&label));
                    self.device
                        .lock()
                        .unwrap()
                        .newCounterSampleBufferWithDescriptor_error(&csb_desc)
                        .unwrap()
                };
                array.push(TimingData {
                    sample_buffer,
                    pass_names: Vec::new(),
                });
            }
            Some(array.into_boxed_slice())
        } else {
            None
        };

        CommandEncoder {
            raw: None,
            name: desc.name.to_string(),
            queue: Arc::clone(&self.queue),
            enable_debug_groups: self.info.enable_debug_groups,
            enable_dispatch_type: self.info.enable_dispatch_type,
            has_open_debug_group: false,
            timing_datas,
            timings: Default::default(),
        }
    }

    fn destroy_command_encoder(&self, _command_encoder: &mut CommandEncoder) {}

    fn submit(&self, encoder: &mut CommandEncoder) -> SyncPoint {
        use metal::MTLCommandBuffer as _;
        let cmd_buf = encoder.finish();
        cmd_buf.commit();
        SyncPoint { cmd_buf }
    }

    fn wait_for(&self, sp: &SyncPoint, timeout_ms: u32) -> bool {
        use metal::MTLCommandBuffer as _;
        let start = time::Instant::now();
        loop {
            if let metal::MTLCommandBufferStatus::Completed = sp.cmd_buf.status() {
                return true;
            }
            if start.elapsed().as_millis() >= timeout_ms as u128 {
                return false;
            }
            thread::sleep(time::Duration::from_millis(1));
        }
    }
}

impl Drop for Context {
    fn drop(&mut self) {
        use metal::MTLCaptureScope as _;
        if let Some(capture_manager) = self.capture.take() {
            if let Some(scope) = capture_manager.defaultCaptureScope() {
                scope.endScope();
            }
            capture_manager.stopCapture();
        }
    }
}

fn make_bottom_level_acceleration_structure_desc(
    meshes: &[crate::AccelerationStructureMesh],
) -> Retained<metal::MTLPrimitiveAccelerationStructureDescriptor> {
    let mut geometry_descriptors = Vec::with_capacity(meshes.len());
    for mesh in meshes {
        geometry_descriptors.push(unsafe {
            let descriptor = metal::MTLAccelerationStructureTriangleGeometryDescriptor::new();
            descriptor.setOpaque(mesh.is_opaque);
            descriptor.setVertexBuffer(Some(mesh.vertex_data.buffer.as_ref()));
            descriptor.setVertexBufferOffset(mesh.vertex_data.offset as usize);
            descriptor.setVertexStride(mesh.vertex_stride as _);
            descriptor.setTriangleCount(mesh.triangle_count as _);
            if let Some(index_type) = mesh.index_type {
                descriptor.setIndexBuffer(Some(mesh.index_data.buffer.as_ref()));
                descriptor.setIndexBufferOffset(mesh.index_data.offset as usize);
                descriptor.setIndexType(map_index_type(index_type));
            }
            //TODO: requires macOS-13 ?
            if false {
                let (_, attribute_format) = map_vertex_format(mesh.vertex_format);
                descriptor.setVertexFormat(attribute_format);
                if !mesh.transform_data.buffer.raw.is_null() {
                    descriptor
                        .setTransformationMatrixBuffer(Some(mesh.transform_data.buffer.as_ref()));
                    descriptor
                        .setTransformationMatrixBufferOffset(mesh.transform_data.offset as usize);
                }
            }
            Retained::cast_unchecked(descriptor)
        });
    }

    let geometry_descriptor_array =
        objc2_foundation::NSArray::from_retained_slice(&geometry_descriptors);
    let accel_descriptor = metal::MTLPrimitiveAccelerationStructureDescriptor::descriptor();
    accel_descriptor.setGeometryDescriptors(Some(&geometry_descriptor_array));
    accel_descriptor
}
