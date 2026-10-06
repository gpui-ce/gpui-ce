//! Shader value types and their CPU representations.
//!
//! Folding and fallback evaluation use `wgsl_rs::std` values and functions.

use std::ops::{Add, Div, Mul, Sub};

use wgsl_rs::{
    ir,
    std::{Vec2f, Vec3f, Vec4f},
};

use super::{Expr, prelude::Fragment};

/// The WGSL type of an expression node.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[doc(hidden)]
pub enum Ty {
    F32,
    Bool,
    Vec2,
    Vec3,
    Vec4,
    Fragment,
}

impl Ty {
    /// Number of `f32` lanes a parameter of this type occupies.
    pub(crate) fn lanes(self) -> usize {
        match self {
            Self::F32 | Self::Bool => 1,
            Self::Vec2 => 2,
            Self::Vec3 => 3,
            Self::Vec4 => 4,
            Self::Fragment => unreachable!("fragments are never parameters"),
        }
    }

    /// The WGSL spelling.
    pub(crate) fn wgsl(self) -> &'static str {
        match self {
            Self::F32 => "f32",
            Self::Bool => "bool",
            Self::Vec2 => "vec2<f32>",
            Self::Vec3 => "vec3<f32>",
            Self::Vec4 => "vec4<f32>",
            Self::Fragment => "Fragment",
        }
    }

    pub(crate) fn ir(self) -> ir::Type {
        let vector = |elements| ir::Type::Vector {
            elements,
            scalar_ty: Some(ir::ScalarType::F32),
        };
        match self {
            Self::F32 => ir::Type::Scalar(ir::ScalarType::F32),
            Self::Bool => ir::Type::Scalar(ir::ScalarType::Bool),
            Self::Vec2 => vector(2),
            Self::Vec3 => vector(3),
            Self::Vec4 => vector(4),
            Self::Fragment => ir::Type::Struct {
                name: "Fragment".into(),
                type_args: vec![],
            },
        }
    }

    /// Map a Naga type, recognizing the namespaced `Fragment` by its name.
    pub(crate) fn from_naga(
        module: &naga::Module,
        ty: naga::Handle<naga::Type>,
        fragment: &str,
    ) -> Option<Self> {
        use naga::{ScalarKind, TypeInner, VectorSize};
        match &module.types[ty].inner {
            TypeInner::Scalar(scalar) => match scalar.kind {
                ScalarKind::Float if scalar.width == 4 => Some(Self::F32),
                ScalarKind::Bool => Some(Self::Bool),
                _ => None,
            },
            TypeInner::Vector { size, scalar }
                if scalar.kind == ScalarKind::Float && scalar.width == 4 =>
            {
                Some(match size {
                    VectorSize::Bi => Self::Vec2,
                    VectorSize::Tri => Self::Vec3,
                    VectorSize::Quad => Self::Vec4,
                })
            }
            TypeInner::Struct { .. } => module.types[ty]
                .name
                .as_deref()
                .is_some_and(|name| name == fragment)
                .then_some(Self::Fragment),
            _ => None,
        }
    }
}

/// A value on the CPU: the domain of constant folding and fallback evaluation.
#[derive(Clone, Copy, Debug, PartialEq)]
#[doc(hidden)]
pub enum Val {
    F32(f32),
    Bool(bool),
    Vec2(Vec2f),
    Vec3(Vec3f),
    Vec4(Vec4f),
    Fragment(Fragment),
}

impl Val {
    pub(crate) fn ty(self) -> Ty {
        match self {
            Self::F32(_) => Ty::F32,
            Self::Bool(_) => Ty::Bool,
            Self::Vec2(_) => Ty::Vec2,
            Self::Vec3(_) => Ty::Vec3,
            Self::Vec4(_) => Ty::Vec4,
            Self::Fragment(_) => Ty::Fragment,
        }
    }

    /// The value of type `ty` in left-aligned `lanes`; the inverse of [`Val::lanes`].
    pub(crate) fn from_lanes(ty: Ty, [x, y, z, w]: [f32; 4]) -> Self {
        match ty {
            Ty::F32 => Self::F32(x),
            Ty::Bool => Self::Bool(x != 0.0),
            Ty::Vec2 => Self::Vec2(Vec2f { x, y }),
            Ty::Vec3 => Self::Vec3(Vec3f { x, y, z }),
            Ty::Vec4 => Self::Vec4(Vec4f { x, y, z, w }),
            Ty::Fragment => unreachable!("fragments are never parameters"),
        }
    }

