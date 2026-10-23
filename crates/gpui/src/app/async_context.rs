use crate::{
    AnyView, AnyWindowHandle, App, AppCell, AppContext, BackgroundExecutor, BorrowAppContext,
    Entity, EntityId, EventEmitter, Focusable, ForegroundExecutor, Global, GpuiBorrow,
    PromptButton, PromptLevel, Render, Reservation, Result, Subscription, Task, VisualContext,
    Window, WindowHandle,
};
use anyhow::{Context as _, bail};
use derive_more::{Deref, DerefMut};
use futures::channel::oneshot;
use futures::future::FutureExt;
use std::{future::Future, rc::Weak};

use super::{Context, WeakEntity};

/// A handle to the app that you can hold across `.await` points.
///
/// [`App`] can't be borrowed across an `.await`, because the rest of the application
/// needs it while your future waits. `AsyncApp` is the alternative: you get one as
/// the argument of a future started with [`App::spawn`], or from [`App::to_async`].
/// Each method borrows the app for the length of one call, runs a complete update,
/// and releases the borrow. Entity methods are on the [`AppContext`] trait, which
/// `AsyncApp` implements, so helpers written against that trait work here too.
///
/// `AsyncApp` holds a weak reference to the app, so it doesn't keep the app alive.
/// Most methods panic if the app has been released. The `AppContext` methods that return a
/// `Result` or `Option`, such as `update_window` and `with_window`, report it as an error or
/// `None` instead. Keep an `AsyncApp` only inside the futures that use it. One stored
/// somewhere else can outlive the app.
///
/// See [contexts](crate::_documentation::concepts::contexts#async-contexts) for how
/// async contexts fit with the other contexts.
///
/// # Examples
///
/// ```
/// use gpui::{AppContext, TestApp};
///
/// let mut app = TestApp::new();
/// let count = app.update(|cx| cx.new(|_| 0_usize));
///
/// let task = app.to_async().spawn({
///     let count = count.clone();
///     async move |cx| {
///         cx.update(|cx| {
///             count.update(cx, |count, cx| {
///                 *count += 1;
///                 cx.notify();
///             })
///         });
///     }
/// });
/// task.detach();
///
/// app.run_until_parked();
/// app.read(|cx| assert_eq!(*count.read(cx), 1));
/// ```
#[derive(Clone)]
pub struct AsyncApp {
    pub(crate) app: Weak<AppCell>,
    pub(crate) background_executor: BackgroundExecutor,
    pub(crate) foreground_executor: ForegroundExecutor,
}

impl AsyncApp {
    fn app(&self) -> std::rc::Rc<AppCell> {
        self.app
            .upgrade()
            .expect("app was released before async operation completed")
    }
}

