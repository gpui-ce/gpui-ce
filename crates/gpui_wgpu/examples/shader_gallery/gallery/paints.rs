//! Expression paints for the gallery cards.

use gpui::{
    rgb,
    shader::{Paint, Vec3, color, constant, iterate, noise, paint, rgba, shape, vec2, vec3},
    white,
};

/// Changing the time uniform animates the waves without recompiling.
pub fn waves(time: f32) -> Paint {
    paint(|px| {
        let uv = px.uv();
        let wave = (uv.x() * 9.0 + uv.y() * 4.0 + time * 2.0).sin() * 0.5 + 0.5;
        color(rgb(0x315bff)).mix(color(rgb(0xf48bcb)), wave)
    })
}

/// Combine signed distances before applying antialiased coverage.
pub fn orbits(time: f32) -> Paint {
    paint(|px| {
        let p = px.centered();
        let ring = shape::circle(&p, 52.0).abs() - 2.5;
        let moon = shape::circle(&p - vec2(time.cos() * 52.0, time.sin() * 52.0), 11.0);
        let core = shape::circle(&p, 18.0 + (time * 3.0).sin() * 3.0);
        let body = shape::smooth_union(ring, moon, 12.0).min(core);
        color(rgb(0xffd166)).clip(body).over(color(rgb(0x1a1f3a)))
    })
}

pub fn aurora(time: f32) -> Paint {
    paint(|px| {
        let uv = px.uv();
        let drift = noise::value(&uv * vec2(3.0, 5.0) + vec2(time * 0.3, time * 0.1));
        let band = ((uv.y() + drift * 0.35) * 9.0 - time).sin().abs();
        let glow = (1.0 - band).pow(6.0) * (1.0 - uv.y() * 0.6);
        color(rgb(0x3ee6b0))
            .mix(color(rgb(0x7b5cff)), uv.x())
            .mask(glow)
            .over(color(rgb(0x060914)))
    })
}

/// Fractal clouds: five octaves of noise in one GPU loop.
pub fn clouds(time: f32) -> Paint {
    paint(|px| {
        let p = px.uv() * 3.0 + vec2(time * 0.15, time * 0.05);
        let fbm = iterate(5, vec2(0.0, 0.5), |acc, octave| {
            let scale = constant(2.0).pow(octave);
            vec2(acc.x() + noise::value(&p * &scale) * acc.y(), acc.y() * 0.5)
        });
        color(rgb(0x1b2a6b)).mix(color(rgb(0xf6d6ff)), fbm.x().smoothstep(0.25, 0.85))
    })
}

/// A checkerboard, swirled by remapping where it is sampled.
pub fn swirl(time: f32) -> Paint {
    let checker = paint(|px| {
        let cell = (px.position() / 16.0).floor();
        let parity = ((cell.x() + cell.y()) * 0.5).fract() * 2.0;
        color(rgb(0x1a1f3a)).mix(color(rgb(0xe8ecff)), parity)
    });
    checker.map_uv(|uv| {
        let centered = uv - 0.5;
        let twist = (0.5 - centered.length()).max(0.0) * 5.0 * time.sin();
        centered.rotate(twist) + 0.5
    })
}

/// A hue sweep around the box's center, for a border.
pub fn rainbow(time: f32) -> Paint {
    paint(|px| {
        let p = px.centered();
        let turn = p.y().atan2(p.x()) + time;
        let hue = (Vec3::splat(turn) + vec3(0.0, 2.1, 4.2)).cos() * 0.5 + 0.5;
        rgba(hue.x(), hue.y(), hue.z(), 1.0)
    })
}

pub fn stripes(time: f32) -> Paint {
    paint(|px| {
        let position = px.position();
        let band = ((position.x() + position.y()) / 10.0 - time * 3.0)
            .sin()
            .step(0.0);
        color(rgb(0xff5f6d)).mix(color(rgb(0xffc371)), band)
    })
}

/// Glass: the scene behind the box, magnified with a little chromatic split.
pub fn lens() -> Paint {
    paint(|px| {
        let offset = px.uv() - 0.5;
        let zoom = |amount: f32| px.backdrop(&offset * amount + 0.5);
        let channel = rgba(zoom(0.70).red(), zoom(0.72).green(), zoom(0.74).blue(), 1.0);
        color(white()).opacity(0.12).over(channel)
    })
}
