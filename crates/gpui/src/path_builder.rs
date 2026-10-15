use anyhow::Error;
use etagere::euclid::{Point2D, Vector2D};
use lyon::geom::Angle;
use lyon::math::{Vector, vector};
use lyon::path::traits::SvgPathBuilder;
use lyon::path::{ArcFlags, Polygon};
use lyon::tessellation::{
    BuffersBuilder, FillTessellator, FillVertex, StrokeTessellator, StrokeVertex, VertexBuffers,
};

pub use lyon::math::Transform;
pub use lyon::tessellation::{FillOptions, FillRule, StrokeOptions};

use crate::{Path, Pixels, Point, point, px};

/// How a [`crate::PathBuilder`] tessellates its path: as an outline, or as a filled shape.
pub enum PathStyle {
    /// Draws the outline as a line, with these [`StrokeOptions`].
    Stroke(StrokeOptions),
    /// Fills the interior, with these [`FillOptions`].
    Fill(FillOptions),
}

/// Builds a [`Path`] from lines and curves, and tessellates it into triangles.
///
/// Create a builder with [`crate::PathBuilder::fill`] or [`crate::PathBuilder::stroke`], add commands, and call
/// [`crate::PathBuilder::build`]. Paint the result with [`crate::Window::paint_path`]. Coordinates are in window
/// pixels, so use the bounds that a [`canvas`](crate::canvas) gives you.
///
/// # Examples
///
/// ```
/// # use gpui::{PathBuilder, point, px};
/// let mut builder = PathBuilder::fill();
/// builder.move_to(point(px(0.), px(0.)));
/// builder.line_to(point(px(10.), px(0.)));
/// builder.line_to(point(px(0.), px(10.)));
/// builder.close();
/// let path = builder.build().unwrap();
/// assert_eq!(path.vertices.len() % 3, 0);
/// assert_eq!(path.bounds.size.width, px(10.));
/// assert_eq!(path.bounds.size.height, px(10.));
/// ```
///
/// See [drawing custom graphics](crate::_documentation::guides::custom_drawing) for more.
pub struct PathBuilder {
    raw: lyon::path::builder::WithSvg<lyon::path::BuilderImpl>,
    transform: Option<lyon::math::Transform>,
    /// How [`crate::PathBuilder::build`] tessellates the path. [`crate::PathBuilder::stroke`],
    /// [`crate::PathBuilder::fill`] and [`crate::PathBuilder::with_style`] set it.
    pub style: PathStyle,
    dash_array: Option<Vec<Pixels>>,
}

impl From<lyon::path::Builder> for PathBuilder {
    fn from(builder: lyon::path::Builder) -> Self {
        Self {
            raw: builder.with_svg(),
            ..Default::default()
        }
    }
}

impl From<lyon::path::builder::WithSvg<lyon::path::BuilderImpl>> for PathBuilder {
    fn from(raw: lyon::path::builder::WithSvg<lyon::path::BuilderImpl>) -> Self {
        Self {
            raw,
            ..Default::default()
        }
    }
}

impl From<lyon::math::Point> for Point<Pixels> {
    fn from(p: lyon::math::Point) -> Self {
        point(px(p.x), px(p.y))
    }
}

impl From<Point<Pixels>> for lyon::math::Point {
    fn from(p: Point<Pixels>) -> Self {
        lyon::math::point(p.x.0, p.y.0)
    }
}

impl From<Point<Pixels>> for Vector {
    fn from(p: Point<Pixels>) -> Self {
        vector(p.x.0, p.y.0)
    }
}

impl From<Point<Pixels>> for Point2D<f32, Pixels> {
    fn from(p: Point<Pixels>) -> Self {
        Point2D::new(p.x.0, p.y.0)
    }
}

impl Default for PathBuilder {
    fn default() -> Self {
        Self {
            raw: lyon::path::Path::builder().with_svg(),
            style: PathStyle::Fill(FillOptions::default()),
            transform: None,
            dash_array: None,
        }
    }
}

