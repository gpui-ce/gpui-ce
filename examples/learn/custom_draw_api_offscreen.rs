//! Custom Draw Offscreen Example
//!
//! Demonstrates offscreen render targets + depth testing, then samples the
//! target onto the main window surface.

#[path = "../prelude.rs"]
mod example_prelude;

use std::sync::Arc;

use gpui::{
    App, AppContext, Application, Bounds, Colors, Context, CustomAddressMode, CustomBindingDesc,
    CustomBindingKind, CustomBindingName, CustomBindingValue, CustomBlendMode, CustomBufferDesc,
    CustomBufferId, CustomBufferSource, CustomDepthCompare, CustomDepthFormat, CustomDepthState,
    CustomDepthTargetDesc, CustomDepthTargetId, CustomDrawParams, CustomFilterMode,
    CustomIndexBuffer, CustomIndexFormat, CustomPipelineDesc, CustomPipelineId,
    CustomPipelineState, CustomPrimitiveTopology, CustomRenderTarget, CustomRenderTargetDesc,
    CustomSamplerDesc, CustomSamplerId, CustomTextureFormat, CustomTextureId,
    CustomVertexAttribute, CustomVertexAttributeName, CustomVertexBuffer, CustomVertexFetch,
    CustomVertexFormat, CustomVertexLayout, Hsla, Render, Styled, Window, WindowBounds,
    WindowOptions, canvas, div, prelude::*, px, size,
};

const OFFSCREEN_SHADER_SOURCE: &str = r#"
struct VertexInput {
  a0: vec3<f32>,
  a1: vec3<f32>,
};

struct VertexOutput {
  @builtin(position) position: vec4<f32>,
  @location(0) color: vec3<f32>,
};

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
  var out: VertexOutput;
  out.position = vec4<f32>(input.a0, 1.0);
  out.color = input.a1;
  return out;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
  return vec4<f32>(input.color, 1.0);
}
"#;

const BLIT_SHADER_SOURCE: &str = r#"
struct VertexInput {
  a0: vec2<f32>,
  a1: vec2<f32>,
};

struct VertexOutput {
  @builtin(position) position: vec4<f32>,
  @location(0) uv: vec2<f32>,
};

var b0: texture_2d<f32>;
var b1: sampler;

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
  var out: VertexOutput;
  out.position = vec4<f32>(input.a0, 0.0, 1.0);
  out.uv = input.a1;
  return out;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
  return textureSample(b0, b1, input.uv);
}
"#;

struct OffscreenCustomDrawExample {
    offscreen_pipeline: Option<CustomPipelineId>,
    onscreen_pipeline: Option<CustomPipelineId>,
    offscreen_vertex_buffer: Option<CustomBufferId>,
    offscreen_index_buffer: Option<CustomBufferId>,
    onscreen_vertex_buffer: Option<CustomBufferId>,
    render_target: Option<CustomTextureId>,
    depth_target: Option<CustomDepthTargetId>,
    sampler: Option<CustomSamplerId>,
    error: Option<String>,
}

