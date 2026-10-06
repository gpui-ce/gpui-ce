//! Typed shader expressions over a shared, immutable DAG.

use std::{
    cell::Cell,
    fmt,
    marker::PhantomData,
    ops::{Add, Div, Mul, Neg, Sub},
    sync::{Arc, LazyLock},
};

use smallvec::SmallVec;
use wgsl_rs::{
    ir,
    std::{Vec2f, Vec3f, Vec4f},
};

use super::{
    library::fragment_at,
    prelude::Fragment,
    value::{
        ADD, Arith, DIV, Float, MUL, Operand, SUB, Ty, Val, Value, Vector, Widen, bin_op,
        sealed::Sealed,
    },
};

/// How often a node's value can change, and therefore where it runs.
///
/// Constant and uniform subtrees with CPU semantics are folded on the CPU
/// before any shader code is generated; only fragment-rate work reaches the GPU.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum Rate {
    Constant,
    Uniform,
    Fragment,
}

/// CPU semantics of an operation.
pub(crate) type Eval = fn(&[Val]) -> Val;

/// A function with a CPU twin, called by name.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum Callee {
    /// A WGSL builtin, by its `wgsl_rs` name (`inverse_sqrt` renders as `inverseSqrt`).
    Builtin(&'static str),
    /// A [prelude](super::prelude) function: `Color::over` renders as `Color_over`.
    Prelude(&'static str, &'static str),
}

impl Callee {
    pub(crate) fn path(self) -> ir::FnPath {
        match self {
            Self::Builtin(name) => ir::FnPath::Ident(name.into()),
            Self::Prelude(ty, method) => ir::FnPath::TypeMethod {
                ty: ty.into(),
                method: method.into(),
            },
        }
    }
}

pub(crate) enum Op {
    /// A CPU value, uploaded as a uniform parameter.
    Uniform(Val),
    /// A value embedded in the generated code.
    Constant(Val),
    /// The fragment being shaded. There is exactly one such node.
    Fragment,
    /// A vector swizzle or struct member.
    Member(&'static str),
    Unary(ir::UnOp, Eval),
    Binary(ir::BinOp, Eval),
    /// A builtin or prelude function, and its CPU twin.
    Call(Callee, Eval),
    /// `vecN(args..)` for the node type; one argument splats.
    Construct,
    /// WGSL `select(if_false, if_true, condition)`.
    Select,
    /// Evaluate `args[0]` at the fragment `args[1]`.
    Apply,
    /// Sample the scene behind the painted box at `args[1]` (a UV in the
    /// fragment `args[0]`).
    Backdrop,
    /// The state of the [`iterate`] loop at a nesting level.
    LoopState(u8),
    /// The iteration index, as `f32`, of the loop at a nesting level.
    LoopIndex(u8),
    /// `count` iterations of the loop at `level`. Arguments: the initial
    /// state, the state and index leaves, the next state, and optionally a
    /// condition, checked before each step, that ends the loop early.
    Loop {
        count: u32,
        level: u8,
    },
    /// An expression that cannot compile, carrying its diagnostic.
    Invalid(Arc<str>),
}

#[doc(hidden)]
pub struct Node {
    pub(crate) op: Op,
    pub(crate) args: SmallVec<[Arc<Node>; 3]>,
    pub(crate) ty: Ty,
    pub(crate) rate: Rate,
    /// Whether this whole subtree has CPU semantics, given a fragment.
    pub(crate) cpu: bool,
    /// Longest path to a leaf, bounding every recursive pass.
    pub(crate) depth: u32,
    /// Levels of the enclosing loops whose state or index this reads, one bit
    /// each. Such nodes change every iteration, so they never fold.
    pub(crate) open: u32,
}

impl Node {
    pub(crate) fn new(op: Op, ty: Ty, args: SmallVec<[Arc<Node>; 3]>) -> Arc<Self> {
        let (own_rate, own_cpu) = match &op {
            Op::Uniform(_) => (Rate::Uniform, true),
            Op::Constant(_) => (Rate::Constant, true),
            Op::Fragment => (Rate::Fragment, true),
            Op::Backdrop => (Rate::Fragment, false),
            Op::Invalid(_) => (Rate::Fragment, false),
            Op::Member(_)
            | Op::Unary(..)
            | Op::Binary(..)
            | Op::Call(..)
            | Op::Construct
            | Op::Select
            | Op::Apply
            | Op::LoopState(_)
            | Op::LoopIndex(_)
            | Op::Loop { .. } => (Rate::Constant, true),
        };
        let rate = match op {
            // A body evaluated at another fragment only varies if it reads one.
            Op::Apply => args[0].rate,
            _ => args.iter().map(|arg| arg.rate).fold(own_rate, Rate::max),
        };
        let cpu = own_cpu && args.iter().all(|arg| arg.cpu);
        let depth = args.iter().map(|arg| arg.depth + 1).max().unwrap_or(0);
        let inner = args.iter().fold(0, |open, arg| open | arg.open);
        let open = match op {
            Op::LoopState(level) | Op::LoopIndex(level) => 1 << level,
            // A loop closes over its own state and index.
            Op::Loop { level, .. } => inner & !(1 << level),
            _ => inner,
        };
        Arc::new(Self {
            op,
            args,
            ty,
            rate,
            cpu,
            depth,
            open,
        })
    }

