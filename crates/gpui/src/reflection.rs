//! Typed trait access to erased elements.
//! Borrowed methods dispatch to concrete implementations; owned builders and
//! `#[reflect(wrapper_default)]` methods use the trait body on [`ReflectedElement`].
//!
//! Generic derives list all traits in `#[reflect(...)]`, including builtins, and forward
//! `Element::reflection` to `<Self as Reflect>::reflection()` with matching trait bounds.
//! Handwritten providers implement `Reflect::build_reflection` and use the same hook.
//! Unregistered elements use shared empty metadata.
//!
//! `#[reflect_trait(membership)]` permits associated items and generic methods
//! without granting callable access.
//!
//! Reflection describes the concrete element stored after conversion. A
//! `ViewElement<Icon>` does not inherit traits registered for `Icon` or its rendered
//! root. Animation elements likewise keep their own receiver. `Stateful<E>` is a
//! builder that converts to `E::Element`, so assigning an ID to a `Div` does not
//! give the erased `Div` the `StatefulInteractiveElement` trait.
//! A component can retain custom traits by implementing `Element` on its outer
//! concrete type and delegating directly to a stored element, as in the
//! `trait_reflection` example.

use crate::{
    AnyElement, App, Bounds, Element, ElementId, GlobalElementId, InspectorElementId, IntoElement,
    LayoutId, Pixels, Window,
};

pub use gpui_macros::{Reflect, reflect_trait, trait_set};
use parking_lot::RwLock;
use smallvec::SmallVec;
use std::{
    any::{Any, TypeId},
    cell::RefCell,
    collections::HashMap,
    marker::PhantomData,
    panic,
    sync::LazyLock,
};

#[cfg(feature = "bench-support")]
#[doc(hidden)]
pub mod benchmarks;

#[cfg(test)]
#[path = "../examples/learn/trait_reflection.rs"]
#[allow(dead_code)]
mod component_example;

#[cfg(test)]
#[macro_use]
#[path = "reflection/element_test_support.rs"]
mod element_test_support;

/// Identifies a trait made available to element reflection.
#[derive(Clone, Copy, Debug)]
pub struct ReflectedTrait {
    /// The fully qualified name of the trait.
    pub name: &'static str,
    type_id: fn() -> TypeId,
    /// Returns this trait's directly inherited reflected traits.
    pub supertraits: fn() -> &'static [ReflectedTrait],
}

impl ReflectedTrait {
    /// Creates a descriptor for a unique trait marker.
    #[doc(hidden)]
    pub const fn new(name: &'static str, type_id: fn() -> TypeId) -> Self {
        Self {
            name,
            type_id,
            supertraits: || &[],
        }
    }

    /// Attaches inheritance emitted by the trait's reflection macro.
    #[doc(hidden)]
    pub const fn with_supertraits(
        mut self,
        supertraits: fn() -> &'static [ReflectedTrait],
    ) -> Self {
        self.supertraits = supertraits;

        self
    }

    fn trait_type_id(&self) -> TypeId {
        (self.type_id)()
    }
}

impl PartialEq for ReflectedTrait {
    fn eq(&self, other: &Self) -> bool {
        self.trait_type_id() == other.trait_type_id()
    }
}

impl Eq for ReflectedTrait {}

/// A typed reflected trait descriptor.
pub trait ReflectionToken: Copy + 'static {
    /// The group for this token alone.
    type Group: ReflectionGroup;
    /// The erased runtime descriptor for this trait.
    fn reflected_trait(self) -> ReflectedTrait;

    /// Whether matching requires a callable adapter.
    fn requires_adapter(self) -> bool;

    /// Returns the membership and adapter requirements for this token.
    fn requirement(self) -> ReflectionRequirement {
        ReflectionRequirement {
            descriptor: self.reflected_trait(),
            requires_adapter: self.requires_adapter(),
            adapter_type: None,
        }
    }

    /// Requirements for this token and its callable parents.
    fn requirements(self) -> Vec<ReflectionRequirement> {
        vec![self.requirement()]
    }
}

/// A token whose generated method table permits calls on an erased element.
pub trait CallableReflectionToken: ReflectionToken {
    /// The trait's generated method table.
    type Methods: Any + Send + Sync;
}

/// A trait membership check, optionally requiring a compatible callable table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReflectionRequirement {
    /// The trait whose membership is required.
    pub descriptor: ReflectedTrait,
    /// Whether a callable adapter is required.
    pub requires_adapter: bool,
    adapter_type: Option<TypeId>,
}

impl ReflectionRequirement {
    /// Requires membership only.
    pub fn membership(descriptor: ReflectedTrait) -> Self {
        descriptor.requirement()
    }

    /// Requires membership and the token's exact method table.
    pub fn callable<Token: CallableReflectionToken>(token: Token) -> Self {
        Self {
            descriptor: token.reflected_trait(),
            requires_adapter: true,
            adapter_type: Some(TypeId::of::<Token::Methods>()),
        }
    }
}

impl ReflectionToken for ReflectedTrait {
    type Group = ErasedReflectionGroup;

    fn reflected_trait(self) -> ReflectedTrait {
        self
    }

    fn requires_adapter(self) -> bool {
        false
    }
}

/// The group for membership-only tokens and erased descriptors.
#[doc(hidden)]
pub struct ErasedReflectionGroup;

impl ReflectionGroup for ErasedReflectionGroup {}

/// A generated set of reflected traits.
#[doc(hidden)]
pub trait ReflectionGroup: 'static {}

/// Proves group membership for a trait.
#[doc(hidden)]
pub trait IncludesReflectedTrait<Token>: ReflectionGroup
where
    Token: ReflectionToken,
{
}

