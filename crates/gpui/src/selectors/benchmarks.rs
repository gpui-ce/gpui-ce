//! CPU selector workloads. Fixture counts are checked outside measured intervals.

use super::{
    PendingSelector, Select, SelectableElement, SelectorBinding, SelectorMatcher, SelectorRule,
    SelectorRuntime, SelectorScope,
};
use crate::{
    self as gpui, AnyElement, App, AppContext, AvailableSpace, BenchAppContext, Bounds, Context,
    Div, Element, ElementId, Empty, Entity, GlobalElementId, InspectorElementId, IntoElement,
    LayoutId, ParentElement, Pixels, Platform, Render, StyleRefinement, Styled, Window,
    WindowOptions, container_query, deferred, div, px, reflection::trait_set, rgb,
    window::with_element_arena,
};
use criterion::Criterion;
use std::{cell::Cell, hint::black_box, rc::Rc};

#[derive(Default)]
struct Counts {
    callbacks: Cell<usize>,
    layouts: Cell<usize>,
    paints: Cell<usize>,
    renders: Cell<usize>,
    cached_renders: Cell<usize>,
}

impl Counts {
    fn reset(&self) {
        self.callbacks.set(0);
        self.layouts.set(0);
        self.paints.set(0);
        self.renders.set(0);
        self.cached_renders.set(0);
    }
}

#[derive(gpui_macros::Reflect, gpui_macros::Styled, gpui_macros::ParentElement)]
#[reflect(crate::Styled, crate::ParentElement)]
struct Node {
    #[style(delegate)]
    #[children(delegate)]
    inner: Div,
    counts: Rc<Counts>,
}

impl IntoElement for Node {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for Node {
    type RequestLayoutState = <Div as Element>::RequestLayoutState;
    type PrepaintState = <Div as Element>::PrepaintState;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        self.counts.layouts.set(self.counts.layouts.get() + 1);

        self.inner
            .request_layout(global_id, inspector_id, window, cx)
    }

    fn prepaint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        state: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        self.inner
            .prepaint(global_id, inspector_id, bounds, state, window, cx)
    }

    fn paint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.counts.paints.set(self.counts.paints.get() + 1);
        self.inner.paint(
            global_id,
            inspector_id,
            bounds,
            layout,
            prepaint,
            window,
            cx,
        );
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Work {
    None,
    Miss,
    Identity,
    Fluent,
    Parent,
    This,
    Exhausted,
}

impl Work {
    fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Miss => "miss",
            Self::Identity => "identity",
            Self::Fluent => "fluent",
            Self::Parent => "parent",
            Self::This => "this",
            Self::Exhausted => "exhausted",
        }
    }
}

fn reset_arena() {
    with_element_arena(|arena| arena.clear());
}

fn node(counts: &Rc<Counts>) -> AnyElement {
    Node {
        inner: div().size(px(8.)).bg(rgb(0x223344)).into_element(),
        counts: counts.clone(),
    }
    .class("row")
    .into_any_element()
}

fn rows(nodes: usize, counts: &Rc<Counts>) -> AnyElement {
    div()
        .flex()
        .flex_wrap()
        .size_full()
        .children((0..nodes).map(|_idx| node(counts)))
        .into_any_element()
}

fn attach(mut root: AnyElement, work: Work, rules: usize, counts: &Rc<Counts>) -> AnyElement {
    if work == Work::None {
        return root;
    }

    for _idx in 0..rules {
        let counts = counts.clone();
        let selector = match work {
            Work::This => Select::this(),
            _work => Select::descendants(),
        }
        .class(if work == Work::Miss { "missing" } else { "row" });

        root = match work {
            Work::Identity => root.select(selector.reflects(Styled), move |element| {
                counts.callbacks.set(counts.callbacks.get() + 1);

                element
            }),
            Work::Parent => root.select(
                selector.reflects(trait_set![crate::Styled, crate::ParentElement]),
                move |element| {
                    counts.callbacks.set(counts.callbacks.get() + 1);

                    element.text_xl().text_color(rgb(0x112233)).child(Empty)
                },
            ),
            _work => {
                let selector = selector.reflects(Styled);
                let selector = if work == Work::Exhausted {
                    selector.nth(0)
                } else {
                    selector
                };

                root.select(selector, move |element| {
                    counts.callbacks.set(counts.callbacks.get() + 1);

                    element.text_xl().text_color(rgb(0x112233))
                })
            }
        }
        .into_any_element();
    }

    root
}

