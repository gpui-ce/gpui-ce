use super::*;
use crate::{hsla, rgb};

fn fragment(uv: [f32; 2], size: [f32; 2]) -> Fragment {
    Fragment {
        uv: vec2f(uv[0], uv[1]),
        position: vec2f(uv[0] * size[0], uv[1] * size[1]),
        size: vec2f(size[0], size[1]),
        origin: vec2f(0.0, 0.0),
        scale: 1.0,
    }
}

fn waves(phase: f32) -> Paint {
    paint(|px| {
        let wave = (px.uv().x() * 12.0 + phase).sin() * 0.5 + 0.5;
        color(rgb(0x315bff)).mix(color(rgb(0xf48bcb)), wave)
    })
}

#[test]
fn rebuilding_with_new_numbers_reuses_the_program() {
    let a = waves(0.0).compile().unwrap();
    let b = waves(1.5).compile().unwrap();
    assert_eq!(a.program.id(), b.program.id());
    assert_ne!(a.params, b.params);
    assert_ne!(a, b);
    assert_eq!(a, waves(0.0).compile().unwrap());
}

#[test]
fn uniform_subtrees_fold_on_the_cpu() {
    let folded = color(rgb(0xff0000))
        .mix(color(rgb(0x0000ff)), 0.25)
        .opacity(0.5);
    let solid = color(rgb(0x00ff00));
    let folded_compiled = folded.compile().unwrap();
    // Folding leaves only the parameter read, so both share one program.
    assert_eq!(
        folded_compiled.program.id(),
        solid.compile().unwrap().program.id()
    );
    assert_eq!(folded_compiled.program.parameter_slots(), 1);
    let [r, g, b, a] = folded_compiled.params[0];
    assert!((a - 0.5).abs() < 1e-6, "alpha {a}");
    assert!((r - 0.375).abs() < 1e-6 && g.abs() < 1e-6 && (b - 0.125).abs() < 1e-6);
}

#[test]
fn shared_uniforms_share_a_slot_and_independent_ones_do_not() {
    let shared = Scalar::uniform(2.0);
    let one = paint(|px| rgba(px.uv().x() * &shared + &shared, 0.0, 0.0, 1.0));
    let two = paint(|px| rgba(px.uv().x() * 2.0 + 2.0, 0.0, 0.0, 1.0));
    let one = one.compile().unwrap();
    let two = two.compile().unwrap();
    assert_ne!(one.program.id(), two.program.id());
    assert_eq!(one.params[0][..1], [2.0]);
    assert_eq!(two.params[0][..2], [2.0, 2.0]);
}

#[test]
fn constants_are_embedded_and_uniforms_are_not() {
    let embedded = |value| {
        paint(|px| rgba(px.uv().x() * constant(value), 0.0, 0.0, 1.0))
            .compile()
            .unwrap()
    };
    let uniform = |value| {
        paint(|px| rgba(px.uv().x() * value, 0.0, 0.0, 1.0))
            .compile()
            .unwrap()
    };
    let (embedded_a, embedded_b) = (embedded(3.0), embedded(4.0));
    let (uniform_a, uniform_b) = (uniform(3.0), uniform(4.0));
    assert_ne!(embedded_a.program.id(), embedded_b.program.id());
    assert_eq!(uniform_a.program.id(), uniform_b.program.id());
    assert_eq!(uniform_a.params[0][0], 3.0);
    assert_eq!(uniform_b.params[0][0], 4.0);
    assert_ne!(embedded_a.program.id(), uniform_a.program.id());
}

#[test]
fn map_uv_evaluates_its_paint_at_mapped_coordinates() {
    let ramp = paint(|px| rgba(px.uv().x(), px.position().y() / 100.0, 0.0, 1.0));
    let mirrored = ramp.map_uv(|uv| vec2(1.0 - uv.x(), uv.y() * 0.5));
    mirrored.compile().unwrap();

    let at = fragment([0.25, 0.5], [100.0, 100.0]);
    let rgba = mirrored.evaluate(at).unwrap();
    assert!((rgba.x - 0.75).abs() < 1e-6);
    assert!((rgba.y - 0.25).abs() < 1e-6);
    assert!((ramp.evaluate(at).unwrap().x - 0.25).abs() < 1e-6);
}

