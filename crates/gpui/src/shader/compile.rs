//! From an expression DAG to a cached, validated WGSL program.
//!
//! Canonicalization folds CPU expressions and assigns shared subtrees one id.
//! The resulting structure keys the program cache; uniform values are excluded.
//! On a cache miss, lowering emits one function per remapped paint body,
//! wgsl-rs renders the IR, and Naga validates it. Finally, values are packed
//! into the cached program's parameter layout.

use std::{
    fmt,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
};

use collections::FxHashMap;
use smallvec::SmallVec;
use wgsl_rs::ir;

use super::{
    expr::{Callee, Node, Op, Rate, eval_construct, eval_member, eval_select},
    prelude::{self, Fragment},
    value::{Ty, Val, sealed::Sealed},
};

const MAX_DEPTH: u32 = 512;
const MAX_NODES: usize = 16_384;
const MAX_SLOTS: usize = 4096;
const MAX_SOURCE_BYTES: usize = 1 << 20;
const CACHE_CAPACITY: usize = 256;

/// Why a shader could not be built.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ShaderError {
    /// An expression nests more than 512 levels deep.
    TooDeep,
    /// An expression has more than 16,384 distinct nodes.
    TooManyNodes,
    /// A paint needs more than 4,096 parameter slots.
    TooManyParameters,
    /// A parameter or constant is NaN or infinite.
    NonFinite,
    /// Generated code exceeds 1 MiB.
    SourceTooLarge,
    /// Foreign shader source was rejected, or a call did not match it.
    Source(String),
}

impl fmt::Display for ShaderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooDeep => write!(f, "shader expression nests deeper than {MAX_DEPTH} levels"),
            Self::TooManyNodes => write!(f, "shader expression exceeds {MAX_NODES} nodes"),
            Self::TooManyParameters => write!(f, "shader exceeds {MAX_SLOTS} parameter slots"),
            Self::NonFinite => f.write_str("shader values must be finite"),
            Self::SourceTooLarge => write!(f, "generated shader exceeds {MAX_SOURCE_BYTES} bytes"),
            Self::Source(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for ShaderError {}

/// A compiled fragment program, shared by every paint with the same structure.
///
/// Its [source](Program::source) defines
/// `fn paint_program(fragment: Fragment, base: u32) -> vec4<f32>`, returning
/// premultiplied color. Renderers link it against the [prelude](super::prelude)
/// and provide `paint_param(index: u32) -> vec4<f32>` (parameter slots start
/// at `base`) and, when [`Program::uses_backdrop`], `paint_backdrop(fragment:
/// Fragment, uv: vec2<f32>) -> vec4<f32>`.
#[derive(Clone)]
pub struct Program(Arc<ProgramInner>);

struct ProgramInner {
    id: u64,
    source: String,
    lanes: Box<[Lanes]>,
    slots: usize,
    uses_backdrop: bool,
}

/// Where one parameter lives: `len` lanes of slot `slot`, starting at `lane`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Lanes {
    pub(crate) slot: u32,
    pub(crate) lane: u8,
    pub(crate) len: u8,
}

impl Lanes {
    /// The swizzle selecting these lanes, if they are not a whole slot.
    pub(crate) fn swizzle(self) -> Option<&'static str> {
        let start = self.lane as usize;
        (self.len < 4).then(|| &"xyzw"[start..start + self.len as usize])
    }
}

/// Pack values of these types greedily into vec4 slots, in order.
pub(crate) fn layout(types: impl IntoIterator<Item = Ty>) -> (Vec<Lanes>, u32) {
    let mut lanes = Vec::new();
    let mut slots = 0u32;
    let mut open = 4u8;
    for ty in types {
        let len = ty.lanes() as u8;
        if open + len > 4 {
            slots += 1;
            open = 0;
        }
        lanes.push(Lanes {
            slot: slots - 1,
            lane: open,
            len,
        });
        open += len;
    }
    (lanes, slots)
}

