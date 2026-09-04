use super::*;
use collections::FxHashMap;

pub(in crate::wgpu_renderer) struct SurfaceCache {
    texture_cache: core_video::metal_texture_cache::CVMetalTextureCache,
    surfaces: FxHashMap<usize, CachedSurface>,
}

impl SurfaceCache {
    pub(in crate::wgpu_renderer) fn new(device: &wgpu::Device) -> anyhow::Result<Self> {
        use metal::foreign_types::ForeignTypeRef as _;

        let hal_device = unsafe { device.as_hal::<wgpu::hal::api::Metal>() }
            .ok_or_else(|| anyhow::anyhow!("macOS WGPU device did not expose the Metal HAL"))?;
        let raw_device = objc2::rc::Retained::as_ptr(hal_device.raw_device())
            .cast_mut()
            .cast();
        let metal_device = unsafe { metal::DeviceRef::from_ptr(raw_device) }.to_owned();
        let texture_cache =
            core_video::metal_texture_cache::CVMetalTextureCache::new(None, metal_device, None)
                .map_err(|error| {
                    anyhow::anyhow!("failed to create CoreVideo Metal texture cache: {error}")
                })?;
        Ok(Self {
            texture_cache,
            surfaces: FxHashMap::default(),
        })
    }
}

struct CachedSurface {
    _luma_texture: wgpu::Texture,
    _chroma_texture: wgpu::Texture,
    binding: SurfaceBinding,
}

pub(super) fn retain_surface_cache(renderer: &WgpuRenderer, surfaces: &[PaintSurface]) {
    let active_keys = surfaces
        .iter()
        .filter_map(|surface| {
            let gpui::SurfaceSource::Surface(image_buffer) = &surface.source else {
                return None;
            };
            core_video_surface_key(image_buffer).ok()
        })
        .collect::<smallvec::SmallVec<[usize; 4]>>();
    renderer
        .resources()
        .surface_cache
        .borrow_mut()
        .surfaces
        .retain(|key, _| active_keys.contains(key));
}

pub(super) fn draw_surfaces(
    renderer: &WgpuRenderer,
    surfaces: &[PaintSurface],
    pass: &mut wgpu::RenderPass<'_>,
) -> frame::DrawResult {
    use core_video::pixel_buffer::kCVPixelFormatType_420YpCbCr8BiPlanarFullRange;

    let mut keyed_surfaces = smallvec::SmallVec::<[(&PaintSurface, usize); 4]>::new();
    for surface in surfaces {
        let gpui::SurfaceSource::Surface(image_buffer) = &surface.source else {
            log::error!("surface source cannot be imported by the macOS renderer");
            return Err(frame::DrawError::ExternalSurface);
        };
        if image_buffer.get_pixel_format() != kCVPixelFormatType_420YpCbCr8BiPlanarFullRange {
            log::error!("unsupported CoreVideo surface pixel format");
            return Err(frame::DrawError::ExternalSurface);
        }
        keyed_surfaces.push((surface, core_video_surface_key(image_buffer)?));
    }

    let resources = renderer.resources();
    let mut cache = resources.surface_cache.borrow_mut();

    for (surface, key) in keyed_surfaces {
        let gpui::SurfaceSource::Surface(image_buffer) = &surface.source else {
            return Err(frame::DrawError::ExternalSurface);
        };
        let mut imported = cache.surfaces.remove(&key).map(Ok).unwrap_or_else(|| {
            create_core_video_surface(renderer, &cache.texture_cache, image_buffer)
        })?;
        renderer.draw_surface_binding(
            surface,
            SurfaceColorFormat::Yuv,
            &mut imported.binding,
            pass,
        )?;
        cache.surfaces.insert(key, imported);
    }
    Ok(())
}

fn core_video_surface_key(
    image_buffer: &core_video::pixel_buffer::CVPixelBuffer,
) -> Result<usize, frame::DrawError> {
    use core_foundation::base::TCFType as _;

    let io_surface = unsafe {
        core_video::pixel_buffer_io_surface::CVPixelBufferGetIOSurface(
            image_buffer.as_concrete_TypeRef(),
        )
    };
    if io_surface.is_null() {
        log::error!(
            "CoreVideo surface is not IOSurface-backed; allocate it with \
             kCVPixelBufferIOSurfacePropertiesKey and \
             kCVPixelBufferMetalCompatibilityKey"
        );
        return Err(frame::DrawError::ExternalSurface);
    }
    Ok(io_surface as usize)
}

