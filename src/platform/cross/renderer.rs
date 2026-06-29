use std::collections::HashMap;
use std::mem::ManuallyDrop;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use wgpu::util::DeviceExt;

use crate::{
    AtlasTextureId, AtlasTile, BackdropFilter, DevicePixels, FilterBoundary, GpuSpecs,
    GradientStop, LinearColorStop, MonochromeSprite, Pixels, PlatformAtlas, PrimitiveBatch, Quad,
    ScaledPixels, Scene, TransformationMatrix, color, geometry,
    platform::cross::{
        atlas::WgpuAtlas,
        render_context::{WgpuContext, ensure_buffer_size},
        surface_registry::SurfaceId,
    },
};

const fn map_attributes<const N: usize>(
    attribs: &'static [wgpu::VertexAttribute; N],
    location_offset: u32,
    offset_offset: wgpu::BufferAddress,
) -> [wgpu::VertexAttribute; N] {
    let mut result = [wgpu::VertexAttribute {
        offset: 0,
        shader_location: 0,
        // NOTE(mdeand): Dummy format, will be overwritten.
        format: wgpu::VertexFormat::Uint8x2,
    }; N];
    let mut i = 0;

    while i < result.len() {
        result[i] = wgpu::VertexAttribute {
            offset: attribs[i].offset + offset_offset,
            shader_location: attribs[i].shader_location + location_offset,
            format: attribs[i].format,
        };
        i += 1;
    }

    result
}

impl color::Hsla {
    const VERTEX_ATTRIBUTES: &'static [wgpu::VertexAttribute; 4] = &[
        wgpu::VertexAttribute {
            offset: std::mem::offset_of!(color::Hsla, h) as wgpu::BufferAddress,
            shader_location: 0,
            format: wgpu::VertexFormat::Float32,
        },
        wgpu::VertexAttribute {
            offset: std::mem::offset_of!(color::Hsla, s) as wgpu::BufferAddress,
            shader_location: 1,
            format: wgpu::VertexFormat::Float32,
        },
        wgpu::VertexAttribute {
            offset: std::mem::offset_of!(color::Hsla, l) as wgpu::BufferAddress,
            shader_location: 2,
            format: wgpu::VertexFormat::Float32,
        },
        wgpu::VertexAttribute {
            offset: std::mem::offset_of!(color::Hsla, a) as wgpu::BufferAddress,
            shader_location: 3,
            format: wgpu::VertexFormat::Float32,
        },
    ];
}

impl color::GradientStop {
    const VERTEX_ATTRIBUTES: &'static [wgpu::VertexAttribute; 2] = &[
        wgpu::VertexAttribute {
            offset: std::mem::offset_of!(GradientStop, color) as wgpu::BufferAddress,
            shader_location: 0,
            format: wgpu::VertexFormat::Float32x4,
        },
        wgpu::VertexAttribute {
            offset: std::mem::offset_of!(GradientStop, position) as wgpu::BufferAddress,
            shader_location: 1,
            format: wgpu::VertexFormat::Float32,
        },
    ];
}

impl color::LinearColorStop {
    const VERTEX_ATTRIBUTES: &'static [wgpu::VertexAttribute; 2] = &[
        wgpu::VertexAttribute {
            offset: std::mem::offset_of!(LinearColorStop, color) as wgpu::BufferAddress,
            shader_location: 0,
            format: wgpu::VertexFormat::Float32x4,
        },
        wgpu::VertexAttribute {
            offset: std::mem::offset_of!(LinearColorStop, percentage) as wgpu::BufferAddress,
            shader_location: 1,
            format: wgpu::VertexFormat::Float32,
        },
    ];
}

impl color::Background {
    const VERTEX_ATTRIBUTES: &'static [wgpu::VertexAttribute; 9] = &{
        let linear_color_stop_vertex_attributes = map_attributes(
            GradientStop::VERTEX_ATTRIBUTES,
            7,
            std::mem::offset_of!(color::Background, colors) as wgpu::BufferAddress,
        );

        [
            wgpu::VertexAttribute {
                offset: std::mem::offset_of!(color::Background, tag) as wgpu::BufferAddress,
                shader_location: 0,
                format: wgpu::VertexFormat::Uint32,
            },
            wgpu::VertexAttribute {
                offset: std::mem::offset_of!(color::Background, color_space) as wgpu::BufferAddress,
                shader_location: 1,
                format: wgpu::VertexFormat::Uint32,
            },
            wgpu::VertexAttribute {
                offset: std::mem::offset_of!(color::Background, solid) as wgpu::BufferAddress,
                shader_location: 2,
                format: wgpu::VertexFormat::Uint32,
            },
            wgpu::VertexAttribute {
                offset: std::mem::offset_of!(color::Background, param0) as wgpu::BufferAddress,
                shader_location: 3,
                format: wgpu::VertexFormat::Float32,
            },
            wgpu::VertexAttribute {
                offset: std::mem::offset_of!(color::Background, param1) as wgpu::BufferAddress,
                shader_location: 4,
                format: wgpu::VertexFormat::Float32,
            },
            wgpu::VertexAttribute {
                offset: std::mem::offset_of!(color::Background, param2) as wgpu::BufferAddress,
                shader_location: 5,
                format: wgpu::VertexFormat::Float32,
            },
            wgpu::VertexAttribute {
                offset: std::mem::offset_of!(color::Background, param3) as wgpu::BufferAddress,
                shader_location: 6,
                format: wgpu::VertexFormat::Float32,
            },
            linear_color_stop_vertex_attributes[0],
            linear_color_stop_vertex_attributes[1],
            // wgpu::VertexAttribute {
            //     offset: std::mem::offset_of!(color::Background, pad) as wgpu::BufferAddress,
            //     shader_location: 9,
            //     format: wgpu::VertexFormat::Uint8,
            // },
        ]
    };
}

impl color::TextColor {
    const VERTEX_ATTRIBUTES: &'static [wgpu::VertexAttribute; 7] = &{
        let linear_color_stop_vertex_attributes = map_attributes(
            LinearColorStop::VERTEX_ATTRIBUTES,
            4,
            std::mem::offset_of!(color::TextColor, colors) as wgpu::BufferAddress,
        );

        [
            wgpu::VertexAttribute {
                offset: std::mem::offset_of!(color::TextColor, tag) as wgpu::BufferAddress,
                shader_location: 0,
                format: wgpu::VertexFormat::Uint32,
            },
            wgpu::VertexAttribute {
                offset: std::mem::offset_of!(color::TextColor, color_space) as wgpu::BufferAddress,
                shader_location: 1,
                format: wgpu::VertexFormat::Uint32,
            },
            wgpu::VertexAttribute {
                offset: std::mem::offset_of!(color::TextColor, solid) as wgpu::BufferAddress,
                shader_location: 2,
                format: wgpu::VertexFormat::Uint32,
            },
            wgpu::VertexAttribute {
                offset: std::mem::offset_of!(color::TextColor, gradient_angle_or_reserved)
                    as wgpu::BufferAddress,
                shader_location: 3,
                format: wgpu::VertexFormat::Float32,
            },
            linear_color_stop_vertex_attributes[0],
            linear_color_stop_vertex_attributes[1],
            wgpu::VertexAttribute {
                offset: std::mem::offset_of!(color::TextColor, pad) as wgpu::BufferAddress,
                shader_location: 6,
                format: wgpu::VertexFormat::Uint32,
            },
        ]
    };
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GlobalParams {
    viewport_size: [f32; 2],
    premultimated_alpha: u32,
    pad: u32,
}

impl GlobalParams {
    const VERTEX_ATTRIBUTES: &'static [wgpu::VertexAttribute; 3] = &[
        wgpu::VertexAttribute {
            offset: std::mem::offset_of!(GlobalParams, viewport_size) as wgpu::BufferAddress,
            shader_location: 0,
            format: wgpu::VertexFormat::Float32x2,
        },
        wgpu::VertexAttribute {
            offset: std::mem::offset_of!(GlobalParams, premultimated_alpha) as wgpu::BufferAddress,
            shader_location: 1,
            format: wgpu::VertexFormat::Uint32,
        },
        wgpu::VertexAttribute {
            offset: std::mem::offset_of!(GlobalParams, pad) as wgpu::BufferAddress,
            shader_location: 2,
            format: wgpu::VertexFormat::Uint32,
        },
    ];
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Bounds {
    origin: [f32; 2],
    size: [f32; 2],
}

impl geometry::Corners<ScaledPixels> {
    const VERTEX_ATTRIBUTES: &'static [wgpu::VertexAttribute; 4] = &[
        wgpu::VertexAttribute {
            offset: std::mem::offset_of!(geometry::Corners<ScaledPixels>, top_left)
                as wgpu::BufferAddress,
            shader_location: 0,
            format: wgpu::VertexFormat::Float32,
        },
        wgpu::VertexAttribute {
            offset: std::mem::offset_of!(geometry::Corners<ScaledPixels>, top_right)
                as wgpu::BufferAddress,
            shader_location: 1,
            format: wgpu::VertexFormat::Float32,
        },
        wgpu::VertexAttribute {
            offset: std::mem::offset_of!(geometry::Corners<ScaledPixels>, bottom_right)
                as wgpu::BufferAddress,
            shader_location: 2,
            format: wgpu::VertexFormat::Float32,
        },
        wgpu::VertexAttribute {
            offset: std::mem::offset_of!(geometry::Corners<ScaledPixels>, bottom_left)
                as wgpu::BufferAddress,
            shader_location: 3,
            format: wgpu::VertexFormat::Float32,
        },
    ];
}

impl geometry::Edges<ScaledPixels> {
    const VERTEX_ATTRIBUTES: &'static [wgpu::VertexAttribute; 4] = &[
        wgpu::VertexAttribute {
            offset: std::mem::offset_of!(geometry::Edges<ScaledPixels>, top) as wgpu::BufferAddress,
            shader_location: 0,
            format: wgpu::VertexFormat::Float32,
        },
        wgpu::VertexAttribute {
            offset: std::mem::offset_of!(geometry::Edges<ScaledPixels>, right)
                as wgpu::BufferAddress,
            shader_location: 1,
            format: wgpu::VertexFormat::Float32,
        },
        wgpu::VertexAttribute {
            offset: std::mem::offset_of!(geometry::Edges<ScaledPixels>, bottom)
                as wgpu::BufferAddress,
            shader_location: 2,
            format: wgpu::VertexFormat::Float32,
        },
        wgpu::VertexAttribute {
            offset: std::mem::offset_of!(geometry::Edges<ScaledPixels>, left)
                as wgpu::BufferAddress,
            shader_location: 3,
            format: wgpu::VertexFormat::Float32,
        },
    ];
}

