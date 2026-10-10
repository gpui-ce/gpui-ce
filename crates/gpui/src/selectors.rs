use crate::{
    AnyElement, ElementId, ImageStyle, InteractiveElement, Interactivity, IntoElement,
    IntoItemMatch, IntoListMatch, IntoMatchValues, ItemMatch, ListMatch, ParentElement,
    ParentElementTyped, SharedString, StatefulInteractiveElement, StyleRefinement, Styled,
    StyledImage, TextStyleRefinement,
    reflection::{
        ElementReflection, ReflectedElement, ReflectedTraits, ReflectionGroup,
        ReflectionRequirement,
    },
    window::with_element_arena,
};
use smallvec::SmallVec;
use std::{
    cell::{Cell, Ref, RefCell, RefMut},
    marker::PhantomData,
    mem,
    num::NonZeroUsize,
    rc::Rc,
};

#[cfg(feature = "bench-support")]
#[doc(hidden)]
#[path = "selectors/benchmarks.rs"]
pub mod selector_benchmarks;

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
    reflected_traits: SmallVec<[ReflectionRequirement; 2]>,
    reflection_admission: Cell<[Option<ReflectionAdmission>; 2]>,
    element_id: Option<ItemMatch<ElementId>>,
    classes: SmallVec<[ListMatch<SharedString>; 2]>,
    nth: Option<usize>,
    every: Option<NonZeroUsize>,
}

#[derive(Clone, Copy)]
struct ReflectionAdmission {
    metadata: &'static ElementReflection,
    satisfies: bool,
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
    ///
    /// Components and entity views retain a [`crate::ViewElement`] whose child is
    /// the rendered root. Animation wrappers likewise retain their result as a child.
    /// `children()` on a wrapper reaches its child; on its parent it reaches the
    /// wrapper. Use [`Self::descendants`] to cross these boundaries.
    ///
    /// Annotations and deferred scheduling add no depth. Actual elements keep
    /// their declared children.
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
                reflection_admission: Cell::new([None; 2]),
                element_id: None,
                classes: SmallVec::new(),
                nth: None,
                every: None,
            },
            group: PhantomData,
        }
    }

    /// Requires trait membership and compatible callable tables, including reflected
    /// parents, before counting positions. Borrowed methods honor concrete implementations
    /// unless marked `#[reflect(wrapper_default)]`.
    pub fn reflects<Traits>(mut self, reflected_traits: Traits) -> Select<Traits::Group>
    where
        Traits: ReflectedTraits,
    {
        self.matcher
            .reflected_traits
            .extend(reflected_traits.reflected_requirements());

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

    /// Selects the matching element at a zero-based index in layout order.
    ///
    /// Measurements are excluded, and virtualized lists count only rendered elements.
    /// Prefer color changes over size changes when selecting list rows by position.
    pub fn nth(mut self, idx: usize) -> Self {
        self.matcher.nth = Some(idx);

        self
    }

    /// Selects matching elements at indexes `0, step, 2 * step, ...`.
    ///
    /// Uses the same indexing rules as [`Self::nth`].
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
    #[inline]
    fn matches_reflection(&self, metadata: &'static ElementReflection) -> bool {
        if self.reflected_traits.is_empty() {
            return true;
        }

        let cached = self.reflection_admission.get();

        for admission in cached.into_iter().flatten() {
            if std::ptr::eq(admission.metadata, metadata) {
                return admission.satisfies;
            }
        }

        self.resolve_reflection_admission(metadata)
    }

    #[cold]
    fn resolve_reflection_admission(&self, metadata: &'static ElementReflection) -> bool {
        // Immutable metadata makes admission reusable. Classes and IDs remain per-node checks.
        let satisfies = self
            .reflected_traits
            .iter()
            .all(|requirement| metadata.satisfies(*requirement));
        self.reflection_admission.set([
            Some(ReflectionAdmission {
                metadata,
                satisfies,
            }),
            self.reflection_admission.get()[0],
        ]);

        satisfies
    }

    fn matches_predicates(&self, element: &AnyElement) -> bool {
        if !self.matches_reflection(element.reflection()) {
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

    fn can_reach_view_contents(&self, depth: usize) -> bool {
        debug_assert!(depth >= 1);

        match self.scope {
            SelectScope::This => false,
            SelectScope::Children => depth == 1,
            SelectScope::Descendants => true,
        }
    }

    fn can_select_future_match(&self, next_match_idx: usize, measuring: bool) -> bool {
        if measuring && self.has_position() {
            return false;
        }

        self.nth.is_none_or(|expected| next_match_idx <= expected)
    }
}

/// An element builder with a pending class list or selector.
///
/// Preserves [`trait@Styled`], [`trait@ParentElement`], [`ParentElementTyped`],
/// [`StyledImage`], [`trait@InteractiveElement`], and [`trait@StatefulInteractiveElement`]
/// when the builder supports them.
/// Conversion attaches annotations to the same node and preserves its reflection.
/// Borrowed methods delegate; consuming trait methods use their defaults.
/// Call inherent or unsupported custom builder methods before annotating.
pub struct Annotated<ElementType> {
    element: ElementType,
    annotation: Annotation,
}

enum Annotation {
    Classes(SmallVec<[SharedString; 2]>),
    Selector(PendingSelector),
}

impl<ElementType: IntoElement> IntoElement for Annotated<ElementType> {
    type Element = AnyElement;

    fn into_element(self) -> Self::Element {
        let mut element = self.element.into_any_element();

        match self.annotation {
            Annotation::Classes(classes) => element.classes_mut().extend(classes),
            Annotation::Selector(selector) => element.add_selector(selector),
        }

        element
    }
}

impl<ElementType: Styled> Styled for Annotated<ElementType> {
    fn style(&mut self) -> &mut StyleRefinement {
        self.element.style()
    }

    fn text_style(&mut self) -> &mut TextStyleRefinement {
        self.element.text_style()
    }
}

impl<ElementType: ParentElement> ParentElement for Annotated<ElementType> {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.element.extend(elements);
    }
}

impl<ElementType: ParentElementTyped> ParentElementTyped for Annotated<ElementType> {
    type Child = ElementType::Child;

    fn extend(&mut self, elements: impl IntoIterator<Item = Self::Child>) {
        self.element.extend(elements);
    }
}

impl<ElementType: InteractiveElement> InteractiveElement for Annotated<ElementType> {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.element.interactivity()
    }
}

impl<ElementType: StatefulInteractiveElement> StatefulInteractiveElement
    for Annotated<ElementType>
{
}

impl<ElementType: StyledImage> StyledImage for Annotated<ElementType> {
    fn image_style(&mut self) -> &mut ImageStyle {
        self.element.image_style()
    }
}

/// Adds classes and selectors while preserving common builder traits.
///
/// Use [`IntoElement::into_any_element`] where an [`AnyElement`] is required.
///
/// ```
/// use gpui::{Select, div, prelude::*, rgb};
///
/// let icon = div().class("icon").bg(rgb(0x123456)).child(div());
/// let container = div()
///     .select(Select::children().class("icon").reflects(gpui::Styled),
///         |element| element.text_xl())
///     .bg(rgb(0x202020))
///     .child(icon)
///     .into_any_element();
/// ```
pub trait SelectableElement: IntoElement + Sized {
    /// Tags this element with one class or a collection of positive class names.
    ///
    /// Arrays, vectors, slices, and tuples are supported. Boolean expressions are
    /// only accepted by [`Select::class`].
    ///
    /// Classes stay on the converted node. Default component conversion tags its
    /// [`crate::ViewElement`]; use a class prop in `render` to tag the root.
    /// Tag animated children before wrapping them or inside the animator.
    ///
    /// ```compile_fail
    /// use gpui::{SelectableElement, div, not};
    ///
    /// let element = div().class(not("apple"));
    /// ```
    fn class<Kind>(self, class: impl IntoMatchValues<SharedString, Kind>) -> Annotated<Self> {
        let mut classes = SmallVec::new();
        class.extend_match_values(&mut classes);

        Annotated {
            element: self,
            annotation: Annotation::Classes(classes),
        }
    }

    /// Transforms matching elements before layout.
    ///
    /// The callback may run for measurements or retries, so keep results repeatable
    /// and avoid changing application state.
    /// Elements erased inside a callback keep its generation provenance even when
    /// retained for later use. The construction scope is synchronous and cannot
    /// cross an asynchronous suspension.
    fn select<Group, Output>(
        self,
        selector: Select<Group>,
        mut transform: impl FnMut(ReflectedElement<Group>) -> Output + 'static,
    ) -> Annotated<Self>
    where
        Group: ReflectionGroup,
        Output: IntoElement + 'static,
    {
        let transform = move |element| transform(ReflectedElement::new(element)).into_any_element();

        Annotated {
            element: self,
            annotation: Annotation::Selector(PendingSelector::new(
                selector.into_matcher(),
                Box::new(transform),
            )),
        }
    }
}

impl<ElementType> SelectableElement for ElementType where ElementType: IntoElement {}

pub(crate) struct ElementMetadata {
    pub(crate) classes: SmallVec<[SharedString; 2]>,
    pub(crate) selectors: Vec<PendingSelector>,
    pub(crate) node_state: Option<SelectorNodeState>,
}

