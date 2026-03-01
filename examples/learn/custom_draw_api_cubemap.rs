//! Custom Draw API (Cubemap) Example
//!
//! Demonstrates sampling from a cubemap texture.

#[path = "../prelude.rs"]
mod example_prelude;

use std::sync::Arc;

use gpui::{
    App, AppContext, Application, Bounds, Colors, Context, CustomAddressMode, CustomBindingDesc,
    CustomBindingKind, CustomBindingName, CustomBindingValue, CustomBufferDesc, CustomBufferId,
    CustomBufferSource, CustomDrawParams, CustomFilterMode, CustomPipelineDesc, CustomPipelineId,
    CustomPipelineState, CustomPrimitiveTopology, CustomSamplerDesc, CustomSamplerId,
    CustomTextureDesc, CustomTextureDimension, CustomTextureFormat, CustomTextureId,
    CustomTextureUsage, CustomVertexAttribute, CustomVertexAttributeName, CustomVertexBuffer,
    CustomVertexFetch, CustomVertexFormat, CustomVertexLayout, Hsla, Render, Styled, Window,
    WindowBounds, WindowOptions, canvas, div, prelude::*, px, size,
};

const SHADER_SOURCE: &str = r#"
struct VertexInput {
  a0: vec2<f32>,
  a1: vec3<f32>,
};

struct VertexOutput {
  @builtin(position) position: vec4<f32>,
  @location(0) direction: vec3<f32>,
};

var b0: texture_cube<f32>;
var b1: sampler;

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
  var out: VertexOutput;
  out.position = vec4<f32>(input.a0, 0.0, 1.0);
  out.direction = input.a1;
  return out;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
  return textureSample(b0, b1, normalize(input.direction));
}
"#;

struct CubemapExample {
    pipeline: Option<CustomPipelineId>,
    vertex_buffer: Option<CustomBufferId>,
    texture: Option<CustomTextureId>,
    sampler: Option<CustomSamplerId>,
    error: Option<String>,
}

impl CubemapExample {
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
            name: "custom_draw_cubemap".to_string(),
            shader_source: SHADER_SOURCE.to_string(),
            vertex_entry: "vs_main".to_string(),
            fragment_entry: "fs_main".to_string(),
            vertex_fetches: vec![CustomVertexFetch {
                layout: CustomVertexLayout {
                    stride: 20,
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
                            format: CustomVertexFormat::F32Vec3,
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
            ],
        })?;

        let vertex_buffer = window.create_custom_buffer(CustomBufferDesc {
            name: "cubemap_vertices".to_string(),
            data: cubemap_vertex_data(),
        })?;

        let texture = window.create_custom_texture(CustomTextureDesc {
            name: "cubemap_texture".to_string(),
            dimension: CustomTextureDimension::Cube,
            width: 2,
            height: 2,
            format: CustomTextureFormat::Rgba8Unorm,
            usage: CustomTextureUsage::SAMPLED,
            data: vec![cubemap_texture_data()],
        })?;

        let sampler = window.create_custom_sampler(CustomSamplerDesc {
            name: "cubemap_sampler".to_string(),
            min_filter: CustomFilterMode::Nearest,
            mag_filter: CustomFilterMode::Nearest,
            mipmap_filter: CustomFilterMode::Nearest,
            address_modes: [CustomAddressMode::ClampToEdge; 3],
        })?;

        Ok((pipeline, vertex_buffer, texture, sampler))
    }
}

