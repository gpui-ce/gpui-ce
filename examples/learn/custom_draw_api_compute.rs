//! Custom Draw API (Compute) Example
//!
//! Demonstrates dispatching a compute pipeline to populate a storage buffer
//! which is then consumed by a custom draw pipeline.

#[path = "../prelude.rs"]
mod example_prelude;

use std::sync::Arc;
use std::time::Instant;

use gpui::{
    App, AppContext, Application, Bounds, Colors, Context, CustomBindingDesc, CustomBindingKind,
    CustomBindingName, CustomBindingValue, CustomBufferDesc, CustomBufferId, CustomBufferSource,
    CustomComputeDispatch, CustomComputePipelineDesc, CustomComputePipelineId, CustomDrawParams,
    CustomPipelineDesc, CustomPipelineId, CustomPipelineState, CustomPrimitiveTopology,
    CustomUniformBuilder, Hsla, Render, Styled, Window, WindowBounds, WindowOptions, canvas, div,
    prelude::*, px, size,
};

const COMPUTE_SHADER_SOURCE: &str = r#"
struct Params {
  bounds: vec4<f32>,
  viewport: vec4<f32>,
};

var<storage, read_write> b0: array<vec2<f32>, 6>;
var<uniform> b1: Params;

@compute @workgroup_size(1)
fn cs_main(@builtin(global_invocation_id) id: vec3<u32>) {
  let index = id.x;
  if index >= 6u {
    return;
  }

  var pos: vec2<f32>;
  switch index {
    case 0u: { pos = vec2<f32>(-0.5, -0.5); }
    case 1u: { pos = vec2<f32>(0.5, -0.5); }
    case 2u: { pos = vec2<f32>(0.5, 0.5); }
    case 3u: { pos = vec2<f32>(-0.5, -0.5); }
    case 4u: { pos = vec2<f32>(0.5, 0.5); }
    default: { pos = vec2<f32>(-0.5, 0.5); }
  }

  b0[index] = pos * b1.viewport.z;
}
"#;

const DRAW_SHADER_SOURCE: &str = r#"
struct Params {
  bounds: vec4<f32>,
  viewport: vec4<f32>,
};

struct VertexOutput {
  @builtin(position) position: vec4<f32>,
};

var<storage, read> b0: array<vec2<f32>, 6>;
var<uniform> b1: Params;

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
  var out: VertexOutput;
  let local = b0[vertex_index];
  let origin = b1.bounds.xy;
  let size = b1.bounds.zw;
  let viewport = b1.viewport.xy;
  let pixel = origin + (local + vec2<f32>(0.5, 0.5)) * size;
  let ndc = vec2<f32>(
    (pixel.x / viewport.x) * 2.0 - 1.0,
    1.0 - (pixel.y / viewport.y) * 2.0
  );
  out.position = vec4<f32>(ndc, 0.0, 1.0);
  return out;
}

@fragment
fn fs_main() -> @location(0) vec4<f32> {
  return vec4<f32>(0.35, 0.75, 0.95, 1.0);
}
"#;

const VERTEX_COUNT: u32 = 6;

struct ComputeDrawExample {
    compute_pipeline: Option<CustomComputePipelineId>,
    render_pipeline: Option<CustomPipelineId>,
    positions_buffer: Option<CustomBufferId>,
    error: Option<String>,
    start: Instant,
}

struct ComputeFrame {
    compute: CustomComputeDispatch,
    draw: CustomDrawParams,
}

impl ComputeDrawExample {
    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            compute_pipeline: None,
            render_pipeline: None,
            positions_buffer: None,
            error: None,
            start: Instant::now(),
        }
    }

    fn ensure_resources(&mut self, window: &mut Window) {
        if self.compute_pipeline.is_some() || self.error.is_some() {
            return;
        }

        match self.build_resources(window) {
            Ok((compute_pipeline, render_pipeline, positions_buffer)) => {
                self.compute_pipeline = Some(compute_pipeline);
                self.render_pipeline = Some(render_pipeline);
                self.positions_buffer = Some(positions_buffer);
            }
            Err(err) => {
                self.error = Some(err.to_string());
            }
        }
    }

    fn build_resources(
        &self,
        window: &mut Window,
    ) -> anyhow::Result<(CustomComputePipelineId, CustomPipelineId, CustomBufferId)> {
        let compute_pipeline =
            window.create_custom_compute_pipeline(CustomComputePipelineDesc {
                name: "custom_draw_compute".to_string(),
                shader_source: COMPUTE_SHADER_SOURCE.to_string(),
                entry_point: "cs_main".to_string(),
                push_constants: None,
                bindings: vec![
                    CustomBindingDesc {
                        name: CustomBindingName::B0,
                        kind: CustomBindingKind::Buffer,
                        slot: None,
                    },
                    CustomBindingDesc {
                        name: CustomBindingName::B1,
                        kind: CustomBindingKind::Uniform { size: 32 },
                        slot: None,
                    },
                ],
            })?;

        let render_pipeline = window.create_custom_pipeline(CustomPipelineDesc {
            name: "custom_draw_compute_render".to_string(),
            shader_source: DRAW_SHADER_SOURCE.to_string(),
            vertex_entry: "vs_main".to_string(),
            fragment_entry: "fs_main".to_string(),
            vertex_fetches: Vec::new(),
            primitive: CustomPrimitiveTopology::TriangleList,
            color_targets: Vec::new(),
            state: CustomPipelineState::default(),
            push_constants: None,
            bindings: vec![
                CustomBindingDesc {
                    name: CustomBindingName::B0,
                    kind: CustomBindingKind::Buffer,
                    slot: None,
                },
                CustomBindingDesc {
                    name: CustomBindingName::B1,
                    kind: CustomBindingKind::Uniform { size: 32 },
                    slot: None,
                },
            ],
        })?;

        let positions_buffer = window.create_custom_buffer(CustomBufferDesc {
            name: "custom_draw_positions".to_string(),
            data: Arc::from(vec![0u8; VERTEX_COUNT as usize * 2 * 4]),
        })?;

        Ok((compute_pipeline, render_pipeline, positions_buffer))
    }
}