/// Proves callable access to a trait.
pub trait IncludesCallableTrait<Token>: IncludesReflectedTrait<Token>
where
    Token: CallableReflectionToken,
{
}

/// Runtime requirements for a generated trait group.
#[doc(hidden)]
pub struct ReflectedTraitGroup<Group>
where
    Group: ReflectionGroup,
{
    requirements: SmallVec<[ReflectionRequirement; 2]>,
    group: PhantomData<fn() -> Group>,
}

impl<Group> ReflectedTraitGroup<Group>
where
    Group: ReflectionGroup,
{
    /// Creates a reflected trait group from its runtime descriptors.
    #[doc(hidden)]
    pub fn new(traits: impl IntoIterator<Item = ReflectedTrait>) -> Self {
        Self::with_requirements(traits.into_iter().map(ReflectionRequirement::membership))
    }

    /// Creates a group with membership and adapter requirements.
    #[doc(hidden)]
    pub fn with_requirements(
        requirements: impl IntoIterator<Item = ReflectionRequirement>,
    ) -> Self {
        let mut registered = SmallVec::new();

        for requirement in requirements {
            if !registered.contains(&requirement) {
                registered.push(requirement);
            }
        }

        Self {
            requirements: registered,
            group: PhantomData,
        }
    }
}

/// Supplies a trait group's runtime requirements.
#[doc(hidden)]
pub trait ReflectedTraits {
    /// The type that records group membership.
    type Group: ReflectionGroup;

    /// Returns the erased runtime descriptors in this trait set.
    fn reflected_traits(self) -> SmallVec<[ReflectedTrait; 2]>
    where
        Self: Sized,
    {
        self.reflected_requirements()
            .into_iter()
            .map(|requirement| requirement.descriptor)
            .collect()
    }

    /// Returns each trait's membership and adapter requirements.
    fn reflected_requirements(self) -> SmallVec<[ReflectionRequirement; 2]>;
}

impl<Token> ReflectedTraits for Token
where
    Token: ReflectionToken,
    Token::Group: ReflectionGroup,
{
    type Group = Token::Group;

    fn reflected_requirements(self) -> SmallVec<[ReflectionRequirement; 2]> {
        self.requirements().into_iter().collect()
    }
}

impl<Group> ReflectedTraits for ReflectedTraitGroup<Group>
where
    Group: ReflectionGroup,
{
    type Group = Group;

    fn reflected_requirements(self) -> SmallVec<[ReflectionRequirement; 2]> {
        self.requirements
    }
}

/// An owned erased element exposing a statically known trait set.
/// Borrowed methods dispatch to its concrete type; owned builders and explicit
/// `#[reflect(wrapper_default)]` methods use their trait bodies.
#[doc(hidden)]
#[derive(gpui_macros::Reflect)]
#[reflect()]
pub struct ReflectedElement<Group>
where
    Group: ReflectionGroup,
{
    pub(crate) element: AnyElement,
    group: PhantomData<fn() -> Group>,
}

