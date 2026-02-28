use ash::vk;
use gpu_alloc_ash::AshMemoryDevice;
use std::{mem, ptr};

struct Allocation {
    memory: vk::DeviceMemory,
    offset: u64,
    data: *mut u8,
    handle: usize,
    memory_type: crate::Memory,
}

impl super::Context {
    fn allocate_memory(
        &self,
        requirements: vk::MemoryRequirements,
        memory: crate::Memory,
    ) -> Allocation {
        let mut manager = self.memory.lock().unwrap();
        let device_address_usage = if self.device.ray_tracing.is_some() {
            gpu_alloc::UsageFlags::DEVICE_ADDRESS
        } else {
            gpu_alloc::UsageFlags::empty()
        };
        let alloc_usage = match memory {
            crate::Memory::Device | crate::Memory::External(_) => {
                gpu_alloc::UsageFlags::FAST_DEVICE_ACCESS | device_address_usage
            }
            crate::Memory::Shared => {
                gpu_alloc::UsageFlags::HOST_ACCESS
                    | gpu_alloc::UsageFlags::DOWNLOAD
                    | gpu_alloc::UsageFlags::UPLOAD
                    | gpu_alloc::UsageFlags::FAST_DEVICE_ACCESS
                    | device_address_usage
            }
            crate::Memory::Upload => {
                gpu_alloc::UsageFlags::HOST_ACCESS | gpu_alloc::UsageFlags::UPLOAD
            }
        };
        let memory_types = requirements.memory_type_bits & manager.valid_ash_memory_types;
        let mut block = match memory {
            crate::Memory::External(e) => {
                let memory_properties = unsafe {
                    self.instance
                        .core
                        .get_physical_device_memory_properties(self.physical_device)
                };
                let memory_type_index = memory_types.ilog2();
                let memory_type: vk::MemoryType =
                    memory_properties.memory_types[memory_type_index as usize];

                let handle_type = external_source_handle_type(e);
                let external_info: &mut dyn vk::ExtendsMemoryAllocateInfo = match e {
                    #[cfg(target_os = "windows")]
                    crate::ExternalMemorySource::Win32(Some(handle))
                    | crate::ExternalMemorySource::Win32KMT(Some(handle)) => {
                        &mut vk::ImportMemoryWin32HandleInfoKHR {
                            handle_type,
                            handle,
                            ..Default::default()
                        }
                    }
                    #[cfg(not(target_os = "windows"))]
                    crate::ExternalMemorySource::Fd(Some(fd)) => &mut vk::ImportMemoryFdInfoKHR {
                        handle_type,
                        fd,
                        ..Default::default()
                    },
                    #[cfg(target_os = "linux")]
                    crate::ExternalMemorySource::Dma(Some(fd)) => &mut vk::ImportMemoryFdInfoKHR {
                        handle_type,
                        fd,
                        ..Default::default()
                    },
                    crate::ExternalMemorySource::HostAllocation(ptr) => {
                        &mut vk::ImportMemoryHostPointerInfoEXT {
                            handle_type,
                            p_host_pointer: ptr as *mut std::ffi::c_void,
                            ..Default::default()
                        }
                    }
                    // All sources that are exporting don't need a specific struct
                    _ => &mut vk::ExportMemoryAllocateInfo {
                        handle_types: handle_type,
                        ..Default::default()
                    },
                };

                let allocation_info = vk::MemoryAllocateInfo {
                    allocation_size: requirements.size,
                    memory_type_index,
                    ..vk::MemoryAllocateInfo::default()
                }
                .push_next(external_info);

                let memory = unsafe {
                    self.device
                        .core
                        .allocate_memory(&allocation_info, None)
                        .unwrap()
                };

                unsafe {
                    manager.allocator.import_memory(
                        memory,
                        memory_type_index,
                        gpu_alloc_ash::memory_properties_from_ash(memory_type.property_flags),
                        0,
                        requirements.size,
                    )
                }
            }
            _ => unsafe {
                manager
                    .allocator
                    .alloc(
                        AshMemoryDevice::wrap(&self.device.core),
                        gpu_alloc::Request {
                            size: requirements.size,
                            align_mask: requirements.alignment - 1,
                            usage: alloc_usage,
                            memory_types,
                        },
                    )
                    .unwrap()
            },
        };

        let data = match memory {
            crate::Memory::External(crate::ExternalMemorySource::HostAllocation(ptr)) => {
                ptr as *mut u8
            }
            crate::Memory::Device | crate::Memory::External(_) => ptr::null_mut(),
            crate::Memory::Shared | crate::Memory::Upload => unsafe {
                block
                    .map(
                        AshMemoryDevice::wrap(&self.device.core),
                        0,
                        requirements.size as usize,
                    )
                    .unwrap()
                    .as_ptr()
            },
        };
        Allocation {
            memory: *block.memory(),
            offset: block.offset(),
            data,
            handle: manager.slab.insert(block),
            memory_type: memory,
        }
    }

