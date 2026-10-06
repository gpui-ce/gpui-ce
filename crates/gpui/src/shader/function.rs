//! Foreign shader code — WGSL files, GLSL, or `#[wgsl]` modules — as typed
//! functions that compose with expressions.
//!
//! Every source is parsed by Naga, checked, and re-emitted as WGSL with each
//! of its declarations prefixed by the source's hash. Foreign code therefore
//! never collides with GPUI's shaders, other sources, or itself.

use std::{
    fmt,
    hash::{Hash, Hasher},
    marker::PhantomData,
    sync::{Arc, Mutex, OnceLock},
};

use collections::FxHashMap;
use smallvec::{SmallVec, smallvec};
use wgsl_rs::{ir, std::Vec4f};

use super::{
    Expr, Operand, Paint,
    compile::{ShaderError, prelude_source, validate_module},
    expr::{Node, Op},
    value::{Ty, Val, Value, sealed::Sealed},
};

/// Declarations available to GLSL sources, mirroring the prelude's `Fragment`.
const GLSL_PRELUDE: &str =
    "struct Fragment { vec2 uv; vec2 position; vec2 size; vec2 origin; float scale; };\n";

pub type CpuEval = Box<dyn Fn(&[Val]) -> Val + Send + Sync>;

/// One function from a foreign source.
pub(crate) struct Foreign {
    /// Identity of this function within its source.
    pub(crate) id: u64,
    /// Identity of the namespaced source text.
    pub(crate) chunk_id: u64,
    pub(crate) chunk: Arc<str>,
    pub(crate) name: String,
    fragment: String,
    params: SmallVec<[Ty; 6]>,
    ret: Ty,
    pub(crate) cpu: Option<CpuEval>,
}

impl Foreign {
    /// Re-express GPUI's fragment as this source's copy of the struct.
    pub(crate) fn convert_fragment(&self, fragment: &ir::Expr) -> ir::Expr {
        let field = |name: &str| ir::Expr::FieldAccess {
            base: Box::new(fragment.clone()),
            field: name.into(),
        };
        ir::Expr::FnCall {
            path: ir::FnPath::Ident(self.fragment.clone()),
            type_args: vec![],
            params: ["uv", "position", "size", "origin", "scale"]
                .map(field)
                .into(),
        }
    }

    fn call<T: Value>(self: &Arc<Self>, args: SmallVec<[Arc<Node>; 3]>) -> Expr<T> {
        let found: SmallVec<[Ty; 6]> = args.iter().map(|arg| arg.ty).collect();
        if found != self.params || T::TY != self.ret {
            let message = format!(
                "`{}` takes {:?} and returns {:?}, but was called with {found:?} for {:?}",
                self.name,
                self.params,
                self.ret,
                T::TY
            );
            return Expr::from_node(Node::invalid(T::TY, message));
        }
        Expr::make(Op::Foreign(self.clone()), args)
    }
}

/// A parsed, namespaced foreign source. Look up functions and paint entry
/// points by name. Cloning is cheap, and identical sources are parsed once.
#[derive(Clone)]
pub struct Library(pub(crate) Arc<LibraryInner>);

pub(crate) struct LibraryInner {
    pub(crate) chunk_id: u64,
    pub(crate) chunk: Arc<str>,
    pub(crate) prefix: String,
    /// The namespaced module, for reflection.
    pub(crate) module: naga::Module,
    signatures: FxHashMap<String, Result<(SmallVec<[Ty; 6]>, Ty), String>>,
}

