//! Semantic geometry and coverage composition over the shader expression graph.

use super::{Expr, ShaderBuilder, Shape};

/// A signed distance field: negative inside, zero at the boundary, positive outside.
///
/// Coordinates, distances, widths, and softness must share logical-pixel units.
/// This wrapper distinguishes geometry from opacity; it does not prove physical
/// units or that a custom expression is a mathematically exact distance.
/// CSG operations preserve the boundary and sign, but may change distance accuracy.
///
/// ```
/// use gpui::paint::PaintRoot;
/// let shader = PaintRoot::new().shader(|cx| {
///     let outer = cx.circle_field(cx.position(), cx.vec2([24., 24.]), cx.scalar(20.));
///     let hole = cx.circle_field(cx.position(), cx.vec2([30., 24.]), cx.scalar(12.));
///     outer.subtract(hole).coverage().mask(cx.vec4([0.2, 0.6, 1., 1.]))
/// }).unwrap();
/// assert!(!shader.wgsl().is_empty());
/// ```
///
/// Coverage cannot be used as geometry:
///
/// ```compile_fail
/// use gpui::paint::PaintRoot;
/// PaintRoot::new().shader(|cx| {
///     let field = cx.circle_field(cx.position(), cx.vec2([0., 0.]), cx.scalar(10.));
///     field.clone().union(field.coverage()).coverage().mask(cx.vec4([1.; 4]))
/// });
/// ```
#[derive(Clone)]
pub struct Distance(pub(super) Expr<f32>);

impl Distance {
    /// Describe a custom signed distance expression, measured in logical pixels.
    /// The caller supplies the negative-inside sign convention and matching units.
    pub fn new(expression: Expr<f32>) -> Self {
        Self(expression)
    }

    /// Access the distance expression for custom geometry or shading calculations.
    pub fn expression(&self) -> Expr<f32> {
        self.0.clone()
    }

    /// Include points inside either field.
    pub fn union(self, other: Self) -> Self {
        Self(self.0.min(other.0))
    }

    /// Include only points inside both fields.
    pub fn intersect(self, other: Self) -> Self {
        Self(self.0.max(other.0))
    }

    /// Remove the interior of `other` from this field.
    pub fn subtract(self, other: Self) -> Self {
        Self(self.0.max(-other.0))
    }

    /// Move the boundary outward by a signed logical-pixel amount.
    /// Positive values expand the shape; negative values contract it.
    pub fn offset(self, amount: Expr<f32>) -> Self {
        Self(self.0 - amount)
    }

    /// Expand the boundary by a nonnegative logical-pixel amount.
    /// Negative amounts are clamped to zero, including animated values.
    pub fn dilate(self, amount: Expr<f32>) -> Self {
        let zero = scalar(&self.0, 0.);
        self.offset(amount.max(zero))
    }

    /// Contract the boundary by a nonnegative logical-pixel amount.
    /// Negative amounts are clamped to zero, including animated values.
    pub fn inset(self, amount: Expr<f32>) -> Self {
        let zero = scalar(&self.0, 0.);
        self.offset(-amount.max(zero))
    }

    /// Blend the union boundary across a logical-pixel softness distance.
    /// Softness is clamped to a small positive value to keep division defined
    /// when parameters reach zero or become negative.
    pub fn smooth_union(self, other: Self, softness: Expr<f32>) -> Self {
        let zero = scalar(&self.0, 0.);
        let one = scalar(&self.0, 1.);
        let half = scalar(&self.0, 0.5);
        let epsilon = scalar(&self.0, 0.0001);
        let softness = softness.max(epsilon);
        let blend = (half.clone() + half * (other.0.clone() - self.0.clone()) / softness.clone())
            .clamp(zero, one.clone());
        // Polynomial smooth minimum; use the graph's typed interpolation.
        let correction = softness * blend.clone() * (one - blend.clone());
        Self(other.0.mix(self.0, blend) - correction)
    }

    /// Keep an inward band of the given full width in logical pixels.
    /// The outside boundary stays in place; zero or negative widths are empty.
    pub fn inward_stroke(self, width: Expr<f32>) -> Self {
        let zero = scalar(&self.0, 0.);
        let empty = scalar(&self.0, 1.);
        let positive = width.clone().gt(zero);
        let inner = -self.0.clone() - width;
        Self(positive.select(self.0.max(inner), empty))
    }

    /// Keep a band centered on the boundary, with the given full logical-pixel width.
    /// Zero or negative widths are empty.
    pub fn centered_stroke(self, width: Expr<f32>) -> Self {
        let zero = scalar(&self.0, 0.);
        let half = scalar(&self.0, 0.5);
        let empty = scalar(&self.0, 1.);
        let positive = width.clone().gt(zero);
        Self(positive.select(self.0.abs() - width * half, empty))
    }

    /// Convert geometry into antialiased coverage using fragment derivatives.
    pub fn coverage(self) -> Coverage {
        Coverage(self.0.coverage())
    }
}

/// A bounded opacity expression, distinct from signed distance geometry.
/// Combine geometry before converting it to coverage to antialias its final edge.
#[derive(Clone)]
pub struct Coverage(pub(super) Expr<f32>);

