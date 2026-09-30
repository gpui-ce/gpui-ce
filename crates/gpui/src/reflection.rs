//! Trait reflection and typed access to erased elements.

use crate::{
    AnyElement, App, Bounds, Element, ElementId, GlobalElementId, InspectorElementId,
    Interactivity, IntoElement, LayoutId, Pixels, StyleRefinement, Window,
};
pub use gpui_macros::{Reflect, reflect_trait, trait_set};
use smallvec::SmallVec;
use std::{
    any::{Any, TypeId},
    collections::HashMap,
    marker::PhantomData,
    panic,
    sync::LazyLock,
};

/// Accesses a reflected element's style state.
#[doc(hidden)]
pub type ReflectedStyleAccessor =
    for<'element> fn(&'element mut dyn Any) -> &'element mut StyleRefinement;

/// Accesses a reflected element's interactivity state.
#[doc(hidden)]
pub type ReflectedInteractivityAccessor =
    for<'element> fn(&'element mut dyn Any) -> &'element mut Interactivity;

/// Adds children to a reflected parent element.
#[doc(hidden)]
pub type ReflectedParentExtender = fn(&mut dyn Any, Vec<AnyElement>);

/// Operations exposed by GPUI's preset reflected traits.
#[derive(Default)]
#[doc(hidden)]
pub struct ReflectedCapabilities {
    /// Accesses `Styled::style` when the concrete type implements `Styled`.
    pub style: Option<ReflectedStyleAccessor>,
    /// Accesses `InteractiveElement::interactivity` when implemented.
    pub interactivity: Option<ReflectedInteractivityAccessor>,
    /// Calls `ParentElement::extend` when implemented.
    pub extend: Option<ReflectedParentExtender>,
}

/// Identifies a trait made available to element reflection.
#[derive(Clone, Copy, Debug)]
pub struct ReflectedTrait {
    /// The fully qualified name of the trait.
    pub name: &'static str,
    type_id: fn() -> TypeId,
}

impl ReflectedTrait {
    /// Creates a descriptor for a unique trait marker.
    #[doc(hidden)]
    pub const fn new(name: &'static str, type_id: fn() -> TypeId) -> Self {
        Self { name, type_id }
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

/// A typed descriptor for a trait made available to reflection.
#[doc(hidden)]
pub trait ReflectionToken: Copy + 'static {
    /// The reflection group produced when this token is used by itself.
    type Group: ReflectionGroup;

    /// The erased runtime descriptor for this trait.
    fn reflected_trait(self) -> ReflectedTrait;
}

impl ReflectionToken for ReflectedTrait {
    type Group = ErasedReflectionGroup;

    fn reflected_trait(self) -> ReflectedTrait {
        self
    }
}

/// The group used when only an erased runtime descriptor is available.
#[doc(hidden)]
pub struct ErasedReflectionGroup;

impl ReflectionGroup for ErasedReflectionGroup {}

/// Marks a type generated to represent a set of reflected traits.
#[doc(hidden)]
pub trait ReflectionGroup: 'static {}

/// Proves that a reflected trait is included in a generated reflection group.
#[doc(hidden)]
pub trait IncludesReflectedTrait<Token>: ReflectionGroup
where
    Token: ReflectionToken,
{
}

/// A value containing the runtime descriptors for a generated reflection group.
#[doc(hidden)]
pub struct ReflectedTraitGroup<Group>
where
    Group: ReflectionGroup,
{
    traits: SmallVec<[ReflectedTrait; 2]>,
    group: PhantomData<fn() -> Group>,
}

impl<Group> ReflectedTraitGroup<Group>
where
    Group: ReflectionGroup,
{
    /// Creates a reflected trait group from its runtime descriptors.
    #[doc(hidden)]
    pub fn new(traits: impl IntoIterator<Item = ReflectedTrait>) -> Self {
        Self {
            traits: traits.into_iter().collect(),
            group: PhantomData,
        }
    }
}

/// Converts one or more typed reflection descriptors into a selector trait set.
#[doc(hidden)]
pub trait ReflectedTraits {
    /// The generated type that records membership of every reflected trait.
    type Group: ReflectionGroup;