    /// The single fragment leaf shared by every expression.
    pub(crate) fn fragment() -> Arc<Self> {
        static FRAGMENT: LazyLock<Arc<Node>> =
            LazyLock::new(|| Node::new(Op::Fragment, Ty::Fragment, SmallVec::new()));
        FRAGMENT.clone()
    }

    pub(crate) fn invalid(ty: Ty, message: impl Into<Arc<str>>) -> Arc<Self> {
        Node::new(Op::Invalid(message.into()), ty, SmallVec::new())
    }

    /// Whether this subtree can be evaluated once on the CPU.
    pub(crate) fn folds(&self) -> bool {
        self.cpu && self.rate != Rate::Fragment && self.open == 0
    }
}

impl Drop for Node {
    // Long chains built in loops must not overflow the stack when dropped.
    fn drop(&mut self) {
        let mut pending: Vec<Arc<Node>> = self.args.drain(..).collect();
        while let Some(node) = pending.pop() {
            if let Ok(mut node) = Arc::try_unwrap(node) {
                pending.extend(node.args.drain(..));
            }
        }
    }
}

/// A typed shader expression.
///
/// Expressions are immutable and cheap to clone; clones share structure.
/// Plain CPU values passed as operands become uniform parameters, so
/// rebuilding an expression with new numbers reuses its compiled program.
pub struct Expr<T: Value> {
    pub(crate) node: Arc<Node>,
    ty: PhantomData<fn() -> T>,
}

/// A scalar expression.
pub type Scalar = Expr<f32>;
/// A two-component vector expression.
pub type Vec2 = Expr<Vec2f>;
/// A three-component vector expression.
pub type Vec3 = Expr<Vec3f>;
/// A four-component vector expression.
pub type Vec4 = Expr<Vec4f>;
/// A boolean expression.
pub type Bool = Expr<bool>;

impl<T: Value> Clone for Expr<T> {
    fn clone(&self) -> Self {
        Self::from_node(self.node.clone())
    }
}

impl<T: Value> fmt::Debug for Expr<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Expr<{:?}>", T::TY)
    }
}

impl<T: Value> Expr<T> {
    pub(crate) fn from_node(node: Arc<Node>) -> Self {
        debug_assert_eq!(node.ty, T::TY);
        Self {
            node,
            ty: PhantomData,
        }
    }

    pub(crate) fn make(op: Op, args: impl IntoIterator<Item = Arc<Node>>) -> Self {
        Self::from_node(Node::new(op, T::TY, args.into_iter().collect()))
    }

    /// A uniform parameter: changing it never changes the compiled program.
    pub fn uniform(value: T) -> Self {
        Self::make(Op::Uniform(value.into_val()), [])
    }

    /// A value embedded in the generated code. Prefer [`Expr::uniform`] (or
    /// a plain operand) for anything that animates.
    pub fn constant(value: T) -> Self {
        Self::make(Op::Constant(value.into_val()), [])
    }

    fn member<U: Value>(&self, name: &'static str) -> Expr<U> {
        Expr::make(Op::Member(name), [self.node.clone()])
    }

    pub(crate) fn call(
        callee: Callee,
        eval: Eval,
        args: impl IntoIterator<Item = Arc<Node>>,
    ) -> Self {
        Self::make(Op::Call(callee, eval), args)
    }

    /// An operand as a `T`, splatting scalars across vectors.
    pub(crate) fn widen<R: Operand<Value: Widen<T>>>(operand: R) -> Self {
        let node = operand.into_expr().node;
        if node.ty == T::TY {
            Self::from_node(node)
        } else {
            Self::make(Op::Construct, [node])
        }
    }
}

/// Embed a value in the generated code.
pub fn constant<T: Value>(value: T) -> Expr<T> {
    Expr::constant(value)
}

/// Loops nest at most this deep.
const MAX_LOOP_NESTING: u8 = 8;