impl fmt::Debug for Library {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Library")
            .field("functions", &self.0.signatures.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

#[derive(Hash)]
enum Language {
    Wgsl,
    Glsl,
    /// WGSL assembled by `wgsl_rs`, which imports the prelude itself if needed.
    Module,
}

impl Library {
    /// Parse WGSL. The [prelude](super::prelude) (`Fragment`, `Color`, ...) is
    /// in scope. Resource bindings and pipeline overrides are rejected.
    pub fn wgsl(source: &str) -> Result<Self, ShaderError> {
        Self::load(Language::Wgsl, source)
    }

    /// Parse GLSL (version 450) functions. A `Fragment` struct mirroring the
    /// prelude's is in scope.
    pub fn glsl(source: &str) -> Result<Self, ShaderError> {
        Self::load(Language::Glsl, source)
    }

    /// Use a `wgsl_rs` `#[wgsl]` module.
    pub fn module(source: &'static wgsl_rs::Source) -> Result<Self, ShaderError> {
        let text = source
            .wgsl_source()
            .map_err(|error| ShaderError::Source(format!("{error:?}")))?;
        Self::load(Language::Module, &text)
    }

    fn load(language: Language, source: &str) -> Result<Self, ShaderError> {
        let mut hasher = collections::FxHasher::default();
        language.hash(&mut hasher);
        source.hash(&mut hasher);
        let chunk_id = hasher.finish();

        static LIBRARIES: OnceLock<Mutex<FxHashMap<u64, Library>>> = OnceLock::new();
        let libraries = LIBRARIES.get_or_init(Default::default);
        if let Some(library) = libraries
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&chunk_id)
        {
            return Ok(library.clone());
        }
        let library = Self::parse(language, source, chunk_id)?;
        libraries
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(chunk_id, library.clone());
        Ok(library)
    }

    fn parse(language: Language, source: &str, chunk_id: u64) -> Result<Self, ShaderError> {
        let mut module = match language {
            Language::Wgsl => validate_module(&format!("{}\n{source}", prelude_source()))?.0,
            Language::Module => validate_module(source)?.0,
            Language::Glsl => {
                let source = format!("#version 450\n{GLSL_PRELUDE}{source}\nvoid main() {{}}\n");
                naga::front::glsl::Frontend::default()
                    .parse(&naga::ShaderStage::Fragment.into(), &source)
                    .map_err(|error| ShaderError::Source(error.emit_to_string(&source)))?
            }
        };

        if let Some((_, global)) = module
            .global_variables
            .iter()
            .find(|(_, global)| global.binding.is_some())
        {
            return Err(ShaderError::Source(format!(
                "foreign shader code cannot declare resource bindings (found `{}`); pass values as arguments",
                global.name.as_deref().unwrap_or("?"),
            )));
        }
        if !module.overrides.is_empty() {
            return Err(ShaderError::Source(
                "foreign shader code cannot declare pipeline overrides".into(),
            ));
        }

        // Namespace every declaration, then re-emit WGSL.
        let prefix = format!("x{chunk_id:016x}_");
        let rename = |name: &mut Option<String>| {
            if let Some(name) = name {
                *name = format!("{prefix}{name}");
            }
        };
        module.entry_points.clear();
        for (_, function) in module.functions.iter_mut() {
            rename(&mut function.name);
        }
        for (_, constant) in module.constants.iter_mut() {
            rename(&mut constant.name);
        }
        for (_, global) in module.global_variables.iter_mut() {
            rename(&mut global.name);
        }
        let structs: Vec<_> = module
            .types
            .iter()
            .filter(|(_, ty)| ty.name.is_some())
            .map(|(handle, ty)| (handle, ty.clone()))
            .collect();
        for (handle, mut ty) in structs {
            rename(&mut ty.name);
            module.types.replace(handle, ty);
        }
        let info = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::empty(),
        )
        .validate(&module)
        .map_err(|error| ShaderError::Source(format!("{error:?}")))?;
        let chunk =
            naga::back::wgsl::write_string(&module, &info, naga::back::wgsl::WriterFlags::empty())
                .map_err(|error| ShaderError::Source(error.to_string()))?;

        let fragment = format!("{prefix}Fragment");
        let signatures = module
            .functions
            .iter()
            .filter_map(|(_, function)| {
                let name = function.name.as_deref()?.strip_prefix(&prefix)?.to_owned();
                let params = function
                    .arguments
                    .iter()
                    .map(|argument| Ty::from_naga(&module, argument.ty, &fragment))
                    .collect::<Option<SmallVec<_>>>();
                let ret = function
                    .result
                    .as_ref()
                    .and_then(|result| Ty::from_naga(&module, result.ty, &fragment));
                let signature = params.zip(ret).ok_or_else(|| {
                    format!("`{name}` uses types other than f32, bool, vecN<f32>, and Fragment")
                });
                Some((name, signature))
            })
            .collect();

        Ok(Self(Arc::new(LibraryInner {
            chunk_id,
            chunk: chunk.into(),
            prefix,
            module,
            signatures,
        })))
    }

    fn foreign(&self, name: &str, cpu: Option<CpuEval>) -> Result<Arc<Foreign>, ShaderError> {
        let (params, ret) = self
            .0
            .signatures
            .get(name)
            .ok_or_else(|| {
                let mut known: Vec<_> = self.0.signatures.keys().collect();
                known.sort();
                ShaderError::Source(format!("no function `{name}`; found {known:?}"))
            })?
            .clone()
            .map_err(ShaderError::Source)?;
        let mut hasher = collections::FxHasher::default();
        (self.0.chunk_id, name).hash(&mut hasher);
        Ok(Arc::new(Foreign {
            id: hasher.finish(),
            chunk_id: self.0.chunk_id,
            chunk: self.0.chunk.clone(),
            name: format!("{}{name}", self.0.prefix),
            fragment: format!("{}Fragment", self.0.prefix),
            params,
            ret,
            cpu,
        }))
    }