fn create_core_video_surface(
    renderer: &WgpuRenderer,
    texture_cache: &core_video::metal_texture_cache::CVMetalTextureCache,
    image_buffer: &core_video::pixel_buffer::CVPixelBuffer,
) -> Result<CachedSurface, frame::DrawError> {
    use core_foundation::base::TCFType as _;
    use core_video::metal_texture::CVMetalTextureGetTexture;

    let resources = renderer.resources();
    let luma = texture_cache
        .create_texture_from_image(
            image_buffer.as_concrete_TypeRef(),
            None,
            metal::MTLPixelFormat::R8Unorm,
            image_buffer.get_width_of_plane(0),
            image_buffer.get_height_of_plane(0),
            0,
        )
        .map_err(|error| {
            log::error!("failed to create CoreVideo luma texture: {error}");
            frame::DrawError::ExternalSurface
        })?;
    let chroma = texture_cache
        .create_texture_from_image(
            image_buffer.as_concrete_TypeRef(),
            None,
            metal::MTLPixelFormat::RG8Unorm,
            image_buffer.get_width_of_plane(1),
            image_buffer.get_height_of_plane(1),
            1,
        )
        .map_err(|error| {
            log::error!("failed to create CoreVideo chroma texture: {error}");
            frame::DrawError::ExternalSurface
        })?;
    let luma_texture = unsafe {
        import_core_video_texture(
            &resources.device,
            CVMetalTextureGetTexture(luma.as_concrete_TypeRef()).cast(),
            wgpu::TextureFormat::R8Unorm,
            plane_size(image_buffer, 0),
        )
    }
    .ok_or(frame::DrawError::ExternalSurface)?;
    let chroma_texture = unsafe {
        import_core_video_texture(
            &resources.device,
            CVMetalTextureGetTexture(chroma.as_concrete_TypeRef()).cast(),
            wgpu::TextureFormat::Rg8Unorm,
            plane_size(image_buffer, 1),
        )
    }
    .ok_or(frame::DrawError::ExternalSurface)?;
    let luma_view = luma_texture.create_view(&wgpu::TextureViewDescriptor::default());
    let chroma_view = chroma_texture.create_view(&wgpu::TextureViewDescriptor::default());
    Ok(CachedSurface {
        _luma_texture: luma_texture,
        _chroma_texture: chroma_texture,
        binding: SurfaceBinding::new(renderer, luma_view, chroma_view),
    })
}

fn plane_size(
    image_buffer: &core_video::pixel_buffer::CVPixelBuffer,
    plane: usize,
) -> wgpu::Extent3d {
    wgpu::Extent3d {
        width: image_buffer.get_width_of_plane(plane) as u32,
        height: image_buffer.get_height_of_plane(plane) as u32,
        depth_or_array_layers: 1,
    }
}

unsafe fn import_core_video_texture(
    device: &wgpu::Device,
    raw_texture: *mut objc2::runtime::AnyObject,
    format: wgpu::TextureFormat,
    size: wgpu::Extent3d,
) -> Option<wgpu::Texture> {
    use objc2::{rc::Retained, runtime::ProtocolObject};

    // SAFETY: CoreVideo returned a live MTLTexture; the retain count transfers into the
    // HAL texture so it outlives the CVMetalTexture wrapper.
    let object = unsafe { Retained::retain(raw_texture) }?;
    let raw =
        unsafe { Retained::cast_unchecked::<ProtocolObject<dyn objc2_metal::MTLTexture>>(object) };
    let hal_texture = unsafe {
        wgpu::hal::metal::Device::texture_from_raw(
            raw,
            format,
            objc2_metal::MTLTextureType::Type2D,
            1,
            1,
            size.into(),
        )
    };
    Some(unsafe {
        device.create_texture_from_hal::<wgpu::hal::api::Metal>(
            hal_texture,
            &wgpu::TextureDescriptor {
                label: Some("core_video_surface_plane"),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
        )
    })
}
