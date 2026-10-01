//! Retained canvas composition using the same paints as procedural shaders.

use std::{collections::HashMap, sync::Arc};

use super::{
    Expr, PaintRoot, Parameter, ParameterUpdates, Shader, ShaderBuilder, ShaderError, ShaderType,
};
use crate::{
    Background, Bounds, ContentMask, Hsla, Path, Pixels, Point, Rgba, Window, fill, outline, point,
    px, size,
};

/// A surface paint, shared by canvas fills and borders.
#[derive(Clone)]
pub enum Paint {
    /// A native GPUI color, gradient, or pattern.
    Background(Background),
    /// A typed procedural shader evaluated in the shape's bounds.
    Shader(Shader),
}

impl From<Background> for Paint {
    fn from(value: Background) -> Self {
        Self::Background(value)
    }
}
impl From<Hsla> for Paint {
    fn from(value: Hsla) -> Self {
        Self::Background(value.into())
    }
}
impl From<Rgba> for Paint {
    fn from(value: Rgba) -> Self {
        Self::Background(value.into())
    }
}
impl From<Shader> for Paint {
    fn from(value: Shader) -> Self {
        Self::Shader(value)
    }
}
impl From<&Shader> for Paint {
    fn from(value: &Shader) -> Self {
        Self::Shader(value.clone())
    }
}

#[derive(Clone)]
enum Geometry {
    Rect(Bounds<Pixels>, Paint),
    Quad(crate::PaintQuad),
    Path(Arc<Path<Pixels>>, Background),
}

#[derive(Clone)]
struct Command {
    geometry: Geometry,
    clip: Option<Bounds<Pixels>>,
    opacity: f32,
}

/// A reusable drawing recorded in logical pixel coordinates.
///
/// Paths are tessellated before recording; replay clones their vertex buffers
/// before submitting existing geometry.
/// Shader rectangles reuse their compiled graph and parameter values.
#[derive(Clone, Default)]
pub struct CanvasDrawing {
    commands: Arc<[Command]>,
}