/// Write `values` into `slots` at their `lanes`.
pub(crate) fn pack(values: &[Val], lanes: &[Lanes], slots: &mut [[f32; 4]]) {
    for (value, location) in values.iter().zip(lanes) {
        let (start, len) = (location.lane as usize, location.len as usize);
        slots[location.slot as usize][start..start + len].copy_from_slice(&value.lanes()[..len]);
    }
}

impl Program {
    /// Identity, stable while the program stays cached.
    pub fn id(&self) -> u64 {
        self.0.id
    }

    /// WGSL source; see [`Program`].
    pub fn source(&self) -> &str {
        &self.0.source
    }

    /// Whether the program samples the scene behind the painted box.
    pub fn uses_backdrop(&self) -> bool {
        self.0.uses_backdrop
    }

    /// Number of `vec4<f32>` parameter slots each use of the program reads.
    pub fn parameter_slots(&self) -> usize {
        self.0.slots
    }
}

impl fmt::Debug for Program {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Program")
            .field("id", &self.0.id)
            .field("slots", &self.0.slots)
            .field("uses_backdrop", &self.0.uses_backdrop)
            .finish_non_exhaustive()
    }
}

/// A [`Program`] and one paint's parameter values.
#[derive(Clone, Debug)]
pub struct CompiledPaint {
    /// The shared program.
    pub program: Program,
    /// Parameter slots, laid out for the program.
    pub params: Arc<[[f32; 4]]>,
}

impl PartialEq for CompiledPaint {
    fn eq(&self, other: &Self) -> bool {
        self.program.id() == other.program.id()
            && self.params.len() == other.params.len()
            && self
                .params
                .iter()
                .flatten()
                .zip(other.params.iter().flatten())
                .all(|(a, b)| a.to_bits() == b.to_bits())
    }
}

pub(crate) fn compile(root: &Arc<Node>) -> Result<CompiledPaint, ShaderError> {
    if root.depth > MAX_DEPTH {
        return Err(ShaderError::TooDeep);
    }
    let mut canonical = Canonical::default();
    canonical.visit(root)?;

    let cached = cache()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&canonical.codes);
    let program = match cached {
        Some(program) => program,
        None => {
            let program = canonical.lower()?;
            cache()
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(canonical.codes.clone(), program.clone());
            program
        }
    };

    let mut params = vec![[0.0; 4]; program.0.slots];
    pack(&canonical.params, &program.0.lanes, &mut params);
    Ok(CompiledPaint {
        program,
        params: params.into(),
    })
}

/// Evaluate an expression on the CPU, at `fragment` if it reads one.
pub(crate) fn evaluate(node: &Node, fragment: Option<Fragment>) -> Option<Val> {
    if node.depth > MAX_DEPTH {
        return None;
    }
    Evaluator {
        fragment,
        bindings: FxHashMap::default(),
        memo: FxHashMap::default(),
    }
    .eval(node)
}

struct Evaluator {
    fragment: Option<Fragment>,
    /// The state and index of each enclosing loop.
    bindings: FxHashMap<*const Node, Val>,
    memo: FxHashMap<*const Node, Val>,
}

impl Evaluator {
    /// An evaluator for a body that sees these bindings, and `extra`.
    fn nested(&self, fragment: Option<Fragment>, extra: &[(&Node, Val)]) -> Self {
        let mut bindings = self.bindings.clone();
        bindings.extend(
            extra
                .iter()
                .map(|&(node, value)| (std::ptr::from_ref(node), value)),
        );
        Self {
            fragment,
            bindings,
            memo: FxHashMap::default(),
        }
    }