    fn free_memory(&self, handle: usize) {
        let mut manager = self.memory.lock().unwrap();
        let block = manager.slab.remove(handle);
        unsafe {
            manager
                .allocator
                .dealloc(AshMemoryDevice::wrap(&self.device.core), block);
        }
    }

    //TODO: move these into `ResourceDevice` trait when ready
    pub fn get_bottom_level_acceleration_structure_sizes(
        &self,
        meshes: &[crate::AccelerationStructureMesh],
    ) -> crate::AccelerationStructureSizes {
        let blas_input = self.device.map_acceleration_structure_meshes(meshes);
        let rt = self.device.ray_tracing.as_ref().unwrap();
        let mut sizes_raw = vk::AccelerationStructureBuildSizesInfoKHR::default();
        unsafe {
            rt.acceleration_structure
                .get_acceleration_structure_build_sizes(
                    vk::AccelerationStructureBuildTypeKHR::DEVICE,
                    &blas_input.build_info,
                    &blas_input.max_primitive_counts,
                    &mut sizes_raw,
                )
        };
        crate::AccelerationStructureSizes {
            data: sizes_raw.acceleration_structure_size,
            scratch: sizes_raw.build_scratch_size,
        }
    }

    pub fn get_top_level_acceleration_structure_sizes(
        &self,
        instance_count: u32,
    ) -> crate::AccelerationStructureSizes {
        let geometry = vk::AccelerationStructureGeometryKHR::default()
            .geometry_type(vk::GeometryTypeKHR::INSTANCES)
            .geometry(vk::AccelerationStructureGeometryDataKHR {
                instances: vk::AccelerationStructureGeometryInstancesDataKHR::default(),
            });
        let geometries = [geometry];
        let build_info = vk::AccelerationStructureBuildGeometryInfoKHR::default()
            .ty(vk::AccelerationStructureTypeKHR::TOP_LEVEL)
            .mode(vk::BuildAccelerationStructureModeKHR::BUILD)
            .geometries(&geometries);

        let rt = self.device.ray_tracing.as_ref().unwrap();
        let mut sizes_raw = vk::AccelerationStructureBuildSizesInfoKHR::default();
        unsafe {
            rt.acceleration_structure
                .get_acceleration_structure_build_sizes(
                    vk::AccelerationStructureBuildTypeKHR::DEVICE,
                    &build_info,
                    &[instance_count],
                    &mut sizes_raw,
                )
        };
        crate::AccelerationStructureSizes {
            data: sizes_raw.acceleration_structure_size,
            scratch: sizes_raw.build_scratch_size,
        }
    }

    pub fn create_acceleration_structure_instance_buffer(
        &self,
        instances: &[crate::AccelerationStructureInstance],
        bottom_level: &[super::AccelerationStructure],
    ) -> super::Buffer {
        let buffer = self.create_buffer(crate::BufferDesc {
            name: "instance buffer",
            size: (instances.len().max(1) * mem::size_of::<vk::AccelerationStructureInstanceKHR>())
                as u64,
            memory: crate::Memory::Shared,
        });
        let rt = self.device.ray_tracing.as_ref().unwrap();
        for (i, instance) in instances.iter().enumerate() {
            let device_address_info = vk::AccelerationStructureDeviceAddressInfoKHR {
                acceleration_structure: bottom_level
                    [instance.acceleration_structure_index as usize]
                    .raw,
                ..Default::default()
            };
            let vk_instance = vk::AccelerationStructureInstanceKHR {
                transform: unsafe { mem::transmute(instance.transform) },
                instance_custom_index_and_mask: vk::Packed24_8::new(
                    instance.custom_index,
                    instance.mask as u8,
                ),
                instance_shader_binding_table_record_offset_and_flags: vk::Packed24_8::new(0, 0),
                acceleration_structure_reference: vk::AccelerationStructureReferenceKHR {
                    device_handle: unsafe {
                        rt.acceleration_structure
                            .get_acceleration_structure_device_address(&device_address_info)
                    },
                },
            };
            unsafe {
                ptr::write(
                    (buffer.data() as *mut vk::AccelerationStructureInstanceKHR).add(i),
                    vk_instance,
                );
            }
        }
        buffer
    }

