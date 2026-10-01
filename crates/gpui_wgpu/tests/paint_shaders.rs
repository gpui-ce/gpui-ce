//! Typed graph validation and real GPU regression coverage. GPU failures are failures.
#![cfg(feature = "test-support")]
use gpui::{
    Bounds, ContentMask, DevicePixels, PlatformHeadlessRenderer, Point, Quad, ScaledPixels, Scene,
    ShaderQuad, Size,
    paint::{PaintRoot, Shader},
    solid_background,
};
use gpui_ce_wgpu::WgpuHeadlessRenderer;

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
fn shader_quad(shader: Shader, b: Bounds<ScaledPixels>) -> ShaderQuad {
    ShaderQuad {
        order: 0,
        bounds: b,
        content_mask: ContentMask {
            bounds: bounds(0., 0., 256., 256.),
        },
        shader,
        opacity: 1.,
        scale_factor: 1.,
    }
}
fn render(renderer: &mut WgpuHeadlessRenderer, mut scene: Scene) -> image::RgbaImage {
    scene.finish();
    renderer
        .render_scene_to_image(
            &scene,
            Size {
                width: DevicePixels(256),
                height: DevicePixels(256),
            },
        )
        .expect("GPU shader rendering must succeed")
}
fn check(image: &image::RgbaImage, x: u32, y: u32, rgb: [u8; 3]) {
    let actual = image.get_pixel(x, y).0;
    assert!(
        actual[..3]
            .iter()
            .zip(rgb)
            .all(|(&a, e)| a.abs_diff(e) <= 3),
        "at ({x},{y}) got {actual:?}, expected {rgb:?}"
    );
}
fn constant(color: [f32; 4]) -> Shader {
    PaintRoot::new().shader(|cx| cx.vec4(color)).unwrap()
}
fn validate(shader: &Shader) {
    let source = shader.wgsl();
    let module = naga::front::wgsl::parse_str(source)
        .unwrap_or_else(|e| panic!("{}", e.emit_to_string(source)));
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::empty(),
    )
    .validate(&module)
    .expect("typed builder must emit valid WGSL");
}

#[test]
fn all_typed_operator_permutations_validate() {
    let root = PaintRoot::new();
    for variant in 0..16 {
        let shader = root
            .shader(|cx| {
                let p = cx.parameter([0.2, 0.4, 0.6, 1.]);
                let value = cx.uniform(p);
                let a = cx.uv().x();
                let b = cx.uv().y();
                let x = match variant {
                    0 => a + b,
                    1 => a - b,
                    2 => a * b,
                    3 => a / (b + cx.scalar(1.)),
                    4 => -a,
                    5 => a.sin(),
                    6 => a.cos(),
                    7 => a.abs(),
                    8 => a.floor(),
                    9 => a.fract(),
                    10 => a.min(b),
                    11 => a.max(b),
                    12 => a.clamp(cx.scalar(0.), cx.scalar(1.)),
                    13 => a.mix(b, cx.scalar(0.5)),
                    14 => a.scale(b),
                    _ => a.smoothstep(cx.scalar(0.), cx.scalar(1.)),
                };
                value.mix(
                    cx.rgba(x.clone(), x.clone(), x, cx.scalar(1.)),
                    cx.scalar(0.5),
                )
            })
            .unwrap();
        validate(&shader);
    }
    validate(
        &root
            .shader(|cx| {
                let a = cx.parameter([1., 2.]);
                let b = cx.parameter([1., 2., 3.]);
                let v = cx.uniform(b);
                let x =
                    v.clone().dot(v.clone()) + v.clone().z() + v.length() + cx.uniform(a).length();
                cx.rgba(x.clone(), x.clone(), x, cx.scalar(1.))
            })
            .unwrap(),
    );
    validate(
        &root
            .shader(|cx| {
                let d = cx.rounded_rect(
                    cx.position(),
                    cx.vec2([20., 20.]),
                    cx.vec2([15., 15.]),
                    cx.scalar(4.),
                );
                cx.rgba(
                    cx.scalar(1.),
                    cx.scalar(0.),
                    cx.scalar(0.),
                    d.stroke(cx.scalar(2.), cx.scalar(1.)),
                )
            })
            .unwrap(),
    );
}

