//! Custom Draw API (Streaming Texture) Example
//!
//! Demonstrates updating a texture each frame from a buffer with row padding.
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
    CustomPipelineState, CustomPrimitiveTopology, CustomSamplerDesc, CustomSamplerId,
    CustomTextureBufferUpdate, CustomTextureDesc, CustomTextureDimension, CustomTextureFormat,
    CustomTextureId, CustomTextureUsage, CustomVertexAttribute, CustomVertexAttributeName,
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

const TEXTURE_WIDTH: u32 = 96;
const TEXTURE_HEIGHT: u32 = 96;
const BYTES_PER_PIXEL: u32 = 4;
const ROW_PADDING: u32 = 16;
const PACKED_BYTES_PER_ROW: u32 = TEXTURE_WIDTH * BYTES_PER_PIXEL;
const BYTES_PER_ROW: u32 = PACKED_BYTES_PER_ROW + ROW_PADDING;

struct StreamingTextureExample {
    pipeline: Option<CustomPipelineId>,
    vertex_buffer: Option<CustomBufferId>,
    texture: Option<CustomTextureId>,
    texture_buffer: Option<CustomBufferId>,
    sampler: Option<CustomSamplerId>,
    start: Instant,
    error: Option<String>,
}

impl StreamingTextureExample {
    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            pipeline: None,
            vertex_buffer: None,
            texture: None,
            texture_buffer: None,
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
            Ok((pipeline, buffer, texture, texture_buffer, sampler)) => {
                self.pipeline = Some(pipeline);
                self.vertex_buffer = Some(buffer);
                self.texture = Some(texture);
                self.texture_buffer = Some(texture_buffer);
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
        CustomBufferId,
        CustomSamplerId,
    )> {
        let pipeline = window.create_custom_pipeline(CustomPipelineDesc {
            name: "custom_draw_demo_streaming_texture".to_string(),
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
            color_targets: Vec::new(),
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

        let vertex_data = quad_vertex_data();
        let vertex_buffer = window.create_custom_buffer(CustomBufferDesc {
            name: "quad_vertices_streaming".to_string(),
            data: vertex_data,
        })?;

        let packed_data =
            streaming_texture_data(TEXTURE_WIDTH, TEXTURE_HEIGHT, PACKED_BYTES_PER_ROW, 0.0);
        let texture = window.create_custom_texture(CustomTextureDesc {
            name: "streaming_texture".to_string(),
            dimension: CustomTextureDimension::D2,
            width: TEXTURE_WIDTH,
            height: TEXTURE_HEIGHT,
            format: CustomTextureFormat::Rgba8Unorm,
            usage: CustomTextureUsage::SAMPLED,
            data: vec![packed_data],
        })?;

        let buffer_data = streaming_texture_data(TEXTURE_WIDTH, TEXTURE_HEIGHT, BYTES_PER_ROW, 0.0);
        let texture_buffer = window.create_custom_buffer(CustomBufferDesc {
            name: "streaming_texture_buffer".to_string(),
            data: buffer_data,
        })?;

        let sampler = window.create_custom_sampler(CustomSamplerDesc {
            name: "streaming_sampler".to_string(),
            min_filter: CustomFilterMode::Linear,
            mag_filter: CustomFilterMode::Linear,
            mipmap_filter: CustomFilterMode::Nearest,
            address_modes: [CustomAddressMode::ClampToEdge; 3],
        })?;

        Ok((pipeline, vertex_buffer, texture, texture_buffer, sampler))
    }
}

impl Render for StreamingTextureExample {
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
                    .child("Custom Draw API (Streaming Texture)"),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(colors.text_muted)
                    .child("Buffer-backed texture updates with row padding"),
            );

        let surface: Hsla = colors.surface.into();
        let content = if let Some(err) = &self.error {
            div()
                .text_sm()
                .text_color(colors.error)
                .child(format!("Custom draw unsupported: {err}"))
        } else if let (
            Some(pipeline),
            Some(buffer),
            Some(texture),
            Some(texture_buffer),
            Some(sampler),
        ) = (
            self.pipeline,
            self.vertex_buffer,
            self.texture,
            self.texture_buffer,
            self.sampler,
        ) {
            let start = self.start;
            let prepaint = move |bounds: Bounds<_>, window: &mut Window, _cx: &mut App| {
                let vertex_data = quad_vertex_data_for_bounds(bounds, window.viewport_size());
                if let Err(err) = window.update_custom_buffer(buffer, Arc::clone(&vertex_data)) {
                    log::error!("custom draw vertex update failed: {err}");
                }

                let time = start.elapsed().as_secs_f32();
                let texture_data =
                    streaming_texture_data(TEXTURE_WIDTH, TEXTURE_HEIGHT, BYTES_PER_ROW, time);
                if let Err(err) =
                    window.update_custom_buffer(texture_buffer, Arc::clone(&texture_data))
                {
                    log::error!("custom draw texture buffer update failed: {err}");
                }

                if let Err(err) = window.update_custom_texture_from_buffer(
                    texture,
                    CustomTextureBufferUpdate {
                        level: 0,
                        buffer: CustomBufferSource::Buffer(texture_buffer),
                        bytes_per_row: Some(BYTES_PER_ROW),
                    },
                ) {
                    log::error!("custom draw texture update failed: {err}");
                }

                CustomDrawParams {
                    bounds,
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

fn streaming_texture_data(width: u32, height: u32, bytes_per_row: u32, time: f32) -> Arc<[u8]> {
    let mut data = vec![0u8; (bytes_per_row * height) as usize];
    let width_scale = (width.saturating_sub(1)).max(1) as f32;
    let height_scale = (height.saturating_sub(1)).max(1) as f32;

    for row in 0..height {
        let row_offset = row as usize * bytes_per_row as usize;
        let row_ratio = row as f32 / height_scale;
        for column in 0..width {
            let column_ratio = column as f32 / width_scale;
            let wave = (time * 2.0 + column_ratio * 6.0 + row_ratio * 4.0).sin() * 0.5 + 0.5;
            let red = (column_ratio * 255.0) as u8;
            let green = (row_ratio * 255.0) as u8;
            let blue = (wave * 255.0) as u8;
            let offset = row_offset + column as usize * 4;
            data[offset] = red;
            data[offset + 1] = green;
            data[offset + 2] = blue;
            data[offset + 3] = 0xff;
        }
    }

    Arc::from(data)
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

    let viewport_width = f32::from(viewport.width).max(1.0);
    let viewport_height = f32::from(viewport.height).max(1.0);

    let to_ndc_x = |x: gpui::Pixels| (f32::from(x) / viewport_width) * 2.0 - 1.0;
    let to_ndc_y = |y: gpui::Pixels| 1.0 - (f32::from(y) / viewport_height) * 2.0;

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
            |_, cx| cx.new(|cx| StreamingTextureExample::new(cx)),
        )
        .expect("Failed to open window");

        example_prelude::init_example(cx, "Custom Draw API (Streaming Texture)");
    });
}
