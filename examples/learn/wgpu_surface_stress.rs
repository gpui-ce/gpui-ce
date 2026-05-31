/// Example: stress multi-surface rendering with an 8xN grid of WgpuSurface views.
///
/// Set `WGPU_SURFACE_STRESS_ROWS` to control the row count (default: 8).
use gpui::{
    div, prelude::*, px, rgb, rgba, wgpu_surface, App, Application, Context, Render,
    WgpuSurfaceHandle, Window, WindowOptions,
};
use std::sync::Arc;
use wgpu::util::DeviceExt;

const GRID_COLUMNS: usize = 8;
const DEFAULT_GRID_ROWS: usize = 8;
const SURFACE_WIDTH: u32 = 256;
const SURFACE_HEIGHT: u32 = 144;

const SHADER: &str = r#"
struct Uniforms {
    mvp: mat4x4<f32>,
}

@group(0) @binding(0) var<uniform> uniforms: Uniforms;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) color: vec3<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec3<f32>,
}

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.clip_position = uniforms.mvp * vec4<f32>(in.position, 1.0);
    out.color = in.color;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(in.color, 1.0);
}
"#;

#[rustfmt::skip]
const VERTICES: &[[f32; 6]] = &[
    [-0.5, -0.5,  0.5,  0.90, 0.20, 0.20], [ 0.5, -0.5,  0.5,  0.90, 0.20, 0.20],
    [ 0.5,  0.5,  0.5,  1.00, 0.50, 0.50], [-0.5,  0.5,  0.5,  1.00, 0.50, 0.50],
    [ 0.5, -0.5, -0.5,  0.20, 0.80, 0.20], [-0.5, -0.5, -0.5,  0.20, 0.80, 0.20],
    [-0.5,  0.5, -0.5,  0.50, 1.00, 0.50], [ 0.5,  0.5, -0.5,  0.50, 1.00, 0.50],
    [-0.5, -0.5, -0.5,  0.20, 0.20, 0.90], [-0.5, -0.5,  0.5,  0.20, 0.20, 0.90],
    [-0.5,  0.5,  0.5,  0.50, 0.50, 1.00], [-0.5,  0.5, -0.5,  0.50, 0.50, 1.00],
    [ 0.5, -0.5,  0.5,  0.90, 0.90, 0.20], [ 0.5, -0.5, -0.5,  0.90, 0.90, 0.20],
    [ 0.5,  0.5, -0.5,  1.00, 1.00, 0.50], [ 0.5,  0.5,  0.5,  1.00, 1.00, 0.50],
    [-0.5,  0.5,  0.5,  0.20, 0.90, 0.90], [ 0.5,  0.5,  0.5,  0.20, 0.90, 0.90],
    [ 0.5,  0.5, -0.5,  0.50, 1.00, 1.00], [-0.5,  0.5, -0.5,  0.50, 1.00, 1.00],
    [-0.5, -0.5, -0.5,  0.90, 0.20, 0.90], [ 0.5, -0.5, -0.5,  0.90, 0.20, 0.90],
    [ 0.5, -0.5,  0.5,  1.00, 0.50, 1.00], [-0.5, -0.5,  0.5,  1.00, 0.50, 1.00],
];

#[rustfmt::skip]
const INDICES: &[u16] = &[
     0,  1,  2,   0,  2,  3,
     4,  5,  6,   4,  6,  7,
     8,  9, 10,   8, 10, 11,
    12, 13, 14,  12, 14, 15,
    16, 17, 18,  16, 18, 19,
    20, 21, 22,  20, 22, 23,
];

struct CubeRenderState {
    pipeline: wgpu::RenderPipeline,
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    uniform_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    depth_view: wgpu::TextureView,
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    start_time: std::time::Instant,
    width: u32,
    height: u32,
    speed: f32,
    phase: f32,
    clear: [f64; 3],
}

