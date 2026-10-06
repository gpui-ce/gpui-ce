//! WGSL builtins, each paired with its CPU twin from `wgsl_rs::std`.
//!
//! One list drives everything: the GPU call (wgsl-rs renders `inverse_sqrt`
//! as `inverseSqrt`), the CPU implementation used for constant folding, and
//! the typed method on [`Expr`]. Each entry names the method's receiver and
//! parameters, then WGSL's argument order: `smoothstep(x, low, high) => (low,
//! high, x)` is `x.smoothstep(low, high)` in Rust.

use wgsl_rs::std::{self as wgsl, Vec2f, Vec3f, Vec4f};

use super::{
    Expr, Operand, Scalar,
    expr::Callee,
    library::shape,
    value::{Float, Val, Vector, Widen},
};

macro_rules! with_float_builtins {
    ($callback:ident $($prefix:tt)*) => {
        $callback! {
            $($prefix)*
            unary: sin cos tan asin acos atan sinh cosh tanh abs sign floor ceil round fract trunc
                sqrt inverse_sqrt exp exp2 log log2 saturate radians degrees;
            calls:
                /// WGSL `min`, component-wise.
                min(x, y) => (x, y);
                /// WGSL `max`, component-wise.
                max(x, y) => (x, y);
                /// WGSL `pow`, component-wise.
                pow(x, y) => (x, y);
                /// The angle of the point `(x, self)`, component-wise, like `f32::atan2`.
                atan2(y, x) => (y, x);
                /// WGSL `clamp`, component-wise.
                clamp(x, low, high) => (x, low, high);
                /// Linear interpolation towards `y`, following WGSL `mix`. The
                /// amount is not clamped: values outside `[0, 1]` extrapolate.
                mix(x, y, amount) => (x, y, amount);
                /// Hermite interpolation of this value between `low` and `high`.
                smoothstep(x, low, high) => (low, high, x);
                /// `1.0` where this value is at least `edge`, else `0.0`.
                step(x, edge) => (edge, x);
        }
    };
}

macro_rules! float_ops {
    (
        unary: $($unary:ident)*;
        calls: $($(#[$doc:meta])* $call:ident($recv:ident $(, $arg:ident)*) => ($($wgsl:ident),*);)*
    ) => {
        /// CPU twins of the component-wise float builtins, in WGSL argument order.
        pub trait FloatOps: Sized {
            $(fn $unary(x: Self) -> Self;)*
            $(fn $call($($wgsl: Self),*) -> Self;)*
        }

        mod eval {
            use super::*;

            $(pub fn $unary<T: Float>(args: &[Val]) -> Val {
                T::$unary(T::from_val(args[0])).into_val()
            })*
            $(pub fn $call<T: Float>(args: &[Val]) -> Val {
                let mut args = args.iter().copied();
                $(let $wgsl = T::from_val(args.next().expect(stringify!($wgsl)));)*
                T::$call($($wgsl),*).into_val()
            })*
        }

        impl<T: Float> Expr<T> {
            $(
                #[doc = concat!("WGSL `", stringify!($unary), "`, component-wise.")]
                pub fn $unary(&self) -> Self {
                    Self::call(
                        Callee::Builtin(stringify!($unary)),
                        eval::$unary::<T>,
                        [self.node.clone()],
                    )
                }
            )*
            $(
                $(#[$doc])*
                ///
                /// Scalar arguments splat across vectors.
                pub fn $call(&self, $($arg: impl Operand<Value: Widen<T>>),*) -> Self {
                    let $recv = self.node.clone();
                    $(let $arg = Self::widen($arg).node;)*
                    Self::call(Callee::Builtin(stringify!($call)), eval::$call::<T>, [$($wgsl),*])
                }
            )*

            /// Screen-space derivative along x.
            pub fn dpdx(&self) -> Self {
                self.derivative("dpdx")
            }

            /// Screen-space derivative along y.
            pub fn dpdy(&self) -> Self {
                self.derivative("dpdy")
            }

            /// Sum of absolute screen-space derivatives.
            pub fn fwidth(&self) -> Self {
                self.derivative("fwidth")
            }

            /// A derivative. Values evaluated on the CPU are the same across
            /// fragments (uniforms) or sampled at a single point (fallbacks),
            /// so their derivatives are zero.
            fn derivative(&self, name: &'static str) -> Self {
                fn zero(args: &[Val]) -> Val {
                    Val::from_lanes(args[0].ty(), [0.0; 4])
                }
                Self::call(Callee::Builtin(name), zero, [self.node.clone()])
            }
        }
    };
}

with_float_builtins!(float_ops);

macro_rules! float_ops_impl {
    (
        $ty:ty;
        unary: $($unary:ident)*;
        calls: $($(#[$doc:meta])* $call:ident($recv:ident $(, $arg:ident)*) => ($($wgsl:ident),*);)*
    ) => {
        impl FloatOps for $ty {
            $(fn $unary(x: Self) -> Self { wgsl::$unary(x) })*
            $(fn $call($($wgsl: Self),*) -> Self { wgsl::$call($($wgsl),*) })*
        }
    };
}

with_float_builtins!(float_ops_impl f32;);
with_float_builtins!(float_ops_impl Vec2f;);
with_float_builtins!(float_ops_impl Vec3f;);
with_float_builtins!(float_ops_impl Vec4f;);

fn eval_length<T: Vector + wgsl::NumericBuiltinLength>(args: &[Val]) -> Val {
    Val::F32(wgsl::length(T::from_val(args[0])))
}

fn eval_distance<T: Vector + wgsl::NumericBuiltinDistance>(args: &[Val]) -> Val {
    Val::F32(wgsl::distance(T::from_val(args[0]), T::from_val(args[1])))
}

fn eval_dot<T: Vector + wgsl::NumericBuiltinDot<Scalar = f32>>(args: &[Val]) -> Val {
    Val::F32(wgsl::dot(T::from_val(args[0]), T::from_val(args[1])))
}

fn eval_normalize<T: Vector + wgsl::NumericBuiltinNormalize>(args: &[Val]) -> Val {
    wgsl::normalize(T::from_val(args[0])).into_val()
}

impl<T> Expr<T>
where
    T: Vector
        + wgsl::NumericBuiltinLength
        + wgsl::NumericBuiltinDistance
        + wgsl::NumericBuiltinDot<Scalar = f32>
        + wgsl::NumericBuiltinNormalize,
{
    /// Euclidean length.
    pub fn length(&self) -> Scalar {
        Expr::call(
            Callee::Builtin("length"),
            eval_length::<T>,
            [self.node.clone()],
        )
    }

    /// Euclidean distance to `other`.
    pub fn distance(&self, other: impl Operand<Value = T>) -> Scalar {
        Expr::call(
            Callee::Builtin("distance"),
            eval_distance::<T>,
            [self.node.clone(), other.into_expr().node],
        )
    }

    /// Dot product.
    pub fn dot(&self, other: impl Operand<Value = T>) -> Scalar {
        Expr::call(
            Callee::Builtin("dot"),
            eval_dot::<T>,
            [self.node.clone(), other.into_expr().node],
        )
    }

    /// This vector scaled to unit length.
    pub fn normalize(&self) -> Self {
        Self::call(
            Callee::Builtin("normalize"),
            eval_normalize::<T>,
            [self.node.clone()],
        )
    }
}

impl Scalar {
    /// The antialiased coverage of this signed distance (negative inside):
    /// `1` inside, `0` outside, blending across one device pixel. See
    /// [`shape`](super::shape).
    pub fn coverage(&self) -> Scalar {
        shape::coverage(self, self.fwidth())
    }
}