impl<Group> ReflectedElement<Group>
where
    Group: ReflectionGroup,
{
    #[allow(dead_code)]
    pub(crate) fn new(element: AnyElement) -> Self {
        Self {
            element,
            group: PhantomData,
        }
    }

    /// Borrows the original element and its generated method table.
    #[doc(hidden)]
    pub fn __reflection_parts_mut<Token>(
        &mut self,
        token: Token,
    ) -> (&'static Token::Methods, &mut dyn Any)
    where
        Token: CallableReflectionToken,
        Group: IncludesCallableTrait<Token>,
    {
        let methods = self.element.reflection().methods_for(token);

        (methods, self.element.inner_element_mut())
    }

    /// Borrows the original element and its generated method table.
    #[doc(hidden)]
    pub fn __reflection_parts<Token>(&self, token: Token) -> (&'static Token::Methods, &dyn Any)
    where
        Token: CallableReflectionToken,
        Group: IncludesCallableTrait<Token>,
    {
        let methods = self.element.reflection().methods_for(token);

        (methods, self.element.inner_element())
    }
}

impl<Group> Element for ReflectedElement<Group>
where
    Group: ReflectionGroup,
{
    type RequestLayoutState = <AnyElement as Element>::RequestLayoutState;
    type PrepaintState = <AnyElement as Element>::PrepaintState;

    fn reflection(&self) -> &'static ElementReflection {
        self.element.reflection()
    }

    fn into_any(self) -> AnyElement {
        self.element
    }

    fn id(&self) -> Option<ElementId> {
        <AnyElement as Element>::id(&self.element)
    }

    fn source_location(&self) -> Option<&'static panic::Location<'static>> {
        <AnyElement as Element>::source_location(&self.element)
    }

    fn request_layout(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        <AnyElement as Element>::request_layout(
            &mut self.element,
            global_id,
            inspector_id,
            window,
            cx,
        )
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
        <AnyElement as Element>::prepaint(
            &mut self.element,
            global_id,
            inspector_id,
            bounds,
            request_layout,
            window,
            cx,
        )
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
        <AnyElement as Element>::paint(
            &mut self.element,
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

impl<Group> IntoElement for ReflectedElement<Group>
where
    Group: ReflectionGroup,
{
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }

    fn into_any_element(self) -> AnyElement {
        self.element
    }
}

/// Supplies concrete registrations through `build_reflection`.
/// Handwritten and generic providers must forward [`Element::reflection`] to `Reflect::reflection`.
///
/// Registrations belong to `Self`. Deriving this trait on a [`crate::RenderOnce`]
/// component does not reflect its [`crate::ViewElement`] wrapper or rendered root.
/// For callable component traits after erasure, implement and reflect those traits
/// on the concrete [`Element`] that survives conversion.
pub trait Reflect: Sized + 'static {
    /// Builds deterministic registrations without retaining element instances or callback state.
    fn build_reflection() -> Vec<ReflectedImplementation>;

    /// Returns the process-owned metadata for this concrete type.
    fn reflection() -> &'static ElementReflection {
        metadata_for::<Self>(Self::build_reflection)
    }

    /// Returns the reflected traits implemented by this concrete type.
    fn reflected_traits() -> Vec<ReflectedTrait> {
        Self::reflection().descriptors().to_vec()
    }
}

/// A concrete implementation of a reflected trait.
pub struct ReflectedImplementation {
    /// The identity and inheritance of the implemented trait.
    pub descriptor: ReflectedTrait,
    /// Generated forwarding functions, or `None` for membership alone.
    pub methods: Option<Box<dyn Any + Send + Sync>>,
}

/// Registers a callable table once and reports whether it was inserted.
#[doc(hidden)]
pub fn __register_callable<Token: CallableReflectionToken>(
    token: Token,
    implementations: &mut Vec<ReflectedImplementation>,
    build_methods: impl FnOnce() -> Token::Methods,
) -> bool {
    let descriptor = token.reflected_trait();

    if implementations.iter().any(|implementation| {
        implementation.descriptor == descriptor
            && implementation
                .methods
                .as_deref()
                .is_some_and(|methods| methods.is::<Token::Methods>())
    }) {
        return false;
    }

    implementations.push(ReflectedImplementation {
        descriptor,
        methods: Some(Box::new(build_methods())),
    });

    true
}

/// Immutable reflection metadata retained by erased elements for process lifetime.
pub struct ElementReflection {
    concrete_type: Option<TypeId>,
    descriptors: Vec<ReflectedTrait>,
    methods: HashMap<TypeId, Box<dyn Any + Send + Sync>>,
}

impl ElementReflection {
    fn build(concrete_type: TypeId, implementations: Vec<ReflectedImplementation>) -> Self {
        let mut reflection = Self {
            concrete_type: Some(concrete_type),
            descriptors: Vec::new(),
            methods: HashMap::new(),
        };

        for implementation in implementations {
            let trait_id = implementation.descriptor.trait_type_id();

            if !reflection.descriptors.contains(&implementation.descriptor) {
                reflection.descriptors.push(implementation.descriptor);
            }

            let Some(methods) = implementation.methods else {
                continue;
            };

            if let Some(existing) = reflection.methods.get(&trait_id) {
                assert_eq!(
                    existing.as_ref().type_id(),
                    methods.as_ref().type_id(),
                    "conflicting method tables for {} on {concrete_type:?}",
                    implementation.descriptor.name,
                );

                continue;
            }

            reflection.methods.insert(trait_id, methods);
        }

        reflection
    }

    /// The concrete receiver identity, or `None` for shared unreflected metadata.
    pub fn concrete_type(&self) -> Option<TypeId> {
        self.concrete_type
    }

    /// Returns registered trait membership, including traits without adapters.
    pub fn descriptors(&self) -> &[ReflectedTrait] {
        &self.descriptors
    }

    /// Checks membership and any adapter requirement supplied by a token.
    pub fn implements_trait<Token: ReflectionToken>(&self, token: Token) -> bool {
        self.satisfies(token.requirement())
    }

    /// Checks membership and the required method table before granting callable access.
    pub fn satisfies(&self, requirement: ReflectionRequirement) -> bool {
        if !self.descriptors.contains(&requirement.descriptor) {
            return false;
        }

        if !requirement.requires_adapter {
            return true;
        }

        self.methods
            .get(&requirement.descriptor.trait_type_id())
            .is_some_and(|methods| {
                requirement
                    .adapter_type
                    .is_none_or(|expected| methods.as_ref().type_id() == expected)
            })
    }

    /// Returns whether the exact generated table for a callable token is available.
    pub fn has_adapter<Token: CallableReflectionToken>(&self, token: Token) -> bool {
        self.satisfies(ReflectionRequirement::callable(token))
    }

    fn methods_for<Token: CallableReflectionToken>(
        &'static self,
        token: Token,
    ) -> &'static Token::Methods {
        let descriptor = token.reflected_trait();

        self.methods
            .get(&descriptor.trait_type_id())
            .and_then(|methods| methods.downcast_ref())
            .unwrap_or_else(|| {
                panic!(
                    "element {:?} has no method table for {}",
                    self.concrete_type, descriptor.name
                )
            })
    }

    pub(crate) fn validate_receiver(&self, expected: TypeId) {
        if let Some(actual) = self.concrete_type {
            assert_eq!(
                actual, expected,
                "reflection metadata targets a different concrete receiver"
            );
        }
    }
}

/// A statically linked metadata factory emitted for a monomorphic derive.
#[doc(hidden)]
pub struct ReflectionRegistration {
    /// Returns the registered concrete type's identity.
    pub type_id: fn() -> TypeId,
    /// Initializes that type's metadata on demand.
    pub reflection: fn() -> &'static ElementReflection,
}

inventory::collect!(ReflectionRegistration);

type ReflectionFactory = fn() -> &'static ElementReflection;

static LINKED_REFLECTIONS: LazyLock<HashMap<TypeId, ReflectionFactory>> = LazyLock::new(|| {
    let mut factories = HashMap::new();

    for registration in inventory::iter::<ReflectionRegistration> {
        let type_id = (registration.type_id)();
        assert!(
            factories.insert(type_id, registration.reflection).is_none(),
            "duplicate reflection factories for {type_id:?}",
        );
    }

    factories
});

static REFLECTIONS: LazyLock<RwLock<HashMap<TypeId, &'static ElementReflection>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

