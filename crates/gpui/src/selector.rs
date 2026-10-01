use crate::{
    AnyElement, ElementId, IntoElement, IntoItemMatch, IntoListMatch, IntoMatchValues, ItemMatch,
    ListMatch, SharedString,
    reflection::{ReflectedElement, ReflectedTrait, ReflectedTraits, ReflectionGroup},
};
use smallvec::SmallVec;
use std::{cell::RefCell, marker::PhantomData, num::NonZeroUsize};

/// The reflection group used by selectors without a reflected trait predicate.
#[doc(hidden)]
pub struct NoReflectedTraits;

impl ReflectionGroup for NoReflectedTraits {}

/// Selects elements relative to an element tree root.
pub struct Select<Group = NoReflectedTraits>
where
    Group: ReflectionGroup,
{
    matcher: SelectorMatcher,
    group: PhantomData<fn() -> Group>,
}

struct SelectorMatcher {
    scope: SelectScope,
    reflected_traits: SmallVec<[ReflectedTrait; 2]>,
    element_id: Option<ItemMatch<ElementId>>,
    classes: SmallVec<[ListMatch<SharedString>; 2]>,
    nth: Option<usize>,
    every: Option<NonZeroUsize>,
}

#[derive(Clone, Copy)]
enum SelectScope {
    This,
    Children,
    Descendants,
}

impl Select<NoReflectedTraits> {
    /// Selects the element on which the selector is installed.
    pub fn this() -> Self {
        Self::new(SelectScope::This)
    }

    /// Selects direct children of the element on which the selector is installed.
    pub fn children() -> Self {
        Self::new(SelectScope::Children)
    }

    /// Selects every descendant of the element on which the selector is installed.
    pub fn descendants() -> Self {
        Self::new(SelectScope::Descendants)
    }

    fn new(scope: SelectScope) -> Self {
        Self {
            matcher: SelectorMatcher {
                scope,
                reflected_traits: SmallVec::new(),
                element_id: None,
                classes: SmallVec::new(),
                nth: None,
                every: None,
            },
            group: PhantomData,
        }
    }

    /// Requires matching elements to reflect every trait in the given trait set.
    pub fn reflects<Traits>(mut self, reflected_traits: Traits) -> Select<Traits::Group>
    where
        Traits: ReflectedTraits,
    {
        self.matcher
            .reflected_traits
            .extend(reflected_traits.reflected_traits());

        Select {
            matcher: self.matcher,
            group: PhantomData,
        }
    }
}

impl<Group> Select<Group>
where
    Group: ReflectionGroup,
{
    /// Matches one element ID, optionally negated with [`crate::not`].
    ///
    /// Composite IDs such as `("row", 3_usize)` remain one ID. A negated ID also
    /// matches elements without an ID. Calling this method again replaces the ID condition.
    ///
    /// ```compile_fail
    /// use gpui::Select;
    ///
    /// let selector = Select::children().id(["save", "cancel"]);
    /// ```
    ///
    /// ```compile_fail
    /// use gpui::{Select, any};
    ///
    /// let selector = Select::children().id(any(["save", "cancel"]));
    /// ```
    pub fn id<Kind>(mut self, element_id: impl IntoItemMatch<ElementId, Kind>) -> Self {
        self.matcher.element_id = Some(element_id.into_item_match());

        self
    }

    /// Matches class membership using values, arrays, vectors, slices, or tuples.
    ///
    /// Collections require every expression to match. Use [`crate::any`] for OR,
    /// and [`crate::not`] to negate an expression. Repeated calls require all conditions.
    ///
    /// ```
    /// use gpui::{Select, any, not};
    ///
    /// let selector = Select::children().class(("apple", "pear", not("plum")));
    /// let selector = Select::children().class(not(any(["apple", "pear"])));
    /// ```
    pub fn class<Kind>(mut self, class: impl IntoListMatch<SharedString, Kind>) -> Self {
        self.matcher.classes.push(class.into_list_match());

        self
    }

    /// Selects the matching element at a zero-based index in layout visitation order.
    ///
    /// Scope, reflected traits, classes, and ID conditions run before indexing,
    /// regardless of builder order. Late-created children continue the same index sequence.
    /// Temporary measurement elements are excluded, and virtualized lists only index
    /// materialized elements. Calling this method again replaces the index condition.
    pub fn nth(mut self, idx: usize) -> Self {
        self.matcher.nth = Some(idx);

        self
    }

    /// Selects matching indexes `0, step, 2 * step, ...` in layout visitation order.
    ///
    /// Uses the same indexing rules as [`Self::nth`]. If both methods are used,
    /// both conditions test the same index. Calling this method again replaces the interval.
    ///
    /// # Panics
    ///
    /// Panics if `step` is zero.
    pub fn every(mut self, step: usize) -> Self {
        self.matcher.every =
            Some(NonZeroUsize::new(step).expect("selector interval must be nonzero"));

        self
    }

    fn into_matcher(self) -> SelectorMatcher {
        self.matcher
    }
}

