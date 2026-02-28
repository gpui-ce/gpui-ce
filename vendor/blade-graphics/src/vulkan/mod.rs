use ash::{
    khr,
    vk::{self},
};
use std::{mem, num::NonZeroU32, path::PathBuf, ptr, sync::Mutex};

mod command;
mod descriptor;
mod init;
mod pipeline;
mod resource;
mod surface;

const QUERY_POOL_SIZE: usize = crate::limits::PASS_COUNT + 1;

#[derive(Debug)]
pub enum PlatformError {
    Loading(ash::LoadingError),
    Init(vk::Result),
}

struct Instance {
    core: ash::Instance,
    _debug_utils: ash::ext::debug_utils::Instance,
    get_physical_device_properties2: khr::get_physical_device_properties2::Instance,
    get_surface_capabilities2: Option<khr::get_surface_capabilities2::Instance>,
    surface: Option<khr::surface::Instance>,
}

#[derive(Clone)]
struct RayTracingDevice {
    acceleration_structure: khr::acceleration_structure::Device,
    scratch_buffer_alignment: u64,
}

#[derive(Clone, Default)]
struct CommandScopeDevice {}
#[derive(Clone, Default)]
struct TimingDevice {
    period: f32,
}

#[derive(Clone)]
struct Workarounds {
    extra_sync_src_access: vk::AccessFlags,
    extra_sync_dst_access: vk::AccessFlags,
    extra_descriptor_pool_create_flags: vk::DescriptorPoolCreateFlags,
}

#[derive(Clone)]
struct Device {
    core: ash::Device,
    device_information: crate::DeviceInformation,
    swapchain: Option<khr::swapchain::Device>,
    debug_utils: ash::ext::debug_utils::Device,
    timeline_semaphore: khr::timeline_semaphore::Device,
    dynamic_rendering: khr::dynamic_rendering::Device,
    ray_tracing: Option<RayTracingDevice>,
    buffer_marker: Option<ash::amd::buffer_marker::Device>,
    shader_info: Option<ash::amd::shader_info::Device>,
    full_screen_exclusive: Option<ash::ext::full_screen_exclusive::Device>,
    #[cfg(target_os = "windows")]
    external_memory: Option<ash::khr::external_memory_win32::Device>,
    #[cfg(not(target_os = "windows"))]
    external_memory: Option<ash::khr::external_memory_fd::Device>,
    command_scope: Option<CommandScopeDevice>,
    timing: Option<TimingDevice>,
    workarounds: Workarounds,
}

struct MemoryManager {
    allocator: gpu_alloc::GpuAllocator<vk::DeviceMemory>,
    slab: slab::Slab<gpu_alloc::MemoryBlock<vk::DeviceMemory>>,
    valid_ash_memory_types: u32,
}