    fn eval(&mut self, node: &Node) -> Option<Val> {
        let key = std::ptr::from_ref(node);
        if let Some(value) = self.memo.get(&key) {
            return Some(*value);
        }
        let value = match &node.op {
            Op::Apply if node.args[0].rate == Rate::Fragment => {
                let fragment = Fragment::from_val(self.eval(&node.args[1])?);
                self.nested(Some(fragment), &[]).eval(&node.args[0])?
            }
            Op::Apply => self.eval(&node.args[0])?,
            Op::Loop { count, .. } => {
                let [init, state, index, next] = [0, 1, 2, 3].map(|arg| &*node.args[arg]);
                let mut value = self.eval(init)?;
                for iteration in 0..*count {
                    let index_value = Val::F32(iteration as f32);
                    let mut body =
                        self.nested(self.fragment, &[(state, value), (index, index_value)]);
                    if let Some(done) = node.args.get(4)
                        && bool::from_val(body.eval(done)?)
                    {
                        break;
                    }
                    value = body.eval(next)?;
                }
                value
            }
            Op::LoopState(_) | Op::LoopIndex(_) => *self.bindings.get(&key)?,
            op => {
                let args = node
                    .args
                    .iter()
                    .map(|arg| self.eval(arg))
                    .collect::<Option<SmallVec<[Val; 4]>>>()?;
                match op {
                    Op::Uniform(value) | Op::Constant(value) => *value,
                    Op::Fragment => Val::Fragment(self.fragment?),
                    Op::Member(name) => eval_member(name, args[0]),
                    Op::Unary(_, eval) | Op::Binary(_, eval) | Op::Call(_, eval) => eval(&args),
                    Op::Construct => eval_construct(node.ty, &args),
                    Op::Select => eval_select(&args),
                    Op::Backdrop | Op::Invalid(_) => return None,
                    Op::Apply | Op::Loop { .. } | Op::LoopState(_) | Op::LoopIndex(_) => {
                        unreachable!()
                    }
                }
            }
        };
        self.memo.insert(key, value);
        Some(value)
    }
}

