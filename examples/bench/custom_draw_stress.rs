//! Custom Draw Stress Harness
//!
//! Renders many instanced quads using the custom draw API.

#[path = "../prelude.rs"]
mod example_prelude;

use std::sync::{Arc, Mutex};
use std::time::Instant;

use gpui::{
    App, AppContext, Application, Bounds, Colors, Context, CustomAddressMode, CustomBindingDesc,
    CustomBindingKind, CustomBindingName, CustomBindingValue, CustomBufferDesc, CustomBufferId,
    CustomBufferSource, CustomDrawParams, CustomFilterMode, CustomPipelineDesc, CustomPipelineId,
    CustomPipelineState, CustomPrimitiveTopology, CustomSamplerDesc, CustomSamplerId,
    CustomTextureDesc, CustomTextureFormat, CustomTextureId, CustomTextureUsage,
    CustomUniformBuilder, CustomVertexAttribute, CustomVertexAttributeName, CustomVertexBuffer,
    CustomVertexFetch, CustomVertexFormat, CustomVertexLayout, Hsla, Render, Styled, Window,
    WindowBounds, WindowOptions, canvas, div, prelude::*, px, size,
};

const SHADER_SOURCE: &str = r#"
struct VertexInput {
  a0: vec2<f32>,
  a1: vec2<f32>,
};

struct VertexOutput {
  @builtin(position) position: vec4<f32>,
  @location(0) uv: vec2<f32>,
};

struct Uniforms {
  bounds_origin: vec2<f32>,
  bounds_size: vec2<f32>,
  viewport: vec2<f32>,
  grid: vec2<f32>,
  pad: vec2<f32>,
  pad2: vec2<f32>,
};

var b0: texture_2d<f32>;
var b1: sampler;
var<uniform> b2: Uniforms;

@vertex
fn vs_main(input: VertexInput, @builtin(instance_index) instance_index: u32) -> VertexOutput {
  var out: VertexOutput;
  let grid_x = max(u32(b2.grid.x), 1u);
  let grid_y = max(u32(b2.grid.y), 1u);
  let gx = f32(instance_index % grid_x);
  let gy = f32(instance_index / grid_x);
  let fx = select(0.5, gx / max(f32(grid_x - 1u), 1.0), grid_x > 1u);
  let fy = select(0.5, gy / max(f32(grid_y - 1u), 1.0), grid_y > 1u);
  let inner = max(b2.bounds_size - b2.pad * 2.0, vec2<f32>(1.0, 1.0));
  let px = b2.bounds_origin + b2.pad + inner * vec2<f32>(fx, fy);
  let ndc = vec2<f32>(
    (px.x / b2.viewport.x) * 2.0 - 1.0,
    1.0 - (px.y / b2.viewport.y) * 2.0
  );
  let pos = input.a0 + ndc;
  out.position = vec4<f32>(pos, 0.0, 1.0);
  out.uv = input.a1;
  return out;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
  let tex = textureSample(b0, b1, input.uv);
  return tex;
}
"#;

const DEFAULT_INSTANCE_COUNT: usize = 400;
const DEFAULT_QUAD_SIZE_PX: f32 = 14.0;
const DEFAULT_GRID_PAD_PX: f32 = 8.0;
const DEFAULT_BOUNDS_INSET_PX: f32 = 1.0;

#[derive(Debug, Clone, Copy)]
struct StressConfig {
    instances: usize,
    quad_size_px: f32,
    grid_pad_px: f32,
    bounds_inset_px: f32,
}

impl StressConfig {
    fn from_args() -> Self {
        let mut config = StressConfig {
            instances: DEFAULT_INSTANCE_COUNT,
            quad_size_px: DEFAULT_QUAD_SIZE_PX,
            grid_pad_px: DEFAULT_GRID_PAD_PX,
            bounds_inset_px: DEFAULT_BOUNDS_INSET_PX,
        };

        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--instances" => {
                    if let Some(value) = args.next() {
                        if let Ok(parsed) = value.parse::<usize>() {
                            config.instances = parsed.max(1);
                        }
                    }
                }
                "--quad-size" => {
                    if let Some(value) = args.next() {
                        if let Ok(parsed) = value.parse::<f32>() {
                            config.quad_size_px = parsed.max(1.0);
                        }
                    }
                }
                "--grid-pad" => {
                    if let Some(value) = args.next() {
                        if let Ok(parsed) = value.parse::<f32>() {
                            config.grid_pad_px = parsed.max(0.0);
                        }
                    }
                }
                "--bounds-inset" => {
                    if let Some(value) = args.next() {
                        if let Ok(parsed) = value.parse::<f32>() {
                            config.bounds_inset_px = parsed.max(0.0);
                        }
                    }
                }
                _ => {}
            }
        }

        config
    }
}