impl PathBuilder {
    /// Returns a builder that strokes the path with a line `width` logical pixels wide.
    ///
    /// The stroke has butt caps and miter joins, with a miter limit of 4. Use [`crate::PathBuilder::fill`] for a
    /// filled shape.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{PathBuilder, point, px, size};
    /// let mut builder = PathBuilder::stroke(px(2.));
    /// builder.move_to(point(px(0.), px(5.)));
    /// builder.line_to(point(px(20.), px(5.)));
    /// let path = builder.build().unwrap();
    /// assert_eq!(path.bounds.origin, point(px(0.), px(4.)));
    /// assert_eq!(path.bounds.size, size(px(20.), px(2.)));
    /// ```
    pub fn stroke(width: Pixels) -> Self {
        Self {
            style: PathStyle::Stroke(StrokeOptions::default().with_line_width(width.0)),
            ..Self::default()
        }
    }

    /// Returns a builder that fills the interior of the path using the even-odd rule.
    ///
    /// An open sub-path is closed implicitly when it is filled. For the non-zero rule, pass a style to
    /// [`crate::PathBuilder::with_style`].
    pub fn fill() -> Self {
        Self::default()
    }

    /// Replaces the style of this builder.
    ///
    /// Use this for options that [`crate::PathBuilder::stroke`] and [`crate::PathBuilder::fill`] don't set, such as the
    /// non-zero [`FillRule`].
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{FillOptions, FillRule, PathBuilder, PathStyle, point, px};
    /// let style = PathStyle::Fill(FillOptions::default().with_fill_rule(FillRule::NonZero));
    /// let mut builder = PathBuilder::fill().with_style(style);
    /// builder.move_to(point(px(0.), px(0.)));
    /// builder.line_to(point(px(10.), px(0.)));
    /// builder.line_to(point(px(0.), px(10.)));
    /// builder.close();
    /// assert!(builder.build().is_ok());
    /// ```
    pub fn with_style(self, style: PathStyle) -> Self {
        Self { style, ..self }
    }

    /// Sets the dash pattern of a stroked path.
    ///
    /// The values alternate between dash and gap lengths, in logical pixels, measured along the path.
    /// An odd number of values is repeated, so `[5, 3, 2]` is the same as `[5, 3, 2, 5, 3, 2]`. The
    /// pattern has no effect on fills.
    ///
    /// Pass a non-empty slice of finite, positive lengths. [`crate::PathBuilder::build`] returns an
    /// error for an empty or invalid pattern, including a length too small to advance along the path.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{PathBuilder, point, px};
    /// let mut solid = PathBuilder::stroke(px(1.));
    /// solid.move_to(point(px(0.), px(0.)));
    /// solid.line_to(point(px(20.), px(0.)));
    /// let solid = solid.build().unwrap();
    ///
    /// let mut dashed = PathBuilder::stroke(px(1.)).dash_array(&[px(4.), px(4.)]);
    /// dashed.move_to(point(px(0.), px(0.)));
    /// dashed.line_to(point(px(20.), px(0.)));
    /// let dashed = dashed.build().unwrap();
    ///
    /// assert!(dashed.vertices.len() > solid.vertices.len());
    /// assert_eq!(dashed.bounds.size.width, px(20.));
    /// ```
    pub fn dash_array(mut self, dash_array: &[Pixels]) -> Self {
        // If an odd number of values is provided, then the list of values is repeated to yield an even number of values.
        // Thus, 5,3,2 is equivalent to 5,3,2,5,3,2.
        let array = if dash_array.len() % 2 == 1 {
            let mut new_dash_array = dash_array.to_vec();
            new_dash_array.extend_from_slice(dash_array);
            new_dash_array
        } else {
            dash_array.to_vec()
        };

        self.dash_array = Some(array);
        self
    }

