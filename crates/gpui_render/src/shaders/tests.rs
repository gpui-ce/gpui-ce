use super::*;

#[test]
fn validates_subpixel_shader() {
    let generated = subpixel_sprite::WGSL_SOURCE.wgsl_source().unwrap();
    wgsl_rs::validate_wgsl_source(&format!("enable dual_source_blending;\n{generated}")).unwrap();
}

#[test]
fn build_generated_shaders_match_rust_sources() {
    assert_eq!(
        base::WGSL_SOURCE.wgsl_source().unwrap(),
        crate::artifacts::BASE_WGSL
    );
    let subpixel = subpixel_sprite::WGSL_SOURCE.wgsl_source().unwrap();
    assert_eq!(
        format!("enable dual_source_blending;\n{subpixel}"),
        crate::artifacts::SUBPIXEL_DUAL_SOURCE_WGSL
    );
}

#[test]
fn shader_interface_matches_generated_sources() {
    let standard_source = base::WGSL_SOURCE.wgsl_source().unwrap();
    let subpixel_source = subpixel_sprite::WGSL_SOURCE.wgsl_source().unwrap();
    for pipeline in interface::ALL {
        let source = if pipeline.label == interface::SUBPIXEL_SPRITES.label {
            &subpixel_source
        } else {
            &standard_source
        };
        assert!(
            source.contains(&format!("fn {}(", pipeline.vertex_entry)),
            "missing vertex entry point {}",
            pipeline.vertex_entry,
        );
        assert!(
            source.contains(&format!("fn {}(", pipeline.fragment_entry)),
            "missing fragment entry point {}",
            pipeline.fragment_entry,
        );
    }

    assert_eq!(common::GLOBALS.group, interface::GLOBAL_BIND_GROUP);
    assert_eq!(common::GLOBALS.binding, interface::GLOBAL_UNIFORMS_BINDING);
    assert_eq!(
        common::FONT_RASTERIZATION.group,
        interface::GLOBAL_BIND_GROUP
    );
    assert_eq!(
        common::FONT_RASTERIZATION.binding,
        interface::FONT_RASTERIZATION_BINDING
    );
    assert_eq!(quad::QUADS.group(), interface::DATA_BIND_GROUP);
    assert_eq!(quad::QUADS.binding(), interface::DATA_BUFFER_BINDING);
}

fn vertex_output_shape(module: &naga::Module, entry_name: &str) -> (usize, u32) {
    let entry = module
        .entry_points
        .iter()
        .find(|entry| entry.name == entry_name)
        .unwrap_or_else(|| panic!("missing shader entry point {entry_name}"));
    let result = entry
        .function
        .result
        .as_ref()
        .unwrap_or_else(|| panic!("shader entry point {entry_name} has no output"));
    let naga::TypeInner::Struct { members, .. } = &module.types[result.ty].inner else {
        panic!("shader entry point {entry_name} must return a struct");
    };

    members
        .iter()
        .filter(|member| matches!(member.binding, Some(naga::Binding::Location { .. })))
        .fold((0, 0), |(locations, components), member| {
            let member_components = match module.types[member.ty].inner {
                naga::TypeInner::Scalar(_) => 1,
                naga::TypeInner::Vector { size, .. } => u32::from(size),
                ref ty => panic!("unsupported stage output type {ty:?} in {entry_name}"),
            };
            (locations + 1, components + member_components)
        })
}

#[test]
fn ordinary_vertex_interfaces_stay_within_compact_budgets() {
    let module = naga::front::wgsl::parse_str(&base::WGSL_SOURCE.wgsl_source().unwrap()).unwrap();
    for (entry, maximum_shape) in [
        ("vertex_quad", (8, 29)),
        ("vertex_shadow", (5, 17)),
        ("vertex_polychrome_sprite", (3, 7)),
        ("vertex_blur_composite", (2, 6)),
    ] {
        let actual_shape = vertex_output_shape(&module, entry);
        assert!(
            actual_shape.0 <= maximum_shape.0 && actual_shape.1 <= maximum_shape.1,
            "{entry} uses {actual_shape:?}, exceeding {maximum_shape:?} (locations, components)"
        );
    }
}

#[test]
fn scene_instance_storage_sizes_are_bounded() {
    for (name, actual, maximum) in [
        ("Quad", std::mem::size_of::<gpui::Quad>(), 248),
        ("Shadow", std::mem::size_of::<gpui::Shadow>(), 184),
        (
            "PolychromeSprite",
            std::mem::size_of::<gpui::PolychromeSprite>(),
            112,
        ),
    ] {
        assert!(
            actual <= maximum,
            "{name} is {actual} bytes, exceeding its {maximum}-byte storage budget"
        );
    }
}