    /// Parameter lanes, left-aligned. Booleans travel as `0.0` or `1.0`.
    pub(crate) fn lanes(self) -> [f32; 4] {
        match self {
            Self::F32(x) => [x, 0., 0., 0.],
            Self::Bool(b) => [f32::from(u8::from(b)), 0., 0., 0.],
            Self::Vec2(v) => [v.x, v.y, 0., 0.],
            Self::Vec3(v) => [v.x, v.y, v.z, 0.],
            Self::Vec4(v) => [v.x, v.y, v.z, v.w],
            Self::Fragment(_) => unreachable!("fragments are never parameters"),
        }
    }

    pub(crate) fn is_finite(self) -> bool {
        self.lanes().iter().all(|lane| lane.is_finite())
    }

    /// Bit pattern used to key embedded constants.
    pub(crate) fn bits(self) -> [u32; 4] {
        self.lanes().map(f32::to_bits)
    }

    /// The WGSL literal for an embedded constant.
    pub(crate) fn literal(self) -> ir::Expr {
        let float = |value: f32| {
            let mut text = format!("{value:?}");
            if !text.contains(['.', 'e', 'E']) {
                text.push_str(".0");
            }
            ir::Expr::Lit(ir::Lit::Float { text })
        };
        let vector = |name: &str, lanes: &[f32]| ir::Expr::FnCall {
            path: ir::FnPath::Ident(name.into()),
            type_args: vec![],
            params: lanes.iter().copied().map(float).collect(),
        };
        match self {
            Self::F32(x) => float(x),
            Self::Bool(b) => ir::Expr::Lit(ir::Lit::Bool(b)),
            Self::Vec2(v) => vector("vec2f", &[v.x, v.y]),
            Self::Vec3(v) => vector("vec3f", &[v.x, v.y, v.z]),
            Self::Vec4(v) => vector("vec4f", &[v.x, v.y, v.z, v.w]),
            Self::Fragment(_) => unreachable!("fragments are never constants"),
        }
    }
}

pub(crate) mod sealed {
    use super::{Ty, Val};

    pub trait Sealed: Copy + Send + Sync + 'static {
        const TY: Ty;
        fn into_val(self) -> Val;
        fn from_val(value: Val) -> Self;
    }
}

/// A type that can flow through shader expressions: `f32`, `bool`, the
/// `wgsl_rs` float vectors, and the [`Fragment`] being shaded.
pub trait Value: sealed::Sealed {}

macro_rules! values {
    ($($rust:ty => $variant:ident),* $(,)?) => {$(
        impl sealed::Sealed for $rust {
            const TY: Ty = Ty::$variant;
            fn into_val(self) -> Val {
                Val::$variant(self)
            }
            fn from_val(value: Val) -> Self {
                match value {
                    Val::$variant(value) => value,
                    other => unreachable!("expected {:?}, found {other:?}", Ty::$variant),
                }
            }
        }
        impl Value for $rust {}
    )*};
}

values!(f32 => F32, bool => Bool, Vec2f => Vec2, Vec3f => Vec3, Vec4f => Vec4, Fragment => Fragment);

/// A plain CPU value of a shader type: `f32`, `bool`, a `wgsl_rs` float
/// vector, or an `[f32; N]` array standing in for one.
pub trait CpuValue: Copy {
    /// The shader type.
    type Value: Value;
    /// Convert to the shader type's CPU form.
    fn into_value(self) -> Self::Value;
}

macro_rules! cpu_values {
    ($($rust:ty => $value:ty, $convert:expr;)*) => {$(
        impl CpuValue for $rust {
            type Value = $value;
            fn into_value(self) -> $value {
                #[allow(clippy::redundant_closure_call)]
                ($convert)(self)
            }
        }
    )*};
}

