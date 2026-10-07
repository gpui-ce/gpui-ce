use crate::{App, BorrowAppContext};
use std::{
    any::TypeId,
    hash::{BuildHasher, Hasher},
};

/// A marker trait for types that can be stored in GPUI's global state.
///
/// This trait exists to provide type-safe access to globals by ensuring only
/// types that implement [`Global`] can be used with the accessor methods. For
/// example, trying to access a global with a type that does not implement
/// [`Global`] will result in a compile-time error.
///
/// Implement this on types you want to store in the context as a global.
///
/// ## Restricting Access to Globals
///
/// In some situations you may need to store some global state, but want to
/// restrict access to reading it or writing to it.
///
/// In these cases, Rust's visibility system can be used to restrict access to
/// a global value. For example, you can create a private struct that implements
/// [`Global`] and holds the global state. Then create a newtype struct that wraps
/// the global type and create custom accessor methods to expose the desired subset
/// of operations.
#[deprecated(note = "migrating to entity-based globals")]
pub trait Global: 'static {
    // This trait is intentionally left empty, by virtue of being a marker trait.
    //
    // Use additional traits with blanket implementations to attach functionality
    // to types that implement `Global`.
}

/// A trait for reading a global value from the context.
#[deprecated(note = "migrating to entity-based globals")]
pub trait ReadGlobal {
    /// Returns the global instance of the implementing type.
    ///
    /// Panics if a global for that type has not been assigned.
    fn global(cx: &App) -> &Self;
}

impl<T: Global> ReadGlobal for T {
    fn global(cx: &App) -> &Self {
        cx.global::<T>()
    }
}

/// A trait for updating a global value in the context.
#[deprecated(note = "migrating to entity-based globals")]
pub trait UpdateGlobal {
    /// Updates the global instance of the implementing type using the provided closure.
    ///
    /// This method provides the closure with mutable access to the context and the global simultaneously.
    fn update_global<C, F, R>(cx: &mut C, update: F) -> R
    where
        C: BorrowAppContext,
        F: FnOnce(&mut Self, &mut C) -> R;

    /// Set the global instance of the implementing type.
    fn set_global<C>(cx: &mut C, global: Self)
    where
        C: BorrowAppContext;
}

impl<T: Global> UpdateGlobal for T {
    #[track_caller]
    fn update_global<C, F, R>(cx: &mut C, update: F) -> R
    where
        C: BorrowAppContext,
        F: FnOnce(&mut Self, &mut C) -> R,
    {
        cx.update_global(update)
    }

    fn set_global<C>(cx: &mut C, global: Self)
    where
        C: BorrowAppContext,
    {
        cx.set_global(global)
    }
}

/// Unique id for each bin of global entities. Internally represented as "any value that is hashable and partial-eq",
/// where the hash value of TypeId is custom defined by gpui.
#[derive(Eq)]
pub(crate) struct GlobalId(Box<dyn collections::any::AnyHash>);
impl GlobalId {
    pub(crate) fn new(value: impl collections::any::AnyHash) -> Self {
        Self(Box::new(value))
    }
}
impl std::hash::Hash for GlobalId {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        match self.0.downcast_ref::<TypeId>() {
            Some(type_id) => {
                let mut hasher = gpui_util::TypeIdHashBuilder.build_hasher();
                type_id.hash(&mut hasher);
                hasher.finish().hash(state);
            }
            None => collections::any::AnyHash::hash(&self.0, state),
        }
    }
}
impl PartialEq for GlobalId {
    fn eq(&self, other: &Self) -> bool {
        collections::any::AnyHash::eq(&self.0, &other.0)
    }
}
