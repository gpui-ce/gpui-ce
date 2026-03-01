//! Custom Draw API (Multi-Group) Example
//!
//! Demonstrates multiple explicit bind groups alongside implicit b0/b1 bindings.

#[path = "../prelude.rs"]
mod example_prelude;

use std::sync::Arc;

use gpui::{
    App, AppContext, Application, Bounds, Colors, Context, CustomAddressMode, CustomBindingDesc,
    CustomBindingKind, CustomBindingName, CustomBindingValue, CustomBufferDesc, CustomBufferId,
    CustomBufferSource, CustomDrawParams, CustomFilterMode, CustomPipelineDesc, CustomPipelineId,
    CustomPipelineState, CustomPrimitiveTopology, CustomSamplerDesc, CustomSamplerId,
    CustomTextureDesc, CustomTextureFormat, CustomTextureId, CustomTextureUsage,
    CustomVertexAttribute, CustomVertexAttributeName, CustomVertexBuffer, CustomVertexFetch,
    CustomVertexFormat, CustomVertexLayout, Hsla, Render, Styled, Window, WindowBounds,
    WindowOptions, canvas, div, prelude::*, px, size,
};

const SHADER_SOURCE: &str = r#"
struct VertexInput {
  @location(0) position: vec2<f32>,
  a1: vec2<f32>,
};

struct VertexOutput {
  @builtin(position) position: vec4<f32>,
  @location(0) uv: vec2<f32>,
};

struct Transform {
  transform: mat4x4<f32>,
};

@group(1) @binding(0) var<uniform> u: Transform;
@group(2) @binding(1) var<uniform> tint: vec4<f32>;
var b0: texture_2d<f32>;
var b1: sampler;

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
  var out: VertexOutput;
  out.position = u.transform * vec4<f32>(input.position, 0.0, 1.0);
  out.uv = input.a1;
  return out;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
  return textureSample(b0, b1, input.uv) * tint;
}
"#;

struct MultiGroupExample {
    pipeline: Option<CustomPipelineId>,
    vertex_buffer: Option<CustomBufferId>,
    texture: Option<CustomTextureId>,
    sampler: Option<CustomSamplerId>,
    error: Option<String>,
}

const SPARSE_SLOT: u32 = 1;
const BOUNDS_INSET_PX: f32 = 1.0;

impl MultiGroupExample {
    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            pipeline: None,
            vertex_buffer: None,
            texture: None,
            sampler: None,
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
            name: "custom_draw_multi_group".to_string(),
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
                            location: Some(0),
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
            push_constants: None,
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
                    kind: CustomBindingKind::Uniform { size: 64 },
                    slot: Some(gpui::CustomBindingSlot {
                        group: 1,
                        binding: 0,
                    }),
                },
                CustomBindingDesc {
                    name: CustomBindingName::B3,
                    kind: CustomBindingKind::Uniform { size: 16 },
                    slot: Some(gpui::CustomBindingSlot {
                        group: 2,
                        binding: SPARSE_SLOT,
                    }),
                },
            ],
        })?;

        let vertex_buffer = window.create_custom_buffer(CustomBufferDesc {
            name: "multi_group_vertices".to_string(),
            data: quad_vertex_data(),
        })?;

        let texture = window.create_custom_texture(CustomTextureDesc {
            name: "multi_group_texture".to_string(),
            width: 2,
            height: 2,
            format: CustomTextureFormat::Rgba8Unorm,
            usage: CustomTextureUsage::SAMPLED,
            data: vec![checker_texture_data()],
        })?;

        let sampler = window.create_custom_sampler(CustomSamplerDesc {
            name: "multi_group_sampler".to_string(),
            min_filter: CustomFilterMode::Nearest,
            mag_filter: CustomFilterMode::Nearest,
            mipmap_filter: CustomFilterMode::Nearest,
            address_modes: [CustomAddressMode::ClampToEdge; 3],
        })?;

        Ok((pipeline, vertex_buffer, texture, sampler))
    }
}

impl Render for MultiGroupExample {
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
                    .child("Custom Draw API (Multi-Group)"),
            )
            .child(div().text_sm().text_color(colors.text_muted).child(format!(
                "Explicit @group(1/2) uniforms + implicit b0/b1 (sparse binding {})",
                SPARSE_SLOT
            )));

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
            let prepaint = move |bounds: Bounds<_>, window: &mut Window, _cx: &mut App| {
                let layout_bounds = inset_bounds(bounds, px(BOUNDS_INSET_PX));
                let viewport = window.viewport_size();
                let vertex_data = quad_vertex_data_for_bounds(layout_bounds, viewport);
                if let Err(err) = window.update_custom_buffer(buffer, Arc::clone(&vertex_data)) {
                    log::error!("custom draw vertex update failed: {err}");
                }
                CustomDrawParams {
                    bounds: layout_bounds,
                    pipeline,
                    vertex_buffers: vec![CustomVertexBuffer {
                        source: CustomBufferSource::Buffer(buffer),
                    }],
                    vertex_count: 6,
                    index_buffer: None,
                    index_count: 0,
                    target: None,
                    instance_count: 1,
                    push_constants: None,
                    bindings: vec![
                        CustomBindingValue::Texture(texture),
                        CustomBindingValue::Sampler(sampler),
                        CustomBindingValue::Uniform(CustomBufferSource::Inline(
                            identity_transform(),
                        )),
                        CustomBindingValue::Uniform(CustomBufferSource::Inline(tint_uniform())),
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

fn inset_bounds(bounds: Bounds<gpui::Pixels>, inset: gpui::Pixels) -> Bounds<gpui::Pixels> {
    let width = (bounds.size.width - inset * 2.0).max(px(1.0));
    let height = (bounds.size.height - inset * 2.0).max(px(1.0));
    Bounds {
        origin: bounds.origin + gpui::Point::new(inset, inset),
        size: gpui::Size::new(width, height),
    }
}

fn quad_vertex_data_for_bounds(
    bounds: Bounds<gpui::Pixels>,
    viewport: gpui::Size<gpui::Pixels>,
) -> Arc<[u8]> {
    let mut data = Vec::with_capacity(6 * 4 * 4);
    let viewport_w = f32::from(viewport.width).max(1.0);
    let viewport_h = f32::from(viewport.height).max(1.0);

    let left_px = f32::from(bounds.origin.x);
    let top_px = f32::from(bounds.origin.y);
    let right_px = left_px + f32::from(bounds.size.width);
    let bottom_px = top_px + f32::from(bounds.size.height);

    let left_ndc = (left_px / viewport_w) * 2.0 - 1.0;
    let right_ndc = (right_px / viewport_w) * 2.0 - 1.0;
    let top_ndc = 1.0 - (top_px / viewport_h) * 2.0;
    let bottom_ndc = 1.0 - (bottom_px / viewport_h) * 2.0;

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

fn identity_transform() -> Arc<[u8]> {
    let mut data = Vec::with_capacity(16 * 4);
    let values = [
        1.0f32, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ];
    for value in values {
        push_f32(&mut data, value);
    }
    Arc::from(data)
}

fn tint_uniform() -> Arc<[u8]> {
    let mut data = Vec::with_capacity(4 * 4);
    let values = [0.85f32, 0.95, 1.0, 1.0];
    for value in values {
        push_f32(&mut data, value);
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
            |_, cx| cx.new(|cx| MultiGroupExample::new(cx)),
        )
        .expect("Failed to open window");

        example_prelude::init_example(cx, "Custom Draw API (Multi-Group)");
    });
}