struct Queue {
    raw: vk::Queue,
    timeline_semaphore: vk::Semaphore,
    last_progress: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct InternalFrame {
    acquire_semaphore: vk::Semaphore,
    present_semaphore: vk::Semaphore,
    image: vk::Image,
    view: vk::ImageView,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Swapchain {
    raw: vk::SwapchainKHR,
    format: crate::TextureFormat,
    alpha: crate::AlphaMode,
    target_size: [u16; 2],
}

pub struct Surface {
    device: khr::swapchain::Device,
    raw: vk::SurfaceKHR,
    frames: Vec<InternalFrame>,
    next_semaphore: vk::Semaphore,
    swapchain: Swapchain,
    full_screen_exclusive: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Presentation {
    swapchain: vk::SwapchainKHR,
    image_index: u32,
    acquire_semaphore: vk::Semaphore,
    present_semaphore: vk::Semaphore,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    swapchain: Swapchain,
    image_index: Option<u32>,
    internal: InternalFrame,
}

impl Frame {
    pub fn texture(&self) -> Texture {
        Texture {
            raw: self.internal.image,
            memory_handle: !0,
            target_size: self.swapchain.target_size,
            format: self.swapchain.format,
            external: None,
        }
    }

    pub fn texture_view(&self) -> TextureView {
        TextureView {
            raw: self.internal.view,
            target_size: self.swapchain.target_size,
            aspects: crate::TexelAspects::COLOR,
        }
    }
}

fn map_timeout(millis: u32) -> u64 {
    if millis == !0 {
        !0
    } else {
        millis as u64 * 1_000_000
    }
}

pub struct Context {
    memory: Mutex<MemoryManager>,
    device: Device,
    queue_family_index: u32,
    queue: Mutex<Queue>,
    physical_device: vk::PhysicalDevice,
    naga_flags: naga::back::spv::WriterFlags,
    shader_debug_path: Option<PathBuf>,
    min_buffer_alignment: u64,
    sample_count_flags: vk::SampleCountFlags,
    instance: Instance,
    entry: ash::Entry,
}

#[derive(Clone, Copy, Debug, Hash, PartialEq)]
pub struct Buffer {
    raw: vk::Buffer,
    memory_handle: usize,
    mapped_data: *mut u8,
    external: Option<crate::ExternalMemorySource>,
}

impl Default for Buffer {
    fn default() -> Self {
        Self {
            raw: vk::Buffer::null(),
            memory_handle: !0,
            mapped_data: ptr::null_mut(),
            external: None,
        }
    }
}

impl Buffer {
    pub fn data(&self) -> *mut u8 {
        self.mapped_data
    }
}

unsafe impl Send for Buffer {}
unsafe impl Sync for Buffer {}

#[derive(Clone, Copy, Debug, Hash, PartialEq)]
pub struct Texture {
    raw: vk::Image,
    memory_handle: usize,
    target_size: [u16; 2],
    format: crate::TextureFormat,
    external: Option<crate::ExternalMemorySource>,
}

impl Default for Texture {
    fn default() -> Self {
        Self {
            raw: vk::Image::default(),
            memory_handle: !0,
            target_size: [0; 2],
            format: crate::TextureFormat::Rgba8Unorm,
            external: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Hash, PartialEq)]
pub struct TextureView {
    raw: vk::ImageView,
    target_size: [u16; 2],
    aspects: crate::TexelAspects,
}

#[derive(Clone, Copy, Debug, Hash, PartialEq)]
pub struct Sampler {
    raw: vk::Sampler,
}

#[derive(Clone, Copy, Debug, Default, Hash, PartialEq)]
pub struct AccelerationStructure {
    raw: vk::AccelerationStructureKHR,
    buffer: vk::Buffer,
    memory_handle: usize,
}

#[derive(Debug, Default)]
struct DescriptorSetLayout {
    raw: vk::DescriptorSetLayout,
    update_template: vk::DescriptorUpdateTemplate,
    template_size: u32,
    template_offsets: Box<[u32]>,
}

impl DescriptorSetLayout {
    fn is_empty(&self) -> bool {
        self.template_size == 0
    }
}

#[derive(Debug)]
struct PipelineLayout {
    raw: vk::PipelineLayout,
    descriptor_set_layouts: Vec<DescriptorSetLayout>,
}

pub struct PipelineContext<'a> {
    update_data: &'a mut [u8],
    template_offsets: &'a [u32],
}

#[derive(Debug)]
pub struct ComputePipeline {
    raw: vk::Pipeline,
    layout: PipelineLayout,
    wg_size: [u32; 3],
}

impl ComputePipeline {
    pub fn get_workgroup_size(&self) -> [u32; 3] {
        self.wg_size
    }
}

#[derive(Debug)]
pub struct RenderPipeline {
    raw: vk::Pipeline,
    layout: PipelineLayout,
}

#[derive(Debug)]
struct CommandBuffer {
    raw: vk::CommandBuffer,
    descriptor_pool: descriptor::DescriptorPool,
    query_pool: vk::QueryPool,
    timed_pass_names: Vec<String>,
}

struct CrashHandler {
    name: String,
    marker_buf: Buffer,
    raw_string: Box<[u8]>,
    next_offset: usize,
}

pub struct CommandEncoder {
    pool: vk::CommandPool,
    buffers: Box<[CommandBuffer]>,
    device: Device,
    update_data: Vec<u8>,
    present: Option<Presentation>,
    crash_handler: Option<CrashHandler>,
    temp_label: Vec<u8>,
    timings: crate::Timings,
}
pub struct TransferCommandEncoder<'a> {
    raw: vk::CommandBuffer,
    device: &'a Device,
}
pub struct AccelerationStructureCommandEncoder<'a> {
    raw: vk::CommandBuffer,
    device: &'a Device,
}
pub struct ComputeCommandEncoder<'a> {
    cmd_buf: &'a mut CommandBuffer,
    device: &'a Device,
    update_data: &'a mut Vec<u8>,
}
//Note: we aren't merging this with `ComputeCommandEncoder`
// because the destructors are different, and they can't be specialized
// https://github.com/rust-lang/rust/issues/46893
pub struct RenderCommandEncoder<'a> {
    cmd_buf: &'a mut CommandBuffer,
    device: &'a Device,
    update_data: &'a mut Vec<u8>,
}