    pub fn get_external_texture_source(
        &self,
        texture: super::Texture,
    ) -> Option<crate::ExternalMemorySource> {
        texture.external
    }

    pub fn get_external_buffer_source(
        &self,
        buffer: super::Buffer,
    ) -> Option<crate::ExternalMemorySource> {
        buffer.external
    }
}

#[hidden_trait::expose]
impl crate::traits::ResourceDevice for super::Context {
    type Buffer = super::Buffer;
    type Texture = super::Texture;
    type TextureView = super::TextureView;
    type Sampler = super::Sampler;
    type AccelerationStructure = super::AccelerationStructure;

    fn create_buffer(&self, desc: crate::BufferDesc) -> super::Buffer {
        use vk::BufferUsageFlags as Buf;
        let external_source = match desc.memory {
            crate::Memory::External(e) => Some(e),
            _ => None,
        };
        let mut external_next = external_source.map(|e| vk::ExternalMemoryBufferCreateInfo {
            handle_types: external_source_handle_type(e),
            ..Default::default()
        });
        let mut vk_info = vk::BufferCreateInfo {
            size: desc.size,
            usage: Buf::TRANSFER_SRC
                | Buf::TRANSFER_DST
                | Buf::STORAGE_BUFFER
                | Buf::INDEX_BUFFER
                | Buf::VERTEX_BUFFER
                | Buf::INDIRECT_BUFFER,
            sharing_mode: vk::SharingMode::EXCLUSIVE,
            ..Default::default()
        };
        if let Some(external_next) = external_next.as_mut() {
            vk_info = vk_info.push_next(external_next);
        }
        if self.device.ray_tracing.is_some() {
            vk_info.usage |=
                Buf::SHADER_DEVICE_ADDRESS | Buf::ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_KHR;
        }

        let raw = unsafe { self.device.core.create_buffer(&vk_info, None).unwrap() };
        let mut requirements = unsafe { self.device.core.get_buffer_memory_requirements(raw) };
        requirements.alignment = requirements.alignment.max(self.min_buffer_alignment);
        let allocation = self.allocate_memory(requirements, desc.memory);

        log::info!(
            "Creating buffer {:?} of size {}, name '{}', handle {:?}",
            raw,
            desc.size,
            desc.name,
            allocation.handle
        );
        unsafe {
            self.device
                .core
                .bind_buffer_memory(raw, allocation.memory, allocation.offset)
                .unwrap()
        };
        if !desc.name.is_empty() {
            self.set_object_name(raw, desc.name);
        }

        super::Buffer {
            raw,
            memory_handle: allocation.handle,
            mapped_data: allocation.data,
            external: fetch_external_source(&self.device, allocation),
        }
    }

    fn sync_buffer(&self, _buffer: super::Buffer) {}

    fn destroy_buffer(&self, buffer: super::Buffer) {
        log::info!(
            "Destroying buffer {:?}, handle {:?}",
            buffer.raw,
            buffer.memory_handle
        );
        unsafe { self.device.core.destroy_buffer(buffer.raw, None) };
        self.free_memory(buffer.memory_handle);
    }

