#[wgsl_rs::wgsl]
pub mod quad {
    use super::super::common::*;
    use wgsl_rs::std::*;

    #[derive(Clone, Copy, Wgsl)]
    pub struct Quad {
        pub order: u32,
        pub border_style: BorderStyle,
        pub bounds: Bounds,
        pub content_mask: Bounds,
        pub background: Background,
        pub border_color: Hsla,
        pub corner_radii: Corners,
        pub border_widths: Edges,
    }
    storage!(group(1), binding(0), QUADS: RuntimeArray<Quad>);

    pub const DASH_LENGTH_PER_BORDER_WIDTH: f32 = 2.0;
    pub const DASH_GAP_PER_BORDER_WIDTH: f32 = 1.0;
    pub const DEFINITELY_OUTSIDE_INNER_BORDER: f32 = -1.0;

    #[derive(Clone, Copy, Wgsl)]
    pub struct QuadGeometry {
        pub point: Vec2f,
        pub center_to_point: Vec2f,
        pub corner_radius: f32,
        pub corner_center_to_point: Vec2f,
        pub reduced_border: Vec2f,
        pub straight_border_inner_corner_to_point: Vec2f,
        pub near_rounded_corner: bool,
        pub unrounded: bool,
    }

    #[derive(Clone, Copy, Wgsl)]
    pub struct BorderDistances {
        pub outer: f32,
        pub inner: f32,
    }

    #[derive(Clone, Copy, Wgsl)]
    pub struct DashPosition {
        pub position: f32,
        pub perimeter: f32,
        pub velocity: f32,
    }

    #[derive(Clone, Copy, Wgsl)]
    pub struct RoundedDashLayout {
        pub side_velocities: Edges,
        pub corner_velocities: Corners,
        pub right_start: f32,
        pub bottom_right_start: f32,
        pub bottom_left_start: f32,
        pub left_start: f32,
        pub top_left_start: f32,
        pub perimeter: f32,
    }

    pub fn quad_geometry(quad: Quad, position: Vec2f) -> QuadGeometry {
        let half_size = Bounds::half_size(quad.bounds);
        let point = position - quad.bounds.origin;
        let center_to_point = point - half_size;
        let corner_radius = pick_corner_radius(center_to_point, quad.corner_radii);
        let corner_to_point = abs(center_to_point) - half_size;
        let corner_center_to_point = corner_to_point + corner_radius;
        let border = vec2f(
            select(
                quad.border_widths.right,
                quad.border_widths.left,
                center_to_point.x < 0.0,
            ),
            select(
                quad.border_widths.bottom,
                quad.border_widths.top,
                center_to_point.y < 0.0,
            ),
        );
        let reduced_border = vec2f(
            select(border.x, -PIXEL_ANTIALIAS_RADIUS, border.x == 0.0),
            select(border.y, -PIXEL_ANTIALIAS_RADIUS, border.y == 0.0),
        );
        QuadGeometry {
            point,
            center_to_point,
            corner_radius,
            corner_center_to_point,
            reduced_border,
            straight_border_inner_corner_to_point: corner_to_point + reduced_border,
            near_rounded_corner: corner_center_to_point.x >= 0.0 && corner_center_to_point.y >= 0.0,
            unrounded: Corners::is_zero(quad.corner_radii),
        }
    }

    pub fn is_unaffected_background(geometry: QuadGeometry) -> bool {
        geometry.straight_border_inner_corner_to_point.x < -PIXEL_ANTIALIAS_RADIUS
            && geometry.straight_border_inner_corner_to_point.y < -PIXEL_ANTIALIAS_RADIUS
            && !geometry.near_rounded_corner
    }

    pub fn inner_border_signed_distance(geometry: QuadGeometry, outer_signed_distance: f32) -> f32 {
        if geometry.corner_center_to_point.x <= 0.0 || geometry.corner_center_to_point.y <= 0.0 {
            return -max(
                geometry.straight_border_inner_corner_to_point.x,
                geometry.straight_border_inner_corner_to_point.y,
            );
        }
        if geometry.straight_border_inner_corner_to_point.x > 0.0
            || geometry.straight_border_inner_corner_to_point.y > 0.0
        {
            return DEFINITELY_OUTSIDE_INNER_BORDER;
        }
        if geometry.reduced_border.x == geometry.reduced_border.y {
            return -(outer_signed_distance + geometry.reduced_border.x);
        }

        let ellipse_radii = max(
            vec2f(0.0, 0.0),
            geometry.corner_radius - geometry.reduced_border,
        );
        quarter_ellipse_signed_distance(geometry.corner_center_to_point, ellipse_radii)
    }

    pub fn border_distances(geometry: QuadGeometry) -> BorderDistances {
        let outer = rounded_rectangle_signed_distance_from_corner(
            geometry.corner_center_to_point,
            geometry.corner_radius,
        );
        BorderDistances {
            outer,
            inner: inner_border_signed_distance(geometry, outer),
        }
    }

    pub fn dash_velocity(border_width: f32) -> f32 {
        if border_width <= 0.0 {
            0.0
        } else {
            1.0 / (DASH_LENGTH_PER_BORDER_WIDTH + DASH_GAP_PER_BORDER_WIDTH) / border_width
        }
    }

