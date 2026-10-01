//! Typed, composable fragment paint programs shared by canvas and element painting.
//!
//! Programs are compiled once. Updating a parameter copies its small value table and
//! shares the immutable program; it does not regenerate WGSL or change pipeline identity.
//! Coordinates and distances use logical pixels; `uv` spans the painted bounds.
//! A root shares compiled program descriptions, not GPU device ownership. Normal
//! Rust functions compose expressions; this API does not transpile Rust source.
//!
//! ```compile_fail
//! use gpui::paint::PaintRoot;
//! PaintRoot::new().shader(|cx| {
//!     let invalid = cx.scalar(1.0) + cx.vec2([1.0, 2.0]);
//!     cx.vec4([1.0; 4])
//! });
//! ```
//!
//! ```
//! use gpui::paint::PaintRoot;
//! let root = PaintRoot::new();
//! let phase = root.parameter(0.0).unwrap();
//! let shader = root.shader(|cx| {
//!     let red = (cx.uv().x() + cx.uniform(phase)).sin();
//!     cx.rgba(red, cx.scalar(0.2), cx.scalar(0.8), cx.scalar(1.0))
//! }).unwrap();
//! let animated = shader.with_parameter(phase, 0.5).unwrap();
//! assert_eq!(shader.program_id(), animated.program_id());
//! ```
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet, VecDeque},
    fmt,
    marker::PhantomData,
    ops::{Add, Div, Mul, Neg, Sub},
    rc::Rc,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

#[path = "paint/canvas.rs"]
mod canvas;
pub use canvas::*;
#[path = "paint/field.rs"]
mod field;
pub use field::{Coverage, Distance};
#[path = "paint/function.rs"]
mod function;
pub use function::{Function, ValueShape};

const MAX_NODES: usize = 4096;
const MAX_PARAMETERS: usize = 64;
static NEXT_PROGRAM: AtomicU64 = AtomicU64::new(1);

mod sealed {
    pub trait Sealed {}
    impl Sealed for f32 {}
    impl Sealed for [f32; 2] {}
    impl Sealed for [f32; 3] {}
    impl Sealed for [f32; 4] {}
}
/// Supported shader numeric types. Implementations are sealed to preserve WGSL validity.
pub trait ShaderType: sealed::Sealed + Copy + 'static {
    /// WGSL type name.
    const WGSL: &'static str;
    /// Number of numeric components.
    const COMPONENTS: usize;
    /// Encode into one aligned uniform slot.
    fn slot(self) -> [f32; 4];
}
impl ShaderType for f32 {
    const WGSL: &'static str = "f32";
    const COMPONENTS: usize = 1;
    fn slot(self) -> [f32; 4] {
        [self, 0., 0., 0.]
    }
}
macro_rules! vector_type {
    ($n:literal, $wgsl:literal) => {
        impl ShaderType for [f32; $n] {
            const WGSL: &'static str = $wgsl;
            const COMPONENTS: usize = $n;
            fn slot(self) -> [f32; 4] {
                let mut slot = [0.; 4];
                slot[..$n].copy_from_slice(&self);
                slot
            }
        }
    };
}
vector_type!(2, "vec2<f32>");
vector_type!(3, "vec3<f32>");
vector_type!(4, "vec4<f32>");

/// Errors found while constructing a program or updating its values.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShaderError {
    /// A value contains NaN or infinity.
    NonFiniteValue,
    /// Expressions or handles belong to different shader builders.
    ForeignGraph,
    /// The graph exceeds its bounded construction budget.
    TooManyNodes,
    /// A program exceeds 64 reachable uniform parameters.
    TooManyParameters,
    /// The active renderer does not support custom shader paint.
    UnsupportedBackend,
    /// Paint geometry contains invalid dimensions, coordinates, or scale.
    InvalidGeometry,
    /// Two imported instances bind different values to the same logical parameter.
    AmbiguousParameter,
    /// A reusable function captures coordinates or uniforms instead of accepting inputs.
    CapturedInput,
    /// A reusable function exceeds 64 input or output leaves.
    TooManyFunctionLeaves,
}
impl fmt::Display for ShaderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NonFiniteValue => "shader values must be finite",
            Self::ForeignGraph => "shader expression or parameter belongs to another program",
            Self::TooManyNodes => "shader construction exceeds 4096 nodes and parameter bindings",
            Self::TooManyParameters => "shader exceeds 64 reachable parameters",
            Self::UnsupportedBackend => "the active renderer does not support custom shader paint",
            Self::CapturedInput => {
                "reusable functions must accept coordinates and parameters as explicit inputs"
            }
            Self::TooManyFunctionLeaves => {
                "reusable functions support at most 64 input and 64 output leaves"
            }
            Self::AmbiguousParameter => {
                "composed shaders bind conflicting values to the same parameter"
            }
            Self::InvalidGeometry => {
                "shader bounds and scale must be finite, with nonnegative size and positive scale"
            }
        })
    }
}
impl std::error::Error for ShaderError {}

/// Shared entry point for constructing paint programs and canvas commands.
/// Clones share a bounded cache of 256 compiled descriptions, including across frames.
/// At capacity, the oldest description without a live shader is evicted first.
/// If all 256 descriptions are live, the oldest is evicted; rebuilding that source
/// can receive a new program identity while existing shader instances remain valid.
#[derive(Clone, Debug)]
pub struct PaintRoot {
    id: u64,
    programs: Arc<Mutex<ProgramCache>>,
}
impl Default for PaintRoot {
    fn default() -> Self {
        Self {
            id: NEXT_PROGRAM.fetch_add(1, Ordering::Relaxed),
            programs: Arc::new(Mutex::new(ProgramCache::default())),
        }
    }
}
const MAX_CACHED_PROGRAMS: usize = 256;
#[derive(Default, Debug)]
struct ProgramCache {
    entries: HashMap<Arc<str>, Arc<Program>>,
    insertion_order: VecDeque<Arc<str>>,
}
impl PaintRoot {
    /// Create a paint composition root.
    pub fn new() -> Self {
        Self::default()
    }
    /// Declare an immutable typed parameter shared by materials under this root.
    /// Each shader or drawing binds its own value; updates do not mutate this default.
    pub fn parameter<T: ShaderType>(&self, value: T) -> Result<Parameter<T>, ShaderError> {
        let initial = value.slot();
        finite(initial)?;
        Ok(Parameter {
            key: ParameterKey {
                owner: self.id,
                index: NEXT_PROGRAM.fetch_add(1, Ordering::Relaxed),
            },
            initial,
            root_scoped: true,
            marker: PhantomData,
        })
    }
    /// Build a reusable fragment program returning straight-alpha RGBA.
    pub fn shader(
        &self,
        build: impl FnOnce(&ShaderBuilder) -> Expr<[f32; 4]>,
    ) -> Result<Shader, ShaderError> {
        let graph = Rc::new(RefCell::new(Graph::new(self.id)));
        let builder = ShaderBuilder {
            graph: graph.clone(),
        };
        let output = build(&builder);
        if !Rc::ptr_eq(&graph, &output.graph) {
            return Err(ShaderError::ForeignGraph);
        }
        self.finish(&graph, output.id)
    }
    /// Compose an existing material with a typed color or coverage transformation.
    /// Existing handles remain valid when their uniforms are reachable in the result.
    pub fn map_shader(
        &self,
        shader: &Shader,
        build: impl FnOnce(&ShaderBuilder, Expr<[f32; 4]>) -> Expr<[f32; 4]>,
    ) -> Result<Shader, ShaderError> {
        self.shader(|builder| {
            let color = builder.sample(shader);
            build(builder, color)
        })
    }

