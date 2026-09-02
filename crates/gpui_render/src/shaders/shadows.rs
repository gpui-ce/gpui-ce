#[wgsl_rs::wgsl]
pub mod shadow {
    use super::super::common::*;
    use wgsl_rs::std::*;

    #[derive(Clone, Copy, Wgsl)]
    pub struct Shadow {
        pub order: u32,
        pub blur_radius: f32,
        pub bounds: Bounds,
        pub corner_radii: Corners,
        pub content_mask: Bounds,
        pub color: Hsla,
        pub element_bounds: Bounds,
        pub element_corner_radii: Corners,
        pub inset: ShaderBool,
        pub padding: u32,
    }
    storage!(group(1), binding(0), SHADOWS: RuntimeArray<Shadow>);

    pub const SHADOW_INTEGRATION_SAMPLE_COUNT: i32 = 4;

    pub fn shadow_geometry(shadow: Shadow) -> Bounds {
        if is_enabled(shadow.inset) {
            return shadow.element_bounds;
        }

        let margin = GAUSSIAN_CUTOFF_STANDARD_DEVIATIONS * shadow.blur_radius;
        Bounds {
            origin: shadow.bounds.origin - vec2f(margin, margin),
            size: shadow.bounds.size + vec2f(2.0 * margin, 2.0 * margin),
        }
    }

    pub fn blurred_shadow_coverage(shadow: Shadow, position: Vec2f) -> f32 {
        let half_size = Bounds::half_size(shadow.bounds);
        let center_to_point = position - Bounds::center(shadow.bounds);
        let corner_radius = pick_corner_radius(center_to_point, shadow.corner_radii);
        let vertical_minimum = center_to_point.y - half_size.y;
        let vertical_maximum = center_to_point.y + half_size.y;
        let integration_radius = GAUSSIAN_CUTOFF_STANDARD_DEVIATIONS * shadow.blur_radius;
        let integration_start = clamp(-integration_radius, vertical_minimum, vertical_maximum);
        let integration_end = clamp(integration_radius, vertical_minimum, vertical_maximum);
        let sample_step =
            (integration_end - integration_start) / SHADOW_INTEGRATION_SAMPLE_COUNT as f32;

        let mut coverage = 0.0;
        let mut sample_index = 0;
        let mut sample_position = integration_start + sample_step * 0.5;
        while sample_index < SHADOW_INTEGRATION_SAMPLE_COUNT {
            coverage += integrated_rounded_rectangle_coverage(
                center_to_point.x,
                center_to_point.y - sample_position,
                shadow.blur_radius,
                corner_radius,
                half_size,
            ) * gaussian(sample_position, shadow.blur_radius)
                * sample_step;
            sample_position += sample_step;
            sample_index += 1;
        }
        coverage
    }

    pub fn shadow_coverage(shadow: Shadow, position: Vec2f) -> f32 {
        let mut coverage = 0.0;
        if shadow.blur_radius == 0.0 {
            coverage = antialiased_coverage(rounded_rectangle_signed_distance(
                position,
                shadow.bounds,
                shadow.corner_radii,
            ));
        } else {
            coverage = blurred_shadow_coverage(shadow, position);
        }

        if is_enabled(shadow.inset) {
            coverage = (1.0 - coverage)
                * antialiased_coverage(rounded_rectangle_signed_distance(
                    position,
                    shadow.element_bounds,
                    shadow.element_corner_radii,
                ));
        }
        coverage
    }

    #[derive(Wgsl)]
    pub struct ShadowVarying {
        #[builtin(position)]
        pub position: Vec4f,
        #[location(0)]
        #[interpolate(flat)]
        pub color: Vec4f,
        #[location(1)]
        #[interpolate(flat)]
        pub shadow_id: u32,
        #[location(3)]
        pub clip_distances: Vec4f,
    }

    #[vertex]
    pub fn vertex_shadow(
        #[builtin(vertex_index)] vertex_id: u32,
        #[builtin(instance_index)] instance_id: u32,
    ) -> ShadowVarying {
        let shadow = get!(SHADOWS)[instance_id as usize];
        let geometry = shadow_geometry(shadow);
        let vertex = rectangle_vertex(vertex_id, geometry);
        ShadowVarying {
            position: vertex.clip_position,
            color: hsla_to_rgba(shadow.color),
            shadow_id: instance_id,
            clip_distances: clip_distances(vertex.viewport_position, shadow.content_mask),
        }
    }

    #[fragment]
    pub fn fragment_shadow(input: ShadowVarying) -> Vec4f {
        if is_clipped(input.clip_distances) {
            return transparent();
        }
        let shadow = get!(SHADOWS)[input.shadow_id as usize];
        blend_color(input.color, shadow_coverage(shadow, input.position.xy()))
    }
}