thread_local! {
    static LOOP_LEVEL: Cell<u8> = const { Cell::new(0) };
}

/// A loop that runs per fragment: `count` applications of `step`, starting
/// from `init`. `step` receives the state and the iteration index (`0.0`,
/// `1.0`, ...) and runs once, on the CPU, to describe the body, which becomes
/// one WGSL `for` loop. (A Rust loop over expressions would instead unroll into
/// `count` copies.) Anything the body reads that does not change between
/// iterations is computed once, before the loop.
///
/// Carry several values by packing them into a vector.
///
/// ```ignore
/// // Fractal noise: five octaves, each finer and fainter.
/// let fbm = iterate(5, vec2(0.0, 1.0), |acc, octave| {
///     let scale = constant(2.0).pow(octave);
///     vec2(acc.x() + noise::value(&p * &scale) * acc.y(), acc.y() * 0.5)
/// }).x();
/// ```
pub fn iterate<T: Value>(
    count: u32,
    init: impl Operand<Value = T>,
    step: impl FnOnce(Expr<T>, Scalar) -> Expr<T>,
) -> Expr<T> {
    build_loop(count, init.into_expr(), None::<fn(&Expr<T>) -> Bool>, step)
}

/// [`iterate`], stopping early once `done` holds for the state. `done` is
/// checked before each step, so a raymarch can stop at the surface:
///
/// ```ignore
/// let distance = iterate_until(96, 0.0, |t| scene(&origin + &ray * t).lt(0.001), |t, _| {
///     let d = scene(&origin + &ray * &t);
///     t + d
/// });
/// ```
pub fn iterate_until<T: Value>(
    count: u32,
    init: impl Operand<Value = T>,
    done: impl FnOnce(&Expr<T>) -> Bool,
    step: impl FnOnce(Expr<T>, Scalar) -> Expr<T>,
) -> Expr<T> {
    build_loop(count, init.into_expr(), Some(done), step)
}

fn build_loop<T: Value>(
    count: u32,
    init: Expr<T>,
    done: Option<impl FnOnce(&Expr<T>) -> Bool>,
    step: impl FnOnce(Expr<T>, Scalar) -> Expr<T>,
) -> Expr<T> {
    struct Nest(u8);
    impl Drop for Nest {
        fn drop(&mut self) {
            LOOP_LEVEL.set(self.0);
        }
    }

    let level = LOOP_LEVEL.get();
    if level >= MAX_LOOP_NESTING {
        let message = format!("loops nest at most {MAX_LOOP_NESTING} deep");
        return Expr::from_node(Node::invalid(T::TY, message));
    }
    let nest = Nest(level);
    LOOP_LEVEL.set(level + 1);
    let state = Expr::<T>::make(Op::LoopState(level), []);
    let index = Scalar::make(Op::LoopIndex(level), []);
    let done = done.map(|done| done(&state).node);
    let next = step(state.clone(), index.clone());
    drop(nest);

    let mut args: SmallVec<[Arc<Node>; 3]> = [init.node, state.node, index.node, next.node]
        .into_iter()
        .collect();
    args.extend(done);
    Expr::from_node(Node::new(Op::Loop { count, level }, T::TY, args))
}

fn eval_arith<L: Arith<R>, R: Value, const OP: u8>(args: &[Val]) -> Val {
    L::from_val(args[0])
        .apply::<OP>(R::from_val(args[1]))
        .into_val()
}

fn arith<L: Arith<R>, R: Value, const OP: u8>(lhs: Arc<Node>, rhs: Arc<Node>) -> Expr<L::Output> {
    Expr::make(Op::Binary(bin_op(OP), eval_arith::<L, R, OP>), [lhs, rhs])
}

macro_rules! operators {
    ($($trait:ident $method:ident $op:ident),*) => {$(
        impl<L: Value, R: Operand> $trait<R> for Expr<L>
        where
            L: Arith<R::Value>,
        {
            type Output = Expr<L::Output>;
            fn $method(self, rhs: R) -> Self::Output {
                arith::<L, R::Value, $op>(self.node, rhs.into_expr().node)
            }
        }

        impl<L: Value, R: Operand> $trait<R> for &Expr<L>
        where
            L: Arith<R::Value>,
        {
            type Output = Expr<L::Output>;
            fn $method(self, rhs: R) -> Self::Output {
                arith::<L, R::Value, $op>(self.node.clone(), rhs.into_expr().node)
            }
        }

        impl<R: Value> $trait<Expr<R>> for f32
        where
            f32: Arith<R>,
        {
            type Output = Expr<<f32 as Arith<R>>::Output>;
            fn $method(self, rhs: Expr<R>) -> Self::Output {
                arith::<f32, R, $op>(Expr::uniform(self).node, rhs.node)
            }
        }

        impl<R: Value> $trait<&Expr<R>> for f32
        where
            f32: Arith<R>,
        {
            type Output = Expr<<f32 as Arith<R>>::Output>;
            fn $method(self, rhs: &Expr<R>) -> Self::Output {
                arith::<f32, R, $op>(Expr::uniform(self).node, rhs.node.clone())
            }
        }
    )*};
}