    /// A typed function from this source.
    pub fn function<S: Signature>(&self, name: &str) -> Result<Function<S>, ShaderError> {
        Function::checked(self.foreign(name, None)?)
    }

    /// A paint entry point: a function taking a `Fragment` first, then any
    /// parameters, and returning straight-alpha `vec4<f32>`.
    pub fn shader(&self, name: &str) -> Result<Shader, ShaderError> {
        let foreign = self.foreign(name, None)?;
        if foreign.params.first() != Some(&Ty::Fragment) || foreign.ret != Ty::Vec4 {
            return Err(ShaderError::Source(format!(
                "`{name}` must take a `Fragment` first and return `vec4<f32>`"
            )));
        }
        Ok(Shader(foreign))
    }
}

mod seal {
    use super::*;

    pub trait Signature {
        fn params() -> SmallVec<[Ty; 6]>;
    }

    pub trait Nodes {
        fn into_nodes(self) -> SmallVec<[Arc<Node>; 3]>;
    }

    pub trait Eval<S> {
        fn into_eval(self) -> CpuEval;
    }
}

/// A shader function signature, written as a function pointer type such as
/// `fn(Vec2f, f32) -> f32`.
pub trait Signature: seal::Signature + 'static {
    /// The return type.
    type Output: Value;
}

/// Arguments for a [`Function`] with signature `S`: a single operand, or a
/// tuple of operands.
pub trait Arguments<S: Signature>: seal::Nodes {}

/// A Rust implementation of a shader function, used to evaluate it on the CPU.
pub trait CpuFunction<S: Signature>: seal::Eval<S> + Send + Sync + 'static {}

impl<S: Signature, F: seal::Eval<S> + Send + Sync + 'static> CpuFunction<S> for F {}

/// Arguments for a [`Shader`]: `()`, one operand, or a tuple of operands.
pub trait ShaderArguments: seal::Nodes {}

impl seal::Nodes for () {
    fn into_nodes(self) -> SmallVec<[Arc<Node>; 3]> {
        SmallVec::new()
    }
}

impl ShaderArguments for () {}

impl<X: Operand> seal::Nodes for X {
    fn into_nodes(self) -> SmallVec<[Arc<Node>; 3]> {
        smallvec![self.into_expr().node]
    }
}

impl<X: Operand> ShaderArguments for X {}

impl<A: Value, R: Value, X: Operand<Value = A>> Arguments<fn(A) -> R> for X {}

macro_rules! arities {
    ($(($($arg:ident $operand:ident $index:tt),*))*) => {$(
        impl<$($arg: Value,)* R: Value> seal::Signature for fn($($arg),*) -> R {
            fn params() -> SmallVec<[Ty; 6]> {
                smallvec![$($arg::TY),*]
            }
        }

        impl<$($arg: Value,)* R: Value> Signature for fn($($arg),*) -> R {
            type Output = R;
        }

        impl<F, $($arg: Value,)* R: Value> seal::Eval<fn($($arg),*) -> R> for F
        where
            F: Fn($($arg),*) -> R + Send + Sync + 'static,
        {
            #[allow(unused_variables)]
            fn into_eval(self) -> CpuEval {
                Box::new(move |args: &[Val]| self($($arg::from_val(args[$index])),*).into_val())
            }
        }

        impl<$($arg: Value, $operand: Operand<Value = $arg>,)* R: Value>
            Arguments<fn($($arg),*) -> R> for ($($operand,)*)
        {
        }
    )*};
}

arities! {
    ()
    (A XA 0)
    (A XA 0, B XB 1)
    (A XA 0, B XB 1, C XC 2)
    (A XA 0, B XB 1, C XC 2, D XD 3)
    (A XA 0, B XB 1, C XC 2, D XD 3, E XE 4)
    (A XA 0, B XB 1, C XC 2, D XD 3, E XE 4, G XG 5)
}

macro_rules! shader_tuples {
    ($(($($operand:ident $index:tt),*))*) => {$(
        impl<$($operand: Operand),*> seal::Nodes for ($($operand,)*) {
            fn into_nodes(self) -> SmallVec<[Arc<Node>; 3]> {
                smallvec![$(self.$index.into_expr().node),*]
            }
        }

        impl<$($operand: Operand),*> ShaderArguments for ($($operand,)*) {}
    )*};
}

shader_tuples! {
    (XA 0)
    (XA 0, XB 1)
    (XA 0, XB 1, XC 2)
    (XA 0, XB 1, XC 2, XD 3)
    (XA 0, XB 1, XC 2, XD 3, XE 4)
    (XA 0, XB 1, XC 2, XD 3, XE 4, XF 5)
    (XA 0, XB 1, XC 2, XD 3, XE 4, XF 5, XG 6)
    (XA 0, XB 1, XC 2, XD 3, XE 4, XF 5, XG 6, XH 7)
}