impl ElementMetadata {
    pub(crate) fn new() -> Self {
        Self {
            classes: SmallVec::new(),
            selectors: Vec::new(),
            node_state: None,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct SelectorRuleId(Rc<()>);

impl PartialEq for SelectorRuleId {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for SelectorRuleId {}

#[derive(Clone, Debug)]
struct SelectorSessionId(Rc<()>);

impl PartialEq for SelectorSessionId {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for SelectorSessionId {}

struct SelectorVisit {
    rule: SelectorRuleId,
    session: SelectorSessionId,
}

#[derive(Default)]
pub(crate) struct SelectorNodeState {
    visited: SmallVec<[SelectorVisit; 2]>,
    pub(crate) generated_by: SmallVec<[SelectorRuleId; 2]>,
}

impl SelectorNodeState {
    pub(crate) fn generated(generated_by: SmallVec<[SelectorRuleId; 2]>) -> Self {
        Self {
            generated_by,
            ..Self::default()
        }
    }

    fn record_visit(&mut self, rule: &SelectorRuleId, session: &SelectorSessionId) -> bool {
        self.visited.retain(|visit| visit.session == *session);

        if self.visited.iter().any(|visit| visit.rule == *rule) {
            return false;
        }

        self.visited.push(SelectorVisit {
            rule: rule.clone(),
            session: session.clone(),
        });

        true
    }
}

type SelectorTransform = Box<dyn FnMut(AnyElement) -> AnyElement>;

#[derive(Clone)]
pub(crate) struct PendingSelector(Rc<SelectorRule>);

impl PendingSelector {
    fn new(matcher: SelectorMatcher, transform: SelectorTransform) -> Self {
        Self(Rc::new(SelectorRule {
            identity: SelectorRuleId(Rc::new(())),
            matcher,
            transform: RefCell::new(Some(transform)),
        }))
    }

    pub(crate) fn identity(&self) -> SelectorRuleId {
        self.0.identity.clone()
    }
}

struct SelectorRule {
    identity: SelectorRuleId,
    matcher: SelectorMatcher,
    transform: RefCell<Option<SelectorTransform>>,
}

struct SelectorCallbackLease<'rule> {
    rule: &'rule SelectorRule,
    transform: Option<SelectorTransform>,
}

impl SelectorRule {
    fn try_lease(&self) -> Option<SelectorCallbackLease<'_>> {
        let transform = self.transform.borrow_mut().take()?;

        Some(SelectorCallbackLease {
            rule: self,
            transform: Some(transform),
        })
    }
}

impl Drop for SelectorCallbackLease<'_> {
    fn drop(&mut self) {
        let transform = self
            .transform
            .take()
            .expect("selector callback lease is empty");
        self.rule.transform.borrow_mut().replace(transform);
    }
}

#[derive(Clone)]
struct SelectorBinding {
    rule: Rc<SelectorRule>,
    depth: usize,
    position: Option<Rc<Cell<usize>>>,
}

impl SelectorBinding {
    fn next_match_idx(&self) -> usize {
        self.position.as_ref().map_or(0, |position| position.get())
    }
}

struct SelectorPosition {
    rule: SelectorRuleId,
    next_match_idx: Rc<Cell<usize>>,
}

/// Owns visit identity and positional progress for a draw or standalone operation.
/// A session spans layout, prepaint, paint, deferred work, and retries.
struct SelectorSession {
    identity: SelectorSessionId,
    positions: RefCell<SmallVec<[SelectorPosition; 4]>>,
}

impl SelectorSession {
    fn new() -> Self {
        Self {
            identity: SelectorSessionId(Rc::new(())),
            positions: RefCell::new(SmallVec::new()),
        }
    }

    fn position(&self, rule: &SelectorRuleId) -> Rc<Cell<usize>> {
        let mut positions = self.positions.borrow_mut();

        if let Some(position) = positions.iter().find(|position| position.rule == *rule) {
            return position.next_match_idx.clone();
        }

        let next_match_idx = Rc::new(Cell::new(0));
        positions.push(SelectorPosition {
            rule: rule.clone(),
            next_match_idx: next_match_idx.clone(),
        });

        next_match_idx
    }

    fn checkpoint_positions(self: &Rc<Self>) -> SelectorPositionCheckpoint {
        let positions = self
            .positions
            .borrow()
            .iter()
            .map(|position| {
                (
                    position.next_match_idx.clone(),
                    position.next_match_idx.get(),
                )
            })
            .collect();

        SelectorPositionCheckpoint {
            session: self.clone(),
            positions: Some(positions),
        }
    }
}

/// Holds traversal state restored when a nested selector scope ends.
#[derive(Clone)]
struct SelectorScope {
    session: Rc<SelectorSession>,
    bindings: SmallVec<[SelectorBinding; 4]>,
    measurement_depth: usize,
    generated_by: SmallVec<[SelectorRuleId; 2]>,
}

impl SelectorScope {
    fn new() -> Self {
        Self {
            session: Rc::new(SelectorSession::new()),
            bindings: SmallVec::new(),
            measurement_depth: 0,
            generated_by: SmallVec::new(),
        }
    }

    fn activate_selectors(&mut self, selectors: &[PendingSelector]) {
        for selector in selectors {
            if self
                .bindings
                .iter()
                .any(|binding| binding.rule.identity == selector.0.identity)
            {
                continue;
            }

            let position = selector
                .0
                .matcher
                .has_position()
                .then(|| self.session.position(&selector.0.identity));
            self.bindings.push(SelectorBinding {
                rule: selector.0.clone(),
                depth: 0,
                position,
            });
        }
    }

    fn advance_depth(&mut self) {
        for binding in &mut self.bindings {
            binding.depth += 1;
        }
    }

    fn can_affect_view_contents(&self) -> bool {
        let measuring = self.measurement_depth > 0;

        self.bindings.iter().any(|binding| {
            binding.rule.matcher.can_reach_view_contents(binding.depth)
                && binding
                    .rule
                    .matcher
                    .can_select_future_match(binding.next_match_idx(), measuring)
        })
    }
}

#[derive(Clone)]
struct SelectorOwner(Rc<()>);

impl SelectorOwner {
    fn matches(&self, owner: &Self) -> bool {
        Rc::ptr_eq(&self.0, &owner.0)
    }
}

/// Coordinates selector scopes for one window.
pub(crate) struct SelectorRuntime {
    owner: SelectorOwner,
    active: Rc<RefCell<Option<SelectorScope>>>,
}

#[derive(Clone)]
pub(crate) struct SelectorRuntimeHandle {
    active: Rc<RefCell<Option<SelectorScope>>>,
}

impl SelectorRuntimeHandle {
    pub(crate) fn generation_ancestry(&self) -> SmallVec<[SelectorRuleId; 2]> {
        self.active
            .borrow()
            .as_ref()
            .map(|scope| scope.generated_by.clone())
            .unwrap_or_default()
    }
}

/// Captures traversal state while sharing its session's live position counters.
#[derive(Clone)]
pub(crate) struct CapturedSelectorScope {
    owner: SelectorOwner,
    scope: SelectorScope,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum SelectorPhase {
    RequestLayout,
    Prepaint,
    Paint,
}

#[must_use = "the construction binding lasts until this guard is dropped"]
pub(crate) struct SelectorConstructionGuard {
    // Restore the arena selected on entry, even after allocation routing changes.
    slot: Option<Rc<RefCell<Option<SelectorRuntimeHandle>>>>,
    previous: Option<SelectorRuntimeHandle>,
}

impl Drop for SelectorConstructionGuard {
    fn drop(&mut self) {
        if let Some(slot) = self.slot.take() {
            slot.replace(self.previous.take());
        }
    }
}

#[must_use = "the selector scope lasts until this guard is dropped"]
pub(crate) struct SelectorScopeGuard {
    active: Option<Rc<RefCell<Option<SelectorScope>>>>,
    previous: Option<SelectorScope>,
    _construction: SelectorConstructionGuard,
}

impl SelectorScopeGuard {
    fn unchanged(construction: SelectorConstructionGuard) -> Self {
        Self {
            active: None,
            previous: None,
            _construction: construction,
        }
    }
}

impl Drop for SelectorScopeGuard {
    fn drop(&mut self) {
        if let Some(active) = self.active.take() {
            active.replace(self.previous.take());
        }
    }
}

#[must_use = "the generation ancestry lasts until this guard is dropped"]
struct SelectorGenerationGuard<'a> {
    runtime: &'a SelectorRuntime,
    previous: SmallVec<[SelectorRuleId; 2]>,
    _construction: SelectorConstructionGuard,
}

impl Drop for SelectorGenerationGuard<'_> {
    fn drop(&mut self) {
        self.runtime.borrow_scope_mut().generated_by = mem::take(&mut self.previous);
    }
}

/// Restores positional progress on drop unless committed, including on unwind.
#[must_use = "selector positions roll back unless this checkpoint is committed"]
pub(crate) struct SelectorPositionCheckpoint {
    session: Rc<SelectorSession>,
    positions: Option<SmallVec<[(Rc<Cell<usize>>, usize); 4]>>,
}

impl SelectorPositionCheckpoint {
    /// Keeps position changes made since this checkpoint.
    pub(crate) fn commit(mut self) {
        self.positions.take();
    }
}

impl Drop for SelectorPositionCheckpoint {
    fn drop(&mut self) {
        let Some(positions) = self.positions.take() else {
            return;
        };

        let previous_count = positions.len();

        for (position, idx) in positions {
            position.set(idx);
        }

        // Keep newly registered cells for retained scopes and bindings.
        for position in self.session.positions.borrow().iter().skip(previous_count) {
            position.next_match_idx.set(0);
        }
    }
}

impl SelectorRuntime {
    pub(crate) fn new() -> Self {
        Self {
            owner: SelectorOwner(Rc::new(())),
            active: Rc::new(RefCell::new(None)),
        }
    }

    /// Binds element construction to this window for the guard's lifetime.
    pub(crate) fn bind_construction(&self) -> SelectorConstructionGuard {
        let slot = with_element_arena(|arena| arena.selector_runtime.clone());
        let unchanged = slot
            .borrow()
            .as_ref()
            .is_some_and(|runtime| Rc::ptr_eq(&runtime.active, &self.active));

        if unchanged {
            return SelectorConstructionGuard {
                slot: None,
                previous: None,
            };
        }

        let previous = slot.replace(Some(SelectorRuntimeHandle {
            active: self.active.clone(),
        }));

        SelectorConstructionGuard {
            slot: Some(slot),
            previous,
        }
    }

    /// Enters a fresh session, restoring the previous scope when the guard is dropped.
    pub(crate) fn enter_session(&self) -> SelectorScopeGuard {
        self.enter_scope(SelectorScope::new())
    }

    /// Reuses the active session or enters a fresh one for the guard's lifetime.
    pub(crate) fn enter_session_if_needed(&self) -> SelectorScopeGuard {
        if self.active.borrow().is_some() {
            return SelectorScopeGuard::unchanged(self.bind_construction());
        }

        self.enter_session()
    }

    pub(crate) fn enter_element(
        &self,
        element: &mut AnyElement,
        phase: SelectorPhase,
    ) -> SelectorScopeGuard {
        let selectors = element.attached_selectors();
        let unchanged = selectors.is_empty()
            && self.active.borrow().as_ref().is_some_and(|scope| {
                scope.bindings.is_empty()
                    && scope.generated_by == element.selector_generation_ancestry()
            });

        if unchanged {
            return SelectorScopeGuard::unchanged(self.bind_construction());
        }

        let scope = self
            .active
            .borrow()
            .clone()
            .unwrap_or_else(SelectorScope::new);
        let guard = self.enter_scope(scope);
        self.borrow_scope_mut().activate_selectors(&selectors);

        if phase == SelectorPhase::RequestLayout && element.is_before_layout() {
            self.apply(element);
        }

        // Apply selectors at the element's depth before advancing to its children.
        let mut scope = self.borrow_scope_mut();
        scope.generated_by = element.selector_generation_ancestry();
        scope.advance_depth();

        guard
    }

    pub(crate) fn enter_measurement(&self) -> SelectorScopeGuard {
        let mut scope = self
            .active
            .borrow()
            .clone()
            .unwrap_or_else(SelectorScope::new);
        scope.measurement_depth += 1;

        self.enter_scope(scope)
    }

    pub(crate) fn capture_scope(&self) -> CapturedSelectorScope {
        CapturedSelectorScope {
            owner: self.owner.clone(),
            scope: self.borrow_scope().clone(),
        }
    }

    /// Enters a captured scope from this window's currently active session.
    pub(crate) fn enter_captured_scope(
        &self,
        captured: CapturedSelectorScope,
    ) -> SelectorScopeGuard {
        assert!(
            self.owner.matches(&captured.owner),
            "captured selector scope belongs to another window"
        );
        assert!(
            self.borrow_scope().session.identity == captured.scope.session.identity,
            "captured selector scope belongs to another session"
        );

        self.enter_scope(captured.scope)
    }

    pub(crate) fn checkpoint_positions(&self) -> SelectorPositionCheckpoint {
        self.borrow_scope().session.checkpoint_positions()
    }

    pub(crate) fn can_affect_view_contents(&self) -> bool {
        self.borrow_scope().can_affect_view_contents()
    }

    fn borrow_scope(&self) -> Ref<'_, SelectorScope> {
        Ref::map(self.active.borrow(), |active| {
            active
                .as_ref()
                .expect("selector traversal requires an active session")
        })
    }