impl SelectorMatcher {
    fn matches(&self, element: &AnyElement, depth: usize) -> bool {
        if !self.includes_depth(depth) {
            return false;
        }

        if !self
            .reflected_traits
            .iter()
            .all(|reflected_trait| element.implements_trait(*reflected_trait))
        {
            return false;
        }

        if let Some(element_id) = self.element_id.as_ref()
            && !element_id.evaluate(&|expected| element.element_id().as_ref() == Some(expected))
        {
            return false;
        }

        if !self
            .classes
            .iter()
            .all(|class| class.matches(element.classes()))
        {
            return false;
        }

        true
    }

    fn has_position(&self) -> bool {
        self.nth.is_some() || self.every.is_some()
    }

    fn includes_position(&self, idx: usize) -> bool {
        self.nth.is_none_or(|expected| idx == expected)
            && self.every.is_none_or(|step| idx.is_multiple_of(step.get()))
    }

    fn includes_depth(&self, depth: usize) -> bool {
        match self.scope {
            SelectScope::This => depth == 0,
            SelectScope::Children => depth == 1,
            SelectScope::Descendants => depth >= 1,
        }
    }
}

/// Adds selector metadata and transformations to elements.
pub trait SelectableElement: IntoElement + Sized {
    /// Tags this element with one class or a collection of positive class names.
    ///
    /// Arrays, vectors, slices, and tuples are supported. Boolean expressions are
    /// only accepted by [`Select::class`].
    ///
    /// ```compile_fail
    /// use gpui::{SelectableElement, div, not};
    ///
    /// let element = div().class(not("apple"));
    /// ```
    fn class<Kind>(self, class: impl IntoMatchValues<SharedString, Kind>) -> AnyElement {
        let mut element = self.into_any_element();
        class.extend_match_values(element.classes_mut());

        element
    }

    /// Transforms each element matched by the selector before it requests layout.
    fn select<Group, Output>(
        self,
        selector: Select<Group>,
        mut transform: impl FnMut(ReflectedElement<Group>) -> Output + 'static,
    ) -> AnyElement
    where
        Group: ReflectionGroup,
        Output: IntoElement + 'static,
    {
        let mut element = self.into_any_element();
        let transform = move |element| transform(ReflectedElement::new(element)).into_any_element();
        element.add_selector(PendingSelector {
            selector: selector.into_matcher(),
            transform: Box::new(transform),
            next_match_idx: 0,
        });

        element
    }
}

impl<ElementType> SelectableElement for ElementType where ElementType: IntoElement {}

pub(crate) struct ElementMetadata {
    pub(crate) classes: SmallVec<[SharedString; 2]>,
    pub(crate) selectors: Vec<PendingSelector>,
}

impl ElementMetadata {
    pub(crate) fn new() -> Self {
        Self {
            classes: SmallVec::new(),
            selectors: Vec::new(),
        }
    }
}

pub(crate) struct PendingSelector {
    selector: SelectorMatcher,
    transform: Box<dyn FnMut(AnyElement) -> AnyElement>,
    next_match_idx: usize,
}

struct ActiveSelector {
    pending: PendingSelector,
    depth: usize,
}

#[derive(Default)]
struct ElementTraversal {
    active_selectors: Vec<ActiveSelector>,
    measurement_depth: usize,
}

thread_local! {
    static ELEMENT_TRAVERSAL: RefCell<ElementTraversal> = RefCell::new(ElementTraversal::default());
}

pub(crate) fn has_active_selectors() -> bool {
    ELEMENT_TRAVERSAL.with_borrow(|traversal| !traversal.active_selectors.is_empty())
}