fn predicates(nodes: usize, mixed: bool, counts: &Rc<Counts>) -> Vec<AnyElement> {
    (0..nodes)
        .map(|idx| {
            if mixed && idx.is_multiple_of(2) {
                return Empty.class("row").into_any_element();
            }

            node(counts)
        })
        .collect()
}

fn match_nodes(elements: &[AnyElement], matchers: &[SelectorMatcher]) -> usize {
    let mut matched = 0;

    for element in elements {
        for matcher in matchers {
            matched += usize::from(matcher.matches_predicates(black_box(element)));
        }
    }

    black_box(matched)
}

fn matching(criterion: &mut Criterion, profile: &mut impl FnMut(&str, &mut dyn FnMut())) {
    let mut group = criterion.benchmark_group("selectors/matching");
    let counts = Rc::new(Counts::default());

    for nodes in [32, 256] {
        for rules in [1, 16, 64] {
            for kind in ["all", "missing_trait", "missing_class", "mixed"] {
                reset_arena();
                let elements = predicates(nodes, kind == "mixed", &counts);
                let matchers = (0..rules)
                    .map(|_idx| {
                        if kind == "missing_trait" {
                            return Select::descendants()
                                .reflects(crate::StatefulInteractiveElement)
                                .into_matcher();
                        }

                        Select::descendants()
                            .class(if kind == "missing_class" {
                                "missing"
                            } else {
                                "row"
                            })
                            .reflects(trait_set![crate::Styled, crate::ParentElement])
                            .into_matcher()
                    })
                    .collect::<Vec<_>>();

                let expected = match kind {
                    "all" => nodes * rules,
                    "mixed" => nodes * rules / 2,
                    _kind => 0,
                };
                assert_eq!(match_nodes(&elements, &matchers), expected);

                let name = format!("{kind}/{nodes}/{rules}");
                profile(&format!("matching/{name}"), &mut || {
                    match_nodes(&elements, &matchers);
                });
                group.bench_function(name, |bencher| {
                    bencher.iter(|| match_nodes(&elements, &matchers));
                });
                assert_eq!(match_nodes(&elements, &matchers), expected);
                drop(elements);
            }
        }
    }

    group.finish();
    reset_arena();
}

fn layout(root: &mut AnyElement, window: &mut Window, cx: &mut App) {
    let _session = window.selector_runtime().enter_session();

    root.layout_as_root(AvailableSpace::min_size(), window, cx);
}

