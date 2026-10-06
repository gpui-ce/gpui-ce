// GPUI supplies Fragment and its prelude. Rust passes time via PLASMA.with(time).
// Paint functions return straight-alpha RGBA.

fn paint(fragment: Fragment, time: f32) -> vec4<f32> {
    let p = fragment.uv * 6.0;
    let v = sin(p.x + time)
        + sin(p.y * 1.3 - time * 0.7)
        + sin((p.x + p.y) * 0.7 + time * 1.3)
        + sin(length(p - 3.0) * 1.5 - time);
    let color = 0.5 + 0.5 * cos(vec3<f32>(0.0, 2.1, 4.2) + v * 1.2);
    return vec4<f32>(color, 1.0);
}
