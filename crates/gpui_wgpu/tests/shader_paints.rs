//! Compare WGPU shader pixels with ordinary quads and CPU paint evaluation.
#![cfg(feature = "test-support")]

mod support;

use gpui::{
    AnyElement, App, AppContext, Context, HeadlessAppContext, IntoElement, ParentElement,
    PrimitiveBatch, Render, RenderCommand, Styled, Window, canvas, div, hsla, px, rgb,
    shader::{Fragment, Paint, Pipeline, Shader, color, paint, rgba, vec2, vec2f},
};
use gpui_ce_wgpu::WgpuHeadlessRenderer;
use image::RgbaImage;
use std::sync::Arc;

struct View(Box<dyn Fn() -> AnyElement>);

impl Render for View {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        (self.0)()
    }
}

/// Render `build` into a `width` × `height` window on a black background.
fn render(width: f32, height: f32, build: impl Fn() -> AnyElement + 'static) -> RgbaImage {
    render_with(width, height, build, |_, _| {})
}

fn render_with(
    width: f32,
    height: f32,
    build: impl Fn() -> AnyElement + 'static,
    inspect: impl FnOnce(&mut Window, &mut App),
) -> RgbaImage {
    let mut cx = HeadlessAppContext::with_platform(support::text_system(), Arc::new(()), || {
        Some(Box::new(
            WgpuHeadlessRenderer::new().expect("a GPU adapter"),
        ))
    });
    let root = move || {
        div()
            .size_full()
            .bg(rgb(0x000000))
            .child(build())
            .into_any_element()
    };
    let window = cx
        .open_window(gpui::size(px(width), px(height)), |_, cx| {
            cx.new(|_| View(Box::new(root)))
        })
        .unwrap();
    cx.update_window(window.into(), |_, window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.supports_shader_paint());
        inspect(window, cx);
    })
    .unwrap();
    cx.capture_screenshot(window.into()).unwrap()
}

/// The pixel under logical point (`x`, `y`) of a window `width` logical pixels wide.
fn pixel_at(image: &RgbaImage, width: f32, x: f32, y: f32) -> [u8; 4] {
    let scale = image.width() as f32 / width;
    image.get_pixel((x * scale) as u32, (y * scale) as u32).0
}

fn assert_near(actual: [u8; 4], expected: [u8; 4], tolerance: u8, context: &str) {
    assert!(
        actual
            .iter()
            .zip(expected)
            .all(|(a, e)| a.abs_diff(e) <= tolerance),
        "{context}: got {actual:?}, expected {expected:?} (±{tolerance})"
    );
}

fn assert_images_near(left: &RgbaImage, right: &RgbaImage, tolerance: u8) {
    assert_eq!(left.dimensions(), right.dimensions());
    for (x, y, a) in left.enumerate_pixels() {
        assert_near(
            a.0,
            right.get_pixel(x, y).0,
            tolerance,
            &format!("({x}, {y})"),
        );
    }
}

fn to_bytes(rgba: gpui::shader::Vec4f) -> [u8; 4] {
    [rgba.x, rgba.y, rgba.z, rgba.w].map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8)
}

fn ramp() -> Paint {
    paint(|px| rgba(px.uv().x(), px.uv().y(), 0.25, 1.0))
}

#[test]
fn a_constant_paint_matches_an_ordinary_quad_including_corners_and_borders() {
    let element = |fill: gpui::Fill, border: gpui::Fill| {
        div()
            .absolute()
            .left(px(8.5))
            .top(px(6.0))
            .w(px(47.0))
            .h(px(37.0))
            .rounded(px(11.0))
            .border_3()
            .bg(fill)
            .border_color(border)
            .into_any_element()
    };
    let (fill, border) = (hsla(0.58, 0.7, 0.55, 0.8), hsla(0.08, 0.9, 0.6, 1.0));
    let quads = render(64.0, 48.0, move || element(fill.into(), border.into()));
    let paints = render(64.0, 48.0, move || {
        element(color(fill).into(), color(border).into())
    });
    assert_images_near(&quads, &paints, 1);
}