pub struct PipelineEncoder<'a, 'p> {
    cmd_buf: &'a mut CommandBuffer,
    layout: &'p PipelineLayout,
    bind_point: vk::PipelineBindPoint,
    device: &'a Device,
    update_data: &'a mut Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct SyncPoint {
    progress: u64,
}

#[hidden_trait::expose]
impl crate::traits::CommandDevice for Context {
    type CommandEncoder = CommandEncoder;
    type SyncPoint = SyncPoint;

    fn create_command_encoder(&self, desc: super::CommandEncoderDesc) -> CommandEncoder {
        //TODO: these numbers are arbitrary, needs to be replaced by
        // an abstraction from gpu-alloc, if possible.
        const ROUGH_SET_COUNT: u32 = 60000;
        let mut descriptor_sizes = vec![
            vk::DescriptorPoolSize {
                ty: vk::DescriptorType::INLINE_UNIFORM_BLOCK_EXT,
                descriptor_count: ROUGH_SET_COUNT * crate::limits::PLAIN_DATA_SIZE,
            },
            vk::DescriptorPoolSize {
                ty: vk::DescriptorType::STORAGE_BUFFER,
                descriptor_count: ROUGH_SET_COUNT,
            },
            vk::DescriptorPoolSize {
                ty: vk::DescriptorType::SAMPLED_IMAGE,
                descriptor_count: 2 * ROUGH_SET_COUNT,
            },
            vk::DescriptorPoolSize {
                ty: vk::DescriptorType::SAMPLER,
                descriptor_count: ROUGH_SET_COUNT,
            },
            vk::DescriptorPoolSize {
                ty: vk::DescriptorType::STORAGE_IMAGE,
                descriptor_count: ROUGH_SET_COUNT,
            },
        ];
        if self.device.ray_tracing.is_some() {
            descriptor_sizes.push(vk::DescriptorPoolSize {
                ty: vk::DescriptorType::ACCELERATION_STRUCTURE_KHR,
                descriptor_count: ROUGH_SET_COUNT,
            });
        }

        let pool_info = vk::CommandPoolCreateInfo {
            flags: vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER,
            ..Default::default()
        };
        let pool = unsafe {
            self.device
                .core
                .create_command_pool(&pool_info, None)
                .unwrap()
        };
        let cmd_buf_info = vk::CommandBufferAllocateInfo {
            command_pool: pool,
            command_buffer_count: desc.buffer_count,
            ..Default::default()
        };
        let cmd_buffers = unsafe {
            self.device
                .core
                .allocate_command_buffers(&cmd_buf_info)
                .unwrap()
        };

        let buffers = cmd_buffers
            .into_iter()
            .map(|raw| {
                if !desc.name.is_empty() {
                    self.set_object_name(raw, desc.name);
                };
                let descriptor_pool = self.device.create_descriptor_pool();
                let query_pool = if self.device.timing.is_some() {
                    let query_pool_info = vk::QueryPoolCreateInfo::default()
                        .query_type(vk::QueryType::TIMESTAMP)
                        .query_count(QUERY_POOL_SIZE as u32);
                    unsafe {
                        self.device
                            .core
                            .create_query_pool(&query_pool_info, None)
                            .unwrap()
                    }
                } else {
                    vk::QueryPool::null()
                };
                CommandBuffer {
                    raw,
                    descriptor_pool,
                    query_pool,
                    timed_pass_names: Vec::new(),
                }
            })
            .collect();

        let crash_handler = if self.device.buffer_marker.is_some() {
            Some(CrashHandler {
                name: desc.name.to_string(),
                marker_buf: self.create_buffer(crate::BufferDesc {
                    name: "_marker",
                    size: 4,
                    memory: crate::Memory::Shared,
                }),
                raw_string: vec![0; 0x1000].into_boxed_slice(),
                next_offset: 0,
            })
        } else {
            None
        };

        CommandEncoder {
            pool,
            buffers,
            device: self.device.clone(),
            update_data: Vec::new(),
            present: None,
            crash_handler,
            temp_label: Vec::new(),
            timings: Default::default(),
        }
    }

