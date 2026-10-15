use crate::{
    AnyView, AnyWindowHandle, AppContext, AsyncApp, DispatchPhase, EntityId, EventEmitter,
    FocusHandle, FocusOutEvent, Focusable, Global, KeystrokeObserver, Priority, Reservation,
    SubscriberSet, Subscription, Task, WeakEntity, WeakFocusHandle, Window, WindowHandle,
};
use anyhow::Result;
use futures::FutureExt;
use gpui_util::Deferred;
use std::{
    any::{Any, TypeId},
    borrow::{Borrow, BorrowMut},
    future::Future,
    ops,
    sync::Arc,
};

use super::{App, AsyncWindowContext, Entity, KeystrokeEvent};

/// A mutable handle to the [`App`] that also knows which entity `T` you are working on.
///
/// GPUI passes a `Context<T>` to every callback that works on an entity. [`Entity::update`]
/// and [`WeakEntity::update`] pass one to their closure, and render methods receive one as
/// their last argument. Because `Context<T>` dereferences to [`App`], you can use any `App`
/// method through it. The methods on `Context<T>` itself need the entity: [`Context::notify`],
/// [`Context::emit`], [`Context::observe`], [`Context::subscribe`], [`Context::listener`] and
/// [`Context::spawn`].
///
/// Use the context of the entity you're changing for anything about that entity, and use
/// `&mut App` for work that spans the application. See
/// [contexts](crate::_documentation::concepts::contexts) for when you get each kind of context.
///
/// # Examples
///
/// ```
/// # use gpui::{AppContext, Context, TestApp};
/// struct Clock {
///     ticks: u64,
/// }
///
/// impl Clock {
///     fn tick(&mut self, cx: &mut Context<Self>) {
///         self.ticks += 1;
///         cx.notify();
///     }
/// }
///
/// # let mut app = TestApp::new();
/// # app.update(|cx| {
/// let clock = cx.new(|_cx| Clock { ticks: 0 });
/// clock.update(cx, |clock, cx| clock.tick(cx));
/// assert_eq!(clock.read(cx).ticks, 1);
/// # });
/// ```
pub struct Context<'a, T> {
    app: &'a mut App,
    entity_state: WeakEntity<T>,
}

impl<'a, T> ops::Deref for Context<'a, T> {
    type Target = App;

    fn deref(&self) -> &Self::Target {
        self.app
    }
}

impl<'a, T> ops::DerefMut for Context<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.app
    }
}

