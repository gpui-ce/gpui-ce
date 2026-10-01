//! Capture-free typed functions inline into the same paint graph as handwritten expressions.

use super::*;

mod sealed {
    use super::*;

    pub struct Leaf {
        pub(super) graph: Rc<RefCell<Graph>>,
        pub(super) id: usize,
    }
    pub struct Context<'a> {
        pub(super) graph: &'a Rc<RefCell<Graph>>,
        pub(super) ids: &'a [usize],
        pub(super) next: usize,
    }
    impl Context<'_> {
        pub(super) fn leaf(&mut self) -> Leaf {
            let id = self.ids[self.next];
            self.next += 1;
            Leaf {
                graph: self.graph.clone(),
                id,
            }
        }
    }
    pub trait Sealed {
        fn types(types: &mut Vec<&'static str>);
        fn restore(context: &mut Context<'_>) -> <Self as ValueShape>::Expr
        where
            Self: ValueShape;
        fn leaves(value: &<Self as ValueShape>::Expr, leaves: &mut Vec<Leaf>)
        where
            Self: ValueShape;
    }
}
use sealed::{Context, Leaf, Sealed};

/// A sealed input or output schema for a reusable paint function.
/// Numeric leaves, semantic distances and coverage, predicates, tuples of two to
/// four shapes, and unit all use the existing builder vocabulary.
pub trait ValueShape: Sealed + 'static {
    /// The corresponding expression, semantic wrapper, or tuple of expressions.
    type Expr;
    /// Number of flattened leaves, bounded to 64 when constructing a function.
    const LEAF_COUNT: usize;
}

macro_rules! numeric_shape {
    ($($ty:ty),+) => {
        $(
            impl ValueShape for $ty {
                type Expr = Expr<$ty>;
                const LEAF_COUNT: usize = 1;
            }

            impl Sealed for $ty {
                fn types(types: &mut Vec<&'static str>) {
                    types.push(<$ty as ShaderType>::WGSL);
                }

                fn restore(context: &mut Context<'_>) -> Expr<$ty> {
                    let leaf = context.leaf();
                    Expr {
                        graph: leaf.graph,
                        id: leaf.id,
                        marker: PhantomData,
                    }
                }

                fn leaves(value: &Expr<$ty>, leaves: &mut Vec<Leaf>) {
                    leaves.push(Leaf {
                        graph: value.graph.clone(),
                        id: value.id,
                    });
                }
            }
        )+
    };
}
numeric_shape!(f32, [f32; 2], [f32; 3], [f32; 4]);

