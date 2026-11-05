use crate::{App, AppContext, GpuiBorrow, VisualContext, Window, seal::Sealed};
use anyhow::{Context as _, Result};
use collections::FxHashSet;
use derive_more::{Deref, DerefMut};
use parking_lot::{RwLock, RwLockUpgradableReadGuard};
use slotmap::{KeyData, SecondaryMap, SlotMap};
use std::{
    any::{Any, TypeId, type_name},
    cell::RefCell,
    cmp::Ordering,
    fmt::{self, Display},
    hash::{Hash, Hasher},
    marker::PhantomData,
    num::NonZeroU64,
    sync::{
        Arc, Weak,
        atomic::{AtomicU64, AtomicUsize, Ordering::SeqCst},
    },
    thread::panicking,
};

use super::Context;
use crate::util::atomic_incr_if_not_zero;
#[cfg(any(test, feature = "leak-detection"))]
use collections::HashMap;

slotmap::new_key_type! {
    /// Identifies an entity within the app.
    ///
    /// Two entities never share an id, even after one has been released and its slot is reused. Get the
    /// id of a handle with [`Entity::entity_id`] or [`AnyEntity::entity_id`].
    pub struct EntityId;
}

impl From<u64> for EntityId {
    fn from(value: u64) -> Self {
        Self(KeyData::from_ffi(value))
    }
}

impl EntityId {
    /// Returns the id as a [`NonZeroU64`](std::num::NonZeroU64).
    ///
    /// # Panics
    ///
    /// Panics if the id's value is zero. GPUI never assigns that value, but an id built from `0` with
    /// `From<u64>` does have it.
    pub fn as_non_zero_u64(self) -> NonZeroU64 {
        NonZeroU64::new(self.0.as_ffi()).unwrap()
    }

    /// Returns the id as a `u64`. Converting the value back with `From<u64>` gives the same id.
    pub fn as_u64(self) -> u64 {
        self.0.as_ffi()
    }
}

impl Display for EntityId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_u64())
    }
}

pub(crate) struct EntityMap {
    entities: SecondaryMap<EntityId, Box<dyn Any>>,
    pub accessed_entities: RefCell<FxHashSet<EntityId>>,
    ref_counts: Arc<RwLock<EntityRefCounts>>,
}

#[doc(hidden)]
pub(crate) struct EntityRefCounts {
    counts: SlotMap<EntityId, AtomicUsize>,
    dropped_entity_ids: Vec<EntityId>,
    #[cfg(any(test, feature = "leak-detection"))]
    leak_detector: LeakDetector,
}

impl EntityMap {
    pub fn new() -> Self {
        Self {
            entities: SecondaryMap::new(),
            accessed_entities: RefCell::new(FxHashSet::default()),
            ref_counts: Arc::new(RwLock::new(EntityRefCounts {
                counts: SlotMap::with_key(),
                dropped_entity_ids: Vec::new(),
                #[cfg(any(test, feature = "leak-detection"))]
                leak_detector: LeakDetector {
                    next_handle_id: 0,
                    entity_handles: HashMap::default(),
                },
            })),
        }
    }

    #[doc(hidden)]
    pub fn ref_counts_drop_handle(&self) -> Arc<RwLock<EntityRefCounts>> {
        self.ref_counts.clone()
    }

    /// Captures a snapshot of all entities that currently have alive handles.
    ///
    /// The returned [`LeakDetectorSnapshot`] can later be passed to
    /// [`assert_no_new_leaks`](Self::assert_no_new_leaks) to verify that no
    /// entities created after the snapshot are still alive.
    #[cfg(any(test, feature = "leak-detection"))]
    pub fn leak_detector_snapshot(&self) -> LeakDetectorSnapshot {
        self.ref_counts.read().leak_detector.snapshot()
    }

    /// Asserts that no entities created after `snapshot` still have alive handles.
    ///
    /// See [`LeakDetector::assert_no_new_leaks`] for details.
    #[cfg(any(test, feature = "leak-detection"))]
    pub fn assert_no_new_leaks(&self, snapshot: &LeakDetectorSnapshot) {
        self.ref_counts
            .read()
            .leak_detector
            .assert_no_new_leaks(snapshot)
    }