impl OffscreenCustomDrawExample {
    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            offscreen_pipeline: None,
            onscreen_pipeline: None,
            offscreen_vertex_buffer: None,
            offscreen_index_buffer: None,
            onscreen_vertex_buffer: None,
            render_target: None,
            depth_target: None,
            sampler: None,
            error: None,
        }
    }

    fn ensure_resources(&mut self, window: &mut Window) {
        if self.offscreen_pipeline.is_some() || self.error.is_some() {
            return;
        }

        match self.build_resources(window) {
            Ok((
                offscreen_pipeline,
                onscreen_pipeline,
                offscreen_vertex_buffer,
                offscreen_index_buffer,
                onscreen_vertex_buffer,
                render_target,
                depth_target,
                sampler,
            )) => {
                self.offscreen_pipeline = Some(offscreen_pipeline);
                self.onscreen_pipeline = Some(onscreen_pipeline);
                self.offscreen_vertex_buffer = Some(offscreen_vertex_buffer);
                self.offscreen_index_buffer = Some(offscreen_index_buffer);
                self.onscreen_vertex_buffer = Some(onscreen_vertex_buffer);
                self.render_target = Some(render_target);
                self.depth_target = Some(depth_target);
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
        CustomPipelineId,
        CustomBufferId,
        CustomBufferId,
        CustomBufferId,
        CustomTextureId,
        CustomDepthTargetId,
        CustomSamplerId,
    )> {
        let target_format = CustomTextureFormat::Rgba8Unorm;
        let target_width = 256;
        let target_height = 256;

        let render_target = window.create_custom_render_target(CustomRenderTargetDesc {
            name: "custom_draw_offscreen_color".to_string(),
            width: target_width,
            height: target_height,
            format: target_format,
            clear_color: Some([0.08, 0.08, 0.1, 1.0]),
        })?;

        let depth_target = window.create_custom_depth_target(CustomDepthTargetDesc {
            name: "custom_draw_offscreen_depth".to_string(),
            width: target_width,
            height: target_height,
            format: CustomDepthFormat::Depth32Float,
            clear_depth: Some(1.0),
        })?;

        let offscreen_pipeline = window.create_custom_pipeline(CustomPipelineDesc {
            name: "custom_draw_offscreen_pipeline".to_string(),
            shader_source: OFFSCREEN_SHADER_SOURCE.to_string(),
            vertex_entry: "vs_main".to_string(),
            fragment_entry: "fs_main".to_string(),
            vertex_fetches: vec![CustomVertexFetch {
                layout: CustomVertexLayout {
                    stride: 24,
                    attributes: vec![
                        CustomVertexAttribute {
                            name: CustomVertexAttributeName::A0,
                            offset: 0,
                            format: CustomVertexFormat::F32Vec3,
                            location: None,
                        },
                        CustomVertexAttribute {
                            name: CustomVertexAttributeName::A1,
                            offset: 12,
                            format: CustomVertexFormat::F32Vec3,
                            location: None,
                        },
                    ],
                },
                instanced: false,
            }],
            primitive: CustomPrimitiveTopology::TriangleList,
            target_format: Some(target_format),
            state: CustomPipelineState {
                blend: CustomBlendMode::Opaque,
                depth: Some(CustomDepthState {
                    format: CustomDepthFormat::Depth32Float,
                    compare: CustomDepthCompare::LessEqual,
                    write_enabled: true,
                }),
                ..CustomPipelineState::default()
            },
            bindings: Vec::new(),
        })?;

        let onscreen_pipeline = window.create_custom_pipeline(CustomPipelineDesc {
            name: "custom_draw_offscreen_blit".to_string(),
            shader_source: BLIT_SHADER_SOURCE.to_string(),
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
            ],
        })?;

        let offscreen_vertex_buffer = window.create_custom_buffer(CustomBufferDesc {
            name: "custom_draw_offscreen_vertices".to_string(),
            data: offscreen_vertex_data(),
        })?;

        let offscreen_index_buffer = window.create_custom_buffer(CustomBufferDesc {
            name: "custom_draw_offscreen_indices".to_string(),
            data: offscreen_index_data(),
        })?;

        let onscreen_vertex_buffer = window.create_custom_buffer(CustomBufferDesc {
            name: "custom_draw_offscreen_screen_vertices".to_string(),
            data: onscreen_vertex_placeholder(),
        })?;

        let sampler = window.create_custom_sampler(CustomSamplerDesc {
            name: "custom_draw_offscreen_sampler".to_string(),
            min_filter: CustomFilterMode::Linear,
            mag_filter: CustomFilterMode::Linear,
            mipmap_filter: CustomFilterMode::Nearest,
            address_modes: [CustomAddressMode::ClampToEdge; 3],
        })?;

        Ok((
            offscreen_pipeline,
            onscreen_pipeline,
            offscreen_vertex_buffer,
            offscreen_index_buffer,
            onscreen_vertex_buffer,
            render_target,
            depth_target,
            sampler,
        ))
    }
}

impl Render for OffscreenCustomDrawExample {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl gpui::IntoElement {
        let colors = Colors::for_appearance(window);
        self.ensure_resources(window);

        let header = div()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .text_xl()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(colors.text)
                    .child("Custom Draw Offscreen"),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(colors.text_muted)
                    .child("Offscreen render target + depth test, then blit to the window"),
            );