operators!(Add add ADD, Sub sub SUB, Mul mul MUL, Div div DIV);

fn eval_neg<T: Float>(args: &[Val]) -> Val {
    T::from_val(args[0]).apply::<MUL>(-1.0).into_val()
}

impl<T: Float> Neg for Expr<T> {
    type Output = Self;
    fn neg(self) -> Self {
        Self::make(Op::Unary(ir::UnOp::Neg, eval_neg::<T>), [self.node])
    }
}

impl<T: Float> Neg for &Expr<T> {
    type Output = Expr<T>;
    fn neg(self) -> Expr<T> {
        -self.clone()
    }
}

fn eval_compare<const OP: u8>(args: &[Val]) -> Val {
    let (a, b) = (f32::from_val(args[0]), f32::from_val(args[1]));
    Val::Bool(match OP {
        0 => a < b,
        1 => a <= b,
        2 => a > b,
        3 => a >= b,
        4 => a == b,
        _ => a != b,
    })
}

macro_rules! comparisons {
    ($($method:ident $op:literal $bin:ident),*) => {
        impl Expr<f32> {$(
            #[doc = concat!("Compare per fragment with WGSL `", stringify!($method), "` semantics.")]
            pub fn $method(&self, other: impl Operand<Value = f32>) -> Bool {
                Expr::make(
                    Op::Binary(ir::BinOp::$bin, eval_compare::<$op>),
                    [self.node.clone(), other.into_expr().node],
                )
            }
        )*}
    };
}

comparisons!(lt 0 Lt, le 1 Le, gt 2 Gt, ge 3 Ge, eq 4 Eq, ne 5 Ne);

fn eval_logic<const OP: u8>(args: &[Val]) -> Val {
    let a = bool::from_val(args[0]);
    Val::Bool(match OP {
        0 => a && bool::from_val(args[1]),
        1 => a || bool::from_val(args[1]),
        _ => !a,
    })
}

pub(crate) fn eval_select(args: &[Val]) -> Val {
    if bool::from_val(args[2]) {
        args[1]
    } else {
        args[0]
    }
}

impl Bool {
    /// Logical AND.
    pub fn and(&self, other: impl Operand<Value = bool>) -> Bool {
        Self::make(
            Op::Binary(ir::BinOp::And, eval_logic::<0>),
            [self.node.clone(), other.into_expr().node],
        )
    }

    /// Logical OR.
    pub fn or(&self, other: impl Operand<Value = bool>) -> Bool {
        Self::make(
            Op::Binary(ir::BinOp::Or, eval_logic::<1>),
            [self.node.clone(), other.into_expr().node],
        )
    }

    /// Logical NOT.
    pub fn not(&self) -> Bool {
        Self::make(
            Op::Unary(ir::UnOp::Not, eval_logic::<2>),
            [self.node.clone()],
        )
    }

    /// Choose `if_true` where this holds and `if_false` elsewhere, per fragment.
    /// (An ordinary Rust `if` chooses on the CPU instead.)
    pub fn select<T: Value>(
        &self,
        if_true: impl Operand<Value = T>,
        if_false: impl Operand<Value = T>,
    ) -> Expr<T> {
        Expr::make(
            Op::Select,
            [
                if_false.into_expr().node,
                if_true.into_expr().node,
                self.node.clone(),
            ],
        )
    }
}

pub(crate) fn eval_construct(ty: Ty, args: &[Val]) -> Val {
    let mut lanes = SmallVec::<[f32; 4]>::new();
    for arg in args {
        lanes.extend_from_slice(&arg.lanes()[..arg.ty().lanes()]);
    }
    // One scalar splats.
    lanes.resize(4, lanes[0]);
    Val::from_lanes(ty, [lanes[0], lanes[1], lanes[2], lanes[3]])
}

/// A two-component vector.
pub fn vec2(x: impl Operand<Value = f32>, y: impl Operand<Value = f32>) -> Vec2 {
    Expr::make(Op::Construct, [x.into_expr().node, y.into_expr().node])
}