/// A node with its children replaced by canonical ids.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct Code {
    op: OpCode,
    ty: Ty,
    args: SmallVec<[u32; 3]>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum OpCode {
    Param,
    Constant([u32; 4]),
    Fragment,
    Member(&'static str),
    Unary(ir::UnOp),
    Binary(ir::BinOp),
    Call(Callee),
    Construct,
    Select,
    Apply,
    Backdrop,
    LoopState(u8),
    LoopIndex(u8),
    Loop { count: u32, level: u8 },
}

#[derive(Default)]
struct Canonical {
    ids: FxHashMap<*const Node, u32>,
    codes: Vec<Code>,
    /// Each distinct code's id: structurally equal subtrees, however they
    /// were built, are emitted once. Parameters are never merged; each
    /// carries its own value.
    interned: FxHashMap<Code, u32>,
    params: Vec<Val>,
    /// Levels of the loops being visited, one bit each.
    looping: u32,
}

impl Canonical {
    fn visit(&mut self, node: &Arc<Node>) -> Result<u32, ShaderError> {
        if node.open & !self.looping != 0 {
            return Err(ShaderError::Source(
                "a loop's state or index was used outside the loop".into(),
            ));
        }
        let key = Arc::as_ptr(node);
        if let Some(&id) = self.ids.get(&key) {
            return Ok(id);
        }
        let leaf = |value: Val, rate: Rate, params: &mut Vec<Val>| {
            if !value.is_finite() {
                return Err(ShaderError::NonFinite);
            }
            Ok(if rate == Rate::Constant {
                OpCode::Constant(value.bits())
            } else {
                params.push(value);
                OpCode::Param
            })
        };
        let (op, args) = match &node.op {
            Op::Uniform(value) | Op::Constant(value) => {
                (leaf(*value, node.rate, &mut self.params)?, SmallVec::new())
            }
            _ if node.folds() => {
                let value = evaluate(node, None).expect("foldable expressions evaluate");
                (leaf(value, node.rate, &mut self.params)?, SmallVec::new())
            }
            Op::Invalid(message) => return Err(ShaderError::Source(message.to_string())),
            // The body becomes its own function, outside any loop.
            Op::Apply if node.args[0].open != 0 => {
                return Err(ShaderError::Source(
                    "paints evaluated elsewhere (`at`, `map_uv`) cannot read loop state".into(),
                ));
            }
            op => {
                let looping = self.looping;
                if let Op::Loop { level, .. } = op {
                    self.looping |= 1 << level;
                }
                let args = node
                    .args
                    .iter()
                    .map(|arg| self.visit(arg))
                    .collect::<Result<_, _>>();
                self.looping = looping;
                let args = args?;
                let op = match op {
                    Op::Fragment => OpCode::Fragment,
                    Op::Member(name) => OpCode::Member(name),
                    Op::Unary(op, _) => OpCode::Unary(*op),
                    Op::Binary(op, _) => OpCode::Binary(*op),
                    Op::Call(callee, _) => OpCode::Call(*callee),
                    Op::Construct => OpCode::Construct,
                    Op::Select => OpCode::Select,
                    Op::Apply => OpCode::Apply,
                    Op::Backdrop => OpCode::Backdrop,
                    Op::LoopState(level) => OpCode::LoopState(*level),
                    Op::LoopIndex(level) => OpCode::LoopIndex(*level),
                    Op::Loop { count, level } => OpCode::Loop {
                        count: *count,
                        level: *level,
                    },
                    Op::Uniform(_) | Op::Constant(_) | Op::Invalid(_) => unreachable!(),
                };
                (op, args)
            }
        };
        let code = Code {
            op,
            ty: node.ty,
            args,
        };
        let interned = (code.op != OpCode::Param)
            .then(|| self.interned.get(&code).copied())
            .flatten();
        let id = match interned {
            Some(id) => id,
            None => {
                if self.codes.len() >= MAX_NODES {
                    return Err(ShaderError::TooManyNodes);
                }
                let id = self.codes.len() as u32;
                if code.op != OpCode::Param {
                    self.interned.insert(code.clone(), id);
                }
                self.codes.push(code);
                id
            }
        };
        self.ids.insert(key, id);
        Ok(id)
    }

    fn lower(&self) -> Result<Program, ShaderError> {
        let (lanes, slots) = layout(self.params.iter().map(|value| value.ty()));
        if slots as usize > MAX_SLOTS {
            return Err(ShaderError::TooManyParameters);
        }

        let mut lowering = Lowering {
            codes: &self.codes,
            params: {
                let mut next = lanes.iter();
                self.codes
                    .iter()
                    .map(|code| (code.op == OpCode::Param).then(|| *next.next().unwrap()))
                    .collect()
            },
            open: {
                let mut open = Vec::<u32>::with_capacity(self.codes.len());
                for code in &self.codes {
                    let inner = code
                        .args
                        .iter()
                        .fold(0, |mask, &arg| mask | open[arg as usize]);
                    open.push(match code.op {
                        OpCode::LoopState(level) | OpCode::LoopIndex(level) => 1 << level,
                        OpCode::Loop { level, .. } => inner & !(1 << level),
                        _ => inner,
                    });
                }
                open
            },
            items: Vec::new(),
            functions: FxHashMap::default(),
            uses_backdrop: false,
            names: 0,
        };
        let root = self.codes.len() as u32 - 1;
        lowering.function("paint_program".into(), root);

        let source = ir::render_items(&lowering.items);
        if source.len() > MAX_SOURCE_BYTES {
            return Err(ShaderError::SourceTooLarge);
        }
        validate(&source)?;

        static NEXT_ID: AtomicU64 = AtomicU64::new(1);
        Ok(Program(Arc::new(ProgramInner {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            source,
            lanes: lanes.into(),
            slots: slots as usize,
            uses_backdrop: lowering.uses_backdrop,
        })))
    }
}

struct Lowering<'a> {
    codes: &'a [Code],
    /// Parameter location of each code, if it is a parameter.
    params: Vec<Option<Lanes>>,
    /// Levels of the loops each code reads the state or index of, one bit each.
    open: Vec<u32>,
    items: Vec<ir::Item>,
    /// Function emitted for each body evaluated at another fragment.
    functions: FxHashMap<u32, String>,
    uses_backdrop: bool,
    /// Locals named so far; names are unique across nested blocks.
    names: u32,
}