    fn finish(&self, graph: &Rc<RefCell<Graph>>, output: usize) -> Result<Shader, ShaderError> {
        let graph = graph.borrow();
        if let Some(error) = &graph.error {
            return Err(error.clone());
        }
        let order = reachable_order(&graph, &[output]);
        if order
            .iter()
            .any(|id| matches!(graph.nodes[*id].kind, NodeKind::Argument(_)))
        {
            return Err(ShaderError::CapturedInput);
        }
        let numbering: HashMap<_, _> = order
            .iter()
            .enumerate()
            .map(|(new, old)| (*old, new))
            .collect();
        // Logical handles are independent of packing; first-use packing canonicalizes
        // equivalent programs built with a different parameter declaration order.
        let mut slots = Vec::with_capacity(graph.parameters.len());
        let mut packed = vec![usize::MAX; graph.parameters.len()];
        for id in &order {
            if let NodeKind::Uniform(index, _) = graph.nodes[*id].kind {
                if graph.binding_sources[index] == BindingSource::Unresolved {
                    return Err(ShaderError::ForeignGraph);
                }
                if packed[index] == usize::MAX {
                    if slots.len() == MAX_PARAMETERS {
                        return Err(ShaderError::TooManyParameters);
                    }
                    packed[index] = slots.len();
                    slots.push(index);
                }
            }
        }
        let nodes: Vec<_> = order
            .iter()
            .map(|id| {
                let node = &graph.nodes[*id];
                Node {
                    ty: node.ty,
                    kind: match node.kind {
                        NodeKind::Uniform(index, suffix) => {
                            NodeKind::Uniform(packed[index], suffix)
                        }
                        _ => node.kind.clone(),
                    },
                    dependencies: node.dependencies.iter().map(|id| numbering[id]).collect(),
                }
            })
            .collect();
        let output = numbering[&output];
        let count = slots.len().max(1);
        let mut wgsl = format!(
            "struct GpuiPaintParameters {{ values: array<vec4<f32>, {count}>, }};\n@group(1) @binding(1) var<uniform> gpui_paint_parameters: GpuiPaintParameters;\nfn gpui_paint_fragment(uv: vec2<f32>, position: vec2<f32>, size: vec2<f32>) -> vec4<f32> {{\n"
        );
        for (id, node) in nodes.iter().enumerate() {
            wgsl.push_str(&format!(
                "  let n{id}: {} = {};\n",
                node.ty,
                node.expression()
            ));
        }
        wgsl.push_str(&format!("  return n{};\n}}\n", output));
        let mut cache = self
            .programs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let program = if let Some(program) = cache.entries.get(wgsl.as_str()) {
            program.clone()
        } else {
            let wgsl: Arc<str> = wgsl.into();
            let program = Arc::new(Program {
                id: NEXT_PROGRAM.fetch_add(1, Ordering::Relaxed),
                wgsl: wgsl.clone(),
                node_count: nodes.len(),
                requires_derivatives: requires_derivatives(&nodes),
            });
            if cache.entries.len() == MAX_CACHED_PROGRAMS {
                // Prefer descriptions with no live Shader. Capacity stays bounded even
                // when all descriptions are live; in that case the oldest is evicted.
                let candidate = cache
                    .insertion_order
                    .iter()
                    .position(|key| Arc::strong_count(&cache.entries[key]) == 1)
                    .unwrap_or(0);
                if let Some(oldest) = cache.insertion_order.remove(candidate) {
                    cache.entries.remove(&oldest);
                }
            }
            cache.insertion_order.push_back(wgsl.clone());
            cache.entries.insert(wgsl, program.clone());
            program
        };
        Ok(Shader {
            program,
            values: slots
                .iter()
                .map(|index| graph.parameters[*index])
                .collect::<Vec<_>>()
                .into(),
            bindings: slots
                .iter()
                .map(|index| graph.bindings[*index])
                .collect::<Vec<_>>()
                .into(),
            nodes: nodes.into(),
            output,
            root: self.clone(),
        })
    }
}
/// Deterministic dependency-first traversal shared by shaders and reusable functions.
fn reachable_order(graph: &Graph, outputs: &[usize]) -> Vec<usize> {
    let mut seen = HashSet::new();
    let mut pending: Vec<_> = outputs.iter().rev().map(|id| (*id, false)).collect();
    let mut order = Vec::new();
    while let Some((id, expanded)) = pending.pop() {
        if expanded {
            order.push(id);
        } else if seen.insert(id) {
            pending.push((id, true));
            pending.extend(
                graph.nodes[id]
                    .dependencies
                    .iter()
                    .rev()
                    .map(|id| (*id, false)),
            );
        }
    }
    order
}
fn requires_derivatives(nodes: &[Node]) -> bool {
    nodes
        .iter()
        .any(|node| matches!(node.kind, NodeKind::Unary("fwidth")))
}
#[derive(Debug)]
struct Program {
    id: u64,
    wgsl: Arc<str>,
    node_count: usize,
    requires_derivatives: bool,
}
/// An immutable, thread-safe fragment program with its current parameter values.
#[derive(Clone, Debug)]
pub struct Shader {
    program: Arc<Program>,
    values: Arc<[[f32; 4]]>,
    bindings: Arc<[ParameterKey]>,
    nodes: Arc<[Node]>,
    output: usize,
    root: PaintRoot,
}
impl Shader {
    /// Whether the reachable program uses fragment derivatives.
    pub fn requires_derivatives(&self) -> bool {
        self.program.requires_derivatives
    }
    /// Compose a typed transformation using the originating shared paint root.
    pub fn map(
        &self,
        build: impl FnOnce(&ShaderBuilder, Expr<[f32; 4]>) -> Expr<[f32; 4]>,
    ) -> Result<Self, ShaderError> {
        self.root.map_shader(self, build)
    }
    /// Stable identity shared by all parameter permutations of this program.
    pub fn program_id(&self) -> u64 {
        self.program.id
    }
    /// Valid WGSL declarations and `gpui_paint_fragment` function for renderer integration.
    pub fn wgsl(&self) -> &str {
        &self.program.wgsl
    }
    /// Uniform values packed into aligned 16-byte slots in canonical first-use order.
    pub fn parameter_slots(&self) -> &[[f32; 4]] {
        &self.values
    }
    /// Number of reachable nodes emitted after dead-node elimination.
    pub fn node_count(&self) -> usize {
        self.program.node_count
    }
    /// Whether this material contains a binding for the given logical parameter.
    pub fn has_parameter<T: ShaderType>(&self, parameter: Parameter<T>) -> bool {
        self.bindings.contains(&parameter.key)
    }
    /// Return a program-sharing instance with one typed parameter replaced.
    pub fn with_parameter<T: ShaderType>(
        &self,
        parameter: Parameter<T>,
        value: T,
    ) -> Result<Self, ShaderError> {
        let value = value.slot();
        finite(value)?;
        let index = self
            .bindings
            .iter()
            .position(|key| *key == parameter.key)
            .ok_or(ShaderError::ForeignGraph)?;
        if same_slot(&self.values[index], &value) {
            return Ok(self.clone());
        }
        let mut values = self.values.clone();
        Arc::make_mut(&mut values)[index] = value;
        Ok(self.with_values(values))
    }
    /// Update a batch of typed parameters, copying the small value table at most once.
    /// Every key must be reachable in this shader. Repeated keys use the last value; a no-op
    /// shares the original table. Any nonfinite input rejects the complete batch.
    pub fn with_parameters(
        &self,
        build: impl FnOnce(&mut ParameterUpdates),
    ) -> Result<Self, ShaderError> {
        let updates = ParameterUpdates::new(build)?;
        if updates.keys().any(|key| !self.has_parameter_key(key)) {
            return Err(ShaderError::ForeignGraph);
        }
        Ok(self.apply_parameter_updates(&updates))
    }
    pub(super) fn has_parameter_key(&self, key: ParameterKey) -> bool {
        self.bindings.contains(&key)
    }
    pub(super) fn update_identity(&self) -> (u64, usize, usize) {
        (
            self.program_id(),
            self.values.as_ptr() as usize,
            self.bindings.as_ptr() as usize,
        )
    }
    #[cfg(test)]
    pub(super) fn shares_parameter_values(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.values, &other.values)
    }
    pub(super) fn apply_parameter_updates(&self, updates: &ParameterUpdates) -> Self {
        self.with_values(self.updated_parameter_values(updates))
    }
    pub(super) fn updated_parameter_values(&self, updates: &ParameterUpdates) -> Arc<[[f32; 4]]> {
        let changed = self.bindings.iter().enumerate().any(|(index, key)| {
            updates
                .values
                .get(key)
                .is_some_and(|value| !same_slot(value, &self.values[index]))
        });
        if !changed {
            return self.values.clone();
        }
        let mut values = self.values.clone();
        let slots = Arc::make_mut(&mut values);
        for (index, key) in self.bindings.iter().enumerate() {
            if let Some(value) = updates.values.get(key) {
                slots[index] = *value;
            }
        }
        values
    }
    fn with_values(&self, values: Arc<[[f32; 4]]>) -> Self {
        Self {
            program: self.program.clone(),
            values,
            bindings: self.bindings.clone(),
            nodes: self.nodes.clone(),
            output: self.output,
            root: self.root.clone(),
        }
    }
}
impl PartialEq for Shader {
    fn eq(&self, other: &Self) -> bool {
        self.program.id == other.program.id
            && self.values.len() == other.values.len()
            && self
                .values
                .iter()
                .zip(other.values.iter())
                .all(|(left, right)| same_slot(left, right))
    }
}
impl Eq for Shader {}
impl std::hash::Hash for Shader {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.program.id.hash(state);
        for slot in self.values.iter() {
            for value in slot {
                value.to_bits().hash(state);
            }
        }
    }
}
/// A batch of typed, immutable-instance parameter updates.
/// Use the inferred argument of `Shader::with_parameters` or `CanvasDrawing::with_parameters`.
#[derive(Default)]
pub struct ParameterUpdates {
    values: HashMap<ParameterKey, [f32; 4]>,
    error: Option<ShaderError>,
}
impl ParameterUpdates {
    /// Set a typed parameter. Repeated keys use the last value; any nonfinite value
    /// rejects the complete batch even if a subsequent set overwrites that key.
    pub fn set<T: ShaderType>(&mut self, parameter: Parameter<T>, value: T) -> &mut Self {
        let slot = value.slot();
        if let Err(error) = finite(slot) {
            self.error.get_or_insert(error);
        } else {
            self.values.insert(parameter.key, slot);
        }
        self
    }
    pub(super) fn new(build: impl FnOnce(&mut Self)) -> Result<Self, ShaderError> {
        let mut updates = Self::default();
        build(&mut updates);
        if let Some(error) = updates.error {
            return Err(error);
        }
        Ok(updates)
    }
    pub(super) fn keys(&self) -> impl Iterator<Item = ParameterKey> + '_ {
        self.values.keys().copied()
    }
    pub(super) fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}