    fn create_texture(&self, desc: crate::TextureDesc) -> super::Texture {
        let mut create_flags = vk::ImageCreateFlags::empty();
        if desc.dimension == crate::TextureDimension::D2
            && desc.size.depth % 6 == 0
            && desc.sample_count == 1
            && desc.size.width == desc.size.height
        {
            create_flags |= vk::ImageCreateFlags::CUBE_COMPATIBLE;
        }

        let mut external_next = desc.external.map(|e| vk::ExternalMemoryImageCreateInfo {
            handle_types: external_source_handle_type(e),
            ..Default::default()
        });

        let mut vk_info = vk::ImageCreateInfo {
            flags: create_flags,
            image_type: map_texture_dimension(desc.dimension),
            format: super::map_texture_format(desc.format),
            extent: super::map_extent_3d(&desc.size),
            mip_levels: desc.mip_level_count,
            array_layers: desc.array_layer_count,
            samples: vk::SampleCountFlags::from_raw(desc.sample_count),
            tiling: vk::ImageTiling::OPTIMAL,
            usage: map_texture_usage(desc.usage, desc.format.aspects()),
            sharing_mode: vk::SharingMode::EXCLUSIVE,
            ..Default::default()
        };

        if let Some(external_next) = external_next.as_mut() {
            vk_info = vk_info.push_next(external_next);
        }

        /*
            TODO(ErikWDev): Support lazily allocated texture with transient allocation for efficient msaa?
                            Measure bandwidth usage!
        */
        let raw = unsafe { self.device.core.create_image(&vk_info, None).unwrap() };
        let requirements = unsafe { self.device.core.get_image_memory_requirements(raw) };
        let allocation = self.allocate_memory(
            requirements,
            desc.external
                .map_or(crate::Memory::Device, crate::Memory::External),
        );

        log::info!(
            "Creating texture {:?} of size {} and format {:?}, name '{}', handle {:?}",
            raw,
            desc.size,
            desc.format,
            desc.name,
            allocation.handle
        );
        unsafe {
            self.device
                .core
                .bind_image_memory(raw, allocation.memory, allocation.offset)
                .unwrap()
        };
        if !desc.name.is_empty() {
            self.set_object_name(raw, desc.name);
        }

        super::Texture {
            raw,
            memory_handle: allocation.handle,
            target_size: [desc.size.width as u16, desc.size.height as u16],
            format: desc.format,
            external: fetch_external_source(&self.device, allocation),
        }
    }

    fn destroy_texture(&self, texture: super::Texture) {
        log::info!(
            "Destroying texture {:?}, handle {:?}",
            texture.raw,
            texture.memory_handle
        );
        unsafe { self.device.core.destroy_image(texture.raw, None) };
        self.free_memory(texture.memory_handle);
    }

    fn create_texture_view(
        &self,
        texture: super::Texture,
        desc: crate::TextureViewDesc,
    ) -> super::TextureView {
        let aspects = desc.format.aspects();
        let subresource_range = super::map_subresource_range(desc.subresources, aspects);
        let vk_info = vk::ImageViewCreateInfo {
            image: texture.raw,
            view_type: map_view_dimension(desc.dimension),
            format: super::map_texture_format(desc.format),
            subresource_range,
            ..Default::default()
        };

        let raw = unsafe { self.device.core.create_image_view(&vk_info, None).unwrap() };
        if !desc.name.is_empty() {
            self.set_object_name(raw, desc.name);
        }

        super::TextureView {
            raw,
            target_size: [
                (texture.target_size[0] >> desc.subresources.base_mip_level).max(1),
                (texture.target_size[1] >> desc.subresources.base_mip_level).max(1),
            ],
            aspects,
        }
    }

    fn destroy_texture_view(&self, view: super::TextureView) {
        unsafe { self.device.core.destroy_image_view(view.raw, None) };
    }

    fn create_sampler(&self, desc: crate::SamplerDesc) -> super::Sampler {
        let mut vk_info = vk::SamplerCreateInfo {
            mag_filter: map_filter_mode(desc.mag_filter),
            min_filter: map_filter_mode(desc.min_filter),
            mipmap_mode: map_mip_filter_mode(desc.mipmap_filter),
            address_mode_u: map_address_mode(desc.address_modes[0]),
            address_mode_v: map_address_mode(desc.address_modes[1]),
            address_mode_w: map_address_mode(desc.address_modes[2]),
            min_lod: desc.lod_min_clamp,
            max_lod: desc.lod_max_clamp.unwrap_or(vk::LOD_CLAMP_NONE),
            ..Default::default()
        };

        if let Some(fun) = desc.compare {
            vk_info.compare_enable = vk::TRUE;
            vk_info.compare_op = super::map_comparison(fun);
        }
        if desc.anisotropy_clamp > 1 {
            vk_info.anisotropy_enable = vk::TRUE;
            vk_info.max_anisotropy = desc.anisotropy_clamp as f32;
        }
        if let Some(color) = desc.border_color {
            vk_info.border_color = map_border_color(color);
        }

        let raw = unsafe { self.device.core.create_sampler(&vk_info, None).unwrap() };
        if !desc.name.is_empty() {
            self.set_object_name(raw, desc.name);
        }

        super::Sampler { raw }
    }

    fn destroy_sampler(&self, sampler: super::Sampler) {
        unsafe { self.device.core.destroy_sampler(sampler.raw, None) };
    }