/// A block being emitted, and the codes already available in it.
#[derive(Default)]
struct Body {
    stmts: Vec<ir::Stmt>,
    values: FxHashMap<u32, ir::Expr>,
}

impl Lowering<'_> {
    fn name(&mut self, prefix: &str) -> String {
        self.names += 1;
        format!("{prefix}{}", self.names)
    }

    fn bind(&mut self, scope: &mut Body, init: ir::Expr) -> ir::Expr {
        let name = self.name("v");
        scope.stmts.push(ir::Stmt::Local(ir::Local {
            mutable: false,
            name: name.clone(),
            ty: None,
            init: Some(init),
        }));
        ident(&name)
    }

    /// Lower, ahead of the loop at `level`, every part of its body (`roots`)
    /// that is the same in each iteration.
    fn hoist(&mut self, roots: &[u32], level: u8, scope: &mut Body) {
        let mut seen = collections::FxHashSet::default();
        let mut pending = roots.to_vec();
        let mut invariant = Vec::new();
        while let Some(id) = pending.pop() {
            if !seen.insert(id) || scope.values.contains_key(&id) {
                continue;
            }
            if self.open[id as usize] >> level == 0 {
                invariant.push(id);
                continue;
            }
            let code = &self.codes[id as usize];
            // An `Apply` body is its own function.
            let skip = usize::from(code.op == OpCode::Apply);
            pending.extend(&code.args[skip..]);
        }
        // Children have smaller ids, so this lowers them first.
        invariant.sort_unstable();
        for id in invariant {
            self.lower(id, scope);
        }
    }

    /// `var s = init; for (var i = 0u; i < count; i++) { [if done { break; }] s = next; }`
    fn lower_loop(&mut self, code: &Code, count: u32, level: u8, scope: &mut Body) -> ir::Expr {
        let init = self.lower(code.args[0], scope);
        self.hoist(&code.args[3..], level, scope);
        let (state, index) = (self.name("s"), self.name("i"));
        scope.stmts.push(ir::Stmt::Local(ir::Local {
            mutable: true,
            name: state.clone(),
            ty: Some(code.ty.ir()),
            init: Some(init),
        }));

        let mut body = Body {
            stmts: Vec::new(),
            values: scope.values.clone(),
        };
        body.values.insert(code.args[1], ident(&state));
        body.values
            .insert(code.args[2], call("f32", vec![ident(&index)]));
        if let Some(&done) = code.args.get(4) {
            let condition = self.lower(done, &mut body);
            body.stmts.push(ir::Stmt::If(ir::StmtIf {
                condition,
                then_block: ir::Block {
                    stmts: vec![ir::Stmt::Break],
                },
                else_branch: None,
            }));
        }
        let next = self.lower(code.args[3], &mut body);
        body.stmts.push(ir::Stmt::Assignment {
            lhs: ident(&state),
            rhs: next,
        });

        let u32_literal = |value: u32| {
            ir::Expr::Lit(ir::Lit::Int {
                digits: value.to_string(),
                suffix: "u32".into(),
            })
        };
        scope.stmts.push(ir::Stmt::For(ir::ForLoop {
            var: index,
            var_ty: Some(ir::Type::Scalar(ir::ScalarType::U32)),
            from: u32_literal(0),
            to: u32_literal(count),
            inclusive: false,
            body: ir::Block { stmts: body.stmts },
        }));
        self.bind(scope, ident(&state))
    }

    /// Emit `fn name(fragment: Fragment, base: u32) -> T` returning code `body`.
    fn function(&mut self, name: String, body: u32) {
        let mut scope = Body::default();
        let value = self.lower(body, &mut scope);
        scope.stmts.push(ir::Stmt::Return(Some(value)));
        let argument = |name: &str, ty: ir::Type| ir::FnArg {
            inter_stage_io: vec![],
            name: name.into(),
            ty,
            attrs: vec![],
        };
        self.items.push(ir::Item::Fn(ir::ItemFn {
            type_params: vec![],
            const_params: vec![],
            fn_attrs: ir::FnAttrs::None,
            name: name.into(),
            inputs: vec![
                argument("fragment", Ty::Fragment.ir()),
                argument("base", ir::Type::Scalar(ir::ScalarType::U32)),
            ],
            return_type: ir::ReturnType::Type {
                annotation: ir::ReturnTypeAnnotation::None,
                ty: self.codes[body as usize].ty.ir(),
            },
            block: ir::Block { stmts: scope.stmts },
            attrs: vec![],
        }));
    }

    fn lower(&mut self, id: u32, scope: &mut Body) -> ir::Expr {
        if let Some(value) = scope.values.get(&id) {
            return value.clone();
        }
        let code = &self.codes[id as usize];
        let value = match &code.op {
            OpCode::Param => {
                let lanes = self.params[id as usize].expect("parameters have lanes");
                self.bind(scope, param(lanes, code.ty))
            }
            OpCode::Constant(bits) => constant(code.ty, *bits),
            OpCode::Fragment => ident("fragment"),
            &OpCode::Loop { count, level } => self.lower_loop(code, count, level, scope),
            OpCode::Member(name) => {
                let base = Box::new(self.lower(code.args[0], scope));
                if self.codes[code.args[0] as usize].ty == Ty::Fragment {
                    ir::Expr::FieldAccess {
                        base,
                        field: (*name).into(),
                    }
                } else {
                    ir::Expr::Swizzle {
                        lhs: base,
                        swizzle: (*name).into(),
                        params: None,
                    }
                }
            }
            OpCode::Apply => {
                let name = match self.functions.get(&code.args[0]) {
                    Some(name) => name.clone(),
                    None => {
                        let name = format!("paint_program_{}", self.functions.len() + 1);
                        self.functions.insert(code.args[0], name.clone());
                        self.function(name.clone(), code.args[0]);
                        name
                    }
                };
                let fragment = self.lower(code.args[1], scope);
                self.bind(scope, call(&name, vec![fragment, ident("base")]))
            }
            op => {
                let mut args: Vec<ir::Expr> = code
                    .args
                    .iter()
                    .map(|&arg| self.lower(arg, scope))
                    .collect();
                let value = match op {
                    OpCode::Unary(op) => ir::Expr::Unary {
                        op: *op,
                        expr: Box::new(args.remove(0)),
                    },
                    OpCode::Binary(op) => ir::Expr::Binary {
                        rhs: Box::new(args.remove(1)),
                        op: *op,
                        lhs: Box::new(args.remove(0)),
                    },
                    OpCode::Call(callee) => ir::Expr::FnCall {
                        path: callee.path(),
                        type_args: vec![],
                        params: args,
                    },
                    OpCode::Construct => call(vector_constructor(code.ty), args),
                    OpCode::Select => call("select", args),
                    OpCode::Backdrop => {
                        self.uses_backdrop = true;
                        call("paint_backdrop", args)
                    }
                    // Loop leaves are bound by their loop's body.
                    OpCode::Param
                    | OpCode::Constant(_)
                    | OpCode::Fragment
                    | OpCode::Member(_)
                    | OpCode::Apply
                    | OpCode::Loop { .. }
                    | OpCode::LoopState(_)
                    | OpCode::LoopIndex(_) => unreachable!(),
                };
                self.bind(scope, value)
            }
        };
        scope.values.insert(id, value.clone());
        value
    }
}

