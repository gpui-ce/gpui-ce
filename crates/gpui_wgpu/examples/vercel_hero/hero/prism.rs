//! Pointer controlled lighting around a signed distance triangle.

use gpui::{
    black,
    shader::{Input, Paint, Scalar, color, noise, paint, shape, vec2f},
    white,
};

/// Inverse-square falloff: `1` at zero distance, `1/2` at `radius`.
fn falloff(distance: Scalar, radius: f32) -> Scalar {
    let x = distance / radius;
    1.0 / (1.0 + &x * &x)
}

/// A black prism with light spilling around its edges from behind.
///
/// The pointer pushes the light towards the opposite edge. Hover and press
/// increase its brightness. Input calculations become uniforms, allowing
/// frames to share the compiled program.
pub fn prism(input: &Input) -> Paint {
    let size = input.size;
    let center = vec2f(size.x * 0.5, size.y * 0.47);
    let radius = (size.y * 0.17).clamp(60.0, 180.0);

    let away = center - input.pointer;
    let reach = (away.x * away.x + away.y * away.y).sqrt();
    let push = if reach > 1.0 {
        away * ((reach * 0.4).min(radius * 0.75) / reach)
    } else {
        vec2f(0.0, 0.0)
    };
    let drift = vec2f(
        (input.time * 0.7).sin() * 6.0,
        (input.time * 0.5).cos() * 4.0,
    );
    let light = center + vec2f(0.0, -radius * 0.3) + push + drift;
    let strength = 0.8 + 0.4 * input.hover + if input.pressed { 0.5 } else { 0.0 };

    paint(|px| {
        let p = px.position();
        let distance = shape::triangle(&p - center, radius);
        let lit = falloff((&p - light).length(), radius * 0.65);
        let rim = falloff(distance.abs(), 1.3) * &lit;
        let halo = falloff(distance.max(0.0), radius * 0.14) * &lit;
        let bloom = falloff((&p - center).length(), radius * 2.2) * 0.07;
        // Film grain, fixed to the device pixel grid.
        let grain = noise::hash((&p * px.scale()).floor()) * 0.6 + 0.7;
        let glow = (rim * 0.9 + halo * 0.8) * strength * grain + bloom;
        let outside = 1.0 - distance.coverage();
        color(white()).mask(glow * outside).over(color(black()))
    })
}