impl Render for CubemapExample {
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
                    .child("Custom Draw API (Cubemap)"),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(colors.text_muted)
                    .child("Samples the six cubemap faces"),
            );

        let surface: Hsla = colors.surface.into();
        let content = if let (Some(pipeline), Some(buffer), Some(texture), Some(sampler)) = (
            self.pipeline,
            self.vertex_buffer,
            self.texture,
            self.sampler,
        ) {
            let prepaint = move |bounds: Bounds<_>, window: &mut Window, _cx: &mut App| {
                let vertex_data = cubemap_vertex_data_for_bounds(bounds, window.viewport_size());
                if let Err(err) = window.update_custom_buffer(buffer, Arc::clone(&vertex_data)) {
                    log::error!("custom draw vertex update failed: {err}");
                }
                CustomDrawParams {
                    bounds,
                    pipeline,
                    vertex_buffers: vec![CustomVertexBuffer {
                        source: CustomBufferSource::Buffer(buffer),
                    }],
                    vertex_count: 36,
                    index_buffer: None,
                    index_count: 0,
                    target: None,
                    instance_count: 1,
                    push_constants: None,
                    bindings: vec![
                        CustomBindingValue::Texture(texture),
                        CustomBindingValue::Sampler(sampler),
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
        } else if let Some(err) = &self.error {
            div()
                .text_sm()
                .text_color(colors.text_muted)
                .child(err.clone())
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

fn append_quad_vertices(
    data: &mut Vec<u8>,
    left: f32,
    right: f32,
    top: f32,
    bottom: f32,
    direction: [f32; 3],
) {
    let vertices = [
        (left, top),
        (right, top),
        (right, bottom),
        (left, top),
        (right, bottom),
        (left, bottom),
    ];

    for (x, y) in vertices {
        push_f32(data, x);
        push_f32(data, y);
        push_f32(data, direction[0]);
        push_f32(data, direction[1]);
        push_f32(data, direction[2]);
    }
}

fn cubemap_vertex_data() -> Arc<[u8]> {
    let mut data = Vec::with_capacity(36 * 5 * 4);
    append_quad_vertices(&mut data, -0.95, -0.35, 0.85, 0.15, [1.0, 0.0, 0.0]);
    append_quad_vertices(&mut data, -0.25, 0.35, 0.85, 0.15, [-1.0, 0.0, 0.0]);
    append_quad_vertices(&mut data, 0.45, 0.95, 0.85, 0.15, [0.0, 1.0, 0.0]);
    append_quad_vertices(&mut data, -0.95, -0.35, -0.15, -0.85, [0.0, -1.0, 0.0]);
    append_quad_vertices(&mut data, -0.25, 0.35, -0.15, -0.85, [0.0, 0.0, 1.0]);
    append_quad_vertices(&mut data, 0.45, 0.95, -0.15, -0.85, [0.0, 0.0, -1.0]);
    Arc::from(data)
}

fn cubemap_vertex_data_for_bounds(
    bounds: Bounds<gpui::Pixels>,
    viewport: gpui::Size<gpui::Pixels>,
) -> Arc<[u8]> {
    let left = f32::from(bounds.origin.x);
    let top = f32::from(bounds.origin.y);
    let right = f32::from(bounds.origin.x + bounds.size.width);
    let bottom = f32::from(bounds.origin.y + bounds.size.height);

    let total_width = (right - left).max(1.0);
    let total_height = (bottom - top).max(1.0);
    let gap_x = total_width * 0.04;
    let gap_y = total_height * 0.04;
    let cell_width = ((total_width - gap_x * 2.0) / 3.0).max(1.0);
    let cell_height = ((total_height - gap_y) / 2.0).max(1.0);

    let viewport_w = f32::from(viewport.width).max(1.0);
    let viewport_h = f32::from(viewport.height).max(1.0);

    let to_ndc_x = |x: f32| (x / viewport_w) * 2.0 - 1.0;
    let to_ndc_y = |y: f32| 1.0 - (y / viewport_h) * 2.0;

    let mut data = Vec::with_capacity(36 * 5 * 4);
    let faces = [
        ([1.0, 0.0, 0.0], 0.0, 0.0),
        ([-1.0, 0.0, 0.0], 1.0, 0.0),
        ([0.0, 1.0, 0.0], 2.0, 0.0),
        ([0.0, -1.0, 0.0], 0.0, 1.0),
        ([0.0, 0.0, 1.0], 1.0, 1.0),
        ([0.0, 0.0, -1.0], 2.0, 1.0),
    ];

    for (direction, column, row) in faces {
        let left_px = left + column * (cell_width + gap_x);
        let right_px = left_px + cell_width;
        let top_px = top + row * (cell_height + gap_y);
        let bottom_px = top_px + cell_height;

        let left_ndc = to_ndc_x(left_px);
        let right_ndc = to_ndc_x(right_px);
        let top_ndc = to_ndc_y(top_px);
        let bottom_ndc = to_ndc_y(bottom_px);

        append_quad_vertices(
            &mut data, left_ndc, right_ndc, top_ndc, bottom_ndc, direction,
        );
    }

    Arc::from(data)
}

fn cubemap_texture_data() -> Arc<[u8]> {
    let faces: [[u8; 16]; 6] = [
        [
            0xff, 0x6b, 0x6b, 0xff, 0xff, 0x6b, 0x6b, 0xff, 0xff, 0x6b, 0x6b, 0xff, 0xff, 0x6b,
            0x6b, 0xff,
        ],
        [
            0x6b, 0xff, 0x95, 0xff, 0x6b, 0xff, 0x95, 0xff, 0x6b, 0xff, 0x95, 0xff, 0x6b, 0xff,
            0x95, 0xff,
        ],
        [
            0x6b, 0xc2, 0xff, 0xff, 0x6b, 0xc2, 0xff, 0xff, 0x6b, 0xc2, 0xff, 0xff, 0x6b, 0xc2,
            0xff, 0xff,
        ],
        [
            0xf2, 0xd9, 0x6b, 0xff, 0xf2, 0xd9, 0x6b, 0xff, 0xf2, 0xd9, 0x6b, 0xff, 0xf2, 0xd9,
            0x6b, 0xff,
        ],
        [
            0xc5, 0x6b, 0xff, 0xff, 0xc5, 0x6b, 0xff, 0xff, 0xc5, 0x6b, 0xff, 0xff, 0xc5, 0x6b,
            0xff, 0xff,
        ],
        [
            0x6b, 0xff, 0xf0, 0xff, 0x6b, 0xff, 0xf0, 0xff, 0x6b, 0xff, 0xf0, 0xff, 0x6b, 0xff,
            0xf0, 0xff,
        ],
    ];

    let mut data = Vec::with_capacity(16 * faces.len());
    for face in faces {
        data.extend_from_slice(&face);
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
            |_, cx| cx.new(|cx| CubemapExample::new(cx)),
        )
        .expect("Failed to open window");

        example_prelude::init_example(cx, "Custom Draw API (Cubemap)");
    });
}