    fn destroy_command_encoder(&self, command_encoder: &mut CommandEncoder) {
        for cmd_buf in command_encoder.buffers.iter_mut() {
            let raw_cmd_buffers = [cmd_buf.raw];
            unsafe {
                self.device
                    .core
                    .free_command_buffers(command_encoder.pool, &raw_cmd_buffers);
            }
            self.device
                .destroy_descriptor_pool(&mut cmd_buf.descriptor_pool);
            if self.device.timing.is_some() {
                unsafe {
                    self.device
                        .core
                        .destroy_query_pool(cmd_buf.query_pool, None);
                }
            }
        }
        unsafe {
            self.device
                .core
                .destroy_command_pool(mem::take(&mut command_encoder.pool), None)
        };
        if let Some(crash_handler) = command_encoder.crash_handler.take() {
            self.destroy_buffer(crash_handler.marker_buf);
        };
    }

    fn submit(&self, encoder: &mut CommandEncoder) -> SyncPoint {
        let raw_cmd_buf = encoder.finish();
        let mut queue = self.queue.lock().unwrap();
        queue.last_progress += 1;
        let progress = queue.last_progress;
        let command_buffers = [raw_cmd_buf];
        let wait_values_all = [0];
        let mut wait_semaphores_all = [vk::Semaphore::null()];
        let wait_stages = [vk::PipelineStageFlags::ALL_COMMANDS];
        let mut signal_semaphores_all = [queue.timeline_semaphore, vk::Semaphore::null()];
        let signal_values_all = [progress, 0];
        let (num_wait_semaphores, num_signal_sepahores) = match encoder.present {
            Some(ref presentation) => {
                wait_semaphores_all[0] = presentation.acquire_semaphore;
                signal_semaphores_all[1] = presentation.present_semaphore;
                (1, 2)
            }
            None => (0, 1),
        };
        let mut timeline_info = vk::TimelineSemaphoreSubmitInfo::default()
            .wait_semaphore_values(&wait_values_all[..num_wait_semaphores])
            .signal_semaphore_values(&signal_values_all[..num_signal_sepahores]);
        let vk_info = vk::SubmitInfo::default()
            .command_buffers(&command_buffers)
            .wait_semaphores(&wait_semaphores_all[..num_wait_semaphores])
            .wait_dst_stage_mask(&wait_stages[..num_wait_semaphores])
            .signal_semaphores(&signal_semaphores_all[..num_signal_sepahores])
            .push_next(&mut timeline_info);
        let ret = unsafe {
            self.device
                .core
                .queue_submit(queue.raw, &[vk_info], vk::Fence::null())
        };
        encoder.check_gpu_crash(ret);

        if let Some(presentation) = encoder.present.take() {
            let khr_swapchain = self.device.swapchain.as_ref().unwrap();
            let swapchains = [presentation.swapchain];
            let image_indices = [presentation.image_index];
            let wait_semaphores = [presentation.present_semaphore];
            let present_info = vk::PresentInfoKHR::default()
                .swapchains(&swapchains)
                .image_indices(&image_indices)
                .wait_semaphores(&wait_semaphores);
            let ret = unsafe { khr_swapchain.queue_present(queue.raw, &present_info) };
            let _ = encoder.check_gpu_crash(ret);
        }

        SyncPoint { progress }
    }

