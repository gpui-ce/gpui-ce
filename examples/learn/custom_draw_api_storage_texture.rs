//! Custom Draw API (Storage Texture) Example
//!
//! Demonstrates a compute pipeline writing into a storage texture that is
//! then sampled by a draw pass.

#[path = "../prelude.rs"]
mod example_prelude;

use std::sync::Arc;
use std::time::Instant;

use gpui::{
    App, AppContext, Application, Bounds, Colors, Context, CustomAddressMode, CustomBindingDesc,
    CustomBindingKind, CustomBindingName, CustomBindingValue, CustomBufferDesc, CustomBufferId,
    CustomBufferSource, CustomComputeDispatch, CustomComputePipelineDesc, CustomComputePipelineId,
    CustomDrawParams, CustomFilterMode, CustomPipelineDesc, CustomPipelineId, CustomPipelineState,
    CustomPrimitiveTopology, CustomSamplerDesc, CustomSamplerId, CustomTextureDesc,
    CustomTextureFormat, CustomTextureId, CustomTextureUsage, CustomUniformBuilder,
    CustomVertexAttribute, CustomVertexAttributeName, CustomVertexBuffer, CustomVertexFetch,
    CustomVertexFormat, CustomVertexLayout, Hsla, Render, Styled, Window, WindowBounds,
    WindowOptions, canvas, div, prelude::*, px, size,
};

const STORAGE_TEXTURE_SIZE: u32 = 256;
const WORKGROUP_SIZE: u32 = 8;

const COMPUTE_SHADER_SOURCE: &str = r#"
struct Params {
  time: vec4<f32>,
};

var b0: texture_storage_2d<rgba8unorm, write>;
var<uniform> b1: Params;

@compute @workgroup_size(8, 8)
fn cs_main(@builtin(global_invocation_id) id: vec3<u32>) {
  let size = textureDimensions(b0);
  if id.x >= size.x || id.y >= size.y {
    return;
  }

  let uv = vec2<f32>(f32(id.x) / f32(size.x), f32(id.y) / f32(size.y));
  let pulse = 0.5 + 0.5 * sin(b1.time.x);
  let color = vec4<f32>(uv.x, uv.y, pulse, 1.0);
  textureStore(b0, vec2<i32>(id.xy), color);
}
"#;

const DRAW_SHADER_SOURCE: &str = r#"
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

struct StorageTextureExample {
    compute_pipeline: Option<CustomComputePipelineId>,
    render_pipeline: Option<CustomPipelineId>,
    vertex_buffer: Option<CustomBufferId>,
    storage_texture: Option<CustomTextureId>,
    sampler: Option<CustomSamplerId>,
    error: Option<String>,
    start: Instant,
}

struct StorageTextureFrame {
    compute: CustomComputeDispatch,
    draw: CustomDrawParams,
}