impl<'a, T: 'static> Context<'a, T> {
    pub(crate) fn new_context(app: &'a mut App, entity_state: WeakEntity<T>) -> Self {
        Self { app, entity_state }
    }

    /// Returns the id of the entity this context works on.
    pub fn entity_id(&self) -> EntityId {
        self.entity_state.entity_id
    }

    /// Returns a strong handle to the entity this context works on.
    ///
    /// The handle keeps the entity alive. Use [`Context::weak_entity`] when a callback should not
    /// keep it alive.
    pub fn entity(&self) -> Entity<T> {
        self.weak_entity()
            .upgrade()
            .expect("The entity must be alive if we have a entity context")
    }

    /// Returns a weak handle to the entity this context works on.
    ///
    /// Store the weak handle in callbacks and tasks that can outlive the current update. Upgrade it
    /// when you need the entity, and handle the case where it has been released.
    pub fn weak_entity(&self) -> WeakEntity<T> {
        self.entity_state.clone()
    }

    /// Calls `on_notify` whenever `entity` calls [`Context::notify`].
    ///
    /// Use this to keep one entity in step with another. When `entity` changes, your callback can
    /// read it and update your own state. The callback receives this entity mutably, a handle to the
    /// observed entity, and a context for this entity. It runs after the update that called `notify`
    /// finishes. Several notifications before that point deliver the callback once.
    ///
    /// The callback stays registered while the returned [`Subscription`] is alive. Keep it in a field
    /// so the callback lasts as long as your entity, or call [`Subscription::detach`] to keep it for as
    /// long as both entities exist. Once this entity is released, the callback is never called again.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{AppContext, Context, TestApp};
    /// struct Counter {
    ///     count: usize,
    /// }
    ///
    /// struct Label {
    ///     text: String,
    /// }
    ///
    /// # let mut app = TestApp::new();
    /// # let (counter, label) = app.update(|cx| {
    /// let counter = cx.new(|_cx| Counter { count: 0 });
    /// let label = cx.new(|cx: &mut Context<Label>| {
    ///     cx.observe(&counter, |label, counter, cx| {
    ///         label.text = counter.read(cx).count.to_string();
    ///     })
    ///     .detach();
    ///     Label { text: String::new() }
    /// });
    /// (counter, label)
    /// # });
    /// # app.update(|cx| {
    /// counter.update(cx, |counter, cx| {
    ///     counter.count = 21;
    ///     cx.notify();
    /// });
    /// # });
    /// # app.read(|cx| assert_eq!(label.read(cx).text, "21"));
    /// ```
    ///
    /// See [ownership and data flow](crate::_documentation::concepts::ownership).
    pub fn observe<W>(
        &mut self,
        entity: &Entity<W>,
        mut on_notify: impl FnMut(&mut T, Entity<W>, &mut Context<T>) + 'static,
    ) -> Subscription
    where
        T: 'static,
        W: 'static,
    {
        let this = self.weak_entity();
        self.app.observe_internal(entity, move |e, cx| {
            if let Some(this) = this.upgrade() {
                this.update(cx, |this, cx| on_notify(this, e, cx));
                true
            } else {
                false
            }
        })
    }

    /// Calls `on_event` whenever this entity calls [`Context::notify`].
    ///
    /// It runs after the update that called `notify` finishes. Keep the returned
    /// [`Subscription`] alive for as long as you want the callback to run.
    pub fn observe_self(
        &mut self,
        mut on_event: impl FnMut(&mut T, &mut Context<T>) + 'static,
    ) -> Subscription
    where
        T: 'static,
    {
        let this = self.entity();
        self.app.observe(&this, move |this, cx| {
            this.update(cx, |this, cx| on_event(this, cx))
        })
    }

    /// Calls `on_event` whenever `entity` emits an event of type `Evt`.
    ///
    /// The emitting entity must implement [`EventEmitter<Evt>`](crate::EventEmitter). Events are
    /// delivered after the update that emitted them finishes, in the order they were emitted. Each event
    /// type needs its own subscription. The callback receives this entity mutably, a handle to the
    /// emitting entity, the event, and a context for this entity.
    ///
    /// Keep the returned [`Subscription`] in a field, or call [`Subscription::detach`] to keep the
    /// callback for as long as both entities exist.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{AppContext, Context, EventEmitter, TestApp};
    /// struct Downloader;
    ///
    /// enum DownloadEvent {
    ///     Finished { bytes: usize },
    /// }
    ///
    /// impl EventEmitter<DownloadEvent> for Downloader {}
    ///
    /// struct StatusBar {
    ///     message: String,
    /// }
    ///
    /// # let mut app = TestApp::new();
    /// # let (downloader, status) = app.update(|cx| {
    /// let downloader = cx.new(|_cx| Downloader);
    ///
    /// let status = cx.new(|cx: &mut Context<StatusBar>| {
    ///     cx.subscribe(&downloader, |status, _downloader, event: &DownloadEvent, _cx| match event {
    ///         DownloadEvent::Finished { bytes } => {
    ///             status.message = format!("Downloaded {bytes} bytes");
    ///         }
    ///     })
    ///     .detach();
    ///     StatusBar { message: String::new() }
    /// });
    /// (downloader, status)
    /// # });
    /// # app.update(|cx| {
    /// downloader.update(cx, |_downloader, cx| {
    ///     cx.emit(DownloadEvent::Finished { bytes: 512 });
    /// });
    /// # });
    /// # app.read(|cx| assert_eq!(status.read(cx).message, "Downloaded 512 bytes"));
    /// ```
    pub fn subscribe<T2, Evt>(
        &mut self,
        entity: &Entity<T2>,
        mut on_event: impl FnMut(&mut T, Entity<T2>, &Evt, &mut Context<T>) + 'static,
    ) -> Subscription
    where
        T: 'static,
        T2: 'static + EventEmitter<Evt>,
        Evt: 'static,
    {
        let this = self.weak_entity();
        self.app.subscribe_internal(entity, move |e, event, cx| {
            if let Some(this) = this.upgrade() {
                this.update(cx, |this, cx| on_event(this, e, event, cx));
                true
            } else {
                false
            }
        })
    }

    /// Calls `on_event` whenever this entity emits an event of type `Evt`.
    ///
    /// This is [`Context::subscribe`] for events that this entity emits itself. Keep the returned
    /// [`Subscription`] alive for as long as you want the callback to run.
    pub fn subscribe_self<Evt>(
        &mut self,
        mut on_event: impl FnMut(&mut T, &Evt, &mut Context<T>) + 'static,
    ) -> Subscription
    where
        T: 'static + EventEmitter<Evt>,
        Evt: 'static,
    {
        let this = self.entity();
        self.app.subscribe(&this, move |this, evt, cx| {
            this.update(cx, |this, cx| on_event(this, evt, cx))
        })
    }

    /// Calls `on_release` when GPUI releases this entity.
    ///
    /// GPUI releases an entity at the end of the update in which its last strong handle was dropped.
    /// The callback runs then, while the value still exists, and receives it mutably together with the
    /// app. Use it to tear down resources the entity holds outside GPUI.
    ///
    /// The callback stays registered while the returned [`Subscription`] is alive. Dropping the
    /// subscription cancels the callback. Call [`Subscription::detach`] to keep it until the entity is
    /// released.
    pub fn on_release(&self, on_release: impl FnOnce(&mut T, &mut App) + 'static) -> Subscription
    where
        T: 'static,
    {
        let (subscription, activate) = self.app.release_listeners.insert(
            self.entity_state.entity_id,
            Box::new(move |this, cx| {
                let this = this.downcast_mut().expect("invalid entity type");
                on_release(this, cx);
            }),
        );
        activate();
        subscription
    }

    /// Calls `on_release` when `entity` is released.
    ///
    /// The callback receives this entity mutably and the released value. It doesn't run if this entity
    /// has been released too. The callback stays registered while the returned [`Subscription`] is
    /// alive.
    pub fn observe_release<T2>(
        &self,
        entity: &Entity<T2>,
        on_release: impl FnOnce(&mut T, &mut T2, &mut Context<T>) + 'static,
    ) -> Subscription
    where
        T: Any,
        T2: 'static,
    {
        let entity_id = entity.entity_id();
        let this = self.weak_entity();
        let (subscription, activate) = self.app.release_listeners.insert(
            entity_id,
            Box::new(move |entity, cx| {
                let entity = entity.downcast_mut().expect("invalid entity type");
                if let Some(this) = this.upgrade() {
                    this.update(cx, |this, cx| on_release(this, entity, cx));
                }
            }),
        );
        activate();
        subscription
    }

    /// Calls `f` whenever the global of type `G` is set or updated.
    ///
    /// `f` runs after the update that changed the global finishes. It doesn't receive the value, so
    /// read it with [`App::global`]. Keep the returned [`Subscription`] alive for as long as you want
    /// the callback to run. To also receive a window, use [`Context::observe_global_in`].
    pub fn observe_global<G: 'static>(
        &mut self,
        mut f: impl FnMut(&mut T, &mut Context<T>) + 'static,
    ) -> Subscription
    where
        T: 'static,
    {
        let handle = self.weak_entity();
        let (subscription, activate) = self.global_observers.insert(
            TypeId::of::<G>(),
            Box::new(move |cx| handle.update(cx, |view, cx| f(view, cx)).is_ok()),
        );
        self.defer(move |_| activate());
        subscription
    }

    /// Calls `on_restart` when the application is about to restart.
    ///
    /// The callback doesn't run if this entity has been released. Keep the returned [`Subscription`]
    /// alive for as long as you want the callback to run.
    pub fn on_app_restart(
        &self,
        mut on_restart: impl FnMut(&mut T, &mut App) + 'static,
    ) -> Subscription
    where
        T: 'static,
    {
        let handle = self.weak_entity();
        self.app.on_app_restart(move |cx| {
            handle.update(cx, |entity, cx| on_restart(entity, cx)).ok();
        })
    }

    /// Calls `on_quit` when the application quits, and waits for the future it returns.
    ///
    /// GPUI polls the returned future for up to [`SHUTDOWN_TIMEOUT`](crate::SHUTDOWN_TIMEOUT) while the
    /// app quits. Use this to save state before the process exits. The callback doesn't run if this
    /// entity has been released.
    pub fn on_app_quit<Fut>(
        &self,
        mut on_quit: impl FnMut(&mut T, &mut Context<T>) -> Fut + 'static,
    ) -> Subscription
    where
        Fut: 'static + Future<Output = ()>,
        T: 'static,
    {
        let handle = self.weak_entity();
        self.app.on_app_quit(move |cx| {
            let future = handle.update(cx, |entity, cx| on_quit(entity, cx)).ok();
            async move {
                if let Some(future) = future {
                    future.await;
                }
            }
            .boxed_local()
        })
    }

    /// Marks this entity as changed, so that the windows showing it are redrawn and its observers are
    /// called.
    ///
    /// GPUI doesn't watch your fields: call `notify` after changing state that affects rendering or that
    /// observers registered with [`Context::observe`] care about. The effects run when the current
    /// update finishes, not during this call, and several notifications in one update are delivered
    /// once.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{AppContext, Context, TestApp};
    /// struct Counter {
    ///     count: usize,
    /// }
    ///
    /// impl Counter {
    ///     fn increment(&mut self, cx: &mut Context<Self>) {
    ///         self.count += 1;
    ///         cx.notify();
    ///     }
    /// }
    ///
    /// # let mut app = TestApp::new();
    /// # let counter = app.update(|cx| cx.new(|_| Counter { count: 0 }));
    /// # app.update(|cx| counter.update(cx, |counter, cx| counter.increment(cx)));
    /// # app.read(|cx| assert_eq!(counter.read(cx).count, 1));
    /// ```
    ///
    /// See [ownership and data flow](crate::_documentation::concepts::ownership).
    pub fn notify(&mut self) {
        self.app.notify(self.entity_state.entity_id);
    }

    /// Runs the future returned by `f` on the main thread, with a weak handle to this entity.
    ///
    /// `f` receives a [`WeakEntity<T>`] for this entity and an [`AsyncApp`]. Use the weak handle to
    /// reach the entity after an `.await`. Its `update` returns an error if the entity has been released
    /// in the meantime. Use the [`AsyncApp`] to reach the rest of the app.
    ///
    /// The returned [`Task`] cancels the future when it's dropped. Keep it in a field to cancel the work
    /// when this entity changes, or call [`Task::detach`] to let it continue after the local handle
    /// is gone. Detaching does not guarantee completion during shutdown. For work that needs a
    /// window, use [`Context::spawn_in`]. Use [`AppContext::background_spawn`] inside the future
    /// for work on the background executor; see [`BackgroundExecutor`](crate::BackgroundExecutor)
    /// for platform differences.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{AppContext, Context, TestApp};
    /// struct Search {
    ///     results: Vec<String>,
    /// }
    ///
    /// impl Search {
    ///     fn run(&mut self, query: String, cx: &mut Context<Self>) {
    ///         cx.spawn(async move |search, cx| {
    ///             // A real app would await a network request here.
    ///             let results = vec![format!("result for {query}")];
    ///             search
    ///                 .update(cx, |search, cx| {
    ///                     search.results = results;
    ///                     cx.notify();
    ///                 })
    ///                 .ok();
    ///         })
    ///         .detach();
    ///     }
    /// }
    ///
    /// # let mut app = TestApp::new();
    /// # let search = app.update(|cx| cx.new(|_cx| Search { results: Vec::new() }));
    /// app.update(|cx| {
    ///     search.update(cx, |search, cx| search.run("gpui".to_string(), cx));
    /// });
    /// // `TestApp::update` runs the task before it returns.
    /// app.read(|cx| assert_eq!(search.read(cx).results[0], "result for gpui"));
    /// ```
    ///
    /// See [async work](crate::_documentation::guides::async_work).
    #[track_caller]
    pub fn spawn<AsyncFn, R>(&self, f: AsyncFn) -> Task<R>
    where
        T: 'static,
        AsyncFn: AsyncFnOnce(WeakEntity<T>, &mut AsyncApp) -> R + 'static,
        R: 'static,
    {
        let this = self.weak_entity();
        self.app.spawn(async move |cx| f(this, cx).await)
    }

    /// Wraps `f` in an event handler that receives this entity mutably.
    ///
    /// Element event handlers receive the event, a [`Window`] and an [`App`], but not your entity.
    /// `listener` returns a handler that calls `f` with the entity, the event, the window and a context
    /// for the entity. Pass the result to an element's handler method, such as `on_click`.
    ///
    /// The handler holds a weak handle to the entity, so an element that is still registered doesn't
    /// keep its view alive. If the entity has been released, the handler does nothing. Call
    /// [`Context::notify`] inside `f` to redraw the window after you change state.
    ///
    /// # Examples
    ///
    /// ```
    /// use gpui::{Context, MouseButton, TestApp, Window, div, point, prelude::*, px};
    ///
    /// struct Counter {
    ///     count: usize,
    /// }
    ///
    /// impl Render for Counter {
    ///     fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    ///         div()
    ///             .id("button")
    ///             .size_full()
    ///             .on_click(cx.listener(|counter, _event, _window, cx| {
    ///                 counter.count += 1;
    ///                 cx.notify();
    ///             }))
    ///     }
    /// }
    ///
    /// let mut app = TestApp::new();
    /// let mut window = app.open_window(|_window, _cx| Counter { count: 0 });
    ///
    /// window.simulate_click(point(px(10.), px(10.)), MouseButton::Left);
    /// window.simulate_click(point(px(10.), px(10.)), MouseButton::Left);
    ///
    /// assert_eq!(window.read(|counter, _cx| counter.count), 2);
    /// ```
    ///
    /// See [ownership and data flow](crate::_documentation::concepts::ownership).
    pub fn listener<E: ?Sized>(
        &self,
        f: impl Fn(&mut T, &E, &mut Window, &mut Context<T>) + 'static,
    ) -> impl Fn(&E, &mut Window, &mut App) + 'static {
        let view = self.entity().downgrade();
        move |e: &E, window: &mut Window, cx: &mut App| {
            view.update(cx, |view, cx| f(view, e, window, cx)).ok();
        }
    }

    /// Wraps `f` in a closure that receives this entity mutably and returns a value.
    ///
    /// Use this when the caller needs the handler's return value. For event handlers, prefer
    /// [`Context::listener`]. The returned closure holds a strong handle to this entity, so it
    /// keeps the entity alive for as long as the closure exists.
    pub fn processor<E, R>(
        &self,
        f: impl Fn(&mut T, E, &mut Window, &mut Context<T>) -> R + 'static,
    ) -> impl Fn(E, &mut Window, &mut App) -> R + 'static {
        let view = self.entity();
        move |e: E, window: &mut Window, cx: &mut App| {
            view.update(cx, |view, cx| f(view, e, window, cx))
        }
    }

    /// Returns a guard that calls `f` with this entity when the guard is dropped.
    ///
    /// The guard is returned as a `Deferred`. Call `abort` on it to cancel the callback. If the entity
    /// has been released by the time the guard is dropped, `f` doesn't run.
    ///
    /// # Panics
    ///
    /// Panics if the guard is dropped while the app is already borrowed, for example inside another
    /// update of the same app, and the entity is still alive. Drop the guard from a later update instead.
    pub fn on_drop(
        &self,
        f: impl FnOnce(&mut T, &mut Context<T>) + 'static,
    ) -> Deferred<impl FnOnce()> {
        let this = self.weak_entity();
        let mut cx = self.to_async();
        gpui_util::defer(move || {
            this.update(&mut cx, f).ok();
        })
    }

    /// Focuses `view` in `window`.
    ///
    /// `W` must implement [`Focusable`]. The focus change uses the focus handle that `view` returns.
    pub fn focus_view<W: Focusable>(&mut self, view: &Entity<W>, window: &mut Window) {
        window.focus(&view.focus_handle(self), self);
    }

    /// Runs `f` with this entity and `window` on the next frame of `window`.
    ///
    /// The callback runs once, during the next frame. It doesn't run if this entity has been released.
    pub fn on_next_frame(
        &self,
        window: &mut Window,
        f: impl FnOnce(&mut T, &mut Window, &mut Context<T>) + 'static,
    ) where
        T: 'static,
    {
        let view = self.entity();
        window.on_next_frame(move |window, cx| view.update(cx, |view, cx| f(view, window, cx)));
    }

    /// Runs `f` with this entity and `window` at the end of the current effect cycle.
    ///
    /// Use it to change this entity from code that is still on the stack, such as a method called from
    /// inside this entity's own update. The callback runs after the outermost update finishes, when the
    /// entity is no longer borrowed. It doesn't run if this entity has been released.
    pub fn defer_in(
        &mut self,
        window: &Window,
        f: impl FnOnce(&mut T, &mut Window, &mut Context<T>) + 'static,
    ) {
        let view = self.weak_entity();
        let entity_id = self.entity_id();
        self.ensure_window(entity_id, window.handle.id);
        self.app.defer(move |cx| {
            cx.with_window(entity_id, |window, cx| {
                view.update(cx, |view, cx| f(view, window, cx)).ok();
            });
        });
    }

    /// Calls `on_notify` when `observed` calls [`Context::notify`], and passes it the window.
    ///
    /// Pass the window that displays this entity. If that window has been closed, the callback
    /// does not run.
    pub fn observe_in<V2>(
        &mut self,
        observed: &Entity<V2>,
        window: &mut Window,
        mut on_notify: impl FnMut(&mut T, Entity<V2>, &mut Window, &mut Context<T>) + 'static,
    ) -> Subscription
    where
        V2: 'static,
        T: 'static,
    {
        let observed_id = observed.entity_id();
        let observed = observed.downgrade();
        let observer = self.weak_entity();
        let observer_id = self.entity_id();
        self.ensure_window(observer_id, window.handle.id);
        self.new_observer(
            observed_id,
            Box::new(move |cx| {
                let Some((observer, observed)) = observer.upgrade().zip(observed.upgrade()) else {
                    return false;
                };
                cx.with_window(observer_id, |window, cx| {
                    observer.update(cx, |observer, cx| {
                        on_notify(observer, observed, window, cx);
                    });
                });
                true
            }),
        )
    }

    /// Calls `on_event` when `emitter` emits an event of type `Evt`, and passes it the window.
    ///
    /// Pass the window that displays this entity. If that window has been closed, the callback
    /// does not run.
    pub fn subscribe_in<Emitter, Evt>(
        &mut self,
        emitter: &Entity<Emitter>,
        window: &Window,
        mut on_event: impl FnMut(&mut T, &Entity<Emitter>, &Evt, &mut Window, &mut Context<T>) + 'static,
    ) -> Subscription
    where
        Emitter: EventEmitter<Evt>,
        Evt: 'static,
    {
        let emitter = emitter.downgrade();
        let subscriber = self.weak_entity();
        let subscriber_id = self.entity_id();
        self.ensure_window(subscriber_id, window.handle.id);
        self.new_subscription(
            emitter.entity_id(),
            (
                TypeId::of::<Evt>(),
                Box::new(move |event, cx| {
                    let Some((subscriber, emitter)) = subscriber.upgrade().zip(emitter.upgrade())
                    else {
                        return false;
                    };
                    let event = event.downcast_ref().expect("invalid event type");
                    cx.with_window(subscriber_id, |window, cx| {
                        subscriber.update(cx, |subscriber, cx| {
                            on_event(subscriber, &emitter, event, window, cx);
                        });
                    });
                    true
                }),
            ),
        )
    }

    /// Calls `on_release` with the window when this entity is released.
    ///
    /// The callback doesn't run if the window was closed before this entity was released.
    pub fn on_release_in(
        &mut self,
        window: &Window,
        on_release: impl FnOnce(&mut T, &mut Window, &mut App) + 'static,
    ) -> Subscription {
        let entity = self.entity();
        self.app.observe_release_in(&entity, window, on_release)
    }

    /// Calls `on_release` when `observed` is released, and passes it the window.
    ///
    /// The callback does not run if `window` has been closed, or if this entity has been released.
    pub fn observe_release_in<T2>(
        &self,
        observed: &Entity<T2>,
        window: &Window,
        mut on_release: impl FnMut(&mut T, &mut T2, &mut Window, &mut Context<T>) + 'static,
    ) -> Subscription
    where
        T: 'static,
        T2: 'static,
    {
        let observer = self.weak_entity();
        self.app
            .observe_release_in(observed, window, move |observed, window, cx| {
                observer
                    .update(cx, |observer, cx| {
                        on_release(observer, observed, window, cx)
                    })
                    .ok();
            })
    }

    /// Calls `callback` when the bounds of `window` change.
    ///
    /// Keep the returned [`Subscription`] alive for as long as you want the callback to run. The callback
    /// stops once this entity is released.
    pub fn observe_window_bounds(
        &self,
        window: &mut Window,
        mut callback: impl FnMut(&mut T, &mut Window, &mut Context<T>) + 'static,
    ) -> Subscription {
        let view = self.weak_entity();
        let (subscription, activate) = window.bounds_observers.insert(
            (),
            Box::new(move |window, cx| {
                view.update(cx, |view, cx| callback(view, window, cx))
                    .is_ok()
            }),
        );
        activate();
        subscription
    }

    /// Calls `callback` when `window` becomes active or inactive.
    ///
    /// Keep the returned [`Subscription`] alive for as long as you want the callback to run. The callback
    /// stops once this entity is released.
    pub fn observe_window_activation(
        &self,
        window: &mut Window,
        mut callback: impl FnMut(&mut T, &mut Window, &mut Context<T>) + 'static,
    ) -> Subscription {
        let view = self.weak_entity();
        let (subscription, activate) = window.activation_observers.insert(
            (),
            Box::new(move |window, cx| {
                view.update(cx, |view, cx| callback(view, window, cx))
                    .is_ok()
            }),
        );
        activate();
        subscription
    }

    /// Calls `callback` when the system appearance changes between light and dark.
    ///
    /// Keep the returned [`Subscription`] alive for as long as you want the callback to run. The callback
    /// stops once this entity is released.
    pub fn observe_window_appearance(
        &self,
        window: &mut Window,
        mut callback: impl FnMut(&mut T, &mut Window, &mut Context<T>) + 'static,
    ) -> Subscription {
        let view = self.weak_entity();
        let (subscription, activate) = window.appearance_observers.insert(
            (),
            Box::new(move |window, cx| {
                view.update(cx, |view, cx| callback(view, window, cx))
                    .is_ok()
            }),
        );
        activate();
        subscription
    }

    /// Calls `callback` when the window's titlebar button layout changes.
    ///
    /// Keep the returned [`Subscription`] alive for as long as you want the callback to run. The callback
    /// stops once this entity is released.
    pub fn observe_button_layout_changed(
        &self,
        window: &mut Window,
        mut callback: impl FnMut(&mut T, &mut Window, &mut Context<T>) + 'static,
    ) -> Subscription {
        let view = self.weak_entity();
        let (subscription, activate) = window.button_layout_observers.insert(
            (),
            Box::new(move |window, cx| {
                view.update(cx, |view, cx| callback(view, window, cx))
                    .is_ok()
            }),
        );
        activate();
        subscription
    }

    /// Calls `f` for each key-down keystroke in any window that reaches the end of dispatch, or that a
    /// key binding handles.
    ///
    /// The keystroke's `action` is set when a key binding handled it. The callback doesn't run when a
    /// focused element's listener stops propagation. Keep the returned [`Subscription`] alive for as long
    /// as you want the callback to run.
    pub fn observe_keystrokes(
        &mut self,
        mut f: impl FnMut(&mut T, &KeystrokeEvent, &mut Window, &mut Context<T>) + 'static,
    ) -> Subscription {
        fn inner(
            keystroke_observers: &SubscriberSet<(), KeystrokeObserver>,
            handler: KeystrokeObserver,
        ) -> Subscription {
            let (subscription, activate) = keystroke_observers.insert((), handler);
            activate();
            subscription
        }

        let view = self.weak_entity();
        inner(
            &self.keystroke_observers,
            Box::new(move |event, window, cx| {
                if let Some(view) = view.upgrade() {
                    view.update(cx, |view, cx| f(view, event, window, cx));
                    true
                } else {
                    false
                }
            }),
        )
    }

    /// Calls `callback` when the pending input of `window` changes.
    ///
    /// Pending input is a partly typed key sequence, such as the first key of a multi-key binding. The
    /// callback runs when the sequence starts, grows, or is cleared. Keep the returned [`Subscription`]
    /// alive for as long as you want the callback to run.
    pub fn observe_pending_input(
        &self,
        window: &mut Window,
        mut callback: impl FnMut(&mut T, &mut Window, &mut Context<T>) + 'static,
    ) -> Subscription {
        let view = self.weak_entity();
        let (subscription, activate) = window.pending_input_observers.insert(
            (),
            Box::new(move |window, cx| {
                view.update(cx, |view, cx| callback(view, window, cx))
                    .is_ok()
            }),
        );
        activate();
        subscription
    }

    /// Calls `listener` when `handle` becomes the focused element.
    ///
    /// The listener doesn't run when focus moves into a descendant of `handle`. Use [`Context::on_focus_in`]
    /// to hear about that as well. Keep the returned [`Subscription`] alive for as long as you want the
    /// listener to run.
    pub fn on_focus(
        &mut self,
        handle: &FocusHandle,
        window: &mut Window,
        mut listener: impl FnMut(&mut T, &mut Window, &mut Context<T>) + 'static,
    ) -> Subscription {
        let view = self.weak_entity();
        let focus_id = handle.id;
        let (subscription, activate) =
            window.new_focus_listener(Box::new(move |event, window, cx| {
                view.update(cx, |view, cx| {
                    if event.previous_focus_path.last() != Some(&focus_id)
                        && event.current_focus_path.last() == Some(&focus_id)
                    {
                        listener(view, window, cx)
                    }
                })
                .is_ok()
            }));
        self.defer(|_| activate());
        subscription
    }

    /// Calls `listener` when focus moves into `handle` or one of its descendants.
    ///
    /// The listener doesn't run when focus was already inside `handle` and moves to another descendant.
    /// Keep the returned [`Subscription`] alive for as long as you want the listener to run.
    pub fn on_focus_in(
        &mut self,
        handle: &FocusHandle,
        window: &mut Window,
        mut listener: impl FnMut(&mut T, &mut Window, &mut Context<T>) + 'static,
    ) -> Subscription {
        let view = self.weak_entity();
        let focus_id = handle.id;
        let (subscription, activate) =
            window.new_focus_listener(Box::new(move |event, window, cx| {
                view.update(cx, |view, cx| {
                    if event.is_focus_in(focus_id) {
                        listener(view, window, cx)
                    }
                })
                .is_ok()
            }));
        self.defer(|_| activate());
        subscription
    }

    /// Calls `listener` when `handle` stops being the focused element.
    ///
    /// The listener also runs when focus moves from `handle` to one of its descendants, since `handle`
    /// is no longer the focused element. Use [`Context::on_focus_out`] to hear only about focus leaving
    /// `handle` and its descendants. Keep the returned [`Subscription`] alive for as long as you want the
    /// listener to run.
    pub fn on_blur(
        &mut self,
        handle: &FocusHandle,
        window: &mut Window,
        mut listener: impl FnMut(&mut T, &mut Window, &mut Context<T>) + 'static,
    ) -> Subscription {
        let view = self.weak_entity();
        let focus_id = handle.id;
        let (subscription, activate) =
            window.new_focus_listener(Box::new(move |event, window, cx| {
                view.update(cx, |view, cx| {
                    if event.previous_focus_path.last() == Some(&focus_id)
                        && event.current_focus_path.last() != Some(&focus_id)
                    {
                        listener(view, window, cx)
                    }
                })
                .is_ok()
            }));
        self.defer(|_| activate());
        subscription
    }

    /// Calls `listener` when no element in `window` has focus.
    ///
    /// This happens when the focused element is removed from the tree. Use it to move focus to a sensible
    /// default. Keep the returned [`Subscription`] alive for as long as you want the listener to run.
    pub fn on_focus_lost(
        &mut self,
        window: &mut Window,
        mut listener: impl FnMut(&mut T, &mut Window, &mut Context<T>) + 'static,
    ) -> Subscription {
        let view = self.weak_entity();
        let (subscription, activate) = window.focus_lost_listeners.insert(
            (),
            Box::new(move |window, cx| {
                view.update(cx, |view, cx| listener(view, window, cx))
                    .is_ok()
            }),
        );
        self.defer(|_| activate());
        subscription
    }

    /// Calls `listener` when focus leaves `handle` or one of its descendants.
    ///
    /// The listener receives a [`FocusOutEvent`] that says which element lost focus. Keep the returned
    /// [`Subscription`] alive for as long as you want the listener to run.
    pub fn on_focus_out(
        &mut self,
        handle: &FocusHandle,
        window: &mut Window,
        mut listener: impl FnMut(&mut T, FocusOutEvent, &mut Window, &mut Context<T>) + 'static,
    ) -> Subscription {
        let view = self.weak_entity();
        let focus_id = handle.id;
        let (subscription, activate) =
            window.new_focus_listener(Box::new(move |event, window, cx| {
                view.update(cx, |view, cx| {
                    if let Some(blurred_id) = event.previous_focus_path.last().copied()
                        && event.is_focus_out(focus_id)
                    {
                        let event = FocusOutEvent {
                            blurred: WeakFocusHandle {
                                id: blurred_id,
                                handles: Arc::downgrade(&cx.focus_handles),
                            },
                        };
                        listener(view, event, window, cx)
                    }
                })
                .is_ok()
            }));
        self.defer(|_| activate());
        subscription
    }

    /// Runs the future returned by `f` on the main thread, with a weak handle to this entity and a window
    /// context.
    ///
    /// `f` receives an [`AsyncWindowContext`] for `window`, so the future can reach it after an
    /// `.await`. Window operations through the context return errors once the window has closed.
    ///
    /// The returned [`Task`] cancels the future when it's dropped. Call [`Task::detach`] to let the future
    /// run to completion.
    #[track_caller]
    pub fn spawn_in<AsyncFn, R>(&self, window: &Window, f: AsyncFn) -> Task<R>
    where
        R: 'static,
        AsyncFn: AsyncFnOnce(WeakEntity<T>, &mut AsyncWindowContext) -> R + 'static,
    {
        let view = self.weak_entity();
        window.spawn(self, async move |cx| f(view, cx).await)
    }

    /// Runs the future returned by `f` on the main thread at `priority`, with a weak handle to this entity
    /// and a window context.
    ///
    /// The returned [`Task`] cancels the future when it is dropped.
    #[track_caller]
    pub fn spawn_in_with_priority<AsyncFn, R>(
        &self,
        priority: Priority,
        window: &Window,
        f: AsyncFn,
    ) -> Task<R>
    where
        R: 'static,
        AsyncFn: AsyncFnOnce(WeakEntity<T>, &mut AsyncWindowContext) -> R + 'static,
    {
        let view = self.weak_entity();
        window.spawn_with_priority(priority, self, async move |cx| f(view, cx).await)
    }

    /// Calls `f` with the window whenever the global of type `G` is set or updated.
    ///
    /// Pass the window this entity is shown in. If that window is unavailable, for example during a nested
    /// update of it, the callback is skipped for that change and stays registered. The callback stops
    /// once this entity is released.
    pub fn observe_global_in<G: Global>(
        &mut self,
        window: &Window,
        mut f: impl FnMut(&mut T, &mut Window, &mut Context<T>) + 'static,
    ) -> Subscription {
        let window_handle = window.handle;
        let view = self.weak_entity();
        let (subscription, activate) = self.global_observers.insert(
            TypeId::of::<G>(),
            Box::new(move |cx| {
                // If the entity has been dropped, remove this observer.
                if view.upgrade().is_none() {
                    return false;
                }
                // If the window is unavailable (e.g. temporarily taken during a
                // nested update, or already closed), skip this notification but
                // keep the observer alive so it can fire on future changes.
                let Ok(entity_alive) = window_handle.update(cx, |_, window, cx| {
                    view.update(cx, |view, cx| f(view, window, cx)).is_ok()
                }) else {
                    return true;
                };
                entity_alive
            }),
        );
        self.defer(move |_| activate());
        subscription
    }

    /// Calls `listener` when an action of type `action_type` is dispatched to `window`.
    ///
    /// Pass `TypeId::of::<A>()` for the action type `A`. The listener receives the action as `&dyn Any`
    /// together with the [`DispatchPhase`], so it can tell capture from bubble. It also gets the window and
    /// a context for this entity.
    ///
    /// Like [`Window::on_action`], the listener is registered for the next frame only. Call this during
    /// paint, on every frame that should handle the action.
    pub fn on_action(
        &mut self,
        action_type: TypeId,
        window: &mut Window,
        listener: impl Fn(&mut T, &dyn Any, DispatchPhase, &mut Window, &mut Context<T>) + 'static,
    ) {
        let handle = self.weak_entity();
        window.on_action(action_type, move |action, phase, window, cx| {
            handle
                .update(cx, |view, cx| {
                    listener(view, action, phase, window, cx);
                })
                .ok();
        });
    }

    /// Focuses this entity's focus handle in `window`.
    ///
    /// This entity must implement [`Focusable`]. The focus change happens at the end of the current effect
    /// cycle.
    pub fn focus_self(&mut self, window: &mut Window)
    where
        T: Focusable,
    {
        let view = self.entity();
        window.defer(self, move |window, cx| {
            view.read(cx).focus_handle(cx).focus(window, cx)
        })
    }
}

