use ash::{amd, ext, khr, vk};
use naga::back::spv;
use std::{ffi, fs, sync::Mutex};

use crate::NotSupportedError;

mod db {
    pub mod intel {
        pub const VENDOR: u32 = 0x8086;
    }
}
mod layer {
    use std::ffi::CStr;
    pub const KHRONOS_VALIDATION: &CStr =
        unsafe { CStr::from_bytes_with_nul_unchecked(b"VK_LAYER_KHRONOS_validation\0") };
    pub const MESA_OVERLAY: &CStr =
        unsafe { CStr::from_bytes_with_nul_unchecked(b"VK_LAYER_MESA_overlay\0") };
}

const REQUIRED_DEVICE_EXTENSIONS: &[&ffi::CStr] = &[
    vk::EXT_INLINE_UNIFORM_BLOCK_NAME,
    vk::KHR_TIMELINE_SEMAPHORE_NAME,
    vk::KHR_DESCRIPTOR_UPDATE_TEMPLATE_NAME,
    vk::KHR_DYNAMIC_RENDERING_NAME,
];

#[derive(Debug)]
struct RayTracingCapabilities {
    min_scratch_buffer_alignment: u64,
}

#[derive(Debug)]
struct SystemBugs {
    /// https://gitlab.freedesktop.org/mesa/mesa/-/issues/4688
    intel_unable_to_present: bool,
    /// https://github.com/kvark/blade/issues/117
    intel_fix_descriptor_pool_leak: bool,
}

#[derive(Debug)]
struct AdapterCapabilities {
    api_version: u32,
    properties: vk::PhysicalDeviceProperties,
    device_information: crate::DeviceInformation,
    queue_family_index: u32,
    layered: bool,
    ray_tracing: Option<RayTracingCapabilities>,
    buffer_marker: bool,
    shader_info: bool,
    full_screen_exclusive: bool,
    external_memory: bool,
    timing: bool,
    bugs: SystemBugs,
}

// See https://github.com/canonical/nvidia-prime/blob/587c5012be9dddcc17ab4d958f10a24fa3342b4d/prime-select#L56
fn is_nvidia_prime_forced() -> bool {
    match fs::read_to_string("/etc/prime-discrete") {
        Ok(contents) => contents == "on\n",
        Err(_) => false,
    }
}