impl Bounds {
    const VERTEX_ATTRIBUTES: &'static [wgpu::VertexAttribute; 2] = &[
        wgpu::VertexAttribute {
            offset: std::mem::offset_of!(Bounds, origin) as wgpu::BufferAddress,
            shader_location: 0,
            format: wgpu::VertexFormat::Float32x2,
        },
        wgpu::VertexAttribute {
            offset: std::mem::offset_of!(Bounds, size) as wgpu::BufferAddress,
            shader_location: 1,
            format: wgpu::VertexFormat::Float32x2,
        },
    ];
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct SurfaceParams {
    bounds: Bounds,
    content_mask: Bounds,
}

impl Quad {
    const VERTEX_ATTRIBUTES: &'static [wgpu::VertexAttribute; 22] = &{
        let bounds_vertex_attributes = map_attributes(
            Bounds::VERTEX_ATTRIBUTES,
            2,
            std::mem::offset_of!(Quad, bounds) as wgpu::BufferAddress,
        );

        let content_mask_vertex_attributes = map_attributes(
            Bounds::VERTEX_ATTRIBUTES,
            4,
            std::mem::offset_of!(Quad, content_mask) as wgpu::BufferAddress,
        );

        let background_vertex_attributes = map_attributes(
            color::Background::VERTEX_ATTRIBUTES,
            6,
            std::mem::offset_of!(Quad, background) as wgpu::BufferAddress,
        );

        let border_color_vertex_attributes = map_attributes(
            color::Hsla::VERTEX_ATTRIBUTES,
            11,
            std::mem::offset_of!(Quad, border_color) as wgpu::BufferAddress,
        );

        let corner_radii_vertex_attributes = map_attributes(
            geometry::Corners::<ScaledPixels>::VERTEX_ATTRIBUTES,
            15,
            std::mem::offset_of!(Quad, corner_radii) as wgpu::BufferAddress,
        );

        let border_widths_vertex_attributes = map_attributes(
            geometry::Edges::<ScaledPixels>::VERTEX_ATTRIBUTES,
            19,
            std::mem::offset_of!(Quad, border_widths) as wgpu::BufferAddress,
        );

        [
            wgpu::VertexAttribute {
                offset: std::mem::offset_of!(Quad, order) as wgpu::BufferAddress,
                shader_location: 0,
                format: wgpu::VertexFormat::Uint32,
            },
            wgpu::VertexAttribute {
                offset: std::mem::offset_of!(Quad, border_style) as wgpu::BufferAddress,
                shader_location: 1,
                format: wgpu::VertexFormat::Uint32,
            },
            bounds_vertex_attributes[0],
            bounds_vertex_attributes[1],
            content_mask_vertex_attributes[0],
            content_mask_vertex_attributes[1],
            background_vertex_attributes[0],
            background_vertex_attributes[1],
            background_vertex_attributes[2],
            background_vertex_attributes[3],
            border_color_vertex_attributes[0],
            border_color_vertex_attributes[1],
            border_color_vertex_attributes[2],
            border_color_vertex_attributes[3],
            corner_radii_vertex_attributes[0],
            corner_radii_vertex_attributes[1],
            corner_radii_vertex_attributes[2],
            corner_radii_vertex_attributes[3],
            border_widths_vertex_attributes[0],
            border_widths_vertex_attributes[1],
            border_widths_vertex_attributes[2],
            border_widths_vertex_attributes[3],
        ]
    };
}

#[repr(C)]
struct QuadsData {
    globals: GlobalParams,
}

#[repr(C)]
struct ShadowsData {
    globals: GlobalParams,
}

#[repr(C)]
struct PathRasterizationData {
    globals: GlobalParams,
}

struct PathsData {
    globals: GlobalParams,
    t_sprite: wgpu::TextureView,
    s_sprite: wgpu::Sampler,
}

/// Per-vertex data uploaded to the GPU for path rendering.
/// Layout must exactly match the `GpuPathVertex` struct in `paths.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuPathVertex {
    xy_position: [f32; 2],         // offset  0
    st_position: [f32; 2],         // offset  8
    hsla: [f32; 4],                // offset 16  (h, s, l, a)
    content_mask_origin: [f32; 2], // offset 32
    content_mask_size: [f32; 2],   // offset 40
} // stride  48

struct UnderlinesData {
    globals: GlobalParams,
}

struct MonoSpritesData {
    globals: GlobalParams,
    gamma_ratios: [f32; 4],
    grayscale_enhanced_contrast: f32,
    t_sprite: wgpu::TextureView,
    s_sprite: wgpu::Sampler,
}

struct PolySpritesData {
    globals: GlobalParams,
    t_sprite: wgpu::TextureView,
    s_sprite: wgpu::Sampler,
}

struct SurfacesData {
    globals: GlobalParams,
    surface_params: SurfaceParams,
    t_y: wgpu::TextureView,
    t_cb_cr: wgpu::TextureView,
    s_texture: wgpu::Sampler,
}

struct PathSprite {
    bounds: geometry::Bounds<f32>,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct PathRasterizationVertex {
    xy_position: geometry::Point<ScaledPixels>,
    st_position: geometry::Point<f32>,
    color: color::Background,
    bounds: geometry::Bounds<f32>,
}

impl PathRasterizationVertex {
    const VERTEX_ATTRIBUTES: &'static [wgpu::VertexAttribute; 10] = &{
        let color_vertex_attributes = map_attributes(
            color::Background::VERTEX_ATTRIBUTES,
            2,
            std::mem::offset_of!(PathRasterizationVertex, color) as wgpu::BufferAddress,
        );

        let bounds_vertex_attributes = map_attributes(
            Bounds::VERTEX_ATTRIBUTES,
            8,
            std::mem::offset_of!(PathRasterizationVertex, bounds) as wgpu::BufferAddress,
        );

        [
            wgpu::VertexAttribute {
                offset: std::mem::offset_of!(PathRasterizationVertex, xy_position)
                    as wgpu::BufferAddress,
                shader_location: 0,
                format: wgpu::VertexFormat::Float32x2,
            },
            wgpu::VertexAttribute {
                offset: std::mem::offset_of!(PathRasterizationVertex, st_position)
                    as wgpu::BufferAddress,
                shader_location: 1,
                format: wgpu::VertexFormat::Float32x2,
            },
            color_vertex_attributes[0],
            color_vertex_attributes[1],
            color_vertex_attributes[2],
            color_vertex_attributes[3],
            color_vertex_attributes[4],
            color_vertex_attributes[5],
            bounds_vertex_attributes[0],
            bounds_vertex_attributes[1],
        ]
    };

    fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<PathRasterizationVertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: Self::VERTEX_ATTRIBUTES,
        }
    }
}

impl AtlasTextureId {
    const VERTEX_ATTRIBUTES: &'static [wgpu::VertexAttribute; 2] = &{
        [
            wgpu::VertexAttribute {
                offset: std::mem::offset_of!(AtlasTextureId, index) as wgpu::BufferAddress,
                format: wgpu::VertexFormat::Uint32,
                shader_location: 0,
            },
            wgpu::VertexAttribute {
                offset: std::mem::offset_of!(AtlasTextureId, kind) as wgpu::BufferAddress,
                format: wgpu::VertexFormat::Uint32,
                shader_location: 1,
            },
        ]
    };
}

#[repr(C)]
struct AtlasBounds {
    origin: [i32; 2],
    size: [i32; 2],
}

impl AtlasBounds {
    const VERTEX_ATTRIBUTES: &'static [wgpu::VertexAttribute; 2] = &{
        [
            wgpu::VertexAttribute {
                offset: std::mem::offset_of!(AtlasBounds, origin) as wgpu::BufferAddress,
                format: wgpu::VertexFormat::Sint32x2,
                shader_location: 0,
            },
            wgpu::VertexAttribute {
                offset: std::mem::offset_of!(AtlasBounds, size) as wgpu::BufferAddress,
                format: wgpu::VertexFormat::Sint32x2,
                shader_location: 1,
            },
        ]
    };
}

impl AtlasTile {
    const VERTEX_ATTRIBUTES: &'static [wgpu::VertexAttribute; 6] = &{
        let texture_id_vertex_attributes = map_attributes(
            AtlasTextureId::VERTEX_ATTRIBUTES,
            0,
            std::mem::offset_of!(AtlasTile, texture_id) as wgpu::BufferAddress,
        );

        let bounds_vertex_attributes = map_attributes(
            AtlasBounds::VERTEX_ATTRIBUTES,
            4,
            std::mem::offset_of!(AtlasTile, bounds) as wgpu::BufferAddress,
        );

        [
            texture_id_vertex_attributes[0],
            texture_id_vertex_attributes[1],
            wgpu::VertexAttribute {
                offset: std::mem::offset_of!(AtlasTile, tile_id) as wgpu::BufferAddress,
                format: wgpu::VertexFormat::Uint32,
                shader_location: 2,
            },
            wgpu::VertexAttribute {
                offset: std::mem::offset_of!(AtlasTile, padding) as wgpu::BufferAddress,
                format: wgpu::VertexFormat::Uint32,
                shader_location: 3,
            },
            bounds_vertex_attributes[0],
            bounds_vertex_attributes[1],
        ]
    };
}

impl TransformationMatrix {
    const VERTEX_ATTRIBUTES: &'static [wgpu::VertexAttribute; 2] = &{
        [
            wgpu::VertexAttribute {
                offset: std::mem::offset_of!(TransformationMatrix, rotation_scale)
                    as wgpu::BufferAddress,
                format: wgpu::VertexFormat::Float32x4,
                shader_location: 0,
            },
            wgpu::VertexAttribute {
                offset: std::mem::offset_of!(TransformationMatrix, translation)
                    as wgpu::BufferAddress,
                format: wgpu::VertexFormat::Float32x2,
                shader_location: 1,
            },
        ]
    };
}

impl MonochromeSprite {
    const VERTEX_ATTRIBUTES: &'static [wgpu::VertexAttribute; 21] = &{
        let bounds_vertex_attributes = map_attributes(
            Bounds::VERTEX_ATTRIBUTES,
            2,
            std::mem::offset_of!(MonochromeSprite, bounds) as wgpu::BufferAddress,
        );

        let content_mask_vertex_attributes = map_attributes(
            Bounds::VERTEX_ATTRIBUTES,
            4,
            std::mem::offset_of!(MonochromeSprite, content_mask) as wgpu::BufferAddress,
        );

        let text_color_vertex_attributes = map_attributes(
            color::TextColor::VERTEX_ATTRIBUTES,
            6,
            std::mem::offset_of!(MonochromeSprite, text_color) as wgpu::BufferAddress,
        );

        let tile_vertex_attributes = map_attributes(
            AtlasTile::VERTEX_ATTRIBUTES,
            8,
            std::mem::offset_of!(MonochromeSprite, tile) as wgpu::BufferAddress,
        );