    fn create_acceleration_structure(
        &self,
        desc: crate::AccelerationStructureDesc,
    ) -> super::AccelerationStructure {
        let buffer_info = vk::BufferCreateInfo {
            size: desc.size,
            usage: vk::BufferUsageFlags::ACCELERATION_STRUCTURE_STORAGE_KHR
                | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS,
            sharing_mode: vk::SharingMode::EXCLUSIVE,
            ..Default::default()
        };

        let buffer = unsafe { self.device.core.create_buffer(&buffer_info, None).unwrap() };
        let requirements = unsafe { self.device.core.get_buffer_memory_requirements(buffer) };
        let allocation = self.allocate_memory(requirements, crate::Memory::Device);

        unsafe {
            self.device
                .core
                .bind_buffer_memory(buffer, allocation.memory, allocation.offset)
                .unwrap()
        };

        let raw_ty = match desc.ty {
            crate::AccelerationStructureType::TopLevel => {
                vk::AccelerationStructureTypeKHR::TOP_LEVEL
            }
            crate::AccelerationStructureType::BottomLevel => {
                vk::AccelerationStructureTypeKHR::BOTTOM_LEVEL
            }
        };
        let vk_info = vk::AccelerationStructureCreateInfoKHR {
            ty: raw_ty,
            buffer,
            size: desc.size,
            ..Default::default()
        };

        let rt = self.device.ray_tracing.as_ref().unwrap();
        let raw = unsafe {
            rt.acceleration_structure
                .create_acceleration_structure(&vk_info, None)
                .unwrap()
        };

        if !desc.name.is_empty() {
            self.set_object_name(buffer, desc.name);
            self.set_object_name(raw, desc.name);
        }
        super::AccelerationStructure {
            raw,
            buffer,
            memory_handle: allocation.handle,
        }
    }

    fn destroy_acceleration_structure(&self, acceleration_structure: super::AccelerationStructure) {
        let rt = self.device.ray_tracing.as_ref().unwrap();
        unsafe {
            rt.acceleration_structure
                .destroy_acceleration_structure(acceleration_structure.raw, None);
            self.device
                .core
                .destroy_buffer(acceleration_structure.buffer, None);
        }
        self.free_memory(acceleration_structure.memory_handle);
    }
}

fn map_texture_dimension(dimension: crate::TextureDimension) -> vk::ImageType {
    match dimension {
        crate::TextureDimension::D1 => vk::ImageType::TYPE_1D,
        crate::TextureDimension::D2 => vk::ImageType::TYPE_2D,
        crate::TextureDimension::D3 => vk::ImageType::TYPE_3D,
    }
}

fn map_view_dimension(dimension: crate::ViewDimension) -> vk::ImageViewType {
    use crate::ViewDimension as Vd;
    match dimension {
        Vd::D1 => vk::ImageViewType::TYPE_1D,
        Vd::D1Array => vk::ImageViewType::TYPE_1D_ARRAY,
        Vd::D2 => vk::ImageViewType::TYPE_2D,
        Vd::D2Array => vk::ImageViewType::TYPE_2D_ARRAY,
        Vd::Cube => vk::ImageViewType::CUBE,
        Vd::CubeArray => vk::ImageViewType::CUBE_ARRAY,
        Vd::D3 => vk::ImageViewType::TYPE_3D,
    }
}

pub(super) fn map_texture_usage(
    usage: crate::TextureUsage,
    aspects: crate::TexelAspects,
) -> vk::ImageUsageFlags {
    use vk::ImageUsageFlags as Iuf;

    let mut flags = Iuf::empty();
    if usage.contains(crate::TextureUsage::COPY) {
        flags |= Iuf::TRANSFER_SRC | Iuf::TRANSFER_DST;
    }
    if usage.contains(crate::TextureUsage::RESOURCE) {
        flags |= vk::ImageUsageFlags::SAMPLED;
    }
    if usage.contains(crate::TextureUsage::TARGET) {
        flags |= if aspects.contains(crate::TexelAspects::COLOR) {
            vk::ImageUsageFlags::COLOR_ATTACHMENT
        } else {
            vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT
        };
    }
    if usage.intersects(crate::TextureUsage::STORAGE) {
        flags |= vk::ImageUsageFlags::STORAGE;
    }
    flags
}

fn map_filter_mode(mode: crate::FilterMode) -> vk::Filter {
    match mode {
        crate::FilterMode::Nearest => vk::Filter::NEAREST,
        crate::FilterMode::Linear => vk::Filter::LINEAR,
    }
}