fn first_layout(
    criterion: &mut Criterion,
    platform: &Rc<dyn Platform>,
    profile: &mut impl FnMut(&str, &mut dyn FnMut()),
) {
    let mut group = criterion.benchmark_group("selectors/layout");

    for nodes in [32, 256] {
        for rules in [1, 16] {
            for work in [
                Work::None,
                Work::Miss,
                Work::Identity,
                Work::Fluent,
                Work::Parent,
            ] {
                let name = format!("{}/{nodes}/{rules}", work.name());

                group.bench_function(&name, |bencher| {
                    let mut cx = BenchAppContext::new(platform.clone(), None, bencher);
                    let mut visual = cx.add_empty_window();
                    let counts = Rc::new(Counts::default());
                    let expected = if matches!(work, Work::None | Work::Miss) {
                        0
                    } else {
                        nodes * rules
                    };

                    reset_arena();
                    let mut root = attach(rows(nodes, &counts), work, rules, &counts);
                    visual.update(|window, cx| {
                        window.clear_benchmark_layout();
                        layout(&mut root, window, cx);
                    });
                    assert_eq!(counts.layouts.get(), nodes);
                    assert_eq!(counts.callbacks.get(), expected);
                    drop(root);
                    reset_arena();
                    counts.reset();

                    let mut root = attach(rows(nodes, &counts), work, rules, &counts);
                    visual.update(|window, cx| {
                        window.clear_benchmark_layout();
                        profile(&format!("layout/{name}"), &mut || {
                            layout(&mut root, window, cx)
                        });
                    });
                    drop(root);
                    reset_arena();

                    let mut setup_window = visual.clone();

                    cx.bench_batched(
                        |_cx| {
                            reset_arena();
                            counts.reset();
                            setup_window.update(|window, _cx| window.clear_benchmark_layout());

                            attach(rows(nodes, &counts), work, rules, &counts)
                        },
                        |root, _cx| visual.update(|window, cx| layout(root, window, cx)),
                    );
                    assert_eq!(counts.layouts.get(), nodes);
                    assert_eq!(counts.callbacks.get(), expected);
                    reset_arena();
                    drop(setup_window);
                    drop(visual);
                    cx.teardown();
                });
            }
        }
    }

    group.finish();
}

struct CachedRows {
    nodes: usize,
    counts: Rc<Counts>,
}

impl Render for CachedRows {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        self.counts
            .cached_renders
            .set(self.counts.cached_renders.get() + 1);

        rows(self.nodes, &self.counts)
    }
}

struct Frame {
    kind: &'static str,
    work: Work,
    nodes: usize,
    rules: usize,
    counts: Rc<Counts>,
    cached: Entity<CachedRows>,
}

impl Render for Frame {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        self.counts.renders.set(self.counts.renders.get() + 1);
        let root = match self.kind {
            "cached" => {
                let mut root = div().size_full();

                if self.work == Work::Exhausted {
                    root = root.child(node(&self.counts));
                }

                root.child(
                    self.cached
                        .clone()
                        .cached(StyleRefinement::default().size_full()),
                )
                .into_any_element()
            }
            "deferred" => {
                let counts = self.counts.clone();
                let nodes = self.nodes;

                div()
                    .size_full()
                    .child(deferred(container_query(move |_bounds, _window, _cx| {
                        let counts = counts.clone();

                        div().child(deferred(container_query(move |_bounds, _window, _cx| {
                            rows(nodes, &counts)
                        })))
                    })))
                    .into_any_element()
            }
            _kind => rows(self.nodes, &self.counts),
        };

        attach(root, self.work, self.rules, &self.counts)
    }
}