    /// Starts a new sub-path at `to`. The current point becomes `to`.
    #[inline]
    pub fn move_to(&mut self, to: Point<Pixels>) {
        self.raw.move_to(to.into());
    }

    /// Adds a straight line from the current point to `to`.
    #[inline]
    pub fn line_to(&mut self, to: Point<Pixels>) {
        self.raw.line_to(to.into());
    }

    /// Adds a quadratic Bézier curve from the current point to `to`, bent toward the control point `ctrl`.
    #[inline]
    pub fn curve_to(&mut self, to: Point<Pixels>, ctrl: Point<Pixels>) {
        self.raw.quadratic_bezier_to(ctrl.into(), to.into());
    }

    /// Adds a cubic Bézier curve from the current point to `to`, shaped by the control points
    /// `control_a` and `control_b`.
    #[inline]
    pub fn cubic_bezier_to(
        &mut self,
        to: Point<Pixels>,
        control_a: Point<Pixels>,
        control_b: Point<Pixels>,
    ) {
        self.raw
            .cubic_bezier_to(control_a.into(), control_b.into(), to.into());
    }

    /// Adds an elliptical arc from the current point to `to`, in absolute coordinates.
    ///
    /// `radii` gives the x and y radii in logical pixels. `x_rotation` is the rotation of the ellipse in
    /// **degrees**, even though its type is [`Pixels`]. `large_arc` and `sweep` choose one of the four
    /// possible arcs, as they do in SVG path data.
    pub fn arc_to(
        &mut self,
        radii: Point<Pixels>,
        x_rotation: Pixels,
        large_arc: bool,
        sweep: bool,
        to: Point<Pixels>,
    ) {
        self.raw.arc_to(
            radii.into(),
            Angle::degrees(x_rotation.into()),
            ArcFlags { large_arc, sweep },
            to.into(),
        );
    }

    /// Adds an elliptical arc like [`crate::PathBuilder::arc_to`], but `to` is relative to the current point.
    /// `x_rotation` is in degrees, as it is for `arc_to`.
    pub fn relative_arc_to(
        &mut self,
        radii: Point<Pixels>,
        x_rotation: Pixels,
        large_arc: bool,
        sweep: bool,
        to: Point<Pixels>,
    ) {
        self.raw.relative_arc_to(
            radii.into(),
            Angle::degrees(x_rotation.into()),
            ArcFlags { large_arc, sweep },
            to.into(),
        );
    }

    /// Adds `points` as a sub-path. If `closed` is `true`, the last point connects back to the first.
    pub fn add_polygon(&mut self, points: &[Point<Pixels>], closed: bool) {
        let points = points.iter().copied().map(|p| p.into()).collect::<Vec<_>>();
        self.raw.add_polygon(Polygon {
            points: points.as_ref(),
            closed,
        });
    }

    /// Closes the current sub-path with a line back to its start.
    #[inline]
    pub fn close(&mut self) {
        self.raw.close();
    }

    /// Replaces the transform that [`crate::PathBuilder::build`] applies with `transform`.
    ///
    /// This discards the effect of any [`crate::PathBuilder::translate`], [`crate::PathBuilder::scale`] or
    /// [`crate::PathBuilder::rotate`] calls made before it.
    #[inline]
    pub fn transform(&mut self, transform: Transform) {
        self.transform = Some(transform);
    }

    /// Moves the path by `to` when it is built.
    ///
    /// Each transform call applies to the path after the transform calls before it.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{PathBuilder, point, px};
    /// let mut builder = PathBuilder::fill();
    /// builder.move_to(point(px(0.), px(0.)));
    /// builder.line_to(point(px(10.), px(0.)));
    /// builder.line_to(point(px(0.), px(10.)));
    /// builder.translate(point(px(5.), px(5.)));
    /// let path = builder.build().unwrap();
    /// assert_eq!(path.bounds.origin, point(px(5.), px(5.)));
    /// ```
    #[inline]
    pub fn translate(&mut self, to: Point<Pixels>) {
        if let Some(transform) = self.transform {
            self.transform = Some(transform.then_translate(Vector2D::new(to.x.0, to.y.0)));
        } else {
            self.transform = Some(Transform::translation(to.x.0, to.y.0))
        }
    }

