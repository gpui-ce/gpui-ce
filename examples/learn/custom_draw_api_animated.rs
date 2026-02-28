//! Custom Draw API (Animated) Example
//!
//! Demonstrates custom draw with a uniform binding that animates UVs.
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
};

struct VertexOutput {
  @builtin(position) position: vec4<f32>,
  @location(0) uv: vec2<f32>,
};

struct Uniforms {
  uv_offset: vec2<f32>,
  pad: vec2<f32>,
};

var b0: texture_2d<f32>;
var b1: sampler;
var<uniform> b2: Uniforms;

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
  var out: VertexOutput;
  out.position = vec4<f32>(input.a0, 0.0, 1.0);
  out.uv = input.a1;
  return out;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
  let uv = input.uv + b2.uv_offset;
  return textureSample(b0, b1, uv);
}
"#;

struct AnimatedCustomDrawExample {
    pipeline: Option<CustomPipelineId>,
    vertex_buffer: Option<CustomBufferId>,
    texture: Option<CustomTextureId>,
    sampler: Option<CustomSamplerId>,
    start: Instant,
    error: Option<String>,
}

impl AnimatedCustomDrawExample {
    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            pipeline: None,
            vertex_buffer: None,
            texture: None,
            sampler: None,
            start: Instant::now(),
            error: None,
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
            name: "custom_draw_demo_animated".to_string(),
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
                        },
                        CustomVertexAttribute {
                            name: CustomVertexAttributeName::A1,
                            offset: 8,
                            format: CustomVertexFormat::F32Vec2,
                        },
                    ],
                },
                instanced: false,
            }],
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

        let vertex_data = quad_vertex_data();
        let vertex_buffer = window.create_custom_buffer(CustomBufferDesc {
            name: "quad_vertices_animated".to_string(),
            data: vertex_data,
        })?;

        let texture_data = checker_texture_data();
        let texture = window.create_custom_texture(CustomTextureDesc {
            name: "checker_texture_animated".to_string(),
            width: 2,
            height: 2,
            format: CustomTextureFormat::Rgba8Unorm,
            data: texture_data,
        })?;

        let sampler = window.create_custom_sampler(CustomSamplerDesc {
            name: "checker_sampler_animated".to_string(),
            min_filter: CustomFilterMode::Nearest,
            mag_filter: CustomFilterMode::Nearest,
            mipmap_filter: CustomFilterMode::Nearest,
            address_modes: [CustomAddressMode::Repeat; 3],
        })?;

        Ok((pipeline, vertex_buffer, texture, sampler))
    }

    fn uniform_bytes(&self) -> Arc<[u8]> {
        let t = self.start.elapsed().as_secs_f32();
        let offset = 0.05 * t.sin();
        let mut data = Vec::with_capacity(16);
        push_f32(&mut data, offset);
        push_f32(&mut data, offset);
        push_f32(&mut data, 0.0);
        push_f32(&mut data, 0.0);
        Arc::from(data)
    }
}

impl Render for AnimatedCustomDrawExample {
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
                    .child("Custom Draw API (Animated)"),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(colors.text_muted)
                    .child("Uniform binding animates UVs"),
            );

        let surface: Hsla = colors.surface.into();
        let content = if let Some(err) = &self.error {
            div()
                .text_sm()
                .text_color(colors.error)
                .child(format!("Custom draw unsupported: {err}"))
        } else if let (Some(pipeline), Some(buffer), Some(texture), Some(sampler)) = (
            self.pipeline,
            self.vertex_buffer,
            self.texture,
            self.sampler,
        ) {
            let uniform = self.uniform_bytes();
            let prepaint = move |bounds: Bounds<_>, window: &mut Window, _cx: &mut App| {
                let vertex_data = quad_vertex_data_for_bounds(bounds, window.viewport_size());
                if let Err(err) = window.update_custom_buffer(buffer, Arc::clone(&vertex_data)) {
                    log::error!("custom draw vertex update failed: {err}");
                }
                CustomDrawParams {
                    bounds,
                    pipeline,
                    vertex_buffers: vec![CustomVertexBuffer {
                        source: CustomBufferSource::Buffer(buffer),
                    }],
                    vertex_count: 6,
                    instance_count: 1,
                    bindings: vec![
                        CustomBindingValue::Texture(texture),
                        CustomBindingValue::Sampler(sampler),
                        CustomBindingValue::Uniform(CustomBufferSource::Inline(Arc::clone(
                            &uniform,
                        ))),
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

fn quad_vertex_data() -> Arc<[u8]> {
    let mut data = Vec::with_capacity(6 * 4 * 4);
    let vertices = [
        (-0.6, -0.6, 0.0, 1.0),
        (0.6, -0.6, 1.0, 1.0),
        (0.6, 0.6, 1.0, 0.0),
        (-0.6, -0.6, 0.0, 1.0),
        (0.6, 0.6, 1.0, 0.0),
        (-0.6, 0.6, 0.0, 0.0),
    ];

    for (x, y, u, v) in vertices {
        push_f32(&mut data, x);
        push_f32(&mut data, y);
        push_f32(&mut data, u);
        push_f32(&mut data, v);
    }

    Arc::from(data)
}

fn quad_vertex_data_for_bounds(
    bounds: Bounds<gpui::Pixels>,
    viewport: gpui::Size<gpui::Pixels>,
) -> Arc<[u8]> {
    let mut data = Vec::with_capacity(6 * 4 * 4);
    let left = bounds.origin.x;
    let top = bounds.origin.y;
    let right = bounds.origin.x + bounds.size.width;
    let bottom = bounds.origin.y + bounds.size.height;

    let viewport_w = f32::from(viewport.width).max(1.0);
    let viewport_h = f32::from(viewport.height).max(1.0);

    let to_ndc_x = |x: gpui::Pixels| (f32::from(x) / viewport_w) * 2.0 - 1.0;
    let to_ndc_y = |y: gpui::Pixels| 1.0 - (f32::from(y) / viewport_h) * 2.0;

    let left_ndc = to_ndc_x(left);
    let right_ndc = to_ndc_x(right);
    let top_ndc = to_ndc_y(top);
    let bottom_ndc = to_ndc_y(bottom);

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
    Application::new().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(520.), px(520.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_, cx| cx.new(|cx| AnimatedCustomDrawExample::new(cx)),
        )
        .expect("Failed to open window");

        example_prelude::init_example(cx, "Custom Draw API (Animated)");
    });
}