    /// Reserve a slot for an entity, which you can subsequently use with `insert`.
    pub fn reserve<T: 'static>(&self) -> Slot<T> {
        let id = self.ref_counts.write().counts.insert(1.into());
        Slot(Entity::new(id, Arc::downgrade(&self.ref_counts)))
    }

    /// Insert an entity into a slot obtained by calling `reserve`.
    pub fn insert<T>(&mut self, slot: Slot<T>, entity: T) -> Entity<T>
    where
        T: 'static,
    {
        let mut accessed_entities = self.accessed_entities.get_mut();
        accessed_entities.insert(slot.entity_id);

        let handle = slot.0;
        self.entities.insert(handle.entity_id, Box::new(entity));
        handle
    }

    /// Move an entity to the stack.
    #[track_caller]
    pub fn lease<T>(&mut self, pointer: &Entity<T>) -> Lease<T> {
        self.assert_valid_context(pointer);
        let mut accessed_entities = self.accessed_entities.get_mut();
        accessed_entities.insert(pointer.entity_id);

        let entity = Some(
            self.entities
                .remove(pointer.entity_id)
                .unwrap_or_else(|| double_lease_panic::<T>("update")),
        );
        Lease {
            entity,
            id: pointer.entity_id,
            entity_type: PhantomData,
        }
    }

    /// Returns an entity after moving it to the stack.
    pub fn end_lease<T>(&mut self, mut lease: Lease<T>) {
        self.entities.insert(lease.id, lease.entity.take().unwrap());
    }

    pub fn read<T: 'static>(&self, entity: &Entity<T>) -> &T {
        self.assert_valid_context(entity);
        let mut accessed_entities = self.accessed_entities.borrow_mut();
        accessed_entities.insert(entity.entity_id);

        self.entities
            .get(entity.entity_id)
            .and_then(|entity| entity.downcast_ref())
            .unwrap_or_else(|| double_lease_panic::<T>("read"))
    }

    fn assert_valid_context(&self, entity: &AnyEntity) {
        debug_assert!(
            Weak::ptr_eq(&entity.entity_map, &Arc::downgrade(&self.ref_counts)),
            "used a entity with the wrong context"
        );
    }

    pub fn extend_accessed(&mut self, entities: &FxHashSet<EntityId>) {
        self.accessed_entities
            .get_mut()
            .extend(entities.iter().copied());
    }

    pub fn clear_accessed(&mut self) {
        self.accessed_entities.get_mut().clear();
    }

    pub fn take_dropped(&mut self) -> Vec<(EntityId, Box<dyn Any>)> {
        let mut ref_counts = &mut *self.ref_counts.write();
        let dropped_entity_ids = ref_counts.dropped_entity_ids.drain(..);
        let mut accessed_entities = self.accessed_entities.get_mut();

        dropped_entity_ids
            .filter_map(|entity_id| {
                let count = ref_counts.counts.remove(entity_id).unwrap();
                debug_assert_eq!(
                    count.load(SeqCst),
                    0,
                    "dropped an entity that was referenced"
                );
                accessed_entities.remove(&entity_id);
                // If the EntityId was allocated with `Context::reserve`,
                // the entity may not have been inserted.
                Some((entity_id, self.entities.remove(entity_id)?))
            })
            .collect()
    }
}

#[track_caller]
fn double_lease_panic<T>(operation: &str) -> ! {
    panic!(
        "cannot {operation} {} while it is already being updated",
        std::any::type_name::<T>()
    )
}

pub(crate) struct Lease<T> {
    entity: Option<Box<dyn Any>>,
    pub id: EntityId,
    entity_type: PhantomData<T>,
}

impl<T: 'static> core::ops::Deref for Lease<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.entity.as_ref().unwrap().downcast_ref().unwrap()
    }
}

impl<T: 'static> core::ops::DerefMut for Lease<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.entity.as_mut().unwrap().downcast_mut().unwrap()
    }
}

impl<T> Drop for Lease<T> {
    fn drop(&mut self) {
        if self.entity.is_some() && !panicking() {
            panic!("Leases must be ended with EntityMap::end_lease")
        }
    }
}

#[derive(Deref, DerefMut)]
pub(crate) struct Slot<T>(Entity<T>);

/// A handle to an entity whose type is erased.
///
/// Like [`Entity`], an `AnyEntity` keeps the entity alive. Use [`AnyEntity::downcast`] to get a typed
/// handle back.
///
/// # Examples
///
/// ```
/// # use gpui::{AppContext, TestApp};
/// # let mut app = TestApp::new();
/// # app.update(|cx| {
/// let number = cx.new(|_cx| 7_u32);
/// let any = number.into_any();
///
/// assert!(any.clone().downcast::<String>().is_err());
///
/// let number = any.downcast::<u32>().ok().unwrap();
/// assert_eq!(*number.read(cx), 7);
/// # });
/// ```
pub struct AnyEntity {
    pub(crate) entity_id: EntityId,
    pub(crate) entity_type: TypeId,
    entity_map: Weak<RwLock<EntityRefCounts>>,
    #[cfg(any(test, feature = "leak-detection"))]
    handle_id: HandleId,
}

impl AnyEntity {
    fn new(
        id: EntityId,
        entity_type: TypeId,
        entity_map: Weak<RwLock<EntityRefCounts>>,
        #[cfg(any(test, feature = "leak-detection"))] type_name: &'static str,
    ) -> Self {
        Self {
            entity_id: id,
            entity_type,
            #[cfg(any(test, feature = "leak-detection"))]
            handle_id: entity_map
                .clone()
                .upgrade()
                .unwrap()
                .write()
                .leak_detector
                .handle_created(id, Some(type_name)),
            entity_map,
        }
    }

    /// Returns the id of the entity.
    #[inline]
    pub fn entity_id(&self) -> EntityId {
        self.entity_id
    }

    /// Returns the [`TypeId`](std::any::TypeId) of the type the entity was created with.
    #[inline]
    pub fn entity_type(&self) -> TypeId {
        self.entity_type
    }

    /// Returns a type-erased weak handle to the same entity. It doesn't keep the entity alive.
    pub fn downgrade(&self) -> AnyWeakEntity {
        AnyWeakEntity {
            entity_id: self.entity_id,
            entity_type: self.entity_type,
            entity_ref_counts: self.entity_map.clone(),
        }
    }

    /// Returns a typed [`Entity<T>`] if `T` is the type of the entity.
    ///
    /// Otherwise returns this handle unchanged as the error, so you can try another type.
    pub fn downcast<T: 'static>(self) -> Result<Entity<T>, AnyEntity> {
        if TypeId::of::<T>() == self.entity_type {
            Ok(Entity {
                any_entity: self,
                entity_type: PhantomData,
            })
        } else {
            Err(self)
        }
    }
}

impl Clone for AnyEntity {
    fn clone(&self) -> Self {
        if let Some(entity_map) = self.entity_map.upgrade() {
            let entity_map = entity_map.read();
            let count = entity_map
                .counts
                .get(self.entity_id)
                .expect("detected over-release of a entity");
            let prev_count = count.fetch_add(1, SeqCst);
            assert_ne!(prev_count, 0, "Detected over-release of a entity.");
        }

        Self {
            entity_id: self.entity_id,
            entity_type: self.entity_type,
            entity_map: self.entity_map.clone(),
            #[cfg(any(test, feature = "leak-detection"))]
            handle_id: self
                .entity_map
                .upgrade()
                .unwrap()
                .write()
                .leak_detector
                .handle_created(self.entity_id, None),
        }
    }
}