        let transformation_matrix_vertex_attributes = map_attributes(
            TransformationMatrix::VERTEX_ATTRIBUTES,
            14,
            std::mem::offset_of!(MonochromeSprite, transformation) as wgpu::BufferAddress,
        );

        [
            wgpu::VertexAttribute {
                offset: std::mem::offset_of!(MonochromeSprite, order) as wgpu::BufferAddress,
                format: wgpu::VertexFormat::Uint32,
                shader_location: 0,
            },
            wgpu::VertexAttribute {
                offset: std::mem::offset_of!(MonochromeSprite, pad) as wgpu::BufferAddress,
                format: wgpu::VertexFormat::Uint32,
                shader_location: 1,
            },
            bounds_vertex_attributes[0],
            bounds_vertex_attributes[1],
            content_mask_vertex_attributes[0],
            content_mask_vertex_attributes[1],
            text_color_vertex_attributes[0],
            text_color_vertex_attributes[1],
            text_color_vertex_attributes[2],
            text_color_vertex_attributes[3],
            text_color_vertex_attributes[4],
            text_color_vertex_attributes[5],
            text_color_vertex_attributes[6],
            tile_vertex_attributes[0],
            tile_vertex_attributes[1],
            tile_vertex_attributes[2],
            tile_vertex_attributes[3],
            tile_vertex_attributes[4],
            tile_vertex_attributes[5],
            transformation_matrix_vertex_attributes[0],
            transformation_matrix_vertex_attributes[1],
        ]
    };
}

#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
struct ColorAdjustments {
    gamma_ratios: [f32; 4],
    grayscale_enhanced_contrast: f32,
    _padding: [f32; 3],
}

struct WgpuPipelines {
    color_targets: Vec<Option<wgpu::ColorTargetState>>,

    quads_bind_group_layout: wgpu::BindGroupLayout,
    shadows_bind_group_layout: wgpu::BindGroupLayout,
    backdrop_filters_bind_group_layout: wgpu::BindGroupLayout,
    backdrop_texture_bind_group_layout: wgpu::BindGroupLayout,
    underlines_bind_group_layout: wgpu::BindGroupLayout,
    sprites_bind_group_layout: wgpu::BindGroupLayout,
    mono_sprites_bind_group_layout: wgpu::BindGroupLayout,
    poly_sprites_bind_group_layout: wgpu::BindGroupLayout,
    surfaces_bind_group_layout: wgpu::BindGroupLayout,
    paths_bind_group_layout: wgpu::BindGroupLayout,

    globals_bind_group: wgpu::BindGroup,
    color_adjustments_bind_group: wgpu::BindGroup,

    quads_pipeline: wgpu::RenderPipeline,
    shadows_pipeline: wgpu::RenderPipeline,
    backdrop_filters_pipeline: wgpu::RenderPipeline,
    underlines_pipeline: wgpu::RenderPipeline,
    mono_sprites_pipeline: wgpu::RenderPipeline,
    poly_sprites_pipeline: wgpu::RenderPipeline,
    surfaces_pipeline: wgpu::RenderPipeline,
    paths_pipeline: wgpu::RenderPipeline,
}