/// A typed foreign shader function, callable in expressions.
///
/// ```ignore
/// let ripple: Function<fn(Vec2f, f32) -> f32> =
///     Function::wgsl(include_str!("ripple.wgsl"), "ripple")?;
/// let paint = paint(|px| color(blue).opacity(ripple.call((px.uv(), time))));
/// ```
pub struct Function<S: Signature> {
    foreign: Arc<Foreign>,
    signature: PhantomData<fn() -> S>,
}

impl<S: Signature> Clone for Function<S> {
    fn clone(&self) -> Self {
        Self {
            foreign: self.foreign.clone(),
            signature: PhantomData,
        }
    }
}

impl<S: Signature> fmt::Debug for Function<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Function({})", self.foreign.name)
    }
}

impl<S: Signature> Function<S> {
    fn checked(foreign: Arc<Foreign>) -> Result<Self, ShaderError> {
        if foreign.params != S::params() || foreign.ret != S::Output::TY {
            return Err(ShaderError::Source(format!(
                "`{}` takes {:?} and returns {:?}, not {:?} -> {:?}",
                foreign.name,
                foreign.params,
                foreign.ret,
                S::params(),
                S::Output::TY,
            )));
        }
        Ok(Self {
            foreign,
            signature: PhantomData,
        })
    }

    /// A function from WGSL source; see [`Library::wgsl`].
    pub fn wgsl(source: &str, name: &str) -> Result<Self, ShaderError> {
        Library::wgsl(source)?.function(name)
    }

    /// A function from GLSL source; see [`Library::glsl`].
    pub fn glsl(source: &str, name: &str) -> Result<Self, ShaderError> {
        Library::glsl(source)?.function(name)
    }

    /// A function from a `#[wgsl]` module, with its Rust twin for CPU
    /// evaluation. [`wgsl_fn!`](crate::wgsl_fn) fills these in from a path.
    pub fn module(
        source: &'static wgsl_rs::Source,
        name: &str,
        cpu: impl CpuFunction<S>,
    ) -> Result<Self, ShaderError> {
        Self::checked(Library::module(source)?.foreign(name, Some(cpu.into_eval()))?)
    }

    /// Call this function per fragment.
    pub fn call(&self, args: impl Arguments<S>) -> Expr<S::Output> {
        self.foreign.call(args.into_nodes())
    }
}

/// A function from a `#[wgsl]` module as a typed, CPU-evaluable [`Function`]:
/// `wgsl_fn!(effects::ripple)`.
#[macro_export]
macro_rules! wgsl_fn {
    ($($path:ident)::+) => {
        $crate::__wgsl_fn!([] $($path)::+)
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __wgsl_fn {
    ([$($module:ident)*] $name:ident) => {
        $crate::shader::Function::module(
            &$($module::)* WGSL_SOURCE,
            stringify!($name),
            $($module::)* $name,
        )
    };
    ([$($module:ident)*] $head:ident :: $($rest:tt)+) => {
        $crate::__wgsl_fn!([$($module)* $head] $($rest)+)
    };
}

/// A paint written as a foreign shader function: a traditional `.wgsl` (or
/// GLSL) file defining `fn paint(fragment: Fragment, ...) -> vec4<f32>`.
///
/// ```ignore
/// // ripple.wgsl:
/// //   fn paint(fragment: Fragment, center: vec2<f32>, time: f32) -> vec4<f32> { ... }
/// let ripple = Shader::wgsl(include_str!("ripple.wgsl"))?;
/// div().bg(ripple.with((center, time)).over(rgb(0x101318)))
/// ```
#[derive(Clone)]
pub struct Shader(Arc<Foreign>);

impl fmt::Debug for Shader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Shader({})", self.0.name)
    }
}

impl Shader {
    /// The `paint` entry point of WGSL source.
    pub fn wgsl(source: &str) -> Result<Self, ShaderError> {
        Library::wgsl(source)?.shader("paint")
    }

    /// The `paint` entry point of GLSL source.
    pub fn glsl(source: &str) -> Result<Self, ShaderError> {
        Library::glsl(source)?.shader("paint")
    }

    /// The `paint` entry point of a `#[wgsl]` module.
    pub fn module(source: &'static wgsl_rs::Source) -> Result<Self, ShaderError> {
        Library::module(source)?.shader("paint")
    }

    /// A paint evaluating this shader with the given parameters. Each may be
    /// a CPU value (a uniform) or an expression varying per fragment.
    /// Mismatched parameter types surface when the paint compiles.
    pub fn with(&self, args: impl ShaderArguments) -> Paint {
        let mut nodes = args.into_nodes();
        nodes.insert(0, Node::fragment());
        Paint::straight(self.0.call::<Vec4f>(nodes))
    }
}