impl Drop for AnyEntity {
    fn drop(&mut self) {
        if let Some(entity_map) = self.entity_map.upgrade() {
            let entity_map = entity_map.upgradable_read();
            let count = entity_map
                .counts
                .get(self.entity_id)
                .expect("detected over-release of a handle.");
            let prev_count = count.fetch_sub(1, SeqCst);
            assert_ne!(prev_count, 0, "Detected over-release of a entity.");
            if prev_count == 1 {
                // We were the last reference to this entity, so we can remove it.
                let mut entity_map = RwLockUpgradableReadGuard::upgrade(entity_map);
                entity_map.dropped_entity_ids.push(self.entity_id);
            }
        }

        #[cfg(any(test, feature = "leak-detection"))]
        if let Some(entity_map) = self.entity_map.upgrade() {
            entity_map
                .write()
                .leak_detector
                .handle_released(self.entity_id, self.handle_id)
        }
    }
}

impl<T> From<Entity<T>> for AnyEntity {
    #[inline]
    fn from(entity: Entity<T>) -> Self {
        entity.any_entity
    }
}

impl Hash for AnyEntity {
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.entity_id.hash(state);
    }
}

impl PartialEq for AnyEntity {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.entity_id == other.entity_id
    }
}

impl Eq for AnyEntity {}

impl Ord for AnyEntity {
    #[inline]
    fn cmp(&self, other: &Self) -> Ordering {
        self.entity_id.cmp(&other.entity_id)
    }
}

impl PartialOrd for AnyEntity {
    #[inline]
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl std::fmt::Debug for AnyEntity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AnyEntity")
            .field("entity_id", &self.entity_id.as_u64())
            .finish()
    }
}

/// A handle to state owned by the [`App`].
///
/// An `Entity<T>` keeps its value alive while any handle to it exists. Read the value with
/// [`Entity::read`], change it with [`Entity::update`], and call [`Entity::downgrade`] for a
/// [`WeakEntity`] that doesn't keep it alive.
///
/// Cloning is cheap. The clone is another handle to the same value, and two handles compare equal
/// when they refer to the same entity.
///
/// When the last strong handle is dropped, GPUI releases the value at the end of the current update.
/// See [ownership and data flow](crate::_documentation::concepts::ownership) for the model.
///
/// # Examples
///
/// ```
/// # use gpui::{AppContext, Entity, TestApp};
/// struct Counter {
///     count: usize,
/// }
///
/// # let mut app = TestApp::new();
/// # app.update(|cx| {
/// let counter: Entity<Counter> = cx.new(|_cx| Counter { count: 0 });
/// let same = counter.clone();
///
/// same.update(cx, |counter, _cx| counter.count += 1);
/// assert_eq!(counter.read(cx).count, 1);
/// # });
/// ```
#[derive(Deref, DerefMut)]
pub struct Entity<T> {
    #[deref]
    #[deref_mut]
    pub(crate) any_entity: AnyEntity,
    pub(crate) entity_type: PhantomData<fn(T) -> T>,
}

impl<T> Sealed for Entity<T> {}

