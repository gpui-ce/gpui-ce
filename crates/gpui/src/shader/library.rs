use wgsl_rs::std::{Vec2f, Vec3f, Vec4f, vec3f};

use super::{
    Expr, Operand, Scalar, Vec3, constant,
    expr::{Callee, iterate_until},
    value::{Val, sealed::Sealed},
};
use prelude::{Color, Fragment, Noise, Shape};

/// GPUI's shared shader vocabulary.
///
/// This is an ordinary [`wgsl_rs`] module: every function runs on the CPU and
/// transpiles to WGSL. Paint expressions lower onto it, the renderer's glue
/// builds a [`Fragment`] for it, and your own `#[wgsl]` modules can import it
/// with `use gpui::shader::prelude::*;`. Raw WGSL sources see the same
/// declarations.
///
/// Helpers are namespaced by marker types (`Color::over`, `Shape::circle`), so
/// they cannot collide with the renderer's own shader functions.
#[allow(missing_docs, dead_code)]
#[wgsl_rs::wgsl]
pub mod prelude {
    use wgsl_rs::std::*;

    /// Where a paint is being evaluated.
    #[derive(Clone, Copy, Debug, PartialEq, Wgsl)]
    pub struct Fragment {
        /// Normalized coordinate across the painted box: `(0, 0)` is its
        /// top-left corner and `(1, 1)` its bottom-right corner.
        pub uv: Vec2f,
        /// Logical-pixel offset from the painted box's top-left corner.
        pub position: Vec2f,
        /// Logical-pixel size of the painted box.
        pub size: Vec2f,
        /// Device-pixel position of the box's top-left corner in the target.
        pub origin: Vec2f,
        /// Device pixels per logical pixel.
        pub scale: f32,
    }

    impl Fragment {
        /// This fragment re-addressed at another normalized coordinate. The
        /// box itself (size, origin, scale) is unchanged.
        pub fn at(fragment: Fragment, uv: Vec2f) -> Fragment {
            Fragment {
                uv,
                position: uv * fragment.size,
                size: fragment.size,
                origin: fragment.origin,
                scale: fragment.scale,
            }
        }

        /// Device-pixel position of this fragment in the render target.
        pub fn pixel(fragment: Fragment) -> Vec2f {
            fragment.origin + fragment.position * fragment.scale
        }
    }

    /// Premultiplied-alpha color math. Paints compose in premultiplied space.
    pub struct Color {
        tag: u32,
    }

    impl Color {
        /// Convert straight alpha to premultiplied alpha, clamping alpha to `[0, 1]`.
        pub fn premultiply(straight: Vec4f) -> Vec4f {
            let alpha = saturate(straight.w);
            vec4f(
                straight.x * alpha,
                straight.y * alpha,
                straight.z * alpha,
                alpha,
            )
        }

        /// Convert premultiplied alpha back to straight alpha.
        pub fn unpremultiply(color: Vec4f) -> Vec4f {
            if color.w <= 0.0 {
                return vec4f(0.0, 0.0, 0.0, 0.0);
            }
            vec4f(
                color.x / color.w,
                color.y / color.w,
                color.z / color.w,
                color.w,
            )
        }

        /// Source-over composition of premultiplied colors.
        pub fn over(front: Vec4f, back: Vec4f) -> Vec4f {
            front + back * (1.0 - front.w)
        }

        /// Scale a premultiplied color by an amount clamped to `[0, 1]`.
        pub fn fade(color: Vec4f, amount: f32) -> Vec4f {
            color * saturate(amount)
        }
    }

    /// Signed distances (negative inside) and their antialiased coverage.
    pub struct Shape {
        tag: u32,
    }

    impl Shape {
        /// Distance to a circle of `radius` centered at the origin.
        pub fn circle(point: Vec2f, radius: f32) -> f32 {
            length(point) - radius
        }

        /// Distance to a box centered at the origin with uniformly rounded corners.
        pub fn rounded_box(point: Vec2f, half_size: Vec2f, radius: f32) -> f32 {
            let corner = abs(point) - half_size + radius;
            length(max(corner, vec2f(0.0, 0.0))) + min(max(corner.x, corner.y), 0.0) - radius
        }

