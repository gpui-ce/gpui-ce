use std::sync::Arc;

use crate::{
    AtlasTextureId, AtlasTile, DevicePixels, GpuSpecs, Hsla, LinearColorStop, MonochromeSprite,
    PlatformAtlas, PrimitiveBatch, Quad, ScaledPixels, Scene, TransformationMatrix, color,
    geometry,
    platform::cross::{atlas::WgpuAtlas, render_context::WgpuContext},
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
    const VERTEX_ATTRIBUTES: &'static [wgpu::VertexAttribute; 7] = &{
        let linear_color_stop_vertex_attributes = map_attributes(
            LinearColorStop::VERTEX_ATTRIBUTES,
            4,
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
                offset: std::mem::offset_of!(color::Background, gradient_angle_or_pattern_height)
                    as wgpu::BufferAddress,
                shader_location: 3,
                format: wgpu::VertexFormat::Float32,
            },
            linear_color_stop_vertex_attributes[0],
            linear_color_stop_vertex_attributes[1],
            wgpu::VertexAttribute {
                offset: std::mem::offset_of!(color::Background, pad) as wgpu::BufferAddress,
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
    const VERTEX_ATTRIBUTES: &'static [wgpu::VertexAttribute; 16] = &{
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

        let color_vertex_attributes = map_attributes(
            Hsla::VERTEX_ATTRIBUTES,
            6,
            std::mem::offset_of!(MonochromeSprite, color) as wgpu::BufferAddress,
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
            color_vertex_attributes[0],
            color_vertex_attributes[1],
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
    underlines_bind_group_layout: wgpu::BindGroupLayout,
    sprites_bind_group_layout: wgpu::BindGroupLayout,
    mono_sprites_bind_group_layout: wgpu::BindGroupLayout,
    poly_sprites_bind_group_layout: wgpu::BindGroupLayout,

    globals_bind_group: wgpu::BindGroup,
    color_adjustments_bind_group: wgpu::BindGroup,

    quads_pipeline: wgpu::RenderPipeline,
    shadows_pipeline: wgpu::RenderPipeline,
    underlines_pipeline: wgpu::RenderPipeline,
    mono_sprites_pipeline: wgpu::RenderPipeline,
    poly_sprites_pipeline: wgpu::RenderPipeline,
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
                    bind_group_layouts: &[&globals_bind_group_layout, &quads_bind_group_layout],
                    push_constant_ranges: &[],
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
                    bind_group_layouts: &[&globals_bind_group_layout, &shadows_bind_group_layout],
                    push_constant_ranges: &[],
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
                        &globals_bind_group_layout,
                        &underlines_bind_group_layout,
                    ],
                    push_constant_ranges: &[],
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
                        &globals_bind_group_layout,
                        &color_adjustments_bind_group_layout,
                        &sprites_bind_group_layout,
                        &mono_sprites_bind_group_layout,
                    ],
                    push_constant_ranges: &[],
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
                        &globals_bind_group_layout,
                        &sprites_bind_group_layout,
                        &poly_sprites_bind_group_layout,
                    ],
                    push_constant_ranges: &[],
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

        Self {
            color_targets: color_targets.to_vec(),

            quads_bind_group_layout,
            shadows_bind_group_layout,
            underlines_bind_group_layout,
            mono_sprites_bind_group_layout,
            sprites_bind_group_layout,
            poly_sprites_bind_group_layout,

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
                    multiview: None,
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
                    multiview: None,
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
                    multiview: None,
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
                    multiview: None,
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
                    multiview: None,
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

pub struct WgpuRenderer {
    context: Arc<WgpuContext>,
    surface: wgpu::Surface<'static>,
    surface_configuration: wgpu::SurfaceConfiguration,
    atlas_sampler: wgpu::Sampler,
    atlas: Arc<WgpuAtlas>,
    pipelines: WgpuPipelines,
    rendering_parameters: RenderingParameters,
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

        let surface_configuration = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width,
            height,
            present_mode: wgpu::PresentMode::Fifo,
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

        let pipelines =
            WgpuPipelines::new(context.as_ref(), &surface_configuration, path_sample_count);

        Ok(Self {
            context: context.clone(),
            surface,
            surface_configuration,
            atlas,
            atlas_sampler,
            pipelines,
            rendering_parameters: RenderingParameters::from_env(),
        })
    }

    pub fn draw(&self, scene: &Scene) {
        let mut command_encoder =
            self.context
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("main"),
                });

        self.atlas.before_frame(&mut command_encoder);

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
            self.context
                .queue
                .write_buffer(&self.context.quads_buffer, 0, unsafe {
                    as_bytes(&scene.quads)
                });
        }
        if !scene.shadows.is_empty() {
            self.context
                .queue
                .write_buffer(&self.context.shadows_buffer, 0, unsafe {
                    as_bytes(&scene.shadows)
                });
        }
        if !scene.underlines.is_empty() {
            self.context
                .queue
                .write_buffer(&self.context.underlines_buffer, 0, unsafe {
                    as_bytes(&scene.underlines)
                });
        }
        if !scene.monochrome_sprites.is_empty() {
            self.context
                .queue
                .write_buffer(&self.context.mono_sprites_buffer, 0, unsafe {
                    as_bytes(&scene.monochrome_sprites)
                });
        }
        if !scene.polychrome_sprites.is_empty() {
            self.context
                .queue
                .write_buffer(&self.context.poly_sprites_buffer, 0, unsafe {
                    as_bytes(&scene.polychrome_sprites)
                });
        }

        let surface_texture = self
            .surface
            .get_current_texture()
            .expect("Failed to acquire next swap chain texture");

        let quads_bind_group = self
            .context
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("quads_bind_group"),
                layout: &self.pipelines.quads_bind_group_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &self.context.quads_buffer,
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
                            buffer: &self.context.shadows_buffer,
                            offset: 0,
                            size: None,
                        }),
                    }],
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
                            buffer: &self.context.underlines_buffer,
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
                            buffer: &self.context.mono_sprites_buffer,
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
                            buffer: &self.context.poly_sprites_buffer,
                            offset: 0,
                            size: None,
                        }),
                    }],
                });

        {
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
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            let mut quads_first_instance: u32 = 0;
            let mut shadows_first_instance: u32 = 0;
            let mut underlines_first_instance: u32 = 0;
            let mut mono_sprites_first_instance: u32 = 0;
            let mut poly_sprites_first_instance: u32 = 0;

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
                    // TODO(mdeand): Implement paths and surfaces rendering.
                    PrimitiveBatch::Paths(_) | PrimitiveBatch::Surfaces(_) => {}
                }
            }
        }

        self.context.queue.submit(Some(command_encoder.finish()));

        surface_texture.present();
    }

    pub fn update_drawable_size(&mut self, size: geometry::Size<DevicePixels>) {
        self.surface_configuration.width = size.width.0 as u32;
        self.surface_configuration.height = size.height.0 as u32;
        self.surface
            .configure(&self.context.device, &self.surface_configuration);

        // todo!()
    }

    pub fn sprite_atlas(&self) -> Arc<dyn PlatformAtlas> {
        self.atlas.clone()
    }

    pub fn gpu_specs(&self) -> GpuSpecs {
        GpuSpecs {
            is_software_emulated: false,
            device_name: "gpu 9000".to_owned(),
            driver_name: "gpu 9000 driver".to_owned(),
            driver_info: "gpu 9000 driver info".to_owned(),
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

        // todo!()
    }

    pub fn destroy(&mut self) {
        println!("WgpuRenderer destroyed");
        // TODO(mdeand): Implement proper destruction logic.
    }

    pub fn viewport_size(&self) -> wgpu::Extent3d {
        // TODO(mdeand): Hack
        wgpu::Extent3d {
            width: 500,
            height: 500,
            depth_or_array_layers: 1,
        }
    }
}