impl<T: 'static> Entity<T> {
    #[inline]
    fn new(id: EntityId, entity_map: Weak<RwLock<EntityRefCounts>>) -> Self
    where
        T: 'static,
    {
        Self {
            any_entity: AnyEntity::new(
                id,
                TypeId::of::<T>(),
                entity_map,
                #[cfg(any(test, feature = "leak-detection"))]
                std::any::type_name::<T>(),
            ),
            entity_type: PhantomData,
        }
    }

    /// Returns the id of the entity.
    ///
    /// Two handles have the same id exactly when they refer to the same entity.
    #[inline]
    pub fn entity_id(&self) -> EntityId {
        self.any_entity.entity_id
    }

    /// Returns a [`WeakEntity`] to the same value.
    ///
    /// A weak handle doesn't keep the value alive. Use it for back-references, and for callbacks that may
    /// run after the entity is released.
    #[inline]
    pub fn downgrade(&self) -> WeakEntity<T> {
        WeakEntity {
            any_entity: self.any_entity.downgrade(),
            entity_type: self.entity_type,
        }
    }

    /// Converts this handle into an [`AnyEntity`]. The result keeps the value alive, without its type.
    #[inline]
    pub fn into_any(self) -> AnyEntity {
        self.any_entity
    }

    /// Returns a shared reference to the value, borrowed from the app.
    ///
    /// The reference borrows `cx`, so you can't update the app while you hold it. Copy out what you need,
    /// or work inside a single [`Entity::update`].
    ///
    /// # Panics
    ///
    /// Panics if this entity is currently being updated, for example when you read it from inside its own
    /// [`Entity::update`] closure. Use the `&mut T` that the closure received instead.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{AppContext, TestApp};
    /// # let mut app = TestApp::new();
    /// # app.update(|cx| {
    /// let name = cx.new(|_cx| String::from("GPUI"));
    ///
    /// let len = name.read(cx).len();
    /// name.update(cx, |name, _cx| name.push_str("!"));
    ///
    /// assert_eq!(len, 4);
    /// assert_eq!(name.read(cx), "GPUI!");
    /// # });
    /// ```
    #[inline]
    pub fn read<'a>(&self, cx: &'a App) -> &'a T {
        cx.entities.read(self)
    }

    /// Calls `read` with the value and the app, and returns what `read` returns.
    ///
    /// Use this when the callback needs the app as well as the value. It works with any [`AppContext`],
    /// including [`Context`] and [`AsyncApp`](crate::AsyncApp). It panics in the same cases as [`Entity::read`].
    #[inline]
    pub fn read_with<R, C: AppContext>(&self, cx: &C, f: impl FnOnce(&T, &App) -> R) -> R {
        cx.read_entity(self, f)
    }

    /// Changes the value, and returns what `update` returns.
    ///
    /// `update` receives a `&mut T` and a [`Context<T>`] for this entity. Use it for every change to
    /// an entity. Call [`Context::notify`] inside the closure when anything that depends on the value has
    /// changed. Effects queued by the closure, such as notifications and events, run when the outermost
    /// update finishes.
    ///
    /// Pass any [`AppContext`] as `cx`: `&mut App`, a [`Context`] of another entity, or an [`AsyncApp`](crate::AsyncApp)
    /// inside a task.
    ///
    /// # Panics
    ///
    /// Panics if this entity is already being updated. Updating it from inside its own closure is the
    /// usual cause; use the `&mut T` you were given instead. Updating other entities is fine.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{AppContext, TestApp};
    /// struct Counter {
    ///     count: usize,
    /// }
    ///
    /// # let mut app = TestApp::new();
    /// # app.update(|cx| {
    /// let counter = cx.new(|_cx| Counter { count: 0 });
    ///
    /// let count = counter.update(cx, |counter, cx| {
    ///     counter.count += 1;
    ///     cx.notify();
    ///     counter.count
    /// });
    ///
    /// assert_eq!(count, 1);
    /// # });
    /// ```
    #[inline]
    pub fn update<R, C: AppContext>(
        &self,
        cx: &mut C,
        update: impl FnOnce(&mut T, &mut Context<T>) -> R,
    ) -> R {
        cx.update_entity(self, update)
    }

    /// Returns a guard that dereferences to the value, so you can change it with `*` or with methods.
    ///
    /// The guard borrows `cx` for as long as it lives. When the guard is dropped, the entity is notified,
    /// as if you had called [`Context::notify`]. Use [`Entity::update`] when the change needs a
    /// [`Context`].
    #[inline]
    pub fn as_mut<'a, C: AppContext>(&self, cx: &'a mut C) -> GpuiBorrow<'a, T> {
        cx.as_mut(self)
    }

    /// Replaces the value with `value`, and notifies observers.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{AppContext, TestApp};
    /// # let mut app = TestApp::new();
    /// # app.update(|cx| {
    /// let name = cx.new(|_cx| String::from("draft"));
    /// name.write(cx, String::from("published"));
    /// assert_eq!(name.read(cx), "published");
    /// # });
    /// ```
    pub fn write<C: AppContext>(&self, cx: &mut C, value: T) {
        self.update(cx, |entity, cx| {
            *entity = value;
            cx.notify();
        })
    }

    /// Like [`Entity::update`], but also passes the [`Window`] that the entity is shown in.
    ///
    /// Use it from a [`VisualContext`], which is a context that has a window. Returns an error if the
    /// window has been closed.
    #[inline]
    pub fn update_in<R, C: VisualContext>(
        &self,
        cx: &mut C,
        update: impl FnOnce(&mut T, &mut Window, &mut Context<T>) -> R,
    ) -> C::Result<R> {
        cx.update_window_entity(self, update)
    }
}

impl<T> Clone for Entity<T> {
    #[inline]
    fn clone(&self) -> Self {
        Self {
            any_entity: self.any_entity.clone(),
            entity_type: self.entity_type,
        }
    }
}

impl<T> std::fmt::Debug for Entity<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Entity")
            .field("entity_id", &self.any_entity.entity_id)
            .field("entity_type", &type_name::<T>())
            .finish()
    }
}

impl<T> Hash for Entity<T> {
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.any_entity.hash(state);
    }
}

impl<T> PartialEq for Entity<T> {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.any_entity == other.any_entity
    }
}

impl<T> Eq for Entity<T> {}

impl<T> PartialEq<WeakEntity<T>> for Entity<T> {
    #[inline]
    fn eq(&self, other: &WeakEntity<T>) -> bool {
        self.any_entity.entity_id() == other.entity_id()
    }
}

impl<T: 'static> Ord for Entity<T> {
    #[inline]
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.entity_id().cmp(&other.entity_id())
    }
}

impl<T: 'static> PartialOrd for Entity<T> {
    #[inline]
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// A type-erased handle to an entity that doesn't keep it alive.
///
/// Upgrade it with [`AnyWeakEntity::upgrade`] to get a strong [`AnyEntity`] while the entity exists.
#[derive(Clone)]
pub struct AnyWeakEntity {
    pub(crate) entity_id: EntityId,
    entity_type: TypeId,
    entity_ref_counts: Weak<RwLock<EntityRefCounts>>,
}

impl AnyWeakEntity {
    /// Returns the id of the entity this handle refers to.
    #[inline]
    pub fn entity_id(&self) -> EntityId {
        self.entity_id
    }

    /// Returns whether [`AnyWeakEntity::upgrade`] would succeed now.
    ///
    /// Returns `false` once the last strong handle has been dropped, even before GPUI has released the
    /// entity.
    pub fn is_upgradable(&self) -> bool {
        let ref_count = self
            .entity_ref_counts
            .upgrade()
            .and_then(|ref_counts| Some(ref_counts.read().counts.get(self.entity_id)?.load(SeqCst)))
            .unwrap_or(0);
        ref_count > 0
    }

    /// Returns a strong handle to the entity, or `None` if it has been released.
    ///
    /// An entity whose last strong handle was dropped in the current update already returns `None`.
    pub fn upgrade(&self) -> Option<AnyEntity> {
        let ref_counts = &self.entity_ref_counts.upgrade()?;
        let ref_counts = ref_counts.read();
        let ref_count = ref_counts.counts.get(self.entity_id)?;

        if atomic_incr_if_not_zero(ref_count) == 0 {
            // entity_id is in dropped_entity_ids
            return None;
        }
        drop(ref_counts);

        Some(AnyEntity {
            entity_id: self.entity_id,
            entity_type: self.entity_type,
            entity_map: self.entity_ref_counts.clone(),
            #[cfg(any(test, feature = "leak-detection"))]
            handle_id: self
                .entity_ref_counts
                .upgrade()
                .unwrap()
                .write()
                .leak_detector
                .handle_created(self.entity_id, None),
        })
    }