#[test]
fn shared_program_parameter_draws_interleave_and_update() {
    let mut renderer = WgpuHeadlessRenderer::new().expect("headless GPU required");
    let mut handle = None;
    let shader = PaintRoot::new()
        .shader(|cx| {
            let p = cx.parameter([1., 0., 0., 1.]);
            handle = Some(p);
            cx.uniform(p)
        })
        .unwrap();
    let green = shader
        .with_parameter(handle.unwrap(), [0., 1., 0., 1.])
        .unwrap();
    let mut scene = Scene::default();
    scene.insert_primitive(shader_quad(shader.clone(), bounds(10., 10., 60., 60.)));
    scene.insert_primitive(shader_quad(green.clone(), bounds(40., 40., 60., 60.)));
    scene.insert_primitive(Quad {
        bounds: bounds(60., 60., 60., 60.),
        content_mask: ContentMask {
            bounds: bounds(0., 0., 256., 256.),
        },
        background: solid_background(gpui::rgb(0x0000ff)),
        ..Default::default()
    });
    scene.insert_primitive(shader_quad(shader, bounds(80., 80., 60., 60.)));
    let image = render(&mut renderer, scene);
    check(&image, 20, 20, [255, 0, 0]);
    check(&image, 50, 50, [0, 255, 0]);
    check(&image, 70, 70, [0, 0, 255]);
    check(&image, 90, 90, [255, 0, 0]);
    let mut scene = Scene::default();
    scene.insert_primitive(shader_quad(green, bounds(10., 10., 60., 60.)));
    let before = renderer.paint_diagnostics();
    let image = render(&mut renderer, scene);
    check(&image, 20, 20, [0, 255, 0]);
    assert_eq!(
        renderer.paint_diagnostics().1,
        before.1,
        "parameter updates must reuse pipelines"
    );
}

#[test]
fn logical_coordinates_uv_clip_opacity_and_distance_masks() {
    let mut renderer = WgpuHeadlessRenderer::new().expect("headless GPU required");
    let root = PaintRoot::new();
    let mut scene = Scene::default();
    let gradient = root
        .shader(|cx| {
            cx.rgba(
                cx.uv().x(),
                cx.position().y() / cx.size().y(),
                cx.scalar(0.),
                cx.scalar(1.),
            )
        })
        .unwrap();
    let mut q = shader_quad(gradient, bounds(20., 30., 100., 80.));
    q.scale_factor = 2.;
    q.content_mask = ContentMask {
        bounds: bounds(20., 30., 50., 80.),
    };
    scene.insert_primitive(q);
    let mut q = shader_quad(constant([1., 0., 0., 0.5]), bounds(130., 30., 40., 40.));
    q.opacity = 0.5;
    scene.insert_primitive(q);
    let circle = root
        .shader(|cx| {
            let d = cx.circle(cx.position(), cx.vec2([20., 20.]), cx.scalar(15.));
            cx.rgba(
                cx.scalar(0.),
                cx.scalar(1.),
                cx.scalar(0.),
                d.fill(cx.scalar(1.)),
            )
        })
        .unwrap();
    scene.insert_primitive(shader_quad(circle, bounds(10., 130., 40., 40.)));
    let rounded = root
        .shader(|cx| {
            let d = cx.rounded_rect(
                cx.position(),
                cx.vec2([20., 20.]),
                cx.vec2([20., 20.]),
                cx.scalar(10.),
            );
            cx.rgba(
                cx.scalar(0.),
                cx.scalar(0.),
                cx.scalar(1.),
                d.fill(cx.scalar(1.)),
            )
        })
        .unwrap();
    scene.insert_primitive(shader_quad(rounded, bounds(70., 130., 40., 40.)));
    let image = render(&mut renderer, scene);
    check(&image, 45, 50, [65, 65, 0]);
    check(&image, 90, 50, [0, 0, 0]);
    check(&image, 150, 50, [64, 0, 0]);
    check(&image, 30, 150, [0, 255, 0]);
    check(&image, 10, 130, [0, 0, 0]);
    check(&image, 90, 150, [0, 0, 255]);
    check(&image, 70, 130, [0, 0, 0]);
}