impl StorageTextureExample {
    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            compute_pipeline: None,
            render_pipeline: None,
            vertex_buffer: None,
            storage_texture: None,
            sampler: None,
            error: None,
            start: Instant::now(),
        }
    }

    fn ensure_resources(&mut self, window: &mut Window) {
        if self.compute_pipeline.is_some() || self.error.is_some() {
            return;
        }

        match self.build_resources(window) {
            Ok((compute_pipeline, render_pipeline, vertex_buffer, storage_texture, sampler)) => {
                self.compute_pipeline = Some(compute_pipeline);
                self.render_pipeline = Some(render_pipeline);
                self.vertex_buffer = Some(vertex_buffer);
                self.storage_texture = Some(storage_texture);
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
        CustomComputePipelineId,
        CustomPipelineId,
        CustomBufferId,
        CustomTextureId,
        CustomSamplerId,
    )> {
        let compute_pipeline =
            window.create_custom_compute_pipeline(CustomComputePipelineDesc {
                name: "custom_draw_storage_compute".to_string(),
                shader_source: COMPUTE_SHADER_SOURCE.to_string(),
                entry_point: "cs_main".to_string(),
                bindings: vec![
                    CustomBindingDesc {
                        name: CustomBindingName::B0,
                        kind: CustomBindingKind::StorageTexture,
                        slot: None,
                    },
                    CustomBindingDesc {
                        name: CustomBindingName::B1,
                        kind: CustomBindingKind::Uniform { size: 16 },
                        slot: None,
                    },
                ],
            })?;

        let render_pipeline = window.create_custom_pipeline(CustomPipelineDesc {
            name: "custom_draw_storage_render".to_string(),
            shader_source: DRAW_SHADER_SOURCE.to_string(),
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

        let vertex_buffer = window.create_custom_buffer(CustomBufferDesc {
            name: "storage_texture_vertices".to_string(),
            data: quad_vertex_data(),
        })?;

        let texture = window.create_custom_texture(CustomTextureDesc {
            name: "storage_texture".to_string(),
            width: STORAGE_TEXTURE_SIZE,
            height: STORAGE_TEXTURE_SIZE,
            format: CustomTextureFormat::Rgba8Unorm,
            usage: CustomTextureUsage::SAMPLED | CustomTextureUsage::STORAGE,
            data: vec![Arc::from(vec![
                0u8;
                (STORAGE_TEXTURE_SIZE * STORAGE_TEXTURE_SIZE * 4)
                    as usize
            ])],
        })?;

        let sampler = window.create_custom_sampler(CustomSamplerDesc {
            name: "storage_texture_sampler".to_string(),
            min_filter: CustomFilterMode::Linear,
            mag_filter: CustomFilterMode::Linear,
            mipmap_filter: CustomFilterMode::Nearest,
            address_modes: [CustomAddressMode::ClampToEdge; 3],
        })?;

        Ok((
            compute_pipeline,
            render_pipeline,
            vertex_buffer,
            texture,
            sampler,
        ))
    }

    fn uniform_for_time(&self) -> Arc<[u8]> {
        let t = self.start.elapsed().as_secs_f32();
        let mut builder = CustomUniformBuilder::new();
        builder.push_vec4(t, 0.0, 0.0, 0.0);
        builder.finish()
    }
}

impl Render for StorageTextureExample {
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
                    .child("Custom Draw API (Storage Texture)"),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(colors.text_muted)
                    .child("Compute writes a storage texture sampled in a draw pass"),
            );

        let surface: Hsla = colors.surface.into();
        let content = if let Some(err) = &self.error {
            div()
                .text_sm()
                .text_color(colors.error)
                .child(format!("Custom draw unsupported: {err}"))
        } else if let (
            Some(compute_pipeline),
            Some(render_pipeline),
            Some(vertex_buffer),
            Some(storage_texture),
            Some(sampler),
        ) = (
            self.compute_pipeline,
            self.render_pipeline,
            self.vertex_buffer,
            self.storage_texture,
            self.sampler,
        ) {
            let uniform = self.uniform_for_time();
            let prepaint = move |bounds: Bounds<_>, window: &mut Window, _cx: &mut App| {
                let vertex_data = quad_vertex_data_for_bounds(bounds, window.viewport_size());
                if let Err(err) =
                    window.update_custom_buffer(vertex_buffer, Arc::clone(&vertex_data))
                {
                    log::error!("custom draw vertex update failed: {err}");
                }
                let group_count = (STORAGE_TEXTURE_SIZE + WORKGROUP_SIZE - 1) / WORKGROUP_SIZE;
                StorageTextureFrame {
                    compute: CustomComputeDispatch {
                        pipeline: compute_pipeline,
                        bindings: vec![
                            CustomBindingValue::Texture(storage_texture),
                            CustomBindingValue::Uniform(CustomBufferSource::Inline(Arc::clone(
                                &uniform,
                            ))),
                        ],
                        workgroup_count: [group_count, group_count, 1],
                    },
                    draw: CustomDrawParams {
                        bounds,
                        pipeline: render_pipeline,
                        vertex_buffers: vec![CustomVertexBuffer {
                            source: CustomBufferSource::Buffer(vertex_buffer),
                        }],
                        vertex_count: 6,
                        index_buffer: None,
                        index_count: 0,
                        target: None,
                        instance_count: 1,
                        bindings: vec![
                            CustomBindingValue::Texture(storage_texture),
                            CustomBindingValue::Sampler(sampler),
                        ],
                    },
                }
            };

            let paint = move |_bounds: Bounds<_>,
                              params: StorageTextureFrame,
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

fn main() {
    Application::new().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(520.), px(520.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_, cx| cx.new(|cx| StorageTextureExample::new(cx)),
        )
        .expect("Failed to open window");

        example_prelude::init_example(cx, "Custom Draw API (Storage Texture)");
    });
}