    /// Asserts that the entity this handle refers to has been released.
    ///
    /// Use this in tests to find a strong handle that was leaked. A task or a subscription that still holds
    /// a strong handle makes the check fail. The check needs the leak detector, which is enabled in tests
    /// and with the `leak-detection` feature.
    ///
    /// GPUI releases an entity when the update that dropped its last handle finishes. Run an update before
    /// you call this, as the example does.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{AppContext, TestApp};
    /// # struct Document;
    /// # let mut app = TestApp::new();
    /// let document = app.update(|cx| cx.new(|_cx| Document));
    /// let weak = document.downgrade();
    /// drop(document);
    ///
    /// // The update finishes, and GPUI releases the entity.
    /// app.update(|_cx| {});
    /// weak.assert_released();
    /// ```
    ///
    /// # Debugging Leaks
    ///
    /// If this method panics because of a leaked handle, set the `LEAK_BACKTRACE` environment variable to
    /// see where the handle was created:
    ///
    /// ```bash
    /// LEAK_BACKTRACE=1 cargo test my_test
    /// ```
    ///
    /// # Panics
    ///
    /// - Panics if any strong handles to the entity are still alive.
    /// - Panics if the entity was dropped in the current update. Its resources are kept until that update
    ///   finishes.
    #[cfg(any(test, feature = "leak-detection"))]
    pub fn assert_released(&self) {
        self.entity_ref_counts
            .upgrade()
            .unwrap()
            .write()
            .leak_detector
            .assert_released(self.entity_id);

        if self
            .entity_ref_counts
            .upgrade()
            .and_then(|ref_counts| Some(ref_counts.read().counts.get(self.entity_id)?.load(SeqCst)))
            .is_some()
        {
            panic!(
                "entity was recently dropped but resources are retained until the end of the effect cycle."
            )
        }
    }

    /// Returns a weak handle that never upgrades. Use it as a placeholder until you have a real entity.
    pub fn new_invalid() -> Self {
        /// To hold the invariant that all ids are unique, and considering that slotmap
        /// increases their IDs from `0`, we can decrease ours from `u64::MAX` so these
        /// two will never conflict (u64 is way too large).
        static UNIQUE_NON_CONFLICTING_ID_GENERATOR: AtomicU64 = AtomicU64::new(u64::MAX);
        let entity_id = UNIQUE_NON_CONFLICTING_ID_GENERATOR.fetch_sub(1, SeqCst);

        Self {
            // Safety:
            //   Docs say this is safe but can be unspecified if slotmap changes the representation
            //   after `1.0.7`, that said, providing a valid entity_id here is not necessary as long
            //   as we guarantee that `entity_id` is never used if `entity_ref_counts` equals
            //   to `Weak::new()` (that is, it's unable to upgrade), that is the invariant that
            //   actually needs to be hold true.
            //
            //   And there is no sane reason to read an entity slot if `entity_ref_counts` can't be
            //   read in the first place, so we're good!
            entity_id: entity_id.into(),
            entity_type: TypeId::of::<()>(),
            entity_ref_counts: Weak::new(),
        }
    }
}

impl std::fmt::Debug for AnyWeakEntity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct(type_name::<Self>())
            .field("entity_id", &self.entity_id)
            .field("entity_type", &self.entity_type)
            .finish()
    }
}

impl<T> From<WeakEntity<T>> for AnyWeakEntity {
    #[inline]
    fn from(entity: WeakEntity<T>) -> Self {
        entity.any_entity
    }
}

impl Hash for AnyWeakEntity {
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.entity_id.hash(state);
    }
}

impl PartialEq for AnyWeakEntity {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.entity_id == other.entity_id
    }
}

impl Eq for AnyWeakEntity {}

impl Ord for AnyWeakEntity {
    #[inline]
    fn cmp(&self, other: &Self) -> Ordering {
        self.entity_id.cmp(&other.entity_id)
    }
}

impl PartialOrd for AnyWeakEntity {
    #[inline]
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// A handle to an entity that doesn't keep it alive.
///
/// Use a `WeakEntity` for back-references, such as from a child view to its parent, and for callbacks
/// that may run after the entity is released. Its [`update`](WeakEntity::update) and
/// [`read_with`](WeakEntity::read_with) methods return a `Result`, which is an error once the entity
/// has been released.
#[derive(Deref, DerefMut)]
pub struct WeakEntity<T> {
    #[deref]
    #[deref_mut]
    any_entity: AnyWeakEntity,
    entity_type: PhantomData<fn(T) -> T>,
}

impl<T> std::fmt::Debug for WeakEntity<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct(type_name::<Self>())
            .field("entity_id", &self.any_entity.entity_id)
            .field("entity_type", &type_name::<T>())
            .finish()
    }
}

impl<T> Clone for WeakEntity<T> {
    fn clone(&self) -> Self {
        Self {
            any_entity: self.any_entity.clone(),
            entity_type: self.entity_type,
        }
    }
}