static EMPTY_REFLECTION: LazyLock<ElementReflection> = LazyLock::new(|| ElementReflection {
    concrete_type: None,
    descriptors: Vec::new(),
    methods: HashMap::new(),
});

thread_local! {
    static INITIALIZING: RefCell<Vec<TypeId>> = const { RefCell::new(Vec::new()) };
}

struct InitializationGuard(TypeId);

impl InitializationGuard {
    fn enter(type_id: TypeId) -> Self {
        INITIALIZING.with(|stack| {
            let mut stack = stack.borrow_mut();
            assert!(
                !stack.contains(&type_id),
                "recursive reflection initialization for {type_id:?}"
            );
            stack.push(type_id);
        });

        Self(type_id)
    }
}

impl Drop for InitializationGuard {
    fn drop(&mut self) {
        INITIALIZING.with(|stack| {
            let completed = stack.borrow_mut().pop();

            debug_assert_eq!(completed, Some(self.0));
        });
    }
}

/// Initializes a concrete registration outside the cache lock and retains one immutable winner.
#[doc(hidden)]
pub fn metadata_for<Type: 'static>(
    build: fn() -> Vec<ReflectedImplementation>,
) -> &'static ElementReflection {
    let type_id = TypeId::of::<Type>();

    if let Some(reflection) = REFLECTIONS.read().get(&type_id).copied() {
        return reflection;
    }

    let _initialization = InitializationGuard::enter(type_id);
    let candidate = Box::new(ElementReflection::build(type_id, build()));
    let mut cache = REFLECTIONS.write();

    if let Some(reflection) = cache.get(&type_id).copied() {
        drop(cache);
        drop(candidate);

        return reflection;
    }

    let reflection = Box::leak(candidate);
    cache.insert(type_id, reflection);

    reflection
}

/// Resolves linked metadata, or metadata already initialized through a concrete provider.
/// Ordinary unreflected types share an empty entry and do not populate the demand cache.
#[doc(hidden)]
pub fn linked_metadata_for<Type: 'static>() -> &'static ElementReflection {
    linked_metadata(TypeId::of::<Type>())
}

fn linked_metadata(type_id: TypeId) -> &'static ElementReflection {
    if let Some(reflection) = REFLECTIONS.read().get(&type_id).copied() {
        return reflection;
    }

    let Some(factory) = LINKED_REFLECTIONS.get(&type_id) else {
        return &EMPTY_REFLECTION;
    };

    let reflection = factory();
    reflection.validate_receiver(type_id);
    assert_eq!(
        reflection.concrete_type(),
        Some(type_id),
        "linked reflection factory returned empty metadata"
    );

    reflection
}

