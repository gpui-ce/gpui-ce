use crate::{
    AnyElement, ElementId, IntoElement, SharedString,
    reflection::{ReflectedElement, ReflectedTrait, ReflectedTraits, ReflectionGroup},
};
use smallvec::SmallVec;
use std::{cell::RefCell, marker::PhantomData};

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
    element_id: Option<ElementId>,
    classes: SmallVec<[SharedString; 2]>,
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
    /// Requires matching elements to have the given element ID.
    pub fn id(mut self, element_id: impl Into<ElementId>) -> Self {
        self.matcher.element_id = Some(element_id.into());

        self
    }

    /// Requires matching elements to have the given class.
    pub fn class(mut self, class: impl Into<SharedString>) -> Self {
        self.matcher.classes.push(class.into());

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
            && element.element_id().as_ref() != Some(element_id)
        {
            return false;
        }

        if !self
            .classes
            .iter()
            .all(|class| element.classes().contains(class))
        {
            return false;
        }

        true
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
    /// Tags this element with a class used by element selectors.
    fn class(self, class: impl Into<SharedString>) -> AnyElement {
        let mut element = self.into_any_element();
        element.add_class(class.into());

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
}

struct ActiveSelector {
    selector: SelectorMatcher,
    depth: usize,
    transform: Box<dyn FnMut(AnyElement) -> AnyElement>,
}

#[derive(Default)]
struct ElementTraversal {
    active_selectors: Vec<ActiveSelector>,
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
                selector: selector.selector,
                depth: 0,
                transform: selector.transform,
            }));

        selector_start
    });
    let traversal_guard = TraversalResetGuard;

    let result = operation();
    let selectors = ELEMENT_TRAVERSAL.with_borrow_mut(|traversal| {
        traversal
            .active_selectors
            .drain(selector_start..)
            .map(|selector| PendingSelector {
                selector: selector.selector,
                transform: selector.transform,
            })
            .collect()
    });
    drop(traversal_guard);

    (result, selectors)
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
        let matches = ELEMENT_TRAVERSAL.with_borrow(|traversal| {
            let selector = &traversal.active_selectors[selector_idx];

            selector.selector.matches(element, selector.depth)
        });

        if !matches {
            continue;
        }

        let mut selector = ELEMENT_TRAVERSAL
            .with_borrow_mut(|traversal| traversal.active_selectors.remove(selector_idx));
        let selected = element.take();
        let selected = (selector.transform)(selected);
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
        AnyWindowHandle, AppContext, Context, Element, InteractiveElement, ParentElement, Render,
        Styled, TestAppContext, Window, div, rgb,
    };
    use std::{cell::Cell, rc::Rc};

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