impl<T: 'static> WeakEntity<T> {
    /// Returns a strong handle to the entity, or `None` if it has been released.
    pub fn upgrade(&self) -> Option<Entity<T>> {
        Some(Entity {
            any_entity: self.any_entity.upgrade()?,
            entity_type: self.entity_type,
        })
    }

    /// Changes the entity if it still exists. Returns an error if it has been released.
    ///
    /// This is [`Entity::update`] for a handle that may be stale. Use it in callbacks and tasks, where the
    /// entity may have been released before the callback runs.
    ///
    /// # Panics
    ///
    /// Panics if the entity is already being updated, in the same cases as [`Entity::update`]. A weak
    /// handle still upgrades during an update of its own entity, so it doesn't report that case as an
    /// error.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{AppContext, TestApp};
    /// struct Counter {
    ///     count: usize,
    /// }
    ///
    /// # let mut app = TestApp::new();
    /// # let weak = app.update(|cx| {
    /// let counter = cx.new(|_cx| Counter { count: 0 });
    /// let weak = counter.downgrade();
    /// assert!(weak.update(cx, |counter, _cx| counter.count += 1).is_ok());
    ///
    /// drop(counter);
    /// # weak
    /// # });
    /// // The update that dropped the last strong handle has finished, so the counter is gone.
    /// # app.update(|cx| {
    /// assert!(weak.update(cx, |counter, _cx| counter.count += 1).is_err());
    /// # });
    /// ```
    pub fn update<C, R>(
        &self,
        cx: &mut C,
        update: impl FnOnce(&mut T, &mut Context<T>) -> R,
    ) -> Result<R>
    where
        C: AppContext,
    {
        let entity = self.upgrade().context("entity released")?;
        Ok(cx.update_entity(&entity, update))
    }

    /// Like [`WeakEntity::update`], but also passes the [`Window`] that the entity is shown in.
    ///
    /// Returns an error if the entity has been released, or if it isn't shown in a window that is
    /// currently available.
    pub fn update_in<C, R>(
        &self,
        cx: &mut C,
        update: impl FnOnce(&mut T, &mut Window, &mut Context<T>) -> R,
    ) -> Result<R>
    where
        C: AppContext,
    {
        let entity = self.upgrade().context("entity released")?;
        cx.with_window(entity.entity_id(), |window, app| {
            entity.update(app, |entity, cx| update(entity, window, cx))
        })
        .context("entity has no current window")
    }

    /// Calls `read` with the entity if it still exists. Returns an error if it has been released.
    ///
    /// Use this when you only need to read the value. It works with any [`AppContext`].
    pub fn read_with<C, R>(&self, cx: &C, read: impl FnOnce(&T, &App) -> R) -> Result<R>
    where
        C: AppContext,
    {
        let entity = self.upgrade().context("entity released")?;
        Ok(cx.read_entity(&entity, read))
    }

    /// Returns a weak handle that never upgrades. Use it as a placeholder until you have a real entity.
    #[inline]
    pub fn new_invalid() -> Self {
        Self {
            any_entity: AnyWeakEntity::new_invalid(),
            entity_type: PhantomData,
        }
    }
}

impl<T> Hash for WeakEntity<T> {
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.any_entity.hash(state);
    }
}

impl<T> PartialEq for WeakEntity<T> {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.any_entity == other.any_entity
    }
}

impl<T> Eq for WeakEntity<T> {}

impl<T> PartialEq<Entity<T>> for WeakEntity<T> {
    #[inline]
    fn eq(&self, other: &Entity<T>) -> bool {
        self.entity_id() == other.any_entity.entity_id()
    }
}

impl<T: 'static> Ord for WeakEntity<T> {
    #[inline]
    fn cmp(&self, other: &Self) -> Ordering {
        self.entity_id().cmp(&other.entity_id())
    }
}

impl<T: 'static> PartialOrd for WeakEntity<T> {
    #[inline]
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Controls whether backtraces are captured when entity handles are created.
///
/// Set the `LEAK_BACKTRACE` environment variable to any non-empty value to enable
/// backtrace capture. This helps identify where leaked handles were allocated.
#[cfg(any(test, feature = "leak-detection"))]
static LEAK_BACKTRACE: std::sync::LazyLock<bool> =
    std::sync::LazyLock::new(|| std::env::var("LEAK_BACKTRACE").is_ok_and(|b| !b.is_empty()));

/// Unique identifier for a specific entity handle instance.
///
/// This is distinct from `EntityId` - while multiple handles can point to the same
/// entity (same `EntityId`), each handle has its own unique `HandleId`.
#[cfg(any(test, feature = "leak-detection"))]
#[derive(Clone, Copy, Debug, Default, Hash, PartialEq, Eq)]
pub(crate) struct HandleId {
    id: u64,
}

/// Tracks entity handle allocations to detect leaks.
///
/// The leak detector is enabled in tests and when the `leak-detection` feature is active. It records
/// every `Entity<T>` and `AnyEntity` handle that is created and released, so that you can check that
/// all handles to an entity have been dropped.
///
/// # How do leaks happen?
///
/// Entities are reference-counted structures that can own other entities
/// allowing to form cycles. If such a strong-reference counted cycle is
/// created, all participating strong entities in this cycle will effectively
/// leak as they cannot be released anymore.
///
/// Cycles can also happen if an entity owns a task or subscription that it
/// itself owns a strong reference to the entity again.
///
/// # Usage
///
/// Call [`AnyWeakEntity::assert_released`] to check that an entity has been released. Take a
/// snapshot with [`App::leak_detector_snapshot`], and pass it to [`App::assert_no_new_leaks`] to find
/// entities created since the snapshot that still have handles.
///
/// # Debugging Leaks
///
/// When a leak is detected, the detector will panic with information about the leaked
/// handles. To see where the leaked handles were allocated, set the `LEAK_BACKTRACE`
/// environment variable:
///
/// ```bash
/// LEAK_BACKTRACE=1 cargo test my_test
/// ```
///
/// This will capture and display backtraces for each leaked handle, helping you
/// identify where leaked handles were created.
///
/// # How It Works
///
/// - When an entity handle is created (via `Entity::new`, `Entity::clone`, or
///   `WeakEntity::upgrade`), `handle_created` is called to register the handle.
/// - When a handle is dropped, `handle_released` removes it from tracking.
/// - `assert_released` verifies that no handles remain for a given entity.
#[cfg(any(test, feature = "leak-detection"))]
pub(crate) struct LeakDetector {
    next_handle_id: u64,
    entity_handles: HashMap<EntityId, EntityLeakData>,
}

