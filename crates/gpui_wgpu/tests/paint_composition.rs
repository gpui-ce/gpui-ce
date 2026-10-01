//! Consumer stress: reusable animated materials built exclusively through the public API.
#![cfg(feature = "test-support")]
use gpui::paint::{Distance, Function, PaintRoot, Parameter, Shader, ShaderError};

type HeroInput = ([f32; 2], f32, [f32; 2], [f32; 4]);
struct HeroParameters {
    phase: Parameter<f32>,
    offset: Parameter<[f32; 2]>,
    tint: Parameter<[f32; 4]>,
}
impl HeroParameters {
    fn new(root: &PaintRoot, seed: f32) -> Self {
        Self {
            phase: root.parameter(seed).unwrap(),
            offset: root.parameter([seed, 0.]).unwrap(),
            tint: root.parameter([0.2, 0.5, 0.9, 1.]).unwrap(),
        }
    }
    fn animate(&self, shader: &Shader, frame: usize, order: usize) -> Shader {
        let mut result = shader.clone();
        // Permute writes to mixed-width slots: updates must commute.
        for index in [(order % 3), ((order + 1) % 3), ((order + 2) % 3)] {
            result = match index {
                0 => result
                    .with_parameter(self.phase, frame as f32 * 0.01)
                    .unwrap(),
                1 => result
                    .with_parameter(self.offset, [frame as f32 * 0.02, 0.25])
                    .unwrap(),
                _ => result
                    .with_parameter(self.tint, [0.7, 0.2, 0.4, 0.8])
                    .unwrap(),
            };
        }
        result
    }
}
fn hero(root: &PaintRoot) -> Function<HeroInput, [f32; 4]> {
    let warp = root
        .function::<([f32; 2], f32), [f32; 2]>(|cx, (point, phase)| {
            let bend = (point.clone().y().scale(cx.scalar(0.02)) + phase).sin();
            point + cx.xy(bend.scale(cx.scalar(8.)), cx.scalar(0.))
        })
        .unwrap();
    let rim = root.function::<([f32; 2], [f32; 2]), Distance>(|cx, (p, offset)| {
        cx.circle_field(p, offset + cx.vec2([48., 48.]), cx.scalar(34.))
            .subtract(cx.circle_field(cx.position(), cx.vec2([48., 48.]), cx.scalar(20.)))
    });
    // Catch accidental ambient coordinate capture while extracting a component.
    assert!(matches!(rim, Err(ShaderError::CapturedInput)));
    root.function::<HeroInput, [f32; 4]>(|cx, (point, phase, offset, tint)| {
        let p = cx.call(&warp, (point, phase));
        let outer = cx.circle_field(
            p.clone(),
            offset.clone() + cx.vec2([48., 48.]),
            cx.scalar(34.),
        );
        let hole = cx.circle_field(p, offset + cx.vec2([48., 48.]), cx.scalar(20.));
        outer.subtract(hole).coverage().mask(tint)
    })
    .unwrap()
}
fn material(
    root: &PaintRoot,
    component: &Function<HeroInput, [f32; 4]>,
    p: &HeroParameters,
) -> Shader {
    root.shader(|cx| {
        cx.call(
            component,
            (
                cx.position(),
                cx.uniform(p.phase),
                cx.uniform(p.offset),
                cx.uniform(p.tint),
            ),
        )
    })
    .unwrap()
}
fn validate(shader: &Shader) {
    let module = naga::front::wgsl::parse_str(shader.wgsl()).unwrap();
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::empty(),
    )
    .validate(&module)
    .unwrap();
}
#[test]
fn independent_hero_instances_survive_portable_nested_domains_and_permuted_updates() {
    let source_root = PaintRoot::new();
    let component = hero(&source_root);
    let destination = PaintRoot::new();
    let left = HeroParameters::new(&destination, 0.);
    let right = HeroParameters::new(&destination, 0.7);
    let a = material(&destination, &component, &left);
    let b = material(&destination, &component, &right);
    assert_eq!(a.program_id(), b.program_id());
    let a_original = a.parameter_slots().to_vec();
    let b_original = b.parameter_slots().to_vec();
    let portable = PaintRoot::new();
    let composed = portable
        .shader(|cx| {
            let p = cx.position();
            let first = cx.sample_at(&a, p.clone() - cx.vec2([12., 8.]), cx.vec2([96., 96.]));
            let second = cx.sample_at(&b, p - cx.vec2([108., 8.]), cx.vec2([96., 96.]));
            first.mix(second, cx.uv().x())
        })
        .unwrap();
    assert_eq!(composed.parameter_slots().len(), 6);
    validate(&composed);
    for frame in 0..128 {
        let expected = right.animate(&left.animate(&composed, frame, 0), frame + 3, 0);
        for order in 1..3 {
            let updated = left.animate(&right.animate(&composed, frame + 3, order), frame, order);
            assert_eq!(updated, expected);
            assert_eq!(updated.program_id(), composed.program_id());
            assert_eq!(updated.node_count(), composed.node_count());
            assert_eq!(updated.wgsl().as_ptr(), composed.wgsl().as_ptr());
        }
    }
    assert_eq!(a.parameter_slots(), a_original);
    assert_eq!(b.parameter_slots(), b_original);
}
#[test]
fn repeated_identical_calls_share_nodes_but_distinct_domains_do_not_collapse() {
    let root = PaintRoot::new();
    let component = hero(&root);
    let p = HeroParameters::new(&root, 0.);
    let base = material(&root, &component, &p);
    let repeated = root
        .shader(|cx| {
            let first = cx.sample(&base);
            for _ in 0..128 {
                let _ = cx.sample(&base);
            }
            first
        })
        .unwrap();
    assert_eq!(base.program_id(), repeated.program_id());
    assert_eq!(base.node_count(), repeated.node_count());
    let distinct = root
        .shader(|cx| {
            let mut color = cx.vec4([0.; 4]);
            for i in 0..24 {
                let p = cx.position() - cx.vec2([i as f32 * 4., 0.]);
                color = color + cx.sample_at(&base, p, cx.vec2([96., 96.]));
            }
            color.scale(cx.scalar(1. / 24.))
        })
        .unwrap();
    assert!(distinct.node_count() > base.node_count() * 8);
    assert_eq!(distinct.parameter_slots().len(), 3);
    validate(&distinct);
}
#[test]
fn independently_rebound_clone_conflicts_are_order_independent() {
    let root = PaintRoot::new();
    let p = root.parameter([0.2, 0.4, 0.6, 1.]).unwrap();
    let a = root.shader(|cx| cx.uniform(p)).unwrap();
    let b = a.with_parameter(p, [0.9, 0.8, 0.7, 1.]).unwrap();
    for reverse in [false, true] {
        let result = root.shader(|cx| {
            let (first, second) = if reverse { (&b, &a) } else { (&a, &b) };
            cx.sample(first).mix(cx.sample(second), cx.scalar(0.5))
        });
        assert_eq!(result.unwrap_err(), ShaderError::AmbiguousParameter);
    }
}