unsafe fn inspect_adapter(
    phd: vk::PhysicalDevice,
    instance: &super::Instance,
    driver_api_version: u32,
    desc: &crate::ContextDesc,
) -> Option<AdapterCapabilities> {
    let mut inline_uniform_block_properties =
        vk::PhysicalDeviceInlineUniformBlockPropertiesEXT::default();
    let mut timeline_semaphore_properties =
        vk::PhysicalDeviceTimelineSemaphorePropertiesKHR::default();
    let mut descriptor_indexing_properties =
        vk::PhysicalDeviceDescriptorIndexingPropertiesEXT::default();
    let mut acceleration_structure_properties =
        vk::PhysicalDeviceAccelerationStructurePropertiesKHR::default();
    let mut portability_subset_properties =
        vk::PhysicalDevicePortabilitySubsetPropertiesKHR::default();

    let mut driver_properties = vk::PhysicalDeviceDriverPropertiesKHR::default();
    let mut properties2_khr = vk::PhysicalDeviceProperties2KHR::default()
        .push_next(&mut inline_uniform_block_properties)
        .push_next(&mut timeline_semaphore_properties)
        .push_next(&mut descriptor_indexing_properties)
        .push_next(&mut acceleration_structure_properties)
        .push_next(&mut portability_subset_properties)
        .push_next(&mut driver_properties);
    instance
        .get_physical_device_properties2
        .get_physical_device_properties2(phd, &mut properties2_khr);

    let properties = properties2_khr.properties;
    let name = ffi::CStr::from_ptr(properties.device_name.as_ptr());
    log::info!("Adapter: {:?}", name);

    if desc.device_id != 0 && desc.device_id != properties.device_id {
        log::info!("Rejected device ID 0x{:X}", properties.device_id);
        return None;
    }

    let api_version = properties.api_version.min(driver_api_version);
    if api_version < vk::API_VERSION_1_1 {
        log::warn!("\tRejected for API version {}", api_version);
        return None;
    }

    let supported_extension_properties = instance
        .core
        .enumerate_device_extension_properties(phd)
        .unwrap();
    let supported_extensions = supported_extension_properties
        .iter()
        .map(|ext_prop| ffi::CStr::from_ptr(ext_prop.extension_name.as_ptr()))
        .collect::<Vec<_>>();
    for extension in REQUIRED_DEVICE_EXTENSIONS {
        if !supported_extensions.contains(extension) {
            log::warn!(
                "Rejected for device extension {:?} not supported. Please update the driver!",
                extension
            );
            return None;
        }
    }

    let bugs = SystemBugs {
        //Note: this is somewhat broad across X11/Wayland and different drivers.
        // It could be narrower, but at the end of the day if the user forced Prime
        // for GLX it should be safe to assume they want it for Vulkan as well.
        intel_unable_to_present: is_nvidia_prime_forced()
            && properties.vendor_id == db::intel::VENDOR,
        intel_fix_descriptor_pool_leak: cfg!(windows) && properties.vendor_id == db::intel::VENDOR,
    };

    let queue_family_index = 0; //TODO
    if desc.presentation && bugs.intel_unable_to_present {
        log::warn!("Rejecting Intel for not presenting when Nvidia is present (on Linux)");
        return None;
    }

    let mut inline_uniform_block_features =
        vk::PhysicalDeviceInlineUniformBlockFeaturesEXT::default();
    let mut timeline_semaphore_features = vk::PhysicalDeviceTimelineSemaphoreFeaturesKHR::default();
    let mut dynamic_rendering_features = vk::PhysicalDeviceDynamicRenderingFeaturesKHR::default();
    let mut descriptor_indexing_features =
        vk::PhysicalDeviceDescriptorIndexingFeaturesEXT::default();
    let mut buffer_device_address_features =
        vk::PhysicalDeviceBufferDeviceAddressFeaturesKHR::default();
    let mut acceleration_structure_features =
        vk::PhysicalDeviceAccelerationStructureFeaturesKHR::default();
    let mut ray_query_features = vk::PhysicalDeviceRayQueryFeaturesKHR::default();
    let mut features2_khr = vk::PhysicalDeviceFeatures2::default()
        .push_next(&mut inline_uniform_block_features)
        .push_next(&mut timeline_semaphore_features)
        .push_next(&mut dynamic_rendering_features)
        .push_next(&mut descriptor_indexing_features)
        .push_next(&mut buffer_device_address_features)
        .push_next(&mut acceleration_structure_features)
        .push_next(&mut ray_query_features);
    instance
        .get_physical_device_properties2
        .get_physical_device_features2(phd, &mut features2_khr);

    if inline_uniform_block_properties.max_inline_uniform_block_size
        < crate::limits::PLAIN_DATA_SIZE
        || inline_uniform_block_properties.max_descriptor_set_inline_uniform_blocks == 0
        || inline_uniform_block_features.inline_uniform_block == 0
    {
        log::warn!(
            "\tRejected for inline uniform blocks. Properties = {:?}, Features = {:?}",
            inline_uniform_block_properties,
            inline_uniform_block_features,
        );
        return None;
    }

    if timeline_semaphore_features.timeline_semaphore == 0 {
        log::warn!(
            "\tRejected for timeline semaphore. Properties = {:?}, Features = {:?}",
            timeline_semaphore_properties,
            timeline_semaphore_features,
        );
        return None;
    }

    if dynamic_rendering_features.dynamic_rendering == 0 {
        log::warn!(
            "\tRejected for dynamic rendering. Features = {:?}",
            dynamic_rendering_features,
        );
        return None;
    }

    let external_memory = supported_extensions.contains(&vk::KHR_EXTERNAL_MEMORY_NAME);
    let external_memory = external_memory
        && supported_extensions.contains(if cfg!(target_os = "windows") {
            &vk::KHR_EXTERNAL_MEMORY_WIN32_NAME
        } else {
            &vk::KHR_EXTERNAL_MEMORY_FD_NAME
        });

    let timing = if properties.limits.timestamp_compute_and_graphics == vk::FALSE {
        log::info!("No timing because of queue support");
        false
    } else {
        true
    };

    let ray_tracing = if !supported_extensions.contains(&vk::KHR_ACCELERATION_STRUCTURE_NAME)
        || !supported_extensions.contains(&vk::KHR_RAY_QUERY_NAME)
    {
        log::info!("No ray tracing extensions are supported");
        None
    } else if descriptor_indexing_properties.max_per_stage_update_after_bind_resources == vk::FALSE
        || descriptor_indexing_features.descriptor_binding_partially_bound == vk::FALSE
        || descriptor_indexing_features.shader_storage_buffer_array_non_uniform_indexing
            == vk::FALSE
        || descriptor_indexing_features.shader_sampled_image_array_non_uniform_indexing == vk::FALSE
    {
        log::info!(
            "No ray tracing because of the descriptor indexing. Properties = {:?}. Features = {:?}",
            descriptor_indexing_properties,
            descriptor_indexing_features
        );
        None
    } else if buffer_device_address_features.buffer_device_address == vk::FALSE {
        log::info!(
            "No ray tracing because of the buffer device address. Features = {:?}",
            buffer_device_address_features
        );
        None
    } else if acceleration_structure_properties.max_geometry_count == 0
        || acceleration_structure_features.acceleration_structure == vk::FALSE
    {
        log::info!("No ray tracing because of the acceleration structure. Properties = {:?}. Features = {:?}",
            acceleration_structure_properties, acceleration_structure_features);
        None
    } else if ray_query_features.ray_query == vk::FALSE {
        log::info!(
            "No ray tracing because of the ray query. Features = {:?}",
            ray_query_features
        );
        None
    } else {
        log::info!("Ray tracing is supported");
        log::debug!("Ray tracing properties: {acceleration_structure_properties:#?}");
        Some(RayTracingCapabilities {
            min_scratch_buffer_alignment: acceleration_structure_properties
                .min_acceleration_structure_scratch_offset_alignment
                as u64,
        })
    };

    let buffer_marker = supported_extensions.contains(&vk::AMD_BUFFER_MARKER_NAME);
    let shader_info = supported_extensions.contains(&vk::AMD_SHADER_INFO_NAME);
    let full_screen_exclusive = supported_extensions.contains(&vk::EXT_FULL_SCREEN_EXCLUSIVE_NAME);

    let device_information = crate::DeviceInformation {
        is_software_emulated: properties.device_type == vk::PhysicalDeviceType::CPU,
        device_name: ffi::CStr::from_ptr(properties.device_name.as_ptr())
            .to_string_lossy()
            .to_string(),
        driver_name: ffi::CStr::from_ptr(driver_properties.driver_name.as_ptr())
            .to_string_lossy()
            .to_string(),
        driver_info: ffi::CStr::from_ptr(driver_properties.driver_info.as_ptr())
            .to_string_lossy()
            .to_string(),
    };

    Some(AdapterCapabilities {
        api_version,
        properties,
        device_information,
        queue_family_index,
        layered: portability_subset_properties.min_vertex_input_binding_stride_alignment != 0,
        ray_tracing,
        buffer_marker,
        shader_info,
        full_screen_exclusive,
        external_memory,
        timing,
        bugs,
    })
}

