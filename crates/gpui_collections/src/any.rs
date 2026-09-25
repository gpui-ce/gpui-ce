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

        // Write the hash of the type
        hasher.write_u64(hash_type_id(&TypeId::of::<Self>()));

        // Write the value hash
        if let Some(type_id) = self.as_any().downcast_ref::<TypeId>() {
            hasher.write_u64(hash_type_id(type_id));
        } else {
            Hash::hash(&self, &mut hasher);
        }

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
        let boxed = Box::new(value);

        // If a caller provides a GenericId as the value, this should just return the existing value.
        // To do so, we have to do some magic to first box the GenericId to determine if its a GenericId.
        // Then we can emulate Box::downcast to unbox the newly boxed GenericId to return it downcasted.
        if boxed.as_any().is::<Self>() {
            // SAFETY: Check ensures the type is correct.
            // This is `Box<dyn Any>::downcast_unchecked`
            let boxed_id = unsafe {
                let (raw, alloc): (*mut dyn Any, _) = Box::into_raw_with_allocator(boxed);
                Box::from_raw_in(raw as *mut Self, alloc)
            };
            return *boxed_id;
        }

        Self(boxed)
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
        state.write_u64(self.0.as_hash())
    }
}
impl PartialEq for GenericId {
    fn eq(&self, other: &Self) -> bool {
        if self.0.as_any().type_id() != other.0.as_any().type_id() {
            return false;
        }
        self.0.eq_any(&*other.0)
    }
}
impl Eq for GenericId {}

impl std::fmt::Debug for GenericId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("GenericId(")?;
        self.0.as_any().type_id().fmt(f)?;
        f.write_str(": ")?;
        // NOTE: could use DebugTuple::field_with once https://github.com/rust-lang/rust/pull/146099 stabilizes in 1.101
        self.0.fmt_debug_any(f)?;
        f.write_str(")")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_type_ids() {
        let id_value = TypeId::of::<String>();
        let data = GenericId::new(id_value);
        assert_eq!(data.0.as_any().type_id(), id_value.type_id());
        assert_eq!(data.0.as_hash(), id_value.as_hash());
    }

    #[test]
    fn test_debug() {
        let id_value = String::from("this is some text");
        let type_id = TypeId::of::<String>();
        let data = GenericId::new(id_value.clone());
        assert_eq!(
            format!("{data:?}"),
            format!("GenericId({type_id:?}: {id_value:?})")
        );
    }

    #[test]
    fn test_string_types() {
        let value_str = "some str";
        let value_string = String::from("some string");
        // TODO: GlobalId from &str, String, SharedString does not map to the same hash value!
        assert_ne!(GenericId::new(value_str), GenericId::new(value_string));
    }

    #[test]
    fn test_new_recursion() {
        let id_value = 42u8;
        assert_eq!(
            GenericId::new(GenericId::new(id_value)),
            GenericId::new(id_value)
        );
    }
}