#[test]
fn imported_root_parameter_reads_commute_with_sampling() {
    let source = PaintRoot::new();
    let phase = source.parameter(0.25).unwrap();
    let material = source
        .shader(|cx| {
            cx.rgba(
                cx.uniform(phase),
                cx.scalar(0.),
                cx.scalar(0.),
                cx.scalar(1.),
            )
        })
        .unwrap()
        .with_parameter(phase, 0.75)
        .unwrap();
    let destination = PaintRoot::new();
    let before = destination
        .shader(|cx| {
            let phase = cx.uniform(phase);
            let material = cx.sample(&material);
            material.scale(phase)
        })
        .unwrap();
    let after = destination
        .shader(|cx| {
            let material = cx.sample(&material);
            let phase = cx.uniform(phase);
            material.scale(phase)
        })
        .unwrap();
    assert_eq!(before, after);
    assert_eq!(before.wgsl(), after.wgsl());
    assert_eq!(before.parameter_slots(), &[[0.75, 0., 0., 0.]]);
    validate(&before);
    assert_eq!(
        destination
            .shader(|cx| {
                cx.rgba(
                    cx.uniform(phase),
                    cx.scalar(0.),
                    cx.scalar(0.),
                    cx.scalar(1.),
                )
            })
            .unwrap_err(),
        ShaderError::ForeignGraph
    );
}