macro_rules! semantic_shape {
    ($ty:ident) => {
        impl ValueShape for $ty {
            type Expr = Self;
            const LEAF_COUNT: usize = 1;
        }
        impl Sealed for $ty {
            fn types(types: &mut Vec<&'static str>) {
                types.push("f32");
            }
            fn restore(context: &mut Context<'_>) -> Self {
                Self(<f32 as Sealed>::restore(context))
            }
            fn leaves(value: &Self, leaves: &mut Vec<Leaf>) {
                <f32 as Sealed>::leaves(&value.expression(), leaves);
            }
        }
    };
}
semantic_shape!(Distance);
semantic_shape!(Coverage);
impl ValueShape for Predicate {
    type Expr = Self;
    const LEAF_COUNT: usize = 1;
}
impl Sealed for Predicate {
    fn types(types: &mut Vec<&'static str>) {
        types.push("bool");
    }
    fn restore(context: &mut Context<'_>) -> Self {
        let leaf = context.leaf();
        Self {
            graph: leaf.graph,
            id: leaf.id,
        }
    }
    fn leaves(value: &Self, leaves: &mut Vec<Leaf>) {
        leaves.push(Leaf {
            graph: value.graph.clone(),
            id: value.id,
        });
    }
}
impl ValueShape for () {
    type Expr = ();
    const LEAF_COUNT: usize = 0;
}
impl Sealed for () {
    fn types(_: &mut Vec<&'static str>) {}
    fn restore(_: &mut Context<'_>) {}
    fn leaves(_: &(), _: &mut Vec<Leaf>) {}
}
macro_rules! tuple_shape {
    ($($ty:ident: $index:tt),+) => {
        impl<$($ty: ValueShape),+> ValueShape for ($($ty,)+) {
            type Expr = ($($ty::Expr,)+);
            const LEAF_COUNT: usize = 0usize$(.saturating_add($ty::LEAF_COUNT))+;
        }

        impl<$($ty: ValueShape),+> Sealed for ($($ty,)+) {
            fn types(types: &mut Vec<&'static str>) {
                $(
                    $ty::types(types);
                )+
            }

            fn restore(context: &mut Context<'_>) -> <Self as ValueShape>::Expr {
                ($($ty::restore(context),)+)
            }

            fn leaves(value: &<Self as ValueShape>::Expr, leaves: &mut Vec<Leaf>) {
                $(
                    $ty::leaves(&value.$index, leaves);
                )+
            }
        }
    };
}
tuple_shape!(A: 0, B: 1);
tuple_shape!(A: 0, B: 1, C: 2);
tuple_shape!(A: 0, B: 1, C: 2, D: 3);

const MAX_MEMOIZED_CALLS: usize = 128;
#[derive(Clone, Hash, PartialEq, Eq)]
pub(super) struct CallKey {
    function: u64,
    arguments: Box<[usize]>,
}

struct FunctionGraph {
    id: u64,
    nodes: Arc<[Node]>,
    outputs: Arc<[usize]>,
    requires_derivatives: bool,
}

/// An immutable, portable, capture-free expression function.
/// Calling a function substitutes its arguments into the existing DAG, so repeated
/// expressions share nodes and no extra GPU function or pipeline is introduced.
///
/// ```
/// use gpui::paint::PaintRoot;
/// let root = PaintRoot::new();
/// let wave = root.function::<(f32, [f32; 2]), f32>(|_, (phase, uv)| {
///     (uv.x() + phase).sin()
/// }).unwrap();
/// let phase = root.parameter(0.0).unwrap();
/// let shader = root.shader(|cx| {
///     let red = cx.call(&wave, (cx.uniform(phase), cx.uv()));
///     cx.rgba(red, cx.scalar(0.2), cx.scalar(0.8), cx.scalar(1.0))
/// }).unwrap();
/// ```
///
/// ```compile_fail
/// use gpui::paint::PaintRoot;
/// let root = PaintRoot::new();
/// let square = root.function::<f32, f32>(|_, x| x.clone() * x).unwrap();
/// root.shader(|cx| {
///     let invalid = cx.call(&square, cx.vec2([1.0, 2.0]));
///     cx.vec4([1.0; 4])
/// });
/// ```
pub struct Function<I: ValueShape, O: ValueShape> {
    graph: Arc<FunctionGraph>,
    marker: PhantomData<fn(I) -> O>,
}
impl<I: ValueShape, O: ValueShape> fmt::Debug for Function<I, O> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Function")
            .field("node_count", &self.node_count())
            .field("requires_derivatives", &self.requires_derivatives())
            .finish()
    }
}
impl<I: ValueShape, O: ValueShape> Clone for Function<I, O> {
    fn clone(&self) -> Self {
        Self {
            graph: self.graph.clone(),
            marker: PhantomData,
        }
    }
}
impl<I: ValueShape, O: ValueShape> Function<I, O> {
    /// Whether any reachable output uses fragment derivatives.
    pub fn requires_derivatives(&self) -> bool {
        self.graph.requires_derivatives
    }
    /// Number of shared instructions, including referenced argument leaves.
    pub fn node_count(&self) -> usize {
        self.graph.nodes.len()
    }
}