    pub fn straight_dash_position(quad: Quad, geometry: QuadGeometry) -> DashPosition {
        let horizontal = geometry.corner_center_to_point.x < geometry.corner_center_to_point.y;
        let border_width = select(
            max(quad.border_widths.right, quad.border_widths.left),
            max(quad.border_widths.bottom, quad.border_widths.top),
            horizontal,
        );
        let velocity = dash_velocity(border_width);
        DashPosition {
            position: select(geometry.point.y, geometry.point.x, horizontal) * velocity,
            perimeter: select(quad.bounds.size.y, quad.bounds.size.x, horizontal) * velocity,
            velocity,
        }
    }

    pub fn side_dash_velocities(border_widths: Edges) -> Edges {
        Edges {
            top: dash_velocity(border_widths.top),
            right: dash_velocity(border_widths.right),
            bottom: dash_velocity(border_widths.bottom),
            left: dash_velocity(border_widths.left),
        }
    }

    pub fn straight_side_dash_lengths(bounds: Bounds, radii: Corners, velocities: Edges) -> Edges {
        Edges {
            top: (bounds.size.x - radii.top_left - radii.top_right) * velocities.top,
            right: (bounds.size.y - radii.top_right - radii.bottom_right) * velocities.right,
            bottom: (bounds.size.x - radii.bottom_right - radii.bottom_left) * velocities.bottom,
            left: (bounds.size.y - radii.bottom_left - radii.top_left) * velocities.left,
        }
    }

    pub fn corner_dash_velocities(side_velocities: Edges) -> Corners {
        Corners {
            top_left: corner_dash_velocity(side_velocities.top, side_velocities.left),
            top_right: corner_dash_velocity(side_velocities.top, side_velocities.right),
            bottom_right: corner_dash_velocity(side_velocities.bottom, side_velocities.right),
            bottom_left: corner_dash_velocity(side_velocities.bottom, side_velocities.left),
        }
    }

    pub fn corner_dash_lengths(radii: Corners, velocities: Corners) -> Corners {
        let quarter_turn = PI / 2.0;
        Corners {
            top_left: radii.top_left * quarter_turn * velocities.top_left,
            top_right: radii.top_right * quarter_turn * velocities.top_right,
            bottom_right: radii.bottom_right * quarter_turn * velocities.bottom_right,
            bottom_left: radii.bottom_left * quarter_turn * velocities.bottom_left,
        }
    }

    pub fn rounded_dash_layout(quad: Quad) -> RoundedDashLayout {
        let side_velocities = side_dash_velocities(quad.border_widths);
        let side_lengths =
            straight_side_dash_lengths(quad.bounds, quad.corner_radii, side_velocities);
        let corner_velocities = corner_dash_velocities(side_velocities);
        let corner_lengths = corner_dash_lengths(quad.corner_radii, corner_velocities);
        let right_start = side_lengths.top + corner_lengths.top_right;
        let bottom_right_start = right_start + side_lengths.right;
        let bottom_left_start =
            bottom_right_start + corner_lengths.bottom_right + side_lengths.bottom;
        let left_start = bottom_left_start + corner_lengths.bottom_left;
        let top_left_start = left_start + side_lengths.left;
        RoundedDashLayout {
            side_velocities,
            corner_velocities,
            right_start,
            bottom_right_start,
            bottom_left_start,
            left_start,
            top_left_start,
            perimeter: top_left_start + corner_lengths.top_left,
        }
    }

    pub fn rounded_dash_position(quad: Quad, geometry: QuadGeometry) -> DashPosition {
        let radii = quad.corner_radii;
        let dash_layout = rounded_dash_layout(quad);
        let horizontal = geometry.corner_center_to_point.x < geometry.corner_center_to_point.y;
        let on_right = geometry.center_to_point.x >= 0.0;
        let on_bottom = geometry.center_to_point.y >= 0.0;

        let top_position = (geometry.point.x - radii.top_left) * dash_layout.side_velocities.top;
        let right_position = dash_layout.right_start
            + (geometry.point.y - radii.top_right) * dash_layout.side_velocities.right;
        let bottom_position = dash_layout.bottom_left_start
            - (geometry.point.x - radii.bottom_left) * dash_layout.side_velocities.bottom;
        let left_position = dash_layout.top_left_start
            - (geometry.point.y - radii.top_left) * dash_layout.side_velocities.left;
        let horizontal_position = select(top_position, bottom_position, on_bottom);
        let vertical_position = select(left_position, right_position, on_right);
        let side_position = select(vertical_position, horizontal_position, horizontal);
        let horizontal_velocity = select(
            dash_layout.side_velocities.top,
            dash_layout.side_velocities.bottom,
            on_bottom,
        );
        let vertical_velocity = select(
            dash_layout.side_velocities.left,
            dash_layout.side_velocities.right,
            on_right,
        );
        let side_velocity = select(vertical_velocity, horizontal_velocity, horizontal);

        if geometry.near_rounded_corner {
            let corner_position = atan2(
                geometry.corner_center_to_point.y,
                geometry.corner_center_to_point.x,
            ) * geometry.corner_radius;
            let top_right_position =
                dash_layout.right_start - corner_position * dash_layout.corner_velocities.top_right;
            let bottom_right_position = dash_layout.bottom_right_start
                + corner_position * dash_layout.corner_velocities.bottom_right;
            let bottom_left_position = dash_layout.left_start
                - corner_position * dash_layout.corner_velocities.bottom_left;
            let top_left_position = dash_layout.top_left_start
                + corner_position * dash_layout.corner_velocities.top_left;
            let right_position = select(top_right_position, bottom_right_position, on_bottom);
            let left_position = select(top_left_position, bottom_left_position, on_bottom);
            let position = select(left_position, right_position, on_right);
            let right_velocity = select(
                dash_layout.corner_velocities.top_right,
                dash_layout.corner_velocities.bottom_right,
                on_bottom,
            );
            let left_velocity = select(
                dash_layout.corner_velocities.top_left,
                dash_layout.corner_velocities.bottom_left,
                on_bottom,
            );
            return DashPosition {
                position,
                perimeter: dash_layout.perimeter,
                velocity: select(left_velocity, right_velocity, on_right),
            };
        }

        DashPosition {
            position: side_position,
            perimeter: dash_layout.perimeter,
            velocity: side_velocity,
        }
    }