impl super::Context {
    pub unsafe fn init(desc: crate::ContextDesc) -> Result<Self, NotSupportedError> {
        let entry = match ash::Entry::load() {
            Ok(entry) => entry,
            Err(err) => {
                log::error!("Missing Vulkan entry points: {:?}", err);
                return Err(super::PlatformError::Loading(err).into());
            }
        };
        let driver_api_version = match entry.try_enumerate_instance_version() {
            // Vulkan 1.1+
            Ok(Some(version)) => version,
            Ok(None) => return Err(NotSupportedError::NoSupportedDeviceFound),
            Err(err) => {
                log::error!("try_enumerate_instance_version: {:?}", err);
                return Err(super::PlatformError::Init(err).into());
            }
        };

        let supported_layers = match entry.enumerate_instance_layer_properties() {
            Ok(layers) => layers,
            Err(err) => {
                log::error!("enumerate_instance_layer_properties: {:?}", err);
                return Err(super::PlatformError::Init(err).into());
            }
        };
        let supported_layer_names = supported_layers
            .iter()
            .map(|properties| ffi::CStr::from_ptr(properties.layer_name.as_ptr()))
            .collect::<Vec<_>>();

        let mut layers: Vec<&'static ffi::CStr> = Vec::new();
        let mut requested_layers = Vec::<&ffi::CStr>::new();
        if desc.validation {
            requested_layers.push(layer::KHRONOS_VALIDATION);
        }
        if desc.overlay {
            requested_layers.push(layer::MESA_OVERLAY);
        }
        for name in requested_layers {
            if supported_layer_names.contains(&name) {
                layers.push(name);
            } else {
                log::warn!("Requested layer is not found: {:?}", name);
            }
        }

        let supported_instance_extension_properties =
            match entry.enumerate_instance_extension_properties(None) {
                Ok(extensions) => extensions,
                Err(err) => {
                    log::error!("enumerate_instance_extension_properties: {:?}", err);
                    return Err(super::PlatformError::Init(err).into());
                }
            };
        let supported_instance_extensions = supported_instance_extension_properties
            .iter()
            .map(|ext_prop| ffi::CStr::from_ptr(ext_prop.extension_name.as_ptr()))
            .collect::<Vec<_>>();

        let core_instance = {
            let mut create_flags = vk::InstanceCreateFlags::empty();

            let mut instance_extensions = vec![
                vk::EXT_DEBUG_UTILS_NAME,
                vk::KHR_GET_PHYSICAL_DEVICE_PROPERTIES2_NAME,
            ];
            if desc.presentation {
                instance_extensions.push(vk::KHR_SURFACE_NAME);
                instance_extensions.push(vk::KHR_GET_SURFACE_CAPABILITIES2_NAME);
                let candidates = [
                    vk::KHR_WAYLAND_SURFACE_NAME,
                    vk::KHR_XCB_SURFACE_NAME,
                    vk::KHR_XLIB_SURFACE_NAME,
                    vk::KHR_WIN32_SURFACE_NAME,
                    vk::KHR_ANDROID_SURFACE_NAME,
                    vk::EXT_SWAPCHAIN_COLORSPACE_NAME,
                ];
                for candidate in candidates.iter() {
                    if supported_instance_extensions.contains(candidate) {
                        log::info!("Presentation support: {:?}", candidate);
                        instance_extensions.push(candidate);
                    }
                }
            }

            for inst_ext in instance_extensions.iter() {
                if !supported_instance_extensions.contains(inst_ext) {
                    log::error!("Instance extension {:?} is not supported", inst_ext);
                    return Err(NotSupportedError::NoSupportedDeviceFound);
                }
            }
            if supported_instance_extensions.contains(&vk::KHR_PORTABILITY_ENUMERATION_NAME) {
                log::info!("Enabling Vulkan Portability");
                instance_extensions.push(vk::KHR_PORTABILITY_ENUMERATION_NAME);
                create_flags |= vk::InstanceCreateFlags::ENUMERATE_PORTABILITY_KHR;
            }

            let app_info = vk::ApplicationInfo::default()
                .engine_name(ffi::CStr::from_bytes_with_nul(b"blade\0").unwrap())
                .engine_version(1)
                .api_version(vk::HEADER_VERSION_COMPLETE);
            let str_pointers = layers
                .iter()
                .chain(instance_extensions.iter())
                .map(|&s| s.as_ptr())
                .collect::<Vec<_>>();
            let (layer_strings, extension_strings) = str_pointers.split_at(layers.len());
            let create_info = vk::InstanceCreateInfo::default()
                .application_info(&app_info)
                .flags(create_flags)
                .enabled_layer_names(layer_strings)
                .enabled_extension_names(extension_strings);
            unsafe { entry.create_instance(&create_info, None) }
                .map_err(super::PlatformError::Init)?
        };

        let instance =
            super::Instance {
                _debug_utils: ext::debug_utils::Instance::new(&entry, &core_instance),
                get_physical_device_properties2:
                    khr::get_physical_device_properties2::Instance::new(&entry, &core_instance),
                get_surface_capabilities2: if desc.presentation {
                    Some(khr::get_surface_capabilities2::Instance::new(
                        &entry,
                        &core_instance,
                    ))
                } else {
                    None
                },
                surface: if desc.presentation {
                    Some(khr::surface::Instance::new(&entry, &core_instance))
                } else {
                    None
                },
                core: core_instance,
            };

        let physical_devices = instance
            .core
            .enumerate_physical_devices()
            .map_err(super::PlatformError::Init)?;
        let (physical_device, capabilities) = physical_devices
            .into_iter()
            .find_map(|phd| {
                inspect_adapter(phd, &instance, driver_api_version, &desc).map(|caps| (phd, caps))
            })
            .ok_or_else(|| NotSupportedError::NoSupportedDeviceFound)?;

        log::debug!("Adapter {:#?}", capabilities);
        let mut min_buffer_alignment = 1;
        if let Some(ref rt) = capabilities.ray_tracing {
            min_buffer_alignment = min_buffer_alignment.max(rt.min_scratch_buffer_alignment);
        }

        let device_core = {
            let family_info = vk::DeviceQueueCreateInfo::default()
                .queue_family_index(capabilities.queue_family_index)
                .queue_priorities(&[1.0]);
            let family_infos = [family_info];

            let mut device_extensions = REQUIRED_DEVICE_EXTENSIONS.to_vec();
            if desc.presentation {
                device_extensions.push(vk::KHR_SWAPCHAIN_NAME);
            }
            if capabilities.layered {
                log::info!("Enabling Vulkan Portability");
                device_extensions.push(vk::KHR_PORTABILITY_SUBSET_NAME);
            }
            if capabilities.ray_tracing.is_some() {
                if capabilities.api_version < vk::API_VERSION_1_2 {
                    device_extensions.push(vk::EXT_DESCRIPTOR_INDEXING_NAME);
                    device_extensions.push(vk::KHR_BUFFER_DEVICE_ADDRESS_NAME);
                    device_extensions.push(vk::KHR_SHADER_FLOAT_CONTROLS_NAME);
                    device_extensions.push(vk::KHR_SPIRV_1_4_NAME);
                }
                device_extensions.push(vk::KHR_DEFERRED_HOST_OPERATIONS_NAME);
                device_extensions.push(vk::KHR_ACCELERATION_STRUCTURE_NAME);
                device_extensions.push(vk::KHR_RAY_QUERY_NAME);
            }
            if capabilities.buffer_marker {
                device_extensions.push(vk::AMD_BUFFER_MARKER_NAME);
            }
            if capabilities.shader_info {
                device_extensions.push(vk::AMD_SHADER_INFO_NAME);
            }
            if capabilities.full_screen_exclusive {
                device_extensions.push(vk::EXT_FULL_SCREEN_EXCLUSIVE_NAME);
            }
            if capabilities.external_memory {
                device_extensions.push(vk::KHR_EXTERNAL_MEMORY_NAME);
                device_extensions.push(if cfg!(target_os = "windows") {
                    vk::KHR_EXTERNAL_MEMORY_WIN32_NAME
                } else {
                    vk::KHR_EXTERNAL_MEMORY_FD_NAME
                });
            }

            let str_pointers = device_extensions
                .iter()
                .map(|&s| s.as_ptr())
                .collect::<Vec<_>>();

            let mut ext_inline_uniform_block = vk::PhysicalDeviceInlineUniformBlockFeaturesEXT {
                inline_uniform_block: vk::TRUE,
                ..Default::default()
            };
            let mut khr_timeline_semaphore = vk::PhysicalDeviceTimelineSemaphoreFeaturesKHR {
                timeline_semaphore: vk::TRUE,
                ..Default::default()
            };
            let mut khr_dynamic_rendering = vk::PhysicalDeviceDynamicRenderingFeaturesKHR {
                dynamic_rendering: vk::TRUE,
                ..Default::default()
            };
            let mut device_create_info = vk::DeviceCreateInfo::default()
                .queue_create_infos(&family_infos)
                .enabled_extension_names(&str_pointers)
                .push_next(&mut ext_inline_uniform_block)
                .push_next(&mut khr_timeline_semaphore)
                .push_next(&mut khr_dynamic_rendering);

            let mut ext_descriptor_indexing;
            let mut khr_buffer_device_address;
            let mut khr_acceleration_structure;
            let mut khr_ray_query;
            if capabilities.ray_tracing.is_some() {
                ext_descriptor_indexing = vk::PhysicalDeviceDescriptorIndexingFeaturesEXT {
                    shader_storage_buffer_array_non_uniform_indexing: vk::TRUE,
                    shader_sampled_image_array_non_uniform_indexing: vk::TRUE,
                    descriptor_binding_partially_bound: vk::TRUE,
                    ..Default::default()
                };
                khr_buffer_device_address = vk::PhysicalDeviceBufferDeviceAddressFeaturesKHR {
                    buffer_device_address: vk::TRUE,
                    ..Default::default()
                };
                khr_acceleration_structure = vk::PhysicalDeviceAccelerationStructureFeaturesKHR {
                    acceleration_structure: vk::TRUE,
                    ..Default::default()
                };
                khr_ray_query = vk::PhysicalDeviceRayQueryFeaturesKHR {
                    ray_query: vk::TRUE,
                    ..Default::default()
                };
                device_create_info = device_create_info
                    .push_next(&mut ext_descriptor_indexing)
                    .push_next(&mut khr_buffer_device_address)
                    .push_next(&mut khr_acceleration_structure)
                    .push_next(&mut khr_ray_query);
            }

            instance
                .core
                .create_device(physical_device, &device_create_info, None)
                .map_err(super::PlatformError::Init)?
        };

        let device = super::Device {
            swapchain: if desc.presentation {
                Some(khr::swapchain::Device::new(&instance.core, &device_core))
            } else {
                None
            },
            debug_utils: ext::debug_utils::Device::new(&instance.core, &device_core),
            timeline_semaphore: khr::timeline_semaphore::Device::new(&instance.core, &device_core),
            dynamic_rendering: khr::dynamic_rendering::Device::new(&instance.core, &device_core),
            ray_tracing: if let Some(ref caps) = capabilities.ray_tracing {
                Some(super::RayTracingDevice {
                    acceleration_structure: khr::acceleration_structure::Device::new(
                        &instance.core,
                        &device_core,
                    ),
                    scratch_buffer_alignment: caps.min_scratch_buffer_alignment,
                })
            } else {
                None
            },
            buffer_marker: if capabilities.buffer_marker && desc.validation {
                Some(amd::buffer_marker::Device::new(
                    &instance.core,
                    &device_core,
                ))
            } else {
                None
            },
            shader_info: if capabilities.shader_info {
                Some(amd::shader_info::Device::new(&instance.core, &device_core))
            } else {
                None
            },
            full_screen_exclusive: if desc.presentation && capabilities.full_screen_exclusive {
                Some(ext::full_screen_exclusive::Device::new(
                    &instance.core,
                    &device_core,
                ))
            } else {
                None
            },
            external_memory: if capabilities.external_memory {
                #[cfg(not(target_os = "windows"))]
                use khr::external_memory_fd::Device;
                #[cfg(target_os = "windows")]
                use khr::external_memory_win32::Device;

                Some(Device::new(&instance.core, &device_core))
            } else {
                None
            },
            core: device_core,
            device_information: capabilities.device_information,
            command_scope: if desc.capture {
                Some(super::CommandScopeDevice {})
            } else {
                None
            },
            timing: if desc.timing && capabilities.timing {
                Some(super::TimingDevice {
                    period: capabilities.properties.limits.timestamp_period,
                })
            } else {
                None
            },
            //TODO: detect GPU family
            workarounds: super::Workarounds {
                extra_sync_src_access: vk::AccessFlags::TRANSFER_WRITE,
                extra_sync_dst_access: vk::AccessFlags::TRANSFER_WRITE
                    | vk::AccessFlags::TRANSFER_READ
                    | if capabilities.ray_tracing.is_some() {
                        vk::AccessFlags::ACCELERATION_STRUCTURE_WRITE_KHR
                    } else {
                        vk::AccessFlags::NONE
                    },
                extra_descriptor_pool_create_flags: if capabilities
                    .bugs
                    .intel_fix_descriptor_pool_leak
                {
                    vk::DescriptorPoolCreateFlags::FREE_DESCRIPTOR_SET
                } else {
                    vk::DescriptorPoolCreateFlags::empty()
                },
            },
        };

        let memory_manager = {
            let mem_properties = instance
                .core
                .get_physical_device_memory_properties(physical_device);
            let memory_types =
                &mem_properties.memory_types[..mem_properties.memory_type_count as usize];
            let limits = &capabilities.properties.limits;
            let config = gpu_alloc::Config::i_am_prototyping(); //TODO?

            let properties = gpu_alloc::DeviceProperties {
                max_memory_allocation_count: limits.max_memory_allocation_count,
                max_memory_allocation_size: u64::max_value(), // TODO
                non_coherent_atom_size: limits.non_coherent_atom_size,
                memory_types: memory_types
                    .iter()
                    .map(|memory_type| gpu_alloc::MemoryType {
                        props: gpu_alloc::MemoryPropertyFlags::from_bits_truncate(
                            memory_type.property_flags.as_raw() as u8,
                        ),
                        heap: memory_type.heap_index,
                    })
                    .collect(),
                memory_heaps: mem_properties.memory_heaps
                    [..mem_properties.memory_heap_count as usize]
                    .iter()
                    .map(|&memory_heap| gpu_alloc::MemoryHeap {
                        size: memory_heap.size,
                    })
                    .collect(),
                buffer_device_address: capabilities.ray_tracing.is_some(),
            };

            let known_memory_flags = vk::MemoryPropertyFlags::DEVICE_LOCAL
                | vk::MemoryPropertyFlags::HOST_VISIBLE
                | vk::MemoryPropertyFlags::HOST_COHERENT
                | vk::MemoryPropertyFlags::HOST_CACHED
                | vk::MemoryPropertyFlags::LAZILY_ALLOCATED;
            let valid_ash_memory_types = memory_types.iter().enumerate().fold(0, |u, (i, mem)| {
                if !known_memory_flags.contains(mem.property_flags) {
                    log::debug!(
                        "Skipping memory type={} for having unknown flags: {:?}",
                        i,
                        mem.property_flags & !known_memory_flags
                    );
                    u
                } else if mem
                    .property_flags
                    .contains(vk::MemoryPropertyFlags::HOST_VISIBLE)
                    && !mem
                        .property_flags
                        .contains(vk::MemoryPropertyFlags::HOST_COHERENT)
                {
                    //TODO: see if and how we can support this
                    log::debug!("Skipping memory type={} for lack of host coherency", i);
                    u
                } else {
                    u | (1 << i)
                }
            });
            super::MemoryManager {
                allocator: gpu_alloc::GpuAllocator::new(config, properties),
                slab: slab::Slab::new(),
                valid_ash_memory_types,
            }
        };

        let queue = device
            .core
            .get_device_queue(capabilities.queue_family_index, 0);
        let last_progress = 0;
        let mut timeline_info = vk::SemaphoreTypeCreateInfo {
            semaphore_type: vk::SemaphoreType::TIMELINE,
            initial_value: last_progress,
            ..Default::default()
        };
        let timeline_semaphore_create_info =
            vk::SemaphoreCreateInfo::default().push_next(&mut timeline_info);
        let timeline_semaphore = device
            .core
            .create_semaphore(&timeline_semaphore_create_info, None)
            .unwrap();

        let mut naga_flags = spv::WriterFlags::FORCE_POINT_SIZE;
        let shader_debug_path = if desc.validation || desc.capture {
            use std::{env, fs};
            naga_flags |= spv::WriterFlags::DEBUG;
            let dir = env::temp_dir().join("blade");
            let _ = fs::create_dir(&dir);
            Some(dir)
        } else {
            None
        };

        Ok(super::Context {
            memory: Mutex::new(memory_manager),
            device,
            queue_family_index: capabilities.queue_family_index,
            queue: Mutex::new(super::Queue {
                raw: queue,
                timeline_semaphore,
                last_progress,
            }),
            physical_device,
            naga_flags,
            shader_debug_path,
            min_buffer_alignment,
            sample_count_flags: capabilities
                .properties
                .limits
                .framebuffer_color_sample_counts
                & capabilities
                    .properties
                    .limits
                    .framebuffer_depth_sample_counts,
            instance,
            entry,
        })
    }

