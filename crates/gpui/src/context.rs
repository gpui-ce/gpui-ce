//! Module containing various [`AppContext`] subdomain traits for common accessors of [`App`].
use crate::{
    AnyView, AnyWindowHandle, App, AsEntityId, Context, Entity, EntityId, EventEmitter, Global,
    GpuiBorrow, Reservation, Result, Window, WindowHandle,
    app_borrow::{BorrowApp, BorrowAppMut, UpgradeApp},
};
use scheduler::Task;
use std::ops::{Deref, DerefMut};

/// API trait used to unify multiple GPUI contexts under a single api.
pub trait AppContext:
    AppContextEntity + AppContextTask + AppContextWindow + AppContextGlobal
{
}

/// Exposes ways to borrow entities inline.
pub trait AppContextBorrowEntity {
    /// Update a entity in the app context.
    fn as_mut<'a, T>(&'a mut self, handle: &Entity<T>) -> GpuiBorrow<'a, T>
    where
        T: 'static;
}

/// Exposes various apis for interacting with [`Entity`] in the [`App`] that aren't exposed mutable borrow (see [`AppContextBorrowEntity`]).
pub trait AppContextEntity {
    /// Create a new entity in the app context.
    #[expect(
        clippy::wrong_self_convention,
        reason = "`App::new` is an ubiquitous function for creating entities"
    )]
    fn new<T: 'static>(&mut self, build_entity: impl FnOnce(&mut Context<T>) -> T) -> Entity<T>;

    /// Reserve a slot for a entity to be inserted later.
    /// The returned [Reservation] allows you to obtain the [EntityId] for the future entity.
    fn reserve_entity<T: 'static>(&mut self) -> Reservation<T>;

    /// Insert a new entity in the app context based on a [Reservation] previously obtained from [`reserve_entity`].
    ///
    /// [`reserve_entity`]: Self::reserve_entity
    fn insert_entity<T: 'static>(
        &mut self,
        reservation: Reservation<T>,
        build_entity: impl FnOnce(&mut Context<T>) -> T,
    ) -> Entity<T>;

    /// Update a entity in the app context.
    fn update_entity<T, R>(
        &mut self,
        handle: &Entity<T>,
        update: impl FnOnce(&mut T, &mut Context<T>) -> R,
    ) -> R
    where
        T: 'static;

    /// Read a entity from the app context.
    fn read_entity<T, R>(&self, handle: &Entity<T>, read: impl FnOnce(&T, &App) -> R) -> R
    where
        T: 'static;

    /// Tell GPUI that an entity has changed and observers of it should be notified.
    fn notify(&mut self, entity_id: impl AsEntityId);

    /// Emit an event of the specified type, which can be handled by other entities that have subscribed via `subscribe` methods on their respective contexts.
    /// A globally-callable equivalent to `Context::emit` without requiring an entity update.
    fn emit<EntityType, EventType>(&mut self, entity: &Entity<EntityType>, event: EventType)
    where
        EntityType: EventEmitter<EventType>,
        EventType: 'static;
}

/// Exposes apis for interfacing with [`Window`] data.
pub trait AppContextWindow {
    /// Update a window for the given handle.
    fn update_window<T, F>(&mut self, window: AnyWindowHandle, f: F) -> Result<T>
    where
        F: FnOnce(AnyView, &mut Window, &mut App) -> T;

    /// Run `f` against the entity's *current* window — the most recently
    /// rendered window that referenced the entity. Returns `None` if the
    /// entity has no current window or that window is unavailable. See
    /// [`App::with_window`] for the underlying lookup.
    fn with_window<R>(
        &mut self,
        entity_id: EntityId,
        f: impl FnOnce(&mut Window, &mut App) -> R,
    ) -> Option<R>;

    /// Read a window off of the application context.
    fn read_window<T, R>(
        &self,
        window: &WindowHandle<T>,
        read: impl FnOnce(Entity<T>, &App) -> R,
    ) -> Result<R>
    where
        T: 'static;
}

/// Exposes apis for spawning tasks.
pub trait AppContextTask {
    /// Spawn a future on a background thread
    fn background_spawn<R>(&self, future: impl Future<Output = R> + Send + 'static) -> Task<R>
    where
        R: Send + 'static;
}

/// Exposes apis for interfacing with globals (entities which are owned by the [`App`]).
pub trait AppContextGlobal {
    /// Read a global from this app context
    fn read_global<G, R>(&self, callback: impl FnOnce(&G, &App) -> R) -> R
    where
        G: Global;

    /// Stores an entity in the app such that it wont be dropped if no other entities are referencing it.
    /// Entities stored this way are dropped when the app is dropped, unless otherwise removed by [`remove_global_entity`].
    fn insert_global_entity<T: 'static>(&mut self, entity: Entity<T>);

    /// Removes the entity from global storage. If no other entities are holding reference to it,
    /// it will be dropped in the very near future.
    fn remove_global_entity<T: 'static>(&mut self, entity: &Entity<T>);

    /// Returns an iterator of all globally-stored entities which match the provided type.
    fn global_entities<T: 'static>(&self) -> impl Iterator<Item = Entity<T>>;
}

/// Marker trait which auto implements [`AppContextEntity`] if the marked struct also implements [`UpgradeApp`].
/// Implementations route through the borrow and deref to route calls directly to [`App`].
pub trait ImpliedAppEntity {}
impl<AppType> AppContextEntity for AppType
where
    AppType: ImpliedAppEntity + UpgradeApp,
{
    fn new<T: 'static>(&mut self, build_entity: impl FnOnce(&mut Context<T>) -> T) -> Entity<T> {
        let mut app = self.upgrade_mut();
        app.borrow_mut().deref_mut().new(build_entity)
    }