    fn borrow_scope_mut(&self) -> RefMut<'_, SelectorScope> {
        RefMut::map(self.active.borrow_mut(), |active| {
            active
                .as_mut()
                .expect("selector traversal requires an active session")
        })
    }

    fn enter_scope(&self, scope: SelectorScope) -> SelectorScopeGuard {
        let construction = self.bind_construction();
        let previous = self.active.replace(Some(scope));

        SelectorScopeGuard {
            active: Some(self.active.clone()),
            previous,
            _construction: construction,
        }
    }

    fn enter_generation(
        &self,
        generated_by: SmallVec<[SelectorRuleId; 2]>,
    ) -> SelectorGenerationGuard<'_> {
        let construction = self.bind_construction();
        let previous = mem::replace(&mut self.borrow_scope_mut().generated_by, generated_by);

        SelectorGenerationGuard {
            runtime: self,
            previous,
            _construction: construction,
        }
    }

    #[cfg(any(test, feature = "bench-support"))]
    fn enter_attached(&self, selectors: &[PendingSelector]) -> SelectorScopeGuard {
        let scope = self.borrow_scope().clone();
        let guard = self.enter_scope(scope);
        self.borrow_scope_mut().activate_selectors(selectors);

        guard
    }

    #[cfg(feature = "bench-support")]
    fn enter_children(&self) -> SelectorScopeGuard {
        let mut scope = self.borrow_scope().clone();
        scope.advance_depth();

        self.enter_scope(scope)
    }

    fn apply(&self, element: &mut AnyElement) {
        let mut selector_idx = 0;

        loop {
            let candidate = {
                let scope = self.borrow_scope();

                scope.bindings.get(selector_idx).cloned().map(|binding| {
                    (
                        binding,
                        scope.measurement_depth > 0,
                        scope.session.identity.clone(),
                    )
                })
            };

            let Some((binding, measuring, session)) = candidate else {
                break;
            };

            selector_idx += 1;
            let rule = binding.rule;

            if !rule.matcher.includes_depth(binding.depth)
                || (measuring && rule.matcher.has_position())
                || element
                    .selector_generation_ancestry()
                    .contains(&rule.identity)
                || rule.transform.borrow().is_none()
            {
                continue;
            }

            if !element
                .selector_node_state_mut()
                .record_visit(&rule.identity, &session)
                || !rule.matcher.matches_predicates(element)
            {
                continue;
            }

            if let Some(position) = binding.position {
                let idx = position.get();
                position.set(idx + 1);

                if !rule.matcher.includes_position(idx) {
                    continue;
                }
            }

            let Some(mut callback) = rule.try_lease() else {
                continue;
            };

            let attached = element.attached_selectors();
            let selected = element.take();
            let mut ancestry = selected.selector_generation_ancestry();
            ancestry.push(rule.identity.clone());

            let replacement = {
                let _generation = self.enter_generation(ancestry);

                callback
                    .transform
                    .as_mut()
                    .expect("selector callback lease is empty")(selected)
            };

            assert!(
                replacement.is_before_layout(),
                "selector replacement must not have requested layout"
            );

            element.replace(replacement);
            element.inherit_attached_selectors(attached);
            self.borrow_scope_mut()
                .activate_selectors(&element.attached_selectors());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reflection::test_fixtures::{self, TestElement, TestValue};
    use crate::window::DrawPhase;
    use crate::{
        Animation, AnimationExt, AnyView, AnyWindowHandle, App, AppContext, AvailableSpace, Bounds,
        Canvas, Context, Div, Element, Empty, Entity, FocusHandle, GlobalElementId,
        InspectorElementId, InteractiveElement, Interactivity, LayoutId, ListAlignment, ListState,
        ParentElement, Pixels, Render, RenderOnce, Size, SpringAnimation, SpringConfig,
        StatefulInteractiveElement, StyleRefinement, Styled, TestAppContext, TextStyleRefinement,
        VisualTestContext, Window, any, canvas, div, list, not, point, px,
        reflection::{
            ElementReflection, Reflect, ReflectedImplementation, ReflectionToken, trait_set,
        },
        rgb, rgb_to_hsla, size, uniform_list,
    };
    use std::{cell::Cell, panic, rc::Rc, time::Duration};

    #[gpui_macros::reflect_trait]
    trait StyledControl: crate::Styled + crate::StatefulInteractiveElement {}

    #[gpui_macros::reflect_trait(membership)]
    trait Tagged {}

    const FULL_REFLECTION: usize = 0;
    const MEMBERSHIP_ONLY: usize = 1;
    const MISSING_PARENT: usize = 2;

    struct TextOverride<const REGISTRATION: usize = FULL_REFLECTION> {
        inner: Div,
        text: TextStyleRefinement,
    }

    fn text_override(label: &'static str) -> TextOverride {
        text_override_with_reflection(label)
    }

    fn text_override_with_reflection<const REGISTRATION: usize>(
        label: &'static str,
    ) -> TextOverride<REGISTRATION> {
        TextOverride {
            inner: div().id(label).into_element(),
            text: TextStyleRefinement::default(),
        }
    }

    impl<const REGISTRATION: usize> Reflect for TextOverride<REGISTRATION> {
        fn build_reflection() -> Vec<ReflectedImplementation> {
            let mut implementations = Vec::new();
            StyledControl.__register::<Self>(&mut implementations);
            crate::ParentElement.__register::<Self>(&mut implementations);
            Tagged.__register::<Self>(&mut implementations);

            for implementation in &mut implementations {
                if REGISTRATION == MEMBERSHIP_ONLY
                    || (REGISTRATION == MISSING_PARENT
                        && implementation.descriptor == crate::Styled.reflected_trait())
                {
                    implementation.methods = None;
                }
            }

            implementations
        }
    }

    impl<const REGISTRATION: usize> StyledControl for TextOverride<REGISTRATION> {}

    impl<const REGISTRATION: usize> Tagged for TextOverride<REGISTRATION> {}

    impl<const REGISTRATION: usize> Styled for TextOverride<REGISTRATION> {
        fn style(&mut self) -> &mut StyleRefinement {
            self.inner.style()
        }

        fn text_style(&mut self) -> &mut TextStyleRefinement {
            &mut self.text
        }
    }

    impl<const REGISTRATION: usize> InteractiveElement for TextOverride<REGISTRATION> {
        fn interactivity(&mut self) -> &mut Interactivity {
            self.inner.interactivity()
        }
    }

    impl<const REGISTRATION: usize> StatefulInteractiveElement for TextOverride<REGISTRATION> {}

    impl<const REGISTRATION: usize> ParentElement for TextOverride<REGISTRATION> {
        fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
            self.inner.extend(elements);
        }
    }

    impl<const REGISTRATION: usize> IntoElement for TextOverride<REGISTRATION> {
        type Element = Self;

        fn into_element(self) -> Self {
            self
        }
    }

    impl<const REGISTRATION: usize> Element for TextOverride<REGISTRATION> {
        type RequestLayoutState = <Div as Element>::RequestLayoutState;
        type PrepaintState = <Div as Element>::PrepaintState;

        fn reflection(&self) -> &'static ElementReflection {
            <Self as Reflect>::reflection()
        }

        fn id(&self) -> Option<ElementId> {
            Element::id(&self.inner)
        }

        fn source_location(&self) -> Option<&'static panic::Location<'static>> {
            self.inner.source_location()
        }

        fn request_layout(
            &mut self,
            global_id: Option<&GlobalElementId>,
            inspector_id: Option<&InspectorElementId>,
            window: &mut Window,
            cx: &mut App,
        ) -> (LayoutId, Self::RequestLayoutState) {
            self.inner
                .request_layout(global_id, inspector_id, window, cx)
        }

        fn prepaint(
            &mut self,
            global_id: Option<&GlobalElementId>,
            inspector_id: Option<&InspectorElementId>,
            bounds: Bounds<Pixels>,
            request_layout: &mut Self::RequestLayoutState,
            window: &mut Window,
            cx: &mut App,
        ) -> Self::PrepaintState {
            self.inner
                .prepaint(global_id, inspector_id, bounds, request_layout, window, cx)
        }

        fn paint(
            &mut self,
            global_id: Option<&GlobalElementId>,
            inspector_id: Option<&InspectorElementId>,
            bounds: Bounds<Pixels>,
            request_layout: &mut Self::RequestLayoutState,
            prepaint: &mut Self::PrepaintState,
            window: &mut Window,
            cx: &mut App,
        ) {
            self.inner.paint(
                global_id,
                inspector_id,
                bounds,
                request_layout,
                prepaint,
                window,
                cx,
            );
        }
    }

    struct SelectorTestView<Builder> {
        render: Box<dyn Fn() -> Builder>,
    }

    impl<Builder: IntoElement + 'static> Render for SelectorTestView<Builder> {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            (self.render)()
        }
    }

    #[derive(gpui_macros::IntoElement)]
    struct SelectorComponent {
        root: AnyElement,
    }

    impl RenderOnce for SelectorComponent {
        fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
            self.root
        }
    }

    struct TypedContainer(Div);

    impl ParentElementTyped for TypedContainer {
        type Child = Div;

        fn extend(&mut self, elements: impl IntoIterator<Item = Self::Child>) {
            self.0
                .extend(elements.into_iter().map(IntoElement::into_any_element));
        }
    }

    impl IntoElement for TypedContainer {
        type Element = Div;

        fn into_element(self) -> Self::Element {
            self.0
        }
    }

    fn draw_selector_window(window: AnyWindowHandle, cx: &mut TestAppContext) {
        cx.update_window(window, |_view, window, cx| {
            window.draw(cx).clear(cx);
        })
        .unwrap();
    }

    fn record_match(matches: &Rc<RefCell<Vec<ElementId>>>, element: &AnyElement) {
        let mut matches = matches.borrow_mut();
        assert!(matches.len() < 16, "selector rewrite did not terminate");
        matches.push(element.element_id().unwrap());
    }

    fn draw_selector_tree<Builder: IntoElement>(
        cx: &mut TestAppContext,
        build: impl FnOnce() -> Builder,
    ) -> &mut VisualTestContext {
        let visual = cx.add_empty_window();
        visual.draw(
            Default::default(),
            size(px(100.), px(100.)),
            |_window, _cx| build().into_any_element(),
        );

        visual
    }

    #[track_caller]
    fn assert_matches(matches: &Rc<RefCell<Vec<ElementId>>>, expected: &[&'static str]) {
        let expected = expected
            .iter()
            .map(|label| ElementId::from(*label))
            .collect::<Vec<_>>();
        assert_eq!(*matches.borrow(), expected);
    }

    fn selector_row(label: &'static str) -> Div {
        div()
            .id(label)
            .debug_selector(move || label.into())
            .size(px(8.))
            .flex_shrink_0()
            .into_element()
    }

    fn style_component_children(element: impl IntoElement) -> AnyElement {
        element
            .select(
                Select::children().class("root").reflects(crate::Styled),
                |element| element.w(px(24.)),
            )
            .into_any_element()
    }

    fn layout_selector_row(label: &'static str, window: &mut Window, cx: &mut App) {
        let mut element = selector_row(label).class("row").into_any_element();
        element.layout_as_root(AvailableSpace::min_size(), window, cx);
    }

    fn deferred_selector_row(label: &'static str) -> AnyElement {
        crate::deferred(crate::container_query(move |_size, _window, _cx| {
            selector_row(label).class("row")
        }))
        .into_any_element()
    }

    fn construct_window_row(label: &'static str, window: &mut Window, cx: &mut App) -> AnyElement {
        window.invalidator.set_phase(DrawPhase::Prepaint);
        let mut probe = selector_row("window-probe")
            .class("row")
            .select(
                Select::this().class("row").reflects(crate::Styled).nth(0),
                |element| element.w(px(24.)),
            )
            .into_any_element();
        assert_eq!(
            probe.layout_as_root(AvailableSpace::min_size(), window, cx),
            size(px(24.), px(8.))
        );
        window.invalidator.set_phase(DrawPhase::None);

        selector_row(label).class("row").into_any_element()
    }

    fn reject_scope_and_layout(
        captured: CapturedSelectorScope,
        label: &'static str,
        selected: Rc<RefCell<Vec<ElementId>>>,
        window: &mut Window,
        cx: &mut App,
    ) {
        window.invalidator.set_phase(DrawPhase::Prepaint);
        let _session = window.selector_runtime().enter_session();
        let selector = PendingSelector::new(
            Select::this().class("row").nth(0).into_matcher(),
            Box::new(move |element| {
                record_match(&selected, &element);

                element
            }),
        );

        {
            let _attached = window.selector_runtime().enter_attached(&[selector]);
            let result = panic::catch_unwind(panic::AssertUnwindSafe(|| {
                let _captured_scope = window.selector_runtime().enter_captured_scope(captured);

                layout_selector_row("rejected", window, cx);
            }));

            assert!(result.is_err());

            layout_selector_row(label, window, cx);
        }

        window.invalidator.set_phase(DrawPhase::None);
    }

    fn selector_canvas<State: 'static>(
        label: &'static str,
        state: State,
        painted: Rc<RefCell<Vec<(&'static str, Size<Pixels>)>>>,
    ) -> Canvas<State> {
        canvas(
            move |_bounds, _window, _cx| state,
            move |bounds, _state, _window, _cx| {
                painted.borrow_mut().push((label, bounds.size));
            },
        )
        .size(px(8.))
    }

    fn layout_nested_transaction_row(
        fails: bool,
        attached: Rc<RefCell<Vec<ElementId>>>,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<(), ()> {
        let _scope = window
            .selector_runtime()
            .enter_captured_scope(window.selector_runtime().capture_scope());

        window.transact(|window| {
            window
                .selector_runtime()
                .borrow_scope_mut()
                .bindings
                .reverse();

            let mut element = selector_row("inner")
                .class("row")
                .select(Select::this().class("row").every(1), move |element| {
                    record_match(&attached, &element.element);

                    element
                })
                .into_any_element();
            element.layout_as_root(AvailableSpace::min_size(), window, cx);

            // Removed bindings still share progress with the enclosing scope.
            window
                .selector_runtime()
                .borrow_scope_mut()
                .bindings
                .clear();

            if fails {
                return Err(());
            }

            Ok(())
        })
    }

    fn list_retry_row(idx: usize, focus_handle: &FocusHandle) -> AnyElement {
        let label = ["row-0", "row-1", "row-2", "row-3", "row-4", "row-5"][idx];
        let mut row = selector_row(label).w_full();

        if idx == 2 {
            row = row.child(
                canvas(
                    |bounds, window, _cx| {
                        window.request_autoscroll(Bounds::from_corners(
                            point(bounds.left(), bounds.top() - px(36.)),
                            point(bounds.right(), bounds.top() + px(5.)),
                        ));
                    },
                    |_bounds, _state, _window, _cx| {},
                )
                .size_full(),
            );
        }

        if idx == 5 {
            return row
                .track_focus(focus_handle)
                .class("row")
                .into_any_element();
        }

        row.class("row").into_any_element()
    }

    #[crate::test]
    fn annotations_preserve_builder_mutations_and_rendered_paths(cx: &mut TestAppContext) {
        let selected = Rc::new(RefCell::new(Vec::new()));
        let observed = selected.clone();
        let color = rgb_to_hsla(rgb(0x123456));

        let mut overridden = text_override("overridden")
            .class("text")
            .text_color(color)
            .select(Select::this().reflects(crate::Styled), |element| element)
            .text_xl()
            .into_element();
        let concrete = overridden.downcast_mut::<TextOverride>().unwrap();

        assert_eq!(concrete.text.color, Some(color));
        assert!(concrete.text.font_size.is_some());
        assert!(concrete.style().text.color.is_none());

        let path_child = |label: &'static str| {
            canvas(
                move |_bounds, window, _cx| {
                    assert_eq!(
                        window.element_id_stack.as_slice(),
                        &[ElementId::from("container"), ElementId::from(label)],
                    );
                },
                |_bounds, _state, _window, _cx| {},
            )
            .size(px(1.))
        };

        let visual = draw_selector_tree(cx, move || {
            let class_first = div()
                .class(["icon", "icon"])
                .size(px(8.))
                .id("class-first")
                .on_click(|_event, _window, _cx| {})
                .class("extra")
                .child(path_child("class-first"));
            let id_first = div()
                .id("id-first")
                .class("icon")
                .on_click(|_event, _window, _cx| {})
                .size(px(8.))
                .child(path_child("id-first"));

            let container = div()
                .class("container")
                .select(
                    Select::children().class("icon").reflects(trait_set![
                        crate::Styled,
                        crate::InteractiveElement,
                        crate::ParentElement,
                    ]),
                    move |mut element| {
                        record_match(&observed, &element.element);
                        assert_eq!(element.style().size.width, Some(px(8.).into()));
                        assert_eq!(element.interactivity().click_listeners.len(), 1);
                        let label = element.element.element_id().unwrap().to_string();

                        element
                            .class("selected")
                            .size(px(24.))
                            .debug_selector(move || label)
                            .child(selector_row("added-child"))
                    },
                )
                .bg(color)
                .id("container")
                .child(class_first)
                .child(id_first)
                .child(
                    TypedContainer(div())
                        .class("typed")
                        .child(selector_row("typed-child"))
                        .children([selector_row("typed-sibling")]),
                )
                .select(
                    Select::descendants()
                        .class("selected")
                        .reflects(crate::Styled),
                    |element| element.h(px(16.)),
                );

            div().child(container)
        });

        assert_matches(&selected, &["class-first", "id-first"]);

        for (label, width, height) in [
            ("class-first", 24., 16.),
            ("id-first", 24., 16.),
            ("typed-child", 8., 8.),
            ("typed-sibling", 8., 8.),
            ("added-child", 8., 8.),
        ] {
            assert_eq!(
                visual.debug_bounds(label).unwrap().size,
                size(px(width), px(height))
            );
        }
    }

    #[test]
    fn annotated_conversion_preserves_existing_nodes_and_annotation_order() {
        for route_idx in 0..8 {
            let mut original = div()
                .id("original")
                .class("first")
                .select(Select::this(), |element| element)
                .into_any_element();
            let metadata = original.reflection();
            let concrete = original.downcast_mut::<Div>().unwrap() as *mut Div;
            let rule = original.attached_selectors()[0].identity();
            let session = SelectorSessionId(Rc::new(()));
            let state = original.selector_node_state_mut();
            state.record_visit(&rule, &session);
            state.generated_by.push(rule.clone());

            let reflected = ReflectedElement::<
                <crate::__GpuiReflectInteractiveElement as ReflectionToken>::Group,
            >::new(original);
            let annotated = reflected
                .class(["second", "first"])
                .select(Select::this(), |element| element);
            let Annotation::Selector(second_rule) = &annotated.annotation else {
                unreachable!();
            };

            let second_rule = second_rule.identity();

            let annotated = annotated
                .id("renamed")
                .class("third")
                .select(Select::this(), |element| element);
            let Annotation::Selector(third_rule) = &annotated.annotation else {
                unreachable!();
            };

            let third_rule = third_rule.identity();

            let mut element = match route_idx {
                0 => annotated.into_any_element(),
                1 => annotated.into_element(),
                2 => annotated.into_element().into_any(),
                3 => annotated.into_any_element().into_any(),
                4 => annotated.into_element().class("last").into_any_element(),
                5 => annotated
                    .into_any_element()
                    .class("last")
                    .into_element()
                    .into_any(),
                6 => annotated.id("renamed").into_any_element(),
                _route => annotated.id("renamed").into_element().into_any(),
            };

            assert_eq!(element.downcast_mut::<Div>().unwrap() as *mut Div, concrete);
            assert!(std::ptr::eq(metadata, element.reflection()));
            assert_eq!(element.element_id(), Some(ElementId::from("renamed")));

            let mut classes = ["first", "second", "first", "third"]
                .map(SharedString::from)
                .to_vec();

            if (4..6).contains(&route_idx) {
                classes.push("last".into());
            }

            assert_eq!(element.classes(), classes);
            let selectors = element.attached_selectors();
            assert_eq!(selectors.len(), 3);
            assert_eq!(selectors[0].identity(), rule);
            assert_eq!(selectors[1].identity(), second_rule);
            assert_eq!(selectors[2].identity(), third_rule);

            let state = element.selector_node_state_mut();
            assert_eq!(state.visited.len(), 1);
            assert_eq!(state.visited[0].rule, rule);
            assert_eq!(state.visited[0].session, session);
            assert!(state.generated_by.as_slice() == [rule]);
        }
    }

    #[crate::test]
    fn wrapping_preserves_identity_metadata_and_original_descendants(cx: &mut TestAppContext) {
        let wrapped = Rc::new(RefCell::new(Vec::new()));
        let wrapper_children = Rc::new(RefCell::new(Vec::new()));
        let styled = Rc::new(RefCell::new(Vec::new()));

        let matched = wrapped.clone();
        let children = wrapper_children.clone();
        let renamed = styled.clone();
        let attached = styled.clone();

        let visual = draw_selector_tree(cx, move || {
            div()
                .child(
                    div()
                        .id("outer")
                        .debug_selector(|| "outer".into())
                        .child(
                            div()
                                .id("inner")
                                .debug_selector(|| "inner".into())
                                .class(["icon", "leaf"]),
                        )
                        .class("icon")
                        .select(
                            Select::descendants().class("leaf").reflects(crate::Styled),
                            move |element| {
                                record_match(&attached, &element.element);

                                element.size(px(12.))
                            },
                        ),
                )
                .select(
                    Select::descendants()
                        .id("outer")
                        .reflects(crate::InteractiveElement),
                    |element| element.id("renamed"),
                )
                .select(
                    Select::descendants()
                        .id("renamed")
                        .class("icon")
                        .reflects(crate::Styled),
                    move |element| {
                        record_match(&renamed, &element.element);

                        element.size(px(48.))
                    },
                )
                .select(Select::descendants().class("icon"), move |element| {
                    record_match(&matched, &element.element);
                    let children = children.clone();

                    div()
                        .child(element)
                        .select(Select::children(), move |element| {
                            record_match(&children, &element.element);

                            element
                        })
                })
        });

        assert_matches(&wrapped, &["renamed", "inner"]);
        assert_eq!(*wrapper_children.borrow(), *wrapped.borrow());
        assert_matches(&styled, &["renamed", "inner"]);
        assert_eq!(
            visual.debug_bounds("outer").unwrap().size,
            size(px(48.), px(48.))
        );
        assert_eq!(
            visual.debug_bounds("inner").unwrap().size,
            size(px(12.), px(12.))
        );
    }

    #[crate::test]
    fn component_and_animation_classes_preserve_capabilities_and_depth(cx: &mut TestAppContext) {
        let window = cx.open_window(size(px(100.), px(100.)), |_window, _cx| SelectorTestView {
            render: Box::new(|| {
                let component = style_component_children(
                    SelectorComponent {
                        root: selector_row("component-root")
                            .child(
                                SelectorComponent {
                                    root: selector_row("nested-root")
                                        .class("root")
                                        .into_any_element(),
                                }
                                .class("nested-wrapper"),
                            )
                            .class("root")
                            .into_any_element(),
                    }
                    .class("wrapper"),
                );
                let animated = selector_row("animated-root")
                    .class("root")
                    .with_animation(
                        "animation",
                        Animation::new(Duration::from_secs(1)),
                        |element, _progress| element.w(px(12.)),
                    )
                    .class("wrapper")
                    .child(selector_row("animation-child").class("root"));
                let spring = selector_row("spring-root")
                    .into_any_element()
                    .with_spring(
                        "spring",
                        SpringAnimation::new(SpringConfig::new(100., 10., 1.)).to(px(0.)),
                        |element, _value| element.class("root").into_any_element(),
                    )
                    .class("wrapper");
                let card = style_component_children(
                    TestElement::new("card")
                        .size(px(8.))
                        .child(selector_row("card-child").class("root"))
                        .class("card"),
                );
                let wrappers = [
                    component,
                    animated.into_any_element(),
                    spring.into_any_element(),
                ];

                for element in &wrappers {
                    assert_eq!(element.classes(), &[SharedString::from("wrapper")]);
                    assert!(!element.implements_trait(crate::Styled));
                    assert!(!element.implements_trait(TestValue));
                }

                div()
                    .children(wrappers)
                    .child(card)
                    .child(selector_row("plain").class("wrapper"))
                    .select(
                        Select::children().class("root").reflects(crate::Styled),
                        |_element| -> AnyElement {
                            panic!("children skipped a structural wrapper")
                        },
                    )
                    .select(
                        Select::children()
                            .class("wrapper")
                            .reflects(crate::Styled)
                            .nth(0),
                        |element| element.w(px(32.)),
                    )
                    .select(
                        Select::descendants().class("root").reflects(crate::Styled),
                        |element| {
                            assert_eq!(element.element.classes(), &[SharedString::from("root")]);

                            element.h(px(16.))
                        },
                    )
            }),
        });

        draw_selector_window(window.into(), cx);

        cx.update_window(window.into(), |_view, window, _cx| {
            for (label, width, height) in [
                ("component-root", 24., 16.),
                ("nested-root", 8., 16.),
                ("animated-root", 12., 16.),
                ("animation-child", 8., 16.),
                ("spring-root", 8., 16.),
                ("card-child", 24., 16.),
                ("plain", 32., 8.),
            ] {
                assert_eq!(
                    window.rendered_frame.debug_bounds[label].size,
                    size(px(width), px(height))
                );
            }
        })
        .unwrap();
    }

    #[crate::test]
    fn dispatch_and_generation_survive_mutation_and_replacement(cx: &mut TestAppContext) {
        let initial_color = rgb_to_hsla(rgb(0x123456));
        let later_color = rgb_to_hsla(rgb(0x654321));

        let generating = Rc::new(RefCell::new(Vec::new()));
        let reflected = Rc::new(RefCell::new(Vec::new()));
        let later = Rc::new(RefCell::new(Vec::new()));
        let installed = Rc::new(RefCell::new(Vec::new()));

        let first = generating.clone();
        let dispatch = reflected.clone();
        let second = later.clone();
        let new_rules = installed.clone();

        let visual = draw_selector_tree(cx, move || {
            let mut prebuilt = Some(div().id("prebuilt").class("icon").into_any_element());
            let mut concrete = Some(div().id("captured-concrete"));

            div()
                .child(text_override("mutated").class("icon"))
                .child(div().id("replaced").class("icon"))
                .select(
                    Select::descendants().class("icon").reflects(trait_set!(
                        crate::Styled,
                        crate::InteractiveElement,
                        crate::ParentElement
                    )),
                    move |element| {
                        record_match(&first, &element.element);

                        let mut element = element.text_color(initial_color);
                        let label = element.element.element_id().unwrap();
                        let expected_base_color = if label == ElementId::from("mutated") {
                            None
                        } else {
                            Some(initial_color)
                        };

                        assert_eq!(element.text_style().color, Some(initial_color));
                        assert_eq!(element.style().text.color, expected_base_color);

                        if label == ElementId::from("replaced") {
                            return text_override("from-first")
                                .debug_selector(|| "from-first".into())
                                .text_color(initial_color)
                                .child(div().id("fresh-child").class("icon"))
                                .class("icon")
                                .into_any_element();
                        }

                        if label == ElementId::from("mutated") {
                            return element
                                .id("renamed")
                                .child(div().id("appended").class("icon"))
                                .child(prebuilt.take().unwrap())
                                .child(concrete.take().unwrap().class("icon"))
                                .into_any_element();
                        }

                        element.into_any_element()
                    },
                )
                .select(
                    Select::descendants().class("icon").reflects(StyledControl),
                    move |mut element| {
                        record_match(&dispatch, &element.element);
                        let label = element.element.element_id();

                        assert_eq!(element.text_style().color, Some(initial_color));
                        assert!(element.style().text.color.is_none());
                        assert_eq!(element.interactivity().element_id, label);

                        let mut element = element.text_color(later_color);
                        let concrete = element.element.downcast_mut::<TextOverride>().unwrap();

                        assert_eq!(concrete.text.color, Some(later_color));

                        element.id(label.unwrap())
                    },
                )
                .select(Select::descendants().class("icon"), move |element| {
                    record_match(&second, &element.element);

                    if element.element.element_id() != Some(ElementId::from("from-first")) {
                        return element.into_any_element();
                    }

                    let this = new_rules.clone();
                    let children = new_rules.clone();

                    div()
                        .id("from-second")
                        .debug_selector(|| "from-second".into())
                        .child(element)
                        .child(
                            div()
                                .id("generated-child")
                                .debug_selector(|| "generated-child".into())
                                .class("icon"),
                        )
                        .class("icon")
                        .select(Select::this().reflects(crate::Styled), move |element| {
                            record_match(&this, &element.element);

                            element.size(px(40.))
                        })
                        .select(Select::children().reflects(crate::Styled), move |element| {
                            record_match(&children, &element.element);

                            element.size(px(8.))
                        })
                        .into_any_element()
                })
        });

        assert_matches(&generating, &["mutated", "prebuilt", "replaced"]);
        assert_matches(&reflected, &["renamed", "from-first"]);
        assert_matches(
            &later,
            &[
                "renamed",
                "appended",
                "prebuilt",
                "captured-concrete",
                "from-first",
                "fresh-child",
            ],
        );
        assert_matches(
            &installed,
            &["from-second", "from-first", "generated-child"],
        );
        assert_eq!(
            visual.debug_bounds("from-second").unwrap().size,
            size(px(40.), px(40.))
        );

        for label in ["from-first", "generated-child"] {
            assert_eq!(
                visual.debug_bounds(label).unwrap().size,
                size(px(8.), px(8.))
            );
        }
    }

    #[crate::test]
    fn generic_and_manual_metadata_survive_selection_and_replacement(cx: &mut TestAppContext) {
        let painted = Rc::new(RefCell::new(Vec::new()));
        let styled = Rc::new(Cell::new(0));
        let rebound = Rc::new(Cell::new(0));
        let attached = Rc::new(Cell::new(0));

        let initial = styled.clone();
        let later = rebound.clone();
        let inherited = attached.clone();
        let output = painted.clone();

        draw_selector_tree(cx, move || {
            let replacement_output = output.clone();
            let replaceable = selector_canvas("original", 7_u32, output.clone())
                .class(["reflected", "replace"])
                .select(
                    Select::this().reflects(crate::Styled),
                    move |mut element| {
                        assert!(element.element.downcast_mut::<Canvas<String>>().is_some());
                        inherited.set(inherited.get() + 1);

                        element.h(px(9.))
                    },
                );

            div()
                .child(selector_canvas("integer", 42_u32, output.clone()).class("reflected"))
                .child(selector_canvas("string", String::from("state"), output).class("reflected"))
                .child(replaceable)
                .child(text_override("manual").class("reflected"))
                .child(Empty.class("reflected"))
                .select(
                    Select::children()
                        .class("reflected")
                        .reflects(crate::Styled),
                    move |element| {
                        initial.set(initial.get() + 1);
                        let mut element = element.w(px(20.));
                        assert_eq!(element.style().size.width, Some(px(20.).into()));

                        let metadata = element.element.reflection();
                        let replace = element.element.classes().contains(&"replace".into());
                        let mut erased = Element::into_any(element.into_element())
                            .class("reflected")
                            .into_any_element();
                        assert!(std::ptr::eq(metadata, erased.reflection()));

                        if !replace {
                            return erased;
                        }

                        assert!(erased.downcast_mut::<Canvas<u32>>().is_some());
                        let replacement = selector_canvas(
                            "replacement",
                            String::from("new state"),
                            replacement_output.clone(),
                        )
                        .class("reflected")
                        .into_any_element();
                        assert!(!std::ptr::eq(metadata, replacement.reflection()));
                        assert!(!replacement.classes().contains(&"replace".into()));

                        replacement
                    },
                )
                .select(
                    Select::children()
                        .class("reflected")
                        .reflects(crate::Styled),
                    move |element| {
                        later.set(later.get() + 1);

                        element.w(px(30.))
                    },
                )
        });

        assert_eq!(styled.get(), 4);
        assert_eq!(rebound.get(), 4);
        assert_eq!(attached.get(), 1);
        assert_eq!(
            *painted.borrow(),
            vec![
                ("integer", size(px(30.), px(8.))),
                ("string", size(px(30.), px(8.))),
                ("replacement", size(px(30.), px(9.))),
            ]
        );
    }

    #[crate::test]
    fn callable_admission_precedes_positions_and_checks_inherited_tables(cx: &mut TestAppContext) {
        let membership = Rc::new(RefCell::new(Vec::new()));
        let indexed = Rc::new(RefCell::new(Vec::new()));
        let spaced = Rc::new(RefCell::new(Vec::new()));

        let erased = membership.clone();
        let nth = indexed.clone();
        let every = spaced.clone();

        draw_selector_tree(cx, move || {
            let missing_parent = text_override_with_reflection::<MISSING_PARENT>("missing-parent")
                .class("row")
                .into_any_element();
            assert!(missing_parent.implements_trait(StyledControl));
            assert!(!missing_parent.reflection().has_adapter(crate::Styled));

            div()
                .child(
                    text_override_with_reflection::<MEMBERSHIP_ONLY>("membership-first")
                        .class("row"),
                )
                .child(text_override("first").class("row"))
                .child(text_override("unclassified"))
                .child(missing_parent)
                .child(
                    text_override_with_reflection::<MEMBERSHIP_ONLY>("membership-second")
                        .class("row"),
                )
                .child(text_override_with_reflection::<MEMBERSHIP_ONLY>(
                    "membership-hidden",
                ))
                .child(text_override("second").class("row"))
                .child(
                    canvas(
                        |_bounds, _window, _cx| 42_u32,
                        |_bounds, _state, _window, _cx| {},
                    )
                    .class("row"),
                )
                .child(Empty.class("row"))
                .child(text_override("third").class("row"))
                .select(
                    Select::children()
                        .class("row")
                        .reflects(StyledControl.reflected_trait()),
                    move |element| {
                        record_match(&erased, &element.element);

                        element
                    },
                )
                .select(
                    Select::children()
                        .class("row")
                        .reflects(trait_set![Tagged, StyledControl, Tagged])
                        .nth(1),
                    move |mut element| {
                        record_match(&nth, &element.element);
                        element.style().size.width = Some(px(20.).into());
                        let concrete = element.element.downcast_mut::<TextOverride>().unwrap();
                        assert_eq!(concrete.style().size.width, Some(px(20.).into()));

                        let mut element = element.class("composed").text_color(rgb(0x654321));
                        assert_eq!(element.text_style().color, Some(rgb_to_hsla(rgb(0x654321))));

                        element
                    },
                )
                .select(
                    Select::children()
                        .class("row")
                        .reflects(trait_set!(
                            StyledControl,
                            crate::ParentElement,
                            StyledControl
                        ))
                        .every(2),
                    move |mut element| {
                        record_match(&every, &element.element);
                        element.text_style().color = Some(rgb_to_hsla(rgb(0x123456)));
                        let concrete = element.element.downcast_mut::<TextOverride>().unwrap();
                        assert_eq!(concrete.text.color, Some(rgb_to_hsla(rgb(0x123456))));

                        element
                    },
                )
        });

        assert_matches(
            &membership,
            &[
                "membership-first",
                "first",
                "missing-parent",
                "membership-second",
                "second",
                "third",
            ],
        );
        assert_matches(&indexed, &["second"]);
        assert_matches(&spaced, &["first", "third"]);
    }

    #[crate::test]
    fn generated_factories_preserve_origins_and_original_deferred_subtrees(
        cx: &mut TestAppContext,
    ) {
        let generating = Rc::new(RefCell::new(Vec::new()));
        let later = Rc::new(RefCell::new(Vec::new()));

        let generating_for_render = generating.clone();
        let later_for_render = later.clone();
        let window = cx.add_window(move |_window, _cx| SelectorTestView {
            render: Box::new(move || {
                let matches = generating_for_render.clone();
                let concrete = generating_for_render.clone();
                let later = later_for_render.clone();

                div()
                    .size_full()
                    .child(
                        div()
                            .id("original")
                            .child(div().id("original-child").class("icon"))
                            .child(crate::container_query(|_size, _window, _cx| {
                                div().id("original-late").class("icon")
                            }))
                            .class("icon"),
                    )
                    .child(crate::deferred(crate::container_query(
                        |_size, _window, _cx| div().id("ordinary-late").class("icon"),
                    )))
                    .child(div().id("concrete-original").class("factory"))
                    .select(Select::descendants().class("icon"), move |element| {
                        record_match(&matches, &element.element);

                        if element.element.element_id() != Some(ElementId::from("original")) {
                            return element.into_any_element();
                        }

                        crate::deferred(crate::container_query(move |_size, _window, _cx| {
                            div()
                                .id("generated-late")
                                .child(element)
                                .child(crate::deferred(crate::container_query(
                                    |_size, _window, _cx| {
                                        div().id("nested-generated").class("icon")
                                    },
                                )))
                                .class("icon")
                        }))
                        .into_any_element()
                    })
                    .select(Select::descendants().class("factory"), move |element| {
                        record_match(&concrete, &element.element);

                        crate::container_query(|_size, _window, _cx| {
                            div().id("concrete-generated").class("factory")
                        })
                    })
                    .select(
                        Select::descendants()
                            .class(any(["icon", "factory"]))
                            .reflects(crate::Styled),
                        move |element| {
                            record_match(&later, &element.element);

                            element.size(px(16.))
                        },
                    )
            }),
        });

        generating.borrow_mut().clear();
        later.borrow_mut().clear();
        draw_selector_window(window.into(), cx);

        assert_matches(
            &generating,
            &[
                "original",
                "concrete-original",
                "original-child",
                "original-late",
                "ordinary-late",
            ],
        );
        assert_matches(
            &later,
            &[
                "concrete-generated",
                "generated-late",
                "original",
                "original-child",
                "original-late",
                "ordinary-late",
                "nested-generated",
            ],
        );
    }

    #[crate::test]
    fn nested_deferred_scopes_share_positions_and_callback_state(cx: &mut TestAppContext) {
        let visited = Rc::new(RefCell::new(Vec::new()));
        let direct = Rc::new(RefCell::new(Vec::new()));
        let indexed = Rc::new(RefCell::new(Vec::new()));

        let visited_for_render = visited.clone();
        let direct_for_render = direct.clone();
        let indexed_for_render = indexed.clone();
        let window = cx.add_window(move |_window, _cx| SelectorTestView {
            render: Box::new(move || {
                let visited = visited_for_render.clone();
                let direct = direct_for_render.clone();
                let late_direct = direct_for_render.clone();
                let nested_direct = direct_for_render.clone();
                let indexed = indexed_for_render.clone();
                let mut match_count = 0;

                let nested = crate::deferred(
                    crate::container_query(|_size, _window, _cx| {
                        selector_row("nested-first")
                            .child(selector_row("nested-second").class("row"))
                            .class("row")
                    })
                    .select(
                        Select::children().class("row").reflects(crate::Styled),
                        move |element| {
                            record_match(&nested_direct, &element.element);

                            element.h(px(16.))
                        },
                    ),
                )
                .priority(0);
                let low_priority = crate::deferred(
                    crate::container_query(move |_size, _window, _cx| {
                        selector_row("late-low").child(nested).class("row")
                    })
                    .select(
                        Select::children().class("row").reflects(crate::Styled),
                        move |element| {
                            record_match(&late_direct, &element.element);

                            element.h(px(12.))
                        },
                    ),
                )
                .priority(10);

                div()
                    .size_full()
                    .child(selector_row("synchronous").class("row"))
                    .child(
                        crate::deferred(crate::container_query(|_size, _window, _cx| {
                            selector_row("late-high").class("row")
                        }))
                        .priority(20),
                    )
                    .child(low_priority)
                    .select(
                        Select::children().class("row").reflects(crate::Styled),
                        move |element| {
                            record_match(&direct, &element.element);

                            element.h(px(10.))
                        },
                    )
                    .select(Select::descendants().class("row"), move |element| {
                        record_match(&visited, &element.element);

                        element
                    })
                    .select(
                        Select::descendants()
                            .class("row")
                            .reflects(crate::Styled)
                            .every(2),
                        move |element| {
                            record_match(&indexed, &element.element);
                            match_count += 1;

                            element.w(px(10. + 10. * match_count as f32))
                        },
                    )
            }),
        });

        for _redraw in 0..2 {
            visited.borrow_mut().clear();
            direct.borrow_mut().clear();
            indexed.borrow_mut().clear();
            draw_selector_window(window.into(), cx);

            assert_matches(
                &visited,
                &[
                    "synchronous",
                    "late-low",
                    "late-high",
                    "nested-first",
                    "nested-second",
                ],
            );
            assert_matches(&direct, &["synchronous", "late-low", "nested-first"]);
            assert_matches(&indexed, &["synchronous", "late-high", "nested-second"]);

            cx.update_window(window.into(), |_view, window, _cx| {
                for (label, width, height) in [
                    ("synchronous", 20., 10.),
                    ("late-low", 8., 12.),
                    ("late-high", 30., 8.),
                    ("nested-first", 8., 16.),
                    ("nested-second", 40., 8.),
                ] {
                    assert_eq!(
                        window.rendered_frame.debug_bounds[label].size,
                        size(px(width), px(height))
                    );
                }
            })
            .unwrap();
        }
    }

    #[crate::test]
    fn positions_ignore_generated_nodes_and_repeated_or_abandoned_layout(cx: &mut TestAppContext) {
        for select_nth in [false, true] {
            let failed = Rc::new(Cell::new(0));
            let indexed = Rc::new(RefCell::new(Vec::new()));

            let failed_count = failed.clone();
            let matched = indexed.clone();

            let selector = Select::descendants().class("row");
            let selector = if select_nth {
                selector.nth(1)
            } else {
                selector.every(2)
            };

            let retained_size = if select_nth { px(5.) } else { px(20.) };

            draw_selector_tree(cx, move || {
                div()
                    .child(crate::container_query(move |_size, window, cx| {
                        let mut retained = div()
                            .id(("row", 0_usize))
                            .size(px(5.))
                            .class("row")
                            .into_any_element();
                        let available = AvailableSpace::min_size();
                        let expected_size = size(retained_size, retained_size);
                        let result = window.transact(|window| {
                            assert_eq!(
                                retained.layout_as_root(available, window, cx),
                                expected_size
                            );

                            Err::<(), ()>(())
                        });

                        assert!(result.is_err());
                        for available in [
                            available,
                            size(
                                AvailableSpace::Definite(px(50.)),
                                AvailableSpace::MinContent,
                            ),
                        ] {
                            assert_eq!(
                                retained.layout_as_root(available, window, cx),
                                expected_size
                            );
                        }

                        // A new session cannot make a requested drawable transform again.
                        {
                            let _session = window.selector_runtime().enter_session();
                            assert_eq!(
                                retained.layout_as_root(available, window, cx),
                                expected_size
                            );
                        }

                        div().children((1_usize..5).map(|idx| div().id(("row", idx)).class("row")))
                    }))
                    .select(Select::descendants().class("renamed"), move |element| {
                        failed_count.set(failed_count.get() + 1);

                        element
                    })
                    .select(selector, move |element| {
                        record_match(&matched, &element.element);

                        div()
                            .size(px(20.))
                            .child(element.class("renamed"))
                            .class("row")
                    })
            });

            let expected = if select_nth {
                vec![2_usize]
            } else {
                vec![0_usize, 1, 3]
            };

            let expected = expected
                .into_iter()
                .map(|idx| ElementId::from(("row", idx)))
                .collect::<Vec<_>>();
            assert_eq!(failed.get(), 0);
            assert_eq!(*indexed.borrow(), expected);
        }
    }

    #[crate::test]
    fn caught_transform_panics_restore_callbacks_and_construction_context(cx: &mut TestAppContext) {
        let selected = Rc::new(RefCell::new(Vec::new()));
        let observed = selected.clone();
        let recovered = selected.clone();

        draw_selector_tree(cx, move || {
            div()
                .child(crate::container_query(move |_size, window, cx| {
                    let result = panic::catch_unwind(panic::AssertUnwindSafe(|| {
                        window.with_layout_measurement(|window| {
                            let mut doomed = div().id("panic").class("row").into_any_element();
                            doomed.layout_as_root(AvailableSpace::min_size(), window, cx);
                        });
                    }));

                    assert!(result.is_err());

                    let result = panic::catch_unwind(panic::AssertUnwindSafe(|| {
                        let _result = window.transact(|window| {
                            layout_selector_row("transaction-panic", window, cx);

                            Ok::<(), ()>(())
                        });
                    }));

                    assert!(result.is_err());

                    div().id("surviving").class("row")
                }))
                .select(Select::descendants().class("row"), move |element| {
                    if element.element.element_id() == Some(ElementId::from("panic")) {
                        let _generated = div().class("row");
                        panic!("transform failed");
                    }

                    record_match(&recovered, &element.element);

                    element
                })
                .select(Select::descendants().class("row").nth(0), move |element| {
                    record_match(&observed, &element.element);

                    if element.element.element_id() == Some(ElementId::from("transaction-panic")) {
                        let _generated = div().class("row");
                        panic!("positional transform failed");
                    }

                    element
                })
        });

        assert_matches(
            &selected,
            &[
                "transaction-panic",
                "transaction-panic",
                "surviving",
                "surviving",
            ],
        );
        draw_selector_tree(cx, || {
            div()
                .child(div().class("row"))
                .select(Select::descendants().class("row"), |element| element)
        });
    }

    #[crate::test]
    fn window_boundaries_isolate_rules_positions_and_construction(cx: &mut TestAppContext) {
        let other_app = Rc::new(RefCell::new(cx.new_app()));
        let outer = cx.add_window(|_window, _cx| Empty);
        let selected = Rc::new(RefCell::new(Vec::new()));
        let positioned = Rc::new(RefCell::new(Vec::new()));
        let observed = selected.clone();
        let indexed = positioned.clone();
        let callback_app = other_app.clone();

        let mut visual = VisualTestContext::from_window(outer.into(), cx);

        visual.draw(
            Default::default(),
            size(px(100.), px(100.)),
            |_window, _cx| {
                div()
                    .child(
                        selector_row("original")
                            .class("row")
                            .child(crate::container_query(|_size, _window, _cx| {
                                selector_row("original-late").class("row")
                            })),
                    )
                    .child(selector_row("after").class("row"))
                    .select(Select::descendants().class("row"), move |element| {
                        record_match(&observed, &element.element);

                        if element.element.element_id() != Some(ElementId::from("original")) {
                            return element.into_any_element();
                        }

                        let (other, other_row) = callback_app.borrow_mut().update(|cx| {
                            let mut retained = None;
                            let other = cx
                                .open_window(Default::default(), |window, cx| {
                                    retained = Some(construct_window_row("other-app", window, cx));

                                    cx.new(|_cx| Empty)
                                })
                                .unwrap();

                            (other, retained.unwrap())
                        });
                        assert_eq!(other.window_id(), outer.window_id());

                        let generated = selector_row("retained-generated")
                            .class("row")
                            .into_any_element();

                        crate::container_query(move |_size, _window, cx| {
                            let mut retained = None;
                            let second = cx
                                .open_window(Default::default(), |window, cx| {
                                    retained = Some(construct_window_row("new-window", window, cx));

                                    cx.new(|_cx| Empty)
                                })
                                .unwrap();
                            let new_row = retained.unwrap();
                            let updated_row = cx
                                .update_window(second.into(), |_view, window, cx| {
                                    construct_window_row("updated-window", window, cx)
                                })
                                .unwrap();

                            div()
                                .child(element)
                                .child(other_row)
                                .child(new_row)
                                .child(updated_row)
                                .child(generated)
                                .child(selector_row("generated-late").class("row"))
                        })
                        .into_any_element()
                    })
                    .select(Select::descendants().class("row").nth(0), move |element| {
                        record_match(&indexed, &element.element);

                        element
                    })
                    .into_any_element()
            },
        );

        assert_matches(
            &selected,
            &[
                "original",
                "after",
                "other-app",
                "new-window",
                "updated-window",
                "original-late",
            ],
        );
        assert_matches(&positioned, &["after"]);
        other_app.borrow().quit();
    }

    #[crate::test]
    fn direct_layout_preserves_retained_generation_and_restores_construction(
        cx: &mut TestAppContext,
    ) {
        let outer = cx.add_window(|_window, _cx| Empty);
        let generating = Rc::new(RefCell::new(Vec::new()));
        let later = Rc::new(RefCell::new(Vec::new()));
        let retained = Rc::new(RefCell::new(None));

        let observed = generating.clone();
        let retained_nodes = retained.clone();
        let later_observed = later.clone();
        let selectors = [
            PendingSelector::new(
                Select::this().class("row").into_matcher(),
                Box::new(move |element| {
                    record_match(&observed, &element);

                    if element.element_id() == Some(ElementId::from("panic")) {
                        let _generated = selector_row("doomed").class("row").into_any_element();
                        panic!("transform failed");
                    }

                    if element.element_id() == Some(ElementId::from("seed")) {
                        *retained_nodes.borrow_mut() =
                            Some(selector_row("retained").class("row").into_any_element());
                    }

                    element
                }),
            ),
            PendingSelector::new(
                Select::this().class("row").into_matcher(),
                Box::new(move |element| {
                    record_match(&later_observed, &element);

                    element
                }),
            ),
        ];

        cx.update_window(outer.into(), |_view, window, cx| {
            window.invalidator.set_phase(DrawPhase::Prepaint);

            {
                let _session = window.selector_runtime().enter_session();
                let _attached = window.selector_runtime().enter_attached(&selectors);

                layout_selector_row("seed", window, cx);
                let result = panic::catch_unwind(panic::AssertUnwindSafe(|| {
                    layout_selector_row("panic", window, cx);
                }));

                assert!(result.is_err());

                layout_selector_row("recovered", window, cx);
            }

            let mut outside = selector_row("outside").class("row").into_any_element();
            let mut retained = retained.borrow_mut().take().unwrap();

            {
                let _session = window.selector_runtime().enter_session();
                let _attached = window.selector_runtime().enter_attached(&selectors);

                retained.layout_as_root(AvailableSpace::min_size(), window, cx);
                outside.layout_as_root(AvailableSpace::min_size(), window, cx);
            }

            window.invalidator.set_phase(DrawPhase::None);
        })
        .unwrap();

        assert_matches(&generating, &["seed", "panic", "recovered", "outside"]);
        assert_matches(&later, &["seed", "recovered", "retained", "outside"]);
    }

    #[crate::test]
    fn shared_rules_keep_positions_per_window_and_session(cx: &mut TestAppContext) {
        let outer = cx.add_window(|_window, _cx| Empty);
        let second = cx.add_window(|_window, _cx| Empty);
        let selected = Rc::new(RefCell::new(Vec::new()));
        let observed = selected.clone();
        let shared = PendingSelector::new(
            Select::this().class("row").nth(1).into_matcher(),
            Box::new(move |element| {
                record_match(&observed, &element);

                element
            }),
        );

        for _redraw in 0..2 {
            cx.update_window(outer.into(), |_view, window, cx| {
                window.invalidator.set_phase(DrawPhase::Prepaint);
                let _session = window.selector_runtime().enter_session();

                {
                    let _attached = window
                        .selector_runtime()
                        .enter_attached(std::slice::from_ref(&shared));

                    layout_selector_row("outer-first", window, cx);
                }

                cx.update_window(second.into(), |_view, window, cx| {
                    window.invalidator.set_phase(DrawPhase::Prepaint);
                    let _session = window.selector_runtime().enter_session();
                    let _attached = window
                        .selector_runtime()
                        .enter_attached(std::slice::from_ref(&shared));

                    layout_selector_row("second-first", window, cx);
                    layout_selector_row("second-selected", window, cx);
                    window.invalidator.set_phase(DrawPhase::None);
                })
                .unwrap();

                {
                    let _attached = window
                        .selector_runtime()
                        .enter_attached(std::slice::from_ref(&shared));

                    layout_selector_row("outer-selected", window, cx);
                    layout_selector_row("outer-exhausted", window, cx);
                }

                window.invalidator.set_phase(DrawPhase::None);
            })
            .unwrap();
        }

        assert_matches(
            &selected,
            &[
                "second-selected",
                "outer-selected",
                "second-selected",
                "outer-selected",
            ],
        );
    }

    #[crate::test]
    fn rollback_restores_positions_first_attached_inside_transaction(cx: &mut TestAppContext) {
        let outer = cx.add_window(|_window, _cx| Empty);
        let selected = Rc::new(RefCell::new(Vec::new()));
        let observed = selected.clone();
        let selector = PendingSelector::new(
            Select::this().class("row").nth(0).into_matcher(),
            Box::new(move |element| {
                record_match(&observed, &element);

                element
            }),
        );

        cx.update_window(outer.into(), |_view, window, cx| {
            window.invalidator.set_phase(DrawPhase::Prepaint);
            let _session = window.selector_runtime().enter_session();
            let mut retained = None;
            let result = window.transact(|window| {
                let _attached = window
                    .selector_runtime()
                    .enter_attached(std::slice::from_ref(&selector));

                layout_selector_row("attempted", window, cx);
                retained = Some(window.selector_runtime().capture_scope());

                Err::<(), ()>(())
            });

            assert!(result.is_err());

            {
                let _captured_scope = window
                    .selector_runtime()
                    .enter_captured_scope(retained.unwrap());

                layout_selector_row("committed", window, cx);
            }

            {
                let _attached = window
                    .selector_runtime()
                    .enter_attached(std::slice::from_ref(&selector));

                layout_selector_row("exhausted", window, cx);
            }

            window.invalidator.set_phase(DrawPhase::None);
        })
        .unwrap();

        assert_matches(&selected, &["attempted", "committed"]);
    }

    #[crate::test]
    fn captured_scopes_reject_foreign_windows_and_inactive_sessions(cx: &mut TestAppContext) {
        let outer = cx.add_window(|_window, _cx| Empty);
        let second = cx.add_window(|_window, _cx| Empty);
        let mut other_app = cx.new_app();
        let other = other_app.add_window(|_window, _cx| Empty);
        assert_eq!(outer.window_id(), other.window_id());

        let selected = Rc::new(RefCell::new(Vec::new()));
        let receiving = selected.clone();
        let stale = cx
            .update_window(outer.into(), |_view, window, cx| {
                window.invalidator.set_phase(DrawPhase::Prepaint);
                let _session = window.selector_runtime().enter_session();
                let selector = PendingSelector::new(
                    Select::this().class("row").nth(0).into_matcher(),
                    Box::new(move |element| {
                        record_match(&receiving, &element);

                        element
                    }),
                );

                let captured = {
                    let _attached = window.selector_runtime().enter_attached(&[selector]);
                    let captured = window.with_layout_measurement(|window| {
                        window.selector_runtime().capture_scope()
                    });
                    cx.update_window(second.into(), |_view, window, cx| {
                        reject_scope_and_layout(
                            captured.clone(),
                            "same-app",
                            selected.clone(),
                            window,
                            cx,
                        );
                    })
                    .unwrap();
                    other_app
                        .update_window(other.into(), |_view, window, cx| {
                            reject_scope_and_layout(
                                captured.clone(),
                                "other-app",
                                selected.clone(),
                                window,
                                cx,
                            );
                        })
                        .unwrap();

                    {
                        let _nested_session = window.selector_runtime().enter_session();
                        let result = panic::catch_unwind(panic::AssertUnwindSafe(|| {
                            let _captured_scope = window
                                .selector_runtime()
                                .enter_captured_scope(captured.clone());
                        }));

                        assert!(result.is_err());
                    }

                    {
                        let _captured_scope = window
                            .selector_runtime()
                            .enter_captured_scope(captured.clone());

                        layout_selector_row("measured", window, cx);
                    }

                    layout_selector_row("outer", window, cx);

                    captured
                };

                window.invalidator.set_phase(DrawPhase::None);

                captured
            })
            .unwrap();

        cx.update_window(outer.into(), |_view, window, cx| {
            let result = panic::catch_unwind(panic::AssertUnwindSafe(|| {
                let _captured_scope = window
                    .selector_runtime()
                    .enter_captured_scope(stale.clone());

                layout_selector_row("idle-rejected", window, cx);
            }));

            assert!(result.is_err());

            reject_scope_and_layout(stale, "fresh-session", selected.clone(), window, cx);
        })
        .unwrap();

        assert_matches(
            &selected,
            &["same-app", "other-app", "outer", "fresh-session"],
        );
        other_app.quit();
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
                    root = root
                        .select(selector, move |element| {
                            selected
                                .borrow_mut()
                                .push((label, element.element.element_id().unwrap()));

                            element
                        })
                        .into_any_element();
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
                                        .h(px(8.))
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
                            div()
                                .id(("list-row", idx))
                                .h(px(8.))
                                .class("row")
                                .into_any_element()
                        })
                        .h(px(60.))
                        .w_full(),
                    )
                    .select(
                        Select::descendants().class("row").reflects(crate::Styled),
                        |element| element.h(px(20.)),
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

    #[crate::test]
    fn list_autoscroll_retries_rebuild_visible_and_focused_rows(cx: &mut TestAppContext) {
        for focus_offscreen in [false, true] {
            let state = ListState::new(6, ListAlignment::Top, px(0.)).measure_all();
            let attempted = Rc::new(RefCell::new(Vec::new()));
            let first = Rc::new(RefCell::new(Vec::new()));
            let every = Rc::new(RefCell::new(Vec::new()));

            let attempted_matches = attempted.clone();
            let first_matches = first.clone();
            let every_matches = every.clone();
            let state_for_render = state.clone();

            let window = cx.add_window(move |window, cx| {
                let focus_handle = cx.focus_handle();
                state_for_render.splice_focusable(
                    0..6,
                    (0..6).map(|idx| (idx == 5).then(|| focus_handle.clone())),
                );

                if focus_offscreen {
                    window.focus(&focus_handle, cx);
                }

                SelectorTestView {
                    render: Box::new(move || {
                        let focus_handle = focus_handle.clone();
                        let attempted_matches = attempted_matches.clone();
                        let first_matches = first_matches.clone();
                        let every_matches = every_matches.clone();

                        div()
                            .w(px(80.))
                            .h(px(60.))
                            .child(
                                list(state_for_render.clone(), move |idx, _window, _cx| {
                                    list_retry_row(idx, &focus_handle)
                                })
                                .size_full(),
                            )
                            .select(
                                Select::descendants().class("row").reflects(crate::Styled),
                                |element| element.h(px(24.)),
                            )
                            .select(
                                Select::descendants().class("row").every(1),
                                move |element| {
                                    record_match(&attempted_matches, &element.element);

                                    element
                                },
                            )
                            .select(
                                Select::descendants()
                                    .class("row")
                                    .reflects(crate::Styled)
                                    .nth(0),
                                move |element| {
                                    record_match(&first_matches, &element.element);

                                    element.w(px(48.))
                                },
                            )
                            .select(
                                Select::descendants().class("row").every(2),
                                move |element| {
                                    record_match(&every_matches, &element.element);

                                    element
                                },
                            )
                    }),
                }
            });

            attempted.borrow_mut().clear();
            first.borrow_mut().clear();
            every.borrow_mut().clear();
            state.scroll_to(crate::ListOffset {
                item_ix: 2,
                offset_in_item: px(0.),
            });
            draw_selector_window(window.into(), cx);

            assert_eq!(state.logical_scroll_top().item_ix, 0);
            assert_eq!(state.logical_scroll_top().offset_in_item, px(12.));
            assert_eq!(state.max_offset_for_scrollbar().y, px(84.));
            assert_matches(&first, &["row-2", "row-0"]);
            assert_matches(&every, &["row-2", "row-4", "row-0", "row-2"]);

            let expected = if focus_offscreen {
                vec![
                    "row-2", "row-3", "row-4", "row-5", "row-0", "row-1", "row-2", "row-5",
                ]
            } else {
                vec!["row-2", "row-3", "row-4", "row-0", "row-1", "row-2"]
            };
            assert_matches(&attempted, &expected);

            cx.update_window(window.into(), |_view, window, _cx| {
                for (label, top, width) in [
                    ("row-0", -12., 48.),
                    ("row-1", 12., 80.),
                    ("row-2", 36., 80.),
                ] {
                    let bounds = window.rendered_frame.debug_bounds[label];
                    assert_eq!(bounds.origin.y, px(top));
                    assert_eq!(bounds.size, size(px(width), px(24.)));
                }
            })
            .unwrap();
        }
    }

    #[crate::test]
    fn measurement_helpers_preserve_selector_state(cx: &mut TestAppContext) {
        let available_space = size(AvailableSpace::MaxContent, AvailableSpace::MinContent);
        let expected_size = size(px(48.), px(24.));

        let measure_nested_row = move |window: &mut Window, cx: &mut App| {
            let mut element = div()
                .id("nested-measurement")
                .size(px(8.))
                .class("row")
                .into_any_element();

            element.layout_as_root(available_space, window, cx)
        };

        let build_measured_row = move |window: &mut Window, cx: &mut App| {
            let nested_size =
                window.with_layout_measurement(|window| measure_nested_row(window, cx));
            let mut element = div()
                .id("during-construction")
                .size(px(8.))
                .class("row")
                .into_any_element();
            let constructed_size = element.layout_as_root(available_space, window, cx);

            assert_eq!([nested_size, constructed_size], [expected_size; 2]);

            div().id("measured-root").size(px(8.)).class("row")
        };

        let build_panicking_row = |_window: &mut Window, _cx: &mut App| -> Empty {
            panic!("measurement construction failed");
        };

        let render_rows = move |_bounds: Size<Pixels>, window: &mut Window, cx: &mut App| {
            let result = panic::catch_unwind(panic::AssertUnwindSafe(|| {
                window.measure_element(available_space, cx, build_panicking_row);
            }));

            assert!(result.is_err());

            let measured_size = window.measure_element(available_space, cx, build_measured_row);

            assert_eq!(measured_size, expected_size);

            div()
                .child(div().id("first").class("row"))
                .child(div().id("unrelated"))
                .child(div().id("second").class("row"))
        };

        let selected = Rc::new(RefCell::new(Vec::new()));
        let first = selected.clone();
        let every = selected.clone();

        cx.add_empty_window().draw(
            Default::default(),
            size(px(80.), px(120.)),
            move |_window, _cx| {
                div()
                    .child(crate::container_query(render_rows))
                    .select(
                        Select::descendants().class("row").reflects(crate::Styled),
                        |element| element.w(px(48.)).h(px(24.)),
                    )
                    .select(
                        Select::descendants()
                            .class("row")
                            .reflects(crate::Styled)
                            .nth(0),
                        move |element| {
                            first
                                .borrow_mut()
                                .push(("nth", element.element.element_id().unwrap()));

                            element.h(px(99.))
                        },
                    )
                    .select(
                        Select::descendants().class("row").every(2),
                        move |element| {
                            every
                                .borrow_mut()
                                .push(("every", element.element.element_id().unwrap()));

                            element
                        },
                    )
                    .into_any_element()
            },
        );

        assert_eq!(
            *selected.borrow(),
            vec![
                ("nth", ElementId::from("first")),
                ("every", ElementId::from("first")),
            ]
        );
    }

    struct CachedSelectorChild {
        renders: Rc<Cell<usize>>,
    }

    impl Render for CachedSelectorChild {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            self.renders.set(self.renders.get() + 1);

            TestElement::new("cached-row")
                .size_full()
                .occlude()
                .bg(rgb(0x112233))
                .role(accesskit::Role::Button)
                .accessibility_id("cached-card")
                .aria_label("Cached card")
                .debug_selector(|| "cached-row".into())
                .class("row")
        }
    }

    fn cached_selector_style() -> StyleRefinement {
        StyleRefinement::default().w(px(40.)).h(px(20.))
    }

    fn cached_selector_element(child: &Entity<CachedSelectorChild>) -> AnyElement {
        child
            .clone()
            .cached(cached_selector_style())
            .into_any_element()
    }

    fn cached_selector_window<Builder: IntoElement + 'static>(
        cx: &mut TestAppContext,
        render: impl Fn(&Entity<CachedSelectorChild>, &Rc<Cell<usize>>) -> Builder + 'static,
    ) -> (
        AnyWindowHandle,
        Entity<CachedSelectorChild>,
        Rc<Cell<usize>>,
    ) {
        let renders = Rc::new(Cell::new(0));
        let child = cx.new(|_cx| CachedSelectorChild {
            renders: renders.clone(),
        });

        let child_for_render = child.clone();
        let renders_for_render = renders.clone();

        let window = cx
            .add_window(move |_window, _cx| SelectorTestView {
                render: Box::new(move || render(&child_for_render, &renders_for_render)),
            })
            .into();

        draw_selector_window(window, cx);
        renders.set(0);

        (window, child, renders)
    }

    fn measure_cached_selector_child(
        child: &Entity<CachedSelectorChild>,
        non_positional: bool,
        window: &mut Window,
        cx: &mut App,
    ) -> Size<Pixels> {
        window
            .transact(|window| {
                let measured_size = window.with_layout_measurement(|window| {
                    let mut measured = cached_selector_element(child);
                    let measured_size =
                        measured.layout_as_root(AvailableSpace::min_size(), window, cx);

                    if non_positional {
                        measured.prepaint(window, cx);
                        assert_eq!(
                            window.next_frame.hitboxes.last().unwrap().bounds.size,
                            size(px(40.), px(12.))
                        );
                    }

                    measured_size
                });

                Err::<(), Size<Pixels>>(measured_size)
            })
            .unwrap_err()
    }

    #[track_caller]
    fn assert_cached_selector_size(
        window: AnyWindowHandle,
        expected: Size<Pixels>,
        cx: &mut TestAppContext,
    ) {
        cx.update_window(window, |_view, window, _cx| {
            let hitbox = window.rendered_frame.hitboxes.last().unwrap();
            assert_eq!(hitbox.bounds.size, expected);
            assert!(!window.rendered_frame.scene.quads.is_empty());
        })
        .unwrap();
    }

    #[crate::test]
    fn cached_selectors_respect_scope_and_view_depth(cx: &mut TestAppContext) {
        for (scope, on_view) in [
            (SelectScope::This, false),
            (SelectScope::Children, false),
            (SelectScope::Children, true),
        ] {
            let selected = Rc::new(Cell::new(0));
            let observed = selected.clone();
            let (window, _child, renders) = cached_selector_window(cx, move |child, _renders| {
                let selected = observed.clone();
                let cached = cached_selector_element(child);

                if on_view {
                    return div()
                        .child(cached.select(
                            Select::children().reflects(crate::Styled),
                            move |element| {
                                selected.set(selected.get() + 1);

                                element.h(px(12.))
                            },
                        ))
                        .into_any_element();
                }

                div()
                    .child(cached)
                    .select(Select::new(scope), move |element| {
                        selected.set(selected.get() + 1);

                        element
                    })
                    .into_any_element()
            });

            selected.set(0);

            for _redraw in 0..2 {
                draw_selector_window(window, cx);
                let height = if on_view { px(12.) } else { px(20.) };
                assert_cached_selector_size(window, size(px(40.), height), cx);
            }

            assert_eq!(renders.get(), if on_view { 2 } else { 0 });
            assert_eq!(selected.get(), 2);
        }
    }

    #[crate::test]
    fn cached_selectors_use_live_positional_exhaustion(cx: &mut TestAppContext) {
        for (nth, every, earlier_match, deferred_contents, expected_renders, expected_height) in [
            (Some(0), None, true, false, 0, 20.),
            (Some(0), Some(2), true, false, 0, 20.),
            (Some(0), None, false, false, 2, 12.),
            (Some(1), None, true, false, 2, 12.),
            (None, Some(2), true, false, 2, 20.),
            (Some(0), None, true, true, 0, 20.),
        ] {
            let selected = Rc::new(RefCell::new(Vec::new()));
            let observed = selected.clone();
            let (window, _child, renders) = cached_selector_window(cx, move |child, _renders| {
                let selected = observed.clone();
                let mut selector = Select::descendants().class("row").reflects(crate::Styled);

                if let Some(idx) = nth {
                    selector = selector.nth(idx);
                }

                if let Some(step) = every {
                    selector = selector.every(step);
                }

                let cached = cached_selector_element(child);
                let first = selector_row("first").class("row").into_any_element();
                let children = match (earlier_match, deferred_contents) {
                    (true, true) => vec![
                        crate::deferred(crate::container_query(move |_size, _window, _cx| cached))
                            .priority(2)
                            .into_any_element(),
                        crate::deferred(crate::container_query(move |_size, _window, _cx| first))
                            .priority(1)
                            .into_any_element(),
                    ],
                    (true, false) => vec![first, cached],
                    (false, _) => vec![cached],
                };

                div()
                    .size(px(80.))
                    .children(children)
                    .select(selector, move |element| {
                        record_match(&selected, &element.element);

                        element.h(px(12.))
                    })
            });

            selected.borrow_mut().clear();

            for _redraw in 0..2 {
                draw_selector_window(window, cx);
                assert_cached_selector_size(window, size(px(40.), px(expected_height)), cx);
            }

            let target = if earlier_match && nth != Some(1) {
                "first"
            } else {
                "cached-row"
            };

            assert_matches(&selected, &[target, target]);
            assert_eq!(renders.get(), expected_renders);
        }
    }

    #[crate::test]
    fn cached_selectors_recheck_progress_after_measurements_and_rollbacks(cx: &mut TestAppContext) {
        for non_positional in [false, true] {
            let selected = Rc::new(RefCell::new(Vec::new()));
            let observed = selected.clone();
            let (window, _child, renders) = cached_selector_window(cx, move |child, renders| {
                let child = child.clone();
                let renders = renders.clone();
                let selected = observed.clone();
                let mut root = div()
                    .size(px(80.))
                    .child(crate::container_query(move |_size, window, cx| {
                        let available = AvailableSpace::min_size();
                        let before = renders.get();
                        let measured_size = window.with_layout_measurement(|window| {
                            measure_cached_selector_child(&child, non_positional, window, cx)
                        });
                        assert_eq!(measured_size, size(px(40.), px(20.)));
                        // Temporary cached views still render to probe their text direction.
                        assert_eq!(renders.get() - before, 1);

                        let outer = window.transact(|window| {
                            let inner = window.transact(|window| {
                                layout_selector_row("abandoned-first", window, cx);
                                let before = renders.get();
                                cached_selector_element(&child)
                                    .layout_as_root(available, window, cx);
                                assert_eq!(renders.get() - before, 1);

                                Err::<(), ()>(())
                            });

                            assert!(inner.is_err());
                            cached_selector_element(&child).layout_as_root(available, window, cx);

                            Err::<(), ()>(())
                        });

                        assert!(outer.is_err());

                        cached_selector_element(&child)
                    }))
                    .into_any_element();

                if non_positional {
                    root = root
                        .select(
                            Select::descendants().class("row").reflects(crate::Styled),
                            |element| element.h(px(12.)),
                        )
                        .into_any_element();
                }

                root.select(
                    Select::descendants()
                        .class("row")
                        .reflects(crate::Styled)
                        .nth(0),
                    move |element| {
                        record_match(&selected, &element.element);

                        element.h(px(16.))
                    },
                )
            });

            selected.borrow_mut().clear();
            draw_selector_window(window, cx);

            assert_eq!(renders.get(), 4);
            assert_matches(&selected, &["abandoned-first", "cached-row", "cached-row"]);
            assert_cached_selector_size(window, size(px(40.), px(16.)), cx);
        }
    }

    #[crate::test]
    fn cached_selectors_preserve_invalidation_and_cache_transitions(cx: &mut TestAppContext) {
        let settings = Rc::new(Cell::new((None, 40., 0x112233, 80., 0.)));
        let observed = settings.clone();
        let (window, child, renders) = cached_selector_window(cx, move |child, _renders| {
            let (height, width, color, mask_width, inset) = observed.get();
            let mut root = div()
                .h(px(60.))
                .overflow_hidden()
                .child(child.clone().cached(cached_selector_style().w(px(width))))
                .select(Select::this().reflects(crate::Styled), move |element| {
                    element
                        .w(px(mask_width))
                        .pl(px(inset))
                        .text_color(rgb(color))
                })
                .into_any_element();

            if let Some(height) = height {
                root = root
                    .select(
                        Select::descendants()
                            .class("row")
                            .id("cached-row")
                            .reflects(crate::Styled),
                        move |element| element.h(px(height)),
                    )
                    .into_any_element();
            }

            root
        });

        for (inputs, expected_renders) in [
            ((None, 40., 0x112233, 80., 0.), 0),
            ((Some(12.), 40., 0x112233, 80., 0.), 1),
            ((Some(16.), 40., 0x112233, 80., 0.), 1),
            ((None, 40., 0x112233, 80., 0.), 1),
            ((None, 40., 0x112233, 80., 0.), 0),
            ((None, 48., 0x112233, 80., 0.), 1),
            ((None, 48., 0x112233, 80., 0.), 0),
            ((None, 48., 0x334455, 80., 0.), 1),
            ((None, 48., 0x334455, 30., 0.), 1),
            ((None, 48., 0x334455, 30., 3.), 1),
            ((None, 48., 0x334455, 30., 3.), 0),
        ] {
            settings.set(inputs);
            renders.set(0);
            draw_selector_window(window, cx);

            let (height, width, _color, mask_width, inset) = inputs;
            assert_eq!(renders.get(), expected_renders);
            assert_cached_selector_size(window, size(px(width), px(height.unwrap_or(20.))), cx);
            cx.update_window(window, |_view, window, _cx| {
                let hitbox = window.rendered_frame.hitboxes.last().unwrap();
                assert_eq!(hitbox.bounds.origin.x, px(inset));
                assert_eq!(hitbox.content_mask.bounds.size.width, px(mask_width));
            })
            .unwrap();
        }

        for refresh in [false, true] {
            renders.set(0);
            cx.update_window(window, |_view, window, cx| {
                if refresh {
                    window.refresh();
                } else {
                    child.update(cx, |_child, cx| cx.notify());
                }
            })
            .unwrap();

            draw_selector_window(window, cx);
            assert_eq!(renders.get(), 1);
            assert_cached_selector_size(window, size(px(48.), px(20.)), cx);
        }
    }

    #[crate::test]
    fn selectors_visit_cached_view_contents_on_each_frame(cx: &mut TestAppContext) {
        let selected = Rc::new(Cell::new(0));
        let painted = Rc::new(RefCell::new(Vec::new()));
        let observed = selected.clone();
        let observed_painted = painted.clone();
        let (window, _child, renders) = cached_selector_window(cx, move |child, _renders| {
            let selected = observed.clone();
            let painted = observed_painted.clone();

            div()
                .child(AnyView::from(child.clone()).cached(cached_selector_style()))
                .select(
                    Select::descendants()
                        .class("row")
                        .reflects(trait_set![
                            test_fixtures::TestValue,
                            crate::Styled,
                            crate::ParentElement,
                        ])
                        .nth(0),
                    move |mut element| {
                        selected.set(selected.get() + 1);
                        *element.value() = 42;

                        let card = element.element.downcast_mut::<TestElement>().unwrap();
                        assert_eq!(card.value, 42);

                        let painted = painted.clone();

                        element.child(crate::deferred(crate::container_query(
                            move |_bounds, _window, _cx| {
                                selector_canvas("cached-detail", (), painted)
                            },
                        )))
                    },
                )
        });

        cx.update_window(window, |_view, window, _cx| window.set_a11y_forced(true))
            .unwrap();
        selected.set(0);
        renders.set(0);
        painted.borrow_mut().clear();

        let mut node_ids = Vec::new();

        for _redraw in 0..2 {
            draw_selector_window(window, cx);
            assert_cached_selector_size(window, size(px(40.), px(20.)), cx);
            cx.update_window(window, |_view, window, _cx| {
                let tree: serde_json::Value =
                    serde_json::from_str(&window.debug_a11y_tree_json().unwrap()).unwrap();
                let node = tree["nodes"].as_object().unwrap();
                let node = node
                    .values()
                    .find(|node| node["aria"]["author_id"] == "cached-card")
                    .unwrap();
                assert_eq!(node["aria"]["label"], "Cached card");
                assert_eq!(node["bounds"]["width"].as_f64(), Some(40.));
                assert_eq!(node["bounds"]["height"].as_f64(), Some(20.));
                node_ids.push(node["accesskit_id"].clone());

                #[cfg(debug_assertions)]
                assert_eq!(node["view"], std::any::type_name::<CachedSelectorChild>());
            })
            .unwrap();
        }

        assert_eq!(node_ids[0], node_ids[1]);
        assert_eq!(renders.get(), 2);
        assert_eq!(selected.get(), 2);
        assert_eq!(
            *painted.borrow(),
            vec![("cached-detail", size(px(8.), px(8.))); 2]
        );
    }

    #[test]
    #[should_panic(expected = "selector interval must be nonzero")]
    fn rejects_zero_intervals() {
        Select::children().every(0);
    }

    #[crate::test]
    fn nested_retries_restore_positions_and_keep_attempted_callback_logs(cx: &mut TestAppContext) {
        for outer_succeeds in [false, true] {
            let nth = Rc::new(RefCell::new(Vec::new()));
            let every = Rc::new(RefCell::new(Vec::new()));
            let attached = Rc::new(RefCell::new(Vec::new()));

            let nth_matches = nth.clone();
            let every_matches = every.clone();
            let attached_matches = attached.clone();

            let window = cx.add_window(move |_window, _cx| SelectorTestView {
                render: Box::new(move || {
                    let nth_matches = nth_matches.clone();
                    let every_matches = every_matches.clone();
                    let attached_matches = attached_matches.clone();

                    div()
                        .size_full()
                        .child(crate::deferred(crate::container_query(
                            move |bounds, window, cx| {
                                let origin = window.element_offset();
                                let result = window.transact(|window| {
                                    let mut attempted = div()
                                        .child(selector_row("outer-first").class("row"))
                                        .child(deferred_selector_row("attempt-late"))
                                        .into_any_element();
                                    attempted.prepaint_as_root(origin, bounds.into(), window, cx);

                                    let inner = layout_nested_transaction_row(
                                        outer_succeeds,
                                        attached_matches.clone(),
                                        window,
                                        cx,
                                    );

                                    assert_eq!(inner.is_err(), outer_succeeds);
                                    layout_selector_row("outer-last", window, cx);

                                    if outer_succeeds {
                                        return Ok(());
                                    }

                                    Err(())
                                });

                                assert_eq!(result.is_ok(), outer_succeeds);

                                div()
                                    .child(selector_row("committed").class("row"))
                                    .child(selector_row("committed-next").class("row"))
                                    .child(deferred_selector_row("committed-late"))
                            },
                        )))
                        .select(Select::descendants().class("row").nth(1), move |element| {
                            record_match(&nth_matches, &element.element);

                            element
                        })
                        .select(
                            Select::descendants()
                                .class("row")
                                .reflects(crate::Styled)
                                .every(2),
                            move |element| {
                                record_match(&every_matches, &element.element);

                                element.w(px(24.))
                            },
                        )
                }),
            });

            nth.borrow_mut().clear();
            every.borrow_mut().clear();
            attached.borrow_mut().clear();
            draw_selector_window(window.into(), cx);

            assert_matches(&attached, &["inner"]);

            if outer_succeeds {
                assert_matches(&nth, &["inner", "outer-last"]);
                assert_matches(&every, &["outer-first", "committed", "attempt-late"]);
            } else {
                assert_matches(&nth, &["inner", "committed-next"]);
                assert_matches(
                    &every,
                    &["outer-first", "outer-last", "committed", "committed-late"],
                );
            }

            cx.update_window(window.into(), |_view, window, _cx| {
                assert_eq!(
                    window.rendered_frame.deferred_draws.len(),
                    if outer_succeeds { 3 } else { 2 }
                );
                let attempt_bounds = window.rendered_frame.debug_bounds.get("attempt-late");
                assert_eq!(attempt_bounds.is_some(), outer_succeeds);

                if let Some(bounds) = attempt_bounds {
                    assert_eq!(bounds.size.width, px(24.));
                }

                for (label, width) in [
                    ("committed", 24.),
                    ("committed-next", 8.),
                    ("committed-late", if outer_succeeds { 8. } else { 24. }),
                ] {
                    assert_eq!(
                        window.rendered_frame.debug_bounds[label].size.width,
                        px(width)
                    );
                }
            })
            .unwrap();
        }
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
        // The styled container query is a child, and its rendered div is a descendant.
        assert_eq!(children_count.get(), 3);
        assert_eq!(descendants_count.get(), 5);
        assert_eq!(filtered_count.get(), 1);
    }
}