struct TraversalResetGuard;

impl Drop for TraversalResetGuard {
    fn drop(&mut self) {
        if !std::thread::panicking() {
            return;
        }

        ELEMENT_TRAVERSAL.with_borrow_mut(|traversal| {
            traversal.active_selectors.clear();
        });
    }
}

pub(crate) fn with_attached_selectors<ResultType>(
    selectors: Vec<PendingSelector>,
    operation: impl FnOnce() -> ResultType,
) -> (ResultType, Vec<PendingSelector>) {
    let selector_start = ELEMENT_TRAVERSAL.with_borrow_mut(|traversal| {
        let selector_start = traversal.active_selectors.len();
        traversal
            .active_selectors
            .extend(selectors.into_iter().map(|selector| ActiveSelector {
                pending: selector,
                depth: 0,
            }));

        selector_start
    });
    let traversal_guard = TraversalResetGuard;

    let result = operation();
    let selectors = ELEMENT_TRAVERSAL.with_borrow_mut(|traversal| {
        traversal
            .active_selectors
            .drain(selector_start..)
            .map(|selector| selector.pending)
            .collect()
    });
    drop(traversal_guard);

    (result, selectors)
}

struct SelectorMeasurementGuard;

impl Drop for SelectorMeasurementGuard {
    fn drop(&mut self) {
        ELEMENT_TRAVERSAL.with_borrow_mut(|traversal| {
            traversal.measurement_depth -= 1;
        });
    }
}

pub(crate) fn with_selector_measurement<ResultType>(
    operation: impl FnOnce() -> ResultType,
) -> ResultType {
    ELEMENT_TRAVERSAL.with_borrow_mut(|traversal| {
        traversal.measurement_depth += 1;
    });
    let measurement_guard = SelectorMeasurementGuard;
    let result = operation();
    drop(measurement_guard);

    result
}

pub(crate) fn with_selector_transaction<Success, Failure>(
    operation: impl FnOnce() -> Result<Success, Failure>,
) -> Result<Success, Failure> {
    let checkpoint = ELEMENT_TRAVERSAL.with_borrow(|traversal| {
        if !traversal
            .active_selectors
            .iter()
            .any(|selector| selector.pending.selector.has_position())
        {
            return None;
        }

        Some(
            traversal
                .active_selectors
                .iter()
                .map(|selector| selector.pending.next_match_idx)
                .collect::<Vec<_>>(),
        )
    });
    let result = operation();

    if result.is_err()
        && let Some(checkpoint) = checkpoint
    {
        ELEMENT_TRAVERSAL.with_borrow_mut(|traversal| {
            for (selector, idx) in traversal.active_selectors.iter_mut().zip(checkpoint) {
                selector.pending.next_match_idx = idx;
            }
        });
    }

    result
}

pub(crate) fn with_deeper_selector_depth<ResultType>(
    operation: impl FnOnce() -> ResultType,
) -> ResultType {
    ELEMENT_TRAVERSAL.with_borrow_mut(|traversal| {
        for selector in &mut traversal.active_selectors {
            selector.depth += 1;
        }
    });

    let result = operation();

    ELEMENT_TRAVERSAL.with_borrow_mut(|traversal| {
        for selector in &mut traversal.active_selectors {
            selector.depth -= 1;
        }
    });

    result
}