#[test]
fn full_parameter_alignment_arena_growth_and_many_programs() {
    let mut renderer = WgpuHeadlessRenderer::new().expect("headless GPU required");
    let mut last = None;
    let shader = PaintRoot::new()
        .shader(|cx| {
            let mut color = cx.vec4([0.; 4]);
            for _ in 0..63 {
                let parameter = cx.parameter([0.; 4]);
                color = color + cx.uniform(parameter);
            }
            let p = cx.parameter([1., 0., 0., 1.]);
            last = Some(p);
            color + cx.uniform(p)
        })
        .unwrap();
    validate(&shader);
    assert_eq!(
        shader.parameter_slots().len(),
        64,
        "all 64 live aligned slots retained"
    );
    let green = shader
        .with_parameter(last.unwrap(), [0., 1., 0., 1.])
        .unwrap();
    let before = renderer.paint_diagnostics().2;
    let mut scene = Scene::default();
    for i in 0..512 {
        scene.insert_primitive(shader_quad(
            if i % 2 == 0 {
                shader.clone()
            } else {
                green.clone()
            },
            bounds((i % 32) as f32 * 8., (i / 32) as f32 * 8., 8., 8.),
        ));
    }
    let image = render(&mut renderer, scene);
    check(&image, 4, 4, [255, 0, 0]);
    check(&image, 12, 4, [0, 255, 0]);
    check(&image, 252, 124, [0, 255, 0]);
    assert!(
        renderer.paint_diagnostics().2 > before,
        "uniform arena must grow"
    );
    for i in 0..140 {
        let mut scene = Scene::default();
        scene.insert_primitive(shader_quad(
            constant([i as f32 / 140., 0., 0., 1.]),
            bounds(0., 0., 20., 20.),
        ));
        let image = render(&mut renderer, scene);
        check(
            &image,
            10,
            10,
            [(i as f32 / 140. * 255.).round() as u8, 0, 0],
        );
    }
    assert!(
        renderer.paint_diagnostics().0 <= 128,
        "inactive programs must be evicted"
    );
    let mut scene = Scene::default();
    scene.insert_primitive(shader_quad(shader, bounds(0., 0., 20., 20.)));
    scene.insert_primitive(shader_quad(green, bounds(30., 0., 0., 20.)));
    let image = render(&mut renderer, scene);
    check(&image, 10, 10, [255, 0, 0]);
    check(&image, 30, 10, [0, 0, 0]);
}

#[test]
fn procedural_paint_coexists_with_isolated_and_backdrop_blurs() {
    use gpui::{BackdropFilter, FilterBoundary, ScaledFilter};
    use smallvec::smallvec;
    let mut renderer = WgpuHeadlessRenderer::new().expect("headless GPU required");
    let b = bounds(20., 20., 100., 100.);
    let mask = ContentMask {
        bounds: bounds(0., 0., 256., 256.),
    };
    let boundary = |is_start| FilterBoundary {
        order: 0,
        bounds: b,
        content_mask: mask,
        corner_radii: Default::default(),
        corner_smoothing: 0.,
        filters: smallvec![ScaledFilter::Blur(ScaledPixels(3.))],
        opacity: 1.,
        is_start,
    };
    let mut scene = Scene::default();
    scene.insert_primitive(boundary(true));
    scene.insert_primitive(shader_quad(
        constant([1., 0., 0., 1.]),
        bounds(40., 40., 40., 40.),
    ));
    scene.insert_primitive(boundary(false));
    scene.insert_primitive(BackdropFilter {
        order: 0,
        bounds: b,
        content_mask: mask,
        corner_radii: Default::default(),
        corner_smoothing: 0.,
        filters: smallvec![ScaledFilter::Blur(ScaledPixels(2.))],
        opacity: 1.,
    });
    scene.insert_primitive(shader_quad(
        constant([0., 1., 0., 1.]),
        bounds(140., 20., 40., 40.),
    ));
    let image = render(&mut renderer, scene);
    check(&image, 60, 60, [255, 0, 0]);
    check(&image, 160, 40, [0, 255, 0]);
    assert!(
        image.get_pixel(39, 60).0[0] > 20,
        "isolated shader blur must spread beyond geometry"
    );
}

