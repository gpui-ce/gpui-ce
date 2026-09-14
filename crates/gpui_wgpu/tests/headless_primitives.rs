//! Renders one of every primitive kind through the headless WGPU renderer and checks
//! that each one actually lands on the target. This is the cross-platform smoke test for
//! the shared render plan, instance transport, and generated shaders.
#![cfg(feature = "test-support")]

use gpui::{
    AtlasKey, AtlasTile, BackdropFilter, BorderStyle, Bounds, ContentMask, Corners, DevicePixels,
    Edges, Hsla, MonochromeSprite, PlatformHeadlessRenderer, Point, PolychromeSprite, Quad,
    RenderImageParams, RenderSvgParams, ScaledFilter, ScaledPixels, Scene, ShaderBool, Shadow,
    Size, Underline, checkerboard, solid_background,
};
use gpui_ce_wgpu::WgpuHeadlessRenderer;
use smallvec::smallvec;
use std::borrow::Cow;

const TARGET: Size<DevicePixels> = Size {
    width: DevicePixels(200),
    height: DevicePixels(100),
};

fn bounds(x: f32, y: f32, w: f32, h: f32) -> Bounds<ScaledPixels> {
    Bounds {
        origin: Point {
            x: ScaledPixels(x),
            y: ScaledPixels(y),
        },
        size: Size {
            width: ScaledPixels(w),
            height: ScaledPixels(h),
        },
    }
}

fn full_mask() -> ContentMask<ScaledPixels> {
    ContentMask {
        bounds: bounds(0.0, 0.0, 200.0, 100.0),
    }
}

fn mask(width: f32, height: f32) -> ContentMask<ScaledPixels> {
    ContentMask {
        bounds: bounds(0.0, 0.0, width, height),
    }
}

fn tile(renderer: &WgpuHeadlessRenderer, key: AtlasKey, bytes: Vec<u8>) -> AtlasTile {
    renderer
        .sprite_atlas()
        .get_or_insert_with(&key, &mut || {
            Ok(Some((
                Size {
                    width: DevicePixels(8),
                    height: DevicePixels(8),
                },
                Cow::Owned(bytes.clone()),
            )))
        })
        .expect("atlas insert must succeed")
        .expect("atlas insert must produce a tile")
}