impl<T> Context<'_, T> {
    /// Emits `event` from this entity to its subscribers.
    ///
    /// Subscribers registered with [`Context::subscribe`] or [`App::subscribe`] for this event type
    /// receive it after the current update finishes, in the order the events were emitted. Emitting an
    /// event that nobody subscribed to does nothing. This entity must implement [`EventEmitter<Evt>`](crate::EventEmitter).
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{AppContext, Context, EventEmitter, TestApp};
    /// struct Button;
    ///
    /// struct Clicked {
    ///     times: usize,
    /// }
    ///
    /// impl EventEmitter<Clicked> for Button {}
    ///
    /// struct Tally {
    ///     clicks: usize,
    /// }
    ///
    /// # let mut app = TestApp::new();
    /// # let (button, tally) = app.update(|cx| {
    /// let button = cx.new(|_cx| Button);
    /// let tally = cx.new(|cx: &mut Context<Tally>| {
    ///     cx.subscribe(&button, |tally, _button, event: &Clicked, _cx| {
    ///         tally.clicks += event.times;
    ///     })
    ///     .detach();
    ///     Tally { clicks: 0 }
    /// });
    /// (button, tally)
    /// # });
    /// # app.update(|cx| {
    /// button.update(cx, |_button, cx| {
    ///     cx.emit(Clicked { times: 2 });
    ///     cx.emit(Clicked { times: 1 });
    /// });
    /// # });
    /// # app.read(|cx| assert_eq!(tally.read(cx).clicks, 3));
    /// ```
    ///
    /// See [ownership and data flow](crate::_documentation::concepts::ownership).
    pub fn emit<Evt>(&mut self, event: Evt)
    where
        T: EventEmitter<Evt>,
        Evt: 'static,
    {
        self.app.emit(&self.entity(), event);
    }
}

