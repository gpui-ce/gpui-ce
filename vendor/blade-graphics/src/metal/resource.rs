use metal::{MTLDevice as _, MTLResource as _};
use objc2::rc::Retained;
use objc2_foundation::{NSRange, NSString};
use objc2_metal::{self as metal, MTLTexture};
use std::{mem, ptr};

fn map_texture_usage(usage: crate::TextureUsage) -> metal::MTLTextureUsage {
    use crate::TextureUsage as Tu;

    let mut mtl_usage = metal::MTLTextureUsage::Unknown;

    mtl_usage.set(
        metal::MTLTextureUsage::RenderTarget,
        usage.intersects(Tu::TARGET),
    );
    mtl_usage.set(
        metal::MTLTextureUsage::ShaderRead,
        usage.intersects(Tu::RESOURCE),
    );
    mtl_usage.set(
        metal::MTLTextureUsage::ShaderWrite,
        usage.intersects(Tu::STORAGE),
    );

    mtl_usage
}

fn map_view_dimension(
    dimension: crate::ViewDimension,
    sample_count: usize,
) -> metal::MTLTextureType {
    use crate::ViewDimension as Vd;
    use metal::MTLTextureType as Mtt;
    match dimension {
        Vd::D1 => Mtt::Type1D,
        Vd::D1Array => Mtt::Type1DArray,
        Vd::D2 => {
            if sample_count <= 1 {
                Mtt::Type2D
            } else {
                Mtt::Type2DMultisample
            }
        }
        Vd::D2Array => {
            if sample_count <= 1 {
                Mtt::Type2DArray
            } else {
                Mtt::Type2DMultisampleArray
            }
        }
        Vd::D3 => Mtt::Type3D,
        Vd::Cube => Mtt::TypeCube,
        Vd::CubeArray => Mtt::TypeCubeArray,
    }
}

fn map_filter_mode(filter: crate::FilterMode) -> metal::MTLSamplerMinMagFilter {
    use metal::MTLSamplerMinMagFilter as Msf;
    match filter {
        crate::FilterMode::Nearest => Msf::Nearest,
        crate::FilterMode::Linear => Msf::Linear,
    }
}

fn map_address_mode(address: crate::AddressMode) -> metal::MTLSamplerAddressMode {
    use crate::AddressMode as Am;
    use metal::MTLSamplerAddressMode as Msam;
    match address {
        Am::Repeat => Msam::Repeat,
        Am::MirrorRepeat => Msam::MirrorRepeat,
        Am::ClampToEdge => Msam::ClampToEdge,
        Am::ClampToBorder => Msam::ClampToBorderColor,
    }
}

fn map_border_color(color: crate::TextureColor) -> metal::MTLSamplerBorderColor {
    use crate::TextureColor as Tc;
    use metal::MTLSamplerBorderColor as Msbc;
    match color {
        Tc::TransparentBlack => Msbc::TransparentBlack,
        Tc::OpaqueBlack => Msbc::OpaqueBlack,
        Tc::White => Msbc::OpaqueWhite,
    }
}

impl super::Context {
    pub fn get_bottom_level_acceleration_structure_sizes(
        &self,
        meshes: &[crate::AccelerationStructureMesh],
    ) -> crate::AccelerationStructureSizes {
        let descriptor = super::make_bottom_level_acceleration_structure_desc(meshes);
        let accel_sizes = self
            .device
            .lock()
            .unwrap()
            .accelerationStructureSizesWithDescriptor(&descriptor);

        crate::AccelerationStructureSizes {
            data: accel_sizes.accelerationStructureSize as u64,
            scratch: accel_sizes.buildScratchBufferSize as u64,
        }
    }

    pub fn get_top_level_acceleration_structure_sizes(
        &self,
        instance_count: u32,
    ) -> crate::AccelerationStructureSizes {
        let descriptor = metal::MTLInstanceAccelerationStructureDescriptor::descriptor();
        descriptor.setInstanceCount(instance_count as _);

        let accel_sizes = self
            .device
            .lock()
            .unwrap()
            .accelerationStructureSizesWithDescriptor(&descriptor);

        crate::AccelerationStructureSizes {
            data: accel_sizes.accelerationStructureSize as u64,
            scratch: accel_sizes.buildScratchBufferSize as u64,
        }
    }