    fn wait_for(&self, sp: &SyncPoint, timeout_ms: u32) -> bool {
        //Note: technically we could get away without locking the queue,
        // but also this isn't time-sensitive, so it's fine.
        let timeline_semaphore = self.queue.lock().unwrap().timeline_semaphore;
        let semaphores = [timeline_semaphore];
        let semaphore_values = [sp.progress];
        let wait_info = vk::SemaphoreWaitInfoKHR::default()
            .semaphores(&semaphores)
            .values(&semaphore_values);
        let timeout_ns = map_timeout(timeout_ms);
        unsafe {
            self.device
                .timeline_semaphore
                .wait_semaphores(&wait_info, timeout_ns)
                .is_ok()
        }
    }
}

fn map_texture_format(format: crate::TextureFormat) -> vk::Format {
    use crate::TextureFormat as Tf;
    match format {
        Tf::R8Unorm => vk::Format::R8_UNORM,
        Tf::Rg8Unorm => vk::Format::R8G8_UNORM,
        Tf::Rg8Snorm => vk::Format::R8G8_SNORM,
        Tf::Rgba8Unorm => vk::Format::R8G8B8A8_UNORM,
        Tf::Rgba8UnormSrgb => vk::Format::R8G8B8A8_SRGB,
        Tf::Bgra8Unorm => vk::Format::B8G8R8A8_UNORM,
        Tf::Bgra8UnormSrgb => vk::Format::B8G8R8A8_SRGB,
        Tf::Rgba8Snorm => vk::Format::R8G8B8A8_SNORM,
        Tf::R16Float => vk::Format::R16_SFLOAT,
        Tf::Rg16Float => vk::Format::R16G16_SFLOAT,
        Tf::Rgba16Float => vk::Format::R16G16B16A16_SFLOAT,
        Tf::R32Float => vk::Format::R32_SFLOAT,
        Tf::Rg32Float => vk::Format::R32G32_SFLOAT,
        Tf::Rgba32Float => vk::Format::R32G32B32A32_SFLOAT,
        Tf::R32Uint => vk::Format::R32_UINT,
        Tf::Rg32Uint => vk::Format::R32G32_UINT,
        Tf::Rgba32Uint => vk::Format::R32G32B32A32_UINT,
        Tf::Depth32Float => vk::Format::D32_SFLOAT,
        Tf::Depth32FloatStencil8Uint => vk::Format::D32_SFLOAT_S8_UINT,
        Tf::Stencil8Uint => vk::Format::S8_UINT,
        Tf::Bc1Unorm => vk::Format::BC1_RGBA_SRGB_BLOCK,
        Tf::Bc1UnormSrgb => vk::Format::BC1_RGBA_UNORM_BLOCK,
        Tf::Bc2Unorm => vk::Format::BC2_UNORM_BLOCK,
        Tf::Bc2UnormSrgb => vk::Format::BC2_SRGB_BLOCK,
        Tf::Bc3Unorm => vk::Format::BC3_UNORM_BLOCK,
        Tf::Bc3UnormSrgb => vk::Format::BC3_SRGB_BLOCK,
        Tf::Bc4Unorm => vk::Format::BC4_UNORM_BLOCK,
        Tf::Bc4Snorm => vk::Format::BC4_SNORM_BLOCK,
        Tf::Bc5Unorm => vk::Format::BC5_UNORM_BLOCK,
        Tf::Bc5Snorm => vk::Format::BC5_SNORM_BLOCK,
        Tf::Bc6hUfloat => vk::Format::BC6H_UFLOAT_BLOCK,
        Tf::Bc6hFloat => vk::Format::BC6H_SFLOAT_BLOCK,
        Tf::Bc7Unorm => vk::Format::BC7_UNORM_BLOCK,
        Tf::Bc7UnormSrgb => vk::Format::BC7_SRGB_BLOCK,
        Tf::Rgb10a2Unorm => vk::Format::A2B10G10R10_UNORM_PACK32,
        Tf::Rg11b10Ufloat => vk::Format::B10G11R11_UFLOAT_PACK32,
        Tf::Rgb9e5Ufloat => vk::Format::E5B9G9R9_UFLOAT_PACK32,
    }
}

