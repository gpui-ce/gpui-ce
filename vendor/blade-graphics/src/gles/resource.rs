use glow::HasContext as _;
use std::{ptr, slice};

impl super::Context {
    pub fn get_bottom_level_acceleration_structure_sizes(
        &self,
        _meshes: &[crate::AccelerationStructureMesh],
    ) -> crate::AccelerationStructureSizes {
        unimplemented!()
    }

    pub fn get_top_level_acceleration_structure_sizes(
        &self,
        _instance_count: u32,
    ) -> crate::AccelerationStructureSizes {
        unimplemented!()
    }

    pub fn create_acceleration_structure_instance_buffer(
        &self,
        _instances: &[crate::AccelerationStructureInstance],
        _bottom_level: &[super::AccelerationStructure],
    ) -> super::Buffer {
        unimplemented!()
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
        let gl = self.lock();

        let raw = unsafe { gl.create_buffer() }.unwrap();
        let mut data = ptr::null_mut();

        let mut storage_flags = 0;
        let mut map_flags = 0;
        let usage = match desc.memory {
            crate::Memory::Device => glow::STATIC_DRAW,
            crate::Memory::Shared => {
                map_flags = glow::MAP_READ_BIT | glow::MAP_WRITE_BIT | glow::MAP_PERSISTENT_BIT;
                storage_flags = glow::MAP_PERSISTENT_BIT
                    | glow::MAP_COHERENT_BIT
                    | glow::MAP_READ_BIT
                    | glow::MAP_WRITE_BIT;
                glow::DYNAMIC_DRAW //TEMP
            }
            crate::Memory::Upload => {
                map_flags =
                    glow::MAP_WRITE_BIT | glow::MAP_PERSISTENT_BIT | glow::MAP_UNSYNCHRONIZED_BIT;
                storage_flags =
                    glow::MAP_PERSISTENT_BIT | glow::MAP_COHERENT_BIT | glow::MAP_WRITE_BIT;
                glow::DYNAMIC_DRAW
            }
            crate::Memory::External(_) => unimplemented!(),
        };

        unsafe {
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(raw));
            if self
                .capabilities
                .contains(super::Capabilities::BUFFER_STORAGE)
            {
                gl.buffer_storage(glow::ARRAY_BUFFER, desc.size as _, None, storage_flags);
                if map_flags != 0 {
                    data = gl.map_buffer_range(glow::ARRAY_BUFFER, 0, desc.size as _, map_flags);
                    assert!(!data.is_null());
                }
            } else {
                gl.buffer_data_size(glow::ARRAY_BUFFER, desc.size as _, usage);
                let data_vec = vec![0; desc.size as usize];
                data = Vec::leak(data_vec).as_mut_ptr();
            }
            gl.bind_buffer(glow::ARRAY_BUFFER, None);
            #[cfg(not(target_arch = "wasm32"))]
            if !desc.name.is_empty() && gl.supports_debug() {
                gl.object_label(glow::BUFFER, std::mem::transmute(raw), Some(desc.name));
            }
        }
        super::Buffer {
            raw,
            size: desc.size,
            data,
        }
    }

    fn sync_buffer(&self, buffer: super::Buffer) {
        if !self
            .capabilities
            .contains(super::Capabilities::BUFFER_STORAGE)
        {
            let gl = self.lock();
            unsafe {
                let data = slice::from_raw_parts(buffer.data, buffer.size as usize);
                gl.bind_buffer(glow::ARRAY_BUFFER, Some(buffer.raw));
                gl.buffer_sub_data_u8_slice(glow::ARRAY_BUFFER, 0, data);
            }
        }
    }

    fn destroy_buffer(&self, buffer: super::Buffer) {
        let gl = self.lock();
        unsafe { gl.delete_buffer(buffer.raw) };
        if !buffer.data.is_null()
            && !self
                .capabilities
                .contains(super::Capabilities::BUFFER_STORAGE)
        {
            unsafe {
                Vec::from_raw_parts(buffer.data, buffer.size as usize, buffer.size as usize);
            }
        }
    }

    fn create_texture(&self, desc: crate::TextureDesc) -> super::Texture {
        let gl = self.lock();
        let format_desc = super::describe_texture_format(desc.format);

        let inner = if crate::TextureUsage::TARGET.contains(desc.usage)
            && desc.dimension == crate::TextureDimension::D2
            && desc.array_layer_count == 1
        {
            let raw = unsafe { gl.create_renderbuffer().unwrap() };
            unsafe {
                gl.bind_renderbuffer(glow::RENDERBUFFER, Some(raw));

                if desc.sample_count <= 1 {
                    gl.renderbuffer_storage(
                        glow::RENDERBUFFER,
                        format_desc.internal,
                        desc.size.width as i32,
                        desc.size.height as i32,
                    );
                } else {
                    gl.renderbuffer_storage_multisample(
                        glow::RENDERBUFFER,
                        desc.sample_count as i32,
                        format_desc.internal,
                        desc.size.width as i32,
                        desc.size.height as i32,
                    );
                }

                gl.bind_renderbuffer(glow::RENDERBUFFER, None);
                #[cfg(not(target_arch = "wasm32"))]
                if !desc.name.is_empty() && gl.supports_debug() {
                    gl.object_label(
                        glow::RENDERBUFFER,
                        std::mem::transmute(raw),
                        Some(desc.name),
                    );
                }
            }

            super::TextureInner::Renderbuffer { raw }
        } else {
            let raw = unsafe { gl.create_texture().unwrap() };

            let target = match desc.dimension {
                crate::TextureDimension::D1 => {
                    if desc.sample_count > 1 {
                        log::warn!("Sample count is ignored: not supported for 1D textures",);
                    }
                    if desc.array_layer_count > 1 {
                        glow::TEXTURE_1D_ARRAY
                    } else {
                        glow::TEXTURE_1D
                    }
                }
                crate::TextureDimension::D2 => {
                    if desc.array_layer_count > 1 {
                        if desc.sample_count <= 1 {
                            glow::TEXTURE_2D_ARRAY
                        } else {
                            glow::TEXTURE_2D_MULTISAMPLE_ARRAY
                        }
                    } else {
                        if desc.sample_count <= 1 {
                            glow::TEXTURE_2D
                        } else {
                            glow::TEXTURE_2D_MULTISAMPLE
                        }
                    }
                }
                crate::TextureDimension::D3 => {
                    if desc.sample_count > 1 {
                        log::warn!("Sample count is ignored: not supported for 3D textures",);
                    }
                    glow::TEXTURE_3D
                }
            };

            unsafe {
                gl.bind_texture(target, Some(raw));
                // Reset default filtering mode. This has to be done before
                // assigning the storage.
                gl.tex_parameter_i32(target, glow::TEXTURE_MIN_FILTER, glow::NEAREST as i32);
                gl.tex_parameter_i32(target, glow::TEXTURE_MAG_FILTER, glow::NEAREST as i32);

                match desc.dimension {
                    crate::TextureDimension::D3 => {
                        gl.tex_storage_3d(
                            target,
                            desc.mip_level_count as i32,
                            format_desc.internal,
                            desc.size.width as i32,
                            desc.size.height as i32,
                            desc.size.depth as i32,
                        );
                    }
                    crate::TextureDimension::D2 => {
                        if desc.sample_count <= 1 {
                            gl.tex_storage_2d(
                                target,
                                desc.mip_level_count as i32,
                                format_desc.internal,
                                desc.size.width as i32,
                                desc.size.height as i32,
                            );
                        } else {
                            assert_eq!(desc.mip_level_count, 1);
                            gl.tex_storage_2d_multisample(
                                target,
                                desc.sample_count as i32,
                                format_desc.internal,
                                desc.size.width as i32,
                                desc.size.height as i32,
                                true,
                            );
                        }
                    }
                    crate::TextureDimension::D1 => {
                        gl.tex_storage_1d(
                            target,
                            desc.mip_level_count as i32,
                            format_desc.internal,
                            desc.size.width as i32,
                        );
                    }
                }

                gl.bind_texture(target, None);
                #[cfg(not(target_arch = "wasm32"))]
                if !desc.name.is_empty() && gl.supports_debug() {
                    gl.object_label(glow::TEXTURE, std::mem::transmute(raw), Some(desc.name));
                }
            }

            super::TextureInner::Texture { raw, target }
        };

        super::Texture {
            inner,
            target_size: [desc.size.width as u16, desc.size.height as u16],
            format: desc.format,
        }
    }

    fn destroy_texture(&self, texture: super::Texture) {
        let gl = self.lock();
        match texture.inner {
            super::TextureInner::Renderbuffer { raw, .. } => unsafe {
                gl.delete_renderbuffer(raw);
            },
            super::TextureInner::Texture { raw, .. } => unsafe {
                gl.delete_texture(raw);
            },
        }
    }

    fn create_texture_view(
        &self,
        texture: super::Texture,
        desc: crate::TextureViewDesc,
    ) -> super::TextureView {
        //TODO: actual reinterpretation
        super::TextureView {
            inner: texture.inner,
            target_size: texture.target_size,
            aspects: desc.format.aspects(),
        }
    }

    fn destroy_texture_view(&self, _view: super::TextureView) {}

    fn create_sampler(&self, desc: crate::SamplerDesc) -> super::Sampler {
        let gl = self.lock();

        let wrap_enums = [
            glow::TEXTURE_WRAP_S,
            glow::TEXTURE_WRAP_T,
            glow::TEXTURE_WRAP_R,
        ];
        let (min, mag) = map_filter_modes(desc.min_filter, desc.mag_filter, desc.mipmap_filter);
        let border = match desc.border_color {
            None | Some(crate::TextureColor::TransparentBlack) => [0.0; 4],
            Some(crate::TextureColor::OpaqueBlack) => [0.0, 0.0, 0.0, 1.0],
            Some(crate::TextureColor::White) => [1.0; 4],
        };

        let raw = unsafe { gl.create_sampler().unwrap() };
        unsafe {
            gl.sampler_parameter_i32(raw, glow::TEXTURE_MIN_FILTER, min as i32);
            gl.sampler_parameter_i32(raw, glow::TEXTURE_MAG_FILTER, mag as i32);

            for (&address_mode, wrap_enum) in desc.address_modes.iter().zip(wrap_enums) {
                gl.sampler_parameter_i32(raw, wrap_enum, map_address_mode(address_mode) as i32)
            }
            if desc.border_color.is_some() {
                gl.sampler_parameter_f32_slice(raw, glow::TEXTURE_BORDER_COLOR, &border)
            }
            gl.sampler_parameter_f32(raw, glow::TEXTURE_MIN_LOD, desc.lod_min_clamp);
            if let Some(clamp) = desc.lod_max_clamp {
                gl.sampler_parameter_f32(raw, glow::TEXTURE_MAX_LOD, clamp);
            }
            if desc.anisotropy_clamp > 1 {
                gl.sampler_parameter_i32(
                    raw,
                    glow::TEXTURE_MAX_ANISOTROPY,
                    desc.anisotropy_clamp as i32,
                );
            }

            if let Some(compare) = desc.compare {
                gl.sampler_parameter_i32(
                    raw,
                    glow::TEXTURE_COMPARE_MODE,
                    glow::COMPARE_REF_TO_TEXTURE as i32,
                );
                gl.sampler_parameter_i32(
                    raw,
                    glow::TEXTURE_COMPARE_FUNC,
                    super::map_compare_func(compare) as i32,
                );
            }

            #[cfg(not(target_arch = "wasm32"))]
            if !desc.name.is_empty() && gl.supports_debug() {
                gl.object_label(glow::SAMPLER, std::mem::transmute(raw), Some(desc.name));
            }
        }
        super::Sampler { raw }
    }

    fn destroy_sampler(&self, sampler: super::Sampler) {
        let gl = self.lock();
        unsafe { gl.delete_sampler(sampler.raw) };
    }

    fn create_acceleration_structure(
        &self,
        _desc: crate::AccelerationStructureDesc,
    ) -> super::AccelerationStructure {
        unimplemented!()
    }

    fn destroy_acceleration_structure(
        &self,
        _acceleration_structure: super::AccelerationStructure,
    ) {
        unimplemented!()
    }
}

