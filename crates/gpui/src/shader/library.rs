use wgsl_rs::std::{Vec2f, Vec4f};

use super::{
    Expr, Operand,
    expr::Callee,
    value::{Val, sealed::Sealed},
};
use prelude::{Color, Fragment, Shape};

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
        /// Coverage of a signed distance antialiased over `width` (for example
        /// `fwidth(distance)`).
        pub fn coverage(distance: f32, width: f32) -> f32 {
            saturate(0.5 - distance / max(width, 0.000001))
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

/// Antialiased coverage of signed distances (negative inside).
pub mod shape {
    use super::*;

    bind! {
        /// Coverage of a signed distance antialiased over `width`.
        pub fn coverage(distance: f32, width: f32) -> f32 = Shape::coverage;
    }
}
