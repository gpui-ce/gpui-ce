//! Paints: premultiplied colors that vary per fragment.

use std::{
    fmt,
    sync::{Arc, OnceLock},
};

use wgsl_rs::std::{Vec2f, Vec4f};

use palette::IntoColor;

use crate::{Background, Hsla, Pixels, Rgba, Size, hsla_to_rgba, rgb_to_hsla, solid_background};

use super::{
    Expr, Operand, Scalar, Vec2, Vec3, Vec4,
    compile::{CompiledPaint, ShaderError, compile, evaluate},
    expr::{Node, Op},
    library::{fade, over, premultiply, unpremultiply},
    prelude::{Color, Fragment},
    value::{Val, sealed::Sealed},
};

/// A premultiplied fragment color built from typed shader expressions.
///
/// Immutable expressions built with ordinary Rust helpers and control flow.
/// Compilation caches one fragment program per expression structure; changing
/// uniform values reuses that program.
///
/// ```ignore
/// fn waves(phase: f32) -> Paint {
///     paint(|px| {
///         let wave = (px.uv().x() * 12.0 + phase).sin() * 0.5 + 0.5;
///         color(rgb(0x315bff)).mix(color(rgb(0xf48bcb)), wave)
///     })
/// }
///
/// let compiled = waves(phase).compile()?;
/// ```
#[derive(Clone)]
pub struct Paint {
    pub(crate) node: Arc<Node>,
    compiled: Arc<OnceLock<Result<CompiledPaint, ShaderError>>>,
    fallback: Option<Background>,
}

impl Paint {
    pub(crate) fn from_node(node: Arc<Node>) -> Self {
        debug_assert_eq!(node.ty, Vec4f::TY);
        Self {
            node,
            compiled: Arc::default(),
            fallback: None,
        }
    }

    /// A paint from premultiplied RGBA in GPUI's sRGB channel convention.
    pub fn premultiplied(rgba: impl Operand<Value = Vec4f>) -> Self {
        Self::from_node(rgba.into_expr().node)
    }

    /// A paint from straight-alpha RGBA. Alpha is clamped to `[0, 1]`.
    pub fn straight(rgba: impl Operand<Value = Vec4f>) -> Self {
        Self::premultiplied(premultiply(rgba))
    }

    /// The premultiplied RGBA value. A paint is also an operand standing for
    /// this value, so paints pass straight to `select` and foreign functions.
    pub fn rgba(&self) -> Vec4 {
        Expr::from_node(self.node.clone())
    }

    /// The straight-alpha RGBA value.
    pub fn straight_rgba(&self) -> Vec4 {
        unpremultiply(self)
    }

    /// Straight-alpha red.
    pub fn red(&self) -> Scalar {
        self.straight_rgba().x()
    }

    /// Straight-alpha green.
    pub fn green(&self) -> Scalar {
        self.straight_rgba().y()
    }

    /// Straight-alpha blue.
    pub fn blue(&self) -> Scalar {
        self.straight_rgba().z()
    }

    /// Straight-alpha RGB.
    pub fn rgb(&self) -> Vec3 {
        self.straight_rgba().xyz()
    }

    /// Alpha.
    pub fn alpha(&self) -> Scalar {
        self.rgba().w()
    }

    /// Composite this paint over `back` (premultiplied source-over).
    pub fn over(&self, back: impl Into<Paint>) -> Paint {
        Paint::premultiplied(over(self, back.into()))
    }

    /// Interpolate towards `other` in premultiplied space. The amount is not
    /// clamped, following WGSL `mix`.
    pub fn mix(&self, other: impl Into<Paint>, amount: impl Operand<Value = f32>) -> Paint {
        Paint::premultiplied(self.rgba().mix(other.into(), amount))
    }

    /// This paint where `condition` holds, and `other` elsewhere, per fragment.
    pub fn select(&self, condition: impl Operand<Value = bool>, other: impl Into<Paint>) -> Paint {
        Paint::premultiplied(condition.into_expr().select(self, other.into()))
    }

    /// Scale color and alpha by an opacity clamped to `[0, 1]`.
    pub fn opacity(&self, amount: impl Operand<Value = f32>) -> Paint {
        Paint::premultiplied(fade(self, amount))
    }

    /// Restrict this paint to a coverage clamped to `[0, 1]`, such as a
    /// shape's antialiased edge. Equivalent to [`Paint::opacity`].
    pub fn mask(&self, coverage: impl Operand<Value = f32>) -> Paint {
        self.opacity(coverage)
    }

    /// Restrict this paint to the inside of a signed distance field, with an
    /// antialiased edge: `self.mask(distance.coverage())`. See
    /// [`shape`](super::shape).
    pub fn clip(&self, distance: impl Operand<Value = f32>) -> Paint {
        self.mask(distance.into_expr().coverage())
    }

    /// Evaluate this paint at another fragment.
    pub fn at_fragment(&self, fragment: impl Operand<Value = Fragment>) -> Paint {
        Paint::premultiplied(Expr::<Vec4f>::make(
            Op::Apply,
            [self.node.clone(), fragment.into_expr().node],
        ))
    }

    /// Evaluate this paint at another normalized coordinate of the same box.
    pub fn at(&self, uv: impl Operand<Value = Vec2f>) -> Paint {
        self.at_fragment(Pixel.fragment().at(uv))
    }

    /// Re-address this paint through a mapping of normalized coordinates.
    ///
    /// Mapping changes where the paint is sampled, never the element's
    /// geometry, hit testing, or clipping. Mapping a composite maps all of its
    /// parts; mapping one part before composing affects only that part.
    pub fn map_uv(&self, map: impl FnOnce(Vec2) -> Vec2) -> Paint {
        self.at(map(Pixel.uv()))
    }

