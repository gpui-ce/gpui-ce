#![doc = include_str!("../docs/crate.md")]
#![warn(missing_docs)]
#![allow(clippy::type_complexity)] // Not useful, GPUI makes heavy use of callbacks
#![allow(clippy::collapsible_else_if)] // False positives in platform specific code
#![allow(unused_mut)] // False positives in platform specific code

extern crate self as gpui;
#[macro_use]
mod action;
mod animated;
mod app;

mod arena;
mod asset_cache;
mod assets;
mod bounds_tree;
mod color;
/// The default colors used by GPUI.
pub mod colors;
#[cfg(feature = "profiler")]
mod debug_overlay;
mod element;
mod elements;
mod executor;
mod platform_scheduler;
pub(crate) use platform_scheduler::PlatformScheduler;
mod geometry;
mod gestures;
mod global;
mod input;
mod inspector;
mod interactive;
mod key_dispatch;
mod keymap;
mod lerp;
mod motion;
mod path_builder;
mod platform;
pub mod prelude;
/// Profiling utilities for task, frame, and thread performance tracking.
pub mod profiler;
#[cfg(any(
    test,
    target_os = "windows",
    target_os = "linux",
    target_family = "wasm",
    feature = "test-support",
    feature = "bench-support"
))]
pub mod queue;
mod resize;
mod scene;
mod shared_uri;
mod spring;
mod style;
mod style_transitions;
mod styled;
mod subscription;
mod svg_renderer;
mod tab_stop;
mod taffy;
#[cfg(any(test, feature = "test-support"))]
pub mod test;
mod text_system;
mod transition;
mod util;
mod view;
mod window;

#[cfg(any(test, feature = "test-support"))]
pub use proptest;

pub mod _documentation;

/// Do not touch, here be dragons for use by gpui_macros and such.
#[doc(hidden)]
pub mod private {
    pub use anyhow;
    pub use inventory;
    pub use schemars;
    pub use serde;
    pub use serde_json;
}

mod seal {
    /// A mechanism for restricting implementations of a trait to only those in GPUI.
    /// See: <https://predr.ag/blog/definitive-guide-to-sealed-traits-in-rust/>
    pub trait Sealed {}
}

pub use accesskit;
pub use accesskit::Action as AccessibleAction;
pub use accesskit::{Orientation, Role, Toggled};
pub use action::*;
pub use animated::*;
pub use anyhow::Result;
pub use app::*;
pub(crate) use arena::*;
pub use asset_cache::*;
pub use assets::*;
pub use color::*;
pub use ctor::ctor;
#[cfg(feature = "profiler")]
pub use debug_overlay::*;
pub use element::*;
pub use elements::*;
pub use executor::*;
pub use geometry::*;
pub use gestures::*;
pub use global::*;
pub use gpui_macros::{
    AppContext, InteractiveElement, IntoElement, ParentElement, Render, StatefulInteractiveElement,
    Styled, VisualContext, bench, property_test, register_action, test,
};
pub use resize::*;
pub use spring::*;

/// Defines a Criterion benchmark group for benchmarks annotated with [`gpui::bench`].
///
/// It takes the same arguments as `criterion::criterion_group!`, so GPUI benchmark
/// files keep the shape of ordinary Criterion benchmarks.
///
/// [`gpui::bench`]: crate::bench
#[macro_export]
macro_rules! bench_group {
    ($($tokens:tt)*) => {
        criterion::criterion_group!($($tokens)*);
    };
}

/// Defines the entry point for the benchmark groups, like `criterion::criterion_main!`.
#[macro_export]
macro_rules! bench_main {
    ($($tokens:tt)*) => {
        criterion::criterion_main!($($tokens)*);
    };
}
pub use gpui_shared_string::*;
pub use gpui_util::arc_cow::ArcCow;
/// HTTP client abstraction for making requests.
pub mod http_client;
pub use input::*;
pub use inspector::*;
pub use interactive::*;
use key_dispatch::*;
pub use keymap::*;
pub use lerp::*;
pub use motion::*;
pub use path_builder::*;
pub use platform::*;
pub use profiler::*;
#[cfg(any(target_os = "windows", target_os = "linux", target_family = "wasm"))]
pub use queue::{PriorityQueueReceiver, PriorityQueueSender};
pub use refineable::*;
pub use scene::*;
pub use shared_uri::*;
use std::{any::Any, future::Future};
pub use style::*;
pub use style_transitions::*;
pub use styled::*;
pub use subscription::*;
pub use svg_renderer::*;
pub(crate) use tab_stop::*;
use taffy::TaffyLayoutEngine;
pub use taffy::{AvailableSpace, LayoutId};
#[cfg(any(test, feature = "test-support"))]
pub use test::*;
pub use text_system::*;
pub use transition::*;
pub use util::{FutureExt, Timeout};
pub use view::*;
pub use window::*;

