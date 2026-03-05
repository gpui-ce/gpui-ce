//! Custom Draw API (Metallib) Example
//!
//! Demonstrates creating a custom draw pipeline from precompiled `.metallib` bytes.
//!
//! Notes:
//! - This example is intended for the default Metal backend on macOS.
//! - With `--features macos-blade`, pipeline creation from `.metallib` is expected to fail.
//! - Set `GPUI_EXAMPLE_ENABLE_PIPELINE_CACHE=1` to also exercise persistent pipeline cache.

#[path = "../prelude.rs"]
mod example_prelude;

use std::sync::Arc;

#[cfg(target_os = "macos")]
use std::{
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

#[cfg(target_os = "macos")]
use smol::process::Command;

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

const WGSL_SOURCE: &str = r#"
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

#[cfg(target_os = "macos")]
const MSL_SOURCE: &str = r#"
#include <metal_stdlib>
using namespace metal;

struct VertexInput {
    float2 position [[attribute(0)]];
    float2 uv [[attribute(1)]];
};

struct VertexOutput {
    float4 position [[position]];
    float2 uv;
};

vertex VertexOutput vs_main(VertexInput input [[stage_in]]) {
    VertexOutput output;
    output.position = float4(input.position, 0.0, 1.0);
    output.uv = input.uv;
    return output;
}

fragment float4 fs_main(VertexOutput input [[stage_in]],
                        texture2d<float> b0 [[texture(0)]],
                        sampler b1 [[sampler(1)]]) {
    return b0.sample(b1, input.uv);
}
"#;

struct MetallibCustomDrawExample {
    pipeline: Option<CustomPipelineId>,
    vertex_buffer: Option<CustomBufferId>,
    texture: Option<CustomTextureId>,
    sampler: Option<CustomSamplerId>,
    pipeline_source: Option<&'static str>,
    error: Option<String>,
}

impl MetallibCustomDrawExample {
    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            pipeline: None,
            vertex_buffer: None,
            texture: None,
            sampler: None,
            pipeline_source: None,
            error: None,
        }
    }

    fn ensure_resources(&mut self, window: &mut Window) {
        if self.pipeline.is_some() || self.error.is_some() {
            return;
        }

        match self.build_resources(window) {
            Ok((pipeline, buffer, texture, sampler, pipeline_source)) => {
                self.pipeline = Some(pipeline);
                self.vertex_buffer = Some(buffer);
                self.texture = Some(texture);
                self.sampler = Some(sampler);
                self.pipeline_source = Some(pipeline_source);
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
        #[cfg(target_os = "macos")]
        {
            if std::env::var_os("GPUI_EXAMPLE_ENABLE_PIPELINE_CACHE").is_some() {
                let cache_path =
                    std::env::temp_dir().join("gpui_custom_draw_pipeline_cache.binarchive");
                if let Err(err) = window.set_custom_pipeline_cache_path(&cache_path) {
                    log::warn!("custom draw pipeline cache unavailable: {err}");
                }
            }
        }

        let pipeline = create_pipeline_from_metallib(window)?;

        let vertex_data = quad_vertex_data();
        let vertex_buffer = window.create_custom_buffer(CustomBufferDesc {
            name: "quad_vertices_metallib".to_string(),
            data: vertex_data,
        })?;

        let texture_data = checker_texture_data();
        let texture = window.create_custom_texture(CustomTextureDesc {
            name: "checker_texture_metallib".to_string(),
            dimension: CustomTextureDimension::D2,
            width: 2,
            height: 2,
            format: CustomTextureFormat::Rgba8Unorm,
            usage: CustomTextureUsage::SAMPLED,
            data: vec![texture_data],
        })?;

        let sampler = window.create_custom_sampler(CustomSamplerDesc {
            name: "checker_sampler_metallib".to_string(),
            min_filter: CustomFilterMode::Nearest,
            mag_filter: CustomFilterMode::Nearest,
            mipmap_filter: CustomFilterMode::Nearest,
            address_modes: [CustomAddressMode::ClampToEdge; 3],
        })?;

        Ok((pipeline, vertex_buffer, texture, sampler, "metallib"))
    }
}

impl Render for MetallibCustomDrawExample {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl gpui::IntoElement {
        let colors = Colors::for_appearance(window);
        self.ensure_resources(window);

        let pipeline_source = self.pipeline_source.unwrap_or("pending");
        let header = div()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .text_xl()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(colors.text)
                    .child("Custom Draw API (Metallib)"),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(colors.text_muted)
                    .child(format!("Pipeline source: {pipeline_source}")),
            );

        let surface: Hsla = colors.surface.into();
        let content = if let Some(err) = &self.error {
            div()
                .text_sm()
                .text_color(colors.error)
                .child(format!("Pipeline creation failed: {err}"))
        } else if let (Some(pipeline), Some(buffer), Some(texture), Some(sampler)) = (
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

fn pipeline_desc() -> CustomPipelineDesc {
    CustomPipelineDesc {
        name: "custom_draw_demo_metallib".to_string(),
        shader_source: WGSL_SOURCE.to_string(),
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
    }
}

fn create_pipeline_from_metallib(window: &mut Window) -> anyhow::Result<CustomPipelineId> {
    #[cfg(target_os = "macos")]
    {
        let metallib_path = compile_metallib_file(MSL_SOURCE)?;
        return window.create_custom_pipeline_metallib_file(pipeline_desc(), metallib_path);
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = window;
        Err(anyhow::anyhow!(
            "custom_draw_api_metallib requires macOS with the Metal backend"
        ))
    }
}

#[cfg(target_os = "macos")]
fn compile_metallib_file(msl_source: &str) -> anyhow::Result<PathBuf> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|err| anyhow::anyhow!("system time error: {err}"))?
        .as_millis();
    let directory = std::env::temp_dir().join(format!(
        "gpui_custom_draw_metallib_{}_{}",
        std::process::id(),
        timestamp
    ));
    std::fs::create_dir_all(&directory)?;

    let metal_path = directory.join("custom_draw_shader.metal");
    let air_path = directory.join("custom_draw_shader.air");
    let metallib_path = directory.join("custom_draw_shader.metallib");

    std::fs::write(&metal_path, msl_source)?;

    let metal_output = smol::block_on(
        Command::new("xcrun")
            .args(["-sdk", "macosx", "metal", "-c"])
            .arg(&metal_path)
            .arg("-o")
            .arg(&air_path)
            .output(),
    )
    .map_err(|err| anyhow::anyhow!("failed to run xcrun metal: {err}"))?;
    if !metal_output.status.success() {
        return Err(anyhow::anyhow!(
            "xcrun metal failed: {}",
            String::from_utf8_lossy(&metal_output.stderr)
        ));
    }

    let metallib_output = smol::block_on(
        Command::new("xcrun")
            .args(["-sdk", "macosx", "metallib"])
            .arg(&air_path)
            .arg("-o")
            .arg(&metallib_path)
            .output(),
    )
    .map_err(|err| anyhow::anyhow!("failed to run xcrun metallib: {err}"))?;
    if !metallib_output.status.success() {
        return Err(anyhow::anyhow!(
            "xcrun metallib failed: {}",
            String::from_utf8_lossy(&metallib_output.stderr)
        ));
    }

    Ok(metallib_path)
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
            |_, cx| cx.new(|cx| MetallibCustomDrawExample::new(cx)),
        )
        .expect("Failed to open window");

        example_prelude::init_example(cx, "Custom Draw API (Metallib)");
    });
}