fn map_aspects(aspects: crate::TexelAspects) -> vk::ImageAspectFlags {
    let mut flags = vk::ImageAspectFlags::empty();
    if aspects.contains(crate::TexelAspects::COLOR) {
        flags |= vk::ImageAspectFlags::COLOR;
    }
    if aspects.contains(crate::TexelAspects::DEPTH) {
        flags |= vk::ImageAspectFlags::DEPTH;
    }
    if aspects.contains(crate::TexelAspects::STENCIL) {
        flags |= vk::ImageAspectFlags::STENCIL;
    }
    flags
}

fn map_extent_3d(extent: &crate::Extent) -> vk::Extent3D {
    vk::Extent3D {
        width: extent.width,
        height: extent.height,
        depth: extent.depth,
    }
}

fn map_subresource_range(
    subresources: &crate::TextureSubresources,
    aspects: crate::TexelAspects,
) -> vk::ImageSubresourceRange {
    vk::ImageSubresourceRange {
        aspect_mask: map_aspects(aspects),
        base_mip_level: subresources.base_mip_level,
        level_count: subresources
            .mip_level_count
            .map_or(vk::REMAINING_MIP_LEVELS, NonZeroU32::get),
        base_array_layer: subresources.base_array_layer,
        layer_count: subresources
            .array_layer_count
            .map_or(vk::REMAINING_ARRAY_LAYERS, NonZeroU32::get),
    }
}

fn map_comparison(fun: crate::CompareFunction) -> vk::CompareOp {
    use crate::CompareFunction as Cf;
    match fun {
        Cf::Never => vk::CompareOp::NEVER,
        Cf::Less => vk::CompareOp::LESS,
        Cf::LessEqual => vk::CompareOp::LESS_OR_EQUAL,
        Cf::Equal => vk::CompareOp::EQUAL,
        Cf::GreaterEqual => vk::CompareOp::GREATER_OR_EQUAL,
        Cf::Greater => vk::CompareOp::GREATER,
        Cf::NotEqual => vk::CompareOp::NOT_EQUAL,
        Cf::Always => vk::CompareOp::ALWAYS,
    }
}

fn map_index_type(index_type: crate::IndexType) -> vk::IndexType {
    match index_type {
        crate::IndexType::U16 => vk::IndexType::UINT16,
        crate::IndexType::U32 => vk::IndexType::UINT32,
    }
}

fn map_vertex_format(vertex_format: crate::VertexFormat) -> vk::Format {
    use crate::VertexFormat as Vf;
    match vertex_format {
        Vf::F32 => vk::Format::R32_SFLOAT,
        Vf::F32Vec2 => vk::Format::R32G32_SFLOAT,
        Vf::F32Vec3 => vk::Format::R32G32B32_SFLOAT,
        Vf::F32Vec4 => vk::Format::R32G32B32A32_SFLOAT,
        Vf::U32 => vk::Format::R32_UINT,
        Vf::U32Vec2 => vk::Format::R32G32_UINT,
        Vf::U32Vec3 => vk::Format::R32G32B32_UINT,
        Vf::U32Vec4 => vk::Format::R32G32B32A32_UINT,
        Vf::I32 => vk::Format::R32_SINT,
        Vf::I32Vec2 => vk::Format::R32G32_SINT,
        Vf::I32Vec3 => vk::Format::R32G32B32_SINT,
        Vf::I32Vec4 => vk::Format::R32G32B32A32_SINT,
    }
}