impl WgpuPipelines {
    pub fn new(
        context: &WgpuContext,
        surface_configuration: &wgpu::SurfaceConfiguration,
        _path_sample_count: u32,
    ) -> Self {
        let quads_shader = context
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("quads_shader"),
                source: wgpu::ShaderSource::Wgsl(include_str!("shaders/quads.wgsl").into()),
            });

        let shadows_shader = context
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("shadows_shader"),
                source: wgpu::ShaderSource::Wgsl(include_str!("shaders/shadows.wgsl").into()),
            });

        let backdrop_filter_shader =
            context
                .device
                .create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("backdrop_filter_shader"),
                    source: wgpu::ShaderSource::Wgsl(
                        include_str!("shaders/backdrop_blur.wgsl").into(),
                    ),
                });

        let underlines_shader = context
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("underlines_shader"),
                source: wgpu::ShaderSource::Wgsl(include_str!("shaders/underlines.wgsl").into()),
            });

        let mono_sprite_shader =
            context
                .device
                .create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("mono_sprites shader"),
                    source: wgpu::ShaderSource::Wgsl(
                        include_str!("shaders/mono_sprites.wgsl").into(),
                    ),
                });

        let poly_sprite_shader =
            context
                .device
                .create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("poly_sprites shader"),
                    source: wgpu::ShaderSource::Wgsl(
                        include_str!("shaders/poly_sprites.wgsl").into(),
                    ),
                });

        let blend_mode = match surface_configuration.alpha_mode {
            wgpu::CompositeAlphaMode::PreMultiplied => {
                wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING
            }
            _ => wgpu::BlendState::ALPHA_BLENDING,
        };

        let color_targets = &[Some(wgpu::ColorTargetState {
            format: surface_configuration.format,
            blend: Some(blend_mode),
            write_mask: wgpu::ColorWrites::ALL,
        })];

        let globals_bind_group_layout =
            context
                .device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("globals"),
                    entries: &[wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    }],
                });

        let color_adjustments_bind_group_layout =
            context
                .device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("color_adjustments_bind_group_layout"),
                    entries: &[wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    }],
                });

        let sprites_bind_group_layout =
            context
                .device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("sprite_bind_group_layout"),
                    entries: &[
                        wgpu::BindGroupLayoutEntry {
                            binding: 0,
                            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                            ty: wgpu::BindingType::Texture {
                                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                                view_dimension: wgpu::TextureViewDimension::D2,
                                multisampled: false,
                            },
                            count: None,
                        },
                        wgpu::BindGroupLayoutEntry {
                            binding: 1,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                            count: None,
                        },
                    ],
                });

        let quads_bind_group_layout =
            context
                .device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("quads_bind_group_layout"),
                    entries: &[wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    }],
                });

        let quads_pipeline_layout =
            context
                .device
                .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("quads_pipeline_layout"),
                    bind_group_layouts: &[
                        Some(&globals_bind_group_layout),
                        Some(&quads_bind_group_layout),
                    ],
                    immediate_size: 0,
                });

        let shadows_bind_group_layout =
            context
                .device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("shadows_bind_group_layout"),
                    entries: &[wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    }],
                });

        let shadows_pipeline_layout =
            context
                .device
                .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("shadows_pipeline_layout"),
                    bind_group_layouts: &[
                        Some(&globals_bind_group_layout),
                        Some(&shadows_bind_group_layout),
                    ],
                    immediate_size: 0,
                });

        let backdrop_filters_bind_group_layout =
            context
                .device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("backdrop_filters_bind_group_layout"),
                    entries: &[wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    }],
                });

        let backdrop_texture_bind_group_layout =
            context
                .device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("backdrop_texture_bind_group_layout"),
                    entries: &[
                        wgpu::BindGroupLayoutEntry {
                            binding: 0,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Texture {
                                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                                view_dimension: wgpu::TextureViewDimension::D2,
                                multisampled: false,
                            },
                            count: None,
                        },
                        wgpu::BindGroupLayoutEntry {
                            binding: 1,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                            count: None,
                        },
                    ],
                });

        let backdrop_filters_pipeline_layout =
            context
                .device
                .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("backdrop_filters_pipeline_layout"),
                    bind_group_layouts: &[
                        Some(&globals_bind_group_layout),
                        Some(&backdrop_filters_bind_group_layout),
                        Some(&backdrop_texture_bind_group_layout),
                    ],
                    immediate_size: 0,
                });

        let underlines_bind_group_layout =
            context
                .device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("underlines_bind_group_layout"),
                    entries: &[wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    }],
                });

        let underlines_pipeline_layout =
            context
                .device
                .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("underlines_pipeline_layout"),
                    bind_group_layouts: &[
                        Some(&globals_bind_group_layout),
                        Some(&underlines_bind_group_layout),
                    ],
                    immediate_size: 0,
                });

        let mono_sprites_bind_group_layout =
            context
                .device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("Mono sprites bind group layout"),
                    entries: &[wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    }],
                });

        let mono_sprites_pipeline_layout =
            context
                .device
                .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("Mono sprites pipeline layout"),
                    bind_group_layouts: &[
                        Some(&globals_bind_group_layout),
                        Some(&color_adjustments_bind_group_layout),
                        Some(&sprites_bind_group_layout),
                        Some(&mono_sprites_bind_group_layout),
                    ],
                    immediate_size: 0,
                });

        let poly_sprites_bind_group_layout =
            context
                .device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("Poly sprites bind group layout"),
                    entries: &[wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    }],
                });

        let poly_sprites_pipeline_layout =
            context
                .device
                .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("Poly sprites pipeline layout"),
                    bind_group_layouts: &[
                        Some(&globals_bind_group_layout),
                        Some(&sprites_bind_group_layout),
                        Some(&poly_sprites_bind_group_layout),
                    ],
                    immediate_size: 0,
                });

        let surfaces_shader = context
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("surfaces_shader"),
                source: wgpu::ShaderSource::Wgsl(include_str!("shaders/surfaces.wgsl").into()),
            });

        let surfaces_bind_group_layout =
            context
                .device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("surfaces_bind_group_layout"),
                    entries: &[
                        wgpu::BindGroupLayoutEntry {
                            binding: 0,
                            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                            ty: wgpu::BindingType::Buffer {
                                ty: wgpu::BufferBindingType::Uniform,
                                has_dynamic_offset: false,
                                min_binding_size: None,
                            },
                            count: None,
                        },
                        wgpu::BindGroupLayoutEntry {
                            binding: 1,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Texture {
                                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                                view_dimension: wgpu::TextureViewDimension::D2,
                                multisampled: false,
                            },
                            count: None,
                        },
                        wgpu::BindGroupLayoutEntry {
                            binding: 2,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                            count: None,
                        },
                    ],
                });

        let surfaces_pipeline_layout =
            context
                .device
                .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("surfaces_pipeline_layout"),
                    bind_group_layouts: &[
                        Some(&globals_bind_group_layout),
                        Some(&surfaces_bind_group_layout),
                    ],
                    immediate_size: 0,
                });

        let globals_bind_group = context
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("globals_bind_group"),
                layout: &globals_bind_group_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &context.globals_buffer,
                        offset: 0,
                        size: None,
                    }),
                }],
            });

        let color_adjustments_bind_group =
            context
                .device
                .create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("color_adjustments_bind_group"),
                    layout: &color_adjustments_bind_group_layout,
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: &context.color_adjustments_buffer,
                            offset: 0,
                            size: None,
                        }),
                    }],
                });

        // ---- Paths pipeline ------------------------------------------------
        let paths_shader = context
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("paths_shader"),
                source: wgpu::ShaderSource::Wgsl(include_str!("shaders/paths.wgsl").into()),
            });

        let paths_bind_group_layout =
            context
                .device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("paths_bind_group_layout"),
                    entries: &[wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    }],
                });

        let paths_pipeline_layout =
            context
                .device
                .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("paths_pipeline_layout"),
                    bind_group_layouts: &[
                        Some(&globals_bind_group_layout),
                        Some(&paths_bind_group_layout),
                    ],
                    immediate_size: 0,
                });
        // --------------------------------------------------------------------

        Self {
            color_targets: color_targets.to_vec(),

            quads_bind_group_layout,
            shadows_bind_group_layout,
            backdrop_filters_bind_group_layout,
            backdrop_texture_bind_group_layout,
            underlines_bind_group_layout,
            mono_sprites_bind_group_layout,
            sprites_bind_group_layout,
            poly_sprites_bind_group_layout,
            paths_bind_group_layout,

            globals_bind_group,
            color_adjustments_bind_group,

            quads_pipeline: context.device.create_render_pipeline(
                &wgpu::RenderPipelineDescriptor {
                    label: Some("quads"),
                    layout: Some(&quads_pipeline_layout),
                    vertex: wgpu::VertexState {
                        module: &quads_shader,
                        entry_point: Some("vs_quad"),
                        compilation_options: wgpu::PipelineCompilationOptions::default(),
                        buffers: &[],
                    },
                    primitive: wgpu::PrimitiveState {
                        topology: wgpu::PrimitiveTopology::TriangleStrip,
                        ..Default::default()
                    },
                    depth_stencil: None,
                    multisample: wgpu::MultisampleState::default(),
                    fragment: Some(wgpu::FragmentState {
                        module: &quads_shader,
                        entry_point: Some("fs_quad"),
                        compilation_options: wgpu::PipelineCompilationOptions::default(),
                        targets: color_targets,
                    }),
                    multiview_mask: None,
                    cache: None,
                },
            ),

            shadows_pipeline: context.device.create_render_pipeline(
                &wgpu::RenderPipelineDescriptor {
                    label: Some("shadows"),
                    layout: Some(&shadows_pipeline_layout),
                    vertex: wgpu::VertexState {
                        module: &shadows_shader,
                        entry_point: Some("vs_shadow"),
                        compilation_options: wgpu::PipelineCompilationOptions::default(),
                        buffers: &[],
                    },
                    primitive: wgpu::PrimitiveState {
                        topology: wgpu::PrimitiveTopology::TriangleStrip,
                        ..Default::default()
                    },
                    depth_stencil: None,
                    multisample: wgpu::MultisampleState::default(),
                    fragment: Some(wgpu::FragmentState {
                        module: &shadows_shader,
                        entry_point: Some("fs_shadow"),
                        compilation_options: wgpu::PipelineCompilationOptions::default(),
                        targets: color_targets,
                    }),
                    multiview_mask: None,
                    cache: None,
                },
            ),

            backdrop_filters_pipeline: context.device.create_render_pipeline(
                &wgpu::RenderPipelineDescriptor {
                    label: Some("backdrop_filters"),
                    layout: Some(&backdrop_filters_pipeline_layout),
                    vertex: wgpu::VertexState {
                        module: &backdrop_filter_shader,
                        entry_point: Some("vs_backdrop_filter"),
                        compilation_options: wgpu::PipelineCompilationOptions::default(),
                        buffers: &[],
                    },
                    primitive: wgpu::PrimitiveState {
                        topology: wgpu::PrimitiveTopology::TriangleStrip,
                        ..Default::default()
                    },
                    depth_stencil: None,
                    multisample: wgpu::MultisampleState::default(),
                    fragment: Some(wgpu::FragmentState {
                        module: &backdrop_filter_shader,
                        entry_point: Some("fs_backdrop_filter"),
                        compilation_options: wgpu::PipelineCompilationOptions::default(),
                        targets: color_targets,
                    }),
                    multiview_mask: None,
                    cache: None,
                },
            ),

            underlines_pipeline: context.device.create_render_pipeline(
                &wgpu::RenderPipelineDescriptor {
                    label: Some("underlines"),
                    layout: Some(&underlines_pipeline_layout),
                    vertex: wgpu::VertexState {
                        module: &underlines_shader,
                        entry_point: Some("vs_underline"),
                        compilation_options: wgpu::PipelineCompilationOptions::default(),
                        buffers: &[],
                    },
                    primitive: wgpu::PrimitiveState {
                        topology: wgpu::PrimitiveTopology::TriangleStrip,
                        ..Default::default()
                    },
                    depth_stencil: None,
                    multisample: wgpu::MultisampleState::default(),
                    fragment: Some(wgpu::FragmentState {
                        module: &underlines_shader,
                        entry_point: Some("fs_underline"),
                        compilation_options: wgpu::PipelineCompilationOptions::default(),
                        targets: color_targets,
                    }),
                    multiview_mask: None,
                    cache: None,
                },
            ),

            mono_sprites_pipeline: context.device.create_render_pipeline(
                &wgpu::RenderPipelineDescriptor {
                    label: Some("mono_sprites"),
                    layout: Some(&mono_sprites_pipeline_layout),
                    vertex: wgpu::VertexState {
                        module: &mono_sprite_shader,
                        entry_point: Some("vs_mono_sprite"),
                        compilation_options: wgpu::PipelineCompilationOptions::default(),
                        buffers: &[],
                    },
                    primitive: wgpu::PrimitiveState {
                        topology: wgpu::PrimitiveTopology::TriangleStrip,
                        ..Default::default()
                    },
                    depth_stencil: None,
                    fragment: Some(wgpu::FragmentState {
                        module: &mono_sprite_shader,
                        entry_point: Some("fs_mono_sprite"),
                        compilation_options: wgpu::PipelineCompilationOptions::default(),
                        targets: color_targets,
                    }),
                    multisample: wgpu::MultisampleState::default(),
                    multiview_mask: None,
                    cache: None,
                },
            ),

            poly_sprites_pipeline: context.device.create_render_pipeline(
                &wgpu::RenderPipelineDescriptor {
                    label: Some("poly_sprites"),
                    layout: Some(&poly_sprites_pipeline_layout),
                    vertex: wgpu::VertexState {
                        module: &poly_sprite_shader,
                        entry_point: Some("vs_poly_sprite"),
                        compilation_options: wgpu::PipelineCompilationOptions::default(),
                        buffers: &[],
                    },
                    primitive: wgpu::PrimitiveState {
                        topology: wgpu::PrimitiveTopology::TriangleStrip,
                        ..Default::default()
                    },
                    depth_stencil: None,
                    fragment: Some(wgpu::FragmentState {
                        module: &poly_sprite_shader,
                        entry_point: Some("fs_poly_sprite"),
                        compilation_options: wgpu::PipelineCompilationOptions::default(),
                        targets: color_targets,
                    }),
                    multisample: wgpu::MultisampleState::default(),
                    multiview_mask: None,
                    cache: None,
                },
            ),

            surfaces_bind_group_layout,

            surfaces_pipeline: context.device.create_render_pipeline(
                &wgpu::RenderPipelineDescriptor {
                    label: Some("surfaces"),
                    layout: Some(&surfaces_pipeline_layout),
                    vertex: wgpu::VertexState {
                        module: &surfaces_shader,
                        entry_point: Some("vs_surface"),
                        compilation_options: wgpu::PipelineCompilationOptions::default(),
                        buffers: &[],
                    },
                    primitive: wgpu::PrimitiveState {
                        topology: wgpu::PrimitiveTopology::TriangleStrip,
                        ..Default::default()
                    },
                    depth_stencil: None,
                    fragment: Some(wgpu::FragmentState {
                        module: &surfaces_shader,
                        entry_point: Some("fs_surface"),
                        compilation_options: wgpu::PipelineCompilationOptions::default(),
                        targets: color_targets,
                    }),
                    multisample: wgpu::MultisampleState::default(),
                    multiview_mask: None,
                    cache: None,
                },
            ),

            paths_pipeline: context.device.create_render_pipeline(
                &wgpu::RenderPipelineDescriptor {
                    label: Some("paths"),
                    layout: Some(&paths_pipeline_layout),
                    vertex: wgpu::VertexState {
                        module: &paths_shader,
                        entry_point: Some("vs_path"),
                        compilation_options: wgpu::PipelineCompilationOptions::default(),
                        buffers: &[],
                    },
                    primitive: wgpu::PrimitiveState {
                        topology: wgpu::PrimitiveTopology::TriangleList,
                        ..Default::default()
                    },
                    depth_stencil: None,
                    multisample: wgpu::MultisampleState::default(),
                    fragment: Some(wgpu::FragmentState {
                        module: &paths_shader,
                        entry_point: Some("fs_path"),
                        compilation_options: wgpu::PipelineCompilationOptions::default(),
                        targets: color_targets,
                    }),
                    multiview_mask: None,
                    cache: None,
                },
            ),
        }
    }
}

struct RenderingParameters {
    path_sample_count: u32,
    gamma_ratios: [f32; 4],
    grayscale_enhanced_contrast: f32,
}

impl RenderingParameters {
    fn from_env() -> Self {
        use std::env;

        let path_sample_count = env::var("ZED_PATH_SAMPLE_COUNT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(4);
        let gamma = env::var("ZED_FONTS_GAMMA")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1.8_f32)
            .clamp(1.0, 2.2);
        let gamma_ratios = crate::platform::get_gamma_correction_ratios(gamma);
        let grayscale_enhanced_contrast = env::var("ZED_FONTS_GRAYSCALE_ENHANCED_CONTRAST")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1.0_f32)
            .max(0.0);

        Self {
            path_sample_count,
            gamma_ratios,
            grayscale_enhanced_contrast,
        }
    }
}

/// Cached bounds information for fast surface blitting
#[derive(Clone, Debug)]
struct SurfaceBoundsEntry {
    /// Screen-space bounds where the surface should be rendered
    screen_bounds: geometry::Bounds<Pixels>,
    /// Content mask for clipping
    content_mask: geometry::Bounds<Pixels>,
    /// Layout version when these bounds were computed (for staleness detection)
    layout_version: u64,
}

/// Maximum nesting depth supported for CSS-style content `filter` groups
/// (`with_filter_layer`). Groups nested deeper than this are painted inline,
/// unisolated and unblurred, rather than allocating unbounded offscreen textures.
const MAX_FILTER_DEPTH: usize = 4;