impl Coverage {
    /// Construct custom coverage, clamping the expression to the interval 0–1.
    pub fn new(expression: Expr<f32>) -> Self {
        let zero = scalar(&expression, 0.);
        let one = scalar(&expression, 1.);
        Self(expression.clamp(zero, one))
    }

    /// Access the bounded scalar expression for custom shading calculations.
    pub fn expression(&self) -> Expr<f32> {
        self.0.clone()
    }

    /// Reverse covered and uncovered regions.
    pub fn invert(self) -> Self {
        Self(scalar(&self.0, 1.) - self.0)
    }

    /// Multiply opacity by another coverage mask.
    pub fn multiply(self, other: Self) -> Self {
        Self(self.0 * other.0)
    }

    /// Apply coverage to straight-alpha RGBA, preserving the color's RGB channels.
    pub fn mask(self, color: Expr<[f32; 4]>) -> Expr<[f32; 4]> {
        color.mask(self.0)
    }
}

impl ShaderBuilder {
    /// Circle field with point, center, and radius measured in logical pixels.
    pub fn circle_field(
        &self,
        point: Expr<[f32; 2]>,
        center: Expr<[f32; 2]>,
        radius: Expr<f32>,
    ) -> Distance {
        Distance::new(self.circle(point, center, radius))
    }

    /// Rounded rectangle field with all dimensions measured in logical pixels.
    /// `half_size` is half the full rectangle size; radius must fit that size.
    pub fn rounded_rect_field(
        &self,
        point: Expr<[f32; 2]>,
        center: Expr<[f32; 2]>,
        half_size: Expr<[f32; 2]>,
        radius: Expr<f32>,
    ) -> Distance {
        Distance::new(self.rounded_rect(point, center, half_size, radius))
    }
}

impl Shape {
    /// Compose a typed field in coordinates local to this shape's bounds.
    pub fn field(self, cx: &ShaderBuilder) -> Distance {
        self.field_at(cx, cx.position())
    }

    /// Evaluate this fixed descriptor at an explicit point local to its bounds.
    /// Geometry becomes constants, so this method can be used inside a reusable
    /// function without capturing shader coordinates or allocating parameters.
    pub fn field_at(self, cx: &ShaderBuilder, point: Expr<[f32; 2]>) -> Distance {
        Distance::new(self.distance_at(cx, point))
    }
}

fn scalar(expression: &Expr<f32>, value: f32) -> Expr<f32> {
    ShaderBuilder {
        graph: expression.graph.clone(),
    }
    .scalar(value)
}

#[cfg(test)]
mod tests {
    use super::{Distance, Shape};
    use crate::paint::{PaintRoot, ShaderError};
    use crate::{point, px};

    #[test]
    fn fixed_shape_field_can_use_a_closed_function_domain() {
        let root = PaintRoot::new();
        let shape = Shape::Circle {
            center: point(px(50.), px(70.)),
            radius: px(10.),
        };
        let field = root
            .function::<[f32; 2], Distance>(|cx, domain| shape.field_at(cx, domain))
            .unwrap();
        let composed = root
            .shader(|cx| {
                let distance = cx.call(&field, cx.vec2([13., 14.]));
                cx.rgba(
                    distance.expression(),
                    cx.scalar(0.),
                    cx.scalar(0.),
                    cx.scalar(1.),
                )
            })
            .unwrap();
        // Local center is (10,10): this point is five pixels from the center,
        // hence five pixels inside the radius. Match the direct geometry recipe.
        let direct = root
            .shader(|cx| {
                let distance = shape.field_at(cx, cx.vec2([13., 14.]));
                cx.rgba(
                    distance.expression(),
                    cx.scalar(0.),
                    cx.scalar(0.),
                    cx.scalar(1.),
                )
            })
            .unwrap();
        assert_eq!(composed.wgsl(), direct.wgsl());
        assert!(composed.parameter_slots().is_empty());
    }

    #[test]
    fn animated_strokes_select_an_empty_field_for_nonpositive_widths() {
        for inward in [true, false] {
            let shader = PaintRoot::new()
                .shader(|cx| {
                    let width = cx.parameter(0.0);
                    let field = cx.circle_field(cx.position(), cx.vec2([10., 10.]), cx.scalar(10.));
                    let stroke = if inward {
                        field.inward_stroke(cx.uniform(width))
                    } else {
                        field.centered_stroke(cx.uniform(width))
                    };
                    stroke.coverage().mask(cx.vec4([1.; 4]))
                })
                .unwrap();
            assert!(shader.wgsl().contains("select("));
            assert!(shader.wgsl().contains(" > "));
        }
    }

    #[test]
    fn semantic_composition_preserves_graph_ownership_checks() {
        let mut foreign = None;
        PaintRoot::new()
            .shader(|cx| {
                foreign = Some(cx.circle_field(cx.position(), cx.vec2([0., 0.]), cx.scalar(10.)));
                cx.vec4([1.; 4])
            })
            .unwrap();

        let result = PaintRoot::new().shader(|cx| {
            cx.circle_field(cx.position(), cx.vec2([0., 0.]), cx.scalar(20.))
                .subtract(foreign.unwrap())
                .coverage()
                .mask(cx.vec4([1.; 4]))
        });
        assert_eq!(result.unwrap_err(), ShaderError::ForeignGraph);
    }
}