impl Render for ComputeDrawExample {
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
                    .child("Custom Draw API (Compute)"),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(colors.text_muted)
                    .child("Compute shader drives a storage buffer used by a draw pass"),
            );

        let surface: Hsla = colors.surface.into();
        let content = if let Some(err) = &self.error {
            div()
                .text_sm()
                .text_color(colors.error)
                .child(format!("Custom draw unsupported: {err}"))
        } else if let (Some(compute_pipeline), Some(render_pipeline), Some(positions_buffer)) = (
            self.compute_pipeline,
            self.render_pipeline,
            self.positions_buffer,
        ) {
            let start = self.start;
            let prepaint = move |bounds: Bounds<_>, window: &mut Window, _cx: &mut App| {
                let layout_bounds = inset_bounds(bounds, px(1.0));
                let viewport = window.viewport_size();
                let t = start.elapsed().as_secs_f32();
                let scale = 0.7 + 0.15 * t.sin();
                let uniform = build_uniform(layout_bounds, viewport, scale);
                ComputeFrame {
                    compute: CustomComputeDispatch {
                        pipeline: compute_pipeline,
                        push_constants: None,
                        bindings: vec![
                            CustomBindingValue::Buffer(CustomBufferSource::Buffer(
                                positions_buffer,
                            )),
                            CustomBindingValue::Uniform(CustomBufferSource::Inline(Arc::clone(
                                &uniform,
                            ))),
                        ],
                        workgroup_count: [VERTEX_COUNT, 1, 1],
                    },
                    draw: CustomDrawParams {
                        bounds: layout_bounds,
                        pipeline: render_pipeline,
                        vertex_buffers: Vec::new(),
                        vertex_count: VERTEX_COUNT,
                        index_buffer: None,
                        index_count: 0,
                        target: None,
                        instance_count: 1,
                        push_constants: None,
                        bindings: vec![
                            CustomBindingValue::Buffer(CustomBufferSource::Buffer(
                                positions_buffer,
                            )),
                            CustomBindingValue::Uniform(CustomBufferSource::Inline(Arc::clone(
                                &uniform,
                            ))),
                        ],
                    },
                }
            };

            let paint = move |_bounds: Bounds<_>,
                              params: ComputeFrame,
                              window: &mut Window,
                              _cx: &mut App| {
                if let Err(err) = window.dispatch_custom_compute(params.compute) {
                    log::error!("custom compute dispatch failed: {err}");
                }
                if let Err(err) = window.paint_custom(params.draw) {
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

fn inset_bounds(bounds: Bounds<gpui::Pixels>, inset: gpui::Pixels) -> Bounds<gpui::Pixels> {
    let width = (bounds.size.width - inset * 2.0).max(px(1.0));
    let height = (bounds.size.height - inset * 2.0).max(px(1.0));
    Bounds {
        origin: bounds.origin + gpui::Point::new(inset, inset),
        size: gpui::Size::new(width, height),
    }
}

fn build_uniform(
    bounds: Bounds<gpui::Pixels>,
    viewport: gpui::Size<gpui::Pixels>,
    scale: f32,
) -> Arc<[u8]> {
    let mut builder = CustomUniformBuilder::new();
    builder.push_vec4(
        f32::from(bounds.origin.x),
        f32::from(bounds.origin.y),
        f32::from(bounds.size.width),
        f32::from(bounds.size.height),
    );
    builder.push_vec4(
        f32::from(viewport.width).max(1.0),
        f32::from(viewport.height).max(1.0),
        scale,
        0.0,
    );
    builder.finish()
}

fn main() {
    Application::new().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(520.), px(520.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_, cx| cx.new(|cx| ComputeDrawExample::new(cx)),
        )
        .expect("Failed to open window");

        example_prelude::init_example(cx, "Custom Draw API (Compute)");
    });
}
