//! Adapted from [any_key](https://github.com/Rufflewind/any_key), which is 9+ years out-of-date and contains a no-longer-compiling debugit dependency.
//! https://stackoverflow.com/questions/64838355/how-do-i-create-a-hashmap-with-type-erased-keys
use std::{
    any::{Any, TypeId},
    hash::{BuildHasher, Hash, Hasher},
};

// Performs a custom gpui_util hash for TypeId
fn hash_type_id(type_id: &TypeId) -> u64 {
    let mut hasher = gpui_util::TypeIdHashBuilder.build_hasher();
    type_id.hash(&mut hasher);
    hasher.finish()
}

pub trait AnyHash: Any {
    fn as_any(&self) -> &dyn Any;
    fn as_hash(&self) -> u64;
    fn eq_any(&self, other: &dyn AnyHash) -> bool;
    fn fmt_debug_any(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result;
}
impl<T> AnyHash for T
where
    T: Hash + PartialEq + std::fmt::Debug + 'static,
{
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_hash(&self) -> u64 {
        let mut hasher = std::hash::DefaultHasher::new();
        hasher.write_u64(hash_type_id(&TypeId::of::<Self>()));
        Hash::hash(&self, &mut hasher);
        hasher.finish()
    }

    fn eq_any(&self, other: &dyn AnyHash) -> bool {
        match other.as_any().downcast_ref::<Self>() {
            None => false,
            Some(other) => self.eq(other),
        }
    }

    fn fmt_debug_any(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.fmt(f)
    }
}

/// Unique id for each bin of global entities. Internally represented as "any value that is hashable and partial-eq",
/// where the hash value of TypeId is custom defined by gpui.
pub struct GenericId(Box<dyn AnyHash>);
impl GenericId {
    pub fn new(value: impl AnyHash) -> Self {
        Self(Box::new(value))
    }
}
impl std::ops::Deref for GenericId {
    type Target = dyn Any;

    fn deref(&self) -> &Self::Target {
        self.0.as_any()
    }
}
impl Hash for GenericId {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        match self.downcast_ref::<TypeId>() {
            Some(type_id) => state.write_u64(hash_type_id(type_id)),
            None => state.write_u64(self.0.as_hash()),
        }
    }
}
impl PartialEq for GenericId {
    fn eq(&self, other: &Self) -> bool {
        if self.type_id() != other.type_id() {
            return false;
        }
        self.0.eq_any(&*other.0)
    }
}
impl Eq for GenericId {}

impl std::fmt::Debug for GenericId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("GenericId(")?;
        self.type_id().fmt(f)?;
        f.write_str(": ")?;
        // NOTE: could use DebugTuple::field_with once https://github.com/rust-lang/rust/pull/146099 stabilizes in 1.101
        self.fmt_debug_any(f)?;
        f.write_str(")")?;
        Ok(())
    }
}