        /// Distance to an equilateral triangle centered at the origin, pointing
        /// up (towards -y), whose corners lie `radius` from its center.
        pub fn triangle(point: Vec2f, radius: f32) -> f32 {
            let k = 1.7320508;
            let half_side = radius * 0.8660254;
            let p = vec2f(abs(point.x) - half_side, half_side / k - point.y);
            let folded = vec2f(p.x - k * p.y, -k * p.x - p.y) * 0.5;
            let p = select(p, folded, p.x + k * p.y > 0.0);
            let q = vec2f(p.x - clamp(p.x, -2.0 * half_side, 0.0), p.y);
            -length(q) * sign(q.y)
        }

        /// Distance to the segment from `start` to `end`.
        pub fn segment(point: Vec2f, start: Vec2f, end: Vec2f) -> f32 {
            let edge = end - start;
            let offset = point - start;
            let along = saturate(dot(offset, edge) / max(dot(edge, edge), 0.000001));
            length(offset - edge * along)
        }

        /// Coverage of a signed distance antialiased over `width` (for example
        /// `fwidth(distance)`).
        pub fn coverage(distance: f32, width: f32) -> f32 {
            saturate(0.5 - distance / max(width, 0.000001))
        }

        /// The union of two distances, blended over `radius`.
        pub fn smooth_union(a: f32, b: f32, radius: f32) -> f32 {
            let k = max(radius, 0.000001);
            let h = saturate(0.5 + 0.5 * (b - a) / k);
            mix(b, a, h) - k * h * (1.0 - h)
        }
    }

    /// Cheap deterministic noise.
    pub struct Noise {
        tag: u32,
    }

    impl Noise {
        /// A hash of a 2D point in `[0, 1)`.
        pub fn hash(point: Vec2f) -> f32 {
            fract(sin(dot(point, vec2f(127.1, 311.7))) * 43758.547)
        }

        /// Smoothly interpolated value noise in `[0, 1)`.
        pub fn value(point: Vec2f) -> f32 {
            let cell = floor(point);
            let local = fract(point);
            let blend = local * local * (vec2f(3.0, 3.0) - local * 2.0);
            let a = Noise::hash(cell);
            let b = Noise::hash(cell + vec2f(1.0, 0.0));
            let c = Noise::hash(cell + vec2f(0.0, 1.0));
            let d = Noise::hash(cell + vec2f(1.0, 1.0));
            mix(mix(a, b, blend.x), mix(c, d, blend.x), blend.y)
        }
    }
}