#[test]
fn per_fragment_paints_match_their_cpu_evaluation() {
    let image = render(64.0, 64.0, || {
        div().size(px(64.0)).bg(ramp()).into_any_element()
    });
    let scale = image.width() as f32 / 64.0;
    for (u, v) in [
        (0.0, 0.0),
        (0.2, 0.6),
        (0.5, 0.5),
        (0.99, 0.07),
        (0.78, 0.99),
    ] {
        let x = (u * image.width() as f32) as u32;
        let y = (v * image.height() as f32) as u32;
        // The center of device pixel (x, y), in the box's normalized coordinates.
        let uv = [
            (x as f32 + 0.5) / scale / 64.0,
            (y as f32 + 0.5) / scale / 64.0,
        ];
        let expected = ramp()
            .evaluate(Fragment {
                uv: vec2f(uv[0], uv[1]),
                position: vec2f(uv[0] * 64.0, uv[1] * 64.0),
                size: vec2f(64.0, 64.0),
                origin: vec2f(0.0, 0.0),
                scale: 1.0,
            })
            .unwrap();
        assert_near(
            image.get_pixel(x, y).0,
            to_bytes(expected),
            1,
            &format!("({x}, {y})"),
        );
    }
}

#[test]
fn mapped_paints_sample_at_mapped_coordinates() {
    let flipped = render(64.0, 64.0, || {
        div()
            .size(px(64.0))
            .bg(ramp().map_uv(|uv| vec2(1.0 - uv.x(), uv.y())))
            .into_any_element()
    });
    let plain = render(64.0, 64.0, || {
        div().size(px(64.0)).bg(ramp()).into_any_element()
    });
    let scale = plain.width() / 64;
    for (x, y) in [(3, 7), (40, 22), (60, 60)] {
        let (x, y) = (x * scale, y * scale);
        let mirrored = plain.get_pixel(plain.width() - 1 - x, y).0;
        assert_near(flipped.get_pixel(x, y).0, mirrored, 1, "mirror");
    }
}

#[test]
fn border_paints_leave_the_interior_untouched() {
    let image = render(64.0, 64.0, || {
        div()
            .size(px(64.0))
            .border_4()
            .border_color(ramp())
            .into_any_element()
    });
    assert_eq!(pixel_at(&image, 64.0, 32.0, 32.0), [0, 0, 0, 255]);
    let corner = pixel_at(&image, 64.0, 1.0, 1.0);
    assert!(corner[2] > 40, "border painted: {corner:?}");
}

#[test]
fn background_and_border_with_different_programs_both_paint() {
    let stripes = paint(|px| {
        let band = (px.position().x() * 0.5).sin().step(0.0);
        rgba(&band, 0.0, 1.0 - &band, 1.0)
    });
    let image = render(64.0, 64.0, move || {
        div()
            .size(px(64.0))
            .border_8()
            .bg(stripes.clone())
            .border_color(ramp())
            .into_any_element()
    });
    let border = pixel_at(&image, 64.0, 2.0, 32.0);
    let fill = pixel_at(&image, 64.0, 32.0, 32.0);
    assert_ne!(border, fill);
    assert!(border[2].abs_diff(64) <= 1, "{border:?}");
    assert_eq!(fill[1], 0, "{fill:?}");
}

#[test]
fn backdrop_paints_sample_the_scene_behind_them() {
    let image = render(64.0, 32.0, || {
        div()
            .size_full()
            .child(div().absolute().size(px(32.0)).bg(rgb(0xff0000)))
            .child(
                div()
                    .absolute()
                    .left(px(32.0))
                    .size(px(32.0))
                    .bg(rgb(0x0000ff)),
            )
            .child(div().absolute().size_full().bg(paint(|px| {
                px.backdrop(vec2(1.0 - px.uv().x(), px.uv().y()))
            })))
            .into_any_element()
    });
    assert_near(
        pixel_at(&image, 64.0, 8.0, 16.0),
        [0, 0, 255, 255],
        1,
        "left shows right",
    );
    assert_near(
        pixel_at(&image, 64.0, 56.0, 16.0),
        [255, 0, 0, 255],
        1,
        "right shows left",
    );
}

#[test]
fn paints_sharing_a_program_draw_in_one_batch() {
    let waves = |phase: f32| {
        paint(move |px| {
            let wave = (px.uv().x() * 12.0 + phase).sin() * 0.5 + 0.5;
            color(rgb(0x315bff)).mix(color(rgb(0xf48bcb)), wave)
        })
    };
    render_with(
        128.0,
        16.0,
        move || {
            div()
                .flex()
                .children((0..8).map(|i| div().size(px(16.0)).bg(waves(i as f32))))
                .into_any_element()
        },
        |window, _| {
            let batches = window
                .rendered_scene_commands()
                .iter()
                .filter(|command| {
                    matches!(
                        command,
                        RenderCommand::Batch(PrimitiveBatch::ShaderQuads { .. })
                    )
                })
                .count();
            assert_eq!(batches, 1);
        },
    );
}