#[test]
fn vector_arithmetic_derivatives_and_mapped_parameters_validate() {
    let root = PaintRoot::new();
    for variant in 0..15 {
        let shader = root
            .shader(|cx| {
                let a = cx.vec4([0.2, 0.4, 0.6, 1.]);
                let b = cx.vec4([0.1; 4]);
                match variant {
                    0 => a + b,
                    1 => a - b,
                    2 => a * b,
                    3 => a / b,
                    4 => -a,
                    5 => a.sin(),
                    6 => a.cos(),
                    7 => a.abs(),
                    8 => a.floor(),
                    9 => a.fract(),
                    10 => a.min(b),
                    11 => a.max(b),
                    12 => a.clamp(b, cx.vec4([1.; 4])),
                    13 => a.mix(b, cx.scalar(0.5)),
                    _ => a.scale(cx.scalar(0.5)),
                }
            })
            .unwrap();
        validate(&shader);
    }
    validate(
        &root
            .shader(|cx| {
                cx.rgba(
                    cx.uv().x().fwidth(),
                    cx.scalar(0.),
                    cx.scalar(0.),
                    cx.scalar(1.),
                )
            })
            .unwrap(),
    );
    let mut handle = None;
    let base = root
        .shader(|cx| {
            let p = cx.parameter([1., 0., 0., 1.]);
            handle = Some(p);
            cx.uniform(p)
        })
        .unwrap();
    let mapped = base
        .map(|cx, color| color.mix(cx.vec4([0., 1., 0., 1.]), cx.scalar(0.5)))
        .unwrap();
    validate(&mapped);
    let updated = mapped
        .with_parameter(handle.unwrap(), [0., 0., 1., 1.])
        .unwrap();
    assert_eq!(mapped.program_id(), updated.program_id());
    assert_eq!(updated.parameter_slots()[0], [0., 0., 1., 1.]);
}

#[test]
fn fractional_scaled_bounds_clamp_alpha_and_restore_scissor() {
    let mut renderer = WgpuHeadlessRenderer::new().expect("headless GPU required");
    let shader = PaintRoot::new()
        .shader(|cx| {
            cx.rgba(
                cx.position().x() / cx.size().x(),
                cx.scalar(0.),
                cx.scalar(0.),
                cx.scalar(4.),
            )
        })
        .unwrap();
    let mut q = shader_quad(shader, bounds(10.25, 20.25, 100., 40.));
    q.scale_factor = 1.5;
    q.content_mask = ContentMask {
        bounds: bounds(30.25, 20.25, 30., 40.),
    };
    let mut scene = Scene::default();
    scene.insert_primitive(q);
    scene.insert_primitive(Quad {
        bounds: bounds(50., 40., 100., 40.),
        content_mask: ContentMask {
            bounds: bounds(0., 0., 256., 256.),
        },
        background: solid_background(gpui::rgb(0x00ff00)),
        ..Default::default()
    });
    let image = render(&mut renderer, scene);
    check(&image, 40, 30, [77, 0, 0]);
    check(&image, 30, 30, [52, 0, 0]);
    check(&image, 60, 30, [0, 0, 0]);
    check(&image, 130, 60, [0, 255, 0]);
}