impl AppContext for AsyncApp {
    fn new<T: 'static>(&mut self, build_entity: impl FnOnce(&mut Context<T>) -> T) -> Entity<T> {
        let app = self.app();
        let mut app = app.borrow_mut();
        app.new(build_entity)
    }

    fn reserve_entity<T: 'static>(&mut self) -> Reservation<T> {
        let app = self.app();
        let mut app = app.borrow_mut();
        app.reserve_entity()
    }

    fn insert_entity<T: 'static>(
        &mut self,
        reservation: Reservation<T>,
        build_entity: impl FnOnce(&mut Context<T>) -> T,
    ) -> Entity<T> {
        let app = self.app();
        let mut app = app.borrow_mut();
        app.insert_entity(reservation, build_entity)
    }

    fn update_entity<T: 'static, R>(
        &mut self,
        handle: &Entity<T>,
        update: impl FnOnce(&mut T, &mut Context<T>) -> R,
    ) -> R {
        let app = self.app();
        let mut app = app.borrow_mut();
        app.update_entity(handle, update)
    }

    fn as_mut<'a, T>(&'a mut self, _handle: &Entity<T>) -> GpuiBorrow<'a, T>
    where
        T: 'static,
    {
        panic!("Cannot as_mut with an async context. Try calling update() first")
    }

    fn read_entity<T, R>(&self, handle: &Entity<T>, callback: impl FnOnce(&T, &App) -> R) -> R
    where
        T: 'static,
    {
        let app = self.app();
        let lock = app.borrow();
        lock.read_entity(handle, callback)
    }

    fn notify(&mut self, entity_id: EntityId) {
        let app = self.app();
        let mut lock = app.borrow_mut();
        lock.notify(entity_id);
    }

    fn emit<EntityType, EventType>(&mut self, entity: &Entity<EntityType>, event: EventType)
    where
        EntityType: EventEmitter<EventType>,
        EventType: 'static,
    {
        let app = self.app();
        let mut lock = app.borrow_mut();
        lock.emit(entity, event)
    }

    fn update_window<T, F>(&mut self, window: AnyWindowHandle, f: F) -> Result<T>
    where
        F: FnOnce(AnyView, &mut Window, &mut App) -> T,
    {
        let app = self.app.upgrade().context("app was released")?;
        let mut lock = app.try_borrow_mut()?;
        if lock.quitting {
            bail!("app is quitting");
        }
        lock.update_window(window, f)
    }

    fn with_window<R>(
        &mut self,
        entity_id: EntityId,
        f: impl FnOnce(&mut Window, &mut App) -> R,
    ) -> Option<R> {
        let app = self.app.upgrade()?;
        let mut lock = app.try_borrow_mut().ok()?;
        if lock.quitting {
            return None;
        }
        lock.with_window(entity_id, f)
    }

    fn read_window<T, R>(
        &self,
        window: &WindowHandle<T>,
        read: impl FnOnce(Entity<T>, &App) -> R,
    ) -> Result<R>
    where
        T: 'static,
    {
        let app = self.app.upgrade().context("app was released")?;
        let lock = app.borrow();
        if lock.quitting {
            bail!("app is quitting");
        }
        lock.read_window(window, read)
    }

    #[track_caller]
    fn background_spawn<R>(&self, future: impl Future<Output = R> + Send + 'static) -> Task<R>
    where
        R: Send + 'static,
    {
        self.background_executor.spawn(future)
    }

    fn read_global<G, R>(&self, callback: impl FnOnce(&G, &App) -> R) -> R
    where
        G: Global,
    {
        let app = self.app();
        let mut lock = app.borrow_mut();
        lock.update(|this| this.read_global(callback))
    }

    fn insert_global_entity<T: 'static>(&mut self, entity: Entity<T>) {
        let app = self.app();
        let mut lock = app.borrow_mut();
        lock.insert_global_entity(entity)
    }

    fn remove_global_entity<T: 'static>(&mut self, entity: &Entity<T>) {
        let app = self.app();
        let mut lock = app.borrow_mut();
        lock.remove_global_entity(entity)
    }

    fn global_entities<T: 'static>(&self) -> impl Iterator<Item = Entity<T>> {
        let app = self.app();
        let mut lock = app.borrow_mut();
        let iter = lock.global_entities();
        // Cloning the iterator here so that lock can be released when function is going out of scope
        iter.collect::<Vec<_>>().into_iter()
    }
}

impl AsyncApp {
    /// Schedules a redraw of every window.
    ///
    /// The windows draw on the next frame. This doesn't draw anything itself.
    pub fn refresh(&self) {
        let app = self.app();
        let mut lock = app.borrow_mut();
        // A direct call would leave the refresh effect queued, which cannot wake
        // a platform render loop that has already parked.
        lock.update(|cx| cx.refresh_windows());
    }

    /// Returns the [`BackgroundExecutor`], for running work on other threads.
    pub fn background_executor(&self) -> &BackgroundExecutor {
        &self.background_executor
    }

    /// Returns the [`ForegroundExecutor`], for running futures on the main thread.
    pub fn foreground_executor(&self) -> &ForegroundExecutor {
        &self.foreground_executor
    }

    /// Runs `f` with the app and returns its result.
    ///
    /// Effects queued by `f`, such as notifications, are flushed before this returns.
    ///
    /// # Panics
    ///
    /// Panics if the app has been released, or if it is already borrowed, such as when
    /// called from inside another update.
    pub fn update<R>(&self, f: impl FnOnce(&mut App) -> R) -> R {
        let app = self.app();
        let mut lock = app.borrow_mut();
        lock.update(f)
    }