struct StressHarness {
    pipeline: Option<CustomPipelineId>,
    vertex_buffer: Option<CustomBufferId>,
    texture: Option<CustomTextureId>,
    sampler: Option<CustomSamplerId>,
    bounds_cache: Arc<Mutex<Option<Bounds<gpui::Pixels>>>>,
    viewport_cache: Arc<Mutex<Option<gpui::Size<gpui::Pixels>>>>,
    error: Option<String>,
    frame_count: u32,
    last_fps: f32,
    last_fps_tick: Instant,
    config: StressConfig,
}

impl StressHarness {
    fn new(config: StressConfig, _cx: &mut Context<Self>) -> Self {
        Self {
            pipeline: None,
            vertex_buffer: None,
            texture: None,
            sampler: None,
            bounds_cache: Arc::new(Mutex::new(None)),
            viewport_cache: Arc::new(Mutex::new(None)),
            error: None,
            frame_count: 0,
            last_fps: 0.0,
            last_fps_tick: Instant::now(),
            config,
        }
    }

    fn ensure_resources(&mut self, window: &mut Window) {
        if self.pipeline.is_some() || self.error.is_some() {
            return;
        }

        match self.build_resources(window) {
            Ok((pipeline, buffer, texture, sampler)) => {
                self.pipeline = Some(pipeline);
                self.vertex_buffer = Some(buffer);
                self.texture = Some(texture);
                self.sampler = Some(sampler);
            }
            Err(err) => {
                self.error = Some(err.to_string());
            }
        }
    }

    fn build_resources(
        &self,
        window: &mut Window,
    ) -> anyhow::Result<(
        CustomPipelineId,
        CustomBufferId,
        CustomTextureId,
        CustomSamplerId,
    )> {
        let pipeline = window.create_custom_pipeline(CustomPipelineDesc {
            name: "custom_draw_stress".to_string(),
            shader_source: SHADER_SOURCE.to_string(),
            vertex_entry: "vs_main".to_string(),
            fragment_entry: "fs_main".to_string(),
            vertex_fetches: vec![CustomVertexFetch {
                layout: CustomVertexLayout {
                    stride: 16,
                    attributes: vec![
                        CustomVertexAttribute {
                            name: CustomVertexAttributeName::A0,
                            offset: 0,
                            format: CustomVertexFormat::F32Vec2,
                            location: None,
                        },
                        CustomVertexAttribute {
                            name: CustomVertexAttributeName::A1,
                            offset: 8,
                            format: CustomVertexFormat::F32Vec2,
                            location: None,
                        },
                    ],
                },
                instanced: false,
            }],
            primitive: CustomPrimitiveTopology::TriangleList,
            target_format: None,
            state: CustomPipelineState::default(),
            bindings: vec![
                CustomBindingDesc {
                    name: CustomBindingName::B0,
                    kind: CustomBindingKind::Texture,
                    slot: None,
                },
                CustomBindingDesc {
                    name: CustomBindingName::B1,
                    kind: CustomBindingKind::Sampler,
                    slot: None,
                },
                CustomBindingDesc {
                    name: CustomBindingName::B2,
                    kind: CustomBindingKind::Uniform { size: 48 },
                    slot: None,
                },
            ],
        })?;

        let vertex_buffer = window.create_custom_buffer(CustomBufferDesc {
            name: "stress_vertices".to_string(),
            data: quad_vertex_data(),
        })?;

        let texture = window.create_custom_texture(CustomTextureDesc {
            name: "stress_texture".to_string(),
            width: 2,
            height: 2,
            format: CustomTextureFormat::Rgba8Unorm,
            usage: CustomTextureUsage::SAMPLED,
            data: vec![checker_texture_data()],
        })?;

        let sampler = window.create_custom_sampler(CustomSamplerDesc {
            name: "stress_sampler".to_string(),
            min_filter: CustomFilterMode::Nearest,
            mag_filter: CustomFilterMode::Nearest,
            mipmap_filter: CustomFilterMode::Nearest,
            address_modes: [CustomAddressMode::Repeat; 3],
        })?;

        Ok((pipeline, vertex_buffer, texture, sampler))
    }
}

