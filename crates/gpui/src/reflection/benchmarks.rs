//! Reflection benchmarks for `benches/reflection.rs`, gated by `bench-support`.
//! Workloads live inside `gpui` to access private reflection and arena APIs.

use crate::{
    self as gpui, AnyElement, App, ArenaBox, Bounds, Div, Drawable, Element, ElementId, Empty,
    GlobalElementId, InspectorElementId, IntoElement, LayoutId, ParentElement, Pixels,
    StyleRefinement, Styled, Window, div, hsla,
    reflection::{ElementReflection, ReflectedElement, ReflectedTraits, linked_metadata},
    window::with_element_arena,
};
use criterion::{BatchSize, Criterion};
use std::{any::Any, hint::black_box};

const NODE_COUNT: usize = 128;

#[gpui_macros::reflect_trait]
trait Reading {
    fn refinement(&mut self) -> &mut StyleRefinement;

    fn reading(&self) -> usize {
        0
    }
}

#[derive(gpui_macros::Reflect)]
#[reflect(Reading)]
struct Probe {
    inner: Div,
    children: usize,
}

impl Reading for Probe {
    fn refinement(&mut self) -> &mut StyleRefinement {
        self.inner.style()
    }

    fn reading(&self) -> usize {
        17
    }
}

impl Styled for Probe {
    fn style(&mut self) -> &mut StyleRefinement {
        self.inner.style()
    }
}

impl ParentElement for Probe {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        for element in elements {
            self.children += 1;
            self.inner.extend(std::iter::once(element));
        }
    }
}

impl IntoElement for Probe {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for Probe {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _window: &mut Window,
        _cx: &mut App,
    ) -> (LayoutId, ()) {
        unreachable!("reflection benchmarks do not draw")
    }

    fn prepaint(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout_state: &mut (),
        _window: &mut Window,
        _cx: &mut App,
    ) {
        unreachable!("reflection benchmarks do not draw")
    }

    fn paint(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout_state: &mut (),
        _prepaint_state: &mut (),
        _window: &mut Window,
        _cx: &mut App,
    ) {
        unreachable!("reflection benchmarks do not draw")
    }
}

fn probe() -> AnyElement {
    Probe {
        inner: div(),
        children: 0,
    }
    .into_any_element()
}

fn reset_arena() {
    with_element_arena(|arena| arena.clear());
}

fn erase<Type: Element>(element: Type, retain_metadata: bool) {
    if retain_metadata {
        black_box(element.into_any());

        return;
    }

    // Match the erased drawable allocation while omitting metadata resolution and validation.
    let drawable = with_element_arena(|arena| arena.alloc(|| Drawable::new(element)))
        .map(|drawable| drawable as &mut dyn Any);

    black_box(drawable);
}

fn erase_nodes(kind: &str, retain_metadata: bool) {
    for idx in 0..NODE_COUNT {
        match kind {
            "div" => erase(div(), retain_metadata),
            "empty" => erase(Empty, retain_metadata),
            "mixed" if idx % 2 == 0 => erase(div(), retain_metadata),
            "mixed" => erase(Empty, retain_metadata),
            _kind => unreachable!(),
        }
    }
}

fn metadata(element: &AnyElement, registry: bool) -> &'static ElementReflection {
    if registry {
        return linked_metadata(black_box(element.reflected_type_id()));
    }

    black_box(element.reflection())
}

fn membership(element: &AnyElement, kind: &str, registry: bool) {
    let metadata = metadata(element, registry);

    match kind {
        "descriptors" => {
            black_box(metadata.descriptors());
        }
        "callable" => {
            black_box(metadata.implements_trait(Styled));
        }
        "missing" => {
            black_box(metadata.implements_trait(crate::InteractiveElement));
        }
        _kind => unreachable!(),
    }
}

fn style_access(element: &mut AnyElement, registry: bool) {
    let methods = metadata(element, registry).methods_for(Reading);

    black_box((methods.refinement)(black_box(element).inner_element_mut()));
}

fn reading(element: &AnyElement, registry: bool) {
    let methods = metadata(element, registry).methods_for(Reading);

    black_box((methods.reading)(black_box(element).inner_element()));
}

fn styled_chain<Traits>(element: AnyElement, _traits: Traits) -> AnyElement
where
    Traits: ReflectedTraits,
    ReflectedElement<Traits::Group>: Styled,
{
    ReflectedElement::<Traits::Group>::new(element)
        .text_xl()
        .text_color(hsla(0.5, 0.6, 0.7, 1.))
        .into_any_element()
}

