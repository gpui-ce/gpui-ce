//! Custom Draw API (Instanced) Example
//!
//! Demonstrates instanced rendering with a per-instance buffer.
//!
//! Notes:
//! - Works on Metal (default macOS) and Blade (`macos-blade` feature).
//! - WGSL must omit @location and @group/@binding; use a0.. and b0.. names instead.

#[path = "../prelude.rs"]
mod example_prelude;

use std::sync::Arc;
use std::time::Instant;

use gpui::{
    App, AppContext, Application, Bounds, Colors, Context, CustomAddressMode, CustomBindingDesc,
    CustomBindingKind, CustomBindingName, CustomBindingValue, CustomBufferDesc, CustomBufferId,
    CustomBufferSource, CustomDrawParams, CustomFilterMode, CustomPipelineDesc, CustomPipelineId,
    CustomPrimitiveTopology, CustomSamplerDesc, CustomSamplerId, CustomTextureDesc,
    CustomTextureFormat, CustomTextureId, CustomVertexAttribute, CustomVertexAttributeName,
    CustomVertexBuffer, CustomVertexFetch, CustomVertexFormat, CustomVertexLayout, Hsla, Render,
    Styled, Window, WindowBounds, WindowOptions, canvas, div, prelude::*, px, size,
};

const SHADER_SOURCE: &str = r#"
struct VertexInput {
  a0: vec2<f32>,
  a1: vec2<f32>,
  a2: vec2<f32>,
  a3: vec4<f32>,
};

struct VertexOutput {
  @builtin(position) position: vec4<f32>,
  @location(0) uv: vec2<f32>,
  @location(1) color: vec4<f32>,
};

struct Uniforms {
  time: f32,
  amplitude: f32,
  pad: vec2<f32>,
};

var b0: texture_2d<f32>;
var b1: sampler;
var<uniform> b2: Uniforms;

@vertex
fn vs_main(input: VertexInput, @builtin(instance_index) instance_index: u32) -> VertexOutput {
  var out: VertexOutput;
  let phase = f32(instance_index) * 0.35;
  let wobble = vec2<f32>(sin(b2.time + phase), cos(b2.time + phase)) * b2.amplitude;
  let pos = input.a0 + input.a2 + wobble;
  out.position = vec4<f32>(pos, 0.0, 1.0);
  out.uv = input.a1;
  out.color = input.a3;
  return out;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
  let tex = textureSample(b0, b1, input.uv);
  return tex * input.color;
}
"#;

struct InstancedCustomDrawExample {
    pipeline: Option<CustomPipelineId>,
    vertex_buffer: Option<CustomBufferId>,
    instance_buffer: Option<CustomBufferId>,
    texture: Option<CustomTextureId>,
    sampler: Option<CustomSamplerId>,
    start: Instant,
    error: Option<String>,
    config: InstancedConfig,
}

const DEFAULT_INSTANCE_COUNT: usize = 12;
const DEFAULT_QUAD_SIZE_PX: f32 = 32.0;
const DEFAULT_GRID_PAD_PX: f32 = 20.0;
const DEFAULT_BOUNDS_INSET_PX: f32 = 1.0;

#[derive(Debug, Clone, Copy)]
struct InstancedConfig {
    instances: usize,
    quad_size_px: f32,
    grid_pad_px: f32,
    bounds_inset_px: f32,
}

