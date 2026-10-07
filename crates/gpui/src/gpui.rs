#![doc = include_str!("../README.md")]
#![warn(missing_docs)]
#![allow(clippy::type_complexity)] // Not useful, GPUI makes heavy use of callbacks
#![allow(clippy::collapsible_else_if)] // False positives in platform specific code
#![allow(unused_mut)] // False positives in platform specific code

extern crate self as gpui;
#[macro_use]
mod action;
mod animated;
mod app;
pub mod app_borrow;

mod arena;
mod asset_cache;
mod assets;
mod bounds_tree;
mod color;
/// The default colors used by GPUI.
pub mod colors;
pub mod context;
#[cfg(feature = "profiler")]
mod debug_overlay;
mod element;
mod elements;
mod executor;
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
mod platform_scheduler;
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
#[expect(missing_docs)]
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
pub mod util;
mod view;
mod window;

#[cfg(any(test, feature = "test-support"))]
pub use proptest;

#[cfg(doc)]
pub mod _accessibility;
#[cfg(doc)]
pub mod _ownership_and_data_flow;

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
pub use context::AppContext;
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
pub(crate) use platform_scheduler::PlatformScheduler;
pub use resize::*;
pub use spring::*;

/// Defines a Criterion benchmark group for benchmarks annotated with [`gpui::bench`].
///
/// This mirrors `criterion::criterion_group!` so GPUI benchmark files can keep the
/// same shape as ordinary Criterion benchmarks.
///
/// [`gpui::bench`]: crate::bench
#[macro_export]
macro_rules! bench_group {
    ($($tokens:tt)*) => {
        criterion::criterion_group!($($tokens)*);
    };
}

/// Defines the entry point for GPUI Criterion benchmark groups.
///
/// This mirrors `criterion::criterion_main!` so GPUI benchmark files can keep the
/// same shape as ordinary Criterion benchmarks.
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
pub use style::*;
pub use style_transitions::*;
pub use styled::*;
pub use subscription::*;
pub use svg_renderer::*;
pub(crate) use tab_stop::*;
pub use taffy::{AvailableSpace, LayoutId};
#[cfg(any(test, feature = "test-support"))]
pub use test::*;
pub use text_system::*;
pub use transition::*;
pub use util::{FutureExt, TextRangeExt, Timeout};
pub use view::*;
pub use window::*;

#[cfg(not(target_family = "wasm"))]
pub use pollster::block_on;

use std::any::Any;
use taffy::TaffyLayoutEngine;

/// Returned by [reserve_entity](context::AppContextEntity::reserve_entity) to later be passed to [insert_entity](context::AppContextEntity::insert_entity).
/// Allows you to obtain the [EntityId] for a entity before it is created.
pub struct Reservation<T>(pub(crate) Slot<T>);

impl<T: 'static> Reservation<T> {
    /// Returns the [EntityId] that will be associated with the entity once it is inserted.
    pub fn entity_id(&self) -> EntityId {
        self.0.entity_id()
    }
}

/// This trait is used for the different visual contexts in GPUI that
/// require a window to be present.
pub trait VisualContext: context::AppContextWindow {
    /// The result type for window operations.
    type Result<T>;

    /// Returns the handle of the window associated with this context.
    fn window_handle(&self) -> AnyWindowHandle;

    /// Update a view with the given callback
    fn update_window_entity<T: 'static, R>(
        &mut self,
        entity: &Entity<T>,
        update: impl FnOnce(&mut T, &mut Window, &mut Context<T>) -> R,
    ) -> Self::Result<R>;

    /// Create a new entity, with access to `Window`.
    fn new_window_entity<T: 'static>(
        &mut self,
        build_entity: impl FnOnce(&mut Window, &mut Context<T>) -> T,
    ) -> Self::Result<Entity<T>>;

    /// Replace the root view of a window with a new view.
    fn replace_root_view<V>(
        &mut self,
        build_view: impl FnOnce(&mut Window, &mut Context<V>) -> V,
    ) -> Self::Result<Entity<V>>
    where
        V: 'static + Render;

    /// Focus a entity in the window, if it implements the [`Focusable`] trait.
    fn focus<V>(&mut self, entity: &Entity<V>) -> Self::Result<()>
    where
        V: Focusable;
}

/// A trait for tying together the types of a GPUI entity and the events it can
/// emit.
pub trait EventEmitter<E: Any>: 'static {}

/// A helper trait for auto-implementing certain methods on contexts that
/// can be used interchangeably.
#[deprecated(note = "migrating to entity-based globals")]
pub trait BorrowAppContext {
    /// Set a global value on the context.
    #[deprecated(note = "migrating to entity-based globals")]
    fn set_global<T: Global>(&mut self, global: T);

    /// Updates the global state of the given type.
    #[deprecated(note = "migrating to entity-based globals")]
    fn update_global<G, R>(&mut self, f: impl FnOnce(&mut G, &mut Self) -> R) -> R
    where
        G: Global;

    /// Updates the global state of the given type, creating a default if it didn't exist before.
    #[deprecated(note = "migrating to entity-based globals")]
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

/// Information about the GPU GPUI is running on.
#[derive(Default, Debug, serde::Serialize, serde::Deserialize, Clone)]
pub struct GpuSpecs {
    /// Whether the GPU is really a fake (like `llvmpipe`) running on the CPU.
    pub is_software_emulated: bool,
    /// The name of the device, as reported by Vulkan.
    pub device_name: String,
    /// The name of the driver, as reported by Vulkan.
    pub driver_name: String,
    /// Further information about the driver, as reported by Vulkan.
    pub driver_info: String,
}