#[test]
fn dead_imported_uniforms_do_not_exhaust_live_parameter_budget() {
    let root = PaintRoot::new();
    let parameters: Vec<_> = (0..65).map(|i| root.parameter(i as f32).unwrap()).collect();
    let full = root
        .shader(|cx| {
            let mut sum = cx.scalar(0.);
            for parameter in &parameters[..64] {
                sum = sum + cx.uniform(*parameter);
            }
            cx.rgba(sum, cx.scalar(0.), cx.scalar(0.), cx.scalar(1.))
        })
        .unwrap();
    assert_eq!(full.parameter_slots().len(), 64);
    let pruned = root
        .shader(|cx| {
            let _ = cx.sample(&full);
            cx.rgba(
                cx.uniform(parameters[64]),
                cx.scalar(0.),
                cx.scalar(0.),
                cx.scalar(1.),
            )
        })
        .unwrap();
    let direct = root
        .shader(|cx| {
            cx.rgba(
                cx.uniform(parameters[64]),
                cx.scalar(0.),
                cx.scalar(0.),
                cx.scalar(1.),
            )
        })
        .unwrap();
    assert_eq!(pruned, direct);
    assert_eq!(pruned.parameter_slots().len(), 1);
    assert!(!pruned.has_parameter(parameters[0]));
    assert!(pruned.has_parameter(parameters[64]));
    validate(&pruned);
    let too_many = root.shader(|cx| {
        let mut sum = cx.scalar(0.);
        for parameter in &parameters {
            sum = sum + cx.uniform(*parameter);
        }
        cx.rgba(sum, cx.scalar(0.), cx.scalar(0.), cx.scalar(1.))
    });
    assert_eq!(too_many.unwrap_err(), ShaderError::TooManyParameters);
}

#[test]
fn typed_update_batches_are_atomic_share_noops_and_last_write_wins() {
    let root = PaintRoot::new();
    let component = hero(&root);
    let parameters = HeroParameters::new(&root, 0.25);
    let shader = material(&root, &component, &parameters);
    let snapshot = shader.parameter_slots().to_vec();
    let updated = shader
        .with_parameters(|u| {
            u.set(parameters.phase, 0.5)
                .set(parameters.offset, [3., 4.])
                .set(parameters.tint, [0.9, 0.6, 0.3, 1.])
                .set(parameters.phase, 0.75);
        })
        .unwrap();
    let sequential = shader
        .with_parameter(parameters.phase, 0.75)
        .unwrap()
        .with_parameter(parameters.offset, [3., 4.])
        .unwrap()
        .with_parameter(parameters.tint, [0.9, 0.6, 0.3, 1.])
        .unwrap();
    assert_eq!(updated, sequential);
    assert_eq!(updated.wgsl().as_ptr(), shader.wgsl().as_ptr());
    assert_eq!(shader.parameter_slots(), snapshot);
    let unchanged = shader
        .with_parameters(|u| {
            u.set(parameters.phase, 0.25);
        })
        .unwrap();
    assert_eq!(
        unchanged.parameter_slots().as_ptr(),
        shader.parameter_slots().as_ptr()
    );
    let unknown = root.parameter(1.).unwrap();
    assert_eq!(
        shader
            .with_parameters(|u| {
                u.set(parameters.phase, 3.).set(unknown, 2.);
            })
            .unwrap_err(),
        ShaderError::ForeignGraph
    );
    assert_eq!(
        shader
            .with_parameters(|u| {
                u.set(parameters.phase, 3.)
                    .set(parameters.tint, [0., f32::NAN, 0., 1.]);
            })
            .unwrap_err(),
        ShaderError::NonFiniteValue
    );
    assert_eq!(shader.parameter_slots(), snapshot);
    assert_eq!(
        shader.parameter_slots().as_ptr(),
        unchanged.parameter_slots().as_ptr()
    );
}