    /// Calls `on_event` each time `entity` emits an `Event`.
    ///
    /// The callback receives the emitting entity, the event and the app. The
    /// subscription stays active while you hold it. Dropping it unsubscribes, and
    /// [`Subscription::detach`] keeps the subscription for as long as the entity lives.
    /// Inside an entity's own context, [`Context::subscribe`] is the usual choice.
    pub fn subscribe<T, Event>(
        &mut self,
        entity: &Entity<T>,
        on_event: impl FnMut(Entity<T>, &Event, &mut App) + 'static,
    ) -> Subscription
    where
        T: 'static + EventEmitter<Event>,
        Event: 'static,
    {
        let app = self.app();
        let mut lock = app.borrow_mut();
        lock.subscribe(entity, on_event)
    }

    /// Opens a window with `options` and returns a handle to it.
    ///
    /// `build_root_view` creates the window's root view. The method returns an error if the app is
    /// quitting.
    ///
    /// # Panics
    ///
    /// Panics if the app has been released, or if it is already borrowed.
    pub fn open_window<V>(
        &self,
        options: crate::WindowOptions,
        build_root_view: impl FnOnce(&mut Window, &mut App) -> Entity<V>,
    ) -> Result<WindowHandle<V>>
    where
        V: 'static + Render,
    {
        let app = self.app();
        let mut lock = app.borrow_mut();
        if lock.quitting {
            bail!("app is quitting");
        }
        lock.open_window(options, build_root_view)
    }

    /// Runs the future returned by `f` on the main thread, and returns a task for its
    /// result.
    ///
    /// `f` receives an `&mut AsyncApp`. Dropping the returned [`Task`] cancels the
    /// future, so store it, await it or call [`Task::detach`].
    #[track_caller]
    pub fn spawn<AsyncFn, R>(&self, f: AsyncFn) -> Task<R>
    where
        AsyncFn: AsyncFnOnce(&mut AsyncApp) -> R + 'static,
        R: 'static,
    {
        let mut cx = self.clone();
        self.foreground_executor
            .spawn(async move { f(&mut cx).await }.boxed_local())
    }

    /// Returns whether a global of type `G` has been set.
    pub fn has_global<G: Global>(&self) -> bool {
        let app = self.app();
        let app = app.borrow_mut();
        app.has_global::<G>()
    }

    /// Calls `read` with the global of type `G` and the app, and returns its result.
    ///
    /// Use [`AsyncApp::try_read_global`] if the global may not be set.
    ///
    /// # Panics
    ///
    /// Panics if no global of type `G` has been set, or if the app has been released.
    pub fn read_global<G: Global, R>(&self, read: impl FnOnce(&G, &App) -> R) -> R {
        let app = self.app();
        let app = app.borrow_mut();
        read(app.global(), &app)
    }

    /// Like [`AsyncApp::read_global`], but returns `None` instead of panicking when no
    /// global of type `G` is set, or when the app is quitting.
    pub fn try_read_global<G: Global, R>(&self, read: impl FnOnce(&G, &App) -> R) -> Option<R> {
        let app = self.app();
        let app = app.borrow_mut();
        if app.quitting {
            return None;
        }
        Some(read(app.try_global()?, &app))
    }

    /// Like [`AsyncApp::read_global`], but first sets a `G::default()` if no global of
    /// type `G` has been set.
    pub fn read_default_global<G: Global + Default, R>(
        &self,
        read: impl FnOnce(&G, &App) -> R,
    ) -> R {
        let app = self.app();
        let mut app = app.borrow_mut();
        app.update(|cx| {
            cx.default_global::<G>();
        });
        read(app.global(), &app)
    }

    /// Updates the global of type `G` with `update`, and returns its result.
    ///
    /// While `update` runs, the global is taken out of the app. Reading it through the
    /// app from inside `update` panics, so change the global through the `&mut G` that
    /// `update` receives.
    ///
    /// # Panics
    ///
    /// Panics if no global of type `G` has been set, or if the app has been released.
    pub fn update_global<G: Global, R>(&self, update: impl FnOnce(&mut G, &mut App) -> R) -> R {
        let app = self.app();
        let mut app = app.borrow_mut();
        app.update(|cx| cx.update_global(update))
    }