/// A three-component vector.
pub fn vec3(
    x: impl Operand<Value = f32>,
    y: impl Operand<Value = f32>,
    z: impl Operand<Value = f32>,
) -> Vec3 {
    Expr::make(
        Op::Construct,
        [x.into_expr().node, y.into_expr().node, z.into_expr().node],
    )
}

/// A four-component vector.
pub fn vec4(
    x: impl Operand<Value = f32>,
    y: impl Operand<Value = f32>,
    z: impl Operand<Value = f32>,
    w: impl Operand<Value = f32>,
) -> Vec4 {
    Expr::make(
        Op::Construct,
        [
            x.into_expr().node,
            y.into_expr().node,
            z.into_expr().node,
            w.into_expr().node,
        ],
    )
}

impl<T: Vector> Expr<T> {
    /// A vector with every component set to `value`.
    pub fn splat(value: impl Operand<Value = f32>) -> Self {
        Self::make(Op::Construct, [value.into_expr().node])
    }

    /// The first component.
    pub fn x(&self) -> Scalar {
        self.member("x")
    }

    /// The second component.
    pub fn y(&self) -> Scalar {
        self.member("y")
    }
}

impl Vec2 {
    /// The components in reverse order.
    pub fn yx(&self) -> Vec2 {
        self.member("yx")
    }

    /// This vector rotated by `angle` radians about the origin. With y
    /// pointing down, as in GPUI, positive angles turn clockwise.
    pub fn rotate(&self, angle: impl Operand<Value = f32>) -> Vec2 {
        let angle = angle.into_expr();
        let (sin, cos) = (angle.sin(), angle.cos());
        let (x, y) = (self.x(), self.y());
        vec2(&x * &cos - &y * &sin, x * sin + y * cos)
    }

    /// Append a third component.
    pub fn extend(&self, z: impl Operand<Value = f32>) -> Vec3 {
        Expr::make(Op::Construct, [self.node.clone(), z.into_expr().node])
    }
}

impl Vec3 {
    /// The third component.
    pub fn z(&self) -> Scalar {
        self.member("z")
    }

    /// The first two components.
    pub fn xy(&self) -> Vec2 {
        self.member("xy")
    }

    /// Append a fourth component.
    pub fn extend(&self, w: impl Operand<Value = f32>) -> Vec4 {
        Expr::make(Op::Construct, [self.node.clone(), w.into_expr().node])
    }
}

impl Vec4 {
    /// The third component.
    pub fn z(&self) -> Scalar {
        self.member("z")
    }

    /// The fourth component.
    pub fn w(&self) -> Scalar {
        self.member("w")
    }

    /// The first two components.
    pub fn xy(&self) -> Vec2 {
        self.member("xy")
    }

    /// The first three components.
    pub fn xyz(&self) -> Vec3 {
        self.member("xyz")
    }
}

impl Expr<Fragment> {
    /// Normalized coordinate across the painted box.
    pub fn uv(&self) -> Vec2 {
        self.member("uv")
    }

    /// Logical-pixel offset from the painted box's top-left corner.
    pub fn position(&self) -> Vec2 {
        self.member("position")
    }

    /// Logical-pixel size of the painted box.
    pub fn size(&self) -> Vec2 {
        self.member("size")
    }

    /// Device pixels per logical pixel.
    pub fn scale(&self) -> Scalar {
        self.member("scale")
    }

    /// Logical-pixel offset from the painted box's center.
    pub fn centered(&self) -> Vec2 {
        self.position() - self.size() * constant(0.5)
    }

    /// This fragment re-addressed at another normalized coordinate.
    pub fn at(&self, uv: impl Operand<Value = Vec2f>) -> Expr<Fragment> {
        fragment_at(self, uv)
    }
}

pub(crate) fn eval_member(name: &str, value: Val) -> Val {
    match (value, name) {
        (Val::Fragment(fragment), "uv") => Val::Vec2(fragment.uv),
        (Val::Fragment(fragment), "position") => Val::Vec2(fragment.position),
        (Val::Fragment(fragment), "size") => Val::Vec2(fragment.size),
        (Val::Fragment(fragment), "origin") => Val::Vec2(fragment.origin),
        (Val::Fragment(fragment), "scale") => Val::F32(fragment.scale),
        (_, swizzle) => {
            let (lanes, mut picked) = (value.lanes(), [0.0; 4]);
            for (lane, component) in picked.iter_mut().zip(swizzle.chars()) {
                *lane = lanes["xyzw".find(component).expect("a swizzle")];
            }
            let ty = [Ty::F32, Ty::Vec2, Ty::Vec3, Ty::Vec4][swizzle.len() - 1];
            Val::from_lanes(ty, picked)
        }
    }
}