fn map_filter_modes(
    min: crate::FilterMode,
    mag: crate::FilterMode,
    mip: crate::FilterMode,
) -> (u32, u32) {
    use crate::FilterMode as Fm;

    let mag_filter = match mag {
        Fm::Nearest => glow::NEAREST,
        Fm::Linear => glow::LINEAR,
    };

    let min_filter = match (min, mip) {
        (Fm::Nearest, Fm::Nearest) => glow::NEAREST_MIPMAP_NEAREST,
        (Fm::Nearest, Fm::Linear) => glow::NEAREST_MIPMAP_LINEAR,
        (Fm::Linear, Fm::Nearest) => glow::LINEAR_MIPMAP_NEAREST,
        (Fm::Linear, Fm::Linear) => glow::LINEAR_MIPMAP_LINEAR,
    };

    (min_filter, mag_filter)
}

fn map_address_mode(mode: crate::AddressMode) -> u32 {
    match mode {
        crate::AddressMode::Repeat => glow::REPEAT,
        crate::AddressMode::MirrorRepeat => glow::MIRRORED_REPEAT,
        crate::AddressMode::ClampToEdge => glow::CLAMP_TO_EDGE,
        crate::AddressMode::ClampToBorder => glow::CLAMP_TO_BORDER,
    }
}