impl CubeRenderState {
    fn new(
        device: Arc<wgpu::Device>,
        queue: Arc<wgpu::Queue>,
        width: u32,
        height: u32,
        color_format: wgpu::TextureFormat,
        speed: f32,
        phase: f32,
        clear: [f64; 3],
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("cube_shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("cube_uniforms"),
            size: 64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("cube_bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("cube_bg"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("cube_pipeline_layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("cube_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: 24,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[
                        wgpu::VertexAttribute {
                            offset: 0,
                            shader_location: 0,
                            format: wgpu::VertexFormat::Float32x3,
                        },
                        wgpu::VertexAttribute {
                            offset: 12,
                            shader_location: 1,
                            format: wgpu::VertexFormat::Float32x3,
                        },
                    ],
                }],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: color_format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("cube_vb"),
            contents: bytemuck::cast_slice(VERTICES),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("cube_ib"),
            contents: bytemuck::cast_slice(INDICES),
            usage: wgpu::BufferUsages::INDEX,
        });

        let depth_view = Self::make_depth_view(&device, width, height);

        Self {
            pipeline,
            vertex_buffer,
            index_buffer,
            uniform_buffer,
            bind_group,
            depth_view,
            device,
            queue,
            start_time: std::time::Instant::now(),
            width,
            height,
            speed,
            phase,
            clear,
        }
    }

    fn make_depth_view(device: &wgpu::Device, width: u32, height: u32) -> wgpu::TextureView {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("cube_depth"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        texture.create_view(&wgpu::TextureViewDescriptor::default())
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.width = width;
        self.height = height;
        self.depth_view = Self::make_depth_view(&self.device, width, height);
    }

    fn render(&mut self, view: &wgpu::TextureView) {
        let t = self.start_time.elapsed().as_secs_f32() * self.speed + self.phase;
        let aspect = self.width as f32 / self.height.max(1) as f32;
        let proj = glam::Mat4::perspective_rh(std::f32::consts::FRAC_PI_4, aspect, 0.1, 100.0);
        let camera = glam::Mat4::look_at_rh(
            glam::Vec3::new(0.0, 0.9, 2.5),
            glam::Vec3::ZERO,
            glam::Vec3::Y,
        );
        let model = glam::Mat4::from_rotation_y(t) * glam::Mat4::from_rotation_x(t * 0.65);
        let mvp: [[f32; 4]; 4] = (proj * camera * model).to_cols_array_2d();
        self.queue
            .write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&mvp));

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("cube_encoder"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("cube_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: self.clear[0],
                            g: self.clear[1],
                            b: self.clear[2],
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
            pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
            pass.draw_indexed(0..INDICES.len() as u32, 0, 0..1);
        }
        self.queue.submit(std::iter::once(encoder.finish()));
    }
}

struct SurfaceTile {
    index: usize,
    surface: WgpuSurfaceHandle,
    state: Option<CubeRenderState>,
    frame_count: u32,
    last_fps_update: std::time::Instant,
    display_fps: f64,
    speed: f32,
    phase: f32,
    clear: [f64; 3],
}

impl SurfaceTile {
    fn new(
        index: usize,
        surface: WgpuSurfaceHandle,
        speed: f32,
        phase: f32,
        clear: [f64; 3],
    ) -> Self {
        Self {
            index,
            surface,
            state: None,
            frame_count: 0,
            last_fps_update: std::time::Instant::now(),
            display_fps: 0.0,
            speed,
            phase,
            clear,
        }
    }
}

struct SurfaceStressExample {
    tiles: Vec<SurfaceTile>,
    rows: usize,
}