    /// Returns a guard that calls `f` with `entity` when the guard is dropped.
    ///
    /// `f` doesn't run if the entity has been released by then. Hold the guard in a
    /// variable. A guard that is dropped at once runs `f` immediately.
    ///
    /// # Examples
    ///
    /// ```
    /// use gpui::{AppContext, TestApp};
    ///
    /// let mut app = TestApp::new();
    /// let log = app.update(|cx| cx.new(|_| Vec::<&'static str>::new()));
    /// let guard = app.update(|cx| {
    ///     cx.to_async().on_drop(&log.downgrade(), |log, cx| {
    ///         log.push("finished");
    ///         cx.notify();
    ///     })
    /// });
    /// drop(guard);
    /// app.read(|cx| assert_eq!(*log.read(cx), vec!["finished"]));
    /// ```
    pub fn on_drop<T: 'static, Callback: FnOnce(&mut T, &mut Context<T>) + 'static>(
        &self,
        entity: &WeakEntity<T>,
        f: Callback,
    ) -> gpui_util::Deferred<impl FnOnce() + use<T, Callback>> {
        let entity = entity.clone();
        let mut cx = self.clone();
        gpui_util::defer(move || {
            entity.update(&mut cx, f).ok();
        })
    }
}

/// A handle to the app and one window, for use across `.await` points.
///
/// Get one from [`Context::spawn_in`] or [`Window::spawn`]. It derefs to
/// [`AsyncApp`], so the methods of `AsyncApp` are available on it too. The window
/// methods on this type return an error once the window has closed.
///
/// Use it inside a future when you need the window and not only the app. See
/// [contexts](crate::_documentation::concepts::contexts#async-contexts).
///
/// # Examples
///
/// ```
/// use gpui::{Context, Render, TestApp, Window, div, prelude::*};
///
/// struct Panel {
///     updates: usize,
/// }
///
/// impl Render for Panel {
///     fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
///         div()
///     }
/// }
///
/// let mut app = TestApp::new();
/// let mut window = app.open_window(|_window, _cx| Panel { updates: 0 });
///
/// window.update(|panel, window, cx| {
///     cx.spawn_in(window, async move |panel, cx| {
///         panel
///             .update(cx, |panel, cx| {
///                 panel.updates += 1;
///                 cx.notify();
///             })
///             .ok();
///     })
///     .detach();
/// });
///
/// app.run_until_parked();
/// window.read(|panel, _cx| assert_eq!(panel.updates, 1));
/// ```
#[derive(Clone, Deref, DerefMut)]
pub struct AsyncWindowContext {
    #[deref]
    #[deref_mut]
    app: AsyncApp,
    window: AnyWindowHandle,
}

impl AsyncWindowContext {
    pub(crate) fn new_context(app: AsyncApp, window: AnyWindowHandle) -> Self {
        Self { app, window }
    }

    /// Returns the handle of the window this context is for.
    pub fn window_handle(&self) -> AnyWindowHandle {
        self.window
    }

    /// Runs `update` with the window and the app, and returns its result.
    ///
    /// Effects queued by `update` are flushed before this returns.
    ///
    /// # Errors
    ///
    /// Returns an error if the window has closed, or if the app is quitting or already
    /// borrowed.
    pub fn update<R>(&mut self, update: impl FnOnce(&mut Window, &mut App) -> R) -> Result<R> {
        self.app
            .update_window(self.window, |_, window, cx| update(window, cx))
    }

    /// Like [`AsyncWindowContext::update`], but also passes the window's root view.
    ///
    /// # Errors
    ///
    /// Returns an error under the same conditions as [`AsyncWindowContext::update`].
    pub fn update_root<R>(
        &mut self,
        update: impl FnOnce(AnyView, &mut Window, &mut App) -> R,
    ) -> Result<R> {
        self.app.update_window(self.window, update)
    }