fn parent_chain<Traits>(element: AnyElement, _traits: &Traits) -> AnyElement
where
    Traits: ReflectedTraits,
    ReflectedElement<Traits::Group>: Styled + ParentElement,
{
    ReflectedElement::<Traits::Group>::new(element)
        .text_xl()
        .text_color(hsla(0.5, 0.6, 0.7, 1.))
        .child(Empty)
        .into_any_element()
}

fn fluent<Traits>(element: AnyElement, kind: &str, traits: &Traits)
where
    Traits: ReflectedTraits,
    ReflectedElement<Traits::Group>: Styled + ParentElement,
{
    let element = match kind {
        "accessor" => {
            let mut reflected = ReflectedElement::<crate::__GpuiReflectStyledGroup>::new(element);
            black_box(reflected.style());

            reflected.into_any_element()
        }
        "styled" => styled_chain::<crate::__GpuiReflectStyled>(element, Styled),
        "styled_parent" => parent_chain(element, traits),
        _kind => unreachable!(),
    };

    black_box(element);
}

fn warm_up() {
    let mut element = probe();
    let metadata = element.reflection();
    let methods = metadata.methods_for(Reading);

    assert_eq!((methods.reading)(element.inner_element()), 17);
    assert!(metadata.implements_trait(Styled));
    assert!(!metadata.implements_trait(crate::InteractiveElement));

    style_access(&mut element, false);
    let mut styled = styled_chain(probe(), Styled);
    let traits = gpui_macros::trait_set![crate::Styled, crate::ParentElement];
    let mut parent = parent_chain(probe(), &traits);

    for element in [&mut styled, &mut parent] {
        let concrete = element.downcast_mut::<Probe>().unwrap();

        assert!(concrete.inner.style().text.font_size.is_some());
        assert_eq!(
            concrete.inner.style().text.color,
            Some(hsla(0.5, 0.6, 0.7, 1.))
        );
    }

    assert_eq!(parent.downcast_mut::<Probe>().unwrap().children, 1);
    erase_nodes("mixed", true);
    reset_arena();
    erase_nodes("mixed", true);
    reset_arena();
}

/// Runs warm reflection measurements without a window, renderer, or selector APIs.
pub fn run(criterion: &mut Criterion) {
    warm_up();
    let traits = gpui_macros::trait_set![crate::Styled, crate::ParentElement];
    eprintln!(
        "AnyElement={} bytes; drawable erasure={} bytes; nodes/batch={NODE_COUNT}",
        size_of::<AnyElement>(),
        size_of::<ArenaBox<dyn Any>>()
    );

    let mut group = criterion.benchmark_group("reflection/erasure");

    for kind in ["div", "empty", "mixed"] {
        for retain_metadata in [false, true] {
            let path = if retain_metadata {
                "retained"
            } else {
                "drawable_only"
            };

            group.bench_function(format!("{kind}/{path}"), |bencher| {
                bencher.iter_batched(
                    reset_arena,
                    |()| erase_nodes(kind, retain_metadata),
                    BatchSize::PerIteration,
                );
            });
        }
    }

    group.finish();
    reset_arena();

    let mut element = probe();
    let mut group = criterion.benchmark_group("reflection/membership");

    for kind in ["descriptors", "callable", "missing"] {
        for registry in [true, false] {
            let path = if registry { "registry" } else { "retained" };

            group.bench_function(format!("{kind}/{path}"), |bencher| {
                bencher.iter(|| membership(black_box(&element), kind, registry));
            });
        }
    }

    group.finish();
    let mut group = criterion.benchmark_group("reflection/dispatch");

    for registry in [true, false] {
        let path = if registry { "registry" } else { "retained" };

        group.bench_function(format!("style/{path}"), |bencher| {
            bencher.iter(|| style_access(black_box(&mut element), registry));
        });
        group.bench_function(format!("override/{path}"), |bencher| {
            bencher.iter(|| reading(black_box(&element), registry));
        });
    }

    group.finish();
    let mut group = criterion.benchmark_group("reflection/fluent");

    for kind in ["accessor", "styled", "styled_parent"] {
        group.bench_function(kind, |bencher| {
            bencher.iter_batched(
                || {
                    reset_arena();

                    probe()
                },
                |element| fluent(element, kind, &traits),
                BatchSize::PerIteration,
            );
        });
    }

    group.finish();
    reset_arena();
}