fn ident(name: &str) -> ir::Expr {
    ir::Expr::Ident(name.into())
}

fn call(name: &str, params: Vec<ir::Expr>) -> ir::Expr {
    ir::Expr::FnCall {
        path: ir::FnPath::Ident(name.into()),
        type_args: vec![],
        params,
    }
}

fn vector_constructor(ty: Ty) -> &'static str {
    match ty {
        Ty::Vec2 => "vec2f",
        Ty::Vec3 => "vec3f",
        Ty::Vec4 => "vec4f",
        _ => unreachable!("only float vectors are constructed"),
    }
}

/// `paint_param(base + slot)`, narrowed to the parameter's lanes.
fn param(lanes: Lanes, ty: Ty) -> ir::Expr {
    let index = if lanes.slot == 0 {
        ident("base")
    } else {
        ir::Expr::Binary {
            lhs: Box::new(ident("base")),
            op: ir::BinOp::Add,
            rhs: Box::new(ir::Expr::Lit(ir::Lit::Int {
                digits: lanes.slot.to_string(),
                suffix: "u32".into(),
            })),
        }
    };
    let slot = call("paint_param", vec![index]);
    let value = match lanes.swizzle() {
        None => slot,
        Some(swizzle) => ir::Expr::Swizzle {
            lhs: Box::new(slot),
            swizzle: swizzle.into(),
            params: None,
        },
    };
    if ty == Ty::Bool {
        ir::Expr::Binary {
            lhs: Box::new(value),
            op: ir::BinOp::Ne,
            rhs: Box::new(constant(Ty::F32, [0; 4])),
        }
    } else {
        value
    }
}