impl CanvasDrawing {
    /// Update a logical parameter everywhere it appears in this drawing.
    ///
    /// Programs and path geometry remain shared; updating values does not rebuild
    /// shader graphs or tessellate paths. The original drawing remains unchanged.
    /// Unknown handles return [`ShaderError::ForeignGraph`].
    ///
    /// ```
    /// use gpui::{Bounds, point, px, size};
    /// use gpui::paint::PaintRoot;
    /// # fn example() -> Result<(), gpui::paint::ShaderError> {
    /// let root = PaintRoot::new();
    /// let tint = root.parameter([0.2, 0.4, 0.8, 1.0])?;
    /// let material = root.shader(|cx| cx.uniform(tint))?;
    /// let drawing = root.canvas(|canvas| {
    ///     let bounds = Bounds::new(point(px(0.0), px(0.0)), size(px(160.0), px(80.0)));
    ///     canvas.rounded_rect(bounds, px(12.0)).fill(&material);
    ///     canvas.circle(point(px(80.0), px(40.0)), px(30.0)).stroke(&material, px(3.0));
    /// })?;
    /// let animated = drawing.with_parameters(|patch| {
    ///     patch.set(tint, [0.8, 0.3, 0.2, 0.7]);
    /// })?;
    /// assert_eq!(animated.primitive_count(), drawing.primitive_count());
    /// // During the canvas paint callback: animated.paint_at(bounds.origin, window)?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn with_parameter<T: ShaderType>(
        &self,
        parameter: Parameter<T>,
        value: T,
    ) -> Result<Self, ShaderError> {
        self.with_parameters(|patch| {
            patch.set(parameter, value);
        })
    }

    /// Atomically update typed parameters across all matching paints.
    ///
    /// Every handle must appear in at least one paint in the drawing. Values are
    /// copied once per affected shader instance; untouched geometry stays shared.
    /// Empty updates and unchanged values share the original command table.
    pub fn with_parameters(
        &self,
        update: impl FnOnce(&mut ParameterUpdates),
    ) -> Result<Self, ShaderError> {
        let patch = ParameterUpdates::new(update)?;
        if patch.is_empty() {
            return Ok(self.clone());
        }
        for key in patch.keys() {
            if !self.commands.iter().any(|command| {
                matches!(&command.geometry, Geometry::Rect(_, Paint::Shader(shader)) if shader.has_parameter_key(key))
            }) {
                return Err(ShaderError::ForeignGraph);
            }
        }
        let mut updated = HashMap::new();
        let mut commands: Option<Vec<Command>> = None;
        for (index, command) in self.commands.iter().enumerate() {
            if let Geometry::Rect(_, Paint::Shader(shader)) = &command.geometry {
                if !patch.keys().any(|key| shader.has_parameter_key(key)) {
                    continue;
                }
                let replacement = updated
                    .entry(shader.update_identity())
                    .or_insert_with(|| shader.updated_parameter_values(&patch));
                if !Arc::ptr_eq(&shader.values, replacement) {
                    let commands = commands.get_or_insert_with(|| self.commands.to_vec());
                    if let Geometry::Rect(_, Paint::Shader(shader)) = &mut commands[index].geometry
                    {
                        shader.values = replacement.clone();
                    }
                }
            }
        }
        Ok(commands.map_or_else(
            || self.clone(),
            |commands| Self {
                commands: commands.into(),
            },
        ))
    }

    /// Submit the drawing during the window's paint phase.
    pub fn paint(&self, window: &mut Window) -> Result<(), ShaderError> {
        self.paint_at(Point::default(), window)
    }

    /// Submit the drawing with its origin translated to `origin`.
    pub fn paint_at(&self, origin: Point<Pixels>, window: &mut Window) -> Result<(), ShaderError> {
        if !origin.x.0.is_finite() || !origin.y.0.is_finite() {
            return Err(ShaderError::InvalidGeometry);
        }
        let scale = window.scale_factor();
        if !scale.is_finite() || scale <= 0.0 {
            return Err(ShaderError::InvalidGeometry);
        }
        for command in self.commands.iter() {
            if !finite_geometry(&command.geometry, origin, scale)
                || command.clip.is_some_and(|mut bounds| {
                    bounds.origin += origin;
                    !finite_bounds(bounds, scale)
                })
            {
                return Err(ShaderError::InvalidGeometry);
            }
        }
        if !window.supports_shader_paint()
            && self
                .commands
                .iter()
                .any(|command| matches!(command.geometry, Geometry::Rect(_, Paint::Shader(_))))
        {
            return Err(ShaderError::UnsupportedBackend);
        }
        for command in self.commands.iter() {
            let mask = command.clip.map(|mut bounds| {
                bounds.origin += origin;
                ContentMask {
                    bounds,
                    ..Default::default()
                }
            });
            window.with_content_mask(mask, |window| {
                window.with_element_opacity(Some(command.opacity), |window| {
                    match &command.geometry {
                        Geometry::Rect(bounds, paint) => {
                            let mut bounds = *bounds;
                            bounds.origin += origin;
                            match paint {
                                Paint::Background(background) => {
                                    window.paint_quad(fill(bounds, *background))
                                }
                                Paint::Shader(shader) => window.paint_shader(bounds, shader)?,
                            }
                        }
                        Geometry::Quad(quad) => {
                            let mut quad = quad.clone();
                            quad.bounds.origin += origin;
                            window.paint_quad(quad);
                        }
                        Geometry::Path(path, background) => {
                            let mut path = (**path).clone();
                            translate_path(&mut path, origin);
                            window.paint_path(path, *background);
                        }
                    }
                    Ok::<(), ShaderError>(())
                })
            })?;
        }
        Ok(())
    }

    /// Number of recorded primitive submissions. A shader fill or border uses one.
    pub fn primitive_count(&self) -> usize {
        self.commands.len()
    }
}

/// Records ordered canvas primitives, clips, translations, and opacity.
///
/// Opacity multiplies each primitive's alpha. It is not an offscreen group
/// compositing operation: overlapping primitives remain independently blended.
pub struct CanvasBuilder {
    root: PaintRoot,
    commands: Vec<Command>,
    offset: Point<Pixels>,
    clip: Option<Bounds<Pixels>>,
    opacity: f32,
    error: Option<ShaderError>,
}

impl Default for CanvasBuilder {
    fn default() -> Self {
        Self::new(PaintRoot::new())
    }
}

impl CanvasBuilder {
    fn new(root: PaintRoot) -> Self {
        Self {
            root,
            commands: Vec::new(),
            offset: Point::default(),
            clip: None,
            opacity: 1.0,
            error: None,
        }
    }
}

