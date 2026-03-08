//! Custom Draw API (Compressed Texture) Example
//!
//! Demonstrates sampling a runtime-selected compressed texture.

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

const BC1_RED_BLOCK: [u8; 8] = [0x00, 0xF8, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
const BC1_GREEN_BLOCK: [u8; 8] = [0xE0, 0x07, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
const TEXTURE_WIDTH: u32 = 16;
const TEXTURE_HEIGHT: u32 = 8;

struct CompressedTextureExample {
    pipeline: Option<CustomPipelineId>,
    vertex_buffer: Option<CustomBufferId>,
    texture: Option<CustomTextureId>,
    sampler: Option<CustomSamplerId>,
    format_label: Option<&'static str>,
    error: Option<String>,
}

impl CompressedTextureExample {
    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            pipeline: None,
            vertex_buffer: None,
            texture: None,
            sampler: None,
            format_label: None,
            error: None,
        }
    }

    fn ensure_resources(&mut self, window: &mut Window) {
        if self.pipeline.is_some() || self.error.is_some() {
            return;
        }

        match self.build_resources(window) {
            Ok((pipeline, buffer, texture, sampler, format_label)) => {
                self.pipeline = Some(pipeline);
                self.vertex_buffer = Some(buffer);
                self.texture = Some(texture);
                self.sampler = Some(sampler);
                self.format_label = Some(format_label);
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
        &'static str,
    )> {
        let (compressed_format, format_label) = choose_compressed_format(window)?;

        let pipeline = window.create_custom_pipeline(CustomPipelineDesc {
            name: "custom_draw_compressed_texture".to_string(),
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

        let vertex_buffer = window.create_custom_buffer(CustomBufferDesc {
            name: "compressed_texture_vertices".to_string(),
            data: quad_vertex_data(),
        })?;

        let texture = window.create_custom_texture(CustomTextureDesc {
            name: format!("compressed_texture_{}", format_label.to_lowercase()),
            dimension: CustomTextureDimension::D2,
            width: TEXTURE_WIDTH,
            height: TEXTURE_HEIGHT,
            format: compressed_format,
            usage: CustomTextureUsage::SAMPLED,
            data: vec![compressed_texture_data(
                compressed_format,
                TEXTURE_WIDTH,
                TEXTURE_HEIGHT,
            )?],
        })?;

        let sampler = window.create_custom_sampler(CustomSamplerDesc {
            name: "compressed_texture_sampler".to_string(),
            min_filter: CustomFilterMode::Nearest,
            mag_filter: CustomFilterMode::Nearest,
            mipmap_filter: CustomFilterMode::Nearest,
            address_modes: [CustomAddressMode::ClampToEdge; 3],
        })?;

        Ok((pipeline, vertex_buffer, texture, sampler, format_label))
    }
}

impl Render for CompressedTextureExample {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl gpui::IntoElement {
        let colors = Colors::for_appearance(window);
        self.ensure_resources(window);
        let format_label = self.format_label.unwrap_or("pending");

        let header = div()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .text_xl()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(colors.text)
                    .child("Custom Draw API (Compressed Texture)"),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(colors.text_muted)
                    .child(format!("Samples a {format_label} compressed texture")),
            );

        let surface: Hsla = colors.surface.into();
        let content = if let (Some(pipeline), Some(buffer), Some(texture), Some(sampler)) = (
            self.pipeline,
            self.vertex_buffer,
            self.texture,
            self.sampler,
        ) {
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

fn choose_compressed_format(
    window: &mut Window,
) -> anyhow::Result<(CustomTextureFormat, &'static str)> {
    let candidates = [
        (CustomTextureFormat::Astc8x8Unorm, "ASTC 8x8"),
        (CustomTextureFormat::Astc6x6Unorm, "ASTC 6x6"),
        (CustomTextureFormat::Astc5x5Unorm, "ASTC 5x5"),
        (CustomTextureFormat::Astc4x4Unorm, "ASTC 4x4"),
        (CustomTextureFormat::PvrtcRgba4bppUnorm, "PVRTC RGBA 4bpp"),
        (CustomTextureFormat::PvrtcRgba2bppUnorm, "PVRTC RGBA 2bpp"),
        (CustomTextureFormat::Etc2Rgba8Unorm, "ETC2 RGBA8"),
        (CustomTextureFormat::Bc7Unorm, "BC7"),
        (CustomTextureFormat::Bc3Unorm, "BC3"),
        (CustomTextureFormat::Bc1Unorm, "BC1"),
    ];

    for (format, label) in candidates {
        if window.custom_texture_format_supported(format)? {
            return Ok((format, label));
        }
    }

    Err(anyhow::anyhow!(
        "no supported compressed texture format found on this backend"
    ))
}

fn compressed_block_layout(format: CustomTextureFormat) -> Option<(u32, u32, u32)> {
    match format {
        CustomTextureFormat::Bc1Unorm
        | CustomTextureFormat::Bc1UnormSrgb
        | CustomTextureFormat::Etc2Rgb8Unorm
        | CustomTextureFormat::Etc2Rgb8UnormSrgb => Some((4, 4, 8)),
        CustomTextureFormat::Bc3Unorm
        | CustomTextureFormat::Bc3UnormSrgb
        | CustomTextureFormat::Bc7Unorm
        | CustomTextureFormat::Bc7UnormSrgb
        | CustomTextureFormat::Etc2Rgba8Unorm
        | CustomTextureFormat::Etc2Rgba8UnormSrgb
        | CustomTextureFormat::Astc4x4Unorm
        | CustomTextureFormat::Astc4x4UnormSrgb => Some((4, 4, 16)),
        CustomTextureFormat::Astc5x5Unorm | CustomTextureFormat::Astc5x5UnormSrgb => {
            Some((5, 5, 16))
        }
        CustomTextureFormat::Astc6x6Unorm | CustomTextureFormat::Astc6x6UnormSrgb => {
            Some((6, 6, 16))
        }
        CustomTextureFormat::Astc8x8Unorm | CustomTextureFormat::Astc8x8UnormSrgb => {
            Some((8, 8, 16))
        }
        CustomTextureFormat::PvrtcRgb2bppUnorm
        | CustomTextureFormat::PvrtcRgb2bppUnormSrgb
        | CustomTextureFormat::PvrtcRgba2bppUnorm
        | CustomTextureFormat::PvrtcRgba2bppUnormSrgb => Some((16, 8, 8)),
        CustomTextureFormat::PvrtcRgb4bppUnorm
        | CustomTextureFormat::PvrtcRgb4bppUnormSrgb
        | CustomTextureFormat::PvrtcRgba4bppUnorm
        | CustomTextureFormat::PvrtcRgba4bppUnormSrgb => Some((8, 4, 8)),
        _ => None,
    }
}

fn compressed_texture_data(
    format: CustomTextureFormat,
    width: u32,
    height: u32,
) -> anyhow::Result<Arc<[u8]>> {
    let Some((block_width, block_height, bytes_per_block)) = compressed_block_layout(format) else {
        return Err(anyhow::anyhow!(
            "texture format {format:?} is not compressed"
        ));
    };

    let blocks_w = width.div_ceil(block_width);
    let blocks_h = height.div_ceil(block_height);
    let block_count = blocks_w * blocks_h;

    let mut data = Vec::with_capacity((block_count * bytes_per_block) as usize);
    for block_index in 0..block_count {
        if matches!(
            format,
            CustomTextureFormat::Bc1Unorm | CustomTextureFormat::Bc1UnormSrgb
        ) {
            if block_index % 2 == 0 {
                data.extend_from_slice(&BC1_RED_BLOCK);
            } else {
                data.extend_from_slice(&BC1_GREEN_BLOCK);
            }
            continue;
        }

        for byte_index in 0..bytes_per_block {
            let value = (block_index as u8)
                .wrapping_mul(29)
                .wrapping_add(byte_index as u8)
                .wrapping_add(17);
            data.push(value);
        }
    }

    Ok(Arc::from(data))
}

fn main() {
    Application::new().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(520.), px(520.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_, cx| cx.new(|cx| CompressedTextureExample::new(cx)),
        )
        .expect("Failed to open window");

        example_prelude::init_example(cx, "Custom Draw API (Compressed Texture)");
    });
}