#[test]
fn extreme_finite_runtime_arithmetic_validates() {
    let root = PaintRoot::new();
    for variant in 0..4 {
        let shader = root
            .shader(|cx| {
                let x = match variant {
                    0 => cx.scalar(1.) / cx.scalar(0.),
                    1 => cx.scalar(f32::MAX) * cx.scalar(f32::MAX),
                    2 => cx.scalar(0.5).smoothstep(cx.scalar(0.5), cx.scalar(0.5)),
                    _ => cx.scalar(1.).clamp(cx.scalar(2.), cx.scalar(1.)),
                };
                cx.rgba(x.clone(), x.clone(), x, cx.scalar(1.))
            })
            .unwrap();
        validate(&shader);
    }
}

#[test]
fn independent_materials_compose_share_parameters_and_remap_domains() {
    let mut renderer = WgpuHeadlessRenderer::new().expect("headless GPU required");
    let root = PaintRoot::new();
    let tint = root.parameter([1., 0., 0., 1.]).unwrap();
    let material = root.shader(|cx| cx.uniform(tint)).unwrap();
    let uv_material = root
        .shader(|cx| {
            cx.rgba(
                cx.uv().x(),
                cx.position().y() / cx.size().y(),
                cx.scalar(0.),
                cx.scalar(1.),
            )
        })
        .unwrap();
    let remapped = root
        .shader(|cx| cx.sample_uv(&uv_material, cx.vec2([0.25, 0.75])))
        .unwrap();
    let local = root
        .shader(|cx| cx.sample_at(&uv_material, cx.vec2([10., 20.]), cx.vec2([40., 40.])))
        .unwrap();
    let composed = root
        .shader(|cx| {
            cx.sample(&material)
                .mix(cx.sample(&uv_material), cx.scalar(0.5))
        })
        .unwrap();
    validate(&remapped);
    validate(&local);
    validate(&composed);
    assert!(composed.has_parameter(tint));
    let mut scene = Scene::default();
    scene.insert_primitive(shader_quad(remapped, bounds(0., 0., 40., 40.)));
    scene.insert_primitive(shader_quad(local, bounds(50., 0., 40., 40.)));
    scene.insert_primitive(shader_quad(
        composed.with_parameter(tint, [0., 0., 1., 1.]).unwrap(),
        bounds(100., 0., 40., 40.),
    ));
    let drawing = root
        .canvas(|canvas| {
            canvas
                .rect(gpui::Bounds::new(
                    gpui::Point::default(),
                    gpui::size(gpui::px(40.), gpui::px(40.)),
                ))
                .fill(&material);
            canvas
                .circle(gpui::point(gpui::px(60.), gpui::px(60.)), gpui::px(20.))
                .stroke(&material, gpui::px(3.));
        })
        .unwrap();
    let animated = drawing.with_parameter(tint, [0., 1., 0., 1.]).unwrap();
    assert_eq!(animated.primitive_count(), drawing.primitive_count());
    let image = render(&mut renderer, scene);
    check(&image, 20, 20, [64, 131, 0]);
    check(&image, 70, 20, [64, 128, 0]);
    check(&image, 120, 20, [65, 65, 128]);
}

#[test]
fn semantic_geometry_and_coverage_permutations_validate_and_render() {
    let root = PaintRoot::new();
    let mut programs = Vec::new();
    for variant in 0..9 {
        let shader = root
            .shader(|cx| {
                let a = cx.circle_field(cx.position(), cx.vec2([20., 20.]), cx.scalar(16.));
                let b = cx.circle_field(cx.position(), cx.vec2([28., 20.]), cx.scalar(10.));
                let field = match variant {
                    0 => a.union(b),
                    1 => a.intersect(b),
                    2 => a.subtract(b),
                    3 => a.offset(cx.scalar(2.)),
                    4 => a.dilate(cx.scalar(-2.)),
                    5 => a.inset(cx.scalar(4.)),
                    6 => a.smooth_union(b, cx.scalar(0.)),
                    7 => a.inward_stroke(cx.scalar(4.)),
                    _ => a.centered_stroke(cx.scalar(4.)),
                };
                field.coverage().mask(cx.vec4([0., 1., 0., 1.]))
            })
            .unwrap();
        validate(&shader);
        programs.push(shader);
    }
    validate(
        &root
            .shader(|cx| {
                gpui::paint::Coverage::new(cx.uv().x())
                    .invert()
                    .multiply(gpui::paint::Coverage::new(cx.uv().y()))
                    .mask(cx.vec4([1.; 4]))
            })
            .unwrap(),
    );
    let mut renderer = WgpuHeadlessRenderer::new().expect("headless GPU required");
    let mut scene = Scene::default();
    scene.insert_primitive(shader_quad(programs[2].clone(), bounds(0., 0., 40., 40.)));
    scene.insert_primitive(shader_quad(programs[7].clone(), bounds(50., 0., 40., 40.)));
    let image = render(&mut renderer, scene);
    check(&image, 10, 20, [0, 255, 0]);
    check(&image, 28, 20, [0, 0, 0]);
    check(&image, 55, 20, [0, 255, 0]);
    check(&image, 70, 20, [0, 0, 0]);
}