#[test]
fn shared_mapped_paints_lower_once_per_context() {
    let mut layer = paint(|px| rgba(px.uv().x(), px.uv().y(), 0.0, 1.0));
    for _ in 0..40 {
        // Each level uses the previous one twice, under two mappings.
        layer = layer
            .map_uv(|uv| uv * 0.5)
            .mix(layer.map_uv(|uv| uv * 0.5 + 0.5), 0.5);
    }
    let compiled = layer.compile().unwrap();
    assert!(compiled.program.source().len() < 64 * 1024);
}

#[test]
fn channels_and_select_compose() {
    let base = paint(|px| rgba(px.uv().x(), 0.5, 0.25, 0.5));
    let picked = paint(|px| {
        let left = px.uv().x().lt(0.5);
        rgba(
            left.select(base.red(), base.blue()),
            base.green(),
            0.0,
            base.alpha(),
        )
    });
    for (x, red) in [(0.125, 0.0625), (0.75, 0.125)] {
        let rgba = picked.evaluate(fragment([x, 0.0], [10.0, 10.0])).unwrap();
        assert_eq!(rgba, vec4f(red, 0.25, 0.0, 0.5));
    }
    picked.compile().unwrap();
}

#[test]
fn non_finite_values_are_rejected() {
    let paint = paint(|px| rgba(px.uv().x() * f32::NAN, 0.0, 0.0, 1.0));
    assert_eq!(paint.compile().unwrap_err(), ShaderError::NonFinite);
}

#[test]
fn excessive_depth_is_rejected_without_overflowing() {
    let mut value = Pixel.uv().x();
    for _ in 0..100_000 {
        value = value + 1.0;
    }
    let paint = rgba(value, 0.0, 0.0, 1.0);
    assert_eq!(paint.compile().unwrap_err(), ShaderError::TooDeep);
    assert!(paint.evaluate(fragment([0.5, 0.5], [10.0, 10.0])).is_none());
}

#[test]
fn scalars_widen_across_vectors() {
    let widened = paint(|px| {
        let uv = (px.uv() * 2.0 - 0.5).clamp(0.0, 1.0);
        let eased = uv.smoothstep(0.0, 1.0);
        rgba(uv.x(), uv.y(), eased.x().max(0.25), 1.0)
    });
    let rgba = widened
        .evaluate(fragment([0.6, 0.1], [10.0, 10.0]))
        .unwrap();
    let eased = 0.7 * 0.7 * (3.0 - 2.0 * 0.7);
    assert!((rgba.x - 0.7).abs() < 1e-6 && rgba.y == 0.0);
    assert!((rgba.z - eased).abs() < 1e-6, "blue {}", rgba.z);
    widened.compile().unwrap();
}

#[test]
fn uniform_derivatives_fold_to_zero() {
    let flat = rgba(Scalar::uniform(3.0).fwidth(), 0.0, 0.0, 1.0);
    let compiled = flat.compile().unwrap();
    assert_eq!(compiled.program.parameter_slots(), 1);
    assert_eq!(compiled.params[0][0], 0.0);
}

#[test]
fn paints_are_operands() {
    let left = color(rgb(0xff0000));
    let right = paint(|px| rgba(0.0, px.uv().y(), 1.0, 1.0));
    let split = paint(|px| left.select(px.uv().x().lt(0.5), &right));
    let at = |paint: &Paint, uv| paint.evaluate(fragment(uv, [10.0, 10.0]));
    assert_eq!(at(&split, [0.25, 0.5]).unwrap(), vec4f(1.0, 0.0, 0.0, 1.0));
    assert_eq!(at(&split, [0.75, 0.5]).unwrap(), vec4f(0.0, 0.5, 1.0, 1.0));
    split.compile().unwrap();
}