    pub fn create_acceleration_structure_instance_buffer(
        &self,
        instances: &[crate::AccelerationStructureInstance],
        _bottom_level: &[super::AccelerationStructure],
    ) -> super::Buffer {
        fn packed_vec(v: mint::Vector3<f32>) -> metal::MTLPackedFloat3 {
            metal::MTLPackedFloat3 {
                x: v.x,
                y: v.y,
                z: v.z,
            }
        }
        let mut instance_descriptors = Vec::with_capacity(instances.len());
        for instance in instances {
            let transposed = mint::ColumnMatrix3x4::from(instance.transform);
            instance_descriptors.push(metal::MTLAccelerationStructureUserIDInstanceDescriptor {
                transformationMatrix: metal::MTLPackedFloat4x3 {
                    columns: [
                        packed_vec(transposed.x),
                        packed_vec(transposed.y),
                        packed_vec(transposed.z),
                        packed_vec(transposed.w),
                    ],
                },
                options: metal::MTLAccelerationStructureInstanceOptions::None,
                mask: instance.mask,
                intersectionFunctionTableOffset: 0,
                accelerationStructureIndex: instance.acceleration_structure_index,
                userID: instance.custom_index,
            });
        }
        let object = objc2::rc::autoreleasepool(|_| unsafe {
            self.device
                .lock()
                .unwrap()
                .newBufferWithBytes_length_options(
                    ptr::NonNull::new(instance_descriptors.as_ptr() as *mut _).unwrap(),
                    mem::size_of::<metal::MTLAccelerationStructureUserIDInstanceDescriptor>()
                        * instances.len(),
                    metal::MTLResourceOptions::StorageModeShared,
                )
                .unwrap()
        });
        super::Buffer {
            raw: Retained::into_raw(object),
        }
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
        let options = match desc.memory {
            crate::Memory::Device => metal::MTLResourceOptions::StorageModePrivate,
            crate::Memory::Shared => metal::MTLResourceOptions::StorageModeShared,
            crate::Memory::Upload => {
                metal::MTLResourceOptions::StorageModeShared
                    | metal::MTLResourceOptions::CPUCacheModeWriteCombined
            }
            crate::Memory::External(_) => unimplemented!(),
        };
        let object = objc2::rc::autoreleasepool(|_| {
            self.device
                .lock()
                .unwrap()
                .newBufferWithLength_options(desc.size as usize, options)
                .unwrap()
        });
        if !desc.name.is_empty() {
            object.setLabel(Some(&NSString::from_str(desc.name)));
        }
        super::Buffer {
            raw: Retained::into_raw(object),
        }
    }

    fn sync_buffer(&self, _buffer: super::Buffer) {}

    fn destroy_buffer(&self, buffer: super::Buffer) {
        let _ = unsafe { Retained::from_raw(buffer.raw) };
    }

    fn create_texture(&self, desc: crate::TextureDesc) -> super::Texture {
        let mtl_format = super::map_texture_format(desc.format);

        let mtl_type = match desc.dimension {
            crate::TextureDimension::D1 => {
                if desc.array_layer_count > 1 {
                    metal::MTLTextureType::Type1DArray
                } else {
                    metal::MTLTextureType::Type1D
                }
            }
            crate::TextureDimension::D2 => {
                if desc.array_layer_count > 1 {
                    if desc.sample_count <= 1 {
                        metal::MTLTextureType::Type2DArray
                    } else {
                        metal::MTLTextureType::Type2DMultisampleArray
                    }
                } else {
                    if desc.sample_count <= 1 {
                        metal::MTLTextureType::Type2D
                    } else {
                        metal::MTLTextureType::Type2DMultisample
                    }
                }
            }
            crate::TextureDimension::D3 => metal::MTLTextureType::Type3D,
        };
        let mtl_usage = map_texture_usage(desc.usage);

        let object = objc2::rc::autoreleasepool(|_| unsafe {
            let descriptor = metal::MTLTextureDescriptor::new();
            descriptor.setTextureType(mtl_type);
            descriptor.setWidth(desc.size.width as usize);
            descriptor.setHeight(desc.size.height as usize);
            descriptor.setDepth(desc.size.depth as usize);
            descriptor.setArrayLength(desc.array_layer_count as usize);
            descriptor.setMipmapLevelCount(desc.mip_level_count as usize);
            descriptor.setPixelFormat(mtl_format);
            descriptor.setSampleCount(desc.sample_count as _);
            descriptor.setUsage(mtl_usage);
            descriptor.setStorageMode(metal::MTLStorageMode::Private);

            self.device
                .lock()
                .unwrap()
                .newTextureWithDescriptor(&descriptor)
                .unwrap()
        });
        if !desc.name.is_empty() {
            object.setLabel(Some(&NSString::from_str(desc.name)));
        }
        super::Texture {
            raw: Retained::into_raw(object),
        }
    }