    /// Scales the path by `scale` on both axes when it is built.
    ///
    /// Stroke widths are not scaled, because the geometry is transformed before it is stroked.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{PathBuilder, point, px};
    /// let mut builder = PathBuilder::fill();
    /// builder.move_to(point(px(0.), px(0.)));
    /// builder.line_to(point(px(10.), px(0.)));
    /// builder.line_to(point(px(0.), px(10.)));
    /// builder.scale(2.);
    /// let path = builder.build().unwrap();
    /// assert_eq!(path.bounds.size.width, px(20.));
    /// assert_eq!(path.bounds.size.height, px(20.));
    /// ```
    #[inline]
    pub fn scale(&mut self, scale: f32) {
        if let Some(transform) = self.transform {
            self.transform = Some(transform.then_scale(scale, scale));
        } else {
            self.transform = Some(Transform::scale(scale, scale));
        }
    }

    /// Rotates the path by `angle` degrees around the origin when it is built.
    #[inline]
    pub fn rotate(&mut self, angle: f32) {
        let radians = angle.to_radians();
        if let Some(transform) = self.transform {
            self.transform = Some(transform.then_rotate(Angle::radians(radians)));
        } else {
            self.transform = Some(Transform::rotation(Angle::radians(radians)));
        }
    }

    /// Tessellates the path and returns it as a [`Path`].
    ///
    /// The transform is applied to the geometry first. A fill uses the [`FillOptions`] of its style. A
    /// stroke uses the [`StrokeOptions`] of its style and the dash array.
    ///
    /// Building is the expensive step. Build a path once, and keep it while its shape doesn't change,
    /// rather than rebuilding it every frame.
    ///
    /// # Errors
    ///
    /// Returns an error if a point or the tolerance is NaN, if the mesh needs more vertices than a
    /// `u16` index can address, or if the dash array is empty, has a length that isn't finite and
    /// positive, or has a length too small to advance along the path.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{PathBuilder, point, px};
    /// let mut builder = PathBuilder::fill();
    /// builder.move_to(point(px(0.), px(0.)));
    /// builder.line_to(point(px(10.), px(0.)));
    /// builder.line_to(point(px(0.), px(10.)));
    /// builder.close();
    /// let path = builder.build().unwrap();
    /// assert!(path.vertices.len() >= 3);
    /// assert_eq!(path.vertices.len() % 3, 0);
    /// ```
    #[inline]
    pub fn build(self) -> Result<Path<Pixels>, Error> {
        let path = if let Some(transform) = self.transform {
            self.raw.build().transformed(&transform)
        } else {
            self.raw.build()
        };

        match self.style {
            PathStyle::Stroke(options) => Self::tessellate_stroke(self.dash_array, &path, &options),
            PathStyle::Fill(options) => Self::tessellate_fill(&path, &options),
        }
    }

    fn tessellate_fill(
        path: &lyon::path::Path,
        options: &FillOptions,
    ) -> Result<Path<Pixels>, Error> {
        // Will contain the result of the tessellation.
        let mut buf: VertexBuffers<lyon::math::Point, u16> = VertexBuffers::new();
        let mut tessellator = FillTessellator::new();

        // Compute the tessellation.
        tessellator.tessellate_path(
            path,
            options,
            &mut BuffersBuilder::new(&mut buf, |vertex: FillVertex| vertex.position()),
        )?;

        Ok(Self::build_path(buf))
    }