impl<T> AppContext for Context<'_, T> {
    #[inline]
    fn new<U: 'static>(&mut self, build_entity: impl FnOnce(&mut Context<U>) -> U) -> Entity<U> {
        self.app.new(build_entity)
    }

    #[inline]
    fn reserve_entity<U: 'static>(&mut self) -> Reservation<U> {
        self.app.reserve_entity()
    }

    #[inline]
    fn insert_entity<U: 'static>(
        &mut self,
        reservation: Reservation<U>,
        build_entity: impl FnOnce(&mut Context<U>) -> U,
    ) -> Entity<U> {
        self.app.insert_entity(reservation, build_entity)
    }

    #[inline]
    fn update_entity<U: 'static, R>(
        &mut self,
        handle: &Entity<U>,
        update: impl FnOnce(&mut U, &mut Context<U>) -> R,
    ) -> R {
        self.app.update_entity(handle, update)
    }

    #[inline]
    fn as_mut<'a, E>(&'a mut self, handle: &Entity<E>) -> super::GpuiBorrow<'a, E>
    where
        E: 'static,
    {
        self.app.as_mut(handle)
    }

    #[inline]
    fn read_entity<U, R>(&self, handle: &Entity<U>, read: impl FnOnce(&U, &App) -> R) -> R
    where
        U: 'static,
    {
        self.app.read_entity(handle, read)
    }

    #[inline]
    fn notify(&mut self, entity_id: EntityId) {
        self.app.notify(entity_id)
    }

    #[inline]
    fn emit<EntityType, EventType>(&mut self, entity: &Entity<EntityType>, event: EventType)
    where
        EntityType: EventEmitter<EventType>,
        EventType: 'static,
    {
        self.app.emit(entity, event)
    }

    #[inline]
    fn update_window<R, F>(&mut self, window: AnyWindowHandle, update: F) -> Result<R>
    where
        F: FnOnce(AnyView, &mut Window, &mut App) -> R,
    {
        self.app.update_window(window, update)
    }

    #[inline]
    fn with_window<R>(
        &mut self,
        entity_id: EntityId,
        f: impl FnOnce(&mut Window, &mut App) -> R,
    ) -> Option<R> {
        self.app.with_window(entity_id, f)
    }

    #[inline]
    fn read_window<U, R>(
        &self,
        window: &WindowHandle<U>,
        read: impl FnOnce(Entity<U>, &App) -> R,
    ) -> Result<R>
    where
        U: 'static,
    {
        self.app.read_window(window, read)
    }

    #[inline]
    fn background_spawn<R>(&self, future: impl Future<Output = R> + Send + 'static) -> Task<R>
    where
        R: Send + 'static,
    {
        self.app.background_executor.spawn(future)
    }

    #[inline]
    fn read_global<G, R>(&self, callback: impl FnOnce(&G, &App) -> R) -> R
    where
        G: Global,
    {
        self.app.read_global(callback)
    }

    #[inline]
    fn insert_global_entity<E: 'static>(&mut self, entity: Entity<E>) {
        self.app.insert_global_entity(entity)
    }

    #[inline]
    fn remove_global_entity<E: 'static>(&mut self, entity: &Entity<E>) {
        self.app.remove_global_entity(entity)
    }

    #[inline]
    fn global_entities<E: 'static>(&self) -> impl Iterator<Item = Entity<E>> {
        self.app.global_entities()
    }
}

impl<T> Borrow<App> for Context<'_, T> {
    fn borrow(&self) -> &App {
        self.app
    }
}

impl<T> BorrowMut<App> for Context<'_, T> {
    fn borrow_mut(&mut self) -> &mut App {
        self.app
    }
}
