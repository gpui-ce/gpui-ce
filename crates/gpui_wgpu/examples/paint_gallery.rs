//! Render a gallery using typed paints and the canvas geometry vocabulary.
//!
//! Run with `cargo run -p gpui_ce_wgpu --example paint_gallery --features test-support`.
//! An optional first argument selects the PNG destination.
//! Default macOS windows use the native Metal renderer, which does not support
//! these paints yet. This headless WebGPU renderer can run through Metal on macOS.

use std::path::PathBuf;

use gpui::{
    Bounds, ContentMask, Corners, DevicePixels, Pixels, PlatformHeadlessRenderer, Quad, Scene,
    ShaderQuad,
    paint::{PaintRoot, Shader, Shape},
    point, px, rgb, size, solid_background,
};
use gpui_ce_wgpu::WgpuHeadlessRenderer;

fn bounds(x: f32, y: f32, width: f32, height: f32) -> Bounds<Pixels> {
    Bounds::new(point(px(x), px(y)), size(px(width), px(height)))
}

fn rounded_material(
    root: &PaintRoot,
    material: &Shader,
    width: f32,
    height: f32,
    radius: f32,
    border: Option<f32>,
) -> Result<Shader, gpui::paint::ShaderError> {
    // Shape fields share canvas geometry and pack dimensions/radius as uniforms.
    let shape = Shape::Rect {
        bounds: bounds(0.0, 0.0, width, height),
        radius: px(radius),
    };
    root.shader(|cx| {
        let field = shape.field(cx);
        let field = match border {
            Some(width) => field.inward_stroke(cx.scalar(width)),
            None => field,
        };
        field.coverage().mask(cx.sample(material))
    })
}

fn submit(scene: &mut Scene, bounds: Bounds<Pixels>, shader: Shader) {
    // The render target uses one device pixel per logical pixel.
    let bounds = bounds.scale(1.0);
    scene.insert_primitive(ShaderQuad {
        order: 0,
        bounds,
        content_mask: ContentMask { bounds },
        shader,
        opacity: 1.0,
        scale_factor: 1.0,
    });
}

fn backdrop(scene: &mut Scene, bounds: Bounds<Pixels>, color: u32, radius: f32) {
    let bounds = bounds.scale(1.0);
    scene.insert_primitive(Quad {
        bounds,
        content_mask: ContentMask { bounds },
        background: solid_background(rgb(color)),
        corner_radii: Corners::all(px(radius).scale(1.0)),
        ..Default::default()
    });
}

fn gallery() -> anyhow::Result<Scene> {
    let root = PaintRoot::new();
    let phase = root.parameter(0.0)?;
    let tint = root.parameter([0.22, 0.90, 0.94, 1.0])?;
    // A small typed function takes its domain and values explicitly.
    let waves = root.function::<([f32; 2], f32, [f32; 4]), [f32; 4]>(|cx, (uv, phase, tint)| {
        let angle = uv.clone().x() * cx.scalar(5.0) + uv.y() * cx.scalar(3.0) + phase;
        let wave = angle.sin() * cx.scalar(0.5) + cx.scalar(0.5);
        tint.mix(cx.vec4([0.94, 0.22, 0.64, 1.0]), wave)
    })?;
    let material =
        root.shader(|cx| cx.call(&waves, (cx.uv(), cx.uniform(phase), cx.uniform(tint))))?;
    let mut scene = Scene::default();
    backdrop(&mut scene, bounds(0.0, 0.0, 600.0, 400.0), 0x0b1020, 0.0);
    for row in 0..2 {
        for column in 0..3 {
            let x = 20.0 + column as f32 * 192.0;
            let y = 20.0 + row as f32 * 188.0;
            backdrop(&mut scene, bounds(x, y, 176.0, 172.0), 0x182139, 18.0);
            // Quiet native accents establish the grid without fonts or assets.
            backdrop(
                &mut scene,
                bounds(x + 16.0, y + 151.0, 24.0, 3.0),
                0x596783,
                1.5,
            );
            backdrop(
                &mut scene,
                bounds(x + 45.0, y + 151.0, 8.0, 3.0),
                0x34425c,
                1.5,
            );
        }
    }

    // 1. Rounded fill: geometry and material remain independently composable.
    submit(
        &mut scene,
        bounds(34.0, 38.0, 148.0, 116.0),
        rounded_material(&root, &material, 148.0, 116.0, 22.0, None)?,
    );

    // 2. The same material paints an inward border over a native dark interior.
    backdrop(
        &mut scene,
        bounds(226.0, 38.0, 148.0, 116.0),
        0x0d1528,
        30.0,
    );
    submit(
        &mut scene,
        bounds(226.0, 38.0, 148.0, 116.0),
        rounded_material(&root, &material, 148.0, 116.0, 30.0, Some(9.0))?,
    );

    // 3. Circle coverage comes from the exact same Shape used by CanvasBuilder.
    let circle = Shape::Circle {
        center: point(px(58.0), px(58.0)),
        radius: px(58.0),
    };
    let circle_paint = root.shader(|cx| circle.field(cx).coverage().mask(cx.sample(&material)))?;
    submit(&mut scene, bounds(434.0, 38.0, 116.0, 116.0), circle_paint);

    // 4. Smooth union, subtraction, and final coverage form a soft compound cutout.
    let cutout = root.shader(|cx| {
        let left = cx.circle_field(cx.position(), cx.vec2([51.0, 60.0]), cx.scalar(43.0));
        let right = cx.circle_field(cx.position(), cx.vec2([100.0, 60.0]), cx.scalar(43.0));
        let hole = cx.circle_field(cx.position(), cx.vec2([75.0, 42.0]), cx.scalar(24.0));
        left.smooth_union(right, cx.scalar(18.0))
            .subtract(hole)
            .coverage()
            .mask(cx.sample(&material))
    })?;
    submit(&mut scene, bounds(33.0, 226.0, 150.0, 116.0), cutout);

    // 5. Deform only the material domain, leaving rounded geometry undistorted.
    let warped_material = root.shader(|cx| {
        let uv = cx.uv();
        let ripple = (uv.clone().y() * cx.scalar(16.0)).sin() * cx.scalar(0.18);
        let warped_uv = cx.xy(uv.clone().x() + ripple, uv.y());
        cx.call(&waves, (warped_uv, cx.uniform(phase), cx.uniform(tint)))
    })?;
    submit(
        &mut scene,
        bounds(226.0, 226.0, 148.0, 116.0),
        rounded_material(&root, &warped_material, 148.0, 116.0, 12.0, None)?,
    );

    // 6. Two instances share one program; uniforms independently tint and shift it.
    let paired = rounded_material(&root, &material, 65.0, 116.0, 25.0, None)?;
    let second = paired
        .with_parameter(phase, 2.3)?
        .with_parameter(tint, [0.99, 0.76, 0.23, 1.0])?;
    assert_eq!(paired.program_id(), second.program_id());
    submit(&mut scene, bounds(420.0, 226.0, 65.0, 116.0), paired);
    submit(&mut scene, bounds(499.0, 226.0, 65.0, 116.0), second);

    scene.finish();
    Ok(scene)
}

fn main() -> anyhow::Result<()> {
    let destination = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp/gpui-paint-gallery.png"));
    let mut renderer = WgpuHeadlessRenderer::new()?;
    let image =
        renderer.render_scene_to_image(&gallery()?, size(DevicePixels(600), DevicePixels(400)))?;
    image.save(&destination)?;
    println!("{}", destination.display());
    Ok(())
}