/// A snapshot of the entities that had alive handles at one point in time.
///
/// Take one with [`App::leak_detector_snapshot`], and pass it to [`App::assert_no_new_leaks`]. The
/// check ignores entities that were already alive when the snapshot was taken.
#[cfg(any(test, feature = "leak-detection"))]
pub struct LeakDetectorSnapshot {
    entity_ids: collections::HashSet<EntityId>,
}

#[cfg(any(test, feature = "leak-detection"))]
struct EntityLeakData {
    handles: HashMap<HandleId, Option<backtrace::Backtrace>>,
    type_name: &'static str,
}

#[cfg(any(test, feature = "leak-detection"))]
impl LeakDetector {
    /// Records that a new handle has been created for the given entity.
    ///
    /// Returns a unique `HandleId` that must be passed to `handle_released` when
    /// the handle is dropped. If `LEAK_BACKTRACE` is set, captures a backtrace
    /// at the allocation site.
    #[track_caller]
    pub fn handle_created(
        &mut self,
        entity_id: EntityId,
        type_name: Option<&'static str>,
    ) -> HandleId {
        let id = gpui_util::post_inc(&mut self.next_handle_id);
        let handle_id = HandleId { id };
        let handles = self
            .entity_handles
            .entry(entity_id)
            .or_insert_with(|| EntityLeakData {
                handles: HashMap::default(),
                type_name: type_name.unwrap_or("<unknown>"),
            });
        handles.handles.insert(
            handle_id,
            LEAK_BACKTRACE.then(backtrace::Backtrace::new_unresolved),
        );
        handle_id
    }

    /// Records that a handle has been released (dropped).
    ///
    /// This removes the handle from tracking. The `handle_id` should be the same
    /// one returned by `handle_created` when the handle was allocated.
    pub fn handle_released(&mut self, entity_id: EntityId, handle_id: HandleId) {
        if let std::collections::hash_map::Entry::Occupied(mut data) =
            self.entity_handles.entry(entity_id)
        {
            data.get_mut().handles.remove(&handle_id);
            if data.get().handles.is_empty() {
                data.remove();
            }
        }
    }

    /// Asserts that all handles to the given entity have been released.
    ///
    /// # Panics
    ///
    /// Panics if any handles to the entity are still alive. The panic message
    /// includes backtraces for each leaked handle if `LEAK_BACKTRACE` is set,
    /// otherwise it suggests setting the environment variable to get more info.
    pub fn assert_released(&mut self, entity_id: EntityId) {
        use std::fmt::Write as _;

        if let Some(data) = self.entity_handles.remove(&entity_id) {
            let mut out = String::new();
            for (_, backtrace) in data.handles {
                if let Some(mut backtrace) = backtrace {
                    backtrace.resolve();
                    let backtrace = BacktraceFormatter(backtrace);
                    writeln!(out, "Leaked handle:\n{:?}", backtrace).unwrap();
                } else {
                    writeln!(
                        out,
                        "Leaked handle: (export LEAK_BACKTRACE to find allocation site)"
                    )
                    .unwrap();
                }
            }
            panic!("Handles for {} leaked:\n{out}", data.type_name);
        }
    }

    /// Captures a snapshot of all entity IDs that currently have alive handles.
    ///
    /// The returned [`LeakDetectorSnapshot`] can later be passed to
    /// [`assert_no_new_leaks`](Self::assert_no_new_leaks) to verify that no
    /// entities created after the snapshot are still alive.
    pub fn snapshot(&self) -> LeakDetectorSnapshot {
        LeakDetectorSnapshot {
            entity_ids: self.entity_handles.keys().copied().collect(),
        }
    }

    /// Asserts that no entities created after `snapshot` still have alive handles.
    ///
    /// Entities that were already tracked at the time of the snapshot are ignored,
    /// even if they still have handles. Only *new* entities (those whose
    /// `EntityId` was not present in the snapshot) are considered leaks.
    ///
    /// # Panics
    ///
    /// Panics if any new entity handles exist. The panic message lists every
    /// leaked entity with its type name, and includes allocation-site backtraces
    /// when `LEAK_BACKTRACE` is set.
    pub fn assert_no_new_leaks(&self, snapshot: &LeakDetectorSnapshot) {
        use std::fmt::Write as _;

        let mut out = String::new();
        for (entity_id, data) in &self.entity_handles {
            if snapshot.entity_ids.contains(entity_id) {
                continue;
            }
            for (_, backtrace) in &data.handles {
                if let Some(backtrace) = backtrace {
                    let mut backtrace = backtrace.clone();
                    backtrace.resolve();
                    let backtrace = BacktraceFormatter(backtrace);
                    writeln!(
                        out,
                        "Leaked handle for entity {} ({entity_id:?}):\n{:?}",
                        data.type_name, backtrace
                    )
                    .unwrap();
                } else {
                    writeln!(
                        out,
                        "Leaked handle for entity {} ({entity_id:?}): (export LEAK_BACKTRACE to find allocation site)",
                        data.type_name
                    )
                    .unwrap();
                }
            }
        }

        if !out.is_empty() {
            panic!("New entity leaks detected since snapshot:\n{out}");
        }
    }
}