#[cfg(not(target_family = "wasm"))]
pub use pollster::block_on;

/// Operations shared by GPUI's app, entity and async contexts.
///
/// Write a helper against this trait when it needs only entities, globals or the
/// app. Operations that need a window are on [`VisualContext`]. See
/// [contexts](crate::_documentation::concepts::contexts).
///
/// # Examples
///
/// ```
/// use gpui::{AppContext, Entity};
///
/// fn reset<C: AppContext>(count: &Entity<u64>, cx: &mut C) {
///     count.update(cx, |count, cx| {
///         *count = 0;
///         cx.notify();
///     });
/// }
/// # gpui::TestApp::new().update(|cx| {
/// #     let count = cx.new(|_| 5_u64);
/// #     reset(&count, cx);
/// #     assert_eq!(*count.read(cx), 0);
/// # });
/// ```
pub trait AppContext {
    /// Creates an entity from `build_entity`, and returns a handle to it.
    ///
    /// `build_entity` receives a [`Context`] for the new entity, so it can subscribe to
    /// other entities or start work of its own. The entity lives as long as a handle to
    /// it exists, or as long as it's stored with [`AppContext::insert_global_entity`].
    #[expect(
        clippy::wrong_self_convention,
        reason = "`App::new` is an ubiquitous function for creating entities"
    )]
    fn new<T: 'static>(&mut self, build_entity: impl FnOnce(&mut Context<T>) -> T) -> Entity<T>;

    /// Reserves an entity slot, and returns a [`Reservation`] for it.
    ///
    /// The reservation's [`EntityId`] is known now, so you can refer to the entity before
    /// it exists. Create the entity later with [`AppContext::insert_entity`].
    ///
    /// # Examples
    ///
    /// ```
    /// use gpui::{AppContext, TestApp};
    ///
    /// let mut app = TestApp::new();
    /// app.update(|cx| {
    ///     let reservation = cx.reserve_entity::<u32>();
    ///     let id = reservation.entity_id();
    ///     let number = cx.insert_entity(reservation, |_| 7);
    ///     assert_eq!(number.entity_id(), id);
    ///     assert_eq!(*number.read(cx), 7);
    /// });
    /// ```
    fn reserve_entity<T: 'static>(&mut self) -> Reservation<T>;

    /// Creates the entity for a [`Reservation`] from [`AppContext::reserve_entity`].
    ///
    /// The entity gets the reservation's [`EntityId`], and `build_entity` runs with a
    /// [`Context`] for it.
    fn insert_entity<T: 'static>(
        &mut self,
        reservation: Reservation<T>,
        build_entity: impl FnOnce(&mut Context<T>) -> T,
    ) -> Entity<T>;

    /// Runs `update` with the entity and a [`Context`] for it, and returns the result.
    ///
    /// Effects queued by `update`, such as notifications, run when the outermost update
    /// finishes.
    fn update_entity<T, R>(
        &mut self,
        handle: &Entity<T>,
        update: impl FnOnce(&mut T, &mut Context<T>) -> R,
    ) -> R
    where
        T: 'static;

    /// Borrows the entity mutably, without a closure.
    ///
    /// The returned [`GpuiBorrow`] dereferences to the entity. Dropping it notifies
    /// observers of the entity. Prefer [`AppContext::update_entity`], which ends the
    /// borrow when its closure returns. Use this when you need to hold the borrow across
    /// several statements.
    ///
    /// # Panics
    ///
    /// Panics in [`AsyncApp`] and [`AsyncWindowContext`], which can't hold a borrow of
    /// the app across an `.await`. Call `update` there instead.
    fn as_mut<'a, T>(&'a mut self, handle: &Entity<T>) -> GpuiBorrow<'a, T>
    where
        T: 'static;

    /// Reads the entity with a shared borrow, and returns the result of `read`.
    ///
    /// `read` also receives the app, so it can read other state at the same time.
    fn read_entity<T, R>(&self, handle: &Entity<T>, read: impl FnOnce(&T, &App) -> R) -> R
    where
        T: 'static;

    /// Tells GPUI that the entity with `entity_id` changed. Its observers run and windows
    /// that show it redraw.
    ///
    /// Inside an entity's update, [`Context::notify`] is more convenient.
    fn notify(&mut self, entity_id: EntityId);

    /// Emits `event` from `entity`.
    ///
    /// Subscribers registered with [`Context::subscribe`] or [`AsyncApp::subscribe`]
    /// receive it when the current update finishes. Inside an entity's update,
    /// [`Context::emit`] is more convenient.
    fn emit<EntityType, EventType>(&mut self, entity: &Entity<EntityType>, event: EventType)
    where
        EntityType: EventEmitter<EventType>,
        EventType: 'static;

    /// Runs `f` with the window's root view, the window and the app, and returns the
    /// result.
    ///
    /// # Errors
    ///
    /// Returns an error if the window has closed. Async contexts also return an error when
    /// the app is already borrowed.
    fn update_window<T, F>(&mut self, window: AnyWindowHandle, f: F) -> Result<T>
    where
        F: FnOnce(AnyView, &mut Window, &mut App) -> T;

    /// Runs `f` with the window that most recently rendered the entity `entity_id`, and
    /// returns the result.
    ///
    /// Returns `None` if no window has rendered the entity, or if that window isn't
    /// available.
    fn with_window<R>(
        &mut self,
        entity_id: EntityId,
        f: impl FnOnce(&mut Window, &mut App) -> R,
    ) -> Option<R>;

    /// Reads the root view of `window` with the app, and returns the result.
    ///
    /// # Errors
    ///
    /// Returns an error if the window has closed.
    fn read_window<T, R>(
        &self,
        window: &WindowHandle<T>,
        read: impl FnOnce(Entity<T>, &App) -> R,
    ) -> Result<R>
    where
        T: 'static;

    /// Runs `future` on the background executor and returns a task for its result.
    ///
    /// Native builds use worker threads; single-threaded web builds use the browser's main
    /// thread. The future and its result must be `Send + 'static`. No app context is supplied,
    /// so return to a foreground task to update entities. Dropping the task cancels the future.
    /// See [`BackgroundExecutor::spawn`] and the [executor concept](crate::_documentation::concepts::executors).
    fn background_spawn<R>(&self, future: impl Future<Output = R> + Send + 'static) -> Task<R>
    where
        R: Send + 'static;

    /// Calls `callback` with the global of type `G` and the app, and returns the result.
    ///
    /// # Panics
    ///
    /// Panics if no global of type `G` has been set.
    fn read_global<G, R>(&self, callback: impl FnOnce(&G, &App) -> R) -> R
    where
        G: Global;

    /// Keeps `entity` alive without a handle to it.
    ///
    /// The entity stays alive until the app is dropped, or until
    /// [`AppContext::remove_global_entity`] removes it and no other handle refers to it.
    fn insert_global_entity<T: 'static>(&mut self, entity: Entity<T>);

    /// Stops keeping `entity` alive. It's dropped once no other handle refers to it.
    fn remove_global_entity<T: 'static>(&mut self, entity: &Entity<T>);

    /// Returns the entities stored with [`AppContext::insert_global_entity`] whose type is
    /// `T`.
    fn global_entities<T: 'static>(&self) -> impl Iterator<Item = Entity<T>>;
}