#[test]
fn shader_buffer_layouts_match_host_layouts() {
    let modules = [
        crate::artifacts::BASE_WGSL,
        crate::artifacts::SUBPIXEL_DUAL_SOURCE_WGSL,
    ]
    .map(|source| naga::front::wgsl::parse_str(source).unwrap());

    for layouts in [gpui::SCENE_BUFFER_LAYOUTS, interface::RENDER_BUFFER_LAYOUTS] {
        for layout in layouts {
            let ty = modules
                .iter()
                .find_map(|module| {
                    module
                        .types
                        .iter()
                        .find_map(|(_, ty)| (ty.name.as_deref() == Some(layout.name)).then_some(ty))
                })
                .unwrap_or_else(|| panic!("missing shader ABI type {}", layout.name));
            let naga::TypeInner::Struct { members, span } = &ty.inner else {
                panic!("shader ABI type {} must be a struct", layout.name);
            };

            assert_eq!(
                *span as usize, layout.size,
                "shader ABI size: {}",
                layout.name
            );
            assert_eq!(
                members.len(),
                layout.fields.len(),
                "shader ABI fields: {}",
                layout.name
            );
            for (member, (name, offset)) in members.iter().zip(layout.fields) {
                assert_eq!(
                    member.name.as_deref(),
                    Some(*name),
                    "shader ABI field: {}",
                    layout.name
                );
                assert_eq!(
                    member.offset as usize, *offset,
                    "shader ABI offset: {}.{name}",
                    layout.name
                );
            }
        }
    }
}

#[test]
fn shader_discriminants_match_scene_types() {
    assert_eq!(common::ShaderBool::Disabled as u32, 0);
    assert_eq!(common::ShaderBool::Enabled as u32, 1);
    assert_eq!(
        common::BackgroundTag::Solid as u32,
        gpui::BackgroundTag::Solid as u32
    );
    assert_eq!(
        common::BackgroundTag::LinearGradient as u32,
        gpui::BackgroundTag::LinearGradient as u32
    );
    assert_eq!(
        common::BackgroundTag::PatternSlash as u32,
        gpui::BackgroundTag::PatternSlash as u32
    );
    assert_eq!(
        common::BackgroundTag::Checkerboard as u32,
        gpui::BackgroundTag::Checkerboard as u32
    );
    assert_eq!(
        common::BackgroundTag::RadialGradient as u32,
        gpui::BackgroundTag::RadialGradient as u32
    );
    assert_eq!(
        common::BackgroundTag::AngularGradient as u32,
        gpui::BackgroundTag::AngularGradient as u32
    );
    assert_eq!(
        common::BackgroundTag::DiamondGradient as u32,
        gpui::BackgroundTag::DiamondGradient as u32
    );
    assert_eq!(
        common::ColorSpace::Srgb as u32,
        gpui::ColorSpace::Srgb as u32
    );
    assert_eq!(
        common::ColorSpace::Oklab as u32,
        gpui::ColorSpace::Oklab as u32
    );
    assert_eq!(
        common::BorderStyle::Solid as u32,
        gpui::BorderStyle::Solid as u32
    );
    assert_eq!(
        common::BorderStyle::Dashed as u32,
        gpui::BorderStyle::Dashed as u32
    );
}

#[test]
fn linear_gradients_preserve_native_dithering() {
    use common::*;
    use wgsl_rs::std::*;

    let color = Hsla {
        h: 0.0,
        s: 0.0,
        l: 0.5,
        a: 0.5,
    };
    let background = Background {
        tag: BackgroundTag::LinearGradient,
        color_space: ColorSpace::Srgb,
        solid: color,
        gradient_angle_or_pattern_height: 90.0,
        colors: [
            LinearColorStop {
                color,
                percentage: 0.0,
            },
            LinearColorStop {
                color,
                percentage: 1.0,
            },
        ],
        padding: 0,
    };
    let bounds = Bounds {
        origin: vec2f(0.0, 0.0),
        size: vec2f(100.0, 100.0),
    };
    let paint = Paint::new(background, bounds);
    let prepared = prepare_paint(paint);
    let first = paint_color(paint, vec2f(10.0, 10.0), prepared);
    let second = paint_color(paint, vec2f(11.0, 10.0), prepared);

    assert_ne!(first.w, second.w);
}

fn gradient_paint(tag: common::BackgroundTag, angle: f32, stops: [f32; 2]) -> common::Paint {
    use common::*;
    use wgsl_rs::std::*;

    Paint::new(
        Background {
            tag,
            color_space: ColorSpace::Srgb,
            solid: Hsla {
                h: 0.0,
                s: 0.0,
                l: 0.0,
                a: 0.0,
            },
            gradient_angle_or_pattern_height: angle,
            colors: [
                LinearColorStop {
                    color: Hsla {
                        h: 0.0,
                        s: 0.0,
                        l: 0.0,
                        a: 0.25,
                    },
                    percentage: stops[0],
                },
                LinearColorStop {
                    color: Hsla {
                        h: 0.0,
                        s: 0.0,
                        l: 1.0,
                        a: 0.75,
                    },
                    percentage: stops[1],
                },
            ],
            padding: 0,
        },
        Bounds {
            origin: vec2f(10.0, 20.0),
            size: vec2f(200.0, 100.0),
        },
    )
}