fn constant(ty: Ty, bits: [u32; 4]) -> ir::Expr {
    Val::from_lanes(ty, bits.map(f32::from_bits)).literal()
}

/// The prelude's WGSL, assembled once.
pub(crate) fn prelude_source() -> &'static str {
    static SOURCE: OnceLock<String> = OnceLock::new();
    SOURCE.get_or_init(|| {
        prelude::WGSL_SOURCE
            .wgsl_source()
            .expect("the shader prelude is a concrete module")
    })
}

/// Validate a program against the prelude and stand-ins for renderer glue.
fn validate(source: &str) -> Result<(), ShaderError> {
    const GLUE: &str = "
fn paint_param(index: u32) -> vec4<f32> { return vec4<f32>(0.0); }
fn paint_backdrop(fragment: Fragment, uv: vec2<f32>) -> vec4<f32> { return vec4<f32>(0.0); }
";
    // Renderers clip per pixel around paints, as UI shaders do, so derivatives
    // in loops that exit early are accepted.
    let module = format!(
        "diagnostic(off, derivative_uniformity);\n{}{GLUE}{source}",
        prelude_source()
    );
    validate_module(&module).map(drop)
}

pub(crate) fn validate_module(
    source: &str,
) -> Result<(naga::Module, naga::valid::ModuleInfo), ShaderError> {
    let module = naga::front::wgsl::parse_str(source)
        .map_err(|error| ShaderError::Source(error.emit_to_string(source)))?;
    let info = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::empty(),
    )
    .validate(&module)
    .map_err(|error| ShaderError::Source(error.emit_to_string(source)))?;
    Ok((module, info))
}

#[derive(Default)]
struct Cache {
    programs: FxHashMap<Vec<Code>, (Program, u64)>,
    tick: u64,
}

impl Cache {
    fn get(&mut self, codes: &[Code]) -> Option<Program> {
        self.tick += 1;
        let tick = self.tick;
        self.programs.get_mut(codes).map(|(program, used)| {
            *used = tick;
            program.clone()
        })
    }

    fn insert(&mut self, codes: Vec<Code>, program: Program) {
        if self.programs.len() >= CACHE_CAPACITY {
            if let Some(oldest) = self
                .programs
                .iter()
                .min_by_key(|(_, (_, used))| *used)
                .map(|(codes, _)| codes.clone())
            {
                self.programs.remove(&oldest);
            }
        }
        self.tick += 1;
        self.programs.insert(codes, (program, self.tick));
    }
}

fn cache() -> &'static Mutex<Cache> {
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    CACHE.get_or_init(Mutex::default)
}