/// Typed parameter handle; handles can be retained outside the builder closure.
#[derive(Clone, Copy, Debug)]
pub struct Parameter<T: ShaderType> {
    key: ParameterKey,
    initial: [f32; 4],
    root_scoped: bool,
    marker: PhantomData<T>,
}
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub(super) struct ParameterKey {
    owner: u64,
    index: u64,
}
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
struct Node {
    ty: &'static str,
    kind: NodeKind,
    dependencies: Vec<usize>,
}
/// Structured instructions carry dependencies separately from expression emission.
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
enum NodeKind {
    Constant([u32; 4], usize),
    Argument(usize),
    Input(&'static str),
    Uniform(usize, &'static str),
    Unary(&'static str),
    Binary(&'static str),
    Call(&'static str),
    Swizzle(&'static str),
    Construct,
    Negate,
}
impl Node {
    fn expression(&self) -> String {
        let args = || {
            self.dependencies
                .iter()
                .map(|id| format!("n{id}"))
                .collect::<Vec<_>>()
                .join(", ")
        };
        match &self.kind {
            NodeKind::Argument(_) => {
                unreachable!("function arguments must be substituted before shader emission")
            }
            NodeKind::Constant(bits, count) => {
                let components = bits[..*count]
                    .iter()
                    .map(|bits| format!("{:?}", f32::from_bits(*bits)))
                    .collect::<Vec<_>>()
                    .join(", ");
                if *count == 1 {
                    components
                } else {
                    format!("{}({components})", self.ty)
                }
            }
            NodeKind::Input(name) => name.to_string(),
            NodeKind::Uniform(index, suffix) => {
                format!("gpui_paint_parameters.values[{index}]{suffix}")
            }
            NodeKind::Unary(name) | NodeKind::Call(name) => format!("{name}({})", args()),
            NodeKind::Binary(op) => {
                format!("(n{} {op} n{})", self.dependencies[0], self.dependencies[1])
            }
            NodeKind::Swizzle(component) => format!("n{}.{component}", self.dependencies[0]),
            NodeKind::Construct => format!("{}({})", self.ty, args()),
            NodeKind::Negate => format!("(-n{})", self.dependencies[0]),
        }
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum BindingSource {
    Default,
    Instance,
    Unresolved,
}
struct Graph {
    id: u64,
    nodes: Vec<Node>,
    intern: HashMap<Node, usize>,
    parameters: Vec<[f32; 4]>,
    bindings: Vec<ParameterKey>,
    binding_slots: HashMap<ParameterKey, usize>,
    binding_sources: Vec<BindingSource>,
    root_id: u64,
    error: Option<ShaderError>,
    function_calls: HashMap<function::CallKey, Arc<[usize]>>,
}
impl Graph {
    fn new(root_id: u64) -> Self {
        Self {
            root_id,
            id: NEXT_PROGRAM.fetch_add(1, Ordering::Relaxed),
            nodes: vec![],
            intern: HashMap::new(),
            parameters: vec![],
            bindings: vec![],
            binding_slots: HashMap::new(),
            binding_sources: vec![],
            error: None,
            function_calls: HashMap::new(),
        }
    }
    fn fail(&mut self, error: ShaderError) {
        if self.error.is_none() {
            self.error = Some(error);
        }
    }
    fn bind(&mut self, key: ParameterKey, value: [f32; 4], source: BindingSource) -> usize {
        if let Err(error) = finite(value) {
            self.fail(error);
        }
        if let Some(index) = self.binding_slots.get(&key).copied() {
            if source == BindingSource::Instance {
                if self.binding_sources[index] == BindingSource::Instance
                    && !same_slot(&self.parameters[index], &value)
                {
                    self.fail(ShaderError::AmbiguousParameter);
                } else {
                    self.parameters[index] = value;
                    self.binding_sources[index] = BindingSource::Instance;
                }
            }
            return index;
        }
        if self.parameters.len() + self.nodes.len() >= MAX_NODES {
            self.fail(ShaderError::TooManyNodes);
            return 0;
        }
        let index = self.parameters.len();
        self.parameters.push(value);
        self.bindings.push(key);
        self.binding_slots.insert(key, index);
        self.binding_sources.push(source);
        index
    }
    fn push(&mut self, node: Node) -> usize {
        if let Some(id) = self.intern.get(&node) {
            return *id;
        }
        if self.nodes.len() + self.parameters.len() >= MAX_NODES {
            self.fail(ShaderError::TooManyNodes);
            return 0;
        }
        let id = self.nodes.len();
        self.nodes.push(node.clone());
        self.intern.insert(node, id);
        id
    }
}
fn same_slot(left: &[f32; 4], right: &[f32; 4]) -> bool {
    left.iter()
        .zip(right)
        .all(|(left, right)| left.to_bits() == right.to_bits())
}
fn finite(slot: [f32; 4]) -> Result<(), ShaderError> {
    if slot.iter().all(|v| v.is_finite()) {
        Ok(())
    } else {
        Err(ShaderError::NonFiniteValue)
    }
}
/// A typed value in a fragment graph. Clone reuses the same node.
#[derive(Clone)]
pub struct Expr<T: ShaderType> {
    graph: Rc<RefCell<Graph>>,
    id: usize,
    marker: PhantomData<T>,
}
impl<T: ShaderType> Expr<T> {
    fn node(graph: &Rc<RefCell<Graph>>, kind: NodeKind, dependencies: Vec<usize>) -> Self {
        let id = graph.borrow_mut().push(Node {
            ty: T::WGSL,
            kind,
            dependencies,
        });
        Self {
            graph: graph.clone(),
            id,
            marker: PhantomData,
        }
    }
    fn combine<U: ShaderType>(&self, other: &Expr<U>) {
        if !Rc::ptr_eq(&self.graph, &other.graph) {
            self.graph.borrow_mut().fail(ShaderError::ForeignGraph);
        }
    }
    fn unary(&self, name: &'static str) -> Self {
        Self::node(&self.graph, NodeKind::Unary(name), vec![self.id])
    }
    fn binary(&self, other: Self, op: &'static str) -> Self {
        self.combine(&other);
        Self::node(&self.graph, NodeKind::Binary(op), vec![self.id, other.id])
    }
    fn call(&self, other: Self, name: &'static str) -> Self {
        self.combine(&other);
        Self::node(&self.graph, NodeKind::Call(name), vec![self.id, other.id])
    }
    /// Component-wise sine.
    pub fn sin(self) -> Self {
        self.unary("sin")
    }
    /// Component-wise cosine.
    pub fn cos(self) -> Self {
        self.unary("cos")
    }
    /// Component-wise absolute value.
    pub fn abs(self) -> Self {
        self.unary("abs")
    }
    /// Component-wise floor.
    pub fn floor(self) -> Self {
        self.unary("floor")
    }
    /// Component-wise fractional part.
    pub fn fract(self) -> Self {
        self.unary("fract")
    }
    /// Component-wise minimum.
    pub fn min(self, other: Self) -> Self {
        self.call(other, "min")
    }
    /// Component-wise maximum.
    pub fn max(self, other: Self) -> Self {
        self.call(other, "max")
    }
    /// Restrict each component to an interval.
    pub fn clamp(self, low: Self, high: Self) -> Self {
        self.combine(&low);
        self.combine(&high);
        Self::node(
            &self.graph,
            NodeKind::Call("clamp"),
            vec![self.id, low.id, high.id],
        )
    }
    /// Interpolate with a scalar blend factor.
    pub fn mix(self, other: Self, factor: Expr<f32>) -> Self {
        self.combine(&other);
        self.combine(&factor);
        Self::node(
            &self.graph,
            NodeKind::Call("mix"),
            vec![self.id, other.id, factor.id],
        )
    }
    /// Multiply by a scalar.
    pub fn scale(self, factor: Expr<f32>) -> Self {
        self.combine(&factor);
        Self::node(&self.graph, NodeKind::Binary("*"), vec![self.id, factor.id])
    }
}
macro_rules! arithmetic {
    ($trait:ident,$method:ident,$op:literal) => {
        impl<T: ShaderType> $trait for Expr<T> {
            type Output = Self;
            fn $method(self, rhs: Self) -> Self {
                self.binary(rhs, $op)
            }
        }
    };
}
arithmetic!(Add, add, "+");
arithmetic!(Sub, sub, "-");
arithmetic!(Mul, mul, "*");
arithmetic!(Div, div, "/");
impl<T: ShaderType> Neg for Expr<T> {
    type Output = Self;
    fn neg(self) -> Self {
        Self::node(&self.graph, NodeKind::Negate, vec![self.id])
    }
}
/// A scalar condition, distinct from numeric expressions.
///
/// ```compile_fail
/// use gpui::paint::PaintRoot;
/// PaintRoot::new().shader(|cx| {
///     let condition = cx.scalar(1.0).gt(cx.scalar(0.0));
///     let invalid = condition + cx.scalar(1.0);
///     cx.vec4([1.0; 4])
/// });
/// ```
#[derive(Clone)]
pub struct Predicate {
    graph: Rc<RefCell<Graph>>,
    id: usize,
}
impl Predicate {
    /// Choose a typed value. Both branches must belong to this condition's graph.
    pub fn select<T: ShaderType>(self, when_true: Expr<T>, when_false: Expr<T>) -> Expr<T> {
        for branch in [&when_true, &when_false] {
            if !Rc::ptr_eq(&self.graph, &branch.graph) {
                self.graph.borrow_mut().fail(ShaderError::ForeignGraph);
            }
        }
        Expr::node(
            &self.graph,
            NodeKind::Call("select"),
            vec![when_false.id, when_true.id, self.id],
        )
    }
}
impl Expr<f32> {
    /// Compare two scalar values, producing a condition for typed selection.
    pub fn gt(self, other: Self) -> Predicate {
        self.combine(&other);
        let id = self.graph.borrow_mut().push(Node {
            ty: "bool",
            kind: NodeKind::Binary(">"),
            dependencies: vec![self.id, other.id],
        });
        Predicate {
            graph: self.graph,
            id,
        }
    }
    /// Smooth interpolation between the given edges.
    pub fn smoothstep(self, low: Self, high: Self) -> Self {
        self.combine(&low);
        self.combine(&high);
        Self::node(
            &self.graph,
            NodeKind::Call("smoothstep"),
            vec![low.id, high.id, self.id],
        )
    }
    /// Antialiased fill coverage for a signed distance, with positive logical-pixel feather.
    pub fn fill(self, feather: Self) -> Self {
        let graph = self.graph.clone();
        let zero = Expr::node(
            &graph,
            NodeKind::Constant([0.0_f32.to_bits(), 0, 0, 0], 1),
            vec![],
        );
        let one = Expr::node(
            &graph,
            NodeKind::Constant([1.0_f32.to_bits(), 0, 0, 0], 1),
            vec![],
        );
        one - self.smoothstep(zero, feather)
    }
    /// Screen-space rate of change, useful for scale-independent antialiasing.
    pub fn fwidth(self) -> Self {
        self.unary("fwidth")
    }
    /// Centered antialiased signed-distance coverage using fragment derivatives.
    pub fn coverage(self) -> Self {
        let graph = self.graph.clone();
        let epsilon = Expr::node(
            &graph,
            NodeKind::Constant([0.0001_f32.to_bits(), 0, 0, 0], 1),
            vec![],
        );
        let half = Expr::node(
            &graph,
            NodeKind::Constant([0.5_f32.to_bits(), 0, 0, 0], 1),
            vec![],
        );
        let one = Expr::node(
            &graph,
            NodeKind::Constant([1.0_f32.to_bits(), 0, 0, 0], 1),
            vec![],
        );
        let width = self.clone().fwidth().max(epsilon);
        let edge = width * half;
        one - self.smoothstep(-edge.clone(), edge)
    }
    /// Antialiased centered stroke coverage; width is a half-width in logical pixels.
    pub fn stroke(self, half_width: Self, feather: Self) -> Self {
        (self.abs() - half_width).fill(feather)
    }
}
macro_rules! swizzles {
    ($n:literal; $($name:ident),+) => {
        impl Expr<[f32; $n]> {
            $(
                #[doc = concat!("Extract the `", stringify!($name), "` component.")]
                pub fn $name(self) -> Expr<f32> {
                    Expr::node(&self.graph, NodeKind::Swizzle(stringify!($name)), vec![self.id])
                }
            )+
            /// Euclidean vector length.
            pub fn length(self) -> Expr<f32> {
                Expr::node(&self.graph, NodeKind::Unary("length"), vec![self.id])
            }
            /// Dot product.
            pub fn dot(self, other: Self) -> Expr<f32> {
                self.combine(&other);
                Expr::node(&self.graph, NodeKind::Call("dot"), vec![self.id, other.id])
            }
        }
    };
}
swizzles!(2; x, y);
swizzles!(3; x, y, z);
swizzles!(4; x, y, z, w);
impl Expr<[f32; 4]> {
    /// Multiply alpha by coverage while preserving straight RGB.
    pub fn mask(self, coverage: Expr<f32>) -> Self {
        let builder = ShaderBuilder {
            graph: self.graph.clone(),
        };
        builder.rgba(
            self.clone().x(),
            self.clone().y(),
            self.clone().z(),
            self.w() * coverage,
        )
    }
}
/// Construction context for a single shader graph.
pub struct ShaderBuilder {
    graph: Rc<RefCell<Graph>>,
}
impl ShaderBuilder {
    /// Normalized coordinates across the painted bounds.
    pub fn uv(&self) -> Expr<[f32; 2]> {
        Expr::node(&self.graph, NodeKind::Input("uv"), vec![])
    }
    /// Local coordinates in logical pixels.
    pub fn position(&self) -> Expr<[f32; 2]> {
        Expr::node(&self.graph, NodeKind::Input("position"), vec![])
    }
    /// Painted bounds size in logical pixels.
    pub fn size(&self) -> Expr<[f32; 2]> {
        Expr::node(&self.graph, NodeKind::Input("size"), vec![])
    }
    /// Construct a finite scalar or vector constant.
    pub fn constant<T: ShaderType>(&self, value: T) -> Expr<T> {
        let slot = value.slot();
        if let Err(error) = finite(slot) {
            self.graph.borrow_mut().fail(error);
        }
        let bits = slot.map(|v| if v.is_finite() { v.to_bits() } else { 0 });
        Expr::node(&self.graph, NodeKind::Constant(bits, T::COMPONENTS), vec![])
    }
    /// Construct a scalar constant.
    pub fn scalar(&self, value: f32) -> Expr<f32> {
        self.constant(value)
    }
    /// Construct a two-component vector constant.
    pub fn vec2(&self, value: [f32; 2]) -> Expr<[f32; 2]> {
        self.constant(value)
    }
    /// Construct a three-component vector constant.
    pub fn vec3(&self, value: [f32; 3]) -> Expr<[f32; 3]> {
        self.constant(value)
    }
    /// Construct a four-component vector constant.
    pub fn vec4(&self, value: [f32; 4]) -> Expr<[f32; 4]> {
        self.constant(value)
    }
    /// Compose an RGBA vector from four scalar expressions.
    pub fn rgba(&self, r: Expr<f32>, g: Expr<f32>, b: Expr<f32>, a: Expr<f32>) -> Expr<[f32; 4]> {
        for expr in [&r, &g, &b, &a] {
            if !Rc::ptr_eq(&self.graph, &expr.graph) {
                self.graph.borrow_mut().fail(ShaderError::ForeignGraph);
            }
        }
        Expr::node(
            &self.graph,
            NodeKind::Construct,
            vec![r.id, g.id, b.id, a.id],
        )
    }
    /// Compose a two-component vector from scalar expressions.
    pub fn xy(&self, x: Expr<f32>, y: Expr<f32>) -> Expr<[f32; 2]> {
        for expr in [&x, &y] {
            if !Rc::ptr_eq(&self.graph, &expr.graph) {
                self.graph.borrow_mut().fail(ShaderError::ForeignGraph);
            }
        }
        Expr::node(&self.graph, NodeKind::Construct, vec![x.id, y.id])
    }
    /// Allocate a typed uniform handle.
    /// Only reachable uniforms survive compilation and count toward the 64-slot limit.
    /// Updating a handle unused by the finished material returns `ForeignGraph`.
    pub fn parameter<T: ShaderType>(&self, value: T) -> Parameter<T> {
        let mut graph = self.graph.borrow_mut();
        let slot = value.slot();
        if let Err(error) = finite(slot) {
            graph.fail(error);
        }
        let key = ParameterKey {
            owner: graph.id,
            index: graph.parameters.len() as u64,
        };
        graph.bind(key, slot, BindingSource::Instance);
        Parameter {
            key,
            initial: slot,
            root_scoped: false,
            marker: PhantomData,
        }
    }
    /// Read a typed parameter. Root-declared parameters are packed lazily on first use.
    /// A sampled material supplies its current value; the root default is a fallback.
    /// This precedence is independent of whether `uniform` or `sample` is called first.
    pub fn uniform<T: ShaderType>(&self, parameter: Parameter<T>) -> Expr<T> {
        let index = {
            let mut graph = self.graph.borrow_mut();
            if let Some(index) = graph.binding_slots.get(&parameter.key).copied() {
                index
            } else {
                let source = if parameter.root_scoped && parameter.key.owner == graph.root_id {
                    BindingSource::Default
                } else {
                    BindingSource::Unresolved
                };
                graph.bind(parameter.key, parameter.initial, source)
            }
        };
        let suffix = match T::COMPONENTS {
            1 => ".x",
            2 => ".xy",
            3 => ".xyz",
            _ => "",
        };
        Expr::node(&self.graph, NodeKind::Uniform(index, suffix), vec![])
    }
    /// Compose a completed material in the current coordinate domain.
    /// Shared logical parameters reuse a slot; conflicting instance values return
    /// `AmbiguousParameter` when the enclosing shader is completed.
    pub fn sample(&self, shader: &Shader) -> Expr<[f32; 4]> {
        self.import(shader, None, None, None)
    }
    /// Compose a material with substituted UV, keeping current logical position and size.
    pub fn sample_uv(&self, shader: &Shader, uv: Expr<[f32; 2]>) -> Expr<[f32; 4]> {
        self.import(shader, Some(uv), None, None)
    }
    /// Compose a material in an explicit local domain, deriving UV as position / size.
    /// Size must be nonzero for well-defined normalized coordinates.
    pub fn sample_at(
        &self,
        shader: &Shader,
        position: Expr<[f32; 2]>,
        size: Expr<[f32; 2]>,
    ) -> Expr<[f32; 4]> {
        let uv = position.clone() / size.clone();
        self.import(shader, Some(uv), Some(position), Some(size))
    }
    fn import(
        &self,
        shader: &Shader,
        uv: Option<Expr<[f32; 2]>>,
        position: Option<Expr<[f32; 2]>>,
        size: Option<Expr<[f32; 2]>>,
    ) -> Expr<[f32; 4]> {
        for expr in [&uv, &position, &size].into_iter().flatten() {
            if !Rc::ptr_eq(&self.graph, &expr.graph) {
                self.graph.borrow_mut().fail(ShaderError::ForeignGraph);
            }
        }
        let mut graph = self.graph.borrow_mut();
        let slots: Vec<_> = shader
            .bindings
            .iter()
            .zip(shader.values.iter())
            .map(|(key, value)| graph.bind(*key, *value, BindingSource::Instance))
            .collect();
        let mut nodes = Vec::with_capacity(shader.nodes.len());
        for node in shader.nodes.iter() {
            let substitute = match &node.kind {
                NodeKind::Input("uv") => uv.as_ref(),
                NodeKind::Input("position") => position.as_ref(),
                NodeKind::Input("size") => size.as_ref(),
                _ => None,
            };
            let id = if let Some(expr) = substitute {
                expr.id
            } else {
                let kind = match node.kind {
                    NodeKind::Uniform(index, suffix) => NodeKind::Uniform(slots[index], suffix),
                    _ => node.kind.clone(),
                };
                graph.push(Node {
                    ty: node.ty,
                    kind,
                    dependencies: node.dependencies.iter().map(|id| nodes[*id]).collect(),
                })
            };
            nodes.push(id);
        }
        Expr {
            graph: self.graph.clone(),
            id: nodes[shader.output],
            marker: PhantomData,
        }
    }
    /// Circle signed distance from a local point to its center and radius.
    pub fn circle(
        &self,
        point: Expr<[f32; 2]>,
        center: Expr<[f32; 2]>,
        radius: Expr<f32>,
    ) -> Expr<f32> {
        (point - center).length() - radius
    }
    /// Rounded rectangle signed distance. Half-size and radius use logical pixels.
    pub fn rounded_rect(
        &self,
        point: Expr<[f32; 2]>,
        center: Expr<[f32; 2]>,
        half_size: Expr<[f32; 2]>,
        radius: Expr<f32>,
    ) -> Expr<f32> {
        let radius2 = self.xy(radius.clone(), radius.clone());
        let q = (point - center).abs() - half_size + radius2;
        q.clone().max(self.vec2([0., 0.])).length() + q.clone().x().max(q.y()).min(self.scalar(0.))
            - radius
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parameters_share_program_and_reject_foreign_or_nonfinite_values() {
        let mut handle = None;
        let shader = PaintRoot::new()
            .shader(|cx| {
                let p = cx.parameter([1., 0., 0., 1.]);
                handle = Some(p);
                cx.uniform(p)
            })
            .unwrap();
        let changed = shader
            .with_parameter(handle.unwrap(), [0., 1., 0., 1.])
            .unwrap();
        assert_eq!(shader.program_id(), changed.program_id());
        assert_eq!(shader.wgsl(), changed.wgsl());
        assert_ne!(shader, changed);
        assert_eq!(changed.parameter_slots(), &[[0., 1., 0., 1.]]);
        assert_eq!(
            shader
                .with_parameter(handle.unwrap(), [f32::NAN, 0., 0., 1.])
                .unwrap_err(),
            ShaderError::NonFiniteValue
        );
        let other = PaintRoot::new().shader(|cx| cx.vec4([1.; 4])).unwrap();
        assert_eq!(
            other.with_parameter(handle.unwrap(), [1.; 4]).unwrap_err(),
            ShaderError::ForeignGraph
        );
    }
    #[test]
    fn shared_nodes_and_dead_nodes_are_eliminated() {
        let shader = PaintRoot::new()
            .shader(|cx| {
                let _unused = cx.scalar(345.);
                let mut x = cx.uv().x();
                for _ in 0..12 {
                    x = x.clone() + x;
                }
                cx.rgba(x.clone(), x.clone(), x, cx.scalar(1.))
            })
            .unwrap();
        assert_eq!(shader.node_count(), 16);
        assert!(!shader.wgsl().contains("345"));
        assert!(shader.wgsl().len() < 2000);
    }
    #[test]
    fn construction_errors_do_not_panic() {
        assert_eq!(
            PaintRoot::new()
                .shader(|cx| cx.vec4([f32::INFINITY; 4]))
                .unwrap_err(),
            ShaderError::NonFiniteValue
        );
        let first = ShaderBuilder {
            graph: Rc::new(RefCell::new(Graph::new(0))),
        };
        let foreign = first.scalar(1.);
        assert_eq!(
            PaintRoot::new()
                .shader(|cx| cx.rgba(foreign, cx.scalar(0.), cx.scalar(0.), cx.scalar(1.)))
                .unwrap_err(),
            ShaderError::ForeignGraph
        );
        assert_eq!(
            PaintRoot::new()
                .shader(|cx| {
                    let mut sum = cx.scalar(0.);
                    for _ in 0..65 {
                        let p = cx.parameter(0.);
                        sum = sum + cx.uniform(p);
                    }
                    cx.rgba(sum.clone(), sum.clone(), sum, cx.scalar(1.))
                })
                .unwrap_err(),
            ShaderError::TooManyParameters
        );
    }
    #[test]
    fn root_interns_programs_but_keeps_handles_private() {
        let root = PaintRoot::new();
        let mut first_handle = None;
        let first = root
            .shader(|cx| {
                let p = cx.parameter([1.; 4]);
                first_handle = Some(p);
                cx.uniform(p)
            })
            .unwrap();
        let cloned_root = root.clone();
        let second = cloned_root
            .shader(|cx| {
                let _dead = cx.scalar(99.);
                let p = cx.parameter([0.; 4]);
                cx.uniform(p)
            })
            .unwrap();
        assert_eq!(first.program_id(), second.program_id());
        drop(cloned_root);
        let third = root.shader(|cx| cx.vec4([0.; 4])).unwrap();
        assert_ne!(first.program_id(), third.program_id());
        assert_eq!(
            second
                .with_parameter(first_handle.unwrap(), [1.; 4])
                .unwrap_err(),
            ShaderError::ForeignGraph
        );
    }
    #[test]
    fn mapping_preserves_parameter_values_and_handles() {
        let root = PaintRoot::new();
        let mut original = None;
        let mut added = None;
        let first = root
            .shader(|cx| {
                let p = cx.parameter([1.; 4]);
                original = Some(p);
                cx.uniform(p)
            })
            .unwrap();
        let mapped = first
            .map(|cx, color| {
                let p = cx.parameter(0.5);
                added = Some(p);
                color.mask(cx.uniform(p))
            })
            .unwrap();
        let changed = mapped
            .with_parameter(original.unwrap(), [0., 1., 0., 1.])
            .unwrap()
            .with_parameter(added.unwrap(), 0.25)
            .unwrap();
        assert_eq!(mapped.program_id(), changed.program_id());
        assert_eq!(
            changed.parameter_slots(),
            &[[0., 1., 0., 1.], [0.25, 0., 0., 0.]]
        );
        assert_eq!(
            first.with_parameter(added.unwrap(), 1.).unwrap_err(),
            ShaderError::ForeignGraph
        );
        assert!(changed.wgsl().contains(".w"));
    }
    #[test]
    fn node_budget_is_checked_before_compilation() {
        let result = PaintRoot::new().shader(|cx| {
            let mut x = cx.scalar(0.);
            for _ in 0..MAX_NODES {
                x = x.sin();
            }
            cx.rgba(x.clone(), x.clone(), x, cx.scalar(1.))
        });
        assert_eq!(result.unwrap_err(), ShaderError::TooManyNodes);
    }
    #[test]
    fn canonical_numbering_ignores_independent_construction_order() {
        let root = PaintRoot::new();
        let a = root
            .shader(|cx| {
                let r = cx.scalar(0.2);
                let g = cx.scalar(0.3);
                cx.rgba(r, g, cx.scalar(0.4), cx.scalar(1.))
            })
            .unwrap();
        let b = root
            .shader(|cx| {
                let g = cx.scalar(0.3);
                let r = cx.scalar(0.2);
                cx.rgba(r, g, cx.scalar(0.4), cx.scalar(1.))
            })
            .unwrap();
        assert_eq!(a.program_id(), b.program_id());
    }
    #[test]
    fn mapping_does_not_import_dead_nodes_or_change_identity_when_unchanged() {
        let root = PaintRoot::new();
        let shader = root
            .shader(|cx| {
                let mut dead = cx.scalar(0.);
                for _ in 0..MAX_NODES - 2 {
                    dead = dead.sin();
                }
                cx.vec4([1.; 4])
            })
            .unwrap();
        assert_eq!(shader.node_count(), 1);
        let mapped = shader.map(|cx, color| color.mask(cx.scalar(0.5))).unwrap();
        assert!(mapped.node_count() < 10);
        assert_eq!(
            shader.map(|_, color| color).unwrap().program_id(),
            shader.program_id()
        );
    }
    #[test]
    fn cache_survives_dropped_instances_and_remains_bounded() {
        let root = PaintRoot::new();
        let id = root.shader(|cx| cx.vec4([1.; 4])).unwrap().program_id();
        assert_eq!(root.shader(|cx| cx.vec4([1.; 4])).unwrap().program_id(), id);
        for n in 0..MAX_CACHED_PROGRAMS + 1 {
            root.shader(|cx| cx.vec4([n as f32; 4])).unwrap();
        }
        assert_eq!(
            root.programs.lock().unwrap().entries.len(),
            MAX_CACHED_PROGRAMS
        );
    }
    #[test]
    fn root_parameters_are_ergonomic_shared_and_private_to_root() {
        let root = PaintRoot::new();
        let phase = root.parameter(0.25).unwrap();
        let a = root
            .shader(|cx| cx.vec4([1.; 4]).mask(cx.uniform(phase)))
            .unwrap();
        let b = root
            .shader(|cx| cx.vec4([0.; 4]).mask(cx.uniform(phase)))
            .unwrap();
        assert!(a.has_parameter(phase));
        assert!(b.has_parameter(phase));
        assert_eq!(
            a.with_parameter(phase, 0.75).unwrap().parameter_slots(),
            &[[0.75, 0., 0., 0.]]
        );
        assert_eq!(b.parameter_slots(), &[[0.25, 0., 0., 0.]]);
        assert_eq!(
            PaintRoot::new()
                .shader(|cx| cx.vec4([1.; 4]).mask(cx.uniform(phase)))
                .unwrap_err(),
            ShaderError::ForeignGraph
        );
        assert_eq!(
            root.parameter(f32::NAN).unwrap_err(),
            ShaderError::NonFiniteValue
        );
    }
    #[test]
    fn independent_material_slots_are_remapped_and_original_handles_survive() {
        let root = PaintRoot::new();
        let mut left = None;
        let mut right = None;
        let a = root
            .shader(|cx| {
                let p = cx.parameter([1., 0., 0., 1.]);
                left = Some(p);
                cx.uniform(p)
            })
            .unwrap();
        let b = root
            .shader(|cx| {
                let p = cx.parameter([0., 0., 1., 1.]);
                right = Some(p);
                cx.uniform(p)
            })
            .unwrap();
        let mixed = root
            .shader(|cx| cx.sample(&a).mix(cx.sample(&b), cx.scalar(0.5)))
            .unwrap();
        assert_eq!(mixed.parameter_slots().len(), 2);
        let changed = mixed
            .with_parameter(right.unwrap(), [0., 1., 0., 1.])
            .unwrap()
            .with_parameter(left.unwrap(), [1., 1., 0., 1.])
            .unwrap();
        assert_eq!(
            changed.parameter_slots(),
            &[[1., 1., 0., 1.], [0., 1., 0., 1.]]
        );
        assert_eq!(mixed.program_id(), changed.program_id());
    }
    #[test]
    fn duplicate_samples_share_nodes_and_parameter_bindings() {
        let root = PaintRoot::new();
        let p = root.parameter([0.2, 0.3, 0.4, 1.]).unwrap();
        let shader = root.shader(|cx| cx.uniform(p)).unwrap();
        let repeated = root
            .shader(|cx| {
                let mut color = cx.sample(&shader);
                for _ in 0..12 {
                    color = color + cx.sample(&shader);
                }
                color
            })
            .unwrap();
        assert_eq!(repeated.parameter_slots().len(), 1);
        assert_eq!(repeated.node_count(), 13);
        assert!(repeated.wgsl().len() < 2000);
    }
    #[test]
    fn conflicting_instances_are_explicit_but_existing_binding_reads_current_value() {
        let root = PaintRoot::new();
        let p = root.parameter([1.; 4]).unwrap();
        let shader = root.shader(|cx| cx.uniform(p)).unwrap();
        let changed = shader.with_parameter(p, [0.; 4]).unwrap();
        assert_eq!(
            root.shader(|cx| cx.sample(&shader) + cx.sample(&changed))
                .unwrap_err(),
            ShaderError::AmbiguousParameter
        );
        let read = root
            .shader(|cx| cx.sample(&changed) + cx.uniform(p))
            .unwrap();
        assert_eq!(read.parameter_slots(), &[[0.; 4]]);
        let reverse = root
            .shader(|cx| cx.uniform(p) + cx.sample(&changed))
            .unwrap();
        assert_eq!(reverse.parameter_slots(), &[[0.; 4]]);
        assert_eq!(read.program_id(), reverse.program_id());
    }
    #[test]
    fn material_domains_substitute_only_requested_inputs() {
        let root = PaintRoot::new();
        let material = root
            .shader(|cx| cx.rgba(cx.uv().x(), cx.position().y(), cx.size().x(), cx.scalar(1.)))
            .unwrap();
        let uv = root
            .shader(|cx| cx.sample_uv(&material, cx.vec2([0.25, 0.5])))
            .unwrap();
        assert!(!uv.wgsl().contains("= uv;"));
        assert!(uv.wgsl().contains("= position;"));
        assert!(uv.wgsl().contains("= size;"));
        let at = root
            .shader(|cx| cx.sample_at(&material, cx.vec2([4., 8.]), cx.vec2([16., 32.])))
            .unwrap();
        assert!(!at.wgsl().contains("= uv;"));
        assert!(!at.wgsl().contains("= position;"));
        assert!(!at.wgsl().contains("= size;"));
        assert!(at.wgsl().contains(" / "));
    }
    #[test]
    fn foreign_domain_expressions_are_rejected() {
        let root = PaintRoot::new();
        let material = root.shader(|cx| cx.vec4([1.; 4])).unwrap();
        let other = ShaderBuilder {
            graph: Rc::new(RefCell::new(Graph::new(0))),
        };
        assert_eq!(
            root.shader(|cx| cx.sample_uv(&material, other.uv()))
                .unwrap_err(),
            ShaderError::ForeignGraph
        );
    }
    #[test]
    fn canonical_uniform_packing_ignores_handle_declaration_order() {
        let root = PaintRoot::new();
        let mut ar = None;
        let mut ag = None;
        let mut br = None;
        let mut bg = None;
        let a = root
            .shader(|cx| {
                let r = cx.parameter(0.2);
                let g = cx.parameter(0.3);
                ar = Some(r);
                ag = Some(g);
                cx.rgba(cx.uniform(r), cx.uniform(g), cx.scalar(0.), cx.scalar(1.))
            })
            .unwrap();
        let b = root
            .shader(|cx| {
                let g = cx.parameter(0.3);
                let r = cx.parameter(0.2);
                br = Some(r);
                bg = Some(g);
                cx.rgba(cx.uniform(r), cx.uniform(g), cx.scalar(0.), cx.scalar(1.))
            })
            .unwrap();
        assert_eq!(a.program_id(), b.program_id());
        assert_eq!(a.parameter_slots(), b.parameter_slots());
        assert_eq!(
            a.with_parameter(ar.unwrap(), 0.7)
                .unwrap()
                .parameter_slots()[0][0],
            0.7
        );
        assert_eq!(
            b.with_parameter(br.unwrap(), 0.7)
                .unwrap()
                .parameter_slots()[0][0],
            0.7
        );
        assert_eq!(
            a.with_parameter(ag.unwrap(), 0.8)
                .unwrap()
                .parameter_slots()[1][0],
            0.8
        );
        assert_eq!(
            b.with_parameter(bg.unwrap(), 0.8)
                .unwrap()
                .parameter_slots()[1][0],
            0.8
        );
    }
    #[test]
    fn predicate_select_is_typed_and_compacts_its_boolean_dependency() {
        let root = PaintRoot::new();
        let width = root.parameter(0.).unwrap();
        let shader = root
            .shader(|cx| {
                cx.uniform(width)
                    .gt(cx.scalar(0.))
                    .select(cx.vec4([1.; 4]), cx.vec4([0.; 4]))
            })
            .unwrap();
        assert!(shader.wgsl().contains(": bool = ("));
        assert!(shader.wgsl().contains("select("));
        assert!(shader.has_parameter(width));
        assert_eq!(
            shader.with_parameter(width, 2.).unwrap().program_id(),
            shader.program_id()
        );
    }
    #[test]
    fn predicate_selection_rejects_foreign_branches_and_comparisons() {
        let root = PaintRoot::new();
        let other = ShaderBuilder {
            graph: Rc::new(RefCell::new(Graph::new(0))),
        };
        assert_eq!(
            root.shader(|cx| cx
                .scalar(1.)
                .gt(cx.scalar(0.))
                .select(other.vec4([1.; 4]), cx.vec4([0.; 4])))
                .unwrap_err(),
            ShaderError::ForeignGraph
        );
        assert_eq!(
            root.shader(|cx| cx
                .scalar(1.)
                .gt(other.scalar(0.))
                .select(cx.vec4([1.; 4]), cx.vec4([0.; 4])))
                .unwrap_err(),
            ShaderError::ForeignGraph
        );
    }
    #[test]
    fn batched_updates_are_atomic_last_wins_and_noops_share_the_table() {
        let root = PaintRoot::new();
        let tint = root.parameter([1.; 4]).unwrap();
        let alpha = root.parameter(0.5).unwrap();
        let shader = root
            .shader(|cx| cx.uniform(tint).mask(cx.uniform(alpha)))
            .unwrap();
        let updated = shader
            .with_parameters(|p| {
                p.set(tint, [0., 1., 0., 1.]);
                p.set(alpha, 0.2);
                p.set(alpha, 0.8);
            })
            .unwrap();
        assert_eq!(
            updated.parameter_slots(),
            &[[0., 1., 0., 1.], [0.8, 0., 0., 0.]]
        );
        assert_eq!(updated.program_id(), shader.program_id());
        assert!(
            shader
                .with_parameters(|_| {})
                .unwrap()
                .shares_parameter_values(&shader)
        );
        assert!(
            shader
                .with_parameter(alpha, 0.5)
                .unwrap()
                .shares_parameter_values(&shader)
        );
        let foreign = root.parameter(0.).unwrap();
        assert_eq!(
            shader
                .with_parameters(|p| {
                    p.set(alpha, 0.8);
                    p.set(foreign, 1.);
                })
                .unwrap_err(),
            ShaderError::ForeignGraph
        );
        assert_eq!(
            shader
                .with_parameters(|p| {
                    p.set(alpha, f32::NAN);
                    p.set(alpha, 0.8);
                })
                .unwrap_err(),
            ShaderError::NonFiniteValue
        );
        assert_eq!(shader.parameter_slots(), &[[1.; 4], [0.5, 0., 0., 0.]]);
    }
    #[test]
    fn signed_zero_is_not_a_noop_or_an_equal_bound_instance() {
        let root = PaintRoot::new();
        let p = root.parameter(0.).unwrap();
        let shader = root
            .shader(|cx| cx.vec4([1.; 4]).mask(cx.uniform(p)))
            .unwrap();
        let negative = shader.with_parameter(p, -0.).unwrap();
        assert_eq!(
            negative.parameter_slots()[0][0].to_bits(),
            (-0.0_f32).to_bits()
        );
        assert!(!negative.shares_parameter_values(&shader));
        assert_ne!(negative, shader);
        assert_eq!(
            root.shader(|cx| cx.sample(&shader) + cx.sample(&negative))
                .unwrap_err(),
            ShaderError::AmbiguousParameter
        );
    }
    #[test]
    fn imported_foreign_root_bindings_resolve_independent_of_read_order() {
        let owner = PaintRoot::new();
        let receiver = PaintRoot::new();
        let p = owner.parameter([1.; 4]).unwrap();
        let material = owner
            .shader(|cx| cx.uniform(p))
            .unwrap()
            .with_parameter(p, [0.2, 0.3, 0.4, 1.])
            .unwrap();
        let before = receiver
            .shader(|cx| {
                let value = cx.uniform(p);
                value + cx.sample(&material)
            })
            .unwrap();
        let after = receiver
            .shader(|cx| {
                let material = cx.sample(&material);
                cx.uniform(p) + material
            })
            .unwrap();
        assert_eq!(before, after);
        assert_eq!(before.parameter_slots(), material.parameter_slots());
        assert_eq!(
            receiver.shader(|cx| cx.uniform(p)).unwrap_err(),
            ShaderError::ForeignGraph
        );
        let mut local = None;
        let local_material = owner
            .shader(|cx| {
                let p = cx.parameter([1.; 4]);
                local = Some(p);
                cx.uniform(p)
            })
            .unwrap();
        let local_material = local_material
            .with_parameter(local.unwrap(), [0.7, 0.5, 0.2, 1.])
            .unwrap();
        let before = receiver
            .shader(|cx| {
                let value = cx.uniform(local.unwrap());
                value + cx.sample(&local_material)
            })
            .unwrap();
        let after = receiver
            .shader(|cx| {
                let material = cx.sample(&local_material);
                cx.uniform(local.unwrap()) + material
            })
            .unwrap();
        assert_eq!(before, after);
        assert_eq!(before.parameter_slots(), local_material.parameter_slots());
        assert_eq!(
            receiver
                .shader(|cx| cx.uniform(local.unwrap()))
                .unwrap_err(),
            ShaderError::ForeignGraph
        );
    }
    #[test]
    fn dead_uniforms_and_materials_have_no_finished_binding_or_layout_cost() {
        let root = PaintRoot::new();
        let mut dead_handle = None;
        let dead = root
            .shader(|cx| {
                let p = cx.parameter(0.5);
                dead_handle = Some(p);
                cx.uniform(p);
                cx.vec4([1.; 4])
            })
            .unwrap();
        let plain = root.shader(|cx| cx.vec4([1.; 4])).unwrap();
        assert_eq!(dead, plain);
        assert!(dead.parameter_slots().is_empty());
        assert_eq!(
            dead.with_parameter(dead_handle.unwrap(), 0.7).unwrap_err(),
            ShaderError::ForeignGraph
        );
        let full = root
            .shader(|cx| {
                let mut sum = cx.scalar(0.);
                for _ in 0..64 {
                    let p = cx.parameter(0.1);
                    sum = sum + cx.uniform(p);
                }
                cx.rgba(sum.clone(), sum.clone(), sum, cx.scalar(1.))
            })
            .unwrap();
        assert_eq!(full.parameter_slots().len(), 64);
        let p = root.parameter([0.2, 0.3, 0.4, 1.]).unwrap();
        let pruned = root
            .shader(|cx| {
                cx.sample(&full);
                cx.uniform(p)
            })
            .unwrap();
        assert_eq!(pruned.parameter_slots().len(), 1);
        assert!(pruned.has_parameter(p));
        assert_eq!(
            pruned.program_id(),
            root.shader(|cx| cx.uniform(p)).unwrap().program_id()
        );
        assert_eq!(
            root.shader(|cx| {
                for _ in 0..MAX_NODES {
                    cx.parameter(0.);
                }
                cx.vec4([1.; 4])
            })
            .unwrap_err(),
            ShaderError::TooManyNodes
        );
    }
    #[test]
    fn cache_prefers_inactive_descriptions_and_shares_its_source_allocation() {
        let root = PaintRoot::new();
        let hot = root.shader(|cx| cx.vec4([-1.; 4])).unwrap();
        for n in 0..MAX_CACHED_PROGRAMS + 2 {
            root.shader(|cx| cx.vec4([n as f32; 4])).unwrap();
        }
        assert_eq!(
            root.shader(|cx| cx.vec4([-1.; 4])).unwrap().program_id(),
            hot.program_id()
        );
        let cache = root.programs.lock().unwrap();
        let (key, program) = cache.entries.get_key_value(hot.wgsl()).unwrap();
        assert!(Arc::ptr_eq(key, &program.wgsl));
        assert!(
            cache
                .insertion_order
                .iter()
                .any(|queued| Arc::ptr_eq(queued, key))
        );
        assert_eq!(cache.entries.len(), MAX_CACHED_PROGRAMS);
        drop(cache);
        drop(hot);
        root.shader(|cx| cx.vec4([999.; 4])).unwrap();
        assert!(
            !root
                .programs
                .lock()
                .unwrap()
                .entries
                .keys()
                .any(|source| source.contains("-1.0"))
        );
        let all_live: Vec<_> = (0..MAX_CACHED_PROGRAMS + 2)
            .map(|n| root.shader(|cx| cx.vec4([1000. + n as f32; 4])).unwrap())
            .collect();
        assert_eq!(
            root.programs.lock().unwrap().entries.len(),
            MAX_CACHED_PROGRAMS
        );
        assert_eq!(all_live.len(), MAX_CACHED_PROGRAMS + 2);
    }
    #[test]
    fn shader_is_send_sync() {
        fn check<T: Send + Sync>() {}
        check::<Shader>();
        check::<PaintRoot>();
        check::<Parameter<[f32; 4]>>();
    }
}