    /// The background painted in place of this paint by renderers that
    /// cannot run shaders. By default GPUI evaluates the paint at the center
    /// of the box on the CPU, when it can.
    pub fn fallback(mut self, background: impl Into<Background>) -> Self {
        self.fallback = Some(background.into());
        self
    }

    /// The fallback for a box of `size` logical pixels.
    pub(crate) fn fallback_background(&self, size: Size<Pixels>) -> Background {
        if let Some(fallback) = self.fallback {
            return fallback;
        }
        let size = Vec2f {
            x: f32::from(size.width),
            y: f32::from(size.height),
        };
        let center = Fragment {
            uv: Vec2f { x: 0.5, y: 0.5 },
            position: size * 0.5,
            size,
            origin: Vec2f { x: 0.0, y: 0.0 },
            scale: 1.0,
        };
        let Some(rgba) = self.evaluate(center) else {
            return crate::transparent_black().into();
        };
        let straight = Color::unpremultiply(rgba);
        solid_background(rgb_to_hsla(Rgba::new(
            straight.x, straight.y, straight.z, straight.w,
        )))
    }

    /// Evaluate this paint on the CPU, returning premultiplied RGBA.
    /// Returns `None` for excessive depth, backdrop reads, or foreign functions
    /// without a CPU implementation. Derivatives evaluate to zero, so edges
    /// antialiased with [`Scalar::coverage`] are hard on the CPU.
    pub fn evaluate(&self, fragment: Fragment) -> Option<Vec4f> {
        match evaluate(&self.node, Some(fragment))? {
            Val::Vec4(rgba) => Some(rgba),
            _ => None,
        }
    }

    /// Compile and validate this paint. Clones share the result.
    pub fn compile(&self) -> Result<CompiledPaint, ShaderError> {
        self.compiled.get_or_init(|| compile(&self.node)).clone()
    }
}

impl fmt::Debug for Paint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Paint").finish_non_exhaustive()
    }
}

impl PartialEq for Paint {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.node, &other.node)
            || matches!((self.compile(), other.compile()), (Ok(a), Ok(b)) if a == b)
    }
}

impl From<&Paint> for Paint {
    fn from(paint: &Paint) -> Self {
        paint.clone()
    }
}

impl Operand for Paint {
    type Value = Vec4f;
    fn into_expr(self) -> Vec4 {
        Expr::from_node(self.node)
    }
}

impl Operand for &Paint {
    type Value = Vec4f;
    fn into_expr(self) -> Vec4 {
        self.rgba()
    }
}

impl<C: IntoColor<Hsla>> From<C> for Paint {
    fn from(color: C) -> Self {
        let rgba = hsla_to_rgba(color.into_color());
        color_value([
            rgba.color.red,
            rgba.color.green,
            rgba.color.blue,
            rgba.alpha,
        ])
    }
}

/// A uniform color paint. Accepts any GPUI color.
pub fn color(color: impl Into<Paint>) -> Paint {
    color.into()
}

/// A paint from straight-alpha channels, which may vary per fragment.
pub fn rgba(
    red: impl Operand<Value = f32>,
    green: impl Operand<Value = f32>,
    blue: impl Operand<Value = f32>,
    alpha: impl Operand<Value = f32>,
) -> Paint {
    Paint::straight(super::vec4(red, green, blue, alpha))
}

fn color_value([r, g, b, a]: [f32; 4]) -> Paint {
    let a = if a.is_finite() { a.clamp(0., 1.) } else { a };
    Paint::premultiplied(Vec4f {
        x: r * a,
        y: g * a,
        z: b * a,
        w: a,
    })
}

/// Sample the scene already painted behind the box at a normalized
/// coordinate of the box, for glass and refraction.
///
/// Samples see everything painted earlier, including outside the box, and
/// nothing painted later. Each backdrop draw interrupts the render pass and
/// updates a target snapshot; combining effects in one paint reduces that work.
pub fn backdrop(uv: impl Operand<Value = Vec2f>) -> Paint {
    Paint::premultiplied(Expr::<Vec4f>::make(
        Op::Backdrop,
        [Node::fragment(), uv.into_expr().node],
    ))
}

/// The fragment a paint is evaluated at. Zero-sized and `Copy`: pass it to
/// helpers freely.
#[derive(Clone, Copy, Debug, Default)]
pub struct Pixel;

impl Pixel {
    /// The fragment itself, for foreign shader functions.
    pub fn fragment(self) -> Expr<Fragment> {
        Expr::from_node(Node::fragment())
    }

    /// Normalized coordinate: `(0, 0)` at the box's top-left, `(1, 1)` at its
    /// bottom-right.
    pub fn uv(self) -> Vec2 {
        self.fragment().uv()
    }

    /// Logical-pixel offset from the box's top-left corner.
    pub fn position(self) -> Vec2 {
        self.fragment().position()
    }

    /// Logical-pixel size of the box.
    pub fn size(self) -> Vec2 {
        self.fragment().size()
    }

    /// Logical-pixel offset from the box's center: the natural frame for
    /// [`shape`](super::shape) distances.
    pub fn centered(self) -> Vec2 {
        self.fragment().centered()
    }

    /// Device pixels per logical pixel.
    pub fn scale(self) -> Scalar {
        self.fragment().scale()
    }

    /// The scene behind the box, sampled at a normalized coordinate.
    pub fn backdrop(self, uv: impl Operand<Value = Vec2f>) -> Paint {
        backdrop(uv)
    }
}

/// Build a paint from a closure over the fragment. The closure runs once, on
/// the CPU, to describe the paint; it is not transpiled.
pub fn paint<P: Into<Paint>>(build: impl FnOnce(Pixel) -> P) -> Paint {
    build(Pixel).into()
}