    fn destroy_texture(&self, texture: super::Texture) {
        let _ = unsafe { Retained::from_raw(texture.raw) };
    }

    fn create_texture_view(
        &self,
        texture: super::Texture,
        desc: crate::TextureViewDesc,
    ) -> super::TextureView {
        let texture = texture.as_ref();
        let mtl_format = super::map_texture_format(desc.format);
        let mtl_type = map_view_dimension(desc.dimension, texture.sampleCount());
        let mip_level_count = match desc.subresources.mip_level_count {
            Some(count) => count.get() as usize,
            None => texture.mipmapLevelCount() - desc.subresources.base_mip_level as usize,
        };
        let array_layer_count = match desc.subresources.array_layer_count {
            Some(count) => count.get() as usize,
            None => texture.arrayLength() - desc.subresources.base_array_layer as usize,
        };

        let object = objc2::rc::autoreleasepool(|_| unsafe {
            texture
                .newTextureViewWithPixelFormat_textureType_levels_slices(
                    mtl_format,
                    mtl_type,
                    NSRange {
                        location: desc.subresources.base_mip_level as _,
                        length: mip_level_count,
                    },
                    NSRange {
                        location: desc.subresources.base_array_layer as _,
                        length: array_layer_count,
                    },
                )
                .unwrap()
        });
        if !desc.name.is_empty() {
            object.setLabel(Some(&NSString::from_str(desc.name)));
        }
        super::TextureView {
            raw: Retained::into_raw(object),
            aspects: desc.format.aspects(),
        }
    }

    fn destroy_texture_view(&self, view: super::TextureView) {
        let _ = unsafe { Retained::from_raw(view.raw) };
    }

    fn create_sampler(&self, desc: crate::SamplerDesc) -> super::Sampler {
        let object = objc2::rc::autoreleasepool(|_| {
            let descriptor = metal::MTLSamplerDescriptor::new();

            descriptor.setMinFilter(map_filter_mode(desc.min_filter));
            descriptor.setMagFilter(map_filter_mode(desc.mag_filter));
            descriptor.setMipFilter(match desc.mipmap_filter {
                crate::FilterMode::Nearest => metal::MTLSamplerMipFilter::Nearest,
                crate::FilterMode::Linear => metal::MTLSamplerMipFilter::Linear,
            });

            descriptor.setSAddressMode(map_address_mode(desc.address_modes[0]));
            descriptor.setTAddressMode(map_address_mode(desc.address_modes[1]));
            descriptor.setRAddressMode(map_address_mode(desc.address_modes[2]));

            if desc.anisotropy_clamp > 1 {
                descriptor.setMaxAnisotropy(desc.anisotropy_clamp as usize);
            }

            descriptor.setLodMinClamp(desc.lod_min_clamp);
            if let Some(lod) = desc.lod_max_clamp {
                descriptor.setLodMaxClamp(lod);
            }

            // optimization
            descriptor.setLodAverage(true);

            if let Some(fun) = desc.compare {
                descriptor.setCompareFunction(super::map_compare_function(fun));
            }

            if let Some(border_color) = desc.border_color {
                descriptor.setBorderColor(map_border_color(border_color));
            }

            if !desc.name.is_empty() {
                descriptor.setLabel(Some(&NSString::from_str(desc.name)));
            }
            self.device
                .lock()
                .unwrap()
                .newSamplerStateWithDescriptor(&descriptor)
                .unwrap()
        });

        super::Sampler {
            raw: Retained::into_raw(object),
        }
    }

    fn destroy_sampler(&self, sampler: super::Sampler) {
        let _ = unsafe { Retained::from_raw(sampler.raw) };
    }

    fn create_acceleration_structure(
        &self,
        desc: crate::AccelerationStructureDesc,
    ) -> super::AccelerationStructure {
        let object = objc2::rc::autoreleasepool(|_| {
            //TODO: use `newAccelerationStructureWithDescriptor`
            self.device
                .lock()
                .unwrap()
                .newAccelerationStructureWithSize(desc.size as usize)
                .unwrap()
        });
        if !desc.name.is_empty() {
            object.setLabel(Some(&NSString::from_str(desc.name)));
        }

        super::AccelerationStructure {
            raw: Retained::into_raw(object),
        }
    }

    fn destroy_acceleration_structure(&self, acceleration_structure: super::AccelerationStructure) {
        let _ = unsafe { Retained::from_raw(acceleration_structure.raw) };
    }
}