#[test]
fn every_primitive_kind_renders() {
    let mut renderer = WgpuHeadlessRenderer::new().expect("headless renderer");
    let mono = tile(
        &renderer,
        AtlasKey::Svg(RenderSvgParams {
            path: "test-mono".into(),
            size: Size {
                width: DevicePixels(8),
                height: DevicePixels(8),
            },
        }),
        vec![255; 64],
    );
    let poly = tile(
        &renderer,
        AtlasKey::Image(RenderImageParams {
            image_id: gpui::ImageId(1),
            frame_index: 0,
        }),
        // BGRA red, opaque.
        (0..64).flat_map(|_| [0u8, 0, 255, 255]).collect(),
    );

    let mut scene = Scene::default();
    let green: Hsla = gpui::rgb_to_hsla(gpui::rgb(0x00ff00));
    let white: Hsla = gpui::rgb_to_hsla(gpui::rgb(0xffffff));
    let blue: Hsla = gpui::rgb_to_hsla(gpui::rgb(0x0000ff));

    // 1. Solid quad.
    let quad_bounds = bounds(10.0, 10.0, 30.0, 30.0);
    scene.insert_primitive(Quad {
        order: 0,
        bounds: quad_bounds,
        content_mask: full_mask(),
        background: solid_background(green),
        ..Default::default()
    });
    // 2. Bordered quad (white border, transparent fill).
    let bordered = bounds(50.0, 10.0, 30.0, 30.0);
    scene.insert_primitive(Quad {
        order: 0,
        bounds: bordered,
        content_mask: full_mask(),
        border_color: white.into(),
        border_widths: gpui::Edges::all(ScaledPixels(4.0)),
        ..Default::default()
    });
    // 3. Shadow (blue, no blur so it is a solid block).
    let shadow_bounds = bounds(90.0, 10.0, 30.0, 30.0);
    scene.insert_primitive(Shadow {
        order: 0,
        blur_radius: ScaledPixels(0.0),
        bounds: shadow_bounds,
        corner_radii: Default::default(),
        content_mask: full_mask(),
        color: blue.into(),
        element_bounds: shadow_bounds,
        element_corner_radii: Default::default(),
        inset: ShaderBool::Disabled,
        corner_smoothing: 0.0,
    });
    // 4. Underline (white, solid).
    let underline_bounds = bounds(130.0, 20.0, 30.0, 4.0);
    scene.insert_primitive(Underline {
        order: 0,
        padding: 0,
        bounds: underline_bounds,
        content_mask: full_mask(),
        color: white.into(),
        thickness: ScaledPixels(4.0),
        wavy: ShaderBool::Disabled,
    });
    // 5. Monochrome sprite (white coverage tile tinted green).
    let mono_bounds = bounds(10.0, 60.0, 30.0, 30.0);
    scene.insert_primitive(MonochromeSprite {
        order: 0,
        padding: 0,
        bounds: mono_bounds,
        content_mask: full_mask(),
        color: green.into(),
        tile: mono,
        transformation: Default::default(),
    });
    // 6. Polychrome sprite (red image).
    let poly_bounds = bounds(50.0, 60.0, 30.0, 30.0);
    scene.insert_primitive(PolychromeSprite {
        order: 0,
        grayscale: ShaderBool::Disabled,
        opacity: 1.0,
        corner_smoothing: 0.0,
        bounds: poly_bounds,
        content_mask: full_mask(),
        corner_radii: Default::default(),
        tile: poly,
    });
    // 7. A second quad batch. Overlapping the image pushes this quad above it in draw
    //    order, so it starts past the first quads in the frame's quad buffer: a backend
    //    that loses the batch base draws the wrong quads here.
    let late_quad = bounds(60.0, 70.0, 30.0, 30.0);
    scene.insert_primitive(Quad {
        order: 0,
        bounds: late_quad,
        content_mask: full_mask(),
        background: solid_background(blue),
        ..Default::default()
    });
    scene.finish();
    let quad_batches: Vec<_> = scene
        .render_commands()
        .iter()
        .filter_map(|command| match command {
            gpui::RenderCommand::Batch(gpui::PrimitiveBatch::Quads { range, .. }) => {
                Some(range.clone())
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        quad_batches,
        vec![0..2, 2..3],
        "the scene must split quads across batches"
    );

    let image = renderer
        .render_scene_to_image(&scene, TARGET)
        .expect("render must succeed");
    let px = |x: u32, y: u32| {
        let p = image.get_pixel(x, y).0;
        (p[0], p[1], p[2], p[3])
    };
    let mut failures = Vec::new();
    let mut check = |name: &str, x: u32, y: u32, expected: (u8, u8, u8)| {
        let (r, g, b, _) = px(x, y);
        let close = |a: u8, b: u8| (a as i32 - b as i32).abs() <= 8;
        if !(close(r, expected.0) && close(g, expected.1) && close(b, expected.2)) {
            failures.push(format!(
                "{name} at ({x},{y}): got ({r},{g},{b}) expected {expected:?}"
            ));
        }
    };
    check("solid quad", 25, 25, (0, 255, 0));
    check("bordered quad border", 52, 25, (255, 255, 255));
    check("bordered quad interior", 65, 25, (0, 0, 0));
    check("shadow", 105, 25, (0, 0, 255));
    check("underline", 145, 22, (255, 255, 255));
    check("monochrome sprite", 25, 75, (0, 255, 0));
    check("polychrome sprite", 55, 65, (255, 0, 0));
    check("second quad batch", 85, 85, (0, 0, 255));
    check("background", 190, 90, (0, 0, 0));
    if !failures.is_empty() {
        let path = std::env::temp_dir().join("gpui_headless_primitives.png");
        image.save(&path).ok();
        panic!(
            "{}\n(image saved to {})",
            failures.join("\n"),
            path.display()
        );
    }
}

#[test]
fn smoothed_primitives_share_one_contour() {
    let mut renderer = WgpuHeadlessRenderer::new().expect("headless renderer");
    let image_tile = tile(
        &renderer,
        AtlasKey::Image(RenderImageParams {
            image_id: gpui::ImageId(2),
            frame_index: 0,
        }),
        (0..64).flat_map(|_| [0u8, 0, 255, 255]).collect(),
    );
    let target = Size {
        width: DevicePixels(360),
        height: DevicePixels(170),
    };
    let content_mask = mask(360.0, 170.0);
    let green: Hsla = gpui::rgb_to_hsla(gpui::rgb(0x19d36b));
    let white: Hsla = gpui::rgb_to_hsla(gpui::rgb(0xffffff));
    let blue: Hsla = gpui::rgb_to_hsla(gpui::rgb(0x287cff));
    let black: Hsla = gpui::rgb_to_hsla(gpui::rgb(0x000000));
    let radii = Corners::all(ScaledPixels(18.0));

    let mut scene = Scene::default();

    let fill_bounds = bounds(10.0, 10.0, 50.0, 50.0);
    scene.insert_primitive(Quad {
        bounds: fill_bounds,
        content_mask,
        background: solid_background(green),
        corner_radii: radii,
        corner_smoothing: 1.0,
        ..Default::default()
    });

    let border_bounds = bounds(72.0, 10.0, 50.0, 50.0);
    scene.insert_primitive(Quad {
        bounds: border_bounds,
        content_mask,
        border_color: white.into(),
        border_widths: Edges {
            top: ScaledPixels(2.0),
            right: ScaledPixels(5.0),
            bottom: ScaledPixels(8.0),
            left: ScaledPixels(3.0),
        },
        corner_radii: radii,
        corner_smoothing: 0.6,
        ..Default::default()
    });

    let dashed_bounds = bounds(134.0, 10.0, 58.0, 50.0);
    scene.insert_primitive(Quad {
        bounds: dashed_bounds,
        content_mask,
        border_style: BorderStyle::Dashed,
        border_color: white.into(),
        border_widths: Edges::all(ScaledPixels(3.0)),
        corner_radii: Corners {
            top_left: ScaledPixels(20.0),
            top_right: ScaledPixels(8.0),
            bottom_right: ScaledPixels(16.0),
            bottom_left: ScaledPixels(3.0),
        },
        corner_smoothing: 1.0,
        ..Default::default()
    });

    let image_bounds = bounds(204.0, 10.0, 50.0, 50.0);
    scene.insert_primitive(PolychromeSprite {
        order: 0,
        grayscale: ShaderBool::Disabled,
        opacity: 1.0,
        corner_smoothing: 1.0,
        bounds: image_bounds,
        content_mask,
        corner_radii: radii,
        tile: image_tile,
    });

    let drop_element = bounds(20.0, 96.0, 50.0, 44.0);
    scene.insert_primitive(Shadow {
        order: 0,
        blur_radius: ScaledPixels(5.0),
        bounds: bounds(25.0, 101.0, 50.0, 44.0),
        corner_radii: Corners::all(ScaledPixels(15.0)),
        content_mask,
        color: blue.into(),
        element_bounds: drop_element,
        element_corner_radii: Corners::all(ScaledPixels(15.0)),
        inset: ShaderBool::Disabled,
        corner_smoothing: 0.6,
    });
    scene.insert_primitive(Quad {
        bounds: drop_element,
        content_mask,
        background: solid_background(green),
        corner_radii: Corners::all(ScaledPixels(15.0)),
        corner_smoothing: 0.6,
        ..Default::default()
    });

    let inset_bounds = bounds(100.0, 94.0, 50.0, 48.0);
    scene.insert_primitive(Quad {
        bounds: inset_bounds,
        content_mask,
        background: solid_background(green),
        corner_radii: Corners::all(ScaledPixels(16.0)),
        corner_smoothing: 1.0,
        ..Default::default()
    });
    scene.insert_primitive(Shadow {
        order: 0,
        blur_radius: ScaledPixels(4.0),
        bounds: bounds(104.0, 98.0, 42.0, 40.0),
        corner_radii: Corners::all(ScaledPixels(12.0)),
        content_mask,
        color: black.into(),
        element_bounds: inset_bounds,
        element_corner_radii: Corners::all(ScaledPixels(16.0)),
        inset: ShaderBool::Enabled,
        corner_smoothing: 1.0,
    });

    let filter_bounds = bounds(190.0, 92.0, 70.0, 52.0);
    scene.insert_primitive(Quad {
        bounds: filter_bounds,
        content_mask,
        background: checkerboard(white, 2.0),
        ..Default::default()
    });
    scene.insert_primitive(BackdropFilter {
        order: 0,
        bounds: filter_bounds,
        content_mask,
        corner_radii: Corners::all(ScaledPixels(18.0)),
        corner_smoothing: 1.0,
        filters: smallvec![ScaledFilter::Blur(ScaledPixels(5.0))],
        opacity: 1.0,
    });

    scene.finish();
    let image = renderer
        .render_scene_to_image(&scene, target)
        .expect("render must succeed");
    let mut unfiltered_scene = Scene::default();
    unfiltered_scene.insert_primitive(Quad {
        bounds: filter_bounds,
        content_mask,
        background: checkerboard(white, 2.0),
        ..Default::default()
    });
    unfiltered_scene.finish();
    let unfiltered = renderer
        .render_scene_to_image(&unfiltered_scene, target)
        .expect("reference render must succeed");
    let pixel = |x: u32, y: u32| image.get_pixel(x, y).0;
    let reference_pixel = |x: u32, y: u32| unfiltered.get_pixel(x, y).0;
    let is_black = |x, y| pixel(x, y)[0..3].iter().all(|channel| *channel <= 12);
    let is_white = |x, y| pixel(x, y)[0..3].iter().all(|channel| *channel >= 235);

    assert!(is_black(10, 10), "smoothed fill must exclude its corner");
    assert!(
        pixel(35, 10)[1] > 180,
        "fill shoulder must reach the top edge"
    );
    assert!(pixel(35, 35)[1] > 180, "fill center");

    assert!(is_white(96, 10), "thin top border");
    assert!(
        is_black(96, 14),
        "top border must not grow to the right width"
    );
    assert!(is_white(120, 35), "right border");
    assert!(is_black(115, 35), "right border interior boundary");
    assert!(is_white(96, 56), "thick bottom border");
    assert!(is_black(96, 50), "bottom border interior boundary");

    let dashed_pixels = (8..62)
        .flat_map(|y| (132..194).map(move |x| (x, y)))
        .filter(|&(x, y)| is_white(x, y))
        .count();
    assert!(
        (150..420).contains(&dashed_pixels),
        "dashed contour coverage: {dashed_pixels}"
    );
    assert!(is_black(163, 35), "dashed border interior");

    assert!(is_black(204, 10), "smoothed image must exclude its corner");
    assert!(
        pixel(229, 10)[0] > 220,
        "image shoulder must reach the top edge"
    );
    assert!(pixel(229, 35)[0] > 220, "image center");

    assert!(
        pixel(73, 132)[2] > 35,
        "drop shadow must extend beyond the element"
    );
    assert!(
        pixel(125, 96)[1] < pixel(125, 118)[1],
        "inset shadow must darken the edge"
    );
    assert!(
        pixel(125, 118)[1] > 120,
        "inset shadow must preserve the center"
    );

    let blurred_midtones = (192..258)
        .flat_map(|x| (94..142).map(move |y| pixel(x, y)[0]))
        .filter(|channel| (30..225).contains(channel))
        .count();
    assert!(
        blurred_midtones > 800,
        "backdrop blur did not composite: {blurred_midtones}"
    );
    assert!(
        pixel(190, 92)
            .iter()
            .zip(reference_pixel(190, 92))
            .all(|(&actual, expected)| actual.abs_diff(expected) <= 1),
        "the smoothed mask must exclude the backdrop corner"
    );
    assert!(
        pixel(225, 92)[0].abs_diff(reference_pixel(225, 92)[0]) > 20,
        "the smoothed mask must include the top shoulder"
    );

    if std::env::var_os("GPUI_SAVE_HEADLESS_TESTS").is_some() {
        image
            .save(std::env::temp_dir().join("gpui_smoothed_primitives.png"))
            .expect("save diagnostic image");
    }
}