    pub fn dashed_border_alpha(quad: Quad, geometry: QuadGeometry) -> f32 {
        let mut dash = DashPosition {
            position: 0.0,
            perimeter: 0.0,
            velocity: 0.0,
        };
        if geometry.unrounded {
            dash = straight_dash_position(quad, geometry);
        } else {
            dash = rounded_dash_position(quad, geometry);
        }
        let dash_period_per_width = DASH_LENGTH_PER_BORDER_WIDTH + DASH_GAP_PER_BORDER_WIDTH;
        let dash_length = DASH_LENGTH_PER_BORDER_WIDTH / dash_period_per_width;
        let perimeter = dash.perimeter - select(0.0, dash_length, geometry.unrounded);

        if perimeter >= 1.0 {
            let period = perimeter / floor(perimeter);
            dash_coverage(dash.position, period, dash_length, dash.velocity)
        } else if geometry.unrounded && perimeter > dash_length {
            dash_coverage(dash.position, perimeter, dash_length, dash.velocity)
        } else {
            1.0
        }
    }

    #[derive(Wgsl)]
    pub struct QuadVarying {
        #[builtin(position)]
        pub position: Vec4f,
        #[location(0)]
        #[interpolate(flat)]
        pub border_color: Vec4f,
        #[location(1)]
        #[interpolate(flat)]
        pub quad_id: u32,
        #[location(2)]
        pub clip_distances: Vec4f,
        #[location(3)]
        #[interpolate(flat)]
        pub background_solid: Vec4f,
        #[location(4)]
        #[interpolate(flat)]
        pub background_color0: Vec4f,
        #[location(5)]
        #[interpolate(flat)]
        pub background_color1: Vec4f,
    }

    #[vertex]
    pub fn vertex_quad(
        #[builtin(vertex_index)] vertex_id: u32,
        #[builtin(instance_index)] instance_id: u32,
    ) -> QuadVarying {
        let quad = get!(QUADS)[instance_id as usize];
        let vertex = rectangle_vertex(vertex_id, quad.bounds);
        let gradient = prepare_background(quad.background);
        QuadVarying {
            position: vertex.clip_position,
            border_color: hsla_to_rgba(quad.border_color),
            quad_id: instance_id,
            clip_distances: clip_distances(vertex.viewport_position, quad.content_mask),
            background_solid: gradient.solid,
            background_color0: gradient.color0,
            background_color1: gradient.color1,
        }
    }

    #[fragment]
    pub fn fragment_quad(input: QuadVarying) -> Vec4f {
        if is_clipped(input.clip_distances) {
            return transparent();
        }
        let quad = get!(QUADS)[input.quad_id as usize];
        let background_color = background_color(
            quad.background,
            input.position.xy(),
            quad.bounds,
            PreparedBackground {
                solid: input.background_solid,
                color0: input.background_color0,
                color1: input.background_color1,
            },
        );
        if Edges::is_zero(quad.border_widths) && Corners::is_zero(quad.corner_radii) {
            return blend_color(background_color, 1.0);
        }

        let geometry = quad_geometry(quad, input.position.xy());
        if is_unaffected_background(geometry) {
            return blend_color(background_color, 1.0);
        }

        let distances = border_distances(geometry);
        let mut color = background_color;
        if max(distances.inner, distances.outer) < PIXEL_ANTIALIAS_RADIUS {
            let mut border_color = input.border_color;
            if quad.border_style == BorderStyle::Dashed {
                border_color.w *= dashed_border_alpha(quad, geometry);
            }
            let blended_border = over(background_color, border_color);
            let factor = antialiased_coverage(distances.inner);
            color = mix(
                background_color,
                blended_border,
                vec4f(factor, factor, factor, factor),
            );
        }
        blend_color(color, antialiased_coverage(distances.outer))
    }
}