pub(crate) fn apply_active_selectors(element: &mut AnyElement) {
    let selector_count =
        ELEMENT_TRAVERSAL.with_borrow(|traversal| traversal.active_selectors.len());

    for selector_idx in 0..selector_count {
        let matches = ELEMENT_TRAVERSAL.with_borrow_mut(|traversal| {
            let measuring = traversal.measurement_depth > 0;
            let selector = &mut traversal.active_selectors[selector_idx];

            if measuring && selector.pending.selector.has_position() {
                return false;
            }

            if !selector.pending.selector.matches(element, selector.depth) {
                return false;
            }

            if !selector.pending.selector.has_position() {
                return true;
            }

            let idx = selector.pending.next_match_idx;
            selector.pending.next_match_idx += 1;

            selector.pending.selector.includes_position(idx)
        });

        if !matches {
            continue;
        }

        let mut selector = ELEMENT_TRAVERSAL
            .with_borrow_mut(|traversal| traversal.active_selectors.remove(selector_idx));
        let selected = element.take();
        let selected = (selector.pending.transform)(selected);
        element.replace(selected);

        ELEMENT_TRAVERSAL.with_borrow_mut(|traversal| {
            traversal.active_selectors.insert(selector_idx, selector);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AnyWindowHandle, AppContext, AvailableSpace, Context, Element, InteractiveElement,
        ListAlignment, ListState, ParentElement, Render, StyleRefinement, Styled, TestAppContext,
        Window, any, div, list, not, px, rgb, uniform_list,
    };
    use std::{cell::Cell, panic, rc::Rc};

    struct SelectorTestView {
        render: Box<dyn Fn() -> AnyElement>,
    }

    impl Render for SelectorTestView {
        #[allow(unused_variables)]
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            (self.render)()
        }
    }

    fn draw_selector_window(window: AnyWindowHandle, cx: &mut TestAppContext) {
        cx.update_window(window, |_view, window, cx| {
            window.draw(cx).clear(cx);
        })
        .unwrap();
    }

    #[crate::test]
    #[allow(unused_variables)]
    fn combines_class_conditions_with_scalar_ids(cx: &mut TestAppContext) {
        let selected = Rc::new(RefCell::new(Vec::new()));
        let selected_for_render = selected.clone();
        let window = cx.add_window(move |window, cx| SelectorTestView {
            render: Box::new(move || {
                let selected = selected_for_render.clone();

                div()
                    .child(div().id(("row", 0_usize)).class(["apple", "pear"]))
                    .child(div().id(("row", 1_usize)).class(("apple", "pear", "plum")))
                    .child(div().id(("row", 2_usize)).class(vec!["apple", "pear"]))
                    .child(div().id(("row", 3_usize)).class(&["pear"]))
                    .child(div().class("apple"))
                    .select(
                        Select::children()
                            .reflects(crate::Styled)
                            .class((any(["apple", "orange"]), not("plum")))
                            .class("pear")
                            .id(not(("row", 0_usize))),
                        move |element| {
                            selected.borrow_mut().push(element.element.element_id());

                            element.bg(rgb(0x112233))
                        },
                    )
            }),
        });

        selected.borrow_mut().clear();
        draw_selector_window(window.into(), cx);

        assert_eq!(
            *selected.borrow(),
            vec![Some(ElementId::from(("row", 2_usize)))]
        );
    }

    #[crate::test]
    #[allow(unused_variables)]
    fn indexes_filtered_elements_across_layout_and_prepaint(cx: &mut TestAppContext) {
        let selected = Rc::new(RefCell::new(Vec::new()));
        let selected_for_render = selected.clone();
        let window = cx.add_window(move |window, cx| SelectorTestView {
            render: Box::new(move || {
                let selectors = [
                    ("nth", Select::descendants().class("row").nth(2)),
                    ("reordered", Select::descendants().nth(2).class("row")),
                    ("every", Select::descendants().class("row").every(2)),
                    ("both", Select::descendants().class("row").nth(2).every(2)),
                    (
                        "disjoint",
                        Select::descendants().class("row").nth(1).every(2),
                    ),
                    ("missing", Select::descendants().class("row").nth(99)),
                ];
                let mut root = div()
                    .child(div().id("first").class("row"))
                    .child(div().id("unrelated"))
                    .child(div().id("second").class("row"))
                    .child(crate::container_query(|size, window, cx| {
                        div()
                            .id("late")
                            .child(div().id("late-child").class("row"))
                            .class("row")
                    }))
                    .into_any_element();

                for (label, selector) in selectors {
                    let selected = selected_for_render.clone();
                    root = root.select(selector, move |element| {
                        selected
                            .borrow_mut()
                            .push((label, element.element.element_id().unwrap()));

                        element
                    });
                }

                root
            }),
        });
        let expected = vec![
            ("every", ElementId::from("first")),
            ("nth", ElementId::from("late")),
            ("reordered", ElementId::from("late")),
            ("every", ElementId::from("late")),
            ("both", ElementId::from("late")),
        ];

        for redraw in 0..2 {
            selected.borrow_mut().clear();
            draw_selector_window(window.into(), cx);

            assert_eq!(*selected.borrow(), expected);
        }
    }

    #[crate::test]
    #[allow(unused_variables)]
    fn positional_selectors_exclude_list_measurements(cx: &mut TestAppContext) {
        let selected = Rc::new(RefCell::new(Vec::new()));
        let first = Rc::new(RefCell::new(Vec::new()));
        let selected_for_render = selected.clone();
        let first_for_render = first.clone();
        let state = ListState::new(8, ListAlignment::Top, px(0.)).measure_all();
        let window = cx.add_window(move |window, cx| SelectorTestView {
            render: Box::new(move || {
                let selected = selected_for_render.clone();
                let first = first_for_render.clone();

                div()
                    .flex()
                    .flex_col()
                    .w(px(80.))
                    .h(px(120.))
                    .child(
                        uniform_list("uniform", 8, |range, window, cx| {
                            range
                                .map(|idx| {
                                    div()
                                        .id(("uniform-row", idx))
                                        .h(px(20.))
                                        .class("row")
                                        .into_any_element()
                                })
                                .collect()
                        })
                        .h(px(60.))
                        .w_full(),
                    )
                    .child(
                        list(state.clone(), |idx, window, cx| {
                            div().id(("list-row", idx)).h(px(20.)).class("row")
                        })
                        .h(px(60.))
                        .w_full(),
                    )
                    .select(
                        Select::descendants().class("row").every(1),
                        move |element| {
                            selected
                                .borrow_mut()
                                .push(element.element.element_id().unwrap());

                            element
                        },
                    )
                    .select(Select::descendants().class("row").nth(0), move |element| {
                        first
                            .borrow_mut()
                            .push(element.element.element_id().unwrap());

                        element
                    })
            }),
        });
        let expected = ["uniform-row", "list-row"]
            .into_iter()
            .flat_map(|label| (0_usize..3).map(move |idx| ElementId::from((label, idx))))
            .collect::<Vec<_>>();

        for redraw in 0..2 {
            selected.borrow_mut().clear();
            first.borrow_mut().clear();
            draw_selector_window(window.into(), cx);

            assert_eq!(*selected.borrow(), expected);
            assert_eq!(
                *first.borrow(),
                vec![ElementId::from(("uniform-row", 0_usize))]
            );
        }
    }

    struct CachedSelectorChild {
        renders: Rc<Cell<usize>>,
    }

    impl Render for CachedSelectorChild {
        #[allow(unused_variables)]
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            self.renders.set(self.renders.get() + 1);

            div()
                .id("cached-row")
                .size_full()
                .debug_selector(|| "cached-row".into())
                .class("row")
        }
    }

    #[crate::test]
    #[allow(unused_variables)]
    fn selectors_visit_cached_view_contents_on_each_frame(cx: &mut TestAppContext) {
        let renders = Rc::new(Cell::new(0));
        let renders_for_child = renders.clone();
        let child = cx.new(move |cx| CachedSelectorChild {
            renders: renders_for_child,
        });
        let selected = Rc::new(Cell::new(0));
        let selected_for_render = selected.clone();
        let window = cx.add_window(move |window, cx| SelectorTestView {
            render: Box::new(move || {
                let selected = selected_for_render.clone();
                let mut style = StyleRefinement::default();
                style.size.width = Some(px(40.).into());
                style.size.height = Some(px(20.).into());

                div().child(child.clone().cached(style)).select(
                    Select::descendants().class("row").nth(0),
                    move |element| {
                        selected.set(selected.get() + 1);

                        element
                    },
                )
            }),
        });

        renders.set(0);
        selected.set(0);

        for redraw in 0..2 {
            draw_selector_window(window.into(), cx);

            let bounds = cx
                .update_window(window.into(), |view, window, cx| {
                    window.rendered_frame.debug_bounds["cached-row"]
                })
                .unwrap();

            assert_eq!(bounds.size.width, px(40.));
            assert_eq!(bounds.size.height, px(20.));
        }

        assert_eq!(renders.get(), 2);
        assert_eq!(selected.get(), 2);
    }

    #[test]
    #[should_panic(expected = "selector interval must be nonzero")]
    fn rejects_zero_intervals() {
        Select::children().every(0);
    }

    #[crate::test]
    #[allow(unused_variables)]
    fn restores_positions_when_prepaint_retries(cx: &mut TestAppContext) {
        let selected = Rc::new(RefCell::new(Vec::new()));
        let selected_for_render = selected.clone();
        let window = cx.add_window(move |window, cx| SelectorTestView {
            render: Box::new(move || {
                let selected = selected_for_render.clone();

                div()
                    .child(crate::container_query(|size, window, cx| {
                        let result = window.transact(|window| {
                            let mut discarded = div().id("discarded").class("row");
                            discarded.layout_as_root(
                                size.map(AvailableSpace::Definite),
                                window,
                                cx,
                            );

                            Err::<(), ()>(())
                        });

                        assert!(result.is_err());

                        div().id("committed").class("row")
                    }))
                    .select(Select::descendants().class("row").nth(0), move |element| {
                        selected
                            .borrow_mut()
                            .push(element.element.element_id().unwrap());

                        element
                    })
            }),
        });

        selected.borrow_mut().clear();
        draw_selector_window(window.into(), cx);

        assert_eq!(
            *selected.borrow(),
            vec![ElementId::from("discarded"), ElementId::from("committed")]
        );
    }

    #[test]
    fn restores_measurement_scope_after_unwinding() {
        let result = panic::catch_unwind(|| {
            with_selector_measurement(|| panic!("measurement failed"));
        });

        assert!(result.is_err());
        ELEMENT_TRAVERSAL.with_borrow(|traversal| assert_eq!(traversal.measurement_depth, 0));
    }

    struct SelectorView {
        this_count: Rc<Cell<usize>>,
        children_count: Rc<Cell<usize>>,
        descendants_count: Rc<Cell<usize>>,
        filtered_count: Rc<Cell<usize>>,
    }

    impl Render for SelectorView {
        #[allow(unused_variables)]
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let this_count = self.this_count.clone();
            let children_count = self.children_count.clone();
            let descendants_count = self.descendants_count.clone();
            let filtered_count = self.filtered_count.clone();

            div()
                .child(div().id("target").child(div()).class("card"))
                .child(div())
                .child(crate::container_query(|size, window, cx| div()))
                .select(Select::this().reflects(crate::Styled), move |element| {
                    this_count.set(this_count.get() + 1);

                    element.bg(rgb(0x111111))
                })
                .select(Select::children().reflects(crate::Styled), move |element| {
                    children_count.set(children_count.get() + 1);

                    element.bg(rgb(0x222222))
                })
                .select(
                    Select::descendants().reflects(crate::Styled),
                    move |element| {
                        descendants_count.set(descendants_count.get() + 1);

                        element.bg(rgb(0x333333))
                    },
                )
                .select(
                    Select::children()
                        .reflects(crate::reflection::trait_set!(
                            crate::Styled,
                            crate::ParentElement
                        ))
                        .id("target")
                        .class("card"),
                    move |element| {
                        fn require_traits<ElementType>(element: &ElementType)
                        where
                            ElementType: Element + Styled + ParentElement,
                        {
                            std::hint::black_box(element);
                        }

                        require_traits(&element);
                        filtered_count.set(filtered_count.get() + 1);

                        element.bg(rgb(0x444444))
                    },
                )
        }
    }

    #[crate::test]
    #[allow(unused_variables)]
    fn selects_elements_by_scope_and_predicates(cx: &mut TestAppContext) {
        let this_count = Rc::new(Cell::new(0));
        let children_count = Rc::new(Cell::new(0));
        let descendants_count = Rc::new(Cell::new(0));
        let filtered_count = Rc::new(Cell::new(0));
        let window: AnyWindowHandle = cx
            .add_window({
                let this_count = this_count.clone();
                let children_count = children_count.clone();
                let descendants_count = descendants_count.clone();
                let filtered_count = filtered_count.clone();

                move |window, cx| SelectorView {
                    this_count,
                    children_count,
                    descendants_count,
                    filtered_count,
                }
            })
            .into();

        this_count.set(0);
        children_count.set(0);
        descendants_count.set(0);
        filtered_count.set(0);

        cx.update_window(window, |view, window, cx| {
            window.draw(cx).clear(cx);
        })
        .unwrap();

        assert_eq!(this_count.get(), 1);
        assert_eq!(children_count.get(), 2);
        assert_eq!(descendants_count.get(), 4);
        assert_eq!(filtered_count.get(), 1);
    }
}