    fn reserve_entity<T: 'static>(&mut self) -> Reservation<T> {
        let mut app = self.upgrade_mut();
        app.borrow_mut().deref_mut().reserve_entity()
    }

    fn insert_entity<T: 'static>(
        &mut self,
        reservation: Reservation<T>,
        build_entity: impl FnOnce(&mut Context<T>) -> T,
    ) -> Entity<T> {
        let mut app = self.upgrade_mut();
        app.borrow_mut()
            .deref_mut()
            .insert_entity(reservation, build_entity)
    }

    fn update_entity<T: 'static, R>(
        &mut self,
        handle: &Entity<T>,
        update: impl FnOnce(&mut T, &mut Context<T>) -> R,
    ) -> R {
        let mut app = self.upgrade_mut();
        app.borrow_mut().deref_mut().update_entity(handle, update)
    }

    fn read_entity<T, R>(&self, handle: &Entity<T>, callback: impl FnOnce(&T, &App) -> R) -> R
    where
        T: 'static,
    {
        let app = self.upgrade();
        app.borrow().deref().read_entity(handle, callback)
    }

    fn notify(&mut self, entity_id: impl AsEntityId) {
        let mut app = self.upgrade_mut();
        app.borrow_mut().deref_mut().notify(entity_id);
    }

    fn emit<EntityType, EventType>(&mut self, entity: &Entity<EntityType>, event: EventType)
    where
        EntityType: EventEmitter<EventType>,
        EventType: 'static,
    {
        let mut app = self.upgrade_mut();
        app.borrow_mut().deref_mut().emit(entity, event)
    }
}

/// Marker trait which auto implements [`AppContextWindow`] if the marked struct also implements [`UpgradeApp`].
/// Implementations route through the borrow and deref to route calls directly to [`App`].
pub trait ImpliedAppWindow {}
impl<AppType> AppContextWindow for AppType
where
    AppType: ImpliedAppWindow + UpgradeApp,
{
    fn update_window<T, F>(&mut self, window: AnyWindowHandle, f: F) -> Result<T>
    where
        F: FnOnce(AnyView, &mut Window, &mut App) -> T,
    {
        // NOTE: Diverges from prev impl, which propagated the BorrowMutError to the caller
        let mut app = self.upgrade_mut();
        if app.borrow_mut().quitting {
            anyhow::bail!("app is quitting");
        }
        app.borrow_mut().update_window(window, f)
    }

    fn with_window<R>(
        &mut self,
        entity_id: EntityId,
        f: impl FnOnce(&mut Window, &mut App) -> R,
    ) -> Option<R> {
        // NOTE: Diverges from prev impl, which propagated the BorrowMutError to the caller
        let mut app = self.upgrade_mut();
        if app.borrow_mut().quitting {
            return None;
        }
        app.borrow_mut().with_window(entity_id, f)
    }

    fn read_window<T, R>(
        &self,
        window: &WindowHandle<T>,
        read: impl FnOnce(Entity<T>, &App) -> R,
    ) -> Result<R>
    where
        T: 'static,
    {
        let mut app = self.upgrade();
        if app.borrow().quitting {
            anyhow::bail!("app is quitting");
        }
        app.borrow().read_window(window, read)
    }
}

/// Marker trait which auto implements [`AppContextTask`] if the marked struct also implements [`UpgradeApp`].
/// Implementations route through the borrow and deref to route calls directly to [`App`].
pub trait ImpliedAppTask {}
impl<AppType> AppContextTask for AppType
where
    AppType: ImpliedAppTask + UpgradeApp,
{
    #[inline]
    fn background_spawn<R>(&self, future: impl Future<Output = R> + Send + 'static) -> Task<R>
    where
        R: Send + 'static,
    {
        let app = self.upgrade();
        app.borrow().background_spawn(future)
    }
}

/// Marker trait which auto implements [`AppContextGlobal`] if the marked struct also implements [`UpgradeApp`].
/// Implementations route through the borrow and deref to route calls directly to [`App`].
pub trait ImpliedAppGlobals {}
impl<AppType> AppContextGlobal for AppType
where
    AppType: ImpliedAppGlobals + UpgradeApp,
{
    fn read_global<G, R>(&self, callback: impl FnOnce(&G, &App) -> R) -> R
    where
        G: Global,
    {
        let mut app = self.upgrade();
        app.borrow().read_global(callback)
    }

    fn insert_global_entity<T: 'static>(&mut self, entity: Entity<T>) {
        let mut app = self.upgrade_mut();
        app.borrow_mut().insert_global_entity(entity)
    }

    fn remove_global_entity<T: 'static>(&mut self, entity: &Entity<T>) {
        let mut app = self.upgrade_mut();
        app.borrow_mut().remove_global_entity(entity)
    }

    fn global_entities<T: 'static>(&self) -> impl Iterator<Item = Entity<T>> {
        let app = self.upgrade();
        let borrow = app.borrow();
        let iter = borrow.global_entities();
        // Cloning the iterator here so that lock can be released when function is going out of scope
        iter.collect::<Vec<_>>().into_iter()
    }
}