impl InstancedConfig {
    fn from_args() -> Self {
        let mut config = InstancedConfig {
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

impl InstancedCustomDrawExample {
    fn new(config: InstancedConfig, _cx: &mut Context<Self>) -> Self {
        Self {
            pipeline: None,
            vertex_buffer: None,
            instance_buffer: None,
            texture: None,
            sampler: None,
            start: Instant::now(),
            error: None,
            config,
        }
    }

    fn ensure_resources(&mut self, window: &mut Window) {
        if self.pipeline.is_some() || self.error.is_some() {
            return;
        }

        match self.build_resources(window) {
            Ok((pipeline, buffer, instances, texture, sampler)) => {
                self.pipeline = Some(pipeline);
                self.vertex_buffer = Some(buffer);
                self.instance_buffer = Some(instances);
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
        CustomBufferId,
        CustomTextureId,
        CustomSamplerId,
    )> {
        let pipeline = window.create_custom_pipeline(CustomPipelineDesc {
            name: "custom_draw_demo_instanced".to_string(),
            shader_source: SHADER_SOURCE.to_string(),
            vertex_entry: "vs_main".to_string(),
            fragment_entry: "fs_main".to_string(),
            vertex_fetches: vec![
                CustomVertexFetch {
                    layout: CustomVertexLayout {
                        stride: 16,
                        attributes: vec![
                            CustomVertexAttribute {
                                name: CustomVertexAttributeName::A0,
                                offset: 0,
                                format: CustomVertexFormat::F32Vec2,
                            },
                            CustomVertexAttribute {
                                name: CustomVertexAttributeName::A1,
                                offset: 8,
                                format: CustomVertexFormat::F32Vec2,
                            },
                        ],
                    },
                    instanced: false,
                },
                CustomVertexFetch {
                    layout: CustomVertexLayout {
                        stride: 24,
                        attributes: vec![
                            CustomVertexAttribute {
                                name: CustomVertexAttributeName::A2,
                                offset: 0,
                                format: CustomVertexFormat::F32Vec2,
                            },
                            CustomVertexAttribute {
                                name: CustomVertexAttributeName::A3,
                                offset: 8,
                                format: CustomVertexFormat::F32Vec4,
                            },
                        ],
                    },
                    instanced: true,
                },
            ],
            primitive: CustomPrimitiveTopology::TriangleList,
            bindings: vec![
                CustomBindingDesc {
                    name: CustomBindingName::B0,
                    kind: CustomBindingKind::Texture,
                },
                CustomBindingDesc {
                    name: CustomBindingName::B1,
                    kind: CustomBindingKind::Sampler,
                },
                CustomBindingDesc {
                    name: CustomBindingName::B2,
                    kind: CustomBindingKind::Uniform { size: 16 },
                },
            ],
        })?;

        let vertex_buffer = window.create_custom_buffer(CustomBufferDesc {
            name: "quad_vertices_instanced".to_string(),
            data: quad_vertex_data(),
        })?;

        let instance_buffer = window.create_custom_buffer(CustomBufferDesc {
            name: "quad_instances".to_string(),
            data: instance_data_for_bounds(
                Bounds::default(),
                window.viewport_size(),
                self.config,
            ),
        })?;

        let texture_data = checker_texture_data();
        let texture = window.create_custom_texture(CustomTextureDesc {
            name: "checker_texture_instanced".to_string(),
            width: 2,
            height: 2,
            format: CustomTextureFormat::Rgba8Unorm,
            data: texture_data,
        })?;

        let sampler = window.create_custom_sampler(CustomSamplerDesc {
            name: "checker_sampler_instanced".to_string(),
            min_filter: CustomFilterMode::Nearest,
            mag_filter: CustomFilterMode::Nearest,
            mipmap_filter: CustomFilterMode::Nearest,
            address_modes: [CustomAddressMode::ClampToEdge; 3],
        })?;

        Ok((pipeline, vertex_buffer, instance_buffer, texture, sampler))
    }
}

impl Render for InstancedCustomDrawExample {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl gpui::IntoElement {
        let colors = Colors::for_appearance(window);
        self.ensure_resources(window);
        window.request_animation_frame();

        let header = div()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .text_xl()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(colors.text)
                    .child("Custom Draw API (Instanced)"),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(colors.text_muted)
                    .child("Per-instance buffer for offsets + colors"),
            );

        let surface: Hsla = colors.surface.into();
        let content = if let Some(err) = &self.error {
            div()
                .text_sm()
                .text_color(colors.error)
                .child(format!("Custom draw unsupported: {err}"))
        } else if let (
            Some(pipeline),
            Some(vertex_buffer),
            Some(instance_buffer),
            Some(texture),
            Some(sampler),
        ) = (
            self.pipeline,
            self.vertex_buffer,
            self.instance_buffer,
            self.texture,
            self.sampler,
        ) {
            let start = self.start;
            let config = self.config;
            let prepaint = move |bounds: Bounds<_>, window: &mut Window, _cx: &mut App| {
                let layout_bounds = inset_bounds(bounds, px(config.bounds_inset_px));
                let viewport = window.viewport_size();
                let vertex_data =
                    quad_vertex_data_for_pixel_size(px(config.quad_size_px), viewport);
                if let Err(err) = window.update_custom_buffer(vertex_buffer, Arc::clone(&vertex_data)) {
                    log::error!("custom draw vertex update failed: {err}");
                }
                let instance_data =
                    instance_data_for_bounds(layout_bounds, viewport, config);
                if let Err(err) = window.update_custom_buffer(instance_buffer, Arc::clone(&instance_data))
                {
                    log::error!("custom draw instance update failed: {err}");
                }
                let elapsed = start.elapsed().as_secs_f32();
                let grid = (config.instances as f32).sqrt().ceil();
                let half_quad = config.quad_size_px * 0.5;
                let pad = px(config.grid_pad_px + half_quad);
                let inner_width = (layout_bounds.size.width - pad * 2.0).max(px(1.0));
                let inner_height = (layout_bounds.size.height - pad * 2.0).max(px(1.0));
                let margin_px = if grid > 1.0 {
                    let cell_w = inner_width / (grid - 1.0);
                    let cell_h = inner_height / (grid - 1.0);
                    let cell_min = cell_w.min(cell_h);
                    (cell_min - px(config.quad_size_px)) * 0.5
                } else {
                    px(0.0)
                }
                .max(px(0.0));
                let edge_margin = pad - px(half_quad);
                let max_wobble_px = margin_px.min(edge_margin).max(px(0.0));
                let amplitude = (f32::from(max_wobble_px) * 2.0 / f32::from(viewport.width))
                    .min(f32::from(max_wobble_px) * 2.0 / f32::from(viewport.height));
                let mut uniform = Vec::with_capacity(16);
                push_f32(&mut uniform, elapsed);
                push_f32(&mut uniform, amplitude);
                push_f32(&mut uniform, 0.0);
                push_f32(&mut uniform, 0.0);
                CustomDrawParams {
                    bounds,
                    pipeline,
                    vertex_buffers: vec![
                        CustomVertexBuffer {
                            source: CustomBufferSource::Buffer(vertex_buffer),
                        },
                        CustomVertexBuffer {
                            source: CustomBufferSource::Buffer(instance_buffer),
                        },
                    ],
                    vertex_count: 6,
                    instance_count: config.instances as u32,
                    bindings: vec![
                        CustomBindingValue::Texture(texture),
                        CustomBindingValue::Sampler(sampler),
                        CustomBindingValue::Uniform(CustomBufferSource::Inline(Arc::from(uniform))),
                    ],
                }
            };

            let paint =
                move |_bounds: Bounds<_>, params: CustomDrawParams, window: &mut Window, _cx: &mut App| {
                    if let Err(err) = window.paint_custom(params) {
                        log::error!("custom draw paint failed: {err}");
                    }
                };

            div()
                .w(px(420.))
                .h(px(420.))
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

fn push_f32(data: &mut Vec<u8>, value: f32) {
    data.extend_from_slice(&value.to_le_bytes());
}

fn inset_bounds(bounds: Bounds<gpui::Pixels>, inset: gpui::Pixels) -> Bounds<gpui::Pixels> {
    let width = (bounds.size.width - inset * 2.0).max(px(1.0));
    let height = (bounds.size.height - inset * 2.0).max(px(1.0));
    Bounds {
        origin: bounds.origin + gpui::Point::new(inset, inset),
        size: gpui::Size::new(width, height),
    }
}

fn quad_vertex_data() -> Arc<[u8]> {
    let mut data = Vec::with_capacity(6 * 4 * 4);
    let vertices = [
        (-0.08, -0.08, 0.0, 1.0),
        (0.08, -0.08, 1.0, 1.0),
        (0.08, 0.08, 1.0, 0.0),
        (-0.08, -0.08, 0.0, 1.0),
        (0.08, 0.08, 1.0, 0.0),
        (-0.08, 0.08, 0.0, 0.0),
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

fn instance_data_for_bounds(
    bounds: Bounds<gpui::Pixels>,
    viewport: gpui::Size<gpui::Pixels>,
    config: InstancedConfig,
) -> Arc<[u8]> {
    let mut data = Vec::with_capacity(config.instances * 24);
    let viewport_w = f32::from(viewport.width).max(1.0);
    let viewport_h = f32::from(viewport.height).max(1.0);

    let grid = (config.instances as f32).sqrt().ceil() as usize;
    let pad = px(config.grid_pad_px + config.quad_size_px * 0.5);
    let inner_width = (bounds.size.width - pad * 2.0).max(px(1.0));
    let inner_height = (bounds.size.height - pad * 2.0).max(px(1.0));

    for i in 0..config.instances {
        let gx = (i % grid) as f32;
        let gy = (i / grid) as f32;
        let fx = if grid > 1 { gx / (grid as f32 - 1.0) } else { 0.5 };
        let fy = if grid > 1 { gy / (grid as f32 - 1.0) } else { 0.5 };
        let px_x = bounds.origin.x + pad + inner_width * fx;
        let px_y = bounds.origin.y + pad + inner_height * fy;

        let ndc_x = (f32::from(px_x) / viewport_w) * 2.0 - 1.0;
        let ndc_y = 1.0 - (f32::from(px_y) / viewport_h) * 2.0;

        push_f32(&mut data, ndc_x);
        push_f32(&mut data, ndc_y);
        let t = i as f32 / (config.instances as f32);
        push_f32(&mut data, 0.4 + 0.6 * t);
        push_f32(&mut data, 0.8 - 0.5 * t);
        push_f32(&mut data, 1.0 - 0.3 * t);
        push_f32(&mut data, 1.0);
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
    let config = InstancedConfig::from_args();
    Application::new().run(move |cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(520.), px(520.)), cx);
        let config_copy = config;
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            move |_, cx| cx.new(|cx| InstancedCustomDrawExample::new(config_copy, cx)),
        )
        .expect("Failed to open window");

        example_prelude::init_example(cx, "Custom Draw API (Instanced)");
    });
}