struct BottomLevelAccelerationStructureInput<'a> {
    max_primitive_counts: Box<[u32]>,
    build_range_infos: Box<[vk::AccelerationStructureBuildRangeInfoKHR]>,
    _geometries: Box<[vk::AccelerationStructureGeometryKHR<'a>]>,
    build_info: vk::AccelerationStructureBuildGeometryInfoKHR<'a>,
}

impl Device {
    fn get_device_address(&self, piece: &crate::BufferPiece) -> u64 {
        let vk_info = vk::BufferDeviceAddressInfo {
            buffer: piece.buffer.raw,
            ..Default::default()
        };
        let base = unsafe { self.core.get_buffer_device_address(&vk_info) };
        base + piece.offset
    }

    fn map_acceleration_structure_meshes(
        &self,
        meshes: &[crate::AccelerationStructureMesh],
    ) -> BottomLevelAccelerationStructureInput {
        let mut total_primitive_count = 0;
        let mut max_primitive_counts = Vec::with_capacity(meshes.len());
        let mut build_range_infos = Vec::with_capacity(meshes.len());
        let mut geometries = Vec::with_capacity(meshes.len());
        for mesh in meshes {
            total_primitive_count += mesh.triangle_count;
            max_primitive_counts.push(mesh.triangle_count);
            build_range_infos.push(vk::AccelerationStructureBuildRangeInfoKHR {
                primitive_count: mesh.triangle_count,
                primitive_offset: 0,
                first_vertex: 0,
                transform_offset: 0,
            });

            let mut triangles = vk::AccelerationStructureGeometryTrianglesDataKHR {
                vertex_format: map_vertex_format(mesh.vertex_format),
                vertex_data: {
                    let device_address = self.get_device_address(&mesh.vertex_data);
                    assert!(
                        device_address & 0x3 == 0,
                        "Vertex data address {device_address} is not aligned"
                    );
                    vk::DeviceOrHostAddressConstKHR { device_address }
                },
                vertex_stride: mesh.vertex_stride as u64,
                max_vertex: mesh.vertex_count.saturating_sub(1),
                ..Default::default()
            };
            if let Some(index_type) = mesh.index_type {
                let device_address = self.get_device_address(&mesh.index_data);
                assert!(
                    device_address & 0x3 == 0,
                    "Index data address {device_address} is not aligned"
                );
                triangles.index_type = map_index_type(index_type);
                triangles.index_data = vk::DeviceOrHostAddressConstKHR { device_address };
            }
            if mesh.transform_data.buffer.raw != vk::Buffer::null() {
                let device_address = self.get_device_address(&mesh.transform_data);
                assert!(
                    device_address & 0xF == 0,
                    "Transform data address {device_address} is not aligned"
                );
                triangles.transform_data = vk::DeviceOrHostAddressConstKHR { device_address };
            }

            let geometry = vk::AccelerationStructureGeometryKHR {
                geometry_type: vk::GeometryTypeKHR::TRIANGLES,
                geometry: vk::AccelerationStructureGeometryDataKHR { triangles },
                flags: if mesh.is_opaque {
                    vk::GeometryFlagsKHR::OPAQUE
                } else {
                    vk::GeometryFlagsKHR::empty()
                },
                ..Default::default()
            };
            geometries.push(geometry);
        }
        let build_info = vk::AccelerationStructureBuildGeometryInfoKHR {
            ty: vk::AccelerationStructureTypeKHR::BOTTOM_LEVEL,
            flags: vk::BuildAccelerationStructureFlagsKHR::PREFER_FAST_TRACE,
            mode: vk::BuildAccelerationStructureModeKHR::BUILD,
            geometry_count: geometries.len() as u32,
            p_geometries: geometries.as_ptr(),
            ..Default::default()
        };

        log::debug!(
            "BLAS total {} primitives in {} geometries",
            total_primitive_count,
            geometries.len()
        );
        BottomLevelAccelerationStructureInput {
            max_primitive_counts: max_primitive_counts.into_boxed_slice(),
            build_range_infos: build_range_infos.into_boxed_slice(),
            _geometries: geometries.into_boxed_slice(),
            build_info,
        }
    }
}
