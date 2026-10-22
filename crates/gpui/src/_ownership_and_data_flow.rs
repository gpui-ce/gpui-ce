//! The `App` owns every model and view, collectively called _entities_.
//! Entities use app services and interact with other entities through the app.
//!
//! `run` passes its callback an `App` reference for services such as opening
//! windows and presenting dialogs. Calling `cx.new` creates an entity owned by
//! the app.
//!
//! ```no_run
//! # use gpui::{App, AppContext, Application, Entity};
//! # struct Counter {
//! #     count: usize,
//! # }
//! gpui_platform::application().run(|cx: &mut App| {
//!     let _counter: Entity<Counter> = cx.new(|_cx| Counter { count: 0 });
//!     // ...
//! });
//! ```
//!
//! `cx.new` returns a typed _entity handle_, such as `Entity<Counter>`. The handle
//! identifies the app-owned object and maintains its reference count, but cannot
//! access its state alone.
//!
//! Cloning a handle increments the reference count. Dropping it decrements the
//! count, as with `Rc`. The app still owns the state, and accessing it requires
//! an `App` reference:
//!
//! ```no_run
//! # use gpui::{App, AppContext, Application, Context, Entity};
//! # struct Counter {
//! #     count: usize,
//! # }
//! gpui_platform::application().run(|cx: &mut App| {
//!     let counter: Entity<Counter> = cx.new(|_cx| Counter { count: 0 });
//!     // Call `update` to access the model's state.
//!     counter.update(cx, |counter: &mut Counter, _cx: &mut Context<Counter>| {
//!         counter.count += 1;
//!     });
//! });
//! ```
//!
//! `update` takes a context reference and passes a mutable reference to the
//! counter to its callback.
//!
//! The callback also receives a `Context<Counter>`, which wraps `App` with data
//! identifying the counter.
//!
//! `Context` provides app and entity services. Call `cx.notify()` to inform
//! observers that the entity's state changed.
//!
//! ```no_run
//! # use gpui::{App, AppContext, Application, Entity};
//! # struct Counter {
//! #     count: usize,
//! # }
//! gpui_platform::application().run(|cx: &mut App| {
//!     let counter: Entity<Counter> = cx.new(|_cx| Counter { count: 0 });
//!     counter.update(cx, |counter, cx| {
//!         counter.count += 1;
//!         cx.notify(); // Notify observers
//!     });
//! });
//! ```
//!
//! A second counter calls `observe` on its own `Context` to receive notifications
//! from the first and set its count to twice the first count.
//!
//! Detaching the returned `Subscription` keeps the observation active while both
//! counters exist. Store and drop the subscription to cancel it earlier.
//!
//! The `observe` callback receives a mutable reference to the observer and a
//! handle to the observed counter. Use `read` to access the observed state.
//!
//! ```no_run
//!  # use gpui::{App, AppContext, Application, Entity, prelude::*};
//!  # struct Counter {
//!  #     count: usize,
//!  # }
//!  gpui_platform::application().run(|cx: &mut App| {
//!      let first_counter: Entity<Counter> = cx.new(|_cx| Counter { count: 0 });
//!
//!      let second_counter = cx.new(|cx: &mut Context<Counter>| {
//!          // Note we can set up the callback before the Counter is even created!
//!          cx.observe(
//!              &first_counter,
//!              |second: &mut Counter, first: Entity<Counter>, cx| {
//!                  second.count = first.read(cx).count * 2;
//!              },
//!          )
//!          .detach();
//!
//!          Counter { count: 0 }
//!      });
//!
//!      first_counter.update(cx, |counter, cx| {
//!          counter.count += 1;
//!          cx.notify();
//!      });
//!
//!      assert_eq!(second_counter.read(cx).count, 2);
//!  });
//! ```
//!
//! `observe` and `notify` report state changes. `subscribe` and `emit` handle
//! typed events. The emitting object must implement `EventEmitter`.
//!
//! Define `CounterChangeEvent` and let `Counter` emit it:
//!
//! ```no_run
//! use gpui::EventEmitter;
//! # struct Counter {
//! #     count: usize,
//! # }
//! struct CounterChangeEvent {
//!     increment: usize,
//! }
//!
//! impl EventEmitter<CounterChangeEvent> for Counter {}
//! ```
//!
//! Replace the observation with a subscription. Each increment emits a
//! `CounterChangeEvent` containing the increase.
//!
//! ```no_run
//! # use gpui::{App, AppContext, Application, Context, Entity, EventEmitter};
//! # struct Counter {
//! #     count: usize,
//! # }
//! # struct CounterChangeEvent {
//! #     increment: usize,
//! # }
//! # impl EventEmitter<CounterChangeEvent> for Counter {}
//! gpui_platform::application().run(|cx: &mut App| {
//!     let first_counter: Entity<Counter> = cx.new(|_cx| Counter { count: 0 });
//!
//!     let second_counter = cx.new(|cx: &mut Context<Counter>| {
//!         // Note we can set up the callback before the Counter is even created!
//!         cx.subscribe(&first_counter, |second: &mut Counter, _first: Entity<Counter>, event, _cx| {
//!             second.count += event.increment * 2;
//!         })
//!         .detach();
//!
//!         Counter {
//!             count: first_counter.read(cx).count * 2,
//!         }
//!     });
//!
//!     first_counter.update(cx, |first, cx| {
//!         first.count += 2;
//!         cx.emit(CounterChangeEvent { increment: 2 });
//!         cx.notify();
//!     });
//!
//!     assert_eq!(second_counter.read(cx).count, 4);
//! });
//! ```