impl SurfaceStressExample {
    fn render_tile(tile: &mut SurfaceTile) {
        if let Some((view, (w, h))) = tile.surface.back_view_with_size() {
            let state = tile.state.get_or_insert_with(|| {
                CubeRenderState::new(
                    Arc::new(tile.surface.device().clone()),
                    Arc::new(tile.surface.queue().clone()),
                    w,
                    h,
                    tile.surface.format(),
                    tile.speed,
                    tile.phase,
                    tile.clear,
                )
            });

            if state.width != w || state.height != h {
                state.resize(w, h);
            }

            state.render(&view);
            drop(view);
            tile.surface.swap_buffers();

            tile.frame_count = tile.frame_count.wrapping_add(1);
            let now = std::time::Instant::now();
            if now.duration_since(tile.last_fps_update) >= std::time::Duration::from_secs(1) {
                tile.display_fps = tile.frame_count as f64;
                tile.frame_count = 0;
                tile.last_fps_update = now;
            }
        }
    }

    fn tile_element(tile: &SurfaceTile) -> impl IntoElement {
        div()
            .relative()
            .h(px(112.0))
            .border_1()
            .border_color(rgb(0x2a2e3a))
            .rounded_sm()
            .bg(rgb(0x10131a))
            .child(wgpu_surface(tile.surface.clone()).absolute().inset_0())
            .child(
                div()
                    .absolute()
                    .top(px(4.0))
                    .left(px(6.0))
                    .px_1()
                    .py_0p5()
                    .bg(rgba(0x0000008c))
                    .rounded_sm()
                    .text_color(rgb(0xe4ecff))
                    .text_xs()
                    .child(format!("#{:02} {:.0} fps", tile.index, tile.display_fps)),
            )
    }
}

impl Render for SurfaceStressExample {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        for tile in &mut self.tiles {
            Self::render_tile(tile);
        }
        cx.notify();

        div().size_full().p_2().bg(rgb(0x0b0e14)).child(
            div()
                .id("stress-root")
                .size_full()
                .overflow_scroll()
                .child(
                    div()
                        .w_full()
                        .grid()
                        .grid_cols(GRID_COLUMNS as u16)
                        .gap_1()
                        .children(self.tiles.iter().map(Self::tile_element)),
                )
                .child(
                    div()
                        .absolute()
                        .top(px(8.0))
                        .right(px(8.0))
                        .px_2()
                        .py_1()
                        .bg(rgba(0x000000a8))
                        .rounded_sm()
                        .text_color(rgb(0xe4ecff))
                        .text_xs()
                        .child(format!(
                            "{}x{} surfaces ({})",
                            GRID_COLUMNS,
                            self.rows,
                            self.tiles.len()
                        )),
                ),
        )
    }
}

fn read_rows() -> usize {
    std::env::var("WGPU_SURFACE_STRESS_ROWS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|rows| *rows > 0)
        .unwrap_or(DEFAULT_GRID_ROWS)
}

fn tile_clear_color(index: usize) -> [f64; 3] {
    let red = 0.08 + ((index * 37 % 100) as f64 / 220.0);
    let green = 0.08 + ((index * 67 % 100) as f64 / 220.0);
    let blue = 0.08 + ((index * 97 % 100) as f64 / 220.0);
    [red, green, blue]
}

fn main() {
    env_logger::init();
    let rows = read_rows();
    let tile_count = GRID_COLUMNS * rows;

    Application::new().run(move |cx: &mut App| {
        _ = cx.open_window(
            WindowOptions::default(),
            move |window: &mut Window, cx: &mut App| {
                let mut tiles = Vec::with_capacity(tile_count);
                for index in 0..tile_count {
                    let surface = window
                        .create_wgpu_surface(
                            SURFACE_WIDTH,
                            SURFACE_HEIGHT,
                            wgpu::TextureFormat::Rgba8UnormSrgb,
                        )
                        .expect("WgpuSurface not supported on this platform");

                    let speed = 0.8 + ((index % 17) as f32 * 0.09);
                    let phase = index as f32 * 0.31;
                    tiles.push(SurfaceTile::new(
                        index,
                        surface,
                        speed,
                        phase,
                        tile_clear_color(index),
                    ));
                }

                cx.new(|_cx| SurfaceStressExample { tiles, rows })
            },
        );
    });
}
