//! Paints driven by the layer's pointer and time.

use gpui::{
    rgb,
    shader::{Input, Paint, Vec3, color, paint, rgba, shape, vec2, vec3, vec3f},
    white,
};

/// A torus, raymarched in pure Rust expressions and lit with diffuse,
/// specular, and rim light. The pointer turns it.
pub fn torus(input: &Input) -> Paint {
    let turn = (input.pointer - input.size * 0.5) / input.size.y;
    let yaw = turn.x * 3.0 + input.time * 0.5;
    let pitch = 0.6 + turn.y * 2.0;
    let light = wgsl_rs::std::normalize(vec3f(-0.5, 0.8, -0.6));
    paint(|px| {
        let uv = px.centered() / px.size().y();
        let origin = Vec3::constant(vec3f(0.0, 0.0, -4.4));
        let ray = vec3(uv.x(), -uv.y(), 1.5).normalize();
        let scene = |p: &Vec3| {
            let tilted = vec2(p.y(), p.z()).rotate(pitch);
            let spun = vec2(p.x(), tilted.y()).rotate(yaw);
            let ring = vec2(spun.length() - 0.9, tilted.x()).length();
            ring - 0.34
        };
        let t = shape::raymarch(&origin, &ray, 8.0, 80, scene);
        let normal = shape::normal(&(&origin + &ray * &t), scene);
        let diffuse = normal.dot(light).max(0.0);
        let halfway = (Vec3::constant(light) - &ray).normalize();
        let specular = normal.dot(halfway).max(0.0).pow(48.0);
        let rim = (normal.dot(&ray) + 1.0).pow(3.0);
        let shade = vec3(1.0, 0.42, 0.24) * (diffuse * 0.85 + 0.1)
            + vec3(0.5, 0.7, 1.0) * rim * 0.7
            + Vec3::splat(specular);
        let surface = rgba(shade.x(), shade.y(), shade.z(), 1.0);
        let backdrop = color(rgb(0x0b0e1c)).mix(color(rgb(0x1d2645)), px.uv().y());
        surface.select(t.lt(8.0), backdrop)
    })
}

/// A field of dots that swell and brighten around the pointer.
pub fn spotlight(input: &Input) -> Paint {
    let (pointer, hover) = (input.pointer, input.hover);
    paint(|px| {
        let cell = (px.position() / 14.0).fract() - 0.5;
        let near = (1.0 - (px.position() - pointer).length() / 90.0).max(0.0);
        let dot = shape::circle(cell, &near * hover * 0.3 + 0.1);
        let tint = color(rgb(0x7cf0ff)).mix(color(white()), &near * hover);
        tint.clip(dot)
            .opacity(near * hover * 0.8 + 0.2)
            .over(color(rgb(0x0b0e1c)))
    })
}
