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

struct Hsla {
    h: f32,
    s: f32,
    l: f32,
    a: f32,
}

struct Corners {
    top_left: f32,
    top_right: f32,
    bottom_right: f32,
    bottom_left: f32,
}

struct Edges {
    top: f32,
    right: f32,
    bottom: f32,
    left: f32,
}

struct GradientStop {
    color: Hsla,
    percentage: f32,
}

struct Background {
    tag: u32,
    color_space: u32,
    solid: Hsla,
    gradient_angle_or_pattern_height: f32,
    color0: GradientStop,
    color1: GradientStop,
    pad: u32,
}

struct BackdropBlur {
    order: u32,
    blur_radius: f32,
    bounds: Bounds,
    content_mask: Bounds,
    corner_radii: Corners,
    background: Background,
    border_color: Hsla,
    border_widths: Edges,
}

struct BackdropBlurVarying {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) backdrop_blur_id: u32,
    @location(1) clip_distances: vec4<f32>,
    @location(2) @interpolate(flat) border_color: vec4<f32>,
    @location(3) @interpolate(flat) background_solid: vec4<f32>,
    @location(4) @interpolate(flat) background_color0: vec4<f32>,
    @location(5) @interpolate(flat) background_color1: vec4<f32>,
}

@group(0) @binding(0) var<uniform> globals: Globals;
@group(1) @binding(0) var<storage, read> b_backdrop_blurs: array<BackdropBlur>;
@group(2) @binding(0) var backdrop_texture: texture_2d<f32>;
@group(2) @binding(1) var backdrop_sampler: sampler;

fn to_device_position(unit_vertex: vec2<f32>, bounds: Bounds) -> vec4<f32> {
    let position = unit_vertex * bounds.size + bounds.origin;
    let device_position = position / globals.viewport_size * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0);
    return vec4<f32>(device_position, 0.0, 1.0);
}

fn hsla_to_rgba(hsla: Hsla) -> vec4<f32> {
    let h = hsla.h * 6.0;
    let s = hsla.s;
    let l = hsla.l;
    let a = hsla.a;

    let c = (1.0 - abs(2.0 * l - 1.0)) * s;
    let x = c * (1.0 - abs(h % 2.0 - 1.0));
    let m = l - c / 2.0;
    var color = vec3<f32>(m);

    if h >= 0.0 && h < 1.0 {
        color.r += c;
        color.g += x;
    } else if h >= 1.0 && h < 2.0 {
        color.r += x;
        color.g += c;
    } else if h >= 2.0 && h < 3.0 {
        color.g += c;
        color.b += x;
    } else if h >= 3.0 && h < 4.0 {
        color.g += x;
        color.b += c;
    } else if h >= 4.0 && h < 5.0 {
        color.r += x;
        color.b += c;
    } else {
        color.r += c;
        color.b += x;
    }

    return vec4<f32>(color, a);
}

fn linear_to_srgba(linear: vec4<f32>) -> vec4<f32> {
    let a = 0.055;
    var srgb: vec3<f32>;
    for (var i = 0; i < 3; i++) {
        if linear[i] <= 0.0031308 {
            srgb[i] = linear[i] * 12.92;
        } else {
            srgb[i] = (1.0 + a) * pow(linear[i], 1.0 / 2.4) - a;
        }
    }
    return vec4<f32>(srgb, linear.a);
}

fn srgba_to_linear(srgb: vec4<f32>) -> vec4<f32> {
    var linear: vec3<f32>;
    for (var i = 0; i < 3; i++) {
        if srgb[i] <= 0.04045 {
            linear[i] = srgb[i] / 12.92;
        } else {
            linear[i] = pow((srgb[i] + 0.055) / 1.055, 2.4);
        }
    }
    return vec4<f32>(linear, srgb.a);
}

fn oklab_to_linear_srgb(color: vec4<f32>) -> vec4<f32> {
    let l_ = color.r + 0.3963377774 * color.g + 0.2158037573 * color.b;
    let m_ = color.r - 0.1055613458 * color.g - 0.0638541728 * color.b;
    let s_ = color.r - 0.0894841775 * color.g - 1.2914855480 * color.b;

    let l = l_ * l_ * l_;
    let m = m_ * m_ * m_;
    let s = s_ * s_ * s_;

    return vec4<f32>(
        4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
        -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
        -0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s,
        color.a
    );
}

fn linear_srgb_to_oklab(color: vec4<f32>) -> vec4<f32> {
    let l = 0.4122214708 * color.r + 0.5363325363 * color.g + 0.0514459929 * color.b;
    let m = 0.2119034982 * color.r + 0.6806995451 * color.g + 0.1073969566 * color.b;
    let s = 0.0883024619 * color.r + 0.2817188376 * color.g + 0.6299787005 * color.b;

    let l_ = pow(l, 1.0 / 3.0);
    let m_ = pow(m, 1.0 / 3.0);
    let s_ = pow(s, 1.0 / 3.0);

    return vec4<f32>(
        0.2104542553 * l_ + 0.7936177850 * m_ - 0.0040720468 * s_,
        1.9779984951 * l_ - 2.4285922050 * m_ + 0.4505937099 * s_,
        0.0259040371 * l_ + 0.7827717662 * m_ - 0.8086757660 * s_,
        color.a
    );
}

fn prepare_gradient_color(tag: u32, color_space: u32,
    solid: Hsla, color0: GradientStop, color1: GradientStop) -> array<vec4<f32>, 3> {
    var result: array<vec4<f32>, 3>;

    if tag == 0u || tag == 2u {
        result[0] = hsla_to_rgba(solid);
    } else if tag == 1u {
        result[1] = hsla_to_rgba(color0.color);
        result[2] = hsla_to_rgba(color1.color);

        if color_space == 0u {
            result[1] = linear_to_srgba(result[1]);
            result[2] = linear_to_srgba(result[2]);
        } else if color_space == 1u {
            result[1] = linear_srgb_to_oklab(result[1]);
            result[2] = linear_srgb_to_oklab(result[2]);
        }
    }

    return result;
}