/// Returns descriptors from the same provider used by concrete and erased elements.
/// Generic registrations become visible after their metadata has been requested.
#[doc(hidden)]
pub fn registered_traits(type_id: TypeId) -> &'static [ReflectedTrait] {
    linked_metadata(type_id).descriptors()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate as gpui;
    use crate::reflection::component_example::{CardElement, Draggable, Icon};
    use crate::{
        AppContext, Context, Div, Empty, InteractiveElement, MouseButton, ParentElement, Render,
        StatefulInteractiveElement, StyleRefinement, Styled, TestApp, div, hsla, point, px, rgb,
    };
    use std::{
        cell::{Cell, RefCell},
        rc::Rc,
        sync::{
            Barrier,
            atomic::{AtomicUsize, Ordering},
        },
        thread,
    };

    const TEXT_LABEL: &str = "body";

    mod text {
        #[gpui_macros::reflect_trait]
        pub trait Text {
            fn read<'element>(&'element self, label: &str) -> &'element str;
            fn edit(&mut self, label: &str) -> &mut String;
            fn append(&mut self, characters: impl IntoIterator<Item = char>);
            fn formatter(&'_ self) -> fn(&str) -> &str;

            fn content(&self) -> &str {
                self.read("body")
            }

            fn revise(&mut self) -> &mut String {
                let content = self.edit("body");
                content.push('?');

                content
            }

            #[reflect(wrapper_default)]
            fn suffix(&mut self, suffix: impl AsRef<str>) {
                self.edit("body").push_str(suffix.as_ref());
            }

            #[reflect(wrapper_default)]
            fn wrapper_label(&self) -> &'static str {
                "wrapper"
            }

            #[cfg(all())]
            #[cfg_attr(any(), cfg(any()))]
            fn configured(&self) -> &str {
                self.content()
            }

            #[cfg(any())]
            fn unavailable_direct(&self) -> UnavailableType;

            #[cfg_attr(all(), cfg_attr(all(), cfg(any()), allow(dead_code)))]
            fn unavailable_nested(&self) -> UnavailableType {
                unreachable!()
            }

            #[cfg_attr(all(), cfg(any()))]
            fn unavailable(&mut self) -> UnavailableType;
        }
    }

    mod branches {
        use crate::reflection::tests::text;

        #[gpui_macros::reflect_trait]
        pub trait Left: text::Text {
            fn decorate(mut self, suffix: impl AsRef<str>) -> Self
            where
                Self: Sized,
            {
                self.edit("body").push_str(suffix.as_ref());

                self
            }
        }

        #[gpui_macros::reflect_trait]
        pub trait Right: text::Text {}
    }

    #[gpui_macros::reflect_trait]
    trait Composite: branches::Left + branches::Right + crate::Styled + crate::ParentElement {}

    #[gpui_macros::reflect_trait(membership)]
    trait PaintSource {}

    #[derive(gpui_macros::Reflect, Default)]
    #[reflect(Composite, text::Text, crate::Styled, PaintSource)]
    struct Card {
        text: String,
        calls: Rc<Cell<usize>>,
        style: StyleRefinement,
        children: Vec<AnyElement>,
    }

    impl text::Text for Card {
        fn read<'element>(&'element self, _label: &str) -> &'element str {
            &self.text
        }

        fn edit(&mut self, _label: &str) -> &mut String {
            self.calls.set(self.calls.get() + 1);

            &mut self.text
        }

        fn append(&mut self, characters: impl IntoIterator<Item = char>) {
            self.text.extend(characters);
        }

        fn formatter(&'_ self) -> fn(&str) -> &str {
            |value| value
        }
    }

    #[derive(Default)]
    struct Panel<State> {
        card: Card,
        text_style: crate::TextStyleRefinement,
        state: State,
    }

    impl<State: 'static> Reflect for Panel<State> {
        fn build_reflection() -> Vec<ReflectedImplementation> {
            let mut implementations = Vec::new();
            Composite.__register::<Self>(&mut implementations);
            PaintSource.__register::<Self>(&mut implementations);

            implementations
        }
    }

    impl PaintSource for Card {}

    impl<State> PaintSource for Panel<State> {}

    impl<State> text::Text for Panel<State> {
        fn read<'element>(&'element self, label: &str) -> &'element str {
            text::Text::read(&self.card, label)
        }

        fn edit(&mut self, label: &str) -> &mut String {
            text::Text::edit(&mut self.card, label)
        }

        fn append(&mut self, characters: impl IntoIterator<Item = char>) {
            text::Text::append(&mut self.card, characters);
        }

        fn formatter(&'_ self) -> fn(&str) -> &str {
            text::Text::formatter(&self.card)
        }

        fn content(&self) -> &str {
            "panel override"
        }

        fn revise(&mut self) -> &mut String {
            self.card.text.push('!');

            &mut self.card.text
        }

        fn suffix(&mut self, _suffix: impl AsRef<str>) {
            panic!("explicit wrapper defaults must not dispatch concrete overrides");
        }

        fn wrapper_label(&self) -> &'static str {
            "concrete"
        }
    }

    impl<State> branches::Left for Panel<State> {}

    impl<State> branches::Right for Panel<State> {}

    impl<State> Composite for Panel<State> {}

    impl<State> Styled for Panel<State> {
        fn style(&mut self) -> &mut StyleRefinement {
            &mut self.card.style
        }

        fn text_style(&mut self) -> &mut crate::TextStyleRefinement {
            &mut self.text_style
        }
    }

    impl<State> ParentElement for Panel<State> {
        fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
            self.card.children.extend(elements);
        }
    }

    impl branches::Left for Card {
        fn decorate(mut self, suffix: impl AsRef<str>) -> Self {
            self.text.push_str("concrete builder ");
            self.text.push_str(suffix.as_ref());

            self
        }
    }

    impl branches::Right for Card {}

    impl Composite for Card {}

    impl Styled for Card {
        fn style(&mut self) -> &mut StyleRefinement {
            &mut self.style
        }
    }

    impl ParentElement for Card {
        fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
            self.children.extend(elements);
        }
    }

    element_impl!(Card);
    element_impl!(Panel<State>, [State: 'static], reflection <Self as Reflect>::reflection());

    fn card() -> Card {
        Card {
            text: "hello".into(),
            ..Default::default()
        }
    }

    fn admit<Traits>(element: AnyElement, traits: Traits) -> Option<ReflectedElement<Traits::Group>>
    where
        Traits: ReflectedTraits,
    {
        let requirements = traits.reflected_requirements();

        if !requirements
            .iter()
            .all(|requirement| element.reflection().satisfies(*requirement))
        {
            return None;
        }

        Some(ReflectedElement::new(element))
    }

    fn transform<Traits>(element: AnyElement, traits: Traits) -> AnyElement
    where
        Traits: ReflectedTraits,
        ReflectedElement<Traits::Group>: Composite,
    {
        let mut element = admit(element, traits).unwrap();
        let label = TEXT_LABEL;

        assert_eq!(text::Text::read(&element, label), "hello");
        text::Text::edit(&mut element, label).push(' ');

        let borrowed = String::from("world");
        let formatter = text::Text::formatter(&element);

        assert_eq!(formatter(&borrowed), "world");
        text::Text::append(&mut element, borrowed.chars());

        branches::Left::decorate(element, "!")
            .bg(rgb(0x123456))
            .invisible()
            .child(Empty)
            .into_any_element()
    }

    #[test]
    fn forwards_methods_and_owned_builders_through_inherited_groups() {
        let concrete = branches::Left::decorate(card(), "!");

        assert_eq!(concrete.text, "helloconcrete builder !");

        for combined in [false, true] {
            let card = card();
            let calls = card.calls.clone();
            let element = card.into_any_element();

            let mut element = if combined {
                transform(
                    element,
                    trait_set![
                        Composite,
                        text::Text,
                        branches::Left,
                        crate::Styled,
                        crate::ParentElement,
                        Composite,
                    ],
                )
            } else {
                transform(element, Composite)
            };

            let card = element.downcast_mut::<Card>().unwrap();

            assert_eq!(card.text, "hello world!");
            assert_eq!(calls.get(), 2);
            assert!(card.style.background.is_some());
            assert_eq!(card.style.visibility, Some(crate::Visibility::Hidden));
            assert_eq!(card.children.len(), 1);
        }
    }

    #[test]
    fn dispatches_defaults_and_overrides_through_one_diamond_group() {
        let color = hsla(0.5, 0.5, 0.5, 1.0);

        for (overrides, content, revised) in [
            (false, "hello", "hello?"),
            (true, "panel override", "hello!"),
        ] {
            let original = if overrides {
                Panel {
                    card: card(),
                    state: String::from("manual"),
                    ..Default::default()
                }
                .into_any_element()
            } else {
                card().into_any_element()
            };

            let mut element = admit(
                original,
                trait_set![PaintSource, Composite, text::Text, PaintSource, Composite],
            )
            .unwrap();

            assert_eq!(text::Text::content(&element), content);
            assert_eq!(text::Text::configured(&element), content);
            assert_eq!(text::Text::revise(&mut element), revised);
            assert_eq!(text::Text::wrapper_label(&element), "wrapper");
            text::Text::suffix(&mut element, " suffix");

            let mut element = element.text_color(color).into_any_element();
            let (card, text_color) = if overrides {
                let panel = element.downcast_mut::<Panel<String>>().unwrap();

                assert_eq!(panel.state, "manual");

                (&panel.card, panel.text_style.color)
            } else {
                let card = element.downcast_mut::<Card>().unwrap();

                (&*card, card.style.text.color)
            };

            assert_eq!(card.text, format!("{revised} suffix"));
            assert_eq!(text_color, Some(color));
        }
    }

    #[test]
    fn registers_callable_tables_lazily_by_descriptor_and_type() {
        let descriptor = branches::Right.reflected_trait();

        for existing in [
            None,
            Some(ReflectedImplementation {
                descriptor,
                methods: None,
            }),
            Some(ReflectedImplementation {
                descriptor,
                methods: Some(Box::new(123usize)),
            }),
            Some(ReflectedImplementation {
                descriptor: text::Text.reflected_trait(),
                methods: Some(Box::new(branches::__GpuiReflectRightMethods {})),
            }),
        ] {
            let mut implementations = existing.into_iter().collect::<Vec<_>>();
            let initial_count = implementations.len();
            let calls = Cell::new(0);
            let build_methods = || {
                calls.set(calls.get() + 1);

                branches::__GpuiReflectRightMethods {}
            };

            assert!(__register_callable(
                branches::Right,
                &mut implementations,
                build_methods,
            ));
            assert!(!__register_callable(
                branches::Right,
                &mut implementations,
                build_methods,
            ));
            assert_eq!(calls.get(), 1);
            assert_eq!(implementations.len(), initial_count + 1);

            let inserted = implementations.last().unwrap();

            assert_eq!(inserted.descriptor, descriptor);
            assert!(
                inserted
                    .methods
                    .as_deref()
                    .unwrap()
                    .is::<branches::__GpuiReflectRightMethods>()
            );
        }
    }

    #[test]
    fn records_inheritance_and_deduplicates_concrete_implementations() {
        let descriptor = ReflectionToken::reflected_trait(Composite);
        let parents = (descriptor.supertraits)();

        assert_eq!(parents.len(), 4);
        assert_eq!(
            (parents[0].supertraits)(),
            &[ReflectionToken::reflected_trait(text::Text)],
        );
        assert_eq!(
            (parents[1].supertraits)(),
            &[ReflectionToken::reflected_trait(text::Text)],
        );

        let traits = Card::reflected_traits();
        assert_eq!(traits.len(), 7);
        assert!(traits.contains(&descriptor));
        assert!(traits.contains(&ReflectionToken::reflected_trait(text::Text)));

        let metadata = <Card as Reflect>::reflection();
        let requirements = Composite.reflected_requirements();
        let combined = trait_set![Composite, text::Text].reflected_requirements();

        assert_eq!(requirements.len(), 6);
        assert_eq!(combined.len(), 6);
        assert!(requirements.iter().all(|requirement| {
            requirement.requires_adapter && metadata.satisfies(*requirement)
        }));
        assert!(
            combined
                .iter()
                .all(|requirement| requirements.contains(requirement))
        );

        let mixed = trait_set![PaintSource, Composite, text::Text, PaintSource, Composite]
            .reflected_requirements();

        assert_eq!(mixed.len(), 7);
        assert_eq!(mixed[0], PaintSource.requirement());
        assert!(!mixed[0].requires_adapter);
        assert!(
            mixed
                .iter()
                .all(|requirement| metadata.satisfies(*requirement))
        );
        assert_eq!(PaintSource.requirements(), vec![PaintSource.requirement()]);

        for wrong_table in [false, true] {
            let mut implementations = <Card as Reflect>::build_reflection();
            let inherited = implementations
                .iter_mut()
                .find(|implementation| implementation.descriptor == text::Text.reflected_trait())
                .unwrap();
            inherited.methods =
                wrong_table.then(|| Box::new(123usize) as Box<dyn Any + Send + Sync>);

            let incomplete = ElementReflection::build(TypeId::of::<Card>(), implementations);

            assert!(incomplete.has_adapter(Composite));
            assert!(incomplete.satisfies(PaintSource.requirement()));
            assert!(!incomplete.has_adapter(text::Text));
            assert!(
                !mixed
                    .iter()
                    .all(|requirement| incomplete.satisfies(*requirement))
            );
        }
    }

    #[test]
    fn transparent_erasure_preserves_concrete_identity() {
        fn reflected<Token: ReflectionToken>(
            element: AnyElement,
            _token: Token,
        ) -> ReflectedElement<Token::Group> {
            ReflectedElement::new(element)
        }

        for route in 0..7 {
            let mut element = div().id("original").child(Empty).into_any_element();
            let metadata = element.reflection();
            let original = element.downcast_mut::<Div>().unwrap() as *mut Div;
            let reflected = reflected(element, crate::InteractiveElement);
            let mut element = match route {
                0 => reflected.into_any_element(),
                1 => reflected.into_any(),
                2 => reflected.into_element().into_any(),
                3 => reflected.id("renamed").into_any_element(),
                4 => reflected.id("renamed").into_any(),
                5 => reflected.id("renamed").into_element().into_any(),
                _route => reflected.into_any_element().into_any(),
            };

            let expected_id = if (3..6).contains(&route) {
                "renamed"
            } else {
                "original"
            };

            let concrete = element.downcast_mut::<Div>().unwrap();

            assert_eq!(concrete as *mut Div, original);
            assert_eq!(Element::id(concrete), Some(ElementId::from(expected_id)));
            assert_eq!(element.reflected_type_id(), TypeId::of::<Div>());
            assert!(std::ptr::eq(metadata, element.reflection()));
        }
    }

    struct ComponentView {
        use_component: bool,
        hovers: Rc<RefCell<Vec<bool>>>,
        clicks: Rc<Cell<usize>>,
    }

    fn configure_component<Target>(
        element: Target,
        hovers: Rc<RefCell<Vec<bool>>>,
        clicks: Rc<Cell<usize>>,
    ) -> Target
    where
        Target: Styled + ParentElement + StatefulInteractiveElement,
    {
        let element = element
            .w(px(160.))
            .h(px(80.))
            .bg(rgb(0x123456))
            .role(accesskit::Role::Button)
            .accessibility_id("card")
            .aria_label("Draggable card")
            .on_hover(move |hovered, _window, _cx| hovers.borrow_mut().push(*hovered))
            .on_click(move |_event, _window, _cx| clicks.set(clicks.get() + 1))
            .child(
                div()
                    .id("child")
                    .size(px(24.))
                    .role(accesskit::Role::Image)
                    .aria_label("Child"),
            );

        StatefulInteractiveElement::a11y_synthetic_children(element, |builder| {
            let node_id = builder.synthetic_node_id("payload");
            let mut node = accesskit::Node::new(accesskit::Role::Label);
            node.set_label("Payload");
            assert!(builder.push_child(node_id, node));
        })
    }

    fn reflect_component<Traits>(
        element: AnyElement,
        _traits: Traits,
        hovers: Rc<RefCell<Vec<bool>>>,
        clicks: Rc<Cell<usize>>,
    ) -> AnyElement
    where
        Traits: ReflectedTraits,
        ReflectedElement<Traits::Group>:
            Draggable + Styled + ParentElement + StatefulInteractiveElement,
    {
        let metadata = element.reflection();
        let mut reflected = ReflectedElement::<Traits::Group>::new(element);
        *reflected.drag_payload() = Some("card-data".into());

        let mut element = configure_component(reflected, hovers, clicks).into_any_element();

        assert_eq!(
            element.downcast_mut::<CardElement>().unwrap().drag_payload,
            Some("card-data".into())
        );
        assert!(std::ptr::eq(metadata, element.reflection()));
        assert_eq!(metadata.concrete_type(), Some(TypeId::of::<CardElement>()));

        element
    }

    impl Render for ComponentView {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            let element = if self.use_component {
                reflect_component(
                    CardElement::new("card").into_any_element(),
                    trait_set![
                        crate::reflection::component_example::Draggable,
                        crate::Styled,
                        crate::ParentElement,
                        crate::StatefulInteractiveElement,
                    ],
                    self.hovers.clone(),
                    self.clicks.clone(),
                )
            } else {
                configure_component(div().id("card"), self.hovers.clone(), self.clicks.clone())
                    .into_any_element()
            };

            div()
                .flex()
                .flex_col()
                .child(element)
                .child(Icon::default())
        }
    }

    #[test]
    fn component_delegation_preserves_reflected_mutations_state_and_accessibility() {
        let mut app = TestApp::new();
        let hovers = Rc::new(RefCell::new(Vec::new()));
        let clicks = Rc::new(Cell::new(0));
        let mut window = app.open_window({
            let hovers = hovers.clone();
            let clicks = clicks.clone();

            move |_window, _cx| ComponentView {
                use_component: false,
                hovers,
                clicks,
            }
        });
        let handle = window.handle().into();
        app.update(|cx| {
            cx.update_window(handle, |_view, window, cx| {
                window.set_a11y_forced(true);
                window.draw(cx).clear(cx);
            })
            .unwrap();
        });

        let inspect_tree = |window: &Window| {
            let tree = window.a11y_tree().unwrap();
            let (card_id, card) = tree
                .nodes
                .iter()
                .find(|(_node_id, node)| node.author_id() == Some("card"))
                .unwrap();
            let (child_id, _child) = tree
                .nodes
                .iter()
                .find(|(_node_id, node)| node.label() == Some("Child"))
                .unwrap();
            let (payload_id, _payload) = tree
                .nodes
                .iter()
                .find(|(_node_id, node)| node.label() == Some("Payload"))
                .unwrap();

            assert_eq!(card.role(), accesskit::Role::Button);
            assert_eq!(card.label(), Some("Draggable card"));
            assert!(card.children().contains(child_id));
            assert!(card.children().contains(payload_id));
            assert_eq!(
                window.a11y_node_bounds(*card_id).unwrap().size,
                crate::size(px(160.), px(80.))
            );
            assert_eq!(
                window.a11y_node_bounds(*child_id).unwrap().size,
                crate::size(px(24.), px(24.))
            );

            (*card_id, *child_id, *payload_id)
        };
        let direct_ids = window.update(|_view, window, _cx| inspect_tree(window));

        window.simulate_mouse_move(point(px(80.), px(40.)));
        assert_eq!(*hovers.borrow(), [true]);

        for use_component in [true, true, false] {
            window.update(|view, _window, cx| {
                view.use_component = use_component;
                cx.notify();
            });
            app.update(|cx| {
                cx.update_window(handle, |_view, window, cx| {
                    window.draw(cx).clear(cx);
                    assert_eq!(inspect_tree(window), direct_ids);
                })
                .unwrap();
            });
            assert_eq!(*hovers.borrow(), [true]);
            window.simulate_click(point(px(80.), px(40.)), MouseButton::Left);
        }

        assert_eq!(clicks.get(), 3);
        window.simulate_mouse_move(point(px(200.), px(100.)));
        assert_eq!(*hovers.borrow(), [true, false]);
    }

    #[test]
    fn provider_publishes_one_owner_and_recovers_from_initialization_failures() {
        const WORKERS: usize = 8;
        static BARRIER: LazyLock<Barrier> = LazyLock::new(|| Barrier::new(WORKERS));
        static BUILDS: AtomicUsize = AtomicUsize::new(0);
        static DROPS: AtomicUsize = AtomicUsize::new(0);
        static LINKED_BUILDS: AtomicUsize = AtomicUsize::new(0);

        struct Concurrent;
        struct TrackedTable;
        struct Panicking;
        struct Recursive;
        struct Nested;
        struct Conflicting;
        struct Linked;
        struct WrongLinked;

        impl Reflect for Linked {
            fn build_reflection() -> Vec<ReflectedImplementation> {
                LINKED_BUILDS.fetch_add(1, Ordering::SeqCst);

                Vec::new()
            }
        }

        inventory::submit! {
            ReflectionRegistration {
                type_id: || TypeId::of::<Linked>(),
                reflection: <Linked as Reflect>::reflection,
            }
        }

        inventory::submit! {
            ReflectionRegistration {
                type_id: || TypeId::of::<WrongLinked>(),
                reflection: <Linked as Reflect>::reflection,
            }
        }

        impl Drop for TrackedTable {
            fn drop(&mut self) {
                DROPS.fetch_add(1, Ordering::SeqCst);
            }
        }

        fn registration<Table: Any + Send + Sync>(
            methods: Option<Table>,
        ) -> ReflectedImplementation {
            ReflectedImplementation {
                descriptor: crate::Styled.reflected_trait(),
                methods: methods.map(|methods| Box::new(methods) as Box<dyn Any + Send + Sync>),
            }
        }

        fn assert_panics<Result>(action: impl FnOnce() -> Result) {
            assert!(panic::catch_unwind(panic::AssertUnwindSafe(action)).is_err());
        }

        fn concurrent() -> Vec<ReflectedImplementation> {
            BUILDS.fetch_add(1, Ordering::SeqCst);
            BARRIER.wait();

            vec![registration(Some(TrackedTable))]
        }

        fn recursive() -> Vec<ReflectedImplementation> {
            metadata_for::<Nested>(|| {
                metadata_for::<Recursive>(recursive);

                Vec::new()
            });

            Vec::new()
        }

        assert_eq!(LINKED_BUILDS.load(Ordering::SeqCst), 0);
        assert!(linked_metadata_for::<()>().descriptors().is_empty());
        assert_eq!(LINKED_BUILDS.load(Ordering::SeqCst), 0);
        assert!(std::ptr::eq(
            linked_metadata_for::<Linked>(),
            <Linked as Reflect>::reflection()
        ));
        assert_eq!(LINKED_BUILDS.load(Ordering::SeqCst), 1);
        assert_panics(linked_metadata_for::<WrongLinked>);

        let descriptor = crate::Styled.reflected_trait();
        let normalized = ElementReflection::build(
            TypeId::of::<Concurrent>(),
            [None, Some(1usize), Some(2), None]
                .into_iter()
                .map(registration)
                .collect(),
        );
        let descriptor_only = ElementReflection::build(
            TypeId::of::<Concurrent>(),
            vec![registration::<usize>(None)],
        );

        assert_eq!(normalized.descriptors(), &[descriptor]);
        assert!(normalized.satisfies(ReflectionRequirement {
            descriptor,
            requires_adapter: true,
            adapter_type: Some(TypeId::of::<usize>()),
        }));

        let callers = (0..WORKERS)
            .map(|_idx| thread::spawn(|| metadata_for::<Concurrent>(concurrent)))
            .collect::<Vec<_>>();
        let entries = callers
            .into_iter()
            .map(|caller| caller.join().unwrap())
            .collect::<Vec<_>>();

        assert!(entries.iter().all(|entry| std::ptr::eq(*entry, entries[0])));
        assert_eq!(BUILDS.load(Ordering::SeqCst), WORKERS);
        assert_eq!(DROPS.load(Ordering::SeqCst), WORKERS - 1);

        for metadata in [&normalized, &descriptor_only, entries[0]] {
            assert!(metadata.implements_trait(descriptor));
            assert!(metadata.satisfies(descriptor.requirement()));
            assert!(!metadata.implements_trait(crate::Styled));
            assert!(!metadata.has_adapter(crate::Styled));
            assert!(!metadata.satisfies(crate::Styled.requirement()));
        }

        assert_panics(|| entries[0].validate_receiver(TypeId::of::<Nested>()));
        assert_panics(|| metadata_for::<Panicking>(|| panic!("factory panic")));
        assert_panics(|| metadata_for::<Recursive>(recursive));
        assert_panics(|| {
            metadata_for::<Conflicting>(|| {
                vec![
                    registration(Some(1usize)),
                    registration(Some(String::new())),
                ]
            })
        });

        for entry in [
            metadata_for::<Panicking>(Vec::new),
            metadata_for::<Recursive>(|| {
                metadata_for::<Nested>(Vec::new);

                Vec::new()
            }),
            metadata_for::<Conflicting>(Vec::new),
        ] {
            assert!(entry.descriptors().is_empty());
            assert!(entry.concrete_type().is_some());
        }

        assert!(INITIALIZING.with(|stack| stack.borrow().is_empty()));
    }
}