impl PaintRoot {
    /// Build a retained drawing with the root's shared paint vocabulary.
    pub fn canvas(
        &self,
        build: impl FnOnce(&mut CanvasBuilder),
    ) -> Result<CanvasDrawing, ShaderError> {
        let mut canvas = CanvasBuilder::new(self.clone());
        build(&mut canvas);
        canvas.build()
    }
}

impl CanvasBuilder {
    /// Start a rectangle. Fills and strokes are recorded in call order.
    pub fn rect(&mut self, bounds: Bounds<Pixels>) -> CanvasShape<'_> {
        self.shape(Shape::Rect {
            bounds,
            radius: px(0.0),
        })
    }

    /// Start a rectangle with a uniform corner radius.
    pub fn rounded_rect(&mut self, bounds: Bounds<Pixels>, radius: Pixels) -> CanvasShape<'_> {
        self.shape(Shape::Rect { bounds, radius })
    }

    /// Start a circle with logical pixel coordinates.
    pub fn circle(&mut self, center: Point<Pixels>, radius: Pixels) -> CanvasShape<'_> {
        self.shape(Shape::Circle { center, radius })
    }

    /// Paint a reusable shape descriptor.
    pub fn shape(&mut self, shape: Shape) -> CanvasShape<'_> {
        CanvasShape {
            canvas: self,
            shape,
        }
    }

    /// Record an already tessellated native path.
    /// Shader paths require a coverage mask and are deliberately not accepted here.
    pub fn path(&mut self, mut path: Path<Pixels>, background: impl Into<Background>) -> &mut Self {
        translate_path(&mut path, self.offset);
        self.push(Geometry::Path(Arc::new(path), background.into()));
        self
    }

    /// Clip the nested drawing to an axis-aligned rectangle in current coordinates.
    pub fn clipped(
        &mut self,
        mut bounds: Bounds<Pixels>,
        build: impl FnOnce(&mut Self),
    ) -> &mut Self {
        bounds.origin += self.offset;
        if !finite_bounds(bounds, 1.0) {
            self.error.get_or_insert(ShaderError::InvalidGeometry);
            return self;
        }
        if bounds.size.width < px(0.0) || bounds.size.height < px(0.0) {
            self.error.get_or_insert(ShaderError::InvalidGeometry);
            return self;
        }
        let previous = self.clip;
        self.clip = Some(previous.map_or(bounds, |clip| clip.intersect(&bounds)));
        build(self);
        self.clip = previous;
        self
    }

    /// Translate all geometry and clips recorded inside the closure.
    pub fn translated(
        &mut self,
        offset: Point<Pixels>,
        build: impl FnOnce(&mut Self),
    ) -> &mut Self {
        if !offset.x.0.is_finite() || !offset.y.0.is_finite() {
            self.error.get_or_insert(ShaderError::InvalidGeometry);
            return self;
        }
        let previous = self.offset;
        self.offset += offset;
        if !self.offset.x.0.is_finite() || !self.offset.y.0.is_finite() {
            self.error.get_or_insert(ShaderError::InvalidGeometry);
            self.offset = previous;
            return self;
        }
        build(self);
        self.offset = previous;
        self
    }

    /// Multiply the alpha of each nested primitive by a value clamped to `[0, 1]`.
    /// Non-finite values fail the drawing.
    pub fn opacity(&mut self, opacity: f32, build: impl FnOnce(&mut Self)) -> &mut Self {
        if !opacity.is_finite() {
            self.error.get_or_insert(ShaderError::InvalidGeometry);
            return self;
        }
        let previous = self.opacity;
        self.opacity *= opacity.clamp(0.0, 1.0);
        build(self);
        self.opacity = previous;
        self
    }

    /// Finish recording a reusable drawing.
    pub fn build(self) -> Result<CanvasDrawing, ShaderError> {
        match self.error {
            Some(error) => Err(error),
            None => Ok(CanvasDrawing {
                commands: self.commands.into(),
            }),
        }
    }

    fn push(&mut self, geometry: Geometry) {
        if !finite_geometry(&geometry, Point::default(), 1.0) {
            self.error.get_or_insert(ShaderError::InvalidGeometry);
            return;
        }
        if self.opacity == 0.0 || self.clip.is_some_and(|clip| clip.is_empty()) {
            return;
        }
        self.commands.push(Command {
            geometry,
            clip: self.clip,
            opacity: self.opacity,
        });
    }

    fn rectangle(&mut self, mut bounds: Bounds<Pixels>, paint: Paint) {
        if bounds.is_empty() {
            return;
        }
        bounds.origin += self.offset;
        self.push(Geometry::Rect(bounds, paint));
    }
}