impl PaintRoot {
    /// Build a typed function with explicit inputs and no ambient coordinate or
    /// parameter captures. Dead reads are ignored; construction errors still fail.
    pub fn function<I: ValueShape, O: ValueShape>(
        &self,
        build: impl FnOnce(&ShaderBuilder, I::Expr) -> O::Expr,
    ) -> Result<Function<I, O>, ShaderError> {
        if I::LEAF_COUNT > 64 || O::LEAF_COUNT > 64 {
            return Err(ShaderError::TooManyFunctionLeaves);
        }
        let graph = Rc::new(RefCell::new(Graph::new(self.id)));
        let builder = ShaderBuilder {
            graph: graph.clone(),
        };
        let mut types = Vec::with_capacity(I::LEAF_COUNT);
        I::types(&mut types);
        let ids: Vec<_> = types
            .into_iter()
            .enumerate()
            .map(|(index, ty)| {
                graph.borrow_mut().push(Node {
                    ty,
                    kind: NodeKind::Argument(index),
                    dependencies: vec![],
                })
            })
            .collect();
        let input = I::restore(&mut Context {
            graph: &graph,
            ids: &ids,
            next: 0,
        });
        let output = build(&builder, input);
        let mut leaves = Vec::with_capacity(O::LEAF_COUNT);
        O::leaves(&output, &mut leaves);
        if leaves.iter().any(|leaf| !Rc::ptr_eq(&leaf.graph, &graph)) {
            return Err(ShaderError::ForeignGraph);
        }
        let graph = graph.borrow();
        if let Some(error) = &graph.error {
            return Err(error.clone());
        }
        let outputs: Vec<_> = leaves.iter().map(|leaf| leaf.id).collect();
        let order = reachable_order(&graph, &outputs);
        if order.iter().any(|id| {
            matches!(
                graph.nodes[*id].kind,
                NodeKind::Input(_) | NodeKind::Uniform(..)
            )
        }) {
            return Err(ShaderError::CapturedInput);
        }
        let numbering: HashMap<_, _> = order
            .iter()
            .enumerate()
            .map(|(new, old)| (*old, new))
            .collect();
        let nodes: Vec<_> = order
            .iter()
            .map(|id| {
                let node = &graph.nodes[*id];
                Node {
                    ty: node.ty,
                    kind: node.kind.clone(),
                    dependencies: node.dependencies.iter().map(|id| numbering[id]).collect(),
                }
            })
            .collect();
        Ok(Function {
            graph: Arc::new(FunctionGraph {
                id: NEXT_PROGRAM.fetch_add(1, Ordering::Relaxed),
                requires_derivatives: requires_derivatives(&nodes),
                nodes: nodes.into(),
                outputs: outputs
                    .iter()
                    .map(|id| numbering[id])
                    .collect::<Vec<_>>()
                    .into(),
            }),
            marker: PhantomData,
        })
    }
}
impl ShaderBuilder {
    /// Call a typed reusable function using values in this builder's graph.
    /// The first 128 distinct nonempty argument-bearing calls are memoized per builder;
    /// other calls still reuse instructions through the same DAG interner.
    pub fn call<I: ValueShape, O: ValueShape>(
        &self,
        function: &Function<I, O>,
        input: I::Expr,
    ) -> O::Expr {
        let mut leaves = Vec::with_capacity(I::LEAF_COUNT);
        I::leaves(&input, &mut leaves);
        if leaves
            .iter()
            .any(|leaf| !Rc::ptr_eq(&leaf.graph, &self.graph))
        {
            self.graph.borrow_mut().fail(ShaderError::ForeignGraph);
        }
        let key = if !leaves.is_empty() && !function.graph.outputs.is_empty() {
            Some(CallKey {
                function: function.graph.id,
                arguments: leaves.iter().map(|leaf| leaf.id).collect(),
            })
        } else {
            None
        };
        if let Some(outputs) = key
            .as_ref()
            .and_then(|key| self.graph.borrow().function_calls.get(key).cloned())
        {
            return O::restore(&mut Context {
                graph: &self.graph,
                ids: &outputs,
                next: 0,
            });
        }
        let mut graph = self.graph.borrow_mut();
        let mut nodes = Vec::with_capacity(function.graph.nodes.len());
        for node in function.graph.nodes.iter() {
            let id = if let NodeKind::Argument(index) = node.kind {
                leaves[index].id
            } else {
                graph.push(Node {
                    ty: node.ty,
                    kind: node.kind.clone(),
                    dependencies: node.dependencies.iter().map(|id| nodes[*id]).collect(),
                })
            };
            nodes.push(id);
        }
        let outputs: Arc<[usize]> = function
            .graph
            .outputs
            .iter()
            .map(|id| nodes[*id])
            .collect::<Vec<_>>()
            .into();
        if graph.function_calls.len() < MAX_MEMOIZED_CALLS {
            if let Some(key) = key {
                graph.function_calls.insert(key, outputs.clone());
            }
        }
        drop(graph);
        O::restore(&mut Context {
            graph: &self.graph,
            ids: &outputs,
            next: 0,
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn portable_calls_inline_identically_and_distinct_arguments_stay_distinct() {
        let root = PaintRoot::new();
        let square = root.function::<f32, f32>(|_, x| x.clone() * x).unwrap();
        let other = PaintRoot::new();
        let composed = other
            .shader(|cx| {
                cx.rgba(
                    cx.call(&square, cx.scalar(0.2)),
                    cx.call(&square, cx.scalar(0.3)),
                    cx.scalar(0.),
                    cx.scalar(1.),
                )
            })
            .unwrap();
        let handwritten = other
            .shader(|cx| {
                let r = cx.scalar(0.2);
                let g = cx.scalar(0.3);
                cx.rgba(r.clone() * r, g.clone() * g, cx.scalar(0.), cx.scalar(1.))
            })
            .unwrap();
        assert_eq!(composed.program_id(), handwritten.program_id());
        assert_eq!(composed.wgsl(), handwritten.wgsl());
        assert_eq!(composed.node_count(), handwritten.node_count());
    }
    #[test]
    fn nested_multioutput_functions_share_work_and_unused_derivatives_are_pruned() {
        let root = PaintRoot::new();
        let shared = root
            .function::<f32, (f32, f32)>(|_, x| {
                let squared = x.clone() * x;
                (squared.clone(), squared.fwidth())
            })
            .unwrap();
        assert_eq!(shared.node_count(), 3);
        assert!(shared.requires_derivatives());
        let first = root
            .function::<f32, f32>(|cx, x| cx.call(&shared, x).0)
            .unwrap();
        assert_eq!(first.node_count(), 2);
        assert!(!first.requires_derivatives());
        let shader = root
            .shader(|cx| {
                let x = cx.uv().x();
                let (a, _) = cx.call(&shared, x.clone());
                let b = cx.call(&first, x);
                cx.rgba(a, b, cx.scalar(0.), cx.scalar(1.))
            })
            .unwrap();
        assert!(!shader.requires_derivatives());
        let handwritten = root
            .shader(|cx| {
                let x = cx.uv().x();
                let squared = x.clone() * x;
                cx.rgba(squared.clone(), squared, cx.scalar(0.), cx.scalar(1.))
            })
            .unwrap();
        assert_eq!(shader.program_id(), handwritten.program_id());
    }
    #[test]
    fn captures_are_rejected_but_dead_reads_and_unit_shapes_are_supported() {
        let root = PaintRoot::new();
        let parameter = root.parameter(0.5).unwrap();
        assert_eq!(
            root.function::<(), f32>(|cx, _| cx.uv().x()).unwrap_err(),
            ShaderError::CapturedInput
        );
        assert_eq!(
            root.function::<(), f32>(|cx, _| cx.uniform(parameter))
                .unwrap_err(),
            ShaderError::CapturedInput
        );
        let dead = root
            .function::<(), ()>(|cx, _| {
                cx.uv();
                cx.uniform(parameter);
            })
            .unwrap();
        assert_eq!(dead.node_count(), 0);
        assert!(!dead.requires_derivatives());
        let constants = root
            .function::<(), ((), f32)>(|cx, _| ((), cx.scalar(0.7)))
            .unwrap();
        root.shader(|cx| {
            cx.call(&dead, ());
            let (_, x) = cx.call(&constants, ());
            cx.rgba(x.clone(), x.clone(), x, cx.scalar(1.))
        })
        .unwrap();
    }
    #[test]
    fn every_foreign_input_and_output_leaf_is_validated_before_traversal() {
        let root = PaintRoot::new();
        let mut foreign = None;
        root.shader(|cx| {
            foreign = Some(cx.scalar(1.));
            cx.vec4([1.; 4])
        })
        .unwrap();
        assert_eq!(
            root.function::<(), (f32, f32)>(|cx, _| (cx.scalar(0.), foreign.clone().unwrap()))
                .unwrap_err(),
            ShaderError::ForeignGraph
        );
        let pair = root
            .function::<(f32, (f32, f32)), f32>(|_, (x, (y, z))| x + y + z)
            .unwrap();
        assert_eq!(
            root.shader(|cx| {
                let result = cx.call(&pair, (cx.scalar(0.), (cx.scalar(1.), foreign.unwrap())));
                cx.rgba(result.clone(), result.clone(), result, cx.scalar(1.))
            })
            .unwrap_err(),
            ShaderError::ForeignGraph
        );
        let mut escaped = None;
        root.function::<f32, f32>(|_, x| {
            escaped = Some(x.clone());
            x
        })
        .unwrap();
        assert_eq!(
            root.shader(|cx| cx.rgba(
                escaped.unwrap(),
                cx.scalar(0.),
                cx.scalar(0.),
                cx.scalar(1.)
            ))
            .unwrap_err(),
            ShaderError::ForeignGraph
        );
    }
    #[test]
    fn functions_respect_leaf_and_node_budgets() {
        type Four = (f32, f32, f32, f32);
        type Sixteen = (Four, Four, Four, Four);
        type SixtyFour = (Sixteen, Sixteen, Sixteen, Sixteen);
        type TooMany = (SixtyFour, SixtyFour);
        let root = PaintRoot::new();
        assert_eq!(
            root.function::<TooMany, ()>(|_, _| ()).unwrap_err(),
            ShaderError::TooManyFunctionLeaves
        );
        assert_eq!(
            root.function::<(), TooMany>(|_, _| panic!("must reject before invoking closure"))
                .unwrap_err(),
            ShaderError::TooManyFunctionLeaves
        );
        root.function::<SixtyFour, SixtyFour>(|_, x| x).unwrap();
        assert_eq!(
            root.function::<f32, f32>(|_, mut x| {
                for _ in 0..MAX_NODES {
                    x = x.sin();
                }
                x
            })
            .unwrap_err(),
            ShaderError::TooManyNodes
        );
    }
    #[test]
    fn repeated_calls_memoize_by_stable_function_and_argument_identity() {
        let root = PaintRoot::new();
        let heavy = root
            .function::<f32, f32>(|_, mut x| {
                for _ in 0..1000 {
                    x = x.sin();
                }
                x
            })
            .unwrap();
        root.shader(|cx| {
            let input = cx.uv().x();
            let first = cx.call(&heavy, input.clone());
            for _ in 0..1000 {
                let again = cx.call(&heavy, input.clone());
                assert_eq!(again.id, first.id);
            }
            assert_eq!(cx.graph.borrow().function_calls.len(), 1);
            cx.rgba(first.clone(), first.clone(), first, cx.scalar(1.))
        })
        .unwrap();
        let a = root
            .function::<f32, f32>(|cx, x| x + cx.scalar(1.))
            .unwrap();
        let b = root
            .function::<f32, f32>(|cx, x| x + cx.scalar(2.))
            .unwrap();
        assert_ne!(a.graph.id, b.graph.id);
        root.shader(|cx| {
            let x = cx.scalar(0.3);
            let first = cx.call(&a, x.clone());
            let clone = cx.call(&a.clone(), x.clone());
            let different = cx.call(&a, cx.scalar(0.4));
            let another = cx.call(&b, x);
            assert_eq!(first.id, clone.id);
            assert_ne!(first.id, different.id);
            assert_ne!(first.id, another.id);
            assert_eq!(cx.graph.borrow().function_calls.len(), 3);
            cx.rgba(first, different, another, cx.scalar(1.))
        })
        .unwrap();
    }
    #[test]
    fn temporary_functions_cannot_alias_memo_entries_and_memo_stays_bounded() {
        let root = PaintRoot::new();
        root.shader(|cx| {
            let input = cx.scalar(0.3);
            let mut previous = None;
            for n in 0..130 {
                let temporary = root
                    .function::<f32, f32>(|cx, x| x + cx.scalar(n as f32))
                    .unwrap();
                let value = cx.call(&temporary, input.clone());
                if let Some(previous) = previous {
                    assert_ne!(previous, value.id);
                }
                previous = Some(value.id);
            }
            assert_eq!(cx.graph.borrow().function_calls.len(), MAX_MEMOIZED_CALLS);
            cx.vec4([1.; 4])
        })
        .unwrap();
        let no_input = root.function::<(), f32>(|cx, _| cx.scalar(0.5)).unwrap();
        let no_output = root.function::<f32, ()>(|_, _| ()).unwrap();
        root.shader(|cx| {
            cx.call(&no_input, ());
            cx.call(&no_output, cx.scalar(0.5));
            assert!(cx.graph.borrow().function_calls.is_empty());
            cx.vec4([1.; 4])
        })
        .unwrap();
    }
    #[test]
    fn semantic_shapes_restore_without_extra_clamps_and_preserve_thread_safety() {
        let root = PaintRoot::new();
        let identity = root
            .function::<(Distance, Coverage, Predicate), (Distance, Coverage, Predicate)>(|_, x| x)
            .unwrap();
        let shader = root
            .shader(|cx| {
                let field = Distance::new(cx.scalar(-0.5));
                let coverage = Coverage::new(cx.scalar(0.8));
                let predicate = cx.scalar(1.).gt(cx.scalar(0.));
                let (_, mask, condition) = cx.call(&identity, (field, coverage, predicate));
                condition.select(mask.mask(cx.vec4([1.; 4])), cx.vec4([0.; 4]))
            })
            .unwrap();
        let handwritten = root
            .shader(|cx| {
                let mask = Coverage::new(cx.scalar(0.8));
                cx.scalar(1.)
                    .gt(cx.scalar(0.))
                    .select(mask.mask(cx.vec4([1.; 4])), cx.vec4([0.; 4]))
            })
            .unwrap();
        assert_eq!(shader.program_id(), handwritten.program_id());
        fn send_sync<T: Send + Sync>() {}
        send_sync::<Function<(Distance, Coverage, Predicate), (Distance, Coverage, Predicate)>>();
    }
}