#[test]
fn complete_import_relocates_parameters_from_independent_materials() {
    let root = PaintRoot::new();
    let mut a_handle = None;
    let a = root
        .shader(|cx| {
            let p = cx.parameter([1., 0., 0., 1.]);
            a_handle = Some(p);
            cx.uniform(p)
        })
        .unwrap();
    let mut b_handle = None;
    let b = root
        .shader(|cx| {
            let p = cx.parameter([0., 1., 0., 1.]);
            b_handle = Some(p);
            cx.uniform(p)
        })
        .unwrap();
    let combined = root
        .shader(|cx| cx.sample(&a).mix(cx.sample(&b), cx.scalar(0.5)))
        .unwrap();
    assert_eq!(combined.parameter_slots().len(), 2);
    let updated = combined
        .with_parameter(a_handle.unwrap(), [0., 0., 1., 1.])
        .unwrap()
        .with_parameter(b_handle.unwrap(), [1., 0., 0., 1.])
        .unwrap();
    validate(&updated);
    let batched = combined
        .with_parameters(|p| {
            p.set(a_handle.unwrap(), [0., 0., 1., 1.])
                .set(b_handle.unwrap(), [1., 0., 0., 1.]);
        })
        .unwrap();
    assert_eq!(updated, batched);
    assert_eq!(combined.program_id(), batched.program_id());
    assert_eq!(combined.wgsl().as_ptr(), batched.wgsl().as_ptr());
    let updated = batched;
    let mut renderer = WgpuHeadlessRenderer::new().expect("headless GPU required");
    let mut scene = Scene::default();
    scene.insert_primitive(shader_quad(updated, bounds(0., 0., 40., 40.)));
    let image = render(&mut renderer, scene);
    check(&image, 20, 20, [128, 0, 128]);
    let foreign = PaintRoot::new().parameter(0.5).unwrap();
    assert!(matches!(
        combined.with_parameter(foreign, 1.),
        Err(gpui::paint::ShaderError::ForeignGraph)
    ));
    let conflicting = a
        .with_parameter(a_handle.unwrap(), [0., 0., 1., 1.])
        .unwrap();
    assert!(matches!(
        root.shader(|cx| cx.sample(&a) + cx.sample(&conflicting)),
        Err(gpui::paint::ShaderError::AmbiguousParameter)
    ));
}

#[test]
fn animated_nonpositive_stroke_widths_are_fully_transparent() {
    let root = PaintRoot::new();
    let width = root.parameter(0.).unwrap();
    let mut renderer = WgpuHeadlessRenderer::new().expect("headless GPU required");
    for centered in [false, true] {
        let shader = root
            .shader(|cx| {
                let field = cx.circle_field(cx.position(), cx.vec2([20.5, 20.5]), cx.scalar(15.));
                let stroke = if centered {
                    field.centered_stroke(cx.uniform(width))
                } else {
                    field.inward_stroke(cx.uniform(width))
                };
                stroke.coverage().mask(cx.vec4([0., 1., 0., 1.]))
            })
            .unwrap();
        validate(&shader);
        for value in [0., -1., 4.] {
            let mut scene = Scene::default();
            scene.insert_primitive(shader_quad(
                shader.with_parameter(width, value).unwrap(),
                bounds(0., 0., 41., 41.),
            ));
            let image = render(&mut renderer, scene);
            if value <= 0. {
                assert!(
                    image.pixels().all(|pixel| pixel.0[..3] == [0, 0, 0]),
                    "width{value} centered{centered} must exclude even exact boundary pixels"
                );
            } else {
                assert!(
                    image.get_pixel(6, 20).0[1] > 180,
                    "positive stroke must show border"
                );
                check(&image, 20, 20, [0, 0, 0]);
            }
        }
    }
}