impl Render for StressHarness {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl gpui::IntoElement {
        let colors = Colors::for_appearance(window);
        self.ensure_resources(window);
        window.request_animation_frame();

        self.frame_count = self.frame_count.wrapping_add(1);
        let elapsed = self.last_fps_tick.elapsed().as_secs_f32();
        if elapsed >= 1.0 {
            self.last_fps = self.frame_count as f32 / elapsed;
            self.frame_count = 0;
            self.last_fps_tick = Instant::now();
        }

        let header = div()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .text_xl()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(colors.text)
                    .child("Custom Draw Stress"),
            )
            .child(div().text_sm().text_color(colors.text_muted).child(format!(
                "Draws: 1  |  Instances: {}  |  FPS: {:.1}",
                self.config.instances, self.last_fps
            )));

        let surface: Hsla = colors.surface.into();
        let content = if let Some(err) = &self.error {
            div()
                .text_sm()
                .text_color(colors.error)
                .child(format!("Custom draw unsupported: {err}"))
        } else if let (Some(pipeline), Some(vertex_buffer), Some(texture), Some(sampler)) = (
            self.pipeline,
            self.vertex_buffer,
            self.texture,
            self.sampler,
        ) {
            let bounds_cache = Arc::clone(&self.bounds_cache);
            let viewport_cache = Arc::clone(&self.viewport_cache);
            let config = self.config;
            let prepaint = move |bounds: Bounds<_>, window: &mut Window, _cx: &mut App| {
                let layout_bounds = inset_bounds(bounds, px(config.bounds_inset_px));
                let viewport = window.viewport_size();
                let mut update_vertices = false;

                {
                    let mut cached = viewport_cache.lock().unwrap();
                    if cached.map_or(true, |cached| cached != viewport) {
                        *cached = Some(viewport);
                        update_vertices = true;
                    }
                }
                {
                    let mut cached = bounds_cache.lock().unwrap();
                    if cached.map_or(true, |cached| cached != layout_bounds) {
                        *cached = Some(layout_bounds);
                    }
                }

                if update_vertices {
                    let vertex_data =
                        quad_vertex_data_for_pixel_size(px(config.quad_size_px), viewport);
                    if let Err(err) =
                        window.update_custom_buffer(vertex_buffer, Arc::clone(&vertex_data))
                    {
                        log::error!("custom draw vertex update failed: {err}");
                    }
                }
                let mut uniform = CustomUniformBuilder::new();
                uniform
                    .push_vec2(
                        f32::from(layout_bounds.origin.x),
                        f32::from(layout_bounds.origin.y),
                    )
                    .push_vec2(
                        f32::from(layout_bounds.size.width),
                        f32::from(layout_bounds.size.height),
                    )
                    .push_vec2(f32::from(viewport.width), f32::from(viewport.height));
                let grid = (config.instances as f32).sqrt().ceil();
                uniform.push_vec2(grid, grid).push_vec2(
                    f32::from(px(config.grid_pad_px)),
                    f32::from(px(config.grid_pad_px)),
                );
                CustomDrawParams {
                    bounds,
                    pipeline,
                    vertex_buffers: vec![CustomVertexBuffer {
                        source: CustomBufferSource::Buffer(vertex_buffer),
                    }],
                    vertex_count: 6,
                    index_buffer: None,
                    index_count: 0,
                    target: None,
                    instance_count: config.instances as u32,
                    bindings: vec![
                        CustomBindingValue::Texture(texture),
                        CustomBindingValue::Sampler(sampler),
                        CustomBindingValue::Uniform(CustomBufferSource::Inline(uniform.finish())),
                    ],
                }
            };

            let paint = move |_bounds: Bounds<_>,
                              params: CustomDrawParams,
                              window: &mut Window,
                              _cx: &mut App| {
                if let Err(err) = window.paint_custom(params) {
                    log::error!("custom draw paint failed: {err}");
                }
            };

            div()
                .w(px(640.))
                .h(px(480.))
                .rounded_md()
                .border_1()
                .border_color(colors.border)
                .bg(surface.opacity(0.2))
                .child(canvas(prepaint, paint).size_full())
        } else {
            div()
                .text_sm()
                .text_color(colors.text_muted)
                .child("Initializing custom draw resources...")
        };