    /// Returns the erased runtime descriptors in this trait set.
    fn reflected_traits(self) -> SmallVec<[ReflectedTrait; 2]>;
}

impl<Token> ReflectedTraits for Token
where
    Token: ReflectionToken,
    Token::Group: ReflectionGroup,
{
    type Group = Token::Group;

    fn reflected_traits(self) -> SmallVec<[ReflectedTrait; 2]> {
        std::iter::once(self.reflected_trait()).collect()
    }
}

impl<Group> ReflectedTraits for ReflectedTraitGroup<Group>
where
    Group: ReflectionGroup,
{
    type Group = Group;

    fn reflected_traits(self) -> SmallVec<[ReflectedTrait; 2]> {
        self.traits
    }
}

/// An owned erased element exposing a statically known set of reflected traits.
#[doc(hidden)]
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
}

impl<Group> Element for ReflectedElement<Group>
where
    Group: ReflectionGroup,
{
    type RequestLayoutState = <AnyElement as Element>::RequestLayoutState;
    type PrepaintState = <AnyElement as Element>::PrepaintState;

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

/// Implemented by `#[derive(Reflect)]` for concrete types whose traits can be inspected.
///
/// The derive detects GPUI's four preset traits, including handwritten implementations.
/// Use `#[reflect(MyTrait)]` to expose a custom trait marked with
/// `#[gpui::reflection::reflect_trait]`.
pub trait Reflect: 'static {
    /// Returns the reflected traits implemented by this type.
    fn reflected_traits() -> Vec<ReflectedTrait>;
}

/// A statically linked registration emitted by `#[derive(Reflect)]`.
#[doc(hidden)]
pub struct ReflectionRegistration {
    /// Returns the registered concrete type's identity.
    pub type_id: fn() -> TypeId,
    /// Returns its reflected traits.
    pub traits: fn() -> Vec<ReflectedTrait>,
    /// Returns operations for its reflected preset traits.
    pub capabilities: fn() -> ReflectedCapabilities,
}

inventory::collect!(ReflectionRegistration);

struct RegisteredTraits {
    descriptors: Vec<ReflectedTrait>,
    type_ids: Vec<TypeId>,
    capabilities: ReflectedCapabilities,
}

static REFLECTIONS: LazyLock<HashMap<TypeId, RegisteredTraits>> = LazyLock::new(|| {
    inventory::iter::<ReflectionRegistration>
        .into_iter()
        .map(|registration| {
            let descriptors = (registration.traits)();
            let type_ids = descriptors
                .iter()
                .map(ReflectedTrait::trait_type_id)
                .collect();
            let capabilities = (registration.capabilities)();

            (
                (registration.type_id)(),
                RegisteredTraits {
                    descriptors,
                    type_ids,
                    capabilities,
                },
            )
        })
        .collect()
});

pub(crate) fn traits_for(type_id: TypeId) -> &'static [ReflectedTrait] {
    REFLECTIONS
        .get(&type_id)
        .map(|registration| registration.descriptors.as_slice())
        .unwrap_or(&[])
}

pub(crate) fn implements_trait(type_id: TypeId, reflected_trait: ReflectedTrait) -> bool {
    let Some(registration) = REFLECTIONS.get(&type_id) else {
        return false;
    };

    registration
        .type_ids
        .contains(&reflected_trait.trait_type_id())
}

pub(crate) fn style_accessor(type_id: TypeId) -> Option<ReflectedStyleAccessor> {
    REFLECTIONS
        .get(&type_id)
        .and_then(|registration| registration.capabilities.style)
}

pub(crate) fn interactivity_accessor(type_id: TypeId) -> Option<ReflectedInteractivityAccessor> {
    REFLECTIONS
        .get(&type_id)
        .and_then(|registration| registration.capabilities.interactivity)
}

pub(crate) fn parent_extender(type_id: TypeId) -> Option<ReflectedParentExtender> {
    REFLECTIONS
        .get(&type_id)
        .and_then(|registration| registration.capabilities.extend)
}