        let surface: Hsla = colors.surface.into();
        let content = if let Some(err) = &self.error {
            div()
                .text_sm()
                .text_color(colors.error)
                .child(format!("Custom draw unsupported: {err}"))
        } else if let (
            Some(offscreen_pipeline),
            Some(onscreen_pipeline),
            Some(offscreen_vertex_buffer),
            Some(offscreen_index_buffer),
            Some(onscreen_vertex_buffer),
            Some(render_target),
            Some(depth_target),
            Some(sampler),
        ) = (
            self.offscreen_pipeline,
            self.onscreen_pipeline,
            self.offscreen_vertex_buffer,
            self.offscreen_index_buffer,
            self.onscreen_vertex_buffer,
            self.render_target,
            self.depth_target,
            self.sampler,
        ) {
            let prepaint = move |bounds: Bounds<_>, window: &mut Window, _cx: &mut App| {
                let vertex_data = onscreen_vertex_data_for_bounds(bounds, window.viewport_size());
                if let Err(err) =
                    window.update_custom_buffer(onscreen_vertex_buffer, Arc::clone(&vertex_data))
                {
                    log::error!("custom draw vertex update failed: {err}");
                }

                let target = CustomRenderTarget {
                    color: render_target,
                    depth: Some(depth_target),
                };

                vec![
                    CustomDrawParams {
                        bounds,
                        pipeline: offscreen_pipeline,
                        vertex_buffers: vec![CustomVertexBuffer {
                            source: CustomBufferSource::Buffer(offscreen_vertex_buffer),
                        }],
                        vertex_count: 8,
                        index_buffer: Some(CustomIndexBuffer {
                            source: CustomBufferSource::Buffer(offscreen_index_buffer),
                            format: CustomIndexFormat::U16,
                        }),
                        index_count: 12,
                        target: Some(target),
                        instance_count: 1,
                        bindings: Vec::new(),
                    },
                    CustomDrawParams {
                        bounds,
                        pipeline: onscreen_pipeline,
                        vertex_buffers: vec![CustomVertexBuffer {
                            source: CustomBufferSource::Buffer(onscreen_vertex_buffer),
                        }],
                        vertex_count: 6,
                        index_buffer: None,
                        index_count: 0,
                        target: None,
                        instance_count: 1,
                        bindings: vec![
                            CustomBindingValue::Texture(render_target),
                            CustomBindingValue::Sampler(sampler),
                        ],
                    },
                ]
            };

            let paint = move |_bounds: Bounds<_>,
                              params: Vec<CustomDrawParams>,
                              window: &mut Window,
                              _cx: &mut App| {
                for params in params {
                    if let Err(err) = window.paint_custom(params) {
                        log::error!("custom draw paint failed: {err}");
                    }
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

fn push_u16(data: &mut Vec<u8>, value: u16) {
    data.extend_from_slice(&value.to_le_bytes());
}

fn offscreen_vertex_data() -> Arc<[u8]> {
    let mut data = Vec::with_capacity(8 * 6 * 4);
    let vertices = [
        (-0.6, -0.6, -0.3, 1.0, 0.3, 0.25),
        (0.6, -0.6, -0.3, 1.0, 0.3, 0.25),
        (0.6, 0.6, -0.3, 1.0, 0.3, 0.25),
        (-0.6, 0.6, -0.3, 1.0, 0.3, 0.25),
        (-0.2, -0.2, 0.3, 0.25, 0.45, 1.0),
        (0.8, -0.2, 0.3, 0.25, 0.45, 1.0),
        (0.8, 0.8, 0.3, 0.25, 0.45, 1.0),
        (-0.2, 0.8, 0.3, 0.25, 0.45, 1.0),
    ];

    for (x, y, z, r, g, b) in vertices {
        push_f32(&mut data, x);
        push_f32(&mut data, y);
        push_f32(&mut data, z);
        push_f32(&mut data, r);
        push_f32(&mut data, g);
        push_f32(&mut data, b);
    }

    Arc::from(data)
}

fn offscreen_index_data() -> Arc<[u8]> {
    let mut data = Vec::with_capacity(12 * 2);
    let indices: [u16; 12] = [0, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7];

    for index in indices {
        push_u16(&mut data, index);
    }

    Arc::from(data)
}

fn onscreen_vertex_placeholder() -> Arc<[u8]> {
    Arc::from(vec![0u8; 6 * 4 * 4])
}

fn onscreen_vertex_data_for_bounds(
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

fn main() {
    Application::new().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(520.), px(520.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_, cx| cx.new(|cx| OffscreenCustomDrawExample::new(cx)),
        )
        .expect("Failed to open window");

        example_prelude::init_example(cx, "Custom Draw Offscreen");
    });
}