#[test]
fn typed_predicate_select_validates_scalar_and_vector_branches() {
    let root = PaintRoot::new();
    for vector in [false, true] {
        validate(
            &root
                .shader(|cx| {
                    let condition = cx.uv().x().gt(cx.scalar(0.5));
                    if vector {
                        condition.select(cx.vec4([1., 0., 0., 1.]), cx.vec4([0., 1., 0., 1.]))
                    } else {
                        let x = condition.select(cx.scalar(1.), cx.scalar(0.));
                        cx.rgba(x.clone(), x.clone(), x, cx.scalar(1.))
                    }
                })
                .unwrap(),
        );
    }
}

#[test]
fn reusable_functions_fuse_to_identical_shader_and_gpu_pipeline() {
    let root = PaintRoot::new();
    let function = root
        .function::<(f32, [f32; 2]), [f32; 4]>(|cx, (phase, uv)| {
            cx.rgba(
                (uv.x() + phase).sin(),
                cx.scalar(0.2),
                cx.scalar(0.8),
                cx.scalar(1.),
            )
        })
        .unwrap();
    let phase = root.parameter(0.4).unwrap();
    let composed = root
        .shader(|cx| cx.call(&function, (cx.uniform(phase), cx.uv())))
        .unwrap();
    let handwritten = root
        .shader(|cx| {
            cx.rgba(
                (cx.uv().x() + cx.uniform(phase)).sin(),
                cx.scalar(0.2),
                cx.scalar(0.8),
                cx.scalar(1.),
            )
        })
        .unwrap();
    assert_eq!(
        composed.wgsl(),
        handwritten.wgsl(),
        "functions must disappear into the canonical graph"
    );
    assert_eq!(composed.program_id(), handwritten.program_id());
    assert_eq!(composed.node_count(), handwritten.node_count());
    validate(&composed);
    let mut renderer = WgpuHeadlessRenderer::new().expect("headless GPU required");
    let mut scene = Scene::default();
    scene.insert_primitive(shader_quad(composed, bounds(0., 0., 40., 40.)));
    let composed_image = render(&mut renderer, scene);
    let compiled = renderer.paint_diagnostics().1;
    let mut scene = Scene::default();
    scene.insert_primitive(shader_quad(handwritten, bounds(0., 0., 40., 40.)));
    let handwritten_image = render(&mut renderer, scene);
    assert_eq!(composed_image, handwritten_image);
    assert_eq!(
        renderer.paint_diagnostics().1,
        compiled,
        "handwritten and function graphs share one GPU pipeline"
    );
    let shape = gpui::paint::Shape::Circle {
        center: gpui::point(gpui::px(50.), gpui::px(70.)),
        radius: gpui::px(10.),
    };
    let field = root
        .function::<[f32; 2], gpui::paint::Distance>(|cx, point| shape.field_at(cx, point))
        .unwrap();
    let distance = root
        .shader(|cx| {
            let d = cx.call(&field, cx.vec2([13., 14.])).expression();
            cx.rgba(
                -d / cx.scalar(10.),
                cx.scalar(0.),
                cx.scalar(0.),
                cx.scalar(1.),
            )
        })
        .unwrap();
    validate(&distance);
    let mut scene = Scene::default();
    scene.insert_primitive(shader_quad(distance, bounds(0., 0., 40., 40.)));
    check(&render(&mut renderer, scene), 20, 20, [128, 0, 0]);
}