    fn tessellate_stroke(
        dash_array: Option<Vec<Pixels>>,
        path: &lyon::path::Path,
        options: &StrokeOptions,
    ) -> Result<Path<Pixels>, Error> {
        let path = if let Some(dash_array) = dash_array {
            anyhow::ensure!(
                !dash_array.is_empty()
                    && dash_array
                        .iter()
                        .all(|length| length.0.is_finite() && length.0 > 0.),
                "dash lengths must be finite, positive, and non-empty"
            );
            let measurements = lyon::algorithms::measure::PathMeasurements::from_path(path, 0.01);
            let mut sampler = measurements
                .create_sampler(path, lyon::algorithms::measure::SampleType::Normalized);
            let mut builder = lyon::path::Path::builder();

            let total_length = sampler.length();
            let dash_array_len = dash_array.len();
            let mut pos = 0.;
            let mut dash_index = 0;
            while pos < total_length {
                let dash_length = dash_array[dash_index % dash_array_len].0;
                let next_pos = (pos + dash_length).min(total_length);
                // A dash too short to move `pos` in floating point would loop forever.
                anyhow::ensure!(
                    next_pos > pos,
                    "dash length is too small to advance along the path"
                );
                if dash_index % 2 == 0 {
                    let start = pos / total_length;
                    let end = next_pos / total_length;
                    sampler.split_range(start..end, &mut builder);
                }
                pos = next_pos;
                dash_index += 1;
            }

            &builder.build()
        } else {
            path
        };

        // Will contain the result of the tessellation.
        let mut buf: VertexBuffers<lyon::math::Point, u16> = VertexBuffers::new();
        let mut tessellator = StrokeTessellator::new();

        // Compute the tessellation.
        tessellator.tessellate_path(
            path,
            options,
            &mut BuffersBuilder::new(&mut buf, |vertex: StrokeVertex| vertex.position()),
        )?;

        Ok(Self::build_path(buf))
    }

    /// Converts triangles from a lyon `VertexBuffers` into a [`Path`]. Each run of three indices is one
    /// triangle. An empty buffer gives an empty path at the origin.
    ///
    /// # Panics
    ///
    /// Panics if an index is out of range for the buffer's vertices.
    pub fn build_path(buf: VertexBuffers<lyon::math::Point, u16>) -> Path<Pixels> {
        if buf.vertices.is_empty() {
            return Path::new(Point::default());
        }

        let first_point = buf.vertices[0];

        let mut path = Path::new(first_point.into());
        for i in 0..buf.indices.len() / 3 {
            let i0 = buf.indices[i * 3] as usize;
            let i1 = buf.indices[i * 3 + 1] as usize;
            let i2 = buf.indices[i * 3 + 2] as usize;

            let v0 = buf.vertices[i0];
            let v1 = buf.vertices[i1];
            let v2 = buf.vertices[i2];

            path.push_triangle(
                (v0.into(), v1.into(), v2.into()),
                (point(0., 1.), point(0., 1.), point(0., 1.)),
            );
        }

        path
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dashed_line(dash_array: &[Pixels]) -> Result<Path<Pixels>, Error> {
        let mut builder = PathBuilder::stroke(px(1.)).dash_array(dash_array);
        builder.move_to(point(px(0.), px(0.)));
        builder.line_to(point(px(10.), px(0.)));
        builder.build()
    }

    #[test]
    fn invalid_dash_arrays_return_errors() {
        for dash_array in [
            vec![],
            vec![px(0.)],
            vec![px(-1.)],
            vec![px(f32::NAN)],
            vec![px(f32::INFINITY)],
        ] {
            assert!(dashed_line(&dash_array).is_err(), "accepted {dash_array:?}");
        }
    }

    #[test]
    fn dash_lengths_must_advance_in_floating_point() {
        assert!(dashed_line(&[px(1.), px(f32::MIN_POSITIVE)]).is_err());
    }

    #[test]
    fn valid_dash_array_builds() {
        assert!(dashed_line(&[px(2.), px(1.)]).is_ok());
    }
}