/// A slot for an entity that doesn't exist yet.
///
/// Get one from [`AppContext::reserve_entity`], and pass it to
/// [`AppContext::insert_entity`] to create the entity.
pub struct Reservation<T>(pub(crate) Slot<T>);

impl<T: 'static> Reservation<T> {
    /// Returns the [`EntityId`] the entity will have once it's inserted.
    pub fn entity_id(&self) -> EntityId {
        self.0.entity_id()
    }
}

/// Operations that need a window.
///
/// Contexts that have a window implement this, including [`AsyncWindowContext`].
/// Import it along with [`AppContext`] from [`gpui::prelude`](crate::prelude).
pub trait VisualContext: AppContext {
    /// The return type of window operations.
    ///
    /// [`AsyncWindowContext`] uses [`Result`], because the window may have closed by the
    /// time the call runs.
    type Result<T>;

    /// Returns the handle of the window this context works in.
    fn window_handle(&self) -> AnyWindowHandle;

    /// Runs `update` with the entity, its window and a [`Context`] for the entity, and
    /// returns the result, wrapped in [`VisualContext::Result`](Self::Result).
    fn update_window_entity<T: 'static, R>(
        &mut self,
        entity: &Entity<T>,
        update: impl FnOnce(&mut T, &mut Window, &mut Context<T>) -> R,
    ) -> Self::Result<R>;

    /// Creates an entity with access to the window.
    ///
    /// `build_entity` receives the window and a [`Context`] for the new entity.
    fn new_window_entity<T: 'static>(
        &mut self,
        build_entity: impl FnOnce(&mut Window, &mut Context<T>) -> T,
    ) -> Self::Result<Entity<T>>;

    /// Replaces the root view of the window with the view that `build_view` creates, and
    /// returns a handle to the new view.
    fn replace_root_view<V>(
        &mut self,
        build_view: impl FnOnce(&mut Window, &mut Context<V>) -> V,
    ) -> Self::Result<Entity<V>>
    where
        V: 'static + Render;

    /// Moves keyboard focus to `entity`, using its [`Focusable::focus_handle`].
    fn focus<V>(&mut self, entity: &Entity<V>) -> Self::Result<()>
    where
        V: Focusable;
}

