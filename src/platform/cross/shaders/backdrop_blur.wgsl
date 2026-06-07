const M_PI_F: f32 = 3.1415926;

struct Globals {
    viewport_size: vec2<f32>,
    premultiplied_alpha: u32,
    pad: u32,
}

struct Bounds {
    origin: vec2<f32>,
    size: vec2<f32>,
}

struct Corners {
    top_left: f32,
    top_right: f32,
    bottom_right: f32,
    bottom_left: f32,
}

struct BackdropFilter {
    order: u32,
    bounds: Bounds,
    content_mask: Bounds,
    corner_radii: Corners,
    blur_radius: f32,
    opacity: f32,
}

struct BackdropFilterVarying {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) backdrop_filter_id: u32,
    @location(1) clip_distances: vec4<f32>,
}

@group(0) @binding(0) var<uniform> globals: Globals;
@group(1) @binding(0) var<storage, read> b_backdrop_filters: array<BackdropFilter>;
@group(2) @binding(0) var backdrop_texture: texture_2d<f32>;
@group(2) @binding(1) var backdrop_sampler: sampler;

fn to_device_position(unit_vertex: vec2<f32>, bounds: Bounds) -> vec4<f32> {
    let position = unit_vertex * bounds.size + bounds.origin;
    let device_position = position / globals.viewport_size * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0);
    return vec4<f32>(device_position, 0.0, 1.0);
}

// Gaussian blur function
fn gaussian(x: f32, sigma: f32) -> f32 {
    return exp(-(x * x) / (2.0 * sigma * sigma)) / (sqrt(2.0 * M_PI_F) * sigma);
}

// SDF for rounded rectangle
fn quad_sdf(point: vec2<f32>, bounds: Bounds, corner_radii: Corners) -> f32 {
    let center = bounds.origin + bounds.size / 2.0;
    let half_size = bounds.size / 2.0;
    var radii_size = vec2<f32>(0.0);
    if point.x < center.x {
        if point.y < center.y {
            radii_size = vec2<f32>(corner_radii.top_left);
        } else {
            radii_size = vec2<f32>(corner_radii.bottom_left);
        }
    } else {
        if point.y < center.y {
            radii_size = vec2<f32>(corner_radii.top_right);
        } else {
            radii_size = vec2<f32>(corner_radii.bottom_right);
        }
    }
    let q = abs(point - center) - half_size + radii_size;
    return min(max(q.x, q.y), 0.0) + length(max(q, vec2<f32>(0.0))) - radii_size.x;
}

@vertex
fn vs_backdrop_filter(@builtin(vertex_index) vertex_id: u32, @builtin(instance_index) instance_id: u32) -> BackdropFilterVarying {
    let unit_vertex = vec2<f32>(f32(vertex_id & 1u), 0.5 * f32(vertex_id & 2u));
    let backdrop_filter = b_backdrop_filters[instance_id];

    let position = to_device_position(unit_vertex, backdrop_filter.bounds);

    let content_mask = backdrop_filter.content_mask;
    var clip_distances = vec4<f32>(0.0);
    let pixel_position = unit_vertex * backdrop_filter.bounds.size + backdrop_filter.bounds.origin;
    clip_distances.x = pixel_position.x - content_mask.origin.x;
    clip_distances.y = pixel_position.y - content_mask.origin.y;
    clip_distances.z = content_mask.origin.x + content_mask.size.x - pixel_position.x;
    clip_distances.w = content_mask.origin.y + content_mask.size.y - pixel_position.y;

    return BackdropFilterVarying(position, instance_id, clip_distances);
}

@fragment
fn fs_backdrop_filter(input: BackdropFilterVarying) -> @location(0) vec4<f32> {
    // Clip test
    if any(input.clip_distances < vec4<f32>(0.0)) {
        return vec4<f32>(0.0);
    }

    let backdrop_filter = b_backdrop_filters[input.backdrop_filter_id];
    let pixel_position = input.position.xy;

    // Apply Gaussian blur sampling from the source texture (either a snapshot
    // of the surface for `backdrop-filter`, or an offscreen group texture for
    // a content `filter` group composite).
    var blurred_color = vec4<f32>(0.0);
    var total_weight = 0.0;

    let blur_radius = backdrop_filter.blur_radius;

    // Skip blur sampling if radius is very small
    if blur_radius < 0.5 {
        let uv = pixel_position / globals.viewport_size;
        blurred_color = textureSample(backdrop_texture, backdrop_sampler, uv);
    } else {
        // Cap effective radius so the dynamic loops remain manageable on FXC/DX.
        let max_blur_radius = 32.0;
        let effective_blur_radius = min(blur_radius, max_blur_radius);
        let kernel_size = i32(ceil(effective_blur_radius * 2.0));
        let sigma = max(effective_blur_radius / 2.0, 0.0001);

        // Use explicit loops to avoid WGSL `for` unroll pressure on FXC.
        var dy = -kernel_size;
        loop {
            if dy > kernel_size {
                break;
            }

            var dx = -kernel_size;
            loop {
                if dx > kernel_size {
                    break;
                }

                let offset = vec2<f32>(f32(dx), f32(dy));
                let weight = gaussian(length(offset), sigma);

                let sample_pos = pixel_position + offset;
                let sample_uv = sample_pos / globals.viewport_size;

                // Clamp UV to valid range
                if sample_uv.x >= 0.0 && sample_uv.x <= 1.0 && sample_uv.y >= 0.0 && sample_uv.y <= 1.0 {
                    // Use explicit LOD in dynamic loops (FXC can't derive gradients reliably here).
                    let sample_color = textureSampleLevel(backdrop_texture, backdrop_sampler, sample_uv, 0.0);
                    blurred_color += sample_color * weight;
                    total_weight += weight;
                }

                dx += 1;
            }

            dy += 1;
        }

        if total_weight > 0.0 {
            blurred_color /= total_weight;
        }
    }

    // Apply corner radius masking and element opacity. The sampled color is
    // premultiplied, so scale both rgb and alpha by the same factor to keep
    // it premultiplied.
    let outer_sdf = quad_sdf(pixel_position, backdrop_filter.bounds, backdrop_filter.corner_radii);
    let mask_alpha = saturate(0.5 - outer_sdf);
    let factor = mask_alpha * backdrop_filter.opacity;

    return vec4<f32>(blurred_color.rgb * factor, blurred_color.a * factor);
}