/// Geometry shared between canvas painting and typed signed distance composition.
#[derive(Clone, Copy, Debug)]
pub enum Shape {
    /// An axis-aligned rectangle with a uniform corner radius.
    Rect {
        /// Logical pixel bounds.
        bounds: Bounds<Pixels>,
        /// Corner radius, clamped to fit the rectangle.
        radius: Pixels,
    },
    /// A circle.
    Circle {
        /// Logical pixel center.
        center: Point<Pixels>,
        /// Radius in logical pixels.
        radius: Pixels,
    },
}

impl Shape {
    /// Bounds used to evaluate a shape's paint coordinates.
    pub fn bounds(self) -> Bounds<Pixels> {
        match self {
            Self::Rect { bounds, .. } => bounds,
            Self::Circle { center, radius } => Bounds::new(
                center - point(radius, radius),
                size(radius * 2.0, radius * 2.0),
            ),
        }
    }

    fn radius(self) -> Pixels {
        match self {
            Self::Rect { bounds, radius } => radius
                .max(px(0.0))
                .min(bounds.size.width / 2.0)
                .min(bounds.size.height / 2.0),
            Self::Circle { radius, .. } => radius,
        }
    }

    fn raw_radius(self) -> Pixels {
        match self {
            Self::Rect { radius, .. } | Self::Circle { radius, .. } => radius,
        }
    }

    /// Compose the shape's signed distance in coordinates local to its bounds.
    /// Negative values are inside the shape; units are logical pixels.
    /// The fixed descriptor contributes constants, not uniform parameters.
    pub fn distance(self, cx: &ShaderBuilder) -> Expr<f32> {
        self.distance_at(cx, cx.position())
    }

    pub(super) fn distance_at(self, cx: &ShaderBuilder, point: Expr<[f32; 2]>) -> Expr<f32> {
        self.distance_fields(cx, point, cx.vec4(self.fields(None)))
    }

    fn fields(self, stroke: Option<Pixels>) -> [f32; 4] {
        let bounds = self.bounds();
        [
            bounds.size.width.0 * 0.5,
            bounds.size.height.0 * 0.5,
            self.radius().0,
            stroke.map_or(0.0, |width| width.0),
        ]
    }

    fn distance_fields(
        self,
        cx: &ShaderBuilder,
        point: Expr<[f32; 2]>,
        fields: Expr<[f32; 4]>,
    ) -> Expr<f32> {
        let half = cx.xy(fields.clone().x(), fields.clone().y());
        match self {
            Self::Rect { .. } => cx.rounded_rect(point, half.clone(), half, fields.z()),
            Self::Circle { .. } => cx.circle(point, half, fields.z()),
        }
    }
}

/// A shape under construction, borrowing its canvas recorder.
pub struct CanvasShape<'a> {
    canvas: &'a mut CanvasBuilder,
    shape: Shape,
}

impl CanvasShape<'_> {
    /// Fill the shape with a native background or procedural shader.
    pub fn fill(mut self, paint: impl Into<Paint>) -> Self {
        self.record(paint.into(), None);
        self
    }

    /// Draw an inward border using the same paint and coordinate system as a fill.
    /// Zero width records nothing; negative or non-finite widths fail the drawing.
    pub fn stroke(mut self, paint: impl Into<Paint>, width: Pixels) -> Self {
        if !width.0.is_finite() || width < px(0.0) {
            self.canvas
                .error
                .get_or_insert(ShaderError::InvalidGeometry);
        } else if width > px(0.0) {
            self.record(paint.into(), Some(width));
        }
        self
    }

    fn record(&mut self, paint: Paint, stroke: Option<Pixels>) {
        let mut bounds = self.shape.bounds();
        if bounds.size.width < px(0.0) || bounds.size.height < px(0.0) {
            self.canvas
                .error
                .get_or_insert(ShaderError::InvalidGeometry);
            return;
        }
        if ![
            bounds.origin.x.0,
            bounds.origin.y.0,
            bounds.size.width.0,
            bounds.size.height.0,
            self.shape.raw_radius().0,
        ]
        .into_iter()
        .all(f32::is_finite)
        {
            self.canvas
                .error
                .get_or_insert(ShaderError::InvalidGeometry);
            return;
        }
        if bounds.is_empty() {
            return;
        }
        let radius = self.shape.radius();
        match paint {
            Paint::Background(background) => {
                let mut quad = if let Some(width) = stroke {
                    outline(bounds, background, Default::default()).border_widths(width)
                } else {
                    fill(bounds, background)
                }
                .corner_radii(radius);
                bounds.origin += self.canvas.offset;
                quad.bounds = bounds;
                self.canvas.push(Geometry::Quad(quad));
            }
            Paint::Shader(shader) => {
                let shape = self.shape;
                let mapped = self.canvas.root.map_shader(&shader, |cx, color| {
                    let fields = cx.parameter(shape.fields(stroke));
                    let fields = cx.uniform(fields);
                    let distance = shape.distance_fields(cx, cx.position(), fields.clone());
                    let distance = if stroke.is_some() {
                        distance.clone().max(-distance - fields.w())
                    } else {
                        distance
                    };
                    color.mask(distance.coverage())
                });
                match mapped {
                    Ok(shader) => self.canvas.rectangle(bounds, Paint::Shader(shader)),
                    Err(error) => {
                        if self.canvas.error.is_none() {
                            self.canvas.error = Some(error);
                        }
                    }
                }
            }
        }
    }
}

