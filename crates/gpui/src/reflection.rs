use std::{any::TypeId, collections::HashMap, sync::LazyLock};

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
}

impl PartialEq for ReflectedTrait {
    fn eq(&self, other: &Self) -> bool {
        (self.type_id)() == (other.type_id)()
    }
}

impl Eq for ReflectedTrait {}

/// Implemented by `#[derive(Reflect)]` for concrete types whose traits can be inspected.
///
/// The derive detects GPUI's four preset traits, including handwritten implementations.
/// Use `#[reflect(MyTrait)]` to expose a custom trait marked with
/// `#[gpui::reflect_trait]`.
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
}

inventory::collect!(ReflectionRegistration);

static REFLECTIONS: LazyLock<HashMap<TypeId, Vec<ReflectedTrait>>> = LazyLock::new(|| {
    inventory::iter::<ReflectionRegistration>
        .into_iter()
        .map(|registration| ((registration.type_id)(), (registration.traits)()))
        .collect()
});

pub(crate) fn traits_for(type_id: TypeId) -> &'static [ReflectedTrait] {
    REFLECTIONS.get(&type_id).map(Vec::as_slice).unwrap_or(&[])
}
