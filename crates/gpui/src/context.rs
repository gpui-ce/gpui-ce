use crate::{
    AnyView, AnyWindowHandle, App, AppContextEntity, AppContextGlobal, AppContextTask,
    AppContextWindow, Context, Entity, EntityId, EventEmitter, Global, Reservation, Result, Window,
    WindowHandle,
    app_borrow::{BorrowApp, BorrowAppMut, UpgradeApp},
};
use scheduler::Task;
use std::ops::{Deref, DerefMut};

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

    fn notify(&mut self, entity_id: EntityId) {
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