fn translate_path(path: &mut Path<Pixels>, offset: Point<Pixels>) {
    path.bounds.origin += offset;
    for vertex in &mut path.vertices {
        vertex.xy_position += offset;
    }
}

fn finite_bounds(bounds: Bounds<Pixels>, scale: f32) -> bool {
    [
        bounds.origin.x.0,
        bounds.origin.y.0,
        bounds.size.width.0,
        bounds.size.height.0,
        bounds.right().0,
        bounds.bottom().0,
    ]
    .into_iter()
    .all(|value| value.is_finite() && (value * scale).is_finite())
}

fn finite_geometry(geometry: &Geometry, offset: Point<Pixels>, scale: f32) -> bool {
    let mut bounds = match geometry {
        Geometry::Rect(bounds, _) => *bounds,
        Geometry::Quad(quad) => quad.bounds,
        Geometry::Path(path, _) => {
            if !path.vertices.iter().all(|vertex| {
                let position = vertex.xy_position + offset;
                position.x.0.is_finite()
                    && position.y.0.is_finite()
                    && (position.x.0 * scale).is_finite()
                    && (position.y.0 * scale).is_finite()
            }) {
                return false;
            }
            path.bounds
        }
    };
    bounds.origin += offset;
    finite_bounds(bounds, scale)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounds() -> Bounds<Pixels> {
        Bounds::new(point(px(10.0), px(20.0)), size(px(80.0), px(40.0)))
    }

    #[test]
    fn shader_shapes_and_borders_each_record_one_primitive() {
        let root = PaintRoot::new();
        let shader = root.shader(|cx| cx.vec4([0.2, 0.4, 0.6, 0.8])).unwrap();
        let drawing = root
            .canvas(|canvas| {
                canvas.rect(bounds()).fill(&shader).stroke(&shader, px(3.0));
                canvas
                    .rounded_rect(bounds(), px(12.0))
                    .fill(&shader)
                    .stroke(&shader, px(4.0));
                canvas
                    .circle(point(px(30.0), px(40.0)), px(20.0))
                    .fill(&shader)
                    .stroke(&shader, px(2.0));
            })
            .unwrap();
        assert_eq!(drawing.primitive_count(), 6);
        for command in drawing.commands.iter() {
            let Geometry::Rect(_, Paint::Shader(shader)) = &command.geometry else {
                panic!("expected shader paint")
            };
            assert!(shader.wgsl().contains("fwidth"));
        }
    }

    #[test]
    fn scopes_transform_clips_and_restore_recording_state() {
        let drawing = PaintRoot::new()
            .canvas(|canvas| {
                canvas.translated(point(px(5.0), px(7.0)), |canvas| {
                    canvas.clipped(bounds(), |canvas| {
                        canvas.opacity(0.5, |canvas| {
                            canvas.rect(bounds()).fill(Hsla::default());
                        });
                    });
                });
                canvas.rect(bounds()).fill(Hsla::default());
            })
            .unwrap();
        let first = &drawing.commands[0];
        let Geometry::Quad(quad) = &first.geometry else {
            panic!("expected native paint")
        };
        assert_eq!(quad.bounds.origin, point(px(15.0), px(27.0)));
        assert_eq!(first.clip.unwrap().origin, quad.bounds.origin);
        assert_eq!(first.opacity, 0.5);
        let last = &drawing.commands[1];
        let Geometry::Quad(quad) = &last.geometry else {
            panic!("expected native paint")
        };
        assert_eq!(quad.bounds, bounds());
        assert!(last.clip.is_none());
        assert_eq!(last.opacity, 1.0);
    }

    #[test]
    fn invalid_shapes_fail_the_whole_drawing() {
        let result = PaintRoot::new().canvas(|canvas| {
            canvas
                .rounded_rect(bounds(), px(f32::NAN))
                .fill(Hsla::default());
        });
        assert!(matches!(result, Err(ShaderError::InvalidGeometry)));
    }

    #[test]
    fn masked_parameter_permutations_reuse_the_same_program() {
        let root = PaintRoot::new();
        let mut parameter = None;
        let shader = root
            .shader(|cx| {
                let color = cx.parameter([1.0, 0.0, 0.0, 1.0]);
                parameter = Some(color);
                cx.uniform(color)
            })
            .unwrap();
        let parameter = parameter.unwrap();
        let mut program = None;
        let mut retained = Vec::new();
        for value in [
            [1.0, 0.0, 0.0, 1.0],
            [0.0, 0.5, 1.0, 0.25],
            [0.7, 0.2, 0.1, 0.0],
        ] {
            let updated = shader.with_parameter(parameter, value).unwrap();
            let drawing = root
                .canvas(|canvas| {
                    canvas
                        .rounded_rect(bounds(), px(9.0))
                        .stroke(updated, px(4.0));
                })
                .unwrap();
            let Geometry::Rect(_, Paint::Shader(masked)) = &drawing.commands[0].geometry else {
                panic!("expected shader paint")
            };
            assert_eq!(masked.parameter_slots()[0], value);
            assert!(
                masked
                    .with_parameter(parameter, [0.1, 0.2, 0.3, 0.4])
                    .is_ok()
            );
            if let Some(program) = program {
                assert_eq!(masked.program_id(), program);
            }
            program = Some(masked.program_id());
            retained.push(drawing);
        }
    }

    #[test]
    fn changing_shape_dimensions_only_updates_geometry_uniforms() {
        let root = PaintRoot::new();
        let shader = root.shader(|cx| cx.vec4([0.2, 0.4, 0.6, 0.8])).unwrap();
        let drawing = root
            .canvas(|canvas| {
                for (width, height, radius, border) in [
                    (80.0, 40.0, 3.0, 1.0),
                    (300.0, 160.0, 40.0, 10.0),
                    (12.0, 12.0, 6.0, 3.0),
                ] {
                    let bounds = Bounds::new(Point::default(), size(px(width), px(height)));
                    canvas
                        .rounded_rect(bounds, px(radius))
                        .stroke(&shader, px(border));
                }
            })
            .unwrap();
        let shaders: Vec<_> = drawing
            .commands
            .iter()
            .map(|command| {
                let Geometry::Rect(_, Paint::Shader(shader)) = &command.geometry else {
                    panic!("expected shader paint")
                };
                shader
            })
            .collect();
        assert!(
            shaders
                .iter()
                .all(|shader| shader.program_id() == shaders[0].program_id())
        );
        assert_eq!(shaders[0].parameter_slots()[0], [40.0, 20.0, 3.0, 1.0]);
        assert_eq!(shaders[1].parameter_slots()[0], [150.0, 80.0, 40.0, 10.0]);
    }

    #[test]
    fn drawing_parameter_update_changes_all_matching_paints_and_shares_paths() {
        let root = PaintRoot::new();
        let tint = root.parameter([0.2, 0.4, 0.8, 1.0]).unwrap();
        let unrelated = root.parameter([0.1, 0.3, 0.5, 0.7]).unwrap();
        let material = root.shader(|cx| cx.uniform(tint)).unwrap();
        let other = root.shader(|cx| cx.uniform(unrelated)).unwrap();
        let drawing = root
            .canvas(|canvas| {
                canvas
                    .rounded_rect(bounds(), px(8.0))
                    .fill(&material)
                    .stroke(&material, px(2.0));
                canvas
                    .circle(point(px(80.0), px(40.0)), px(30.0))
                    .fill(&material);
                canvas.rect(bounds()).fill(other);
                canvas.path(Path::new(Point::default()), crate::transparent_black());
            })
            .unwrap();
        let color = [0.8, 0.3, 0.2, 0.5];
        let updated = drawing.with_parameter(tint, color).unwrap();
        assert_eq!(updated.primitive_count(), drawing.primitive_count());
        for (before, after) in drawing.commands.iter().zip(updated.commands.iter()) {
            match (&before.geometry, &after.geometry) {
                (
                    Geometry::Rect(_, Paint::Shader(before)),
                    Geometry::Rect(_, Paint::Shader(after)),
                ) => {
                    assert_eq!(before.program_id(), after.program_id());
                    if before.has_parameter(tint) {
                        assert_ne!(before.parameter_slots(), after.parameter_slots());
                        assert!(after.parameter_slots().contains(&color));
                    } else {
                        assert_eq!(before.parameter_slots(), after.parameter_slots());
                    }
                }
                (Geometry::Path(before, _), Geometry::Path(after, _)) => {
                    assert!(Arc::ptr_eq(before, after))
                }
                _ => panic!("geometry changed during uniform update"),
            }
        }
        let missing = root.parameter(0.25).unwrap();
        assert!(matches!(
            drawing.with_parameter(missing, 0.5),
            Err(ShaderError::ForeignGraph)
        ));
        assert!(matches!(
            drawing.with_parameter(tint, [f32::NAN; 4]),
            Err(ShaderError::NonFiniteValue)
        ));
    }

    #[test]
    fn negative_dimensions_fail_and_zero_dimensions_skip() {
        let root = PaintRoot::new();
        let invalid = root.canvas(|canvas| {
            let bounds = Bounds::new(Point::default(), size(px(-10.0), px(20.0)));
            canvas.rect(bounds).fill(Hsla::default());
        });
        assert!(matches!(invalid, Err(ShaderError::InvalidGeometry)));
        let empty = root
            .canvas(|canvas| {
                let bounds = Bounds::new(Point::default(), size(px(0.0), px(20.0)));
                canvas.rect(bounds).fill(Hsla::default());
            })
            .unwrap();
        assert_eq!(empty.primitive_count(), 0);
    }

    #[test]
    fn preflight_rejects_scaled_overflow_in_bounds_endpoints_and_vertices() {
        let large = f32::MAX * 0.3;
        let bounds = Bounds::new(point(px(large), px(0.0)), size(px(large), px(1.0)));
        assert!(finite_bounds(bounds, 1.0));
        // Origin and width still fit after scaling, but their summed endpoint does not.
        assert!((large * 2.0).is_finite());
        assert!(!finite_bounds(bounds, 2.0));
        let geometry = Geometry::Rect(bounds, Paint::Background(Background::default()));
        assert!(!finite_geometry(&geometry, Point::default(), 2.0));

        let mut path = Path::new(Point::default());
        path.push_triangle(
            (
                point(px(large), px(0.0)),
                point(px(large * 2.0), px(0.0)),
                point(px(large), px(1.0)),
            ),
            (point(0.0, 1.0), point(0.0, 1.0), point(0.0, 1.0)),
        );
        // The vertex check remains protective even if caller-supplied bounds are incomplete.
        path.bounds = Bounds::new(Point::default(), size(px(1.0), px(1.0)));
        let geometry = Geometry::Path(Arc::new(path), Background::default());
        assert!(finite_geometry(&geometry, Point::default(), 1.0));
        assert!(!finite_geometry(&geometry, Point::default(), 2.0));
    }

    #[test]
    fn preflight_accepts_normal_fractional_scale_and_translation() {
        let geometry = Geometry::Rect(bounds(), Paint::Background(Background::default()));
        let origin = point(px(2.5), px(-3.25));
        assert!(finite_geometry(&geometry, origin, 1.25));
        let mut clip = bounds();
        clip.origin += origin;
        assert!(finite_bounds(clip, 1.25));
    }

    #[test]
    fn repeated_fixed_shape_distances_do_not_allocate_uniforms() {
        let root = PaintRoot::new();
        for shape in [
            Shape::Rect {
                bounds: bounds(),
                radius: px(8.0),
            },
            Shape::Circle {
                center: point(px(30.0), px(40.0)),
                radius: px(20.0),
            },
        ] {
            let shader = root
                .shader(|cx| {
                    let distance = shape.distance(cx);
                    for _ in 0..65 {
                        let repeated = shape.distance(cx);
                        assert_eq!(
                            distance.id, repeated.id,
                            "identical fixed geometry must share its expression"
                        );
                    }
                    cx.rgba(distance.clone(), distance.clone(), distance, cx.scalar(1.0))
                })
                .unwrap();
            assert!(shader.parameter_slots().is_empty());
        }
    }

    #[test]
    fn batched_drawing_updates_are_atomic_and_share_repeated_material_values() {
        let root = PaintRoot::new();
        let time = root.parameter(0.0).unwrap();
        let tint = root.parameter([0.2, 0.4, 0.8, 1.0]).unwrap();
        let unknown = root.parameter(1.0).unwrap();
        let animated = root
            .shader(|cx| {
                let t = cx.uniform(time);
                cx.rgba(t.clone(), t.clone(), t, cx.scalar(1.0))
            })
            .unwrap();
        let tinted = root.shader(|cx| cx.uniform(tint)).unwrap();
        let drawing = root
            .canvas(|canvas| {
                canvas.rect(bounds()).fill(&animated);
                canvas
                    .circle(point(px(30.0), px(40.0)), px(20.0))
                    .fill(&tinted);
            })
            .unwrap();
        // Repeated submissions of the same immutable shader instance share both
        // its logical binding table and its value table, unlike independent masks.
        let drawing = CanvasDrawing {
            commands: vec![
                drawing.commands[0].clone(),
                drawing.commands[0].clone(),
                drawing.commands[1].clone(),
            ]
            .into(),
        };
        let partial = drawing
            .with_parameters(|patch| {
                patch.set(time, 0.5);
            })
            .unwrap();
        let (Geometry::Rect(_, Paint::Shader(before)), Geometry::Rect(_, Paint::Shader(after))) =
            (&drawing.commands[2].geometry, &partial.commands[2].geometry)
        else {
            panic!("expected independent material")
        };
        assert!(before.shares_parameter_values(after));
        let updated = drawing
            .with_parameters(|patch| {
                patch.set(time, 0.75);
                patch.set(tint, [0.8, 0.3, 0.2, 0.5]);
            })
            .unwrap();
        let shaders: Vec<_> = updated
            .commands
            .iter()
            .map(|command| {
                let Geometry::Rect(_, Paint::Shader(shader)) = &command.geometry else {
                    panic!("expected shader paint")
                };
                shader
            })
            .collect();
        assert!(Arc::ptr_eq(&shaders[0].values, &shaders[1].values));
        assert!(
            shaders[0]
                .parameter_slots()
                .contains(&[0.75, 0.0, 0.0, 0.0])
        );
        assert!(shaders[2].parameter_slots().contains(&[0.8, 0.3, 0.2, 0.5]));
        for (before, after) in drawing.commands.iter().zip(updated.commands.iter()) {
            let (Geometry::Rect(_, Paint::Shader(before)), Geometry::Rect(_, Paint::Shader(after))) =
                (&before.geometry, &after.geometry)
            else {
                panic!("expected shader paint")
            };
            assert_eq!(before.program_id(), after.program_id());
            assert_ne!(before.parameter_slots(), after.parameter_slots());
        }
        assert!(matches!(
            drawing.with_parameters(|patch| {
                patch.set(time, 0.5);
                patch.set(unknown, 0.9);
            }),
            Err(ShaderError::ForeignGraph)
        ));
        assert!(matches!(
            drawing.with_parameters(|patch| {
                patch.set(time, 0.5);
                patch.set(tint, [f32::NAN; 4]);
            }),
            Err(ShaderError::NonFiniteValue)
        ));
        let Geometry::Rect(_, Paint::Shader(original)) = &drawing.commands[0].geometry else {
            panic!("expected shader paint")
        };
        assert!(original.parameter_slots().contains(&[0.0; 4]));
    }

    #[test]
    fn empty_or_unchanged_batches_share_the_command_table() {
        let root = PaintRoot::new();
        let time = root.parameter(0.25).unwrap();
        let material = root
            .shader(|cx| {
                let t = cx.uniform(time);
                cx.rgba(t.clone(), t.clone(), t, cx.scalar(1.0))
            })
            .unwrap();
        let drawing = root
            .canvas(|canvas| {
                canvas.rect(bounds()).fill(material);
            })
            .unwrap();
        let empty = drawing.with_parameters(|_| {}).unwrap();
        let unchanged = drawing
            .with_parameters(|patch| {
                patch.set(time, 0.25);
            })
            .unwrap();
        assert!(Arc::ptr_eq(&drawing.commands, &empty.commands));
        assert!(Arc::ptr_eq(&drawing.commands, &unchanged.commands));
    }
}