    /// Runs `f` before the window draws its next frame. Does nothing if the window has
    /// closed.
    ///
    /// Schedules the callback like [`Window::on_next_frame`], but tolerates a window that has
    /// already been closed.
    pub fn on_next_frame(&mut self, f: impl FnOnce(&mut Window, &mut App) + 'static) {
        self.app
            .update_window(self.window, |_, window, _| window.on_next_frame(f))
            .ok();
    }

    /// Calls `read` with the global of type `G`, the window and the app, and returns its
    /// result.
    ///
    /// # Errors
    ///
    /// Returns an error if the window has closed.
    ///
    /// # Panics
    ///
    /// Panics if no global of type `G` has been set.
    pub fn read_global<G: Global, R>(
        &mut self,
        read: impl FnOnce(&G, &Window, &App) -> R,
    ) -> Result<R> {
        self.app
            .update_window(self.window, |_, window, cx| read(cx.global(), window, cx))
    }

    /// Updates the global of type `G` with `update`, and returns its result.
    ///
    /// The global is taken out of the app while `update` runs, as with
    /// [`AsyncApp::update_global`].
    ///
    /// # Errors
    ///
    /// Returns an error if the window has closed.
    ///
    /// # Panics
    ///
    /// Panics if no global of type `G` has been set.
    pub fn update_global<G, R>(
        &mut self,
        update: impl FnOnce(&mut G, &mut Window, &mut App) -> R,
    ) -> Result<R>
    where
        G: Global,
    {
        self.app.update_window(self.window, |_, window, cx| {
            cx.update_global(|global, cx| update(global, window, cx))
        })
    }

    /// Runs the future returned by `f` on the main thread, with an
    /// `AsyncWindowContext` for this window, and returns a task for its result.
    ///
    /// Dropping the returned [`Task`] cancels the future. Store it, await it or call
    /// [`Task::detach`].
    #[track_caller]
    pub fn spawn<AsyncFn, R>(&self, f: AsyncFn) -> Task<R>
    where
        AsyncFn: AsyncFnOnce(&mut AsyncWindowContext) -> R + 'static,
        R: 'static,
    {
        let mut cx = self.clone();
        self.foreground_executor
            .spawn(async move { f(&mut cx).await }.boxed_local())
    }

    /// Shows a platform alert with one button for each entry in `answers`.
    ///
    /// The receiver resolves to the index of the button the user clicked. If the window
    /// has closed, the receiver is canceled, so awaiting it returns an error. See
    /// [`Window::prompt`] for `level` and `detail`.
    pub fn prompt<T>(
        &mut self,
        level: PromptLevel,
        message: &str,
        detail: Option<&str>,
        answers: &[T],
    ) -> oneshot::Receiver<usize>
    where
        T: Clone + Into<PromptButton>,
    {
        self.app
            .update_window(self.window, |_, window, cx| {
                window.prompt(level, message, detail, answers, cx)
            })
            .unwrap_or_else(|_| oneshot::channel().1)
    }
}

impl AppContext for AsyncWindowContext {
    fn new<T>(&mut self, build_entity: impl FnOnce(&mut Context<T>) -> T) -> Entity<T>
    where
        T: 'static,
    {
        let mut build_entity = Some(build_entity);
        match self.app.update_window(self.window, |_, _, cx| {
            cx.new(
                build_entity
                    .take()
                    .expect("build_entity is taken exactly once"),
            )
        }) {
            Ok(entity) => entity,
            Err(_) => self.app.new(
                build_entity
                    .take()
                    .expect("update_window returned Err without invoking the closure"),
            ),
        }
    }