/// Declares that an entity type emits events of type `E`.
///
/// Implement it with an empty `impl`. Then emit events with [`Context::emit`], and
/// receive them with [`Context::subscribe`].
///
/// # Examples
///
/// ```
/// use gpui::{AppContext, Context, EventEmitter, TestApp};
///
/// struct Counter {
///     count: usize,
/// }
///
/// struct Incremented(usize);
///
/// impl EventEmitter<Incremented> for Counter {}
///
/// impl Counter {
///     fn add(&mut self, amount: usize, cx: &mut Context<Self>) {
///         self.count += amount;
///         cx.emit(Incremented(amount));
///         cx.notify();
///     }
/// }
///
/// let mut app = TestApp::new();
/// let (counter, total, _subscription) = app.update(|cx| {
///     let counter = cx.new(|_| Counter { count: 0 });
///     let total = cx.new(|_| 0_usize);
///     let total_for_subscriber = total.clone();
///     let subscription = cx.subscribe(&counter, move |_, event: &Incremented, cx| {
///         total_for_subscriber.update(cx, |total, _| *total += event.0);
///     });
///     (counter, total, subscription)
/// });
/// app.update(|cx| counter.update(cx, |counter, cx| counter.add(2, cx)));
/// app.read(|cx| assert_eq!(*total.read(cx), 2));
/// ```
pub trait EventEmitter<E: Any>: 'static {}

/// Helpers for changing globals from any context that can reach the [`App`].
///
/// Implemented for every type that implements `BorrowMut<App>`, including [`App`] and
/// [`Context`].
///
/// # Examples
///
/// ```
/// use gpui::{AppContext, BorrowAppContext, Global, TestApp};
///
/// struct Scale(f32);
///
/// impl Global for Scale {}
///
/// let mut app = TestApp::new();
/// app.update(|cx| {
///     cx.set_global(Scale(1.5));
///     cx.update_global(|scale: &mut Scale, _cx| scale.0 *= 2.0);
///     assert_eq!(cx.read_global(|scale: &Scale, _cx| scale.0), 3.0);
/// });
/// ```
pub trait BorrowAppContext {
    /// Sets the global of type `T` to `global`, replacing any previous value. Observers of
    /// the global are notified when the current update finishes.
    fn set_global<T: Global>(&mut self, global: T);
    /// Updates the global of type `G` with `f`, and returns the result of `f`.
    ///
    /// `f` also receives the context. While `f` runs, the global is taken out of the app,
    /// so reading it through the context panics. Change it only through the reference
    /// that `f` receives. Observers of the global are notified when the update ends.
    ///
    /// # Panics
    ///
    /// Panics if no global of type `G` has been set.
    fn update_global<G, R>(&mut self, f: impl FnOnce(&mut G, &mut Self) -> R) -> R
    where
        G: Global;
    /// Like [`BorrowAppContext::update_global`], but first sets `G::default()` if no
    /// global of type `G` has been set.
    fn update_default_global<G, R>(&mut self, f: impl FnOnce(&mut G, &mut Self) -> R) -> R
    where
        G: Global + Default;
}

impl<C> BorrowAppContext for C
where
    C: std::borrow::BorrowMut<App>,
{
    fn set_global<G: Global>(&mut self, global: G) {
        self.borrow_mut().set_global(global)
    }

    #[track_caller]
    fn update_global<G, R>(&mut self, f: impl FnOnce(&mut G, &mut Self) -> R) -> R
    where
        G: Global,
    {
        let mut global = self.borrow_mut().lease_global::<G>();
        let result = f(&mut global, self);
        self.borrow_mut().end_global_lease(global);
        result
    }

    fn update_default_global<G, R>(&mut self, f: impl FnOnce(&mut G, &mut Self) -> R) -> R
    where
        G: Global + Default,
    {
        self.borrow_mut().default_global::<G>();
        self.update_global(f)
    }
}

/// Describes the GPU that GPUI renders with.
///
/// Get it from [`Window::gpu_specs`], which returns `None` when the platform doesn't
/// report it.
#[derive(Default, Debug, serde::Serialize, serde::Deserialize, Clone)]
pub struct GpuSpecs {
    /// Whether the GPU is a software renderer, such as `llvmpipe`, running on the CPU.
    pub is_software_emulated: bool,
    /// The name of the device, as reported by the graphics backend.
    pub device_name: String,
    /// The name of the driver, as reported by the graphics backend.
    pub driver_name: String,
    /// Further information about the driver, as reported by the graphics backend.
    pub driver_info: String,
}