        div()
            .size_full()
            .p_6()
            .bg(colors.background)
            .child(div().flex().flex_col().gap_4().child(header).child(content))
    }
}

fn inset_bounds(bounds: Bounds<gpui::Pixels>, inset: gpui::Pixels) -> Bounds<gpui::Pixels> {
    let width = (bounds.size.width - inset * 2.0).max(px(1.0));
    let height = (bounds.size.height - inset * 2.0).max(px(1.0));
    Bounds {
        origin: bounds.origin + gpui::Point::new(inset, inset),
        size: gpui::Size::new(width, height),
    }
}

fn push_f32(data: &mut Vec<u8>, value: f32) {
    data.extend_from_slice(&value.to_le_bytes());
}

fn quad_vertex_data() -> Arc<[u8]> {
    let mut data = Vec::with_capacity(6 * 4 * 4);
    let vertices = [
        (-0.02, -0.02, 0.0, 1.0),
        (0.02, -0.02, 1.0, 1.0),
        (0.02, 0.02, 1.0, 0.0),
        (-0.02, -0.02, 0.0, 1.0),
        (0.02, 0.02, 1.0, 0.0),
        (-0.02, 0.02, 0.0, 0.0),
    ];

    for (x, y, u, v) in vertices {
        push_f32(&mut data, x);
        push_f32(&mut data, y);
        push_f32(&mut data, u);
        push_f32(&mut data, v);
    }

    Arc::from(data)
}

fn quad_vertex_data_for_pixel_size(
    size_px: gpui::Pixels,
    viewport: gpui::Size<gpui::Pixels>,
) -> Arc<[u8]> {
    let mut data = Vec::with_capacity(6 * 4 * 4);
    let viewport_w = f32::from(viewport.width).max(1.0);
    let viewport_h = f32::from(viewport.height).max(1.0);

    let half_w = f32::from(size_px) / viewport_w;
    let half_h = f32::from(size_px) / viewport_h;

    let left_ndc = -half_w;
    let right_ndc = half_w;
    let top_ndc = half_h;
    let bottom_ndc = -half_h;

    let vertices = [
        (left_ndc, top_ndc, 0.0, 0.0),
        (right_ndc, top_ndc, 1.0, 0.0),
        (right_ndc, bottom_ndc, 1.0, 1.0),
        (left_ndc, top_ndc, 0.0, 0.0),
        (right_ndc, bottom_ndc, 1.0, 1.0),
        (left_ndc, bottom_ndc, 0.0, 1.0),
    ];

    for (x, y, u, v) in vertices {
        push_f32(&mut data, x);
        push_f32(&mut data, y);
        push_f32(&mut data, u);
        push_f32(&mut data, v);
    }

    Arc::from(data)
}

fn checker_texture_data() -> Arc<[u8]> {
    let data: [u8; 16] = [
        0xff, 0x4d, 0x4d, 0xff, // red
        0x4d, 0xff, 0x4d, 0xff, // green
        0x4d, 0x4d, 0xff, 0xff, // blue
        0xff, 0xff, 0xff, 0xff, // white
    ];
    Arc::from(data)
}

fn main() {
    let config = StressConfig::from_args();
    Application::new().run(move |cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(720.), px(640.)), cx);
        let config_copy = config;
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            move |_, cx| cx.new(|cx| StressHarness::new(config_copy, cx)),
        )
        .expect("Failed to open window");

        example_prelude::init_example(cx, "Custom Draw Stress");
    });
}