    fn reserve_entity<T: 'static>(&mut self) -> Reservation<T> {
        self.app.reserve_entity()
    }

    fn insert_entity<T: 'static>(
        &mut self,
        reservation: Reservation<T>,
        build_entity: impl FnOnce(&mut Context<T>) -> T,
    ) -> Entity<T> {
        let mut args = Some((reservation, build_entity));
        match self.app.update_window(self.window, |_, _, cx| {
            let (reservation, build_entity) = args.take().expect("args are taken exactly once");
            cx.insert_entity(reservation, build_entity)
        }) {
            Ok(entity) => entity,
            Err(_) => {
                let (reservation, build_entity) = args
                    .take()
                    .expect("update_window returned Err without invoking the closure");
                self.app.insert_entity(reservation, build_entity)
            }
        }
    }

    fn update_entity<T: 'static, R>(
        &mut self,
        handle: &Entity<T>,
        update: impl FnOnce(&mut T, &mut Context<T>) -> R,
    ) -> R {
        self.app.update_entity(handle, update)
    }

    fn as_mut<'a, T>(&'a mut self, _: &Entity<T>) -> GpuiBorrow<'a, T>
    where
        T: 'static,
    {
        panic!("Cannot use as_mut() from an async context, call `update`")
    }

    fn read_entity<T, R>(&self, handle: &Entity<T>, read: impl FnOnce(&T, &App) -> R) -> R
    where
        T: 'static,
    {
        self.app.read_entity(handle, read)
    }

    fn notify(&mut self, entity_id: EntityId) {
        self.app.notify(entity_id);
    }

    fn emit<EntityType, EventType>(&mut self, entity: &Entity<EntityType>, event: EventType)
    where
        EntityType: EventEmitter<EventType>,
        EventType: 'static,
    {
        self.app.emit(entity, event)
    }

    fn update_window<T, F>(&mut self, window: AnyWindowHandle, update: F) -> Result<T>
    where
        F: FnOnce(AnyView, &mut Window, &mut App) -> T,
    {
        self.app.update_window(window, update)
    }

    fn with_window<R>(
        &mut self,
        entity_id: EntityId,
        f: impl FnOnce(&mut Window, &mut App) -> R,
    ) -> Option<R> {
        self.app.with_window(entity_id, f)
    }

    fn read_window<T, R>(
        &self,
        window: &WindowHandle<T>,
        read: impl FnOnce(Entity<T>, &App) -> R,
    ) -> Result<R>
    where
        T: 'static,
    {
        self.app.read_window(window, read)
    }

    #[track_caller]
    fn background_spawn<R>(&self, future: impl Future<Output = R> + Send + 'static) -> Task<R>
    where
        R: Send + 'static,
    {
        self.app.background_executor.spawn(future)
    }

    fn read_global<G, R>(&self, callback: impl FnOnce(&G, &App) -> R) -> R
    where
        G: Global,
    {
        self.app.read_global(callback)
    }

    fn insert_global_entity<T: 'static>(&mut self, entity: Entity<T>) {
        self.app.insert_global_entity(entity)
    }

    fn remove_global_entity<T: 'static>(&mut self, entity: &Entity<T>) {
        self.app.remove_global_entity(entity)
    }

    fn global_entities<T: 'static>(&self) -> impl Iterator<Item = Entity<T>> {
        self.app.global_entities()
    }
}

impl VisualContext for AsyncWindowContext {
    type Result<T> = Result<T>;

    fn window_handle(&self) -> AnyWindowHandle {
        self.window
    }

    fn new_window_entity<T: 'static>(
        &mut self,
        build_entity: impl FnOnce(&mut Window, &mut Context<T>) -> T,
    ) -> Result<Entity<T>> {
        self.app.update_window(self.window, |_, window, cx| {
            cx.new(|cx| build_entity(window, cx))
        })
    }

    fn update_window_entity<T: 'static, R>(
        &mut self,
        view: &Entity<T>,
        update: impl FnOnce(&mut T, &mut Window, &mut Context<T>) -> R,
    ) -> Result<R> {
        let view = view.clone();
        self.app
            .with_window(view.entity_id(), |window, app| {
                view.update(app, |entity, cx| update(entity, window, cx))
            })
            .context("entity has no current window")
    }

    fn replace_root_view<V>(
        &mut self,
        build_view: impl FnOnce(&mut Window, &mut Context<V>) -> V,
    ) -> Result<Entity<V>>
    where
        V: 'static + Render,
    {
        self.app.update_window(self.window, |_, window, cx| {
            window.replace_root(cx, build_view)
        })
    }

    fn focus<V>(&mut self, view: &Entity<V>) -> Result<()>
    where
        V: Focusable,
    {
        self.app.update_window(self.window, |_, window, cx| {
            view.read(cx).focus_handle(cx).focus(window, cx);
        })
    }
}