#[test]
fn gradient_geometries_sample_translated_non_square_bounds() {
    use common::*;
    use wgsl_rs::std::*;

    // Expected positions come from the geometries, independently of color conversion.
    for (tag, angle, samples) in [
        (
            BackgroundTag::LinearGradient,
            90.0,
            vec![
                (vec2f(10.0, 70.0), 0.0),
                (vec2f(110.0, 70.0), 0.5),
                (vec2f(210.0, 70.0), 1.0),
            ],
        ),
        (
            BackgroundTag::RadialGradient,
            0.0,
            vec![
                (vec2f(110.0, 70.0), 0.0),
                (vec2f(160.0, 70.0), 0.5),
                (vec2f(110.0, 95.0), 0.5),
                (vec2f(210.0, 70.0), 1.0),
                (vec2f(110.0, 120.0), 1.0),
            ],
        ),
        (
            BackgroundTag::AngularGradient,
            0.0,
            vec![
                (vec2f(110.0, 70.0), 0.0),
                (vec2f(110.0, 20.0), 0.0),
                (vec2f(210.0, 70.0), 0.25),
                (vec2f(110.0, 120.0), 0.5),
                (vec2f(10.0, 70.0), 0.75),
                (vec2f(150.0, 110.0), 0.375),
            ],
        ),
        (
            BackgroundTag::AngularGradient,
            450.0,
            vec![
                (vec2f(210.0, 70.0), 0.0),
                (vec2f(110.0, 120.0), 0.25),
                (vec2f(110.0, 20.0), 0.75),
            ],
        ),
        (
            BackgroundTag::DiamondGradient,
            90.0,
            vec![
                (vec2f(110.0, 70.0), 0.0),
                (vec2f(135.0, 82.5), 0.5),
                (vec2f(160.0, 95.0), 1.0),
            ],
        ),
        (
            BackgroundTag::DiamondGradient,
            135.0,
            vec![(vec2f(160.0, 70.0), std::f32::consts::FRAC_1_SQRT_2)],
        ),
    ] {
        let paint = gradient_paint(tag, angle, [0.0, 1.0]);
        for (position, expected) in samples {
            let actual = gradient_ratio(paint, position);
            assert!(
                (actual - expected).abs() < 0.00001,
                "{tag:?} at {position:?}, angle {angle}: {actual} != {expected}"
            );
        }
    }
}

#[test]
fn non_linear_gradients_preserve_stops_color_space_and_alpha() {
    use common::*;
    use wgsl_rs::std::*;

    for (tag, positions) in [
        (
            BackgroundTag::RadialGradient,
            [vec2f(110.0, 70.0), vec2f(160.0, 70.0), vec2f(210.0, 70.0)],
        ),
        (
            BackgroundTag::AngularGradient,
            [vec2f(210.0, 70.0), vec2f(110.0, 120.0), vec2f(10.0, 70.0)],
        ),
        (
            BackgroundTag::DiamondGradient,
            [vec2f(110.0, 70.0), vec2f(135.0, 82.5), vec2f(160.0, 95.0)],
        ),
    ] {
        for space in [ColorSpace::Srgb, ColorSpace::Oklab] {
            let mut paint = gradient_paint(tag, 0.0, [0.25, 0.75]);
            paint.background.color_space = space;
            let prepared = prepare_paint(paint);
            // Preserve the renderer's existing interpolation, including its legacy 2.2
            // transfer curve for Oklab. Halfway between black and white differs by space.
            let midpoint_gray = if space == ColorSpace::Srgb {
                0.214041
            } else {
                0.388602
            };
            for (position, (expected_alpha, expected_gray)) in
                positions
                    .into_iter()
                    .zip([(0.25, 0.0), (0.5, midpoint_gray), (0.75, 1.0)])
            {
                let color = paint_color(paint, position, prepared);
                for channel in [color.x, color.y, color.z] {
                    assert!(
                        (channel - expected_gray).abs() < 0.01,
                        "{tag:?} in {space:?} at {position:?}: {channel} != gray {expected_gray}"
                    );
                }
                assert!(
                    (color.w - expected_alpha).abs() < 0.02,
                    "{tag:?} in {space:?} at {position:?}: alpha {} != {expected_alpha}",
                    color.w
                );
            }
        }
    }

    let paint = gradient_paint(BackgroundTag::AngularGradient, 0.0, [0.0, 1.0]);
    // First and last colors differ, so the angular gradient deliberately keeps its seam.
    assert!(gradient_ratio(paint, vec2f(109.9, 20.0)) > 0.99);
    assert!(gradient_ratio(paint, vec2f(110.1, 20.0)) < 0.01);
}