fn gradient_color(background: Background, position: vec2<f32>, bounds: Bounds,
    solid_color: vec4<f32>, color0: vec4<f32>, color1: vec4<f32>) -> vec4<f32> {
    if background.tag == 0u || background.tag == 2u {
        return solid_color;
    } else if background.tag == 1u {
        let angle = background.gradient_angle_or_pattern_height;
        let radians = (angle % 360.0 - 90.0) * M_PI_F / 180.0;
        var direction = vec2<f32>(cos(radians), sin(radians));
        let stop0_percentage = background.color0.percentage;
        let stop1_percentage = background.color1.percentage;

        if bounds.size.x > bounds.size.y {
            direction.y *= bounds.size.y / bounds.size.x;
        } else {
            direction.x *= bounds.size.x / bounds.size.y;
        }

        let half_size = bounds.size / 2.0;
        let center = bounds.origin + half_size;
        let center_to_point = position - center;
        var t = dot(center_to_point, direction) / length(direction);
        if abs(direction.x) > abs(direction.y) {
            t = (t + half_size.x) / bounds.size.x;
        } else {
            t = (t + half_size.y) / bounds.size.y;
        }

        t = (t - stop0_percentage) / (stop1_percentage - stop0_percentage);
        t = clamp(t, 0.0, 1.0);

        if background.color_space == 0u {
            return srgba_to_linear(mix(color0, color1, t));
        } else if background.color_space == 1u {
            let oklab_color = mix(color0, color1, t);
            return oklab_to_linear_srgb(oklab_color);
        }
    }
    return vec4<f32>(0.0);
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
fn vs_backdrop_blur(@builtin(vertex_index) vertex_id: u32, @builtin(instance_index) instance_id: u32) -> BackdropBlurVarying {
    let unit_vertex = vec2<f32>(f32(vertex_id & 1u), 0.5 * f32(vertex_id & 2u));
    let backdrop_blur = b_backdrop_blurs[instance_id];

    let position = to_device_position(unit_vertex, backdrop_blur.bounds);

    let colors = prepare_gradient_color(
        backdrop_blur.background.tag,
        backdrop_blur.background.color_space,
        backdrop_blur.background.solid,
        backdrop_blur.background.color0,
        backdrop_blur.background.color1
    );

    let content_mask = backdrop_blur.content_mask;
    var clip_distances = vec4<f32>(0.0);
    let pixel_position = unit_vertex * backdrop_blur.bounds.size + backdrop_blur.bounds.origin;
    clip_distances.x = pixel_position.x - content_mask.origin.x;
    clip_distances.y = pixel_position.y - content_mask.origin.y;
    clip_distances.z = content_mask.origin.x + content_mask.size.x - pixel_position.x;
    clip_distances.w = content_mask.origin.y + content_mask.size.y - pixel_position.y;

    return BackdropBlurVarying(
        position,
        instance_id,
        clip_distances,
        hsla_to_rgba(backdrop_blur.border_color),
        colors[0],
        colors[1],
        colors[2]
    );
}

@fragment
fn fs_backdrop_blur(input: BackdropBlurVarying) -> @location(0) vec4<f32> {
    // Clip test
    if any(input.clip_distances < vec4<f32>(0.0)) {
        return vec4<f32>(0.0);
    }

    let backdrop_blur = b_backdrop_blurs[input.backdrop_blur_id];
    let pixel_position = input.position.xy;

    // Apply Gaussian blur sampling from backdrop texture
    var blurred_color = vec4<f32>(0.0);
    var total_weight = 0.0;

    let blur_radius = backdrop_blur.blur_radius;

    // Skip blur sampling if radius is very small
    if blur_radius < 0.5 {
        let uv = pixel_position / globals.viewport_size;
        blurred_color = textureSample(backdrop_texture, backdrop_sampler, uv);
    } else {
        // Gaussian blur kernel
        let kernel_size = i32(ceil(blur_radius * 2.0));
        let sigma = blur_radius / 2.0;

        for (var dy = -kernel_size; dy <= kernel_size; dy++) {
            for (var dx = -kernel_size; dx <= kernel_size; dx++) {
                let offset = vec2<f32>(f32(dx), f32(dy));
                let weight = gaussian(length(offset), sigma);

                let sample_pos = pixel_position + offset;
                let sample_uv = sample_pos / globals.viewport_size;

                // Clamp UV to valid range
                if sample_uv.x >= 0.0 && sample_uv.x <= 1.0 && sample_uv.y >= 0.0 && sample_uv.y <= 1.0 {
                    let sample_color = textureSample(backdrop_texture, backdrop_sampler, sample_uv);
                    blurred_color += sample_color * weight;
                    total_weight += weight;
                }
            }
        }

        if total_weight > 0.0 {
            blurred_color /= total_weight;
        }
    }

    // Get background color for this element
    let background_color = gradient_color(
        backdrop_blur.background,
        pixel_position,
        backdrop_blur.bounds,
        input.background_solid,
        input.background_color0,
        input.background_color1
    );

    // Composite background color over blurred backdrop (standard over operation)
    let final_color = blurred_color * (1.0 - background_color.a) + background_color;

    // Apply corner radius masking
    let outer_sdf = quad_sdf(pixel_position, backdrop_blur.bounds, backdrop_blur.corner_radii);
    let alpha = saturate(0.5 - outer_sdf);

    return vec4<f32>(final_color.rgb, final_color.a * alpha);
}