/// Typed expression functions for prelude functions. Each runs on the GPU as
/// the transpiled function and on the CPU (for folding and fallbacks) as the
/// Rust function itself.
macro_rules! bind {
    ($($(#[$meta:meta])* $vis:vis fn $name:ident($($arg:ident: $ty:ty),*) -> $ret:ty = $owner:ident::$method:ident;)*) => {$(
        $(#[$meta])*
        $vis fn $name($($arg: impl Operand<Value = $ty>),*) -> Expr<$ret> {
            fn eval(args: &[Val]) -> Val {
                let mut args = args.iter().copied();
                $owner::$method($(<$ty>::from_val(args.next().expect(stringify!($arg)))),*).into_val()
            }
            Expr::call(
                Callee::Prelude(stringify!($owner), stringify!($method)),
                eval,
                [$($arg.into_expr().node),*],
            )
        }
    )*};
}

bind! {
    pub(crate) fn premultiply(straight: Vec4f) -> Vec4f = Color::premultiply;
    pub(crate) fn unpremultiply(color: Vec4f) -> Vec4f = Color::unpremultiply;
    pub(crate) fn over(front: Vec4f, back: Vec4f) -> Vec4f = Color::over;
    pub(crate) fn fade(color: Vec4f, amount: f32) -> Vec4f = Color::fade;
    pub(crate) fn fragment_at(fragment: Fragment, uv: Vec2f) -> Fragment = Fragment::at;
}

/// Signed distance fields: negative inside a shape, positive outside, in the
/// units of the point (usually logical pixels, from
/// [`Pixel::centered`](super::Pixel::centered)).
///
/// Distances compose with ordinary math: `a.min(b)` is a union, `a.max(b)` an
/// intersection, `a.max(-b)` a subtraction, `a.abs() - width` an outline, and
/// `a - radius` a rounding. [`Scalar::coverage`](super::Scalar::coverage)
/// turns a distance into antialiased coverage for [`Paint::mask`](super::Paint::mask).
pub mod shape {
    use super::*;

    bind! {
        /// Distance to a circle of `radius` centered at the origin.
        pub fn circle(point: Vec2f, radius: f32) -> f32 = Shape::circle;
        /// Distance to a box of `half_size` centered at the origin, with corners rounded by `radius`.
        pub fn rounded_box(point: Vec2f, half_size: Vec2f, radius: f32) -> f32 = Shape::rounded_box;
        /// Distance to an equilateral triangle centered at the origin, pointing up, whose corners lie `radius` from its center.
        pub fn triangle(point: Vec2f, radius: f32) -> f32 = Shape::triangle;
        /// Distance to the segment from `start` to `end`.
        pub fn segment(point: Vec2f, start: Vec2f, end: Vec2f) -> f32 = Shape::segment;
        /// The union of two distances, blended over `radius`.
        pub fn smooth_union(a: f32, b: f32, radius: f32) -> f32 = Shape::smooth_union;
        /// Coverage of a signed distance antialiased over `width`.
        pub fn coverage(distance: f32, width: f32) -> f32 = Shape::coverage;
    }

    /// March through a 3D distance field for at most `steps` iterations.
    /// Stops near a surface or beyond `far`, returning the distance travelled.
    /// Exhausting the steps may return a distance below `far` without a hit.
    /// `scene` builds the loop body once on the CPU.
    ///
    /// ```ignore
    /// let scene = |p: &Vec3| p.length() - 1.0;
    /// let t = shape::raymarch(origin, ray, 20.0, 96, scene);
    /// let normal = shape::normal(&(origin + ray * &t), scene);
    /// ```
    pub fn raymarch(
        origin: impl Operand<Value = Vec3f>,
        direction: impl Operand<Value = Vec3f>,
        far: f32,
        steps: u32,
        scene: impl Fn(&Vec3) -> Scalar,
    ) -> Scalar {
        let (origin, direction) = (origin.into_expr(), direction.into_expr());
        let distance = |t: &Scalar| scene(&(&origin + &direction * t));
        iterate_until(
            steps,
            0.0,
            |t| distance(t).lt(t * 0.0005 + 0.0005).or(t.gt(far)),
            |t, _| distance(&t) + t,
        )
    }

    /// The outward unit normal of a 3D distance field at `point`, from four
    /// samples around it.
    pub fn normal(point: &Vec3, scene: impl Fn(&Vec3) -> Scalar) -> Vec3 {
        let corners = [
            vec3f(1.0, -1.0, -1.0),
            vec3f(-1.0, -1.0, 1.0),
            vec3f(-1.0, 1.0, -1.0),
            vec3f(1.0, 1.0, 1.0),
        ];
        let gradient = corners
            .map(|corner| constant(corner) * scene(&(point + constant(corner * 0.001))))
            .into_iter()
            .reduce(|sum, sample| sum + sample)
            .expect("four samples");
        gradient.normalize()
    }
}

/// Deterministic noise using the same formula on the CPU and GPU.
/// Floating point results may differ between devices.
pub mod noise {
    use super::*;

    bind! {
        /// A hash of a 2D point in `[0, 1)`.
        pub fn hash(point: Vec2f) -> f32 = Noise::hash;
        /// Smoothly interpolated value noise in `[0, 1)`, with features one unit apart.
        pub fn value(point: Vec2f) -> f32 = Noise::value;
    }
}