cpu_values! {
    f32 => f32, |x| x;
    bool => bool, |x| x;
    Vec2f => Vec2f, |x| x;
    Vec3f => Vec3f, |x| x;
    Vec4f => Vec4f, |x| x;
    [f32; 2] => Vec2f, |[x, y]: [f32; 2]| Vec2f { x, y };
    [f32; 3] => Vec3f, |[x, y, z]: [f32; 3]| Vec3f { x, y, z };
    [f32; 4] => Vec4f, |[x, y, z, w]: [f32; 4]| Vec4f { x, y, z, w };
}

/// Something usable as a shader operand: an [`Expr`], a borrowed one, or a
/// [`CpuValue`], which becomes a uniform parameter.
pub trait Operand {
    /// The operand's shader value type.
    type Value: Value;
    /// Convert into an expression.
    fn into_expr(self) -> Expr<Self::Value>;
}

impl<T: Value> Operand for Expr<T> {
    type Value = T;
    fn into_expr(self) -> Expr<T> {
        self
    }
}

impl<T: Value> Operand for &Expr<T> {
    type Value = T;
    fn into_expr(self) -> Expr<T> {
        self.clone()
    }
}

impl<C: CpuValue> Operand for C {
    type Value = C::Value;
    fn into_expr(self) -> Expr<C::Value> {
        Expr::uniform(self.into_value())
    }
}

/// Numeric types closed under `+ - * /` with `Rhs`, following WGSL's
/// scalar-vector broadcasting.
pub trait Arith<Rhs: Value>: Value {
    /// The result type.
    type Output: Value;
    #[doc(hidden)]
    fn apply<const OP: u8>(self, rhs: Rhs) -> Self::Output;
}

pub(crate) const ADD: u8 = 0;
pub(crate) const SUB: u8 = 1;
pub(crate) const MUL: u8 = 2;
pub(crate) const DIV: u8 = 3;

pub(crate) const fn bin_op(op: u8) -> ir::BinOp {
    match op {
        ADD => ir::BinOp::Add,
        SUB => ir::BinOp::Sub,
        MUL => ir::BinOp::Mul,
        _ => ir::BinOp::Div,
    }
}

fn arithmetic<const OP: u8, L, R, O>(lhs: L, rhs: R) -> O
where
    L: Add<R, Output = O> + Sub<R, Output = O> + Mul<R, Output = O> + Div<R, Output = O>,
{
    match OP {
        ADD => lhs + rhs,
        SUB => lhs - rhs,
        MUL => lhs * rhs,
        _ => lhs / rhs,
    }
}

macro_rules! arith {
    ($($lhs:ty, $rhs:ty => $out:ty;)*) => {$(
        impl Arith<$rhs> for $lhs {
            type Output = $out;
            fn apply<const OP: u8>(self, rhs: $rhs) -> $out {
                arithmetic::<OP, _, _, _>(self, rhs)
            }
        }
    )*};
}

arith! {
    f32, f32 => f32;
    Vec2f, Vec2f => Vec2f; Vec2f, f32 => Vec2f; f32, Vec2f => Vec2f;
    Vec3f, Vec3f => Vec3f; Vec3f, f32 => Vec3f; f32, Vec3f => Vec3f;
    Vec4f, Vec4f => Vec4f; Vec4f, f32 => Vec4f; f32, Vec4f => Vec4f;
}

/// Operand types that widen to `T`: `T` itself, or an `f32` splatted across
/// a vector. Builtins accept them wherever WGSL would need a `vecN(x)`, so
/// `uv.clamp(0.0, 1.0)` reads as it would on the CPU.
pub trait Widen<T: Value>: Value {}

impl<T: Value> Widen<T> for T {}
impl Widen<Vec2f> for f32 {}
impl Widen<Vec3f> for f32 {}
impl Widen<Vec4f> for f32 {}

/// Float scalars and vectors: every component-wise WGSL builtin applies.
pub trait Float:
    Arith<Self, Output = Self> + Arith<f32, Output = Self> + super::builtins::FloatOps
{
}

/// Float vectors.
pub trait Vector: Float {}

impl Float for f32 {}
impl Float for Vec2f {}
impl Float for Vec3f {}
impl Float for Vec4f {}
impl Vector for Vec2f {}
impl Vector for Vec3f {}
impl Vector for Vec4f {}