#[cfg(any(test, feature = "leak-detection"))]
impl Drop for LeakDetector {
    fn drop(&mut self) {
        use std::fmt::Write;

        if self.entity_handles.is_empty() || std::thread::panicking() {
            return;
        }

        let mut out = String::new();
        for (entity_id, data) in self.entity_handles.drain() {
            for (_handle, backtrace) in data.handles {
                if let Some(mut backtrace) = backtrace {
                    backtrace.resolve();
                    let backtrace = BacktraceFormatter(backtrace);
                    writeln!(
                        out,
                        "Leaked handle for entity {} ({entity_id:?}):\n{:?}",
                        data.type_name, backtrace
                    )
                    .unwrap();
                } else {
                    writeln!(
                        out,
                        "Leaked handle for entity {} ({entity_id:?}): (export LEAK_BACKTRACE to find allocation site)",
                        data.type_name
                    )
                    .unwrap();
                }
            }
        }
        panic!("Exited with leaked handles:\n{out}");
    }
}

#[cfg(any(test, feature = "leak-detection"))]
struct BacktraceFormatter(backtrace::Backtrace);

#[cfg(any(test, feature = "leak-detection"))]
impl fmt::Debug for BacktraceFormatter {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        use backtrace::{BacktraceFmt, BytesOrWideString, PrintFmt};

        let style = if fmt.alternate() {
            PrintFmt::Full
        } else {
            PrintFmt::Short
        };

        // When printing paths we try to strip the cwd if it exists, otherwise
        // we just print the path as-is. Note that we also only do this for the
        // short format, because if it's full we presumably want to print
        // everything.
        let cwd = std::env::current_dir();
        let mut print_path = move |fmt: &mut fmt::Formatter<'_>, path: BytesOrWideString<'_>| {
            let path = path.into_path_buf();
            if style != PrintFmt::Full {
                if let Ok(cwd) = &cwd {
                    if let Ok(suffix) = path.strip_prefix(cwd) {
                        return fmt::Display::fmt(&suffix.display(), fmt);
                    }
                }
            }
            fmt::Display::fmt(&path.display(), fmt)
        };

        let mut f = BacktraceFmt::new(fmt, style, &mut print_path);
        f.add_context()?;
        let mut strip = true;
        for frame in self.0.frames() {
            if let [symbol, ..] = frame.symbols()
                && let Some(name) = symbol.name()
                && let Some(filename) = name.as_str()
            {
                match filename {
                    "test::run_test_in_process"
                    | "scheduler::executor::spawn_local_with_source_location::impl$1::poll<core::pin::Pin<alloc::boxed::Box<dyn$<core::future::future::Future<assoc$<Output,enum2$<core::result::Result<workspace::OpenResult,anyhow::Error> > > > >,alloc::alloc::Global> > >" => {
                        strip = true
                    }
                    "gpui::app::entity_map::LeakDetector::handle_created" => {
                        strip = false;
                        continue;
                    }
                    "zed::main" => {
                        strip = true;
                        f.frame().backtrace_frame(frame)?;
                    }
                    _ => {}
                }
            }
            if strip {
                continue;
            }
            f.frame().backtrace_frame(frame)?;
        }
        f.finish()?;
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use crate::EntityMap;

    struct TestEntity {
        pub i: i32,
    }

    #[test]
    fn test_entity_map_slot_assignment_before_cleanup() {
        // Tests that slots are not re-used before take_dropped.
        let mut entity_map = EntityMap::new();

        let slot = entity_map.reserve::<TestEntity>();
        entity_map.insert(slot, TestEntity { i: 1 });

        let slot = entity_map.reserve::<TestEntity>();
        entity_map.insert(slot, TestEntity { i: 2 });

        let dropped = entity_map.take_dropped();
        assert_eq!(dropped.len(), 2);

        assert_eq!(
            dropped
                .into_iter()
                .map(|(_, entity)| entity.downcast::<TestEntity>().unwrap().i)
                .collect::<Vec<i32>>(),
            vec![1, 2],
        );
    }

    #[test]
    fn test_entity_map_weak_upgrade_before_cleanup() {
        // Tests that weak handles are not upgraded before take_dropped
        let mut entity_map = EntityMap::new();

        let slot = entity_map.reserve::<TestEntity>();
        let handle = entity_map.insert(slot, TestEntity { i: 1 });
        let weak = handle.downgrade();
        drop(handle);

        let strong = weak.upgrade();
        assert_eq!(strong, None);

        let dropped = entity_map.take_dropped();
        assert_eq!(dropped.len(), 1);

        assert_eq!(
            dropped
                .into_iter()
                .map(|(_, entity)| entity.downcast::<TestEntity>().unwrap().i)
                .collect::<Vec<i32>>(),
            vec![1],
        );
    }

    #[test]
    fn test_leak_detector_snapshot_no_leaks() {
        let mut entity_map = EntityMap::new();

        let slot = entity_map.reserve::<TestEntity>();
        let pre_existing = entity_map.insert(slot, TestEntity { i: 1 });

        let snapshot = entity_map.leak_detector_snapshot();

        let slot = entity_map.reserve::<TestEntity>();
        let temporary = entity_map.insert(slot, TestEntity { i: 2 });
        drop(temporary);

        entity_map.assert_no_new_leaks(&snapshot);

        drop(pre_existing);
    }

    #[test]
    #[should_panic(expected = "New entity leaks detected since snapshot")]
    fn test_leak_detector_snapshot_detects_new_leak() {
        let mut entity_map = EntityMap::new();

        let slot = entity_map.reserve::<TestEntity>();
        let pre_existing = entity_map.insert(slot, TestEntity { i: 1 });

        let snapshot = entity_map.leak_detector_snapshot();

        let slot = entity_map.reserve::<TestEntity>();
        let leaked = entity_map.insert(slot, TestEntity { i: 2 });

        // `leaked` is still alive, so this should panic.
        entity_map.assert_no_new_leaks(&snapshot);

        drop(pre_existing);
        drop(leaked);
    }
}