/// Allocates the pool of full-surface-sized offscreen textures that
/// content-filter groups render into, one per supported nesting depth.
fn create_filter_group_textures(
    device: &wgpu::Device,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
) -> (Vec<wgpu::Texture>, Vec<wgpu::TextureView>) {
    (0..MAX_FILTER_DEPTH)
        .map(|_| {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("filter_group_texture"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            (texture, view)
        })
        .unzip()
}

pub struct WgpuRenderer {
    context: Arc<WgpuContext>,
    surface: ManuallyDrop<wgpu::Surface<'static>>,
    surface_configuration: wgpu::SurfaceConfiguration,
    atlas_sampler: wgpu::Sampler,
    surface_sampler: wgpu::Sampler,
    atlas: Arc<WgpuAtlas>,
    pipelines: WgpuPipelines,
    rendering_parameters: RenderingParameters,

    // cache bind groups for each double-buffered surface (index 0/1)
    surface_bind_groups:
        Mutex<HashMap<crate::platform::cross::surface_registry::SurfaceId, [wgpu::BindGroup; 2]>>,

    // Persistent framebuffer for browser-canvas-style blitting
    persistent_framebuffer: Option<wgpu::Texture>,
    persistent_framebuffer_view: Option<wgpu::TextureView>,

    // Backdrop blur texture for capturing framebuffer content
    backdrop_blur_texture: Option<wgpu::Texture>,
    backdrop_blur_texture_view: Option<wgpu::TextureView>,
    backdrop_blur_sampler: wgpu::Sampler,

    // Pool of offscreen textures that content-filter groups (`with_filter_layer`)
    // render into so their content can be blurred and composited as a unit.
    // Indexed by nesting depth, sized to `MAX_FILTER_DEPTH`.
    group_textures: Vec<wgpu::Texture>,
    group_views: Vec<wgpu::TextureView>,

    // Bounds cache for fast surface blitting without compositor
    surface_bounds_cache: Arc<Mutex<HashMap<SurfaceId, SurfaceBoundsEntry>>>,

    // Layout version counter (incremented when compositor runs)
    layout_version: Arc<AtomicU64>,
}

impl WgpuRenderer {
    pub fn new<WindowHandle>(
        context: Arc<WgpuContext>,
        window: WindowHandle,
        atlas: Arc<WgpuAtlas>,
        width: u32,
        height: u32,
        path_sample_count: u32,
    ) -> anyhow::Result<Self>
    where
        WindowHandle: raw_window_handle::HasWindowHandle + raw_window_handle::HasDisplayHandle,
    {
        let surface = unsafe {
            context
                .instance
                .create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                    raw_display_handle: window.display_handle()?.as_raw(),
                    raw_window_handle: window.window_handle()?.as_raw(),
                })?
        };

        let surface_capabilities = surface.get_capabilities(&context.adapter);

        // NOTE(mdeand): The shaders (hsla_to_rgba) output sRGB values directly, so we need a
        // NOTE(mdeand): non-sRGB surface format to avoid a double linear-to-sRGB conversion.
        // NOTE(mdeand): Prefer a non-sRGB format; fall back to whatever is available.
        let format = surface_capabilities
            .formats
            .iter()
            .find(|f| !f.is_srgb())
            .copied()
            .unwrap_or(surface_capabilities.formats[0]);

        let alpha_mode = if surface_capabilities
            .alpha_modes
            .contains(&wgpu::CompositeAlphaMode::PreMultiplied)
        {
            wgpu::CompositeAlphaMode::PreMultiplied
        } else {
            surface_capabilities.alpha_modes[0]
        };

        // allow overriding vsync behaviour.  The default is `Fifo` (vsync
        // enabled) which is what `wgpu` considers the safest presentation mode.
        // Setting `GPUI_DISABLE_VSYNC=1` in the environment will switch to
        // `Immediate`, which drops frames at the display's full rate.  A more
        // fine‑grained control (`GPUI_PRESENT_MODE=mailbox|fifo|immediate`) is
        // also supported for experimentation.
        let present_mode = std::env::var("GPUI_PRESENT_MODE")
            .ok()
            .and_then(|s| match s.to_lowercase().as_str() {
                "mailbox" => Some(wgpu::PresentMode::Mailbox),
                "immediate" => Some(wgpu::PresentMode::Immediate),
                "fifo" => Some(wgpu::PresentMode::Fifo),
                _ => None,
            })
            .unwrap_or_else(|| {
                if std::env::var("GPUI_DISABLE_VSYNC").is_ok() {
                    wgpu::PresentMode::Immediate
                } else {
                    wgpu::PresentMode::Fifo
                }
            });

        let surface_configuration = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            format,
            width,
            height,
            present_mode,
            alpha_mode,
            view_formats: vec![],
            // TODO(mdeand): Make this configurable?
            desired_maximum_frame_latency: 2,
        };

        let atlas_sampler = context.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("atlas_sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let surface_sampler = context.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("surface_sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let backdrop_blur_sampler = context.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("backdrop_blur_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let pipelines =
            WgpuPipelines::new(context.as_ref(), &surface_configuration, path_sample_count);

        // Create persistent framebuffer for browser-canvas-style blitting
        let persistent_framebuffer = context.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("persistent_framebuffer"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });

        let persistent_framebuffer_view =
            persistent_framebuffer.create_view(&wgpu::TextureViewDescriptor::default());

        // Create backdrop blur texture for capturing framebuffer content
        let backdrop_blur_texture = context.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("backdrop_blur_texture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        let backdrop_blur_texture_view =
            backdrop_blur_texture.create_view(&wgpu::TextureViewDescriptor::default());

        let (group_textures, group_views) =
            create_filter_group_textures(&context.device, width, height, format);

        Ok(Self {
            context: context.clone(),
            surface: ManuallyDrop::new(surface),
            surface_configuration,
            atlas,
            atlas_sampler,
            surface_sampler,
            backdrop_blur_sampler,
            pipelines,
            rendering_parameters: RenderingParameters::from_env(),
            surface_bind_groups: Mutex::new(HashMap::new()),
            persistent_framebuffer: Some(persistent_framebuffer),
            persistent_framebuffer_view: Some(persistent_framebuffer_view),
            backdrop_blur_texture: Some(backdrop_blur_texture),
            backdrop_blur_texture_view: Some(backdrop_blur_texture_view),
            group_textures,
            group_views,
            surface_bounds_cache: Arc::new(Mutex::new(HashMap::new())),
            layout_version: Arc::new(AtomicU64::new(0)),
        })
    }

    pub fn draw(&mut self, scene: &Scene) {
        log::debug!("Renderer::draw: starting frame");

        let mut command_encoder =
            self.context
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("main"),
                });

        self.atlas.before_frame(&mut command_encoder);
        log::trace!("Renderer::draw: atlas.before_frame complete");

        // keep track of which surface ids we rendered this frame
        let mut seen_surfaces: Vec<crate::platform::cross::surface_registry::SurfaceId> =
            Vec::new();

        // CRITICAL: Keep surface views alive until after the render pass ends
        // The bind groups reference these views, so they must not be dropped early
        let mut surface_views: Vec<wgpu::TextureView> = Vec::new();
        // Surface bind groups also reference per-surface params buffers.
        let mut surface_param_buffers: Vec<wgpu::Buffer> = Vec::new();

        let color_adjustments = ColorAdjustments {
            gamma_ratios: self.rendering_parameters.gamma_ratios,
            grayscale_enhanced_contrast: self.rendering_parameters.grayscale_enhanced_contrast,
            _padding: [0.0; 3],
        };
        self.context.queue.write_buffer(
            &self.context.color_adjustments_buffer,
            0,
            bytemuck::bytes_of(&color_adjustments),
        );

        let globals = GlobalParams {
            viewport_size: [
                self.surface_configuration.width as f32,
                self.surface_configuration.height as f32,
            ],
            premultimated_alpha: match self.surface_configuration.alpha_mode {
                wgpu::CompositeAlphaMode::PreMultiplied => 1,
                _ => 0,
            },
            pad: 0,
        };

        self.context.queue.write_buffer(
            &self.context.globals_buffer,
            0,
            bytemuck::bytes_of(&globals),
        );

        unsafe fn as_bytes<T>(slice: &[T]) -> &[u8] {
            unsafe {
                std::slice::from_raw_parts(
                    slice.as_ptr() as *const u8,
                    slice.len() * std::mem::size_of::<T>(),
                )
            }
        }

        if !scene.quads.is_empty() {
            let data = unsafe { as_bytes(&scene.quads) };
            ensure_buffer_size(
                &self.context.device,
                &self.context.quads_buffer,
                data.len() as u64,
                "Quads Buffer",
                wgpu::BufferUsages::VERTEX
                    | wgpu::BufferUsages::COPY_DST
                    | wgpu::BufferUsages::STORAGE,
            );
            self.context
                .queue
                .write_buffer(&self.context.quads_buffer.lock().unwrap(), 0, data);
        }
        if !scene.shadows.is_empty() {
            let data = unsafe { as_bytes(&scene.shadows) };
            ensure_buffer_size(
                &self.context.device,
                &self.context.shadows_buffer,
                data.len() as u64,
                "Shadows Buffer",
                wgpu::BufferUsages::VERTEX
                    | wgpu::BufferUsages::COPY_DST
                    | wgpu::BufferUsages::STORAGE,
            );
            self.context
                .queue
                .write_buffer(&self.context.shadows_buffer.lock().unwrap(), 0, data);
        }
        if !scene.backdrop_filters.is_empty() {
            let data = unsafe { as_bytes(&scene.backdrop_filters) };
            ensure_buffer_size(
                &self.context.device,
                &self.context.backdrop_filters_buffer,
                data.len() as u64,
                "Backdrop Filters Buffer",
                wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            );
            self.context.queue.write_buffer(
                &self.context.backdrop_filters_buffer.lock().unwrap(),
                0,
                data,
            );
        }
        if !scene.underlines.is_empty() {
            let data = unsafe { as_bytes(&scene.underlines) };
            ensure_buffer_size(
                &self.context.device,
                &self.context.underlines_buffer,
                data.len() as u64,
                "Underlines Buffer",
                wgpu::BufferUsages::VERTEX
                    | wgpu::BufferUsages::COPY_DST
                    | wgpu::BufferUsages::STORAGE,
            );
            self.context.queue.write_buffer(
                &self.context.underlines_buffer.lock().unwrap(),
                0,
                data,
            );
        }
        if !scene.monochrome_sprites.is_empty() {
            let data = unsafe { as_bytes(&scene.monochrome_sprites) };
            ensure_buffer_size(
                &self.context.device,
                &self.context.mono_sprites_buffer,
                data.len() as u64,
                "Monosprites Buffer",
                wgpu::BufferUsages::VERTEX
                    | wgpu::BufferUsages::COPY_DST
                    | wgpu::BufferUsages::STORAGE,
            );
            self.context.queue.write_buffer(
                &self.context.mono_sprites_buffer.lock().unwrap(),
                0,
                data,
            );
        }
        if !scene.polychrome_sprites.is_empty() {
            let data = unsafe { as_bytes(&scene.polychrome_sprites) };
            ensure_buffer_size(
                &self.context.device,
                &self.context.poly_sprites_buffer,
                data.len() as u64,
                "Poly Sprites Buffer",
                wgpu::BufferUsages::VERTEX
                    | wgpu::BufferUsages::COPY_DST
                    | wgpu::BufferUsages::STORAGE,
            );
            self.context.queue.write_buffer(
                &self.context.poly_sprites_buffer.lock().unwrap(),
                0,
                data,
            );
        }

        // Build flat vertex array for all paths (color + content mask baked per-vertex)
        let mut flat_path_vertices: Vec<GpuPathVertex> = Vec::new();
        for path in &scene.paths {
            let color = path.color.solid;
            let cm = &path.content_mask.bounds;
            let cm_origin = [cm.origin.x.0, cm.origin.y.0];
            let cm_size = [cm.size.width.0, cm.size.height.0];
            for vertex in &path.vertices {
                flat_path_vertices.push(GpuPathVertex {
                    xy_position: [vertex.xy_position.x.0, vertex.xy_position.y.0],
                    st_position: [vertex.st_position.x, vertex.st_position.y],
                    hsla: [color.h, color.s, color.l, color.a],
                    content_mask_origin: cm_origin,
                    content_mask_size: cm_size,
                });
            }
        }
        if !flat_path_vertices.is_empty() {
            let data = bytemuck::cast_slice(&flat_path_vertices);
            ensure_buffer_size(
                &self.context.device,
                &self.context.paths_vertices_buffer,
                data.len() as u64,
                "Path Vertices Buffer",
                wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            );
            self.context.queue.write_buffer(
                &self.context.paths_vertices_buffer.lock().unwrap(),
                0,
                data,
            );
        }

        // Acquire the next swapchain image.  On the first frame after window
        // creation (or after a resize races with the GPU) the surface can be
        // reported as `Outdated` or `Other`.  Rather than panicking we
        // reconfigure and retry once; if the second attempt also fails we
        // simply drop this frame.
        let surface_texture = {
            let first = self.surface.get_current_texture();
            match first {
                Ok(t) => t,
                Err(wgpu::SurfaceError::Outdated)
                | Err(wgpu::SurfaceError::Lost)
                | Err(wgpu::SurfaceError::Other) => {
                    // Reconfigure with the current known size and retry.
                    self.surface
                        .configure(&self.context.device, &self.surface_configuration);
                    match self.surface.get_current_texture() {
                        Ok(t) => t,
                        Err(e) => {
                            log::warn!(
                                "Skipping frame: failed to acquire swap chain texture after reconfigure: {:?}",
                                e
                            );
                            return;
                        }
                    }
                }
                Err(wgpu::SurfaceError::Timeout) => {
                    log::warn!("Skipping frame: swap chain acquire timed out");
                    return;
                }
                Err(wgpu::SurfaceError::OutOfMemory) => {
                    log::warn!("Skipping frame: out of memory");
                    return;
                }
            }
        };

        // Increment layout version - all bounds caches are now fresh
        // IMPORTANT: Only increment after successful swapchain acquisition
        // If we skip the frame, bounds remain valid
        self.layout_version.fetch_add(1, Ordering::Release);

        // Borrow buffers for bind group creation - these borrows must live until bind groups are done
        let quads_buffer_ref = self.context.quads_buffer.lock().unwrap();
        let shadows_buffer_ref = self.context.shadows_buffer.lock().unwrap();
        let backdrop_filters_buffer_ref = self.context.backdrop_filters_buffer.lock().unwrap();
        let underlines_buffer_ref = self.context.underlines_buffer.lock().unwrap();
        let mono_sprites_buffer_ref = self.context.mono_sprites_buffer.lock().unwrap();
        let poly_sprites_buffer_ref = self.context.poly_sprites_buffer.lock().unwrap();
        let paths_vertices_buffer_ref = self.context.paths_vertices_buffer.lock().unwrap();

        let quads_bind_group = self
            .context
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("quads_bind_group"),
                layout: &self.pipelines.quads_bind_group_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &quads_buffer_ref,
                        offset: 0,
                        size: None,
                    }),
                }],
            });

        let shadows_bind_group =
            self.context
                .device
                .create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("shadows_bind_group"),
                    layout: &self.pipelines.shadows_bind_group_layout,
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: &shadows_buffer_ref,
                            offset: 0,
                            size: None,
                        }),
                    }],
                });

        let backdrop_filters_bind_group =
            self.context
                .device
                .create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("backdrop_filters_bind_group"),
                    layout: &self.pipelines.backdrop_filters_bind_group_layout,
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: &backdrop_filters_buffer_ref,
                            offset: 0,
                            size: None,
                        }),
                    }],
                });

        let backdrop_texture_bind_group =
            self.context
                .device
                .create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("backdrop_texture_bind_group"),
                    layout: &self.pipelines.backdrop_texture_bind_group_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(
                                self.backdrop_blur_texture_view.as_ref().unwrap(),
                            ),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::Sampler(&self.backdrop_blur_sampler),
                        },
                    ],
                });

        let underlines_bind_group =
            self.context
                .device
                .create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("underlines_bind_group"),
                    layout: &self.pipelines.underlines_bind_group_layout,
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: &underlines_buffer_ref,
                            offset: 0,
                            size: None,
                        }),
                    }],
                });

        let mono_sprites_bind_group =
            self.context
                .device
                .create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("mono_sprites_bind_group"),
                    layout: &self.pipelines.mono_sprites_bind_group_layout,
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: &mono_sprites_buffer_ref,
                            offset: 0,
                            size: None,
                        }),
                    }],
                });

        let poly_sprites_bind_group =
            self.context
                .device
                .create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("poly_sprites_bind_group"),
                    layout: &self.pipelines.poly_sprites_bind_group_layout,
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: &poly_sprites_buffer_ref,
                            offset: 0,
                            size: None,
                        }),
                    }],
                });

        let paths_bind_group = self
            .context
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("paths_bind_group"),
                layout: &self.pipelines.paths_bind_group_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &paths_vertices_buffer_ref,
                        offset: 0,
                        size: None,
                    }),
                }],
            });

        {
            // Render to swapchain directly for now (TODO: render to framebuffer, then blit)
            let mut pass = command_encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("main"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &surface_texture
                        .texture
                        .create_view(&wgpu::TextureViewDescriptor::default()),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                    resolve_target: None,
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });

            let mut quads_first_instance: u32 = 0;
            let mut shadows_first_instance: u32 = 0;
            let mut backdrop_filters_first_instance: u32 = 0;
            let mut underlines_first_instance: u32 = 0;
            let mut mono_sprites_first_instance: u32 = 0;
            let mut poly_sprites_first_instance: u32 = 0;
            let mut paths_vertex_offset: u32 = 0;

            // Stack of active content-filter groups. Each entry pairs the group's
            // `FilterBoundary` start marker with the `group_textures`/`group_views`
            // slot its content is being rendered into (`None` if the group exceeded
            // `MAX_FILTER_DEPTH` and is being painted inline, unisolated).
            let mut filter_stack: Vec<(FilterBoundary, Option<usize>)> = Vec::new();

            for batch in scene.batches() {
                match batch {
                    PrimitiveBatch::Quads(quads) => {
                        let count = quads.len() as u32;
                        pass.set_pipeline(&self.pipelines.quads_pipeline);
                        pass.set_bind_group(0, &self.pipelines.globals_bind_group, &[]);
                        pass.set_bind_group(1, &quads_bind_group, &[]);
                        pass.draw(0..4, quads_first_instance..quads_first_instance + count);
                        quads_first_instance += count;
                    }

                    PrimitiveBatch::MonochromeSprites {
                        texture_id,
                        sprites,
                    } => {
                        let count = sprites.len() as u32;
                        let tex_info = self.atlas.get_texture_info(texture_id);

                        let sprites_texture_bind_group =
                            self.context
                                .device
                                .create_bind_group(&wgpu::BindGroupDescriptor {
                                    label: Some("sprites_bind_group"),
                                    layout: &self.pipelines.sprites_bind_group_layout,
                                    entries: &[
                                        wgpu::BindGroupEntry {
                                            binding: 0,
                                            resource: wgpu::BindingResource::TextureView(
                                                &tex_info.raw_view,
                                            ),
                                        },
                                        wgpu::BindGroupEntry {
                                            binding: 1,
                                            resource: wgpu::BindingResource::Sampler(
                                                &self.atlas_sampler,
                                            ),
                                        },
                                    ],
                                });

                        pass.set_pipeline(&self.pipelines.mono_sprites_pipeline);
                        pass.set_bind_group(0, &self.pipelines.globals_bind_group, &[]);
                        pass.set_bind_group(1, &self.pipelines.color_adjustments_bind_group, &[]);
                        pass.set_bind_group(2, &sprites_texture_bind_group, &[]);
                        pass.set_bind_group(3, &mono_sprites_bind_group, &[]);
                        pass.draw(
                            0..4,
                            mono_sprites_first_instance..mono_sprites_first_instance + count,
                        );
                        mono_sprites_first_instance += count;
                    }
                    PrimitiveBatch::PolychromeSprites {
                        texture_id,
                        sprites,
                    } => {
                        let count = sprites.len() as u32;
                        let tex_info = self.atlas.get_texture_info(texture_id);

                        let sprites_texture_bind_group =
                            self.context
                                .device
                                .create_bind_group(&wgpu::BindGroupDescriptor {
                                    label: Some("poly_sprites_texture_bind_group"),
                                    layout: &self.pipelines.sprites_bind_group_layout,
                                    entries: &[
                                        wgpu::BindGroupEntry {
                                            binding: 0,
                                            resource: wgpu::BindingResource::TextureView(
                                                &tex_info.raw_view,
                                            ),
                                        },
                                        wgpu::BindGroupEntry {
                                            binding: 1,
                                            resource: wgpu::BindingResource::Sampler(
                                                &self.atlas_sampler,
                                            ),
                                        },
                                    ],
                                });

                        pass.set_pipeline(&self.pipelines.poly_sprites_pipeline);
                        pass.set_bind_group(0, &self.pipelines.globals_bind_group, &[]);
                        pass.set_bind_group(1, &sprites_texture_bind_group, &[]);
                        pass.set_bind_group(2, &poly_sprites_bind_group, &[]);
                        pass.draw(
                            0..4,
                            poly_sprites_first_instance..poly_sprites_first_instance + count,
                        );
                        poly_sprites_first_instance += count;
                    }
                    PrimitiveBatch::Shadows(shadows) => {
                        let count = shadows.len() as u32;
                        pass.set_pipeline(&self.pipelines.shadows_pipeline);
                        pass.set_bind_group(0, &self.pipelines.globals_bind_group, &[]);
                        pass.set_bind_group(1, &shadows_bind_group, &[]);
                        pass.draw(0..4, shadows_first_instance..shadows_first_instance + count);
                        shadows_first_instance += count;
                    }
                    PrimitiveBatch::BackdropFilters(backdrop_filters) => {
                        let count = backdrop_filters.len() as u32;

                        // End the current render pass to copy texture
                        drop(pass);

                        // Copy surface texture to backdrop_blur_texture for sampling
                        if let Some(ref blur_texture) = self.backdrop_blur_texture {
                            // Use actual surface texture size (may differ from configured size)
                            let surface_size = surface_texture.texture.size();

                            // Only copy if sizes match (otherwise skip to avoid validation error)
                            if surface_size.width == blur_texture.width()
                                && surface_size.height == blur_texture.height()
                            {
                                command_encoder.copy_texture_to_texture(
                                    surface_texture.texture.as_image_copy(),
                                    blur_texture.as_image_copy(),
                                    surface_size,
                                );
                            }
                        }

                        // Begin new render pass with Load to preserve existing content
                        pass = command_encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                            label: Some("main_resumed"),
                            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                view: &surface_texture
                                    .texture
                                    .create_view(&wgpu::TextureViewDescriptor::default()),
                                ops: wgpu::Operations {
                                    load: wgpu::LoadOp::Load,
                                    store: wgpu::StoreOp::Store,
                                },
                                resolve_target: None,
                                depth_slice: None,
                            })],
                            depth_stencil_attachment: None,
                            timestamp_writes: None,
                            occlusion_query_set: None,
                            multiview_mask: None,
                        });

                        // Now render the backdrop blur quads
                        pass.set_pipeline(&self.pipelines.backdrop_filters_pipeline);
                        pass.set_bind_group(0, &self.pipelines.globals_bind_group, &[]);
                        pass.set_bind_group(1, &backdrop_filters_bind_group, &[]);
                        pass.set_bind_group(2, &backdrop_texture_bind_group, &[]);
                        pass.draw(
                            0..4,
                            backdrop_filters_first_instance..backdrop_filters_first_instance + count,
                        );
                        backdrop_filters_first_instance += count;
                    }
                    PrimitiveBatch::FilterBoundary(index) => {
                        let boundary = scene.filter_boundaries[index];

                        if boundary.is_start {
                            let depth = filter_stack.len();
                            if depth >= self.group_textures.len() {
                                // Exceeded the supported nesting depth: paint the group's
                                // content inline (unisolated/unblurred) rather than dropping it.
                                filter_stack.push((boundary, None));
                            } else {
                                drop(pass);

                                pass = command_encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                                    label: Some("filter_group"),
                                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                        view: &self.group_views[depth],
                                        ops: wgpu::Operations {
                                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                                            store: wgpu::StoreOp::Store,
                                        },
                                        resolve_target: None,
                                        depth_slice: None,
                                    })],
                                    depth_stencil_attachment: None,
                                    timestamp_writes: None,
                                    occlusion_query_set: None,
                                    multiview_mask: None,
                                });

                                filter_stack.push((boundary, Some(depth)));
                            }
                        } else {
                            let Some((start_boundary, depth)) = filter_stack.pop() else {
                                continue;
                            };

                            let Some(depth) = depth else {
                                // The group was painted inline; nothing to composite.
                                continue;
                            };

                            // End the group's pass: its content is now baked into
                            // `group_textures[depth]`.
                            drop(pass);

                            let surface_view = surface_texture
                                .texture
                                .create_view(&wgpu::TextureViewDescriptor::default());
                            let parent_view: &wgpu::TextureView = match filter_stack.last() {
                                Some((_, Some(parent_depth))) => &self.group_views[*parent_depth],
                                _ => &surface_view,
                            };

                            pass = command_encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                                label: Some("filter_group_resumed"),
                                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                    view: parent_view,
                                    ops: wgpu::Operations {
                                        load: wgpu::LoadOp::Load,
                                        store: wgpu::StoreOp::Store,
                                    },
                                    resolve_target: None,
                                    depth_slice: None,
                                })],
                                depth_stencil_attachment: None,
                                timestamp_writes: None,
                                occlusion_query_set: None,
                                multiview_mask: None,
                            });

                            // Composite the blurred group content back over the parent using
                            // the same backdrop-filter pipeline, sampling from the group's
                            // offscreen texture instead of a surface snapshot.
                            let composite = BackdropFilter {
                                order: 0,
                                bounds: start_boundary.bounds,
                                content_mask: start_boundary.content_mask,
                                corner_radii: start_boundary.corner_radii,
                                blur_radius: start_boundary.blur_radius,
                                opacity: start_boundary.opacity,
                                _pad: 0,
                            };
                            let composite_buffer = self.context.device.create_buffer_init(
                                &wgpu::util::BufferInitDescriptor {
                                    label: Some("filter_group_composite_buffer"),
                                    contents: unsafe { as_bytes(std::slice::from_ref(&composite)) },
                                    usage: wgpu::BufferUsages::STORAGE,
                                },
                            );
                            let composite_bind_group = self.context.device.create_bind_group(
                                &wgpu::BindGroupDescriptor {
                                    label: Some("filter_group_composite_bind_group"),
                                    layout: &self.pipelines.backdrop_filters_bind_group_layout,
                                    entries: &[wgpu::BindGroupEntry {
                                        binding: 0,
                                        resource: wgpu::BindingResource::Buffer(
                                            wgpu::BufferBinding {
                                                buffer: &composite_buffer,
                                                offset: 0,
                                                size: None,
                                            },
                                        ),
                                    }],
                                },
                            );
                            let composite_texture_bind_group = self.context.device.create_bind_group(
                                &wgpu::BindGroupDescriptor {
                                    label: Some("filter_group_texture_bind_group"),
                                    layout: &self.pipelines.backdrop_texture_bind_group_layout,
                                    entries: &[
                                        wgpu::BindGroupEntry {
                                            binding: 0,
                                            resource: wgpu::BindingResource::TextureView(
                                                &self.group_views[depth],
                                            ),
                                        },
                                        wgpu::BindGroupEntry {
                                            binding: 1,
                                            resource: wgpu::BindingResource::Sampler(
                                                &self.backdrop_blur_sampler,
                                            ),
                                        },
                                    ],
                                },
                            );

                            pass.set_pipeline(&self.pipelines.backdrop_filters_pipeline);
                            pass.set_bind_group(0, &self.pipelines.globals_bind_group, &[]);
                            pass.set_bind_group(1, &composite_bind_group, &[]);
                            pass.set_bind_group(2, &composite_texture_bind_group, &[]);
                            pass.draw(0..4, 0..1);
                        }
                    }
                    PrimitiveBatch::Underlines(underlines) => {
                        let count = underlines.len() as u32;
                        pass.set_pipeline(&self.pipelines.underlines_pipeline);
                        pass.set_bind_group(0, &self.pipelines.globals_bind_group, &[]);
                        pass.set_bind_group(1, &underlines_bind_group, &[]);
                        pass.draw(
                            0..4,
                            underlines_first_instance..underlines_first_instance + count,
                        );
                        underlines_first_instance += count;
                    }
                    PrimitiveBatch::Surfaces(surfaces) => {
                        log::debug!("Renderer: processing {} surface(s)", surfaces.len());
                        for surface in surfaces {
                            if let crate::SurfaceContent::Wgpu(surface_id) = &surface.content {
                                // Atomically swap ready ↔ display buffers with GPU sync
                                let _swapped = self
                                    .context
                                    .surface_registry
                                    .swap_ready_display(&self.context.device, *surface_id);

                                if let Some(view) =
                                    self.context.surface_registry.front_view(*surface_id)
                                {
                                    let params = SurfaceParams {
                                        bounds: Bounds {
                                            origin: [
                                                surface.bounds.origin.x.0,
                                                surface.bounds.origin.y.0,
                                            ],
                                            size: [
                                                surface.bounds.size.width.0,
                                                surface.bounds.size.height.0,
                                            ],
                                        },
                                        content_mask: Bounds {
                                            origin: [
                                                surface.content_mask.bounds.origin.x.0,
                                                surface.content_mask.bounds.origin.y.0,
                                            ],
                                            size: [
                                                surface.content_mask.bounds.size.width.0,
                                                surface.content_mask.bounds.size.height.0,
                                            ],
                                        },
                                    };

                                    // Cache bounds for fast surface blitting
                                    // Surface bounds are in ScaledPixels (f32), store as Pixels for caching
                                    self.surface_bounds_cache.lock().unwrap().insert(
                                        *surface_id,
                                        SurfaceBoundsEntry {
                                            screen_bounds: geometry::Bounds {
                                                origin: geometry::Point {
                                                    x: Pixels(surface.bounds.origin.x.0),
                                                    y: Pixels(surface.bounds.origin.y.0),
                                                },
                                                size: geometry::Size {
                                                    width: Pixels(surface.bounds.size.width.0),
                                                    height: Pixels(surface.bounds.size.height.0),
                                                },
                                            },
                                            content_mask: geometry::Bounds {
                                                origin: geometry::Point {
                                                    x: Pixels(
                                                        surface.content_mask.bounds.origin.x.0,
                                                    ),
                                                    y: Pixels(
                                                        surface.content_mask.bounds.origin.y.0,
                                                    ),
                                                },
                                                size: geometry::Size {
                                                    width: Pixels(
                                                        surface.content_mask.bounds.size.width.0,
                                                    ),
                                                    height: Pixels(
                                                        surface.content_mask.bounds.size.height.0,
                                                    ),
                                                },
                                            },
                                            layout_version: self
                                                .layout_version
                                                .load(Ordering::Acquire),
                                        },
                                    );

                                    let params_buffer = self.context.device.create_buffer_init(
                                        &wgpu::util::BufferInitDescriptor {
                                            label: Some("surface_params_buffer"),
                                            contents: bytemuck::bytes_of(&params),
                                            usage: wgpu::BufferUsages::UNIFORM,
                                        },
                                    );

                                    let surface_bind_group = self.context.device.create_bind_group(
                                        &wgpu::BindGroupDescriptor {
                                            label: Some("surface_bind_group"),
                                            layout: &self.pipelines.surfaces_bind_group_layout,
                                            entries: &[
                                                wgpu::BindGroupEntry {
                                                    binding: 0,
                                                    resource: wgpu::BindingResource::Buffer(
                                                        wgpu::BufferBinding {
                                                            buffer: &params_buffer,
                                                            offset: 0,
                                                            size: None,
                                                        },
                                                    ),
                                                },
                                                wgpu::BindGroupEntry {
                                                    binding: 1,
                                                    resource: wgpu::BindingResource::TextureView(
                                                        &view,
                                                    ),
                                                },
                                                wgpu::BindGroupEntry {
                                                    binding: 2,
                                                    resource: wgpu::BindingResource::Sampler(
                                                        &self.surface_sampler,
                                                    ),
                                                },
                                            ],
                                        },
                                    );

                                    pass.set_pipeline(&self.pipelines.surfaces_pipeline);
                                    pass.set_bind_group(0, &self.pipelines.globals_bind_group, &[]);
                                    pass.set_bind_group(1, &surface_bind_group, &[]);
                                    pass.draw(0..4, 0..1);

                                    // CRITICAL: Keep view alive until after render pass ends
                                    // The bind_group holds a reference to it
                                    surface_views.push(view);
                                    surface_param_buffers.push(params_buffer);

                                    // Clear redraw pending AFTER we're done with the view
                                    // This prevents the external thread from triggering another compositor
                                    // pass while we're still using this view
                                    self.context
                                        .surface_registry
                                        .clear_redraw_pending(*surface_id);

                                    seen_surfaces.push(*surface_id);
                                }
                            }
                        }
                    }
                    // TODO(mdeand): Implement paths rendering.
                    PrimitiveBatch::Paths(paths) => {
                        let vertex_count: u32 = paths.iter().map(|p| p.vertices.len() as u32).sum();
                        if vertex_count > 0 {
                            pass.set_pipeline(&self.pipelines.paths_pipeline);
                            pass.set_bind_group(0, &self.pipelines.globals_bind_group, &[]);
                            pass.set_bind_group(1, &paths_bind_group, &[]);
                            pass.draw(
                                paths_vertex_offset..paths_vertex_offset + vertex_count,
                                0..1,
                            );
                            paths_vertex_offset += vertex_count;
                        }
                    }
                }
            }
        }

        // TODO: Blit persistent framebuffer to swapchain (needs proper pipeline)

        log::debug!("Renderer::draw: submitting command buffer");
        self.context.queue.submit(Some(command_encoder.finish()));
        log::debug!("Renderer::draw: presenting surface");
        surface_texture.present();
        log::debug!("Renderer::draw: frame complete");
    }

    /// Fast path: blit all visible surfaces in a single swapchain pass.
    /// Returns true if successful, false if compositor should run.
    pub fn blit_surfaces_direct(&self, pending_surfaces: &[SurfaceId]) -> bool {
        if pending_surfaces.is_empty() {
            return false;
        }

        let layout_version = self.layout_version.load(Ordering::Acquire);
        let mut visible_surfaces = {
            let cache = self.surface_bounds_cache.lock().unwrap();

            // Fast path is valid only when bounds are current.
            if cache
                .values()
                .any(|entry| entry.layout_version != layout_version)
            {
                return false;
            }

            // Every pending surface must be currently visible with cached bounds.
            if pending_surfaces
                .iter()
                .any(|surface_id| !cache.contains_key(surface_id))
            {
                return false;
            }

            cache
                .iter()
                .map(|(surface_id, entry)| (*surface_id, entry.screen_bounds, entry.content_mask))
                .collect::<Vec<_>>()
        };

        if visible_surfaces.is_empty() {
            return false;
        }

        // Keep deterministic ordering.
        visible_surfaces.sort_unstable_by_key(|(surface_id, _, _)| surface_id.0);

        // Flip ready -> display for surfaces that actually rendered new frames.
        for surface_id in pending_surfaces {
            let _ = self
                .context
                .surface_registry
                .swap_ready_display(&self.context.device, *surface_id);
        }

        // Acquire swapchain (handle retryable surface errors the same as regular draw).
        let surface_texture = match self.surface.get_current_texture() {
            Ok(t) => t,
            Err(wgpu::SurfaceError::Outdated)
            | Err(wgpu::SurfaceError::Lost)
            | Err(wgpu::SurfaceError::Other) => {
                self.surface
                    .configure(&self.context.device, &self.surface_configuration);
                match self.surface.get_current_texture() {
                    Ok(t) => t,
                    Err(e) => {
                        log::warn!(
                            "Fast blit failed to acquire swapchain after reconfigure: {:?}",
                            e
                        );
                        return false;
                    }
                }
            }
            Err(wgpu::SurfaceError::Timeout) => {
                log::warn!("Fast blit failed: swapchain acquire timed out");
                return false;
            }
            Err(wgpu::SurfaceError::OutOfMemory) => {
                log::warn!("Fast blit failed: out of memory");
                return false;
            }
        };

        let mut encoder =
            self.context
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("fast_surface_blit"),
                });
        let swapchain_view = surface_texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("fast_surface_blit_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &swapchain_view,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load, // Preserve existing swapchain content
                        store: wgpu::StoreOp::Store,
                    },
                    resolve_target: None,
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });

            pass.set_pipeline(&self.pipelines.surfaces_pipeline);
            pass.set_bind_group(0, &self.pipelines.globals_bind_group, &[]);

            // Keep views and params buffers alive until pass ends (bind groups reference them).
            let mut surface_views = Vec::new();
            let mut surface_param_buffers = Vec::new();

            for (surface_id, screen_bounds, content_mask) in &visible_surfaces {
                let Some(view) = self.context.surface_registry.front_view(*surface_id) else {
                    return false;
                };

                let params = SurfaceParams {
                    bounds: Bounds {
                        origin: [screen_bounds.origin.x.0, screen_bounds.origin.y.0],
                        size: [screen_bounds.size.width.0, screen_bounds.size.height.0],
                    },
                    content_mask: Bounds {
                        origin: [content_mask.origin.x.0, content_mask.origin.y.0],
                        size: [content_mask.size.width.0, content_mask.size.height.0],
                    },
                };

                let params_buffer =
                    self.context
                        .device
                        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some("fast_blit_surface_params_buffer"),
                            contents: bytemuck::bytes_of(&params),
                            usage: wgpu::BufferUsages::UNIFORM,
                        });

                let surface_bind_group =
                    self.context
                        .device
                        .create_bind_group(&wgpu::BindGroupDescriptor {
                            label: Some("fast_blit_surface_bind_group"),
                            layout: &self.pipelines.surfaces_bind_group_layout,
                            entries: &[
                                wgpu::BindGroupEntry {
                                    binding: 0,
                                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                                        buffer: &params_buffer,
                                        offset: 0,
                                        size: None,
                                    }),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 1,
                                    resource: wgpu::BindingResource::TextureView(&view),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 2,
                                    resource: wgpu::BindingResource::Sampler(&self.surface_sampler),
                                },
                            ],
                        });

                pass.set_bind_group(1, &surface_bind_group, &[]);
                pass.draw(0..4, 0..1);
                surface_views.push(view);
                surface_param_buffers.push(params_buffer);
            }
        }

        self.context.queue.submit(Some(encoder.finish()));
        surface_texture.present();

        // Clear redraw flags only for surfaces that presented fresh frames.
        for surface_id in pending_surfaces {
            self.context
                .surface_registry
                .clear_redraw_pending(*surface_id);
        }

        true
    }

    /// Get list of surfaces that have pending redraws
    pub fn get_pending_surfaces(&self) -> Option<Vec<SurfaceId>> {
        let pending = self.context.surface_registry.get_pending_surfaces();
        if pending.is_empty() {
            None
        } else {
            Some(pending)
        }
    }

    /// Present without running compositor (fast blit already updated swapchain)
    pub fn present_framebuffer_only(&self) {
        // NOTE: Fast blit already presented to swapchain, so this is a no-op
        // When we implement persistent framebuffer properly, this will blit framebuffer → swapchain
        log::debug!("Present framebuffer only (no compositor) - fast blit already presented");
    }

    pub fn update_drawable_size(&mut self, size: geometry::Size<DevicePixels>) {
        self.surface_configuration.width = size.width.0 as u32;
        self.surface_configuration.height = size.height.0 as u32;
        self.surface
            .configure(&self.context.device, &self.surface_configuration);

        // Recreate persistent framebuffer at new size
        let persistent_framebuffer = self
            .context
            .device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("persistent_framebuffer"),
                size: wgpu::Extent3d {
                    width: self.surface_configuration.width,
                    height: self.surface_configuration.height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: self.surface_configuration.format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });

        let persistent_framebuffer_view =
            persistent_framebuffer.create_view(&wgpu::TextureViewDescriptor::default());

        self.persistent_framebuffer = Some(persistent_framebuffer);
        self.persistent_framebuffer_view = Some(persistent_framebuffer_view);

        // Recreate backdrop blur capture texture at the new size so that
        // copy_texture_to_texture doesn't silently skip due to a size mismatch.
        let backdrop_blur_texture = self
            .context
            .device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("backdrop_blur_texture"),
                size: wgpu::Extent3d {
                    width: self.surface_configuration.width,
                    height: self.surface_configuration.height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: self.surface_configuration.format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
        let backdrop_blur_texture_view =
            backdrop_blur_texture.create_view(&wgpu::TextureViewDescriptor::default());
        self.backdrop_blur_texture = Some(backdrop_blur_texture);
        self.backdrop_blur_texture_view = Some(backdrop_blur_texture_view);

        // Recreate the content-filter group textures at the new size so they stay
        // pixel-aligned with the surface (group composites sample using
        // `pixel_position / globals.viewport_size` UVs, just like backdrop blur).
        let (group_textures, group_views) = create_filter_group_textures(
            &self.context.device,
            self.surface_configuration.width,
            self.surface_configuration.height,
            self.surface_configuration.format,
        );
        self.group_textures = group_textures;
        self.group_views = group_views;

        // Invalidate bounds cache - all surface bounds are now stale
        self.layout_version.fetch_add(1, Ordering::Release);
        self.surface_bounds_cache.lock().unwrap().clear();
    }

    pub fn sprite_atlas(&self) -> Arc<dyn PlatformAtlas> {
        self.atlas.clone()
    }

    pub fn gpu_specs(&self) -> GpuSpecs {
        let info = self.context.adapter.get_info();
        GpuSpecs {
            is_software_emulated: info.device_type == wgpu::DeviceType::Cpu,
            device_name: info.name,
            driver_name: info.driver,
            driver_info: info.driver_info,
        }
    }

    pub fn update_transparency(&mut self, transparent: bool) {
        self.surface_configuration.alpha_mode = if transparent {
            wgpu::CompositeAlphaMode::PreMultiplied
        } else {
            // TODO(mdeand): Support for non-X11?
            // wgpu::CompositeAlphaMode::Opaque
            wgpu::CompositeAlphaMode::Inherit
        };
        self.surface
            .configure(&self.context.device, &self.surface_configuration);
    }

    pub fn viewport_size(&self) -> geometry::Size<DevicePixels> {
        geometry::Size {
            width: DevicePixels(self.surface_configuration.width as i32),
            height: DevicePixels(self.surface_configuration.height as i32),
        }
    }
}

impl Drop for WgpuRenderer {
    fn drop(&mut self) {
        // SAFETY: This is the only Drop impl and `surface` has not been dropped yet.
        // We take it manually so we can drop it inside catch_unwind, suppressing the Vulkan
        // panic that occurs when a SurfaceTexture's Arc still holds a swapchain semaphore
        // reference at the time the surface is destroyed (e.g. window closed mid-frame).
        let surface = unsafe { ManuallyDrop::take(&mut self.surface) };
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            drop(surface);
        }));
    }
}