fn map_mip_filter_mode(mode: crate::FilterMode) -> vk::SamplerMipmapMode {
    match mode {
        crate::FilterMode::Nearest => vk::SamplerMipmapMode::NEAREST,
        crate::FilterMode::Linear => vk::SamplerMipmapMode::LINEAR,
    }
}

fn map_address_mode(mode: crate::AddressMode) -> vk::SamplerAddressMode {
    match mode {
        crate::AddressMode::ClampToEdge => vk::SamplerAddressMode::CLAMP_TO_EDGE,
        crate::AddressMode::Repeat => vk::SamplerAddressMode::REPEAT,
        crate::AddressMode::MirrorRepeat => vk::SamplerAddressMode::MIRRORED_REPEAT,
        crate::AddressMode::ClampToBorder => vk::SamplerAddressMode::CLAMP_TO_BORDER,
        // wgt::AddressMode::MirrorClamp => vk::SamplerAddressMode::MIRROR_CLAMP_TO_EDGE,
    }
}

fn map_border_color(border_color: crate::TextureColor) -> vk::BorderColor {
    match border_color {
        crate::TextureColor::TransparentBlack => vk::BorderColor::FLOAT_TRANSPARENT_BLACK,
        crate::TextureColor::OpaqueBlack => vk::BorderColor::FLOAT_OPAQUE_BLACK,
        crate::TextureColor::White => vk::BorderColor::FLOAT_OPAQUE_WHITE,
    }
}

fn external_source_handle_type(
    source: crate::ExternalMemorySource,
) -> vk::ExternalMemoryHandleTypeFlags {
    match source {
        #[cfg(target_os = "windows")]
        crate::ExternalMemorySource::Win32(_) => vk::ExternalMemoryHandleTypeFlags::OPAQUE_WIN32,
        #[cfg(target_os = "windows")]
        crate::ExternalMemorySource::Win32KMT(_) => {
            vk::ExternalMemoryHandleTypeFlags::OPAQUE_WIN32_KMT
        }
        #[cfg(not(target_os = "windows"))]
        crate::ExternalMemorySource::Fd(_) => vk::ExternalMemoryHandleTypeFlags::OPAQUE_FD,
        #[cfg(target_os = "linux")]
        crate::ExternalMemorySource::Dma(_) => vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT,
        crate::ExternalMemorySource::HostAllocation(_) => {
            vk::ExternalMemoryHandleTypeFlags::HOST_ALLOCATION_EXT
        }
    }
}

fn fetch_external_source(
    device: &super::Device,
    allocation: Allocation,
) -> Option<crate::ExternalMemorySource> {
    match allocation.memory_type {
        crate::Memory::External(e) => {
            let device = device
                .external_memory
                .as_ref()
                .expect("External memory is not supported");
            let memory = allocation.memory;
            let handle_type = external_source_handle_type(e);

            Some(match e {
                #[cfg(target_os = "windows")]
                crate::ExternalMemorySource::Win32KMT(None)
                | crate::ExternalMemorySource::Win32(None) => {
                    let info = vk::MemoryGetWin32HandleInfoKHR {
                        memory,
                        handle_type,
                        ..vk::MemoryGetWin32HandleInfoKHR::default()
                    };

                    let handle = unsafe { device.get_memory_win32_handle(&info).unwrap() };
                    match e {
                        crate::ExternalMemorySource::Win32(_) => {
                            crate::ExternalMemorySource::Win32(Some(handle))
                        }
                        crate::ExternalMemorySource::Win32KMT(_) => {
                            crate::ExternalMemorySource::Win32KMT(Some(handle))
                        }
                        _ => unreachable!(),
                    }
                }
                #[cfg(not(target_os = "windows"))]
                crate::ExternalMemorySource::Fd(None) => {
                    let info = vk::MemoryGetFdInfoKHR {
                        memory,
                        handle_type,
                        ..vk::MemoryGetFdInfoKHR::default()
                    };

                    let handle = unsafe { device.get_memory_fd(&info).unwrap() };
                    crate::ExternalMemorySource::Fd(Some(handle))
                }
                #[cfg(target_os = "linux")]
                crate::ExternalMemorySource::Dma(None) => {
                    let info = vk::MemoryGetFdInfoKHR {
                        memory,
                        handle_type,
                        ..vk::MemoryGetFdInfoKHR::default()
                    };

                    let handle = unsafe { device.get_memory_fd(&info).unwrap() };
                    crate::ExternalMemorySource::Dma(Some(handle))
                }

                //Means its a import, which already knows what its source is
                _ => e,
            })
        }
        _ => None,
    }
}