fn frames(
    criterion: &mut Criterion,
    platform: &Rc<dyn Platform>,
    profile: &mut impl FnMut(&str, &mut dyn FnMut()),
) {
    let mut group = criterion.benchmark_group("selectors/frame");
    let nodes = 256;
    let rules = 4;

    for kind in ["fresh", "cached", "deferred"] {
        let works = if kind == "cached" {
            &[Work::None, Work::This, Work::Exhausted, Work::Fluent][..]
        } else {
            &[
                Work::None,
                Work::Miss,
                Work::Identity,
                Work::Fluent,
                Work::Parent,
            ][..]
        };

        for &work in works {
            let name = format!("{kind}/{}", work.name());

            group.bench_function(&name, |bencher| {
                let mut cx = BenchAppContext::new(platform.clone(), None, bencher);
                let counts = Rc::new(Counts::default());
                let window = cx.update(|cx| {
                    let cached = cx.new(|_cx| CachedRows {
                        nodes,
                        counts: counts.clone(),
                    });

                    cx.open_window(WindowOptions::default(), |_window, cx| {
                        cx.new(|_cx| Frame {
                            kind,
                            work,
                            nodes,
                            rules,
                            counts: counts.clone(),
                            cached,
                        })
                    })
                    .unwrap()
                });
                let view = window.entity(&cx).unwrap();

                for _warmup in 0..2 {
                    cx.update_window(window.into(), |_view, window, cx| window.draw(cx).clear(cx))
                        .unwrap();
                }

                counts.reset();
                cx.update_window(window.into(), |_view, window, cx| {
                    profile(&format!("frame/{name}"), &mut || window.draw(cx).clear(cx));
                })
                .unwrap();

                // The ordinary frame below also supplies counts when profiling is disabled.
                counts.reset();
                cx.update_window(window.into(), |_view, window, cx| window.draw(cx).clear(cx))
                    .unwrap();
                let cached_reuse = kind == "cached" && work != Work::Fluent;
                let expected_callbacks = match work {
                    Work::None | Work::Miss | Work::This => 0,
                    Work::Exhausted => rules,
                    _work => nodes * rules,
                };
                let expected_layouts = if cached_reuse {
                    usize::from(work == Work::Exhausted)
                } else {
                    nodes
                };

                assert_eq!(counts.renders.get(), 1, "{name}");
                assert_eq!(
                    counts.cached_renders.get(),
                    usize::from(kind == "cached" && !cached_reuse),
                    "{name}"
                );
                assert_eq!(counts.callbacks.get(), expected_callbacks, "{name}");
                assert_eq!(counts.layouts.get(), expected_layouts, "{name}");
                assert_eq!(counts.paints.get(), expected_layouts, "{name}");
                cx.update_window(window.into(), |_view, window, _cx| {
                    assert_eq!(
                        window.rendered_frame.scene.quads.len(),
                        nodes + usize::from(work == Work::Exhausted),
                        "{name}"
                    );
                })
                .unwrap();

                cx.bench_renderer(view, |_view, _window, cx| cx.notify());
                cx.update(|cx| {
                    window
                        .update(cx, |_view, window, _cx| window.remove_window())
                        .unwrap()
                });
                cx.teardown();
            });
        }
    }

    group.finish();
}

fn runtimes(
    criterion: &mut Criterion,
    platform: &Rc<dyn Platform>,
    profile: &mut impl FnMut(&str, &mut dyn FnMut()),
) {
    let mut group = criterion.benchmark_group("selectors/runtime");

    for rules in [0, 1, 4, 16, 64] {
        let pending = (0..rules)
            .map(|_idx| {
                PendingSelector::new(
                    Select::descendants().into_matcher(),
                    Box::new(|element| element),
                )
            })
            .collect::<Vec<_>>();

        group.bench_function(rules.to_string(), |bencher| {
            let mut cx = BenchAppContext::new(platform.clone(), None, bencher);
            let runtime = SelectorRuntime::new();

            {
                let _session = runtime.enter_session();
                let _attached = runtime.enter_attached(&pending);

                profile(&format!("runtime/{rules}"), &mut || {
                    let _children = runtime.enter_children();

                    black_box(runtime.capture_scope());
                });

                cx.bench_iter(|_cx| {
                    let _children = runtime.enter_children();

                    black_box(runtime.capture_scope());
                });
            }

            cx.teardown();
        });
    }

    group.finish();
}

/// Runs matching, first-layout, scope-copy, and CPU frame benchmarks.
pub fn run(
    criterion: &mut Criterion,
    platform: Rc<dyn Platform>,
    mut profile: impl FnMut(&str, &mut dyn FnMut()),
) {
    eprintln!(
        "bytes: AnyElement={} matcher={} rule={} binding={} runtime={} scope={}",
        size_of::<AnyElement>(),
        size_of::<SelectorMatcher>(),
        size_of::<SelectorRule>(),
        size_of::<SelectorBinding>(),
        size_of::<SelectorRuntime>(),
        size_of::<SelectorScope>(),
    );
    matching(criterion, &mut profile);
    first_layout(criterion, &platform, &mut profile);
    frames(criterion, &platform, &mut profile);
    runtimes(criterion, &platform, &mut profile);
}