#[test]
fn rotation_turns_clockwise_on_screen() {
    let turned = paint(|_| {
        let v = Vec2::uniform(vec2f(1.0, 0.0)).rotate(std::f32::consts::FRAC_PI_2);
        rgba(v.x(), v.y(), 0.0, 1.0)
    });
    let rgba = turned.evaluate(fragment([0.0, 0.0], [1.0, 1.0])).unwrap();
    assert!(rgba.x.abs() < 1e-6 && (rgba.y - 1.0).abs() < 1e-6);
    // Entirely uniform, so it folds to one parameter.
    assert_eq!(turned.compile().unwrap().program.parameter_slots(), 1);
}

/// A unit sphere seen from z = -3, lit from the upper left.
fn sphere() -> Paint {
    paint(|px| {
        let scene = |p: &Vec3| p.length() - 1.0;
        let uv = (px.uv() - 0.5) * 2.0;
        let origin = Vec3::uniform(vec3f(0.0, 0.0, -3.0));
        let ray = uv.extend(1.5).normalize();
        let distance = iterate_until(
            64,
            0.0,
            |t| scene(&(&origin + &ray * t)).lt(0.001),
            |t, _| {
                let d = scene(&(&origin + &ray * &t));
                t + d
            },
        );
        let hit = &origin + &ray * &distance;
        let shade = hit
            .normalize()
            .dot(vec3(-0.5, -0.5, -0.7).normalize())
            .max(0.0);
        let inside = distance.lt(10.0);
        color(rgb(0xffffff))
            .mask(shade)
            .select(inside, color(rgb(0x000000)))
    })
}

#[test]
fn loops_run_per_fragment_and_on_the_cpu() {
    let sphere = sphere();
    let compiled = sphere.compile().unwrap();
    let source = compiled.program.source();
    assert_eq!(source.matches("for (var").count(), 1, "{source}");
    assert!(source.contains("break;"));

    let at = |uv| sphere.evaluate(fragment(uv, [100.0, 100.0])).unwrap();
    assert!(at([0.4, 0.4]).x > 0.5, "lit side");
    assert_eq!(at([0.02, 0.02]).x, 0.0, "background");

    // Uniform loops fold to their result.
    let sum = iterate(4, 0.0, |total, index| total + index);
    let folded = rgba(sum, 0.0, 0.0, 1.0).compile().unwrap();
    assert_eq!(folded.params[0][0], 6.0);
    assert!(!folded.program.source().contains("for"));
}

#[test]
fn loop_invariants_are_hoisted_and_loops_nest() {
    let rings = paint(|px| {
        let spin = (px.uv().x() * 7.0).sin();
        let total = iterate(3, 0.0, |outer, i| {
            let inner = iterate(4, 0.0, |acc, j| acc + &spin * (&i + j));
            outer + inner
        });
        rgba(total * 0.01, 0.0, 0.0, 1.0)
    });
    let source = rings.compile().unwrap().program.source().to_owned();
    assert_eq!(source.matches("for (var").count(), 2);
    // `spin` depends on neither loop, so it is computed before both.
    let sin = source.find("sin(").unwrap();
    assert!(sin < source.find("for (var").unwrap(), "{source}");

    let at = rings.evaluate(fragment([0.25, 0.0], [1.0, 1.0])).unwrap();
    let spin = (0.25f32 * 7.0).sin();
    let expected: f32 = (0..3)
        .flat_map(|i| (0..4).map(move |j| spin * (i + j) as f32))
        .sum();
    assert!((at.x - expected * 0.01).abs() < 1e-5);

    let leaked = std::cell::RefCell::new(None);
    let _ = iterate(2, 0.0, |state, _| {
        leaked.replace(Some(state.clone()));
        state
    });
    let leaked = leaked.into_inner().unwrap();
    assert!(rgba(leaked, 0.0, 0.0, 1.0).compile().is_err());
}

#[test]
fn backdrop_paints_are_flagged() {
    let glass = paint(|px| {
        let bend = (px.uv().y() * 18.0).sin() * 0.01;
        px.backdrop(px.uv() + vec2(bend, 0.0))
            .mix(color(rgb(0xffffff)), 0.08)
    });
    let compiled = glass.compile().unwrap();
    assert!(compiled.program.uses_backdrop());
    assert!(glass.evaluate(fragment([0.5, 0.5], [1.0, 1.0])).is_none());
    assert!(!waves(0.0).compile().unwrap().program.uses_backdrop());
}