#[test]
fn element_opacity_scales_shader_paints() {
    let image = render(32.0, 32.0, || {
        div()
            .size(px(32.0))
            .opacity(0.5)
            .bg(color(rgb(0xffffff)).over(ramp()))
            .into_any_element()
    });
    assert_near(
        pixel_at(&image, 32.0, 16.0, 16.0),
        [128, 128, 128, 255],
        1,
        "half white",
    );
}

#[test]
fn a_wgsl_file_renders_with_typed_parameters() {
    let circle = Shader::wgsl(
        "fn paint(fragment: Fragment, center: vec2<f32>, radius: f32) -> vec4<f32> {
            let distance = Shape_circle(fragment.position - center, radius);
            return vec4<f32>(1.0, 0.5, 0.0, Shape_coverage(distance, fwidth(distance)));
        }",
    )
    .unwrap();
    let image = render(64.0, 64.0, move || {
        div()
            .size(px(64.0))
            .bg(circle.with(([32.0, 32.0], 16.0)))
            .into_any_element()
    });
    assert_near(
        pixel_at(&image, 64.0, 32.0, 32.0),
        [255, 128, 0, 255],
        1,
        "inside",
    );
    assert_eq!(pixel_at(&image, 64.0, 4.0, 4.0), [0, 0, 0, 255], "outside");
}

const SPARK: &str = "
struct Varyings { position: vec2<f32>, local: vec2<f32> }

fn vertex(index: u32, center: vec2<f32>, radius: f32, tint: vec4<f32>) -> Varyings {
    let corner = vec2<f32>(f32(index & 1u), f32(index >> 1u)) * 2.0 - 1.0;
    return Varyings(center + corner * radius, corner);
}

fn fragment(in: Varyings, center: vec2<f32>, radius: f32, tint: vec4<f32>) -> vec4<f32> {
    let distance = length(in.local) - 1.0;
    return vec4<f32>(tint.rgb, tint.a * clamp(0.5 - distance * radius, 0.0, 1.0));
}
";

fn sparks(pipeline: Pipeline, instances: Vec<([f32; 2], f32, [f32; 4])>) -> AnyElement {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            window.paint_pipeline(&pipeline, bounds, instances.iter().copied());
        },
    )
    .size_full()
    .into_any_element()
}

#[test]
fn custom_pipelines_draw_their_own_geometry() {
    let pipeline = Pipeline::wgsl(SPARK).unwrap();
    let image = render(64.0, 32.0, move || {
        sparks(
            pipeline.clone(),
            vec![
                ([16.0, 16.0], 10.0, [1.0, 0.0, 0.0, 1.0]),
                ([48.0, 16.0], 10.0, [0.0, 0.0, 1.0, 1.0]),
            ],
        )
    });
    assert_near(
        pixel_at(&image, 64.0, 16.0, 16.0),
        [255, 0, 0, 255],
        1,
        "left",
    );
    assert_near(
        pixel_at(&image, 64.0, 48.0, 16.0),
        [0, 0, 255, 255],
        1,
        "right",
    );
    assert_eq!(
        pixel_at(&image, 64.0, 32.0, 16.0),
        [0, 0, 0, 255],
        "between"
    );
    assert_eq!(
        pixel_at(&image, 64.0, 16.0, 2.0),
        [0, 0, 0, 255],
        "outside the circle"
    );
}

#[test]
fn custom_pipelines_respect_clipping_and_batch_their_draws() {
    let pipeline = Pipeline::wgsl(SPARK).unwrap();
    let image = render_with(
        64.0,
        32.0,
        move || {
            let spark = |x: f32| {
                div()
                    .absolute()
                    .left(px(x))
                    .size(px(32.0))
                    .overflow_hidden()
                    .child(sparks(
                        pipeline.clone(),
                        // Crosses the right edge of its clip, into the neighbor.
                        vec![([30.0, 16.0], 12.0, [0.0, 1.0, 0.0, 1.0])],
                    ))
            };
            div()
                .size_full()
                .child(spark(0.0))
                .child(spark(32.0))
                .into_any_element()
        },
        |window, _| {
            let batches = window
                .rendered_scene_commands()
                .iter()
                .filter(|command| {
                    matches!(command, RenderCommand::Batch(PrimitiveBatch::Pipelines(_)))
                })
                .count();
            assert_eq!(batches, 1);
        },
    );
    assert_near(
        pixel_at(&image, 64.0, 24.0, 16.0),
        [0, 255, 0, 255],
        1,
        "inside",
    );
    assert_eq!(
        pixel_at(&image, 64.0, 36.0, 16.0),
        [0, 0, 0, 255],
        "clipped"
    );
    assert_near(
        pixel_at(&image, 64.0, 56.0, 16.0),
        [0, 255, 0, 255],
        1,
        "second draw",
    );
}