    pub(super) fn set_object_name<T: vk::Handle>(&self, object: T, name: &str) {
        let name_cstr = ffi::CString::new(name).unwrap();
        let name_info = vk::DebugUtilsObjectNameInfoEXT::default()
            .object_handle(object)
            .object_name(&name_cstr);
        let _ = unsafe {
            self.device
                .debug_utils
                .set_debug_utils_object_name(&name_info)
        };
    }

    pub fn capabilities(&self) -> crate::Capabilities {
        crate::Capabilities {
            ray_query: match self.device.ray_tracing {
                Some(_) => crate::ShaderVisibility::all(),
                None => crate::ShaderVisibility::empty(),
            },
            sample_count_mask: self.sample_count_flags.as_raw(),
        }
    }

    pub fn device_information(&self) -> &crate::DeviceInformation {
        &self.device.device_information
    }
}

impl Drop for super::Context {
    fn drop(&mut self) {
        if std::thread::panicking() {
            return;
        }
        unsafe {
            if let Ok(queue) = self.queue.lock() {
                let _ = self.device.core.queue_wait_idle(queue.raw);
                self.device
                    .core
                    .destroy_semaphore(queue.timeline_semaphore, None);
            }
            self.device.core.destroy_device(None);
            self.instance.core.destroy_instance(None);
        }
    }
}
