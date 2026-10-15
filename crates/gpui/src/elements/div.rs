//! [`Div`] is GPUI's general-purpose container for layout, styling, and input.
//!
//! A [`Div`] lays out its children with flexbox and can respond to mouse, keyboard, drag, and
//! accessibility events. Create one with [`div`]. Custom elements can reuse the same behavior through
//! [`Interactivity`], [`InteractiveElement`], and [`StatefulInteractiveElement`]. The
//! [elements concept](crate::_documentation::concepts::elements) explains when to use a div, and the
//! [input dispatch concept](crate::_documentation::concepts::input_dispatch) explains how events reach its handlers.
//!
//! # Example
//!
//! ```
//! # use gpui::{div, prelude::*, rgb};
//! let card = div()
//!     .id("card")
//!     .p_3()
//!     .bg(rgb(0xffffff))
//!     .hover(|style| style.bg(rgb(0xf3f4f6)))
//!     .on_click(|_, _, _| {})
//!     .child("Hello");
//! ```
//!
//! # Build your own element
//!
//! [`Interactivity`] holds listeners, styles, and state. The traits above expose it as a fluent API,
//! and [`Div`] combines it with a [`StyleRefinement`] and a list of children.

#[cfg(debug_assertions)]
use crate::{PaintQuad, TextAlign};

#[cfg(test)]
use crate::{
    AnyWindowHandle, Context, HighlightStyle, InputEvent, Keystroke, StyledText, TestAppContext,
    canvas, hsla, util::FluentBuilder,
};

#[cfg(test)]
use std::{cell::Cell, rc::Weak};

use crate::{
    Action, AnyDrag, AnyElement, AnyTooltip, AnyView, App, AppContext, Bounds, ClickEvent,
    CursorStyle, DispatchPhase, Display, Element, ElementId, Entity, EntityId, ExternalDragPayload,
    FocusHandle, Global, GlobalElementId, Hitbox, HitboxBehavior, HitboxId, InspectorElementId,
    IntoElement, IsZero, KeyContext, KeyDownEvent, KeyUpEvent, KeyboardButton, KeyboardClickEvent,
    LayoutId, ModifiersChangedEvent, MouseButton, MouseClickEvent, MouseDownEvent, MouseExitEvent,
    MouseMoveEvent, MousePressureEvent, MouseUpEvent, OngoingScroll, Overflow, ParentElement,
    PinchEvent, Pixels, Point, PointerChangeCause, Render, ScrollWheelEvent, SharedString, Size,
    Style, StyleRefinement, StyleTransitionContext, StyleTransitionState, StyleTransitions, Styled,
    Task, TooltipId, Visibility, Window, WindowControlArea, point, px, size,
};
use collections::HashMap;
use gpui_util::ResultExt;
use itertools::Either;
use refineable::Refineable;
use smallvec::SmallVec;
use std::{
    any::{Any, TypeId},
    cell::RefCell,
    cmp::Ordering,
    fmt::Debug,
    marker::PhantomData,
    mem,
    rc::Rc,
    sync::Arc,
    time::Duration,
};

use super::ImageCacheProvider;

mod inline;
pub(crate) use inline::InlineContent;
use inline::InlineDivFrameState;
#[cfg(feature = "stacker")]
type StackSafe<T> = stacksafe::StackSafe<T>;
#[cfg(not(feature = "stacker"))]
type StackSafe<T> = T;

const DRAG_THRESHOLD: f64 = 2.;
const DEFAULT_TOOLTIP_SHOW_DELAY: Duration = Duration::from_millis(500);
const HOVERABLE_TOOLTIP_HIDE_DELAY: Duration = Duration::from_millis(500);

/// A style that applies to an element while a named group of elements is hovered, pressed or receives a drag.
pub struct GroupStyle {
    /// The name that links the members of the group. Members must use the same name.
    pub group: SharedString,

    /// The style to apply to the element while the group condition holds.
    pub style: Box<StyleRefinement>,
}

/// A pointer move during a drag of type `T`, passed to [`InteractiveElement::on_drag_move`] listeners.
pub struct DragMoveEvent<T> {
    /// The mouse move event that produced this drag move.
    pub event: MouseMoveEvent,

    /// The bounds of the element that received the listener, in window coordinates.
    pub bounds: Bounds<Pixels>,
    drag: PhantomData<T>,
    dragged_item: Arc<dyn Any>,
}

impl<T: 'static> DragMoveEvent<T> {
    /// Returns the value of the active drag, cast to `T`.
    ///
    /// # Panics
    ///
    /// Panics if the active drag is not of type `T`. The listener runs only for drags of type `T`, so this is a
    /// bug in the code that started the drag.
    pub fn drag<'b>(&self, cx: &'b App) -> &'b T {
        cx.active_drag
            .as_ref()
            .and_then(|drag| drag.value.downcast_ref::<T>())
            .expect("DragMoveEvent is only valid when the stored active drag is of the same type.")
    }

    /// Returns the value that is being dragged, as a `dyn Any`. Downcast it to get the concrete type.
    pub fn dragged_item(&self) -> &dyn Any {
        self.dragged_item.as_ref()
    }
}

impl Interactivity {
    /// Create an `Interactivity`, capturing the caller location in debug mode.
    #[cfg(any(feature = "inspector", debug_assertions))]
    #[track_caller]
    pub fn new() -> Interactivity {
        Interactivity {
            source_location: Some(core::panic::Location::caller()),
            ..Default::default()
        }
    }

    /// Create an `Interactivity`, capturing the caller location in debug mode.
    #[cfg(not(any(feature = "inspector", debug_assertions)))]
    pub fn new() -> Interactivity {
        Interactivity::default()
    }

    /// Returns where this `Interactivity` was created, if the caller location was recorded.
    ///
    /// The location is recorded in debug builds and with the `inspector` feature, and is `None` otherwise.
    /// The inspector uses it to point at the code that created an element.
    pub fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        #[cfg(any(feature = "inspector", debug_assertions))]
        {
            self.source_location
        }

        #[cfg(not(any(feature = "inspector", debug_assertions)))]
        {
            None
        }
    }

    /// Runs `listener` when `button` is pressed over this element.
    ///
    /// The listener runs in the bubble phase, and only if the pointer is over the element's hitbox.
    /// This is the imperative equivalent of [`InteractiveElement::on_mouse_down`]. Prefer the fluent method
    /// when you are building an element in a single chain.
    pub fn on_mouse_down(
        &mut self,
        button: MouseButton,
        listener: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
    ) {
        self.mouse_down_listeners
            .push(Box::new(move |event, phase, hitbox, window, cx| {
                if phase == DispatchPhase::Bubble
                    && event.button == button
                    && hitbox.is_hovered(window)
                {
                    (listener)(event, window, cx)
                }
            }));
    }

    /// Runs `listener` for a press of any button over this element, during the capture phase.
    ///
    /// Use it to see a press before the element on top handles it. The listener runs only while the pointer is over
    /// the element. This is the imperative equivalent of [`InteractiveElement::capture_any_mouse_down`].
    pub fn capture_any_mouse_down(
        &mut self,
        listener: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
    ) {
        self.mouse_down_listeners
            .push(Box::new(move |event, phase, hitbox, window, cx| {
                if phase == DispatchPhase::Capture && hitbox.is_hovered(window) {
                    (listener)(event, window, cx)
                }
            }));
    }

    /// Runs `listener` for a press of any button over this element, during the bubble phase.
    ///
    /// This is the imperative equivalent of [`InteractiveElement::on_any_mouse_down`].
    pub fn on_any_mouse_down(
        &mut self,
        listener: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
    ) {
        self.mouse_down_listeners
            .push(Box::new(move |event, phase, hitbox, window, cx| {
                if phase == DispatchPhase::Bubble && hitbox.is_hovered(window) {
                    (listener)(event, window, cx)
                }
            }));
    }

    /// Runs `listener` for pressure changes over this element, during the bubble phase.
    ///
    /// Pressure events come from force-sensitive trackpads, which report them only on macOS today. The listener runs only while the pointer is over the element.
    /// This is the imperative equivalent of [`InteractiveElement::on_mouse_pressure`].
    pub fn on_mouse_pressure(
        &mut self,
        listener: impl Fn(&MousePressureEvent, &mut Window, &mut App) + 'static,
    ) {
        self.mouse_pressure_listeners
            .push(Box::new(move |event, phase, hitbox, window, cx| {
                if phase == DispatchPhase::Bubble && hitbox.is_hovered(window) {
                    (listener)(event, window, cx)
                }
            }));
    }

    /// Runs `listener` for pressure changes over this element, during the capture phase.
    ///
    /// The listener runs only while the pointer is over the element.
    /// This is the imperative equivalent of [`InteractiveElement::capture_mouse_pressure`].
    pub fn capture_mouse_pressure(
        &mut self,
        listener: impl Fn(&MousePressureEvent, &mut Window, &mut App) + 'static,
    ) {
        self.mouse_pressure_listeners
            .push(Box::new(move |event, phase, hitbox, window, cx| {
                if phase == DispatchPhase::Capture && hitbox.is_hovered(window) {
                    (listener)(event, window, cx)
                }
            }));
    }

    /// Runs `listener` when `button` is released over this element, during the bubble phase.
    ///
    /// The listener runs only while the pointer is over the element. This is the imperative equivalent of
    /// [`InteractiveElement::on_mouse_up`].
    pub fn on_mouse_up(
        &mut self,
        button: MouseButton,
        listener: impl Fn(&MouseUpEvent, &mut Window, &mut App) + 'static,
    ) {
        self.mouse_up_listeners
            .push(Box::new(move |event, phase, hitbox, window, cx| {
                if phase == DispatchPhase::Bubble
                    && event.button == button
                    && hitbox.is_hovered(window)
                {
                    (listener)(event, window, cx)
                }
            }));
    }

    /// Runs `listener` for a release of any button over this element, during the capture phase.
    ///
    /// The listener runs only while the pointer is over the element. This is the imperative equivalent of
    /// [`InteractiveElement::capture_any_mouse_up`].
    pub fn capture_any_mouse_up(
        &mut self,
        listener: impl Fn(&MouseUpEvent, &mut Window, &mut App) + 'static,
    ) {
        self.mouse_up_listeners
            .push(Box::new(move |event, phase, hitbox, window, cx| {
                if phase == DispatchPhase::Capture && hitbox.is_hovered(window) {
                    (listener)(event, window, cx)
                }
            }));
    }

    /// Runs `listener` for a release of any button over this element, during the bubble phase.
    ///
    /// The listener runs only while the pointer is over the element.
    pub fn on_any_mouse_up(
        &mut self,
        listener: impl Fn(&MouseUpEvent, &mut Window, &mut App) + 'static,
    ) {
        self.mouse_up_listeners
            .push(Box::new(move |event, phase, hitbox, window, cx| {
                if phase == DispatchPhase::Bubble && hitbox.is_hovered(window) {
                    (listener)(event, window, cx)
                }
            }));
    }

    /// Runs `listener` for a press of any button outside this element, during the capture phase.
    ///
    /// The listener does not run while a window prompt is open. Use it to close a popover when the user clicks elsewhere.
    /// This is the imperative equivalent of [`InteractiveElement::on_mouse_down_out`].
    pub fn on_mouse_down_out(
        &mut self,
        listener: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
    ) {
        self.mouse_down_listeners
            .push(Box::new(move |event, phase, hitbox, window, cx| {
                if phase == DispatchPhase::Capture
                    && !window.has_active_prompt()
                    && !hitbox.contains(&window.mouse_position())
                {
                    (listener)(event, window, cx)
                }
            }));
    }

    /// Runs `listener` for every mouse press, in both phases, whether or not the pointer is over the element.
    ///
    /// The listener receives the phase, the element's hitbox, and the event. Check the phase yourself.
    /// This is the imperative equivalent of [`InteractiveElement::on_mouse_down_all`].
    pub fn on_mouse_down_all(
        &mut self,
        listener: impl Fn(&MouseDownEvent, DispatchPhase, &Hitbox, &mut Window, &mut App) + 'static,
    ) {
        self.mouse_down_listeners.push(Box::new(listener));
    }

    /// Runs `listener` when `button` is released outside this element, during the capture phase.
    ///
    /// The listener runs only if the pointer is not over the element's hitbox at the time of the release.
    /// This is the imperative equivalent of [`InteractiveElement::on_mouse_up_out`].
    pub fn on_mouse_up_out(
        &mut self,
        button: MouseButton,
        listener: impl Fn(&MouseUpEvent, &mut Window, &mut App) + 'static,
    ) {
        self.mouse_up_listeners
            .push(Box::new(move |event, phase, hitbox, window, cx| {
                if phase == DispatchPhase::Capture
                    && event.button == button
                    && !hitbox.is_hovered(window)
                {
                    (listener)(event, window, cx);
                }
            }));
    }

    /// Runs `listener` for every mouse release, in both phases, whether or not the pointer is over the element.
    ///
    /// The listener receives the phase, the element's hitbox, and the event. Check the phase yourself.
    /// This is the imperative equivalent of [`InteractiveElement::on_mouse_up_all`].
    pub fn on_mouse_up_all(
        &mut self,
        listener: impl Fn(&MouseUpEvent, DispatchPhase, &Hitbox, &mut Window, &mut App) + 'static,
    ) {
        self.mouse_up_listeners.push(Box::new(listener));
    }

    /// Runs `listener` when the pointer moves over this element, during the bubble phase.
    ///
    /// The listener runs only while the pointer is over the element's hitbox. This is the imperative equivalent of
    /// [`InteractiveElement::on_mouse_move`].
    pub fn on_mouse_move(
        &mut self,
        listener: impl Fn(&MouseMoveEvent, &mut Window, &mut App) + 'static,
    ) {
        self.mouse_move_listeners
            .push(Box::new(move |event, phase, hitbox, window, cx| {
                if phase == DispatchPhase::Bubble && hitbox.is_hovered(window) {
                    (listener)(event, window, cx);
                }
            }));
    }

    /// Runs `listener` for every mouse move, whether or not the pointer is over the element.
    ///
    /// The listener receives the phase, the element's hitbox, and the event. Use it to track the pointer while a
    /// drag is in progress outside the element. This is the imperative equivalent of
    /// [`InteractiveElement::on_mouse_move_all`].
    pub fn on_mouse_move_all(
        &mut self,
        listener: impl Fn(&MouseMoveEvent, DispatchPhase, &Hitbox, &mut Window, &mut App) + 'static,
    ) {
        self.mouse_move_listeners.push(Box::new(listener));
    }

    /// Runs `listener` when the pointer leaves the window while it is over this element.
    ///
    /// This is the imperative equivalent of [`InteractiveElement::on_mouse_exit`].
    pub fn on_mouse_exit(
        &mut self,
        listener: impl Fn(&MouseExitEvent, &mut Window, &mut App) + 'static,
    ) {
        self.mouse_exit_listeners
            .push(Box::new(move |event, phase, hitbox, window, cx| {
                if phase == DispatchPhase::Bubble && hitbox.is_hovered(window) {
                    (listener)(event, window, cx);
                }
            }));
    }

    /// Runs `listener` for pointer moves during a drag of type `T`, in the capture phase.
    ///
    /// The listener runs for every move while a drag of type `T` is active, including moves outside this element.
    /// This is the imperative equivalent of [`InteractiveElement::on_drag_move`].
    pub fn on_drag_move<T>(
        &mut self,
        listener: impl Fn(&DragMoveEvent<T>, &mut Window, &mut App) + 'static,
    ) where
        T: 'static,
    {
        self.mouse_move_listeners
            .push(Box::new(move |event, phase, hitbox, window, cx| {
                if phase == DispatchPhase::Capture
                    && let Some(drag) = &cx.active_drag
                    && drag.value.as_ref().type_id() == TypeId::of::<T>()
                {
                    (listener)(
                        &DragMoveEvent {
                            event: event.clone(),
                            bounds: hitbox.bounds,
                            drag: PhantomData,
                            dragged_item: Arc::clone(&drag.value),
                        },
                        window,
                        cx,
                    );
                }
            }));
    }

    /// Runs `listener` for scroll wheel events at the pointer, during the bubble phase.
    ///
    /// The innermost element runs first. The event continues to ancestors unless a listener calls
    /// [`App::stop_propagation`], and elements behind an [`occlude`](InteractiveElement::occlude) hitbox don't receive it.
    /// This is the imperative equivalent of [`InteractiveElement::on_scroll_wheel`].
    pub fn on_scroll_wheel(
        &mut self,
        listener: impl Fn(&ScrollWheelEvent, &mut Window, &mut App) + 'static,
    ) {
        self.scroll_wheel_listeners
            .push(Box::new(move |event, phase, hitbox, window, cx| {
                if phase == DispatchPhase::Bubble && hitbox.should_handle_scroll(window) {
                    (listener)(event, window, cx);
                }
            }));
    }

    /// Runs `listener` for pinch gestures over this element, during the bubble phase.
    ///
    /// The listener runs only while the pointer is over the element's hitbox. This is the imperative equivalent of
    /// [`InteractiveElement::on_pinch`].
    pub fn on_pinch(&mut self, listener: impl Fn(&PinchEvent, &mut Window, &mut App) + 'static) {
        self.pinch_listeners
            .push(Box::new(move |event, phase, hitbox, window, cx| {
                if phase == DispatchPhase::Bubble && hitbox.is_hovered(window) {
                    (listener)(event, window, cx);
                }
            }));
    }

    /// Runs `listener` for pinch gestures during the capture phase, whether or not the pointer is over the element.
    ///
    /// This is the imperative equivalent of [`InteractiveElement::capture_pinch`].
    pub fn capture_pinch(
        &mut self,
        listener: impl Fn(&PinchEvent, &mut Window, &mut App) + 'static,
    ) {
        self.pinch_listeners
            .push(Box::new(move |event, phase, _hitbox, window, cx| {
                if phase == DispatchPhase::Capture {
                    (listener)(event, window, cx);
                } else {
                    cx.propagate();
                }
            }));
    }

    /// Runs `listener` when an action of type `A` is dispatched to this element, during the capture phase.
    ///
    /// The capture phase runs before the bubble phase. If the listener does not call [`App::stop_propagation`],
    /// dispatch continues to the bubble handlers. This is the imperative equivalent of
    /// [`InteractiveElement::capture_action`].
    pub fn capture_action<A: Action>(
        &mut self,
        listener: impl Fn(&A, &mut Window, &mut App) + 'static,
    ) {
        self.action_listeners.push((
            TypeId::of::<A>(),
            Box::new(move |action, phase, window, cx| {
                let action = action.downcast_ref().unwrap();
                if phase == DispatchPhase::Capture {
                    (listener)(action, window, cx)
                } else {
                    cx.propagate();
                }
            }),
        ));
    }

    /// Runs `listener` when an action of type `A` is dispatched along the focus path, during the bubble phase.
    ///
    /// Dispatch reaches this element when it is on the focus path. The listener receives the action and can
    /// stop propagation or call [`App::propagate`]. This is the imperative equivalent of
    /// [`InteractiveElement::on_action`].
    ///
    /// # Example
    ///
    /// ```standalone_crate
    /// use gpui::{actions, div, prelude::*, Context, Window};
    ///
    /// actions!(counter_example, [Increment]);
    ///
    /// struct Counter {
    ///     count: usize,
    /// }
    ///
    /// impl Render for Counter {
    ///     fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    ///         div().on_action(cx.listener(|counter: &mut Counter, _: &Increment, _window, cx| {
    ///             counter.count += 1;
    ///             cx.notify();
    ///         }))
    ///     }
    /// }
    /// ```
    #[track_caller]
    pub fn on_action<A: Action>(&mut self, listener: impl Fn(&A, &mut Window, &mut App) + 'static) {
        self.action_listeners.push((
            TypeId::of::<A>(),
            Box::new(move |action, phase, window, cx| {
                let action = action.downcast_ref().unwrap();
                if phase == DispatchPhase::Bubble {
                    (listener)(action, window, cx)
                }
            }),
        ));
    }

    /// Runs `listener` when an action of the same type as `action` is dispatched, during the bubble phase.
    ///
    /// Use this when the action type is only known at runtime, such as when a component accepts actions from its
    /// caller. The listener receives the `action` value passed here, not the dispatched action, so only its type is
    /// used for matching. This is the imperative equivalent of [`InteractiveElement::on_boxed_action`].
    pub fn on_boxed_action(
        &mut self,
        action: &dyn Action,
        listener: impl Fn(&dyn Action, &mut Window, &mut App) + 'static,
    ) {
        let action = action.boxed_clone();
        self.action_listeners.push((
            (*action).type_id(),
            Box::new(move |_, phase, window, cx| {
                if phase == DispatchPhase::Bubble {
                    (listener)(&*action, window, cx)
                }
            }),
        ));
    }

    /// Runs `listener` for key presses, during the bubble phase.
    ///
    /// The listener runs only if no action bound to the keystroke handled it. The element must be on the focus path.
    /// This is the imperative equivalent of [`InteractiveElement::on_key_down`].
    pub fn on_key_down(
        &mut self,
        listener: impl Fn(&KeyDownEvent, &mut Window, &mut App) + 'static,
    ) {
        self.key_down_listeners
            .push(Box::new(move |event, phase, window, cx| {
                if phase == DispatchPhase::Bubble {
                    (listener)(event, window, cx)
                }
            }));
    }

    /// Runs `listener` for key presses during the capture phase. Like [`on_key_down`](Self::on_key_down), it runs only
    /// if no action bound to the keystroke handled it.
    ///
    /// This is the imperative equivalent of [`InteractiveElement::capture_key_down`].
    pub fn capture_key_down(
        &mut self,
        listener: impl Fn(&KeyDownEvent, &mut Window, &mut App) + 'static,
    ) {
        self.key_down_listeners
            .push(Box::new(move |event, phase, window, cx| {
                if phase == DispatchPhase::Capture {
                    listener(event, window, cx)
                }
            }));
    }

    /// Runs `listener` for key releases, during the bubble phase.
    ///
    /// This is the imperative equivalent of [`InteractiveElement::on_key_up`].
    pub fn on_key_up(&mut self, listener: impl Fn(&KeyUpEvent, &mut Window, &mut App) + 'static) {
        self.key_up_listeners
            .push(Box::new(move |event, phase, window, cx| {
                if phase == DispatchPhase::Bubble {
                    listener(event, window, cx)
                }
            }));
    }

    /// Runs `listener` for key releases during the capture phase.
    ///
    /// This is the imperative equivalent of [`InteractiveElement::capture_key_up`].
    pub fn capture_key_up(
        &mut self,
        listener: impl Fn(&KeyUpEvent, &mut Window, &mut App) + 'static,
    ) {
        self.key_up_listeners
            .push(Box::new(move |event, phase, window, cx| {
                if phase == DispatchPhase::Capture {
                    listener(event, window, cx)
                }
            }));
    }

    /// Runs `listener` when the set of pressed modifier keys changes.
    ///
    /// The event is delivered along the focus path, from the focused element upward. This is the imperative equivalent of
    /// [`InteractiveElement::on_modifiers_changed`].
    pub fn on_modifiers_changed(
        &mut self,
        listener: impl Fn(&ModifiersChangedEvent, &mut Window, &mut App) + 'static,
    ) {
        self.modifiers_changed_listeners
            .push(Box::new(move |event, window, cx| {
                listener(event, window, cx)
            }));
    }

    /// Runs `listener` when a drag of type `T` is released over this element.
    ///
    /// The drop runs whether or not the drag started on this element. Use [`InteractiveElement::can_drop`] or a
    /// [`DropListener`] to refuse drops. This is the imperative equivalent of [`InteractiveElement::on_drop`].
    pub fn on_drop<T: 'static>(&mut self, listener: impl Fn(&T, &mut Window, &mut App) + 'static) {
        let type_id = TypeId::of::<T>();
        let on_drop = Box::new(
            move |dragged_any: &dyn Any, _evt: &MouseUpEvent, window: &mut Window, cx: &mut App| {
                listener(dragged_any.downcast_ref().unwrap(), window, cx);
            },
        );
        // Find a pre-existing DropListener for the provided type T, or create a new one if it does not yet exist.
        match self.get_mut_drop_listener(type_id) {
            Some((_, drop_listener)) => {
                drop_listener.on_drop = Some(on_drop);
            }
            None => {
                self.drop_listeners.push((
                    TypeId::of::<T>(),
                    DropListener::<dyn Any> {
                        can_drop: None,
                        on_drag_over: None,
                        on_drop: Some(on_drop),
                    },
                ));
            }
        }
    }

    /// Attaches a [`DropListener`] that handles drags of type `T` over this element.
    ///
    /// This is the imperative equivalent of [`InteractiveElement::on_drop_alt`].
    pub fn on_drop_alt<T: 'static>(&mut self, listener: DropListener<T>) {
        self.drop_listeners
            .push((TypeId::of::<T>(), listener.into_any()));
    }

    /// Returns the drop listener registered for `type_id`, if there is one.
    fn get_mut_drop_listener(
        &mut self,
        type_id: TypeId,
    ) -> Option<&mut (TypeId, DropListener<dyn Any>)> {
        let mut iter = self.drop_listeners.iter_mut();
        iter.find(|(existing_type_id, _listener)| *existing_type_id == type_id)
    }

    /// Sets a predicate that decides whether this element accepts the value being dragged.
    ///
    /// The predicate runs only while the pointer is over the element, and it receives the dragged value.
    /// This is the imperative equivalent of [`InteractiveElement::can_drop`].
    pub fn can_drop(
        &mut self,
        predicate: impl Fn(&dyn Any, &mut Window, &mut App) -> bool + 'static,
    ) {
        self.can_drop_predicate = Some(Box::new(move |value, hitbox, window, cx| {
            hitbox.is_hovered(window) && predicate(value, window, cx)
        }));
    }

    /// Sets a style function that applies while a dragged value is over this element.
    ///
    /// The style applies only if this element has a drop listener for the dragged type. Use
    /// [`DropListener::on_drag_over`] when the style depends on the dragged type. This is the imperative equivalent of
    /// [`InteractiveElement::on_drag_over_any`].
    pub fn on_drag_over_any(
        &mut self,
        predicate: impl Fn(StyleRefinement, &dyn Any, &mut Window, &mut App) -> StyleRefinement
        + 'static,
    ) {
        self.drag_over_style = Some(Box::new(move |value, window, app| {
            predicate(StyleRefinement::default(), value, window, app)
        }));
    }

    /// Runs `listener` when this element is clicked, or when it is activated from the keyboard.
    ///
    /// A mouse click fires when the left button is pressed and released over the element without the pointer
    /// moving far enough to start a drag. A keyboard click fires when Enter or Space is pressed and released while
    /// the element is focused and no other key is pressed in between. The listener receives a [`ClickEvent`].
    /// This is the imperative equivalent of [`StatefulInteractiveElement::on_click`]. It requires an element id.
    pub fn on_click(&mut self, listener: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static)
    where
        Self: Sized,
    {
        self.click_listeners.push(Rc::new(move |event, window, cx| {
            listener(event, window, cx)
        }));
    }

    /// Runs `listener` for clicks with a button other than the left one.
    ///
    /// This is the imperative equivalent of [`StatefulInteractiveElement::on_aux_click`]. It requires an element id.
    pub fn on_aux_click(&mut self, listener: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static)
    where
        Self: Sized,
    {
        self.aux_click_listeners
            .push(Rc::new(move |event, window, cx| {
                listener(event, window, cx)
            }));
    }

    /// Starts a drag of `value` when the left button is pressed on this element and the pointer moves.
    ///
    /// The `constructor` runs when the drag starts. It receives the value and the cursor offset from the element's
    /// origin, and returns the view that is drawn under the pointer. Any dragged value must be `'static`.
    /// This is the imperative equivalent of [`StatefulInteractiveElement::on_drag`].
    pub fn on_drag<T, W>(
        &mut self,
        value: T,
        constructor: impl Fn(&T, Point<Pixels>, &mut Window, &mut App) -> Entity<W> + 'static,
    ) where
        Self: Sized,
        T: 'static,
        W: 'static + Render,
    {
        let value = Arc::new(value);
        self.on_drag_alt(move |cursor_offset, cursor_style, window, cx| {
            let view = constructor(value.as_ref(), cursor_offset, window, cx);
            AnyDrag {
                view: view.into(),
                value: value.clone(),
                cursor_offset,
                cursor_style,
                external_payload_source: None,
            }
        });
    }

    /// Sets the function that starts a drag from this element.
    ///
    /// The function receives the cursor offset and the cursor style, and returns an [`AnyDrag`]. Use
    /// [`Interactivity::on_drag`] unless you need to build the [`AnyDrag`] yourself.
    /// This is the imperative equivalent of [`StatefulInteractiveElement::on_drag_alt`].
    pub fn on_drag_alt<F>(&mut self, listener: F)
    where
        Self: Sized,
        F: 'static + Fn(Point<Pixels>, Option<CursorStyle>, &mut Window, &mut App) -> AnyDrag,
    {
        debug_assert!(
            self.drag_listener.is_none(),
            "calling on_drag more than once on the same element is not supported"
        );
        self.drag_listener = Some(Box::new(listener));
    }

    /// Registers a resolver for the data to offer the platform when a drag from this element leaves the window.
    ///
    /// The resolver runs at most once for each drag, when the pointer leaves the window. It must be called after
    /// [`Interactivity::on_drag`] and with the same dragged type `T`. This is the imperative equivalent of
    /// [`StatefulInteractiveElement::external_drag_payload`].
    ///
    /// # Panics
    ///
    /// Panics in debug builds if called before [`Interactivity::on_drag_alt`] or [`Interactivity::on_drag`]. If called
    /// twice on the same element, it panics in debug builds when a drag starts.
    pub fn external_drag_payload<T>(
        &mut self,
        resolver: impl Fn(&T, &mut Window, &mut App) -> Option<ExternalDragPayload> + 'static,
    ) where
        Self: Sized,
        T: 'static,
    {
        let Some(drag_listener) = self.drag_listener.take() else {
            debug_assert!(false, "external_drag_payload must be called after on_drag");
            return;
        };

        let resolver = Rc::new(resolver);
        self.on_drag_alt(move |cursor_offset, cursor_style, window, cx| {
            let mut any_drag = drag_listener(cursor_offset, cursor_style, window, cx);
            debug_assert!(
                any_drag.external_payload_source.is_none(),
                "calling external_drag_payload more than once on the same element is not supported"
            );
            any_drag.external_payload_source = Some(Box::new({
                let value = any_drag.value.clone();
                let resolver = resolver.clone();
                move |window, cx| {
                    let value_ref = value
                        .downcast_ref::<T>()
                        .expect("drag value type does not match external payload type");
                    resolver(value_ref, window, cx)
                }
            }));
            any_drag
        });
    }

    /// Runs `listener` with `true` when the pointer enters this element and with `false` when it leaves.
    ///
    /// Transitions caused by layout changes under a stationary pointer also run the listener. The listener does not
    /// run while a drag is in progress. This is the imperative equivalent of [`StatefulInteractiveElement::on_hover`].
    pub fn on_hover(&mut self, listener: impl Fn(&bool, &mut Window, &mut App) + 'static)
    where
        Self: Sized,
    {
        self.on_hover_region(
            |_, _| Some(()),
            move |value, window, cx| listener(&value.is_some(), window, cx),
        );
    }

    /// Resolves this element's cursor from pointer position and bounds.
    /// Return `None` to defer to another cursor request.
    pub fn cursor_with(
        &mut self,
        resolver: impl Fn(Point<Pixels>, Bounds<Pixels>) -> Option<CursorStyle> + 'static,
    ) where
        Self: Sized,
    {
        self.cursor_style_resolver = Some(Rc::new(resolver));
    }

    /// Calls `listener` when the projected pointer value changes.
    /// The projection receives window coordinates and element bounds; return `None` outside active regions.
    /// Layout changes and occlusion are observed independently of event propagation.
    /// Keep listener order stable across renders.
    pub fn on_hover_region<R: PartialEq + 'static>(
        &mut self,
        project: impl Fn(Point<Pixels>, Bounds<Pixels>) -> Option<R> + 'static,
        listener: impl Fn(&Option<R>, &mut Window, &mut App) + 'static,
    ) where
        Self: Sized,
    {
        self.hover_region_listeners
            .push(Box::new(TypedHoverRegionListener {
                project: Rc::new(project),
                callback: Rc::new(listener),
            }));
    }

    /// Sets a function that builds a tooltip view shown while the pointer rests over this element.
    ///
    /// The tooltip is hidden as soon as the pointer leaves the element, so the user cannot move onto it. Use
    /// [`Interactivity::hoverable_tooltip`] for a tooltip that stays open. This is the imperative equivalent of
    /// [`StatefulInteractiveElement::tooltip`].
    ///
    /// # Panics
    ///
    /// Panics in debug builds if a tooltip is already set on this element.
    pub fn tooltip(&mut self, build_tooltip: impl Fn(&mut Window, &mut App) -> AnyView + 'static)
    where
        Self: Sized,
    {
        debug_assert!(
            self.tooltip_builder.is_none(),
            "calling tooltip more than once on the same element is not supported"
        );
        self.tooltip_builder = Some(TooltipBuilder {
            build: Rc::new(build_tooltip),
            hoverable: false,
        });
    }

    /// Sets a function that builds a tooltip view that stays open while the pointer is over it.
    ///
    /// This is the same as [`Interactivity::tooltip`], except the pointer can move into the tooltip without closing it.
    /// This is the imperative equivalent of [`StatefulInteractiveElement::hoverable_tooltip`].
    ///
    /// # Panics
    ///
    /// Panics in debug builds if a tooltip is already set on this element.
    pub fn hoverable_tooltip(
        &mut self,
        build_tooltip: impl Fn(&mut Window, &mut App) -> AnyView + 'static,
    ) where
        Self: Sized,
    {
        debug_assert!(
            self.tooltip_builder.is_none(),
            "calling tooltip more than once on the same element is not supported"
        );
        self.tooltip_builder = Some(TooltipBuilder {
            build: Rc::new(build_tooltip),
            hoverable: true,
        });
    }

    /// Sets how long the pointer must rest over this element before its tooltip appears.
    ///
    /// The default is 500 milliseconds. This is the imperative equivalent of
    /// [`StatefulInteractiveElement::tooltip_show_delay`].
    pub fn tooltip_show_delay(&mut self, delay: Duration) {
        self.tooltip_show_delay = Some(delay);
    }

    /// Stops the mouse from reaching elements behind this element.
    ///
    /// Use [`Interactivity::block_mouse_except_scroll`] for most cases, because it still lets scroll events reach
    /// the elements behind. This is the imperative equivalent of [`InteractiveElement::occlude`].
    pub fn occlude_mouse(&mut self) {
        self.hitbox_behavior = HitboxBehavior::BlockMouse;
    }

    /// Marks this element's bounds as a window control area, such as a title bar button.
    ///
    /// The platform uses `area` to decide what clicks and drags here do, such as moving or closing the window.
    /// This is the imperative equivalent of [`InteractiveElement::window_control_area`].
    pub fn window_control_area(&mut self, area: WindowControlArea) {
        self.window_control = Some(area);
    }

    /// Stops mouse interactions from reaching elements behind this element, but lets scroll events through.
    ///
    /// This is the imperative equivalent of [`InteractiveElement::block_mouse_except_scroll`].
    pub fn block_mouse_except_scroll(&mut self) {
        self.hitbox_behavior = HitboxBehavior::BlockMouseExceptScroll;
    }

    fn has_pinch_listeners(&self) -> bool {
        !self.pinch_listeners.is_empty()
    }

    /// Runs `listener` during prepaint, once the element's bounds and content size are known.
    ///
    /// The listener is consumed each frame, so register it again on every render. This is the imperative equivalent of
    /// [`StatefulInteractiveElement::on_prepaint`].
    pub fn on_prepaint(
        &mut self,
        listener: impl Fn(&InteractivityPrepaint, &mut Window, &mut App) + 'static,
    ) where
        Self: Sized,
    {
        self.prepaint_listeners
            .push(Box::new(move |payload, window, cx| {
                listener(payload, window, cx)
            }));
    }
}

/// An element that can respond to the mouse and keyboard, style itself on state, and take focus.
///
/// Implement this trait with [`Element`] to give a custom element the event handlers and styles of
/// [`Div`]. Return its [`Interactivity`] from [`interactivity`](Self::interactivity); the default
/// methods update that value and return `self`, so calls chain.
///
/// Most handlers need no id. The exceptions are the ones that keep state between frames, such as clicks, starting a
/// drag, scrolling and tooltips. Those live on [`StatefulInteractiveElement`], and are reached by calling
/// [`id`](Self::id) first.
///
/// Event handlers run in two phases. Capture walks from the back of the window to the front, then
/// bubble walks from the front to the back. Methods named `capture_*` use capture; most other
/// methods use bubble. Each `*_out` and `*_all` variant documents its own hitbox rule. See the
/// [input dispatch concept](crate::_documentation::concepts::input_dispatch) for the full path.
///
/// # Example
///
/// ```
/// # use gpui::{div, prelude::*, rgb};
/// let toggle = div()
///     .id("toggle")
///     .px_2()
///     .bg(rgb(0xe5e7eb))
///     .hover(|style| style.bg(rgb(0xd1d5db)))
///     .on_click(|_, _, _| {});
/// ```
pub trait InteractiveElement: Sized {
    /// Returns the [`Interactivity`] that stores this element's listeners, styles and state.
    ///
    /// Every other method on this trait is implemented by calling this one, so implement it by returning a field.
    fn interactivity(&mut self) -> &mut Interactivity;

    /// Animates changes to this element's style properties over time.
    ///
    /// `build` receives a [`StyleTransitions`] and returns it with the properties to animate and their timing. When a
    /// style property changes between frames, such as a width set by a hover style, the change plays as a transition
    /// instead of jumping.
    ///
    /// If the element has no id, GPUI derives one from the call site. Give repeated elements built at the same
    /// call site an explicit [`id`](Self::id), so that each one keeps its own motion state.
    #[track_caller]
    fn transitions(mut self, build: impl FnOnce(StyleTransitions) -> StyleTransitions) -> Self {
        let transitions = build(StyleTransitions::new());
        let interactivity = self.interactivity();
        interactivity
            .element_id
            .get_or_insert_with(|| ElementId::CodeLocation(*core::panic::Location::caller()));
        interactivity.style_transitions = Some(Box::new(transitions));
        self
    }

    /// Adds this element to a named group, so that group-level styles can react to any member.
    ///
    /// Members of the group share a name. Other elements can then use [`group_hover`](Self::group_hover),
    /// [`group_active`](StatefulInteractiveElement::group_active) or [`group_drag_over`](Self::group_drag_over)
    /// with that name to style themselves when any member is hovered, pressed or has a dragged value over it.
    /// The group name must match exactly.
    fn group(mut self, group: impl Into<SharedString>) -> Self {
        self.interactivity().group = Some(group.into());
        self
    }

    /// Gives this element an [`ElementId`], which turns it into a [`Stateful`] element.
    ///
    /// An id is required for anything that keeps state between frames: clicks, scrolling, drag and drop, active
    /// styles, and tooltips. GPUI matches the id against the previous frame to find that state, so the id must
    /// be stable across renders and unique among siblings. Use a value that identifies the content, such as a
    /// database id, for elements that can move. Ids may be strings, numbers, or tuples of these.
    ///
    /// # Example
    ///
    /// ```
    /// # use gpui::{div, prelude::*};
    /// let row = div()
    ///     .id(("row", 3usize))
    ///     .on_click(|_, _, _| {});
    /// ```
    fn id(mut self, id: impl Into<ElementId>) -> Stateful<Self> {
        self.interactivity().element_id = Some(id.into());

        Stateful { element: self }
    }

    /// Makes this element focusable with the given [`FocusHandle`], and styles it with the focus styles when focused.
    ///
    /// The element joins the focus path when the handle is focused, so keyboard input reaches it. Focus styles set with
    /// [`focus`](Self::focus), [`in_focus`](Self::in_focus) and [`focus_visible`](Self::focus_visible) apply only to
    /// elements that track a handle. Use [`focusable`](StatefulInteractiveElement::focusable) when you don't need to
    /// hold the handle. See the [focus guide](crate::_documentation::guides::focus).
    fn track_focus(mut self, focus_handle: &FocusHandle) -> Self {
        self.interactivity().focusable = true;
        self.interactivity().tracked_focus_handle = Some(focus_handle.clone());
        self
    }

    /// Sets whether this element is reached by Tab navigation.
    ///
    /// When `false`, the element keeps its place in the tab order, but [`Window::focus_next`] and
    /// [`Window::focus_prev`] skip it. Use it on a container that should take focus only to pass it on to its first
    /// tab stop. This works with [`tab_index`](Self::tab_index) and [`tab_group`](Self::tab_group).
    fn tab_stop(mut self, tab_stop: bool) -> Self {
        self.interactivity().tab_stop = tab_stop;
        self
    }

    /// Sets this element's position in the tab order and makes it a tab stop.
    ///
    /// Elements with lower indices are visited first. Elements with the same index, or no index, keep their order in the
    /// tree. It also makes the element focusable, which needs either an id or a focus handle from [`track_focus`](Self::track_focus).
    /// Use it sparingly: a tab group is usually better than numbering every element in a window. See
    /// [`tab_group`](Self::tab_group).
    fn tab_index(mut self, index: isize) -> Self {
        self.interactivity().focusable = true;
        self.interactivity().tab_index = Some(index);
        self.interactivity().tab_stop = true;
        self
    }

    /// Makes this element a tab group, so that its children are ordered within the group.
    ///
    /// A tab group takes its own position in the tab order, from its [`tab_index`](Self::tab_index), which defaults to 0.
    /// Its children start again at index 0, so you can reorder the stops inside the group without renumbering the rest
    /// of the window.
    fn tab_group(mut self) -> Self {
        self.interactivity().tab_group = true;
        if self.interactivity().tab_index.is_none() {
            self.interactivity().tab_index = Some(0);
        }
        self
    }

    /// Sets the key context for this element, which selects the keybindings that apply inside it.
    ///
    /// A binding that names a context applies only when an element with that context is on the focus path. Use a
    /// short name that matches the bindings, such as `"Editor"`. If the value cannot be converted to a
    /// [`KeyContext`], the error is logged and the element keeps no context.
    fn key_context<C, E>(mut self, key_context: C) -> Self
    where
        C: TryInto<KeyContext, Error = E>,
        E: std::fmt::Display,
    {
        if let Some(key_context) = key_context.try_into().log_err() {
            self.interactivity().key_context = Some(key_context);
        }
        self
    }

    /// Applies `f`'s style to this element while the pointer is over it.
    ///
    /// The style applies only while the element is hovered, not during a drag, and not after keyboard input. Hover
    /// is recalculated each frame, so the style follows the pointer without any state in your code.
    ///
    /// # Example
    ///
    /// ```
    /// # use gpui::{div, prelude::*, rgb};
    /// let button = div()
    ///     .id("save")
    ///     .bg(rgb(0x2563eb))
    ///     .hover(|style| style.bg(rgb(0x1d4ed8)));
    /// ```
    fn hover(mut self, f: impl FnOnce(StyleRefinement) -> StyleRefinement) -> Self {
        debug_assert!(
            self.interactivity().hover_style.is_none(),
            "hover style already set"
        );
        self.interactivity().hover_style = Some(Box::new(f(StyleRefinement::default())));
        self
    }

    /// Applies `f`'s style to this element while any member of `group_name` is hovered.
    ///
    /// Set the group with [`group`](Self::group) on the parent. The style does not apply during a drag or after
    /// keyboard input.
    fn group_hover(
        mut self,
        group_name: impl Into<SharedString>,
        f: impl FnOnce(StyleRefinement) -> StyleRefinement,
    ) -> Self {
        self.interactivity().group_hover_style = Some(GroupStyle {
            group: group_name.into(),
            style: Box::new(f(StyleRefinement::default())),
        });
        self
    }

    /// Runs `listener` when `button` is pressed over this element, during the bubble phase.
    ///
    /// The listener runs only if the pointer is over the element's hitbox. It receives the [`MouseDownEvent`], the window
    /// and the app. The fluent equivalent of [`Interactivity::on_mouse_down`]. Use
    /// [`Context::listener`](crate::Context::listener) to call into a view.
    fn on_mouse_down(
        mut self,
        button: MouseButton,
        listener: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.interactivity().on_mouse_down(button, listener);
        self
    }

    #[cfg(any(test, feature = "test-support"))]
    /// Sets a name for this element that tests can use to find its bounds.
    ///
    /// Pass the name to [`VisualTestContext::debug_bounds`](crate::VisualTestContext::debug_bounds) to get the bounds of
    /// the element in a test. This method is available only with the `test-support` feature or in tests; in other
    /// builds it is a no-op.
    fn debug_selector(mut self, f: impl FnOnce() -> String) -> Self {
        self.interactivity().debug_selector = Some(f());
        self
    }

    #[cfg(not(any(test, feature = "test-support")))]
    /// Does nothing in this build. The debug selector is available only in tests and with `test-support`.
    #[inline]
    fn debug_selector(self, _: impl FnOnce() -> String) -> Self {
        self
    }

    /// Runs `listener` for a press of any button over this element, during the capture phase.
    ///
    /// Use it to see a press before the element on top of this one handles it. The fluent equivalent of
    /// [`Interactivity::capture_any_mouse_down`].
    fn capture_any_mouse_down(
        mut self,
        listener: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.interactivity().capture_any_mouse_down(listener);
        self
    }

    /// Runs `listener` for a press of any button over this element, during the bubble phase.
    ///
    /// The fluent equivalent of [`Interactivity::on_any_mouse_down`].
    fn on_any_mouse_down(
        mut self,
        listener: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.interactivity().on_any_mouse_down(listener);
        self
    }

    /// Runs `listener` when `button` is released over this element, during the bubble phase.
    ///
    /// The listener runs only if the pointer is over the element's hitbox. The fluent equivalent of
    /// [`Interactivity::on_mouse_up`].
    fn on_mouse_up(
        mut self,
        button: MouseButton,
        listener: impl Fn(&MouseUpEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.interactivity().on_mouse_up(button, listener);
        self
    }

    /// Runs `listener` for a release of any button over this element, during the capture phase.
    ///
    /// The fluent equivalent of [`Interactivity::capture_any_mouse_up`].
    fn capture_any_mouse_up(
        mut self,
        listener: impl Fn(&MouseUpEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.interactivity().capture_any_mouse_up(listener);
        self
    }

    /// Runs `listener` for pressure changes over this element, during the bubble phase.
    ///
    /// The fluent equivalent of [`Interactivity::on_mouse_pressure`].
    fn on_mouse_pressure(
        mut self,
        listener: impl Fn(&MousePressureEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.interactivity().on_mouse_pressure(listener);
        self
    }

    /// Runs `listener` for pressure changes over this element, during the capture phase.
    ///
    /// The fluent equivalent of [`Interactivity::capture_mouse_pressure`].
    fn capture_mouse_pressure(
        mut self,
        listener: impl Fn(&MousePressureEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.interactivity().capture_mouse_pressure(listener);
        self
    }

    /// Runs `listener` for a press of any button outside this element, during the capture phase.
    ///
    /// Use it to dismiss a popover or menu when the user clicks elsewhere. It does not run while a window prompt is
    /// open. The fluent equivalent of [`Interactivity::on_mouse_down_out`].
    fn on_mouse_down_out(
        mut self,
        listener: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.interactivity().on_mouse_down_out(listener);
        self
    }

    /// Runs `listener` for every mouse press, in both phases, whether or not the pointer is over the element.
    ///
    /// The listener receives the phase, the element's hitbox, and the event. The fluent equivalent of
    /// [`Interactivity::on_mouse_down_all`].
    fn on_mouse_down_all(
        mut self,
        listener: impl Fn(&MouseDownEvent, DispatchPhase, &Hitbox, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.interactivity().on_mouse_down_all(listener);
        self
    }

    /// Runs `listener` when `button` is released outside this element, during the capture phase.
    ///
    /// The listener runs only if the pointer is not over the element's hitbox at the release. The fluent equivalent of
    /// [`Interactivity::on_mouse_up_out`].
    fn on_mouse_up_out(
        mut self,
        button: MouseButton,
        listener: impl Fn(&MouseUpEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.interactivity().on_mouse_up_out(button, listener);
        self
    }

    /// Runs `listener` for every mouse release, in both phases, whether or not the pointer is over the element.
    ///
    /// The listener receives the phase, the element's hitbox, and the event. The fluent equivalent of
    /// [`Interactivity::on_mouse_up_all`].
    fn on_mouse_up_all(
        mut self,
        listener: impl Fn(&MouseUpEvent, DispatchPhase, &Hitbox, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.interactivity().on_mouse_up_all(listener);
        self
    }

    /// Runs `listener` when the pointer moves over this element, during the bubble phase.
    ///
    /// The fluent equivalent of [`Interactivity::on_mouse_move`].
    fn on_mouse_move(
        mut self,
        listener: impl Fn(&MouseMoveEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.interactivity().on_mouse_move(listener);
        self
    }

    /// Runs `listener` for every mouse move, whether or not the pointer is over the element.
    ///
    /// The listener receives the phase, the element's hitbox, and the event. The fluent equivalent of
    /// [`Interactivity::on_mouse_move_all`].
    fn on_mouse_move_all(
        mut self,
        listener: impl Fn(&MouseMoveEvent, DispatchPhase, &Hitbox, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.interactivity().on_mouse_move_all(listener);
        self
    }

    /// Runs `listener` when the pointer leaves the window while it is over this element.
    ///
    /// The fluent equivalent of [`Interactivity::on_mouse_exit`].
    fn on_mouse_exit(
        mut self,
        listener: impl Fn(&MouseExitEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.interactivity().on_mouse_exit(listener);
        self
    }

    /// Runs `listener` for pointer moves during any drag of type `T`, wherever the drag started.
    ///
    /// The listener runs for every move while the drag is active, even outside this element. This is the right
    /// hook for drags that don't drop anywhere, such as resizing a splitter. The fluent equivalent of
    /// [`Interactivity::on_drag_move`].
    fn on_drag_move<T: 'static>(
        mut self,
        listener: impl Fn(&DragMoveEvent<T>, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.interactivity().on_drag_move(listener);
        self
    }

    /// Runs `listener` for scroll wheel events that this element should handle, during the bubble phase.
    ///
    /// The innermost element runs first, and the event continues to ancestors unless a listener calls
    /// [`App::stop_propagation`]. The fluent equivalent of
    /// [`Interactivity::on_scroll_wheel`].
    fn on_scroll_wheel(
        mut self,
        listener: impl Fn(&ScrollWheelEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.interactivity().on_scroll_wheel(listener);
        self
    }

    /// Runs `listener` for pinch gestures over this element, during the bubble phase.
    ///
    /// The fluent equivalent of [`Interactivity::on_pinch`].
    fn on_pinch(mut self, listener: impl Fn(&PinchEvent, &mut Window, &mut App) + 'static) -> Self {
        self.interactivity().on_pinch(listener);
        self
    }

    /// Runs `listener` for pinch gestures during the capture phase.
    ///
    /// The fluent equivalent of [`Interactivity::capture_pinch`].
    fn capture_pinch(
        mut self,
        listener: impl Fn(&PinchEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.interactivity().capture_pinch(listener);
        self
    }
    /// Runs `listener` when an action of type `A` is dispatched to this element, during the capture phase.
    ///
    /// The capture phase runs before the bubble phase, so the listener sees the action before this element's
    /// [`on_action`](Self::on_action) handler and before its descendants'. Dispatch continues unless the listener calls
    /// [`App::stop_propagation`]. The fluent equivalent of [`Interactivity::capture_action`].
    fn capture_action<A: Action>(
        mut self,
        listener: impl Fn(&A, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.interactivity().capture_action(listener);
        self
    }

    /// Runs `listener` when an action of type `A` is dispatched to this element, during the bubble phase.
    ///
    /// The action reaches this element when it is on the focus path and no capture listener stopped it. Dispatch
    /// stops at this element after the listener runs, unless the listener calls [`App::propagate`]. The fluent
    /// equivalent of [`Interactivity::on_action`].
    ///
    /// # Example
    ///
    /// ```standalone_crate
    /// use gpui::{Context, Window, actions, div, prelude::*};
    ///
    /// actions!(note_example, [Save]);
    ///
    /// struct Note {
    ///     saves: usize,
    /// }
    ///
    /// impl Render for Note {
    ///     fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    ///         div().on_action(cx.listener(|note: &mut Note, _: &Save, _window, cx| {
    ///             note.saves += 1;
    ///             cx.notify();
    ///         }))
    ///     }
    /// }
    /// ```
    #[track_caller]
    fn on_action<A: Action>(
        mut self,
        listener: impl Fn(&A, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.interactivity().on_action(listener);
        self
    }

    /// Runs `listener` when an action of the same type as `action` is dispatched, during the bubble phase.
    ///
    /// Use this when the action type is only known at runtime. The listener receives the value passed here, so only its
    /// type decides when it runs. The fluent equivalent of [`Interactivity::on_boxed_action`].
    fn on_boxed_action(
        mut self,
        action: &dyn Action,
        listener: impl Fn(&dyn Action, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.interactivity().on_boxed_action(action, listener);
        self
    }

    /// Runs `listener` for key presses, during the bubble phase.
    ///
    /// The listener runs only if no action bound to the keystroke handled it, and the element must be on the focus path.
    /// Use it for text entry and other keys that have no action. The fluent equivalent of [`Interactivity::on_key_down`].
    fn on_key_down(
        mut self,
        listener: impl Fn(&KeyDownEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.interactivity().on_key_down(listener);
        self
    }

    /// Runs `listener` for key presses during the capture phase. Like [`on_key_down`](Self::on_key_down), it runs only
    /// if no action bound to the keystroke handled it.
    ///
    /// The fluent equivalent of [`Interactivity::capture_key_down`].
    fn capture_key_down(
        mut self,
        listener: impl Fn(&KeyDownEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.interactivity().capture_key_down(listener);
        self
    }

    /// Runs `listener` for key releases, during the bubble phase.
    ///
    /// The fluent equivalent of [`Interactivity::on_key_up`].
    fn on_key_up(
        mut self,
        listener: impl Fn(&KeyUpEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.interactivity().on_key_up(listener);
        self
    }

    /// Runs `listener` for key releases during the capture phase.
    ///
    /// The fluent equivalent of [`Interactivity::capture_key_up`].
    fn capture_key_up(
        mut self,
        listener: impl Fn(&KeyUpEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.interactivity().capture_key_up(listener);
        self
    }

    /// Runs `listener` when the set of pressed modifier keys changes.
    ///
    /// The fluent equivalent of [`Interactivity::on_modifiers_changed`].
    fn on_modifiers_changed(
        mut self,
        listener: impl Fn(&ModifiersChangedEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.interactivity().on_modifiers_changed(listener);
        self
    }

    /// Applies `f`'s style while a value of type `S` is dragged over this element.
    ///
    /// Same as [`on_drag_over`](Self::on_drag_over).
    fn drag_over<S: 'static>(
        mut self,
        f: impl 'static + Fn(StyleRefinement, &S, &mut Window, &mut App) -> StyleRefinement,
    ) -> Self {
        self.on_drag_over(f)
    }

    /// Applies `f`'s style while a value of type `S` is dragged over this element and the element accepts it.
    ///
    /// The style applies only while the pointer is over the element, and only when the element has a drop listener for
    /// `S` whose [`can_drop`](DropListener::can_drop) check passes, if it has one. Style the element differently for
    /// each kind of drag. Use [`on_drag_over_any`](Self::on_drag_over_any) for a style that doesn't depend on the type.
    ///
    /// # Example
    ///
    /// ```
    /// # use gpui::{div, prelude::*, rgb};
    /// #[derive(Clone)]
    /// struct FileDrag(String);
    ///
    /// let drop_zone = div()
    ///     .on_drag_over::<FileDrag>(|style, _file, _window, _cx| {
    ///         style.bg(rgb(0xdbeafe))
    ///     });
    /// ```
    fn on_drag_over<S: 'static>(
        mut self,
        f: impl 'static + Fn(StyleRefinement, &S, &mut Window, &mut App) -> StyleRefinement,
    ) -> Self {
        match self
            .interactivity()
            .get_mut_drop_listener(TypeId::of::<S>())
        {
            Some((_, drop_listener)) => {
                drop_listener.on_drag_over = Some(Box::new(move |value_any, window, cx| {
                    let value = value_any.downcast_ref::<S>().unwrap();
                    f(StyleRefinement::default(), value, window, cx)
                }))
            }
            None => {
                self.interactivity().on_drop_alt(DropListener::<S> {
                    can_drop: None,
                    on_drag_over: Some(Box::new(move |value, window, cx| {
                        f(StyleRefinement::default(), value, window, cx)
                    })),
                    on_drop: None,
                });
            }
        }
        self
    }

    /// Applies `f`'s style while any dragged value is over this element and the element can accept it.
    ///
    /// The fluent equivalent of [`Interactivity::on_drag_over_any`]. It applies only when the element has a drop listener
    /// for the dragged type.
    fn on_drag_over_any(
        mut self,
        f: impl 'static + Fn(StyleRefinement, &dyn Any, &mut Window, &mut App) -> StyleRefinement,
    ) -> Self {
        self.interactivity().on_drag_over_any(f);
        self
    }

    /// Applies `f`'s style while a value of type `S` is dragged over any member of `group_name`.
    ///
    /// Set the group with [`group`](Self::group) on the parent. The style applies only when the dragged type matches
    /// and the group member is hovered.
    fn group_drag_over<S: 'static>(
        mut self,
        group_name: impl Into<SharedString>,
        f: impl FnOnce(StyleRefinement) -> StyleRefinement,
    ) -> Self {
        self.interactivity().group_drag_over_styles.push((
            TypeId::of::<S>(),
            GroupStyle {
                group: group_name.into(),
                style: Box::new(f(StyleRefinement::default())),
            },
        ));
        self
    }

    /// Runs `listener` when a drag of type `T` is released over this element.
    ///
    /// The drop runs whether or not the drag started on this element. Use [`can_drop`](DropListener::can_drop) or a
    /// [`DropListener`] to refuse some drops. The fluent equivalent of [`Interactivity::on_drop`].
    fn on_drop<T: 'static>(
        mut self,
        listener: impl Fn(&T, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.interactivity().on_drop(listener);
        self
    }

    /// Attaches a [`DropListener`] that handles drags of type `T` over this element.
    ///
    /// Use this when one element needs a `can_drop` check, a drag-over style and a drop handler together. The fluent
    /// equivalent of [`Interactivity::on_drop_alt`].
    fn on_drop_alt<T: 'static>(mut self, listener: DropListener<T>) -> Self {
        self.interactivity().on_drop_alt(listener);
        self
    }

    /// Sets a predicate that decides whether this element accepts the dragged value.
    ///
    /// The predicate runs only while the pointer is over the element, and it receives the dragged value. Use it for checks
    /// that don't depend on the type of the value. The fluent equivalent of [`Interactivity::can_drop`].
    fn can_drop(
        mut self,
        predicate: impl Fn(&dyn Any, &mut Window, &mut App) -> bool + 'static,
    ) -> Self {
        self.interactivity().can_drop(predicate);
        self
    }

    /// Stops the mouse from reaching elements behind this element.
    ///
    /// Use it for floating panels, so that a click on the panel doesn't also click the content underneath. Prefer
    /// [`block_mouse_except_scroll`](Self::block_mouse_except_scroll) when the content should still scroll. The fluent
    /// equivalent of [`Interactivity::occlude_mouse`].
    fn occlude(mut self) -> Self {
        self.interactivity().occlude_mouse();
        self
    }

    /// Marks this element's bounds as a window control area, such as a custom title bar button.
    ///
    /// The platform uses `area` to decide what clicks and drags here do, such as moving or closing the window. The fluent
    /// equivalent of [`Interactivity::window_control_area`].
    fn window_control_area(mut self, area: WindowControlArea) -> Self {
        self.interactivity().window_control_area(area);
        self
    }

    /// Stops mouse interactions from reaching elements behind this element, but lets scroll events through.
    ///
    /// Use it for a panel that covers content which should still scroll. The fluent equivalent of
    /// [`Interactivity::block_mouse_except_scroll`].
    fn block_mouse_except_scroll(mut self) -> Self {
        self.interactivity().block_mouse_except_scroll();
        self
    }

    /// Applies `f`'s style when this element itself has focus.
    ///
    /// The element must track a focus handle with [`track_focus`](Self::track_focus). The style applies whenever the
    /// element is focused, whether focus came from the mouse or the keyboard. To style only keyboard focus, use
    /// [`focus_visible`](Self::focus_visible).
    fn focus(mut self, f: impl FnOnce(StyleRefinement) -> StyleRefinement) -> Self
    where
        Self: Sized,
    {
        self.interactivity().focus_style = Some(Box::new(f(StyleRefinement::default())));
        self
    }

    /// Applies `f`'s style when this element or one of its ancestors has focus.
    ///
    /// This tests [`FocusHandle::within_focused`]. To style a parent while its child has focus,
    /// query [`FocusHandle::contains_focused`] during rendering and choose the style from that result.
    /// The element must track a focus handle with [`track_focus`](Self::track_focus).
    fn in_focus(mut self, f: impl FnOnce(StyleRefinement) -> StyleRefinement) -> Self
    where
        Self: Sized,
    {
        self.interactivity().in_focus_style = Some(Box::new(f(StyleRefinement::default())));
        self
    }

    /// Applies `f`'s style when this element has focus and the last input was from the keyboard.
    ///
    /// This is like the CSS `:focus-visible` pseudo-class: the ring shows during keyboard navigation and not after a
    /// mouse click. The element must track a focus handle with [`track_focus`](Self::track_focus).
    ///
    /// # Example
    ///
    /// ```
    /// # use gpui::{div, prelude::*, rgb};
    /// let button = div()
    ///     .focus_visible(|style| style.border_2().border_color(rgb(0x2563eb)));
    /// ```
    fn focus_visible(mut self, f: impl FnOnce(StyleRefinement) -> StyleRefinement) -> Self
    where
        Self: Sized,
    {
        self.interactivity().focus_visible_style = Some(Box::new(f(StyleRefinement::default())));
        self
    }

    /// Resolves this element's cursor from pointer position and bounds.
    /// Return `None` to defer to another cursor request.
    fn cursor_with(
        mut self,
        resolver: impl Fn(Point<Pixels>, Bounds<Pixels>) -> Option<CursorStyle> + 'static,
    ) -> Self
    where
        Self: Sized,
    {
        self.interactivity().cursor_with(resolver);
        self
    }
}

/// An element with an id, which can also be clicked, scrolled, dragged, given a tooltip, and read by accessibility tools.
///
/// The methods here keep state between frames: they need the element to have an [`ElementId`], so call
/// [`id`](InteractiveElement::id) first. The result of `id` is a [`Stateful`] element, which implements this trait.
/// Accessibility methods are here too, because accessibility nodes are matched across frames by id.
///
/// See the [elements concept](crate::_documentation::concepts::elements) for why stateful elements need ids.
pub trait StatefulInteractiveElement: InteractiveElement {
    /// Sets the accessibility role of this element, such as a button or a list.
    ///
    /// Screen readers use the role to describe the element and decide what it can do. Use the role that matches
    /// the element's behavior. [`accesskit::Role::GenericContainer`] is ignored, so use it only for layout. See the
    /// [accessibility guide](crate::_documentation::guides::accessibility) for an overview.
    ///
    /// # Panics
    ///
    /// Panics in debug builds if `role` is [`accesskit::Role::GenericContainer`].
    fn role(mut self, role: accesskit::Role) -> Self {
        debug_assert!(
            role != accesskit::Role::GenericContainer,
            "GenericContainer is filtered out of the a11y tree and has no effect"
        );
        self.interactivity().override_role = Some(role);
        self
    }

    /// Sets the identifier that accessibility clients use to find this element.
    ///
    /// Unlike the [`ElementId`], this value is visible outside the process. Automation tools read it through
    /// UIA `AutomationId` on Windows, `AXIdentifier` on macOS, and AT-SPI `AccessibleId` where the adapter supports it.
    /// Keep it stable across runs and unique within its tree. It does not affect rendering or input.
    fn accessibility_id(mut self, id: impl Into<SharedString>) -> Self {
        self.interactivity().aria.author_id = Some(id.into());
        self
    }

    /// Sets the accessible name of this element, which screen readers announce when focus reaches it.
    ///
    /// Use it when the element has no visible text, such as an icon button. The label names the element; use
    /// [`aria_description`](Self::aria_description) for extra information.
    fn aria_label(mut self, label: impl Into<SharedString>) -> Self {
        self.interactivity().aria.label = Some(label.into());
        self
    }

    /// Sets the accessible description of this element, announced after its name, role and value.
    ///
    /// Use it for supplementary information, such as a hint or a settings subtitle. The [`aria_label`](Self::aria_label)
    /// names the element, and the description adds to it.
    fn aria_description(mut self, description: impl Into<SharedString>) -> Self {
        self.interactivity().aria.description = Some(description.into());
        self
    }

    /// Sets the keyboard shortcut that assistive technology announces for this element.
    ///
    /// The value is passed to AccessKit as the keyboard shortcut. It does not bind any keys: use a
    /// [`KeyBinding`](crate::KeyBinding) for that. Use the same shortcut your keymap uses so that the two match.
    fn aria_keyshortcuts(mut self, keyshortcuts: impl Into<SharedString>) -> Self {
        self.interactivity().aria.keyshortcuts = Some(keyshortcuts.into());
        self
    }

    /// Hides this element and its descendants from assistive technology.
    ///
    /// Rendering, focus and input do not change. Don't hide an element that can receive focus or that contains focusable
    /// descendants, because a screen reader user would then reach a control it cannot see.
    fn aria_hidden(mut self) -> Self {
        self.interactivity().aria.hidden = true;
        self
    }

    /// Reports this element as the focused node in the accessibility tree, while a focused ancestor is present.
    ///
    /// Use it for composite widgets, such as a menu or listbox, that keep keyboard focus on a container while one child is
    /// selected. Set it on the selected child, so that assistive technology announces the child. The element needs a
    /// [`role`](Self::role) and an [`id`](InteractiveElement::id) to produce an accessibility node. If no ancestor holds
    /// focus, the setting is ignored, so it is safe to set on the selected child at all times.
    fn aria_active_descendant(mut self) -> Self {
        self.interactivity().report_active_descendant_focus = true;
        self
    }

    /// Adds accessibility nodes that don't correspond to any element, as children of this element.
    ///
    /// Use it for content that has no elements of its own, such as the runs of text in an editor. GPUI calls `f` after
    /// prepaint, with a builder for the subtree. The element must have an id and either a [`role`](Self::role) or to be
    /// [`aria_hidden`](Self::aria_hidden). See [`Element::a11y_synthetic_children`] for details.
    fn a11y_synthetic_children(
        mut self,
        f: impl FnOnce(&mut crate::A11ySubtreeBuilder) + 'static,
    ) -> Self {
        self.interactivity().a11y_synthetic_children = Some(Box::new(f));
        self
    }

    /// Sets whether this element is selected, such as an item in a list or a tab.
    fn aria_selected(mut self, selected: bool) -> Self {
        self.interactivity().aria.selected = Some(selected);
        self
    }

    /// Sets whether this element is expanded, such as a disclosure triangle or a tree node.
    fn aria_expanded(mut self, expanded: bool) -> Self {
        self.interactivity().aria.expanded = Some(expanded);
        self
    }

    /// Sets the toggled state of this element, such as a checkbox that is on, off or mixed.
    fn aria_toggled(mut self, toggled: accesskit::Toggled) -> Self {
        self.interactivity().aria.toggled = Some(toggled);
        self
    }

    /// Sets whether this element is disabled, so that it rejects input.
    ///
    /// Use it for a disabled control or a group of controls. It maps to AccessKit's `Disabled` flag, which tells assistive
    /// technology that the element is unavailable.
    fn aria_disabled(mut self, disabled: bool) -> Self {
        self.interactivity().aria.disabled = Some(disabled);
        self
    }

    /// Sets the current numeric value of this element, such as the position of a slider.
    fn aria_numeric_value(mut self, value: f64) -> Self {
        self.interactivity().aria.numeric_value = Some(value);
        self
    }

    /// Sets the amount by which the numeric value changes per step, such as when a spin button is incremented.
    fn aria_numeric_value_step(mut self, step: f64) -> Self {
        self.interactivity().aria.numeric_value_step = Some(step);
        self
    }

    /// Sets the text value of this element, such as the contents of a simple text input.
    fn aria_value(mut self, value: impl Into<SharedString>) -> Self {
        self.interactivity().aria.value = Some(value.into());
        self
    }

    /// Sets the placeholder that assistive technology reads when a text input is empty.
    fn aria_placeholder(mut self, placeholder: impl Into<SharedString>) -> Self {
        self.interactivity().aria.placeholder = Some(placeholder.into());
        self
    }

    /// Sets the smallest numeric value this element can take.
    fn aria_min_numeric_value(mut self, value: f64) -> Self {
        self.interactivity().aria.min_numeric_value = Some(value);
        self
    }

    /// Sets the largest numeric value this element can take.
    fn aria_max_numeric_value(mut self, value: f64) -> Self {
        self.interactivity().aria.max_numeric_value = Some(value);
        self
    }

    /// Sets whether this element is laid out horizontally or vertically, such as a scrollbar or a splitter.
    fn aria_orientation(mut self, orientation: accesskit::Orientation) -> Self {
        self.interactivity().aria.orientation = Some(orientation);
        self
    }

    /// Sets the heading level of this element, where 1 is the top level.
    fn aria_level(mut self, level: usize) -> Self {
        self.interactivity().aria.level = Some(level);
        self
    }

    /// Sets the one-based position of this element within its set, such as the third item of five in a list.
    ///
    /// Use it with [`aria_size_of_set`](Self::aria_size_of_set) so that assistive technology can announce "3 of 5".
    fn aria_position_in_set(mut self, position: usize) -> Self {
        self.interactivity().aria.position_in_set = Some(position);
        self
    }

    /// Sets the number of items in the set this element belongs to.
    fn aria_size_of_set(mut self, size: usize) -> Self {
        self.interactivity().aria.size_of_set = Some(size);
        self
    }

    /// Sets the one-based row index of this element within its table or grid.
    fn aria_row_index(mut self, index: usize) -> Self {
        self.interactivity().aria.row_index = Some(index);
        self
    }

    /// Sets the one-based column index of this element within its table or grid.
    fn aria_column_index(mut self, index: usize) -> Self {
        self.interactivity().aria.column_index = Some(index);
        self
    }

    /// Sets the number of rows in the table or grid that contains this element.
    fn aria_row_count(mut self, count: usize) -> Self {
        self.interactivity().aria.row_count = Some(count);
        self
    }

    /// Sets the number of columns in the table or grid that contains this element.
    fn aria_column_count(mut self, count: usize) -> Self {
        self.interactivity().aria.column_count = Some(count);
        self
    }

    /// Marks this element, typically a dialog, as modal.
    ///
    /// While it is open, assistive technology treats the rest of the window as inert. It maps to AccessKit's `Modal` flag.
    fn aria_modal(mut self, modal: bool) -> Self {
        self.interactivity().aria.modal = Some(modal);
        self
    }

    /// Sets how urgently assistive technology announces changes to this element, as an ARIA live region.
    ///
    /// See [`accesskit::Live`] for the options. Set it on the element whose content changes.
    fn aria_live(mut self, live: accesskit::Live) -> Self {
        self.interactivity().aria.live = Some(live);
        self
    }

    /// Sets whether a change to this live region is announced as a whole, instead of only the changed part.
    ///
    /// It is meaningful only with [`aria_live`](Self::aria_live).
    fn aria_live_atomic(mut self, atomic: bool) -> Self {
        self.interactivity().aria.live_atomic = Some(atomic);
        self
    }

    /// Runs `listener` when assistive technology requests `action` on this element.
    ///
    /// The listener receives the action data the client sent, if any, along with the window and the app. Use it to let a
    /// screen reader press a button or change a value. See the [accessibility guide](crate::_documentation::guides::accessibility)
    /// for an overview.
    fn on_a11y_action(
        mut self,
        action: accesskit::Action,
        listener: impl FnMut(Option<&accesskit::ActionData>, &mut crate::Window, &mut crate::App)
        + 'static,
    ) -> Self {
        self.interactivity()
            .a11y_action_listeners
            .push((action, Box::new(listener)));
        self
    }

    /// Makes this element focusable, without giving it a focus handle of your own.
    ///
    /// GPUI creates a focus handle for the element and keeps it while the element is rendered. Use
    /// [`track_focus`](InteractiveElement::track_focus) when you need to focus the element from your code.
    fn focusable(mut self) -> Self {
        self.interactivity().focusable = true;
        self
    }

    /// Makes this element scroll on both axes when its content overflows.
    ///
    /// It sets `overflow_x` and `overflow_y` to scroll. Scrolling needs an id, which turns the element into a
    /// [`Stateful`] element, so that the scroll offset is kept between frames.
    fn overflow_scroll(mut self) -> Self {
        self.interactivity().base_style.overflow.x = Some(Overflow::Scroll);
        self.interactivity().base_style.overflow.y = Some(Overflow::Scroll);
        self
    }

    /// Makes this element scroll horizontally when its content overflows.
    ///
    /// Scrolling needs an id, which turns the element into a [`Stateful`] element.
    fn overflow_x_scroll(mut self) -> Self {
        self.interactivity().base_style.overflow.x = Some(Overflow::Scroll);
        self
    }

    /// Makes this element scroll vertically when its content overflows.
    ///
    /// Scrolling needs an id, which turns the element into a [`Stateful`] element.
    ///
    /// # Example
    ///
    /// ```
    /// # use gpui::{div, prelude::*};
    /// let list = div()
    ///     .id("list")
    ///     .overflow_y_scroll()
    ///     .children((0..100).map(|index| div().child(format!("Row {index}"))));
    /// ```
    fn overflow_y_scroll(mut self) -> Self {
        self.interactivity().base_style.overflow.y = Some(Overflow::Scroll);
        self
    }

    /// Restricts a scroll gesture to the axis it started on, so that a vertical swipe doesn't also scroll sideways.
    ///
    /// See [`Style::restrict_scroll_to_axis`](crate::Style::restrict_scroll_to_axis) for details.
    fn restrict_scroll_to_axis(mut self) -> Self {
        self.interactivity().base_style.restrict_scroll_to_axis = Some(true);
        self
    }

    /// Records this element's scroll state in `scroll_handle`, so that you can read and change the scroll position.
    ///
    /// Use the same handle across renders. Read the offset with [`ScrollHandle::offset`] and move to an item with
    /// [`ScrollHandle::scroll_to_item`].
    fn track_scroll(mut self, scroll_handle: &ScrollHandle) -> Self {
        self.interactivity().tracked_scroll_handle = Some(scroll_handle.clone());
        self
    }

    /// Connects this element to a [`ScrollAnchor`], so that the anchor can scroll the parent to this element.
    ///
    /// Use it when the element is not a direct child of the scrolling parent.
    fn anchor_scroll(mut self, scroll_anchor: Option<ScrollAnchor>) -> Self {
        self.interactivity().scroll_anchor = scroll_anchor;
        self
    }

    /// Applies `f`'s style while the mouse button is held down on this element.
    ///
    /// The style applies after a press that started on the element and until the button is released. It does not apply
    /// after keyboard input.
    fn active(mut self, f: impl FnOnce(StyleRefinement) -> StyleRefinement) -> Self
    where
        Self: Sized,
    {
        self.interactivity().active_style = Some(Box::new(f(StyleRefinement::default())));
        self
    }

    /// Applies `f`'s style while the mouse button is held down on any member of `group_name`.
    ///
    /// Set the group with [`group`](InteractiveElement::group) on the parent.
    fn group_active(
        mut self,
        group_name: impl Into<SharedString>,
        f: impl FnOnce(StyleRefinement) -> StyleRefinement,
    ) -> Self
    where
        Self: Sized,
    {
        self.interactivity().group_active_style = Some(GroupStyle {
            group: group_name.into(),
            style: Box::new(f(StyleRefinement::default())),
        });
        self
    }

    /// Runs `listener` when this element is clicked, or activated from the keyboard.
    ///
    /// A mouse click fires when the left button is pressed and released over the element, with no drag in between. A
    /// keyboard click fires when Enter or Space is pressed and released while the element is focused, with no other key
    /// pressed in between. The listener receives a [`ClickEvent`] that says which kind of click it was.
    ///
    /// Clicks need an id, so call [`id`](InteractiveElement::id) first. Keyboard clicks also need the element to be
    /// focusable, through [`track_focus`](InteractiveElement::track_focus) or [`focusable`](Self::focusable).
    ///
    /// # Example
    ///
    /// ```
    /// # use gpui::{div, prelude::*};
    /// let button = div()
    ///     .id("save")
    ///     .on_click(|event, _window, _cx| {
    ///         if event.is_keyboard() {
    ///             // Activated with Enter or Space.
    ///         }
    ///     });
    /// ```
    fn on_click(mut self, listener: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self
    where
        Self: Sized,
    {
        self.interactivity().on_click(listener);
        self
    }

    /// Runs `listener` for clicks with a button other than the left one, such as the middle button.
    ///
    /// Like [`on_click`](Self::on_click), this needs an id.
    fn on_aux_click(
        mut self,
        listener: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self
    where
        Self: Sized,
    {
        self.interactivity().on_aux_click(listener);
        self
    }

    /// Starts a drag of `value` when the left button is pressed on this element and the pointer moves.
    ///
    /// A drag starts when the pointer has moved more than 2 pixels from the press, and it is cancelled if the button is
    /// released first. The `constructor` runs at the start of the drag. It receives the value and the offset of the cursor
    /// from the element's origin, and returns the view that is drawn under the pointer.
    ///
    /// Use [`on_drop`](InteractiveElement::on_drop) on another element to receive the value. Use
    /// [`on_drag_move`](InteractiveElement::on_drag_move) to follow the pointer during the drag.
    ///
    /// # Example
    ///
    /// ```
    /// # use gpui::{Context, Render, Window, div, prelude::*, rgb};
    /// #[derive(Clone)]
    /// struct Card(&'static str);
    ///
    /// struct Chip(&'static str);
    ///
    /// impl Render for Chip {
    ///     fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
    ///         div().px_2().bg(rgb(0xdbeafe)).child(self.0)
    ///     }
    /// }
    ///
    /// fn card(label: &'static str) -> impl IntoElement {
    ///     div()
    ///         .id(label)
    ///         .on_drag(Card(label), |card, _offset, _window, cx| {
    ///             cx.new(|_| Chip(card.0))
    ///         })
    /// }
    /// ```
    fn on_drag<T, W>(
        mut self,
        value: T,
        constructor: impl Fn(&T, Point<Pixels>, &mut Window, &mut App) -> Entity<W> + 'static,
    ) -> Self
    where
        Self: Sized,
        T: 'static,
        W: 'static + Render,
    {
        self.interactivity().on_drag(value, constructor);
        self
    }

    /// Sets the function that starts a drag from this element.
    ///
    /// The function receives the cursor offset and the cursor style, and returns an [`AnyDrag`]. Use
    /// [`on_drag`](Self::on_drag) unless you need to build the [`AnyDrag`] yourself, for example to provide a different
    /// cursor style.
    fn on_drag_alt<F>(mut self, listener: F) -> Self
    where
        Self: Sized,
        F: 'static + Fn(Point<Pixels>, Option<CursorStyle>, &mut Window, &mut App) -> AnyDrag,
    {
        self.interactivity().on_drag_alt(listener);
        self
    }

    /// Registers a resolver for the data to offer the platform when a drag from this element leaves the window.
    ///
    /// The resolver runs at most once per drag, when the pointer leaves the window. Call it after [`on_drag`](Self::on_drag),
    /// and use the same dragged type `T`.
    ///
    /// # Panics
    ///
    /// Panics in debug builds if called before `on_drag`. If called twice on the same element, it panics in debug
    /// builds when a drag starts.
    fn external_drag_payload<T>(
        mut self,
        resolver: impl Fn(&T, &mut Window, &mut App) -> Option<ExternalDragPayload> + 'static,
    ) -> Self
    where
        Self: Sized,
        T: 'static,
    {
        self.interactivity().external_drag_payload(resolver);
        self
    }

    /// Runs `listener` with `true` when the pointer enters this element and `false` when it leaves.
    ///
    /// The listener does not run while a drag is in progress. Transitions caused by layout changes under a stationary
    /// pointer also run it.
    fn on_hover(mut self, listener: impl Fn(&bool, &mut Window, &mut App) + 'static) -> Self
    where
        Self: Sized,
    {
        self.interactivity().on_hover(listener);
        self
    }

    /// Calls the listener when a typed pointer projection changes.
    fn on_hover_region<R: PartialEq + 'static>(
        mut self,
        project: impl Fn(Point<Pixels>, Bounds<Pixels>) -> Option<R> + 'static,
        listener: impl Fn(&Option<R>, &mut Window, &mut App) + 'static,
    ) -> Self
    where
        Self: Sized,
    {
        self.interactivity().on_hover_region(project, listener);
        self
    }

    /// Shows a tooltip built by `build_tooltip` while the pointer rests over this element.
    ///
    /// The tooltip appears after the show delay, which is 500 milliseconds by default. It hides when the pointer leaves
    /// the element, so the user cannot move onto it. It is not shown while a button is pressed on the element or after
    /// keyboard input. Use [`hoverable_tooltip`](Self::hoverable_tooltip) for a tooltip that stays open. Tooltips need an
    /// id.
    ///
    /// # Example
    ///
    /// ```
    /// # use gpui::{Context, Window, div, prelude::*};
    /// struct Hint;
    ///
    /// impl Render for Hint {
    ///     fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
    ///         div().child("Save the document")
    ///     }
    /// }
    ///
    /// fn save_button() -> impl IntoElement {
    ///     div()
    ///         .id("save")
    ///         .tooltip(|_window, cx| cx.new(|_| Hint).into())
    /// }
    /// ```
    fn tooltip(mut self, build_tooltip: impl Fn(&mut Window, &mut App) -> AnyView + 'static) -> Self
    where
        Self: Sized,
    {
        self.interactivity().tooltip(build_tooltip);
        self
    }

    /// Shows a tooltip built by `build_tooltip` while the pointer rests over this element, and keeps it open while the
    /// pointer is over the tooltip.
    ///
    /// This is the same as [`tooltip`](Self::tooltip) except that the user can move into the tooltip, for example to
    /// select text or press a button in it.
    fn hoverable_tooltip(
        mut self,
        build_tooltip: impl Fn(&mut Window, &mut App) -> AnyView + 'static,
    ) -> Self
    where
        Self: Sized,
    {
        self.interactivity().hoverable_tooltip(build_tooltip);
        self
    }

    /// Sets how long the pointer must rest over this element before its tooltip appears.
    ///
    /// The default is 500 milliseconds.
    fn tooltip_show_delay(mut self, delay: Duration) -> Self
    where
        Self: Sized,
    {
        self.interactivity().tooltip_show_delay(delay);
        self
    }

    /// Runs `listener` during prepaint, once the element's bounds and content size are known.
    ///
    /// The listener is consumed each frame, so register it again on every render.
    fn on_prepaint(
        mut self,
        listener: impl Fn(&InteractivityPrepaint, &mut Window, &mut App) + 'static,
    ) -> Self
    where
        Self: Sized,
    {
        self.interactivity().on_prepaint(listener);
        self
    }
}

/// The bounds and content size of an element, as passed to [`Interactivity::on_prepaint`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InteractivityPrepaint {
    /// The bounds of the element
    pub bounds: Bounds<Pixels>,
    /// The size of the contents of the element
    pub content_size: Size<Pixels>,
}
pub(crate) type PrepaintListener =
    Box<dyn Fn(&InteractivityPrepaint, &mut Window, &mut App) + 'static>;

pub(crate) type MouseDownListener =
    Box<dyn Fn(&MouseDownEvent, DispatchPhase, &Hitbox, &mut Window, &mut App) + 'static>;
pub(crate) type MouseUpListener =
    Box<dyn Fn(&MouseUpEvent, DispatchPhase, &Hitbox, &mut Window, &mut App) + 'static>;
pub(crate) type MousePressureListener =
    Box<dyn Fn(&MousePressureEvent, DispatchPhase, &Hitbox, &mut Window, &mut App) + 'static>;
pub(crate) type MouseMoveListener =
    Box<dyn Fn(&MouseMoveEvent, DispatchPhase, &Hitbox, &mut Window, &mut App) + 'static>;
pub(crate) type MouseExitListener =
    Box<dyn Fn(&MouseExitEvent, DispatchPhase, &Hitbox, &mut Window, &mut App) + 'static>;

pub(crate) type ScrollWheelListener =
    Box<dyn Fn(&ScrollWheelEvent, DispatchPhase, &Hitbox, &mut Window, &mut App) + 'static>;

pub(crate) type PinchListener =
    Box<dyn Fn(&PinchEvent, DispatchPhase, &Hitbox, &mut Window, &mut App) + 'static>;

pub(crate) type ClickListener = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;
type PointerProjector<R> = Rc<dyn Fn(Point<Pixels>, Bounds<Pixels>) -> Option<R>>;
pub(crate) type CursorStyleResolver = PointerProjector<CursorStyle>;
type HoverRegionCallback<R> = Rc<dyn Fn(&Option<R>, &mut Window, &mut App)>;
type HoverRegionObserver = Rc<dyn Fn(Option<Point<Pixels>>, Bounds<Pixels>, &mut Window, &mut App)>;

/// Erases only the stored region type.
#[derive(Default)]
pub(crate) struct HoverRegionState(Option<Rc<dyn Any>>);

pub(crate) trait HoverRegionListener {
    fn bind(&self, state: &mut HoverRegionState) -> HoverRegionObserver;
}

struct TypedHoverRegionListener<R> {
    project: PointerProjector<R>,
    callback: HoverRegionCallback<R>,
}

impl<R: PartialEq + 'static> HoverRegionListener for TypedHoverRegionListener<R> {
    fn bind(&self, state: &mut HoverRegionState) -> HoverRegionObserver {
        // Reset state if this registration changes type.
        let value = state
            .0
            .as_ref()
            .and_then(|state| state.clone().downcast::<RefCell<Rc<Option<R>>>>().ok())
            .unwrap_or_else(|| Rc::new(RefCell::new(Rc::new(None))));
        state.0 = Some(value.clone());
        let project = self.project.clone();
        let callback = self.callback.clone();
        Rc::new(move |position, bounds, window, cx| {
            let next = position.and_then(|position| project(position, bounds));
            let mut previous = value.borrow_mut();
            if **previous == next {
                return;
            }
            let next = Rc::new(next);
            *previous = next.clone();
            // Release the borrow before callbacks can redraw or dispatch input.
            // Keep this snapshot alive across reentrant updates.
            drop(previous);
            callback(&next, window, cx);
        })
    }
}

pub(crate) type DragListener =
    Box<dyn Fn(Point<Pixels>, Option<CursorStyle>, &mut Window, &mut App) -> AnyDrag + 'static>;

/// Function type for [`DropListener::can_drop`], which checks if a typed payload can be dropped on an element's hitbox.
pub type CanDropListenerFn<T> = Box<dyn Fn(&T, &Hitbox, &mut Window, &mut App) -> bool + 'static>;
/// Function type for [`DropListener::on_drag_over`].
pub type OnDragOverFn<T> = Box<dyn Fn(&T, &mut Window, &mut App) -> StyleRefinement + 'static>;
/// Function type for [`DropListener::on_drop`].
pub type OnDropFn<T> = Box<dyn Fn(&T, &MouseUpEvent, &mut Window, &mut App) + 'static>;

/// Handles drags of a single type over an element: whether they can be dropped, how the element looks while
/// they are over it, and what happens when they are released.
///
/// Attach it with [`InteractiveElement::on_drop_alt`]. For the common case of one handler per type, use
/// [`InteractiveElement::on_drop`] instead.
pub struct DropListener<T: ?Sized> {
    /// Decides whether a dragged value of this type can be dropped on the element. If it is `None`, a value is accepted while the pointer is over the element.
    pub can_drop: Option<CanDropListenerFn<T>>,
    /// Returns the style to apply while a value of this type is over the element and [`can_drop`](DropListener::can_drop) allows it.
    pub on_drag_over: Option<OnDragOverFn<T>>,
    /// Runs when a value of this type is released over the element and [`can_drop`](DropListener::can_drop) allows it.
    pub on_drop: Option<OnDropFn<T>>,
}
impl<T> Default for DropListener<T> {
    fn default() -> Self {
        Self {
            can_drop: None,
            on_drag_over: None,
            on_drop: None,
        }
    }
}
impl<T: Sized + 'static> DropListener<T> {
    /// Sets a predicate that decides whether this handler receives drops, which runs only while the pointer is over the element.
    ///
    /// The predicate receives the dragged value and the window and app. The hitbox check is added for you, so the
    /// predicate need not check it. A predicate on a listener for another type never runs.
    pub fn can_drop_within(
        mut self,
        predicate: impl Fn(&T, &mut Window, &mut App) -> bool + 'static,
    ) -> Self {
        self.can_drop_anywhere(move |value, hitbox, window, cx| {
            hitbox.is_hovered(window) && predicate(value, window, cx)
        })
    }

    /// Sets a predicate that decides whether this handler receives drops, whether or not the pointer is over the element.
    ///
    /// The predicate receives the dragged value, the element's hitbox, and the window and app. Check
    /// [`Hitbox::is_hovered`] yourself if you need the pointer to be over the element.
    pub fn can_drop_anywhere(
        mut self,
        predicate: impl Fn(&T, &Hitbox, &mut Window, &mut App) -> bool + 'static,
    ) -> Self {
        self.can_drop = Some(Box::new(predicate));
        self
    }

    /// Sets the style to apply while a value of type `T` is dragged over the element.
    ///
    /// The style applies only if [`can_drop`](DropListener::can_drop) allows the value, and only while the pointer is over the element.
    pub fn on_drag_over(
        mut self,
        predicate: impl Fn(StyleRefinement, &T, &mut Window, &mut App) -> StyleRefinement + 'static,
    ) -> Self {
        self.on_drag_over = Some(Box::new(move |value, window, cx| {
            predicate(StyleRefinement::default(), value, window, cx)
        }));
        self
    }

    /// Sets the function that handles a value of type `T` released over the element.
    ///
    /// It runs only if [`can_drop`](DropListener::can_drop) allows the value. It receives the dragged value and the mouse
    /// release event.
    pub fn on_drop(
        mut self,
        predicate: impl Fn(&T, &MouseUpEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_drop = Some(Box::new(predicate));
        self
    }

    /// Converts this typed listener into one that stores any value and checks the type at runtime.
    ///
    /// GPUI calls this when you pass a `DropListener` to [`InteractiveElement::on_drop_alt`]. The callbacks downcast the
    /// dragged value to `T`. `on_drag_over` and `on_drop` panic if they receive another type, so register the listener only
    /// for the type it was written for.
    pub fn into_any(self) -> DropListener<dyn Any> {
        let DropListener {
            can_drop,
            on_drag_over,
            on_drop,
        } = self;
        let can_drop = can_drop.map(|predicate| {
            Box::new(
                move |any: &dyn Any, hitbox: &Hitbox, window: &mut Window, cx: &mut App| {
                    let Some(value_ref) = any.downcast_ref::<T>() else {
                        return false;
                    };
                    (&predicate)(value_ref, hitbox, window, cx)
                },
            ) as CanDropListenerFn<dyn Any>
        });
        let on_drag_over = on_drag_over.map(|predicate| {
            Box::new(move |any: &dyn Any, window: &mut Window, cx: &mut App| {
                let value_ref = any
                    .downcast_ref::<T>()
                    .expect("drop listener type was incorrect");
                (&predicate)(value_ref, window, cx)
            }) as OnDragOverFn<dyn Any>
        });
        let on_drop = on_drop.map(|predicate| {
            Box::new(
                move |any: &dyn Any, evt: &MouseUpEvent, window: &mut Window, cx: &mut App| {
                    let value_ref = any
                        .downcast_ref::<T>()
                        .expect("drop listener type was incorrect");
                    (&predicate)(value_ref, evt, window, cx)
                },
            ) as OnDropFn<dyn Any>
        });
        DropListener::<dyn Any> {
            can_drop,
            on_drag_over,
            on_drop,
        }
    }
}

type CanDropPredicate = Box<dyn Fn(&dyn Any, &Hitbox, &mut Window, &mut App) -> bool + 'static>;

pub(crate) struct TooltipBuilder {
    build: Rc<dyn Fn(&mut Window, &mut App) -> AnyView + 'static>,
    hoverable: bool,
}

pub(crate) type KeyDownListener =
    Box<dyn Fn(&KeyDownEvent, DispatchPhase, &mut Window, &mut App) + 'static>;

pub(crate) type KeyUpListener =
    Box<dyn Fn(&KeyUpEvent, DispatchPhase, &mut Window, &mut App) + 'static>;

pub(crate) type ModifiersChangedListener =
    Box<dyn Fn(&ModifiersChangedEvent, &mut Window, &mut App) + 'static>;

pub(crate) type ActionListener =
    Box<dyn Fn(&dyn Any, DispatchPhase, &mut Window, &mut App) + 'static>;

/// Creates an empty [`Div`] element, which you style and fill with children.
///
/// Chain styling, event handlers and children onto the result. The element does nothing until it is returned from a
/// `render` method or passed to a parent.
///
/// # Example
///
/// ```
/// # use gpui::{div, prelude::*, rgb};
/// let card = div()
///     .flex()
///     .flex_col()
///     .gap_2()
///     .p_3()
///     .bg(rgb(0xffffff))
///     .child("Title")
///     .child("Body");
/// ```
#[track_caller]
pub fn div() -> Div {
    Div {
        interactivity: Interactivity::new(),
        children: SmallVec::default(),
        prepaint_listener: None,
        image_cache: None,
        prepaint_order_fn: None,
    }
}

/// The all-in-one element for building most user interfaces. Create one with [`div`].
///
/// A `Div` is a flexbox container with styling, children and every event handler in
/// [`InteractiveElement`]. It is not stateful until you give it an [`id`](InteractiveElement::id), which returns a
/// [`Stateful`]. Use it as the basic building block for views and components.
pub struct Div {
    interactivity: Interactivity,
    children: SmallVec<[StackSafe<AnyElement>; 2]>,
    prepaint_listener: Option<Box<dyn Fn(Vec<Bounds<Pixels>>, &mut Window, &mut App) + 'static>>,
    image_cache: Option<Box<dyn ImageCacheProvider>>,
    prepaint_order_fn: Option<Box<dyn Fn(&mut Window, &mut App) -> SmallVec<[usize; 8]>>>,
}

impl Div {
    /// Runs `listener` after the children have been prepainted, with their bounds in order.
    ///
    /// Use it to record where children ended up, for example to scroll to one of them later. The bounds are in window
    /// coordinates. The listener runs once per frame.
    pub fn on_children_prepainted(
        mut self,
        listener: impl Fn(Vec<Bounds<Pixels>>, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.prepaint_listener = Some(Box::new(listener));
        self
    }

    /// Puts an image cache at this point in the element tree.
    ///
    /// Descendants of this element use this cache for the images they load, instead of the cache of an ancestor.
    pub fn image_cache(mut self, cache: impl ImageCacheProvider) -> Self {
        self.image_cache = Some(Box::new(cache));
        self
    }

    /// Sets a function that chooses the order in which children are prepainted.
    ///
    /// The function runs at prepaint and returns the indices of the children in the order to prepaint them. Each index
    /// must appear exactly once. Use it when one child's prepaint changes state that another child reads, such as an
    /// editor that must scroll before its sibling reads the scroll position.
    pub fn with_dynamic_prepaint_order(
        mut self,
        order_fn: impl Fn(&mut Window, &mut App) -> SmallVec<[usize; 8]> + 'static,
    ) -> Self {
        self.prepaint_order_fn = Some(Box::new(order_fn));
        self
    }
}

/// The layout state a [`Div`] keeps between its layout and prepaint phases.
///
/// It holds the layout ids of the children and any inline text layout. You don't create or use this type directly:
/// it appears only in the associated type of [`Div`]'s [`Element`] implementation.
pub struct DivFrameState {
    child_layout_ids: SmallVec<[LayoutId; 2]>,
    inline: Option<InlineDivFrameState>,
    contents_in_parent_paragraph: bool,
}

impl DivFrameState {
    fn standalone_inline(&self) -> Option<&InlineDivFrameState> {
        if self.contents_in_parent_paragraph {
            None
        } else {
            self.inline.as_ref()
        }
    }

    fn standalone_inline_mut(&mut self) -> Option<&mut InlineDivFrameState> {
        if self.contents_in_parent_paragraph {
            None
        } else {
            self.inline.as_mut()
        }
    }

    /// Returns the display text and layout used to paint each inline paragraph.
    #[cfg(any(test, feature = "test-support"))]
    pub fn measured_inline_paragraphs(&self) -> Vec<(SharedString, Arc<crate::InlineLayout>)> {
        self.inline
            .as_ref()
            .map_or_else(Vec::new, |inline| inline.measured_paragraphs())
    }
}

/// The interactivity state of an element, as shown and edited by the inspector.
#[derive(Clone)]
pub struct DivInspectorState {
    /// The element's base style. The inspector both reads and changes it, so changes made there apply to the element.
    #[cfg(any(feature = "inspector", debug_assertions))]
    pub base_style: Box<StyleRefinement>,
    /// The bounds of the element, in window coordinates, from its most recent prepaint.
    pub bounds: Bounds<Pixels>,
    /// The size of the element's content. If the element has no children, this is its bounds size.
    pub content_size: Size<Pixels>,
}

impl Styled for Div {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.interactivity.base_style
    }
}

impl InteractiveElement for Div {
    fn interactivity(&mut self) -> &mut Interactivity {
        &mut self.interactivity
    }
}

impl ParentElement for Div {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        #[cfg(feature = "stacker")]
        self.children
            .extend(elements.into_iter().map(StackSafe::new));
        #[cfg(not(feature = "stacker"))]
        self.children.extend(elements);
    }
}

impl Element for Div {
    type RequestLayoutState = DivFrameState;
    type PrepaintState = Option<Hitbox>;

    fn id(&self) -> Option<ElementId> {
        self.interactivity.element_id.clone()
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        self.interactivity.source_location()
    }

    fn a11y_role(&self) -> Option<accesskit::Role> {
        // Nodes with `GenericContainer` should never be reported to accesskit.
        // Equivalent to an HTML div with no role.
        self.interactivity
            .override_role
            .filter(|role| *role != accesskit::Role::GenericContainer)
    }

    fn is_a11y_hidden(&self) -> bool {
        self.interactivity.aria.hidden
    }

    fn write_a11y_info(&self, node: &mut accesskit::Node) {
        self.interactivity.write_a11y_info(node);
    }

    fn a11y_synthetic_children(
        &mut self,
        _prepaint: &mut Self::PrepaintState,
        builder: &mut crate::A11ySubtreeBuilder,
    ) {
        if let Some(f) = self.interactivity.a11y_synthetic_children.take() {
            f(builder);
        }
    }

    #[cfg_attr(feature = "stacker", stacksafe::stacksafe)]
    fn request_layout(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut child_layout_ids = SmallVec::new();
        let mut inline = None;
        let image_cache = self
            .image_cache
            .as_mut()
            .map(|provider| provider.provide(window, cx));

        let layout_id = window.with_image_cache(image_cache, |window| {
            self.interactivity.request_layout(
                global_id,
                inspector_id,
                window,
                cx,
                |style, window, cx| {
                    window.with_text_style(style.text_style().cloned(), |window| {
                        let display = style.display;
                        let collected_by_ancestor =
                            window.collecting_inline && display == Display::Inline;
                        let previous_collecting_inline = std::mem::replace(
                            &mut window.collecting_inline,
                            matches!(display, Display::Block | Display::Inline),
                        );
                        child_layout_ids = self
                            .children
                            .iter_mut()
                            .map(|child| child.request_layout(window, cx))
                            .collect();
                        window.collecting_inline = previous_collecting_inline;

                        let layout_id = if matches!(display, Display::Block | Display::Inline)
                            && !collected_by_ancestor
                        {
                            let (node_id, state) = InlineDivFrameState::request_layout(
                                &style,
                                &child_layout_ids,
                                window,
                                cx,
                            );

                            inline = Some(state);
                            node_id
                        } else if collected_by_ancestor {
                            window.request_layout(style, std::iter::empty(), cx)
                        } else {
                            window.request_layout(style, child_layout_ids.iter().copied(), cx)
                        };

                        if display == Display::Inline {
                            window.publish_inline_content(
                                layout_id,
                                InlineContent::Container {
                                    children: child_layout_ids.clone(),
                                },
                            );
                        }

                        window.set_layout_logical_children(layout_id, &child_layout_ids);

                        layout_id
                    })
                },
            )
        });

        (
            layout_id,
            DivFrameState {
                child_layout_ids,
                inline,
                contents_in_parent_paragraph: false,
            },
        )
    }

    #[cfg_attr(feature = "stacker", stacksafe::stacksafe)]
    fn prepaint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Hitbox> {
        let image_cache = self
            .image_cache
            .as_mut()
            .map(|provider| provider.provide(window, cx));

        let has_prepaint_listener = self.prepaint_listener.is_some();
        let mut children_bounds = Vec::with_capacity(if has_prepaint_listener {
            request_layout.child_layout_ids.len()
        } else {
            0
        });

        let mut child_min = point(Pixels::MAX, Pixels::MAX);
        let mut child_max = Point::default();
        if let Some(handle) = self.interactivity.scroll_anchor.as_ref() {
            *handle.last_origin.borrow_mut() = bounds.origin - window.element_offset();
        }

        request_layout.contents_in_parent_paragraph = window.current_inline_fragments.is_some();

        let content_size = if let Some(inline) = request_layout.standalone_inline() {
            inline.prepare_layout(
                bounds,
                self.interactivity.tracked_scroll_handle.as_ref(),
                &request_layout.child_layout_ids,
                window,
            )
        } else if request_layout.child_layout_ids.is_empty() {
            bounds.size
        } else if let Some(scroll_handle) = self.interactivity.tracked_scroll_handle.as_ref() {
            let mut state = scroll_handle.0.borrow_mut();
            state.child_bounds = Vec::with_capacity(request_layout.child_layout_ids.len());
            for child_layout_id in &request_layout.child_layout_ids {
                let child_bounds = window.layout_bounds(*child_layout_id);
                child_min = child_min.min(&child_bounds.origin);
                child_max = child_max.max(&child_bounds.bottom_right());
                state.child_bounds.push(child_bounds);
                if has_prepaint_listener {
                    children_bounds.push(child_bounds);
                }
            }
            (child_max - child_min).into()
        } else {
            for child_layout_id in &request_layout.child_layout_ids {
                let child_bounds = window.layout_bounds(*child_layout_id);
                child_min = child_min.min(&child_bounds.origin);
                child_max = child_max.max(&child_bounds.bottom_right());

                if has_prepaint_listener {
                    children_bounds.push(child_bounds);
                }
            }
            (child_max - child_min).into()
        };

        if let Some(scroll_handle) = self.interactivity.tracked_scroll_handle.as_ref() {
            scroll_handle.scroll_to_active_item();
        }

        self.interactivity.prepaint(
            global_id,
            inspector_id,
            bounds,
            content_size,
            window,
            cx,
            |style, scroll_offset, hitbox, window, cx| {
                // skip children
                if style.display == Display::None {
                    return hitbox;
                }

                #[inline]
                fn prepaint_children(
                    children: &mut [StackSafe<AnyElement>],
                    order: Option<&[usize]>,
                    window: &mut Window,
                    cx: &mut App,
                ) {
                    let order = match order {
                        Some(order) => Either::Left(order.iter().copied()),
                        None => Either::Right(0..children.len()),
                    };

                    for index in order {
                        if let Some(child) = children.get_mut(index) {
                            child.prepaint(window, cx);
                        }
                    }
                }

                window.with_image_cache(image_cache, |window| {
                    window.with_style_transition_containing_bounds(bounds, |window| {
                        let has_standalone_inline = request_layout.standalone_inline().is_some();
                        let inline_order = if has_standalone_inline {
                            self.prepaint_order_fn
                                .as_ref()
                                .map(|order_fn| order_fn(window, cx))
                        } else {
                            None
                        };

                        window.with_element_offset(scroll_offset, |window| {
                            if let Some(inline) = request_layout.standalone_inline_mut() {
                                inline.record_paragraph_origins(window);
                                prepaint_children(
                                    &mut self.children,
                                    inline_order.as_deref(),
                                    window,
                                    cx,
                                );
                            } else {
                                let order = self
                                    .prepaint_order_fn
                                    .as_ref()
                                    .map(|order_fn| order_fn(window, cx));
                                prepaint_children(&mut self.children, order.as_deref(), window, cx);
                            }
                        });

                        if has_prepaint_listener && has_standalone_inline {
                            children_bounds.extend(
                                request_layout
                                    .child_layout_ids
                                    .iter()
                                    .map(|layout_id| window.layout_bounds(*layout_id)),
                            );
                        }

                        if let Some(listener) = self.prepaint_listener.as_ref() {
                            listener(children_bounds, window, cx);
                        }
                    });
                });

                hitbox
            },
        )
    }

    #[cfg_attr(feature = "stacker", stacksafe::stacksafe)]
    fn paint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        hitbox: &mut Option<Hitbox>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let image_cache = self
            .image_cache
            .as_mut()
            .map(|provider| provider.provide(window, cx));

        window.with_image_cache(image_cache, |window| {
            self.interactivity.paint(
                global_id,
                inspector_id,
                bounds,
                hitbox.as_ref(),
                window,
                cx,
                |style, window, cx| {
                    // skip children
                    if style.display == Display::None {
                        return;
                    }

                    for child in &mut self.children {
                        child.paint(window, cx);
                    }

                    if let Some(inline) = request_layout.standalone_inline() {
                        inline.paint_paragraphs(window, cx);
                    }
                },
            )
        });
    }
}

impl IntoElement for Div {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

#[derive(Default)]
pub(crate) struct AriaProperties {
    pub(crate) author_id: Option<SharedString>,
    pub(crate) label: Option<SharedString>,
    pub(crate) description: Option<SharedString>,
    pub(crate) keyshortcuts: Option<SharedString>,
    pub(crate) hidden: bool,
    pub(crate) selected: Option<bool>,
    pub(crate) expanded: Option<bool>,
    pub(crate) toggled: Option<accesskit::Toggled>,
    pub(crate) disabled: Option<bool>,
    pub(crate) numeric_value: Option<f64>,
    pub(crate) min_numeric_value: Option<f64>,
    pub(crate) max_numeric_value: Option<f64>,
    pub(crate) numeric_value_step: Option<f64>,
    pub(crate) value: Option<SharedString>,
    pub(crate) placeholder: Option<SharedString>,
    pub(crate) orientation: Option<accesskit::Orientation>,
    pub(crate) level: Option<usize>,
    pub(crate) position_in_set: Option<usize>,
    pub(crate) size_of_set: Option<usize>,
    pub(crate) row_index: Option<usize>,
    pub(crate) column_index: Option<usize>,
    pub(crate) row_count: Option<usize>,
    pub(crate) column_count: Option<usize>,
    pub(crate) modal: Option<bool>,
    pub(crate) live: Option<accesskit::Live>,
    pub(crate) live_atomic: Option<bool>,
}

/// The interactivity state of an element: its listeners, styles, focus and scroll state.
///
/// [`Div`] and [`Stateful`] keep one of these. Use it directly when you build your own element and want the same
/// mouse, keyboard, drag and styling behavior as [`div`]. See the
/// [elements concept](crate::_documentation::concepts::elements) and the
/// [custom drawing guide](crate::_documentation::guides::custom_drawing).
#[derive(Default)]
pub struct Interactivity {
    /// The element's id, if it has one. Elements that keep state between frames need an id.
    pub element_id: Option<ElementId>,
    /// Whether the mouse button is pressed on this element, as of its most recent prepaint.
    ///
    /// This is `None` until the element has been prepainted with a state that tracks presses, such as an
    /// [`on_click`](StatefulInteractiveElement::on_click) listener or an [`active`](StatefulInteractiveElement::active) style.
    pub active: Option<bool>,
    /// Whether the pointer is over this element, as of its most recent paint.
    ///
    /// This is `None` if the element did not get a hitbox during prepaint.
    pub hovered: Option<bool>,
    pub(crate) tooltip_id: Option<TooltipId>,
    pub(crate) content_size: Size<Pixels>,
    pub(crate) key_context: Option<KeyContext>,
    pub(crate) focusable: bool,
    pub(crate) tracked_focus_handle: Option<FocusHandle>,
    pub(crate) tracked_scroll_handle: Option<ScrollHandle>,
    pub(crate) scroll_anchor: Option<ScrollAnchor>,
    pub(crate) scroll_offset: Option<Rc<RefCell<Point<Pixels>>>>,
    pub(crate) ongoing_scroll: Option<Rc<RefCell<OngoingScroll>>>,
    pub(crate) group: Option<SharedString>,
    /// The base style of the element, before any modifications are applied
    /// by focus, active, etc.
    pub base_style: Box<StyleRefinement>,
    pub(crate) style_transitions: Option<Box<StyleTransitions>>,
    pub(crate) focus_style: Option<Box<StyleRefinement>>,
    pub(crate) in_focus_style: Option<Box<StyleRefinement>>,
    pub(crate) focus_visible_style: Option<Box<StyleRefinement>>,
    pub(crate) hover_style: Option<Box<StyleRefinement>>,
    pub(crate) group_hover_style: Option<GroupStyle>,
    pub(crate) active_style: Option<Box<StyleRefinement>>,
    pub(crate) group_active_style: Option<GroupStyle>,
    pub(crate) drag_over_style:
        Option<Box<dyn Fn(&dyn Any, &mut Window, &mut App) -> StyleRefinement>>,
    pub(crate) group_drag_over_styles: Vec<(TypeId, GroupStyle)>,
    pub(crate) prepaint_listeners: Vec<PrepaintListener>,
    pub(crate) mouse_down_listeners: Vec<MouseDownListener>,
    pub(crate) mouse_up_listeners: Vec<MouseUpListener>,
    pub(crate) mouse_pressure_listeners: Vec<MousePressureListener>,
    pub(crate) mouse_move_listeners: Vec<MouseMoveListener>,
    pub(crate) mouse_exit_listeners: Vec<MouseExitListener>,
    pub(crate) scroll_wheel_listeners: Vec<ScrollWheelListener>,
    pub(crate) pinch_listeners: Vec<PinchListener>,
    pub(crate) key_down_listeners: Vec<KeyDownListener>,
    pub(crate) key_up_listeners: Vec<KeyUpListener>,
    pub(crate) modifiers_changed_listeners: Vec<ModifiersChangedListener>,
    pub(crate) action_listeners: Vec<(TypeId, ActionListener)>,
    pub(crate) drop_listeners: Vec<(TypeId, DropListener<dyn Any>)>,
    pub(crate) can_drop_predicate: Option<CanDropPredicate>,
    pub(crate) click_listeners: Vec<ClickListener>,
    pub(crate) aux_click_listeners: Vec<ClickListener>,
    pub(crate) drag_listener: Option<DragListener>,
    pub(crate) hover_region_listeners: Vec<Box<dyn HoverRegionListener>>,
    /// Resolves a cursor from the current pointer position and this element's bounds.
    pub(crate) cursor_style_resolver: Option<CursorStyleResolver>,
    pub(crate) tooltip_builder: Option<TooltipBuilder>,
    pub(crate) tooltip_show_delay: Option<Duration>,
    pub(crate) window_control: Option<WindowControlArea>,
    pub(crate) hitbox_behavior: HitboxBehavior,
    pub(crate) tab_index: Option<isize>,
    pub(crate) tab_group: bool,
    pub(crate) tab_stop: bool,

    pub(crate) a11y_action_listeners:
        Vec<(accesskit::Action, crate::window::a11y::A11yActionListener)>,
    pub(crate) a11y_synthetic_children: Option<Box<dyn FnOnce(&mut crate::A11ySubtreeBuilder)>>,
    pub(crate) report_active_descendant_focus: bool,
    pub(crate) override_role: Option<accesskit::Role>,
    pub(crate) aria: AriaProperties,

    #[cfg(any(feature = "inspector", debug_assertions))]
    pub(crate) source_location: Option<&'static core::panic::Location<'static>>,

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) debug_selector: Option<String>,
}

impl Interactivity {
    /// Sets the scroll offset of the element, for use during layout.
    ///
    /// Does nothing unless the element's style has `overflow` set to scroll on an axis. Call it from
    /// `request_layout`, before the children are laid out.
    pub fn set_scroll_offset(
        &self,
        global_id: Option<&GlobalElementId>,
        window: &mut Window,
        point: Point<Pixels>,
    ) {
        window.with_optional_element_state::<gpui::InteractiveElementState, _>(
            global_id,
            |element_state, _window| {
                let mut element_state =
                    element_state.map(|element_state| element_state.unwrap_or_default());
                let overflow = &self.base_style.overflow;
                if (overflow.x == Some(Overflow::Scroll) || overflow.y == Some(Overflow::Scroll))
                    && let Some(element_state) = element_state.as_mut()
                {
                    let scroll_offset = element_state.scroll_offset.get_or_insert_with(Rc::default);
                    *scroll_offset.borrow_mut() = point;
                }
                ((), element_state)
            },
        );
    }
}

impl Interactivity {
    /// Lays out this element using its configured styles.
    ///
    /// `f` receives the computed [`Style`] and returns the [`LayoutId`] for the element. Call it from an element's
    /// `request_layout` to apply the element's listeners, styles and state to its layout.
    pub fn request_layout(
        &mut self,
        global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
        f: impl FnOnce(Style, &mut Window, &mut App) -> LayoutId,
    ) -> LayoutId {
        #[cfg(any(feature = "inspector", debug_assertions))]
        window.with_inspector_state(
            _inspector_id,
            cx,
            |inspector_state: &mut Option<DivInspectorState>, _window| {
                if let Some(inspector_state) = inspector_state {
                    self.base_style = inspector_state.base_style.clone();
                } else {
                    *inspector_state = Some(DivInspectorState {
                        base_style: self.base_style.clone(),
                        bounds: Default::default(),
                        content_size: Default::default(),
                    })
                }
            },
        );

        window.with_optional_element_state::<InteractiveElementState, _>(
            global_id,
            |element_state, window| {
                let mut element_state =
                    element_state.map(|element_state| element_state.unwrap_or_default());

                if let Some(element_state) = element_state.as_ref()
                    && cx.has_active_drag()
                {
                    if let Some(pending_mouse_down) = element_state.pending_mouse_down.as_ref() {
                        *pending_mouse_down.borrow_mut() = None;
                    }
                    if let Some(clicked_state) = element_state.clicked_state.as_ref() {
                        *clicked_state.borrow_mut() = ElementClickedState::default();
                    }
                }

                // Ensure we store a focus handle in our element state if we're focusable.
                // If there's an explicit focus handle we're tracking, use that. Otherwise
                // create a new handle and store it in the element state, which lives for as many
                // frames as contain an element with this id.
                if self.focusable
                    && self.tracked_focus_handle.is_none()
                    && let Some(element_state) = element_state.as_mut()
                {
                    let mut handle = element_state
                        .focus_handle
                        .get_or_insert_with(|| cx.focus_handle())
                        .clone()
                        .tab_stop(self.tab_stop);

                    if let Some(index) = self.tab_index {
                        handle = handle.tab_index(index);
                    }

                    self.tracked_focus_handle = Some(handle);
                }

                if let Some(scroll_handle) = self.tracked_scroll_handle.as_ref() {
                    let scroll_handle_state = scroll_handle.0.borrow();
                    self.scroll_offset = Some(scroll_handle_state.offset.clone());
                    self.ongoing_scroll = Some(scroll_handle_state.ongoing_scroll.clone());
                } else if (self.base_style.overflow.x == Some(Overflow::Scroll)
                    || self.base_style.overflow.y == Some(Overflow::Scroll))
                    && let Some(element_state) = element_state.as_mut()
                {
                    self.scroll_offset = Some(
                        element_state
                            .scroll_offset
                            .get_or_insert_with(Rc::default)
                            .clone(),
                    );
                    self.ongoing_scroll = Some(
                        element_state
                            .ongoing_scroll
                            .get_or_insert_with(|| Rc::new(RefCell::new(OngoingScroll::default())))
                            .clone(),
                    );
                }

                let style =
                    self.compute_style_internal(None, None, element_state.as_mut(), window, cx);
                let layout_id = f(style, window, cx);
                (layout_id, element_state)
            },
        )
    }

    /// Commits the bounds of this element, computes its final style and hitbox, and then runs `f`.
    ///
    /// `f` receives the computed style, the scroll offset, and the element's hitbox if one was inserted. Prepaint listeners
    /// registered with [`Interactivity::on_prepaint`] run before `f`.
    pub fn prepaint<R>(
        &mut self,
        global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        content_size: Size<Pixels>,
        window: &mut Window,
        cx: &mut App,
        f: impl FnOnce(&Style, Point<Pixels>, Option<Hitbox>, &mut Window, &mut App) -> R,
    ) -> R {
        self.content_size = content_size;

        if !self.prepaint_listeners.is_empty() {
            let payload = InteractivityPrepaint {
                bounds,
                content_size,
            };
            for listener in self.prepaint_listeners.drain(..) {
                listener(&payload, window, cx);
            }
        }

        #[cfg(any(feature = "inspector", debug_assertions))]
        window.with_inspector_state(
            _inspector_id,
            cx,
            |inspector_state: &mut Option<DivInspectorState>, _window| {
                if let Some(inspector_state) = inspector_state {
                    inspector_state.bounds = bounds;
                    inspector_state.content_size = content_size;
                }
            },
        );

        if let Some(focus_handle) = self.tracked_focus_handle.as_ref() {
            window.set_focus_handle(focus_handle, cx);

            if window.a11y.is_active() {
                if let Some(global_id) = global_id {
                    let node_id = global_id.accesskit_node_id();
                    window.a11y.set_focusable(node_id, focus_handle.id);
                    if focus_handle.is_focused(window) {
                        window.a11y.set_focus(node_id);
                    }
                } else if focus_handle.is_focused(window) {
                    // Focusable, but with no element id it can't have an
                    // accessibility node, so screen readers fall back to the
                    // whole window.
                    window
                        .a11y
                        .note_focus_without_node(focus_handle.id, "it has no element id");
                }
            }
        }

        if self.report_active_descendant_focus && window.a11y.is_active() {
            if let Some(global_id) = global_id {
                window
                    .a11y
                    .set_active_descendant(global_id.accesskit_node_id());
            }
        }
        window.with_optional_element_state::<InteractiveElementState, _>(
            global_id,
            |element_state, window| {
                let mut element_state =
                    element_state.map(|element_state| element_state.unwrap_or_default());
                let style = self.compute_style_internal(
                    None,
                    Some(bounds),
                    element_state.as_mut(),
                    window,
                    cx,
                );

                if let Some(element_state) = element_state.as_mut() {
                    if let Some(clicked_state) = element_state.clicked_state.as_ref() {
                        let clicked_state = clicked_state.borrow();
                        self.active = Some(clicked_state.element);
                    }
                    if self.hover_style.is_some() || self.group_hover_style.is_some() {
                        element_state
                            .hover_state
                            .get_or_insert_with(Default::default);
                    }
                    if let Some(active_tooltip) = element_state.active_tooltip.as_ref() {
                        if self.tooltip_builder.is_some() {
                            self.tooltip_id = set_tooltip_on_window(active_tooltip, window);
                        } else {
                            // If there is no longer a tooltip builder, remove the active tooltip.
                            element_state.active_tooltip.take();
                        }
                    }
                }

                window.with_text_style(style.text_style().cloned(), |window| {
                    window.with_content_mask(
                        style.overflow_mask(bounds, window.rem_size()),
                        |window| {
                            let hitbox = if self.should_insert_hitbox(&style, window, cx) {
                                let hitbox = window.insert_hitbox_mut(bounds, self.hitbox_behavior);
                                // attach the group of the div to the hitbox's tags so it can be queried in hit-test related listeners
                                if let Some(group) = &self.group {
                                    hitbox.tags.push(group.clone());
                                }
                                Some(hitbox.clone())
                            } else {
                                None
                            };

                            let scroll_offset =
                                self.clamp_scroll_position(bounds, &style, window, cx);
                            let result = f(&style, scroll_offset, hitbox, window, cx);
                            (result, element_state)
                        },
                    )
                })
            },
        )
    }

    fn should_insert_hitbox(&self, style: &Style, window: &Window, cx: &App) -> bool {
        self.hitbox_behavior != HitboxBehavior::Normal
            || self.window_control.is_some()
            || style.mouse_cursor.is_some()
            || self.cursor_style_resolver.is_some()
            || self.group.is_some()
            || self.scroll_offset.is_some()
            || self.tracked_focus_handle.is_some()
            || self.hover_style.is_some()
            || self.group_hover_style.is_some()
            || !self.hover_region_listeners.is_empty()
            || !self.mouse_up_listeners.is_empty()
            || !self.mouse_pressure_listeners.is_empty()
            || !self.mouse_down_listeners.is_empty()
            || !self.mouse_move_listeners.is_empty()
            || !self.mouse_exit_listeners.is_empty()
            || !self.click_listeners.is_empty()
            || !self.aux_click_listeners.is_empty()
            || !self.scroll_wheel_listeners.is_empty()
            || self.has_pinch_listeners()
            || self.drag_listener.is_some()
            || !self.drop_listeners.is_empty()
            || self.drag_over_style.is_some()
            || self.tooltip_builder.is_some()
            || window.is_inspector_picking(cx)
    }

    fn clamp_scroll_position(
        &self,
        bounds: Bounds<Pixels>,
        style: &Style,
        window: &mut Window,
        _cx: &mut App,
    ) -> Point<Pixels> {
        fn round_to_two_decimals(pixels: Pixels) -> Pixels {
            const ROUNDING_FACTOR: f32 = 100.0;
            (pixels * ROUNDING_FACTOR).round() / ROUNDING_FACTOR
        }

        if let Some(scroll_offset) = self.scroll_offset.as_ref() {
            let mut scroll_to_bottom = false;
            let mut tracked_scroll_handle = self
                .tracked_scroll_handle
                .as_ref()
                .map(|handle| handle.0.borrow_mut());
            if let Some(mut scroll_handle_state) = tracked_scroll_handle.as_deref_mut() {
                scroll_handle_state.overflow = style.overflow;
                scroll_to_bottom = mem::take(&mut scroll_handle_state.scroll_to_bottom);
            }

            let rem_size = window.rem_size();
            // Taffy lays the box out with the padding snapped to the device pixel
            // grid (`to_taffy`); recomputed unsnapped, e.g. py_1 at a fractional
            // rem size, it exceeds `bounds` and leaves the box scrollable by the
            // sub-pixel difference.
            let padding = style
                .padding
                .to_pixels(bounds.size.into(), rem_size)
                .map(|edge| window.pixel_snap(*edge));
            let padding_size = size(padding.left + padding.right, padding.top + padding.bottom);
            // The floating point values produced by Taffy and ours often vary
            // slightly after ~5 decimal places. This can lead to cases where after
            // subtracting these, the container becomes scrollable for less than
            // 0.00000x pixels. As we generally don't benefit from a precision that
            // high for the maximum scroll, we round the scroll max to 2 decimal
            // places here.
            let padded_content_size = self.content_size + padding_size;
            let scroll_max = Point::from(padded_content_size - bounds.size)
                .map(round_to_two_decimals)
                .max(&Default::default());
            // Clamp scroll offset in case scroll max is smaller now (e.g., if children
            // were removed or the bounds became larger).
            let mut scroll_offset = scroll_offset.borrow_mut();

            scroll_offset.x = scroll_offset.x.clamp(-scroll_max.x, px(0.));
            if scroll_to_bottom {
                scroll_offset.y = -scroll_max.y;
            } else {
                scroll_offset.y = scroll_offset.y.clamp(-scroll_max.y, px(0.));
            }

            if let Some(mut scroll_handle_state) = tracked_scroll_handle {
                scroll_handle_state.max_offset = scroll_max;
                scroll_handle_state.bounds = bounds;
            }

            *scroll_offset
        } else {
            Point::default()
        }
    }

    /// Paints this element's background, borders and shadows, and registers its listeners with the window.
    ///
    /// `f` runs the painting of the element's contents, and receives the final style. Call it from an element's
    /// `paint` method, with the hitbox returned from prepaint.
    pub fn paint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        hitbox: Option<&Hitbox>,
        window: &mut Window,
        cx: &mut App,
        f: impl FnOnce(&Style, &mut Window, &mut App),
    ) {
        self.hovered = hitbox.map(|hitbox| hitbox.is_hovered(window));
        window.with_optional_element_state::<InteractiveElementState, _>(
            global_id,
            |element_state, window| {
                let mut element_state =
                    element_state.map(|element_state| element_state.unwrap_or_default());

                let style = self.compute_style_internal(
                    hitbox,
                    Some(bounds),
                    element_state.as_mut(),
                    window,
                    cx,
                );

                if let Some(state) = element_state.as_mut() {
                    state
                        .hover_region_states
                        .truncate(self.hover_region_listeners.len());
                }

                #[cfg(any(feature = "test-support", test))]
                if let Some(debug_selector) = &self.debug_selector {
                    window
                        .next_frame
                        .debug_bounds
                        .insert(debug_selector.clone(), bounds);
                }

                self.paint_hover_group_handler(window, cx);

                if style.visibility == Visibility::Hidden {
                    return ((), element_state);
                }

                let tab_group = self.tab_group.then_some(self.tab_index).flatten();

                let paint_content = |window: &mut Window, cx: &mut App| {
                    // Register the container inside its tab group so `focus_next` enters
                    // its own children. Outside the group, it would sort ahead of the
                    // items in every sibling group and enter the first group instead.
                    if let Some(focus_handle) = &self.tracked_focus_handle {
                        window.next_frame.tab_stops.insert(focus_handle);
                    }

                    if let Some(hitbox) = hitbox {
                        #[cfg(debug_assertions)]
                        self.paint_debug_info(global_id, hitbox, &style, window, cx);

                        if let Some(drag) = cx.active_drag.as_ref() {
                            if let Some(mouse_cursor) = drag.cursor_style {
                                window.set_window_cursor_style(mouse_cursor);
                            }
                        } else {
                            if let Some(mouse_cursor) = style.mouse_cursor {
                                window.set_cursor_style(mouse_cursor, hitbox);
                            }

                            if let Some(resolve_cursor) = self.cursor_style_resolver.clone() {
                                window.set_cursor_style_with(hitbox, move |position| {
                                    resolve_cursor(position, bounds)
                                });
                            }
                        }

                        if let Some(group) = self.group.clone() {
                            GroupHitboxes::push(group, hitbox.id, cx);
                        }

                        if let Some(area) = self.window_control {
                            window.insert_window_control_hitbox(area, hitbox.clone());
                        }

                        self.paint_mouse_listeners(hitbox, element_state.as_mut(), window, cx);
                        self.paint_scroll_listener(hitbox, &style, window, cx);
                    }

                    self.paint_keyboard_listeners(window, cx);

                    if window.a11y.is_active()
                        && let Some(global_id) = global_id
                        && !self.a11y_action_listeners.is_empty()
                    {
                        let node_id = global_id.accesskit_node_id();

                        for (action, listener) in self.a11y_action_listeners.drain(..) {
                            window.on_a11y_action(node_id, action, listener);
                        }
                    }

                    f(&style, window, cx);

                    if let Some(_hitbox) = hitbox {
                        #[cfg(any(feature = "inspector", debug_assertions))]
                        window.insert_inspector_hitbox(_hitbox.id, _inspector_id, cx);

                        if let Some(group) = self.group.as_ref() {
                            GroupHitboxes::pop(group, cx);
                        }
                    }
                };

                let paint_tab_group = |window: &mut Window, cx: &mut App| {
                    window.with_tab_group(tab_group, |window| paint_content(window, cx));
                };

                let paint_content_mask = |window: &mut Window, cx: &mut App| {
                    window.with_content_mask(
                        style.overflow_mask(bounds, window.rem_size()),
                        |window| paint_tab_group(window, cx),
                    );
                };

                let paint_text_style = |window: &mut Window, cx: &mut App| {
                    window.with_text_style(style.text_style().cloned(), |window| {
                        paint_content_mask(window, cx);
                    });
                };

                style.paint(bounds, window, cx, paint_text_style);

                ((), element_state)
            },
        );
    }

    #[cfg(debug_assertions)]
    fn paint_debug_info(
        &self,
        global_id: Option<&GlobalElementId>,
        hitbox: &Hitbox,
        style: &Style,
        window: &mut Window,
        cx: &mut App,
    ) {
        if let Some(global_id) = global_id
            && (style.debug || style.debug_below || cx.has_global::<crate::DebugBelow>())
            && hitbox.is_hovered(window)
        {
            const FONT_SIZE: crate::Pixels = crate::Pixels(10.);
            let element_id = format!("{global_id:?}");
            let str_len = element_id.len();

            let render_debug_text = |window: &mut Window| {
                if let Some(text) = window
                    .text_system()
                    .shape_text(
                        &element_id,
                        FONT_SIZE,
                        &[window.text_style().to_run(str_len)],
                        None,
                        None,
                    )
                    .ok()
                {
                    text.paint(hitbox.origin, FONT_SIZE, TextAlign::Left, None, window, cx)
                        .ok();

                    let text_bounds = crate::Bounds {
                        origin: hitbox.origin,
                        size: text.size(FONT_SIZE),
                    };
                    if let Some(source_location) = self.source_location
                        && text_bounds.contains(&window.mouse_position())
                        && window.modifiers().secondary()
                    {
                        let secondary_held = window.modifiers().secondary();
                        window.on_key_event({
                            move |e: &crate::ModifiersChangedEvent, _phase, window, _cx| {
                                if e.modifiers.secondary() != secondary_held
                                    && text_bounds.contains(&window.mouse_position())
                                {
                                    window.refresh();
                                }
                            }
                        });

                        let was_hovered = hitbox.is_hovered(window);
                        let current_view = window.current_view();
                        window.on_mouse_event({
                            let hitbox = hitbox.clone();
                            move |_: &MouseMoveEvent, phase, window, cx| {
                                if phase == DispatchPhase::Capture {
                                    let hovered = hitbox.is_hovered(window);
                                    if hovered != was_hovered {
                                        cx.notify(current_view)
                                    }
                                }
                            }
                        });

                        window.on_mouse_event({
                            let hitbox = hitbox.clone();
                            move |e: &crate::MouseDownEvent, phase, window, cx| {
                                if text_bounds.contains(&e.position)
                                    && phase.capture()
                                    && hitbox.is_hovered(window)
                                {
                                    cx.stop_propagation();
                                    let Ok(dir) = std::env::current_dir() else {
                                        return;
                                    };

                                    eprintln!(
                                        "This element was created at:\n{}:{}:{}",
                                        dir.join(source_location.file()).to_string_lossy(),
                                        source_location.line(),
                                        source_location.column()
                                    );
                                }
                            }
                        });
                        window.paint_quad(
                            PaintQuad::new(crate::Bounds {
                                origin: hitbox.origin
                                    + crate::point(crate::px(0.), FONT_SIZE - px(2.)),
                                size: crate::Size {
                                    width: text_bounds.size.width,
                                    height: crate::px(1.),
                                },
                            })
                            .border_widths(px(1.))
                            .border_color(crate::red()),
                        )
                    }
                }
            };

            window.with_text_style(
                Some(crate::TextStyleRefinement {
                    color: Some(crate::red()),
                    line_height: Some(FONT_SIZE.into()),
                    background_color: Some(crate::white()),
                    ..Default::default()
                }),
                render_debug_text,
            )
        }
    }

    fn has_drag_over_styles(&self) -> bool {
        self.drag_over_style.is_some() || {
            // whether there is a drop listener which affects styling
            self.drop_listeners
                .iter()
                .find(|(_, listener)| listener.on_drag_over.is_some())
                .is_some()
        }
    }

    fn paint_mouse_listeners(
        &mut self,
        hitbox: &Hitbox,
        element_state: Option<&mut InteractiveElementState>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let is_focused = self
            .tracked_focus_handle
            .as_ref()
            .map(|handle| handle.is_focused(window))
            .unwrap_or(false);

        // If this element can be focused, register a mouse down listener
        // that will automatically transfer focus when hitting the element.
        // This behavior can be suppressed by using `cx.prevent_default()`.
        if let Some(focus_handle) = self.tracked_focus_handle.clone() {
            let hitbox = hitbox.clone();
            window.on_mouse_event(move |_: &MouseDownEvent, phase, window, cx| {
                if phase == DispatchPhase::Bubble
                    && hitbox.is_hovered(window)
                    && !window.default_prevented()
                {
                    window.focus(&focus_handle, cx);
                    // If there is a parent that is also focusable, prevent it
                    // from transferring focus because we already did so.
                    window.prevent_default();
                }
            });
        }

        for listener in self.mouse_down_listeners.drain(..) {
            let hitbox = hitbox.clone();
            window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
                listener(event, phase, &hitbox, window, cx);
            })
        }

        for listener in self.mouse_up_listeners.drain(..) {
            let hitbox = hitbox.clone();
            window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
                listener(event, phase, &hitbox, window, cx);
            })
        }

        for listener in self.mouse_pressure_listeners.drain(..) {
            let hitbox = hitbox.clone();
            window.on_mouse_event(move |event: &MousePressureEvent, phase, window, cx| {
                listener(event, phase, &hitbox, window, cx);
            })
        }

        for listener in self.mouse_move_listeners.drain(..) {
            let hitbox = hitbox.clone();
            window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                listener(event, phase, &hitbox, window, cx);
            })
        }

        for listener in self.mouse_exit_listeners.drain(..) {
            let hitbox = hitbox.clone();
            window.on_mouse_event(move |event: &MouseExitEvent, phase, window, cx| {
                listener(event, phase, &hitbox, window, cx);
            })
        }

        for listener in self.scroll_wheel_listeners.drain(..) {
            let hitbox = hitbox.clone();
            window.on_mouse_event(move |event: &ScrollWheelEvent, phase, window, cx| {
                listener(event, phase, &hitbox, window, cx);
            })
        }

        for listener in self.pinch_listeners.drain(..) {
            let hitbox = hitbox.clone();
            window.on_mouse_event(move |event: &PinchEvent, phase, window, cx| {
                listener(event, phase, &hitbox, window, cx);
            })
        }

        if self.hover_style.is_some()
            || self.base_style.mouse_cursor.is_some()
            || cx.active_drag.is_some() && self.has_drag_over_styles()
        {
            let hitbox = hitbox.clone();
            let hover_state = self.hover_style.as_ref().and_then(|_| {
                element_state
                    .as_ref()
                    .and_then(|state| state.hover_state.as_ref())
                    .cloned()
            });
            let current_view = window.current_view();

            window.on_mouse_event(move |_: &MouseMoveEvent, phase, window, cx| {
                let hovered = hitbox.is_hovered(window);
                let was_hovered = hover_state
                    .as_ref()
                    .is_some_and(|state| state.borrow().element);
                if phase == DispatchPhase::Capture && hovered != was_hovered {
                    if let Some(hover_state) = &hover_state {
                        hover_state.borrow_mut().element = hovered;
                        cx.notify(current_view);
                    }
                }
            });
        }

        if let Some(group_hover) = self.group_hover_style.as_ref() {
            if let Some(group_hitbox_id) = GroupHitboxes::get(&group_hover.group, cx) {
                let hover_state = element_state
                    .as_ref()
                    .and_then(|element| element.hover_state.as_ref())
                    .cloned();
                let current_view = window.current_view();

                window.on_mouse_event(move |_: &MouseMoveEvent, phase, window, cx| {
                    let group_hovered = group_hitbox_id.is_hovered(window);
                    let was_group_hovered = hover_state
                        .as_ref()
                        .is_some_and(|state| state.borrow().group);
                    if phase == DispatchPhase::Capture && group_hovered != was_group_hovered {
                        if let Some(hover_state) = &hover_state {
                            hover_state.borrow_mut().group = group_hovered;
                            cx.notify(current_view);
                        }
                    }
                });
            }
        }

        let drag_cursor_style = self.base_style.as_ref().mouse_cursor;

        let mut drag_listener = mem::take(&mut self.drag_listener);
        let drop_listeners = mem::take(&mut self.drop_listeners);
        let click_listeners = mem::take(&mut self.click_listeners);
        let aux_click_listeners = mem::take(&mut self.aux_click_listeners);
        let can_drop_predicate = mem::take(&mut self.can_drop_predicate);

        if !drop_listeners.is_empty() {
            let hitbox = hitbox.clone();
            window.on_mouse_event({
                move |evt: &MouseUpEvent, phase, window, cx| {
                    if !phase.bubble() {
                        return;
                    }

                    let drag_value = match cx.active_drag() {
                        None => return,
                        Some(drag) => drag.value.clone(),
                    };

                    // Testing whether the element will be able to receive, regardless of if its hitbox is hovered.
                    // Elements that override this can check if the hitbox is hovered, manually (or via interactivity api).
                    if let Some(predicate) = &can_drop_predicate {
                        if !predicate(drag_value.as_ref(), &hitbox, window, cx) {
                            return;
                        }
                    }

                    let drag_state_type = drag_value.as_ref().type_id();
                    for (recv_drop_type, listener) in &drop_listeners {
                        if *recv_drop_type != drag_state_type {
                            continue;
                        }

                        // Test whether the DropListener api can receive the payload
                        // (which may or may not test the hitbox being hovered).
                        if let Some(predicate) = &listener.can_drop {
                            if !predicate(drag_value.as_ref(), &hitbox, window, cx) {
                                continue;
                            }
                        }
                        // Finally, if the element does override can_drop and the listener doesn't,
                        // then we only still permit drop here if the hitbox is hovered.
                        // This allows both previous predicates to override this check if they see fit
                        // (or implement it manually or via interactivity).
                        else if !hitbox.is_hovered(window) {
                            continue;
                        }

                        let drag_value =
                            cx.stop_drag(window).expect("already confirmed drag exists");

                        if let Some(predicate) = &listener.on_drop {
                            predicate(drag_value.as_ref(), evt, window, cx);
                        }
                        window.refresh();
                        cx.stop_propagation();
                        break;
                    }
                }
            });
        }

        if let Some(element_state) = element_state {
            if !click_listeners.is_empty()
                || !aux_click_listeners.is_empty()
                || drag_listener.is_some()
            {
                let pending_mouse_down = element_state
                    .pending_mouse_down
                    .get_or_insert_with(Default::default)
                    .clone();

                let pending_keyboard_down = element_state
                    .pending_keyboard_down
                    .get_or_insert_with(Default::default)
                    .clone();

                let clicked_state = element_state
                    .clicked_state
                    .get_or_insert_with(Default::default)
                    .clone();

                window.on_mouse_event({
                    let pending_mouse_down = pending_mouse_down.clone();
                    let hitbox = hitbox.clone();
                    let has_aux_click_listeners = !aux_click_listeners.is_empty();
                    move |event: &MouseDownEvent, phase, window, _cx| {
                        if phase == DispatchPhase::Bubble
                            && (event.button == MouseButton::Left || has_aux_click_listeners)
                            && hitbox.is_hovered(window)
                        {
                            *pending_mouse_down.borrow_mut() = Some(event.clone());
                            window.refresh();
                        }
                    }
                });

                window.on_mouse_event({
                    let pending_mouse_down = pending_mouse_down.clone();
                    let hitbox = hitbox.clone();
                    move |event: &MouseMoveEvent, phase, window, cx| {
                        if phase == DispatchPhase::Capture || drag_listener.is_none() {
                            return;
                        }

                        let mut pending_mouse_down = pending_mouse_down.borrow_mut();
                        let Some(mouse_down) = pending_mouse_down.clone() else {
                            return;
                        };
                        if cx.has_active_drag() || mouse_down.button != MouseButton::Left {
                            return;
                        }
                        if (event.position - mouse_down.position).magnitude() <= DRAG_THRESHOLD {
                            return;
                        }
                        if let Some(listener) = drag_listener.take() {
                            *clicked_state.borrow_mut() = ElementClickedState::default();
                            let cursor_offset = event.position - hitbox.origin;
                            let drag = listener(cursor_offset, drag_cursor_style, window, cx);
                            cx.start_drag(drag);
                            pending_mouse_down.take();
                            window.refresh();
                            cx.stop_propagation();
                        }
                    }
                });

                if is_focused {
                    // Record the focus generation at which an enter/space key
                    // down event happened on this element. The next key up
                    // event will be mapped to a click event if both of the
                    // following are true:
                    // - no other key events happen in between
                    // - the focus generation is the same (implying focus did not move)
                    //
                    // This design avoids an ABA problem that happens if you
                    // store the focus handle that registered the keypress.
                    window.on_key_event({
                        let pending_keyboard_down = pending_keyboard_down.clone();
                        move |event: &KeyDownEvent, phase, window, _cx| {
                            if phase.bubble() && !window.default_prevented() {
                                let stroke = &event.keystroke;
                                let is_activation_key = (stroke.key.eq("enter")
                                    || stroke.key.eq("space"))
                                    && !stroke.modifiers.modified();
                                *pending_keyboard_down.borrow_mut() =
                                    is_activation_key.then_some(window.focus_generation);
                            }
                        }
                    });

                    // Press enter, space to trigger click, when the element is focused.
                    window.on_key_event({
                        let click_listeners = click_listeners.clone();
                        let hitbox = hitbox.clone();
                        move |event: &KeyUpEvent, phase, window, cx| {
                            if phase.bubble() && !window.default_prevented() {
                                let stroke = &event.keystroke;
                                let keyboard_button = if stroke.key.eq("enter") {
                                    Some(KeyboardButton::Enter)
                                } else if stroke.key.eq("space") {
                                    Some(KeyboardButton::Space)
                                } else {
                                    None
                                };

                                if let Some(button) = keyboard_button
                                    && !stroke.modifiers.modified()
                                {
                                    let pending =
                                        std::mem::take(&mut *pending_keyboard_down.borrow_mut());
                                    if pending != Some(window.focus_generation) {
                                        return;
                                    }

                                    let click_event = ClickEvent::Keyboard(KeyboardClickEvent {
                                        button,
                                        bounds: hitbox.bounds,
                                    });

                                    for listener in &click_listeners {
                                        listener(&click_event, window, cx);
                                    }
                                } else {
                                    // Releasing any other key mid-press means
                                    // this isn't a clean activation, so cancel
                                    // the pending keydown.
                                    *pending_keyboard_down.borrow_mut() = None;
                                }
                            }
                        }
                    });
                }

                window.on_mouse_event({
                    let mut captured_mouse_down = None;
                    let hitbox = hitbox.clone();
                    move |event: &MouseUpEvent, phase, window, cx| match phase {
                        // Clear the pending mouse down during the capture phase,
                        // so that it happens even if another event handler stops
                        // propagation.
                        DispatchPhase::Capture => {
                            let mut pending_mouse_down = pending_mouse_down.borrow_mut();
                            if pending_mouse_down.is_some() && hitbox.is_hovered(window) {
                                captured_mouse_down = pending_mouse_down.take();
                                window.refresh();
                            } else if pending_mouse_down.is_some() {
                                // Clear the pending mouse down event (without firing click handlers)
                                // if the hitbox is not being hovered.
                                // This avoids dragging elements that changed their position
                                // immediately after being clicked.
                                // See https://github.com/zed-industries/zed/issues/24600 for more details
                                pending_mouse_down.take();
                                window.refresh();
                            }
                        }
                        // Fire click handlers during the bubble phase.
                        DispatchPhase::Bubble => {
                            if let Some(mouse_down) = captured_mouse_down.take() {
                                let btn = mouse_down.button;

                                let mouse_click = ClickEvent::Mouse(MouseClickEvent {
                                    down: mouse_down,
                                    up: event.clone(),
                                });

                                match btn {
                                    MouseButton::Left => {
                                        for listener in &click_listeners {
                                            listener(&mouse_click, window, cx);
                                        }
                                    }
                                    _ => {
                                        for listener in &aux_click_listeners {
                                            listener(&mouse_click, window, cx);
                                        }
                                    }
                                }
                            }
                        }
                    }
                });
            }

            if !self.hover_region_listeners.is_empty() {
                let states = &mut element_state.hover_region_states;
                states.resize_with(self.hover_region_listeners.len(), HoverRegionState::default);
                for (listener, state) in self.hover_region_listeners.iter().zip(states) {
                    let observe = listener.bind(state);
                    let region_hitbox = hitbox.clone();
                    let pending_mouse_down = element_state
                        .pending_mouse_down
                        .get_or_insert_with(Default::default)
                        .clone();
                    window.on_pointer_change(move |change, window, cx| {
                        let layout_changed = change.cause == PointerChangeCause::Layout;
                        if layout_changed && pending_mouse_down.borrow().is_some() {
                            return;
                        }
                        let position = if cx.has_active_drag()
                            || !layout_changed && pending_mouse_down.borrow().is_some()
                        {
                            None
                        } else if change.position.is_some() && !region_hitbox.is_hovered(window) {
                            None
                        } else {
                            change.position
                        };
                        observe(position, region_hitbox.bounds, window, cx);
                    });
                }
            }

            if let Some(tooltip_builder) = self.tooltip_builder.take() {
                let active_tooltip = element_state
                    .active_tooltip
                    .get_or_insert_with(Default::default)
                    .clone();
                let pending_mouse_down = element_state
                    .pending_mouse_down
                    .get_or_insert_with(Default::default)
                    .clone();

                let tooltip_is_hoverable = tooltip_builder.hoverable;
                let build_tooltip = Rc::new(move |window: &mut Window, cx: &mut App| {
                    Some(((tooltip_builder.build)(window, cx), tooltip_is_hoverable))
                });

                // Check hitbox geometry directly because hover state is unavailable during prepaint.
                let check_is_hovered_during_prepaint = Rc::new({
                    let pending_mouse_down = pending_mouse_down.clone();
                    let source_hitbox = hitbox.clone();
                    move |window: &Window| {
                        !window.last_input_was_keyboard()
                            && pending_mouse_down.borrow().is_none()
                            && source_hitbox.contains(&window.mouse_position())
                    }
                });
                let check_is_hovered = Rc::new({
                    let hitbox = hitbox.clone();
                    move |window: &Window| {
                        pending_mouse_down.borrow().is_none() && hitbox.is_hovered(window)
                    }
                });
                register_tooltip_mouse_handlers(
                    &active_tooltip,
                    self.tooltip_id,
                    build_tooltip,
                    check_is_hovered,
                    check_is_hovered_during_prepaint,
                    self.tooltip_show_delay,
                    window,
                );
            }

            // We unconditionally bind both the mouse up and mouse down active state handlers
            // Because we might not get a chance to render a frame before the mouse up event arrives.
            let active_state = element_state
                .clicked_state
                .get_or_insert_with(Default::default)
                .clone();

            {
                let active_state = active_state.clone();
                window.on_mouse_event(move |_: &MouseUpEvent, phase, window, _cx| {
                    if phase == DispatchPhase::Capture && active_state.borrow().is_clicked() {
                        *active_state.borrow_mut() = ElementClickedState::default();
                        window.refresh();
                    }
                });
            }

            {
                let active_group_hitbox = self
                    .group_active_style
                    .as_ref()
                    .and_then(|group_active| GroupHitboxes::get(&group_active.group, cx));
                let hitbox = hitbox.clone();
                window.on_mouse_event(move |_: &MouseDownEvent, phase, window, _cx| {
                    if phase == DispatchPhase::Bubble && !window.default_prevented() {
                        let group_hovered = active_group_hitbox
                            .is_some_and(|group_hitbox_id| group_hitbox_id.is_hovered(window));
                        let element_hovered = hitbox.is_hovered(window);
                        if group_hovered || element_hovered {
                            *active_state.borrow_mut() = ElementClickedState {
                                group: group_hovered,
                                element: element_hovered,
                            };
                            window.refresh();
                        }
                    }
                });
            }
        }
    }

    fn paint_keyboard_listeners(&mut self, window: &mut Window, _cx: &mut App) {
        let key_down_listeners = mem::take(&mut self.key_down_listeners);
        let key_up_listeners = mem::take(&mut self.key_up_listeners);
        let modifiers_changed_listeners = mem::take(&mut self.modifiers_changed_listeners);
        let action_listeners = mem::take(&mut self.action_listeners);
        if let Some(context) = self.key_context.clone() {
            window.set_key_context(context);
        }

        for listener in key_down_listeners {
            window.on_key_event(move |event: &KeyDownEvent, phase, window, cx| {
                listener(event, phase, window, cx);
            })
        }

        for listener in key_up_listeners {
            window.on_key_event(move |event: &KeyUpEvent, phase, window, cx| {
                listener(event, phase, window, cx);
            })
        }

        for listener in modifiers_changed_listeners {
            window.on_modifiers_changed(move |event: &ModifiersChangedEvent, window, cx| {
                listener(event, window, cx);
            })
        }

        for (action_type, listener) in action_listeners {
            window.on_action(action_type, listener)
        }
    }

    fn paint_hover_group_handler(&self, window: &mut Window, cx: &mut App) {
        let group_hitbox = self
            .group_hover_style
            .as_ref()
            .and_then(|group_hover| GroupHitboxes::get(&group_hover.group, cx));

        if let Some(group_hitbox) = group_hitbox {
            let was_hovered = group_hitbox.is_hovered(window);
            let current_view = window.current_view();
            window.on_mouse_event(move |_: &MouseMoveEvent, phase, window, cx| {
                let hovered = group_hitbox.is_hovered(window);
                if phase == DispatchPhase::Capture && hovered != was_hovered {
                    cx.notify(current_view);
                }
            });
        }
    }

    fn paint_scroll_listener(
        &self,
        hitbox: &Hitbox,
        style: &Style,
        window: &mut Window,
        _cx: &mut App,
    ) {
        if let Some(scroll_offset) = self.scroll_offset.clone() {
            let ongoing_scroll = self.ongoing_scroll.clone();
            let overflow = style.overflow;
            let allow_concurrent_scroll = style.allow_concurrent_scroll;
            let restrict_scroll_to_axis = style.restrict_scroll_to_axis;
            let line_height = window.line_height();
            let hitbox = hitbox.clone();
            let current_view = window.current_view();
            window.on_mouse_event(move |event: &ScrollWheelEvent, phase, window, cx| {
                if phase == DispatchPhase::Bubble && hitbox.should_handle_scroll(window) {
                    let mut scroll_offset = scroll_offset.borrow_mut();
                    let old_scroll_offset = *scroll_offset;
                    let mut delta = event.delta.pixel_delta(line_height);

                    if restrict_scroll_to_axis
                        && event.delta.precise()
                        && let Some(ongoing_scroll) = &ongoing_scroll
                    {
                        ongoing_scroll
                            .borrow_mut()
                            .filter(&mut delta, event.touch_phase);
                    }

                    let mut delta_x = match overflow.x {
                        Overflow::Scroll if !delta.x.is_zero() => delta.x,
                        Overflow::Scroll
                            if !restrict_scroll_to_axis && overflow.y != Overflow::Scroll =>
                        {
                            delta.y
                        }
                        _ => Pixels::ZERO,
                    };
                    let mut delta_y = match overflow.y {
                        Overflow::Scroll if !delta.y.is_zero() => delta.y,
                        Overflow::Scroll
                            if !restrict_scroll_to_axis && overflow.x != Overflow::Scroll =>
                        {
                            delta.x
                        }
                        _ => Pixels::ZERO,
                    };
                    if !allow_concurrent_scroll && !delta_x.is_zero() && !delta_y.is_zero() {
                        if delta_x.abs() > delta_y.abs() {
                            delta_y = Pixels::ZERO;
                        } else {
                            delta_x = Pixels::ZERO;
                        }
                    }
                    scroll_offset.y += delta_y;
                    scroll_offset.x += delta_x;
                    if *scroll_offset != old_scroll_offset {
                        cx.notify(current_view);
                    }
                }
            });
        }
    }

    /// Computes the visual style of this element for its current state.
    ///
    /// The result includes the base style, plus any focus, hover, active and drag-over styles that apply at the moment.
    pub fn compute_style(
        &self,
        global_id: Option<&GlobalElementId>,
        hitbox: Option<&Hitbox>,
        window: &mut Window,
        cx: &mut App,
    ) -> Style {
        window.with_optional_element_state(global_id, |element_state, window| {
            let mut element_state =
                element_state.map(|element_state| element_state.unwrap_or_default());
            let style = self.compute_style_internal(
                hitbox,
                hitbox.map(|hitbox| hitbox.bounds),
                element_state.as_mut(),
                window,
                cx,
            );
            (style, element_state)
        })
    }

    /// Called from internal methods that have already called with_element_state.
    fn compute_style_internal(
        &self,
        hitbox: Option<&Hitbox>,
        bounds: Option<Bounds<Pixels>>,
        mut element_state: Option<&mut InteractiveElementState>,
        window: &mut Window,
        cx: &mut App,
    ) -> Style {
        let mut style = Style::default();
        style.refine(&self.base_style);

        if let Some(focus_handle) = self.tracked_focus_handle.as_ref() {
            if let Some(in_focus_style) = self.in_focus_style.as_ref()
                && focus_handle.within_focused(window, cx)
            {
                style.refine(in_focus_style);
            }

            if let Some(focus_style) = self.focus_style.as_ref()
                && focus_handle.is_focused(window)
            {
                style.refine(focus_style);
            }

            if let Some(focus_visible_style) = self.focus_visible_style.as_ref()
                && focus_handle.is_focused(window)
                && window.last_input_was_keyboard()
            {
                style.refine(focus_visible_style);
            }
        }

        if !cx.has_active_drag() {
            if let Some(group_hover) = self.group_hover_style.as_ref() {
                let is_group_hovered =
                    if let Some(group_hitbox_id) = GroupHitboxes::get(&group_hover.group, cx) {
                        !window.last_input_was_touch() && group_hitbox_id.is_hovered(window)
                    } else if let Some(element_state) = element_state.as_ref() {
                        element_state
                            .hover_state
                            .as_ref()
                            .is_some_and(|state| state.borrow().group_is_active(window))
                    } else {
                        false
                    };

                if is_group_hovered {
                    style.refine(&group_hover.style);
                }
            }

            if let Some(hover_style) = self.hover_style.as_ref() {
                let is_hovered = if let Some(hitbox) = hitbox {
                    !window.last_input_was_touch() && hitbox.is_hovered(window)
                } else if let Some(element_state) = element_state.as_ref() {
                    element_state
                        .hover_state
                        .as_ref()
                        .is_some_and(|state| state.borrow().element_is_active(window))
                } else {
                    false
                };

                if is_hovered {
                    style.refine(hover_style);
                }
            }
        }

        if let Some(hitbox) = hitbox {
            if let Some(drag) = cx.active_drag.take() {
                let drag_state_type = drag.value.as_ref().type_id();
                let can_drop_without_listener = self
                    .can_drop_predicate
                    .as_ref()
                    .map(|predicate| predicate(drag.value.as_ref(), hitbox, window, cx))
                    .unwrap_or(true);
                let relevant_drop_listener = match can_drop_without_listener {
                    false => None,
                    true => {
                        let mut iter = self.drop_listeners.iter();
                        iter.find(|(drop_state_type, listener)| {
                            if *drop_state_type != drag_state_type {
                                return false;
                            }

                            let mut can_drop = true;
                            if let Some(predicate) = &listener.can_drop {
                                can_drop = predicate(drag.value.as_ref(), hitbox, window, cx);
                            }
                            can_drop
                        })
                    }
                };

                if let Some((_, drop_listener)) = relevant_drop_listener {
                    for (state_type, group_drag_style) in &self.group_drag_over_styles {
                        if let Some(group_hitbox_id) =
                            GroupHitboxes::get(&group_drag_style.group, cx)
                            && *state_type == drag.value.as_ref().type_id()
                            && group_hitbox_id.is_hovered(window)
                        {
                            style.refine(&group_drag_style.style);
                        }
                    }

                    if hitbox.is_hovered(window) {
                        if let Some(drag_over) = &self.drag_over_style {
                            style.refine(&drag_over(drag.value.as_ref(), window, cx));
                        }
                        if let Some(drag_over) = &drop_listener.on_drag_over {
                            style.refine(&drag_over(drag.value.as_ref(), window, cx));
                        }
                    }
                }

                style.mouse_cursor = drag.cursor_style;
                cx.active_drag = Some(drag);
            }
        }

        if let Some(element_state) = element_state.as_deref_mut() {
            let clicked_state = element_state
                .clicked_state
                .get_or_insert_with(Default::default)
                .borrow();
            if clicked_state.group
                && let Some(group) = self.group_active_style.as_ref()
            {
                style.refine(&group.style)
            }

            if let Some(active_style) = self.active_style.as_ref()
                && clicked_state.element
            {
                style.refine(active_style)
            }
        }

        if let Some(element_state) = element_state {
            if let Some(transitions) = self.style_transitions.as_ref() {
                if transitions.apply(
                    &mut style,
                    element_state
                        .style_transitions
                        .get_or_insert_with(Default::default),
                    StyleTransitionContext::new(bounds, window.rem_size())
                        .with_containing_bounds(window.style_transition_containing_bounds()),
                    cx.background_executor().now(),
                    cx.reduce_motion(),
                ) {
                    window.request_animation_frame();
                }
            } else {
                element_state.style_transitions = None;
            }
        }

        style
    }

    pub(crate) fn write_a11y_info(&self, node: &mut accesskit::Node) {
        if let Some(id) = &self.aria.author_id {
            node.set_author_id(id.to_string());
        }
        if let Some(label) = &self.aria.label {
            node.set_label(label.to_string());
        }
        if let Some(description) = &self.aria.description {
            node.set_description(description.to_string());
        }
        if let Some(keyshortcuts) = &self.aria.keyshortcuts {
            node.set_keyboard_shortcut(keyshortcuts.to_string());
        }
        if let Some(selected) = self.aria.selected {
            node.set_selected(selected);
        }
        if let Some(expanded) = self.aria.expanded {
            node.set_expanded(expanded);
        }
        if let Some(toggled) = self.aria.toggled {
            node.set_toggled(toggled);
        }
        if let Some(disabled) = self.aria.disabled {
            if disabled {
                node.set_disabled();
            } else {
                node.clear_disabled();
            }
        }
        if let Some(value) = self.aria.numeric_value {
            node.set_numeric_value(value);
        }
        if let Some(value) = self.aria.min_numeric_value {
            node.set_min_numeric_value(value);
        }
        if let Some(value) = self.aria.max_numeric_value {
            node.set_max_numeric_value(value);
        }
        if let Some(step) = self.aria.numeric_value_step {
            node.set_numeric_value_step(step);
        }
        if let Some(value) = &self.aria.value {
            node.set_value(value.to_string());
        }
        if let Some(placeholder) = &self.aria.placeholder {
            node.set_placeholder(placeholder.to_string());
        }
        if let Some(orientation) = self.aria.orientation {
            node.set_orientation(orientation);
        }
        if let Some(level) = self.aria.level {
            node.set_level(level);
        }
        if let Some(position) = self.aria.position_in_set {
            node.set_position_in_set(position);
        }
        if let Some(size) = self.aria.size_of_set {
            node.set_size_of_set(size);
        }
        if let Some(index) = self.aria.row_index {
            node.set_row_index(index);
        }
        if let Some(index) = self.aria.column_index {
            node.set_column_index(index);
        }
        if let Some(count) = self.aria.row_count {
            node.set_row_count(count);
        }
        if let Some(count) = self.aria.column_count {
            node.set_column_count(count);
        }
        if let Some(modal) = self.aria.modal {
            if modal {
                node.set_modal();
            } else {
                node.clear_modal();
            }
        }
        if let Some(live) = self.aria.live {
            node.set_live(live);
        }
        if let Some(atomic) = self.aria.live_atomic {
            if atomic {
                node.set_live_atomic();
            } else {
                node.clear_live_atomic();
            }
        }
        if !self.click_listeners.is_empty() {
            node.add_action(accesskit::Action::Click);
        }
        if self.tracked_focus_handle.is_some() || self.focusable {
            node.add_action(accesskit::Action::Focus);
        }
        for (action, _) in &self.a11y_action_listeners {
            node.add_action(*action);
        }
    }
}

/// The state that an element with an [`ElementId`] keeps between frames, such as its focus handle and scroll offset.
///
/// GPUI creates this for you. Read it only through the element's own methods.
#[derive(Default)]
pub struct InteractiveElementState {
    pub(crate) focus_handle: Option<FocusHandle>,
    pub(crate) clicked_state: Option<Rc<RefCell<ElementClickedState>>>,
    pub(crate) hover_state: Option<Rc<RefCell<ElementHoverState>>>,
    pub(crate) hover_region_states: Vec<HoverRegionState>,
    pub(crate) pending_mouse_down: Option<Rc<RefCell<Option<MouseDownEvent>>>>,
    /// Set to the window's [`focus_generation`](crate::Window::focus_generation)
    /// when an Enter/Space keydown is received while this element is focused,
    /// recording that we are waiting for the matching keyup to fire a keyboard
    /// click. On keyup the click only fires if the stored generation still
    /// matches the window's current one, i.e. focus never moved during the
    /// press (mirroring the browser clearing a control's pressed state on
    /// blur). `None` means no activation key is pending.
    pub(crate) pending_keyboard_down: Option<Rc<RefCell<Option<u64>>>>,
    pub(crate) scroll_offset: Option<Rc<RefCell<Point<Pixels>>>>,
    ongoing_scroll: Option<Rc<RefCell<OngoingScroll>>>,
    pub(crate) active_tooltip: Option<Rc<RefCell<Option<ActiveTooltip>>>>,
    pub(crate) style_transitions: Option<Box<StyleTransitionState>>,
}

/// Whether an element, or a group that contains it, is pressed by the mouse.
#[derive(Copy, Clone, Default, Eq, PartialEq)]
pub struct ElementClickedState {
    /// True if this element's group is pressed by the mouse.
    pub group: bool,

    /// True if this element is pressed by the mouse.
    pub element: bool,
}

impl ElementClickedState {
    fn is_clicked(&self) -> bool {
        self.group || self.element
    }
}

/// Whether an element, or a group that contains it, is hovered.
#[derive(Copy, Clone, Default, Eq, PartialEq)]
pub struct ElementHoverState {
    /// True if this element's group is hovered.
    pub group: bool,

    /// True if this element is hovered.
    pub element: bool,
}

impl ElementHoverState {
    fn group_is_active(&self, window: &Window) -> bool {
        self.group && !window.last_input_was_keyboard() && !window.last_input_was_touch()
    }

    fn element_is_active(&self, window: &Window) -> bool {
        self.element && !window.last_input_was_keyboard() && !window.last_input_was_touch()
    }
}

pub(crate) enum ActiveTooltip {
    /// Currently delaying before showing the tooltip.
    WaitingForShow { _task: Task<()> },
    /// Tooltip is visible, element was hovered or for hoverable tooltips, the tooltip was hovered.
    Visible {
        tooltip: AnyTooltip,
        is_hoverable: bool,
    },
    /// Tooltip is visible and hoverable, but the mouse is no longer hovering. Currently delaying
    /// before hiding it.
    WaitingForHide {
        tooltip: AnyTooltip,
        _task: Task<()>,
    },
}

pub(crate) fn clear_active_tooltip(
    active_tooltip: &Rc<RefCell<Option<ActiveTooltip>>>,
    window: &mut Window,
) {
    match active_tooltip.borrow_mut().take() {
        None => {}
        Some(ActiveTooltip::WaitingForShow { .. }) => {}
        Some(ActiveTooltip::Visible { .. }) => window.refresh(),
        Some(ActiveTooltip::WaitingForHide { .. }) => window.refresh(),
    }
}

pub(crate) fn clear_active_tooltip_if_not_hoverable(
    active_tooltip: &Rc<RefCell<Option<ActiveTooltip>>>,
    window: &mut Window,
) {
    let should_clear = match active_tooltip.borrow().as_ref() {
        None => false,
        Some(ActiveTooltip::WaitingForShow { .. }) => false,
        Some(ActiveTooltip::Visible { is_hoverable, .. }) => !is_hoverable,
        Some(ActiveTooltip::WaitingForHide { .. }) => false,
    };
    if should_clear {
        active_tooltip.borrow_mut().take();
        window.refresh();
    }
}

pub(crate) fn set_tooltip_on_window(
    active_tooltip: &Rc<RefCell<Option<ActiveTooltip>>>,
    window: &mut Window,
) -> Option<TooltipId> {
    let tooltip = match active_tooltip.borrow().as_ref() {
        None => return None,
        Some(ActiveTooltip::WaitingForShow { .. }) => return None,
        Some(ActiveTooltip::Visible { tooltip, .. }) => tooltip.clone(),
        Some(ActiveTooltip::WaitingForHide { tooltip, .. }) => tooltip.clone(),
    };
    Some(window.set_tooltip(tooltip))
}

pub(crate) fn register_tooltip_mouse_handlers(
    active_tooltip: &Rc<RefCell<Option<ActiveTooltip>>>,
    tooltip_id: Option<TooltipId>,
    build_tooltip: Rc<dyn Fn(&mut Window, &mut App) -> Option<(AnyView, bool)>>,
    check_is_hovered: Rc<dyn Fn(&Window) -> bool>,
    check_is_hovered_during_prepaint: Rc<dyn Fn(&Window) -> bool>,
    show_delay: Option<Duration>,
    window: &mut Window,
) {
    let current_view = window.current_view();
    let show_delay = show_delay.unwrap_or(DEFAULT_TOOLTIP_SHOW_DELAY);

    window.on_mouse_event({
        let active_tooltip = active_tooltip.clone();
        let build_tooltip = build_tooltip.clone();
        let check_is_hovered = check_is_hovered.clone();
        move |_: &MouseMoveEvent, phase, window, cx| {
            handle_tooltip_mouse_move(
                &active_tooltip,
                &build_tooltip,
                &check_is_hovered,
                &check_is_hovered_during_prepaint,
                tooltip_id,
                current_view,
                phase,
                show_delay,
                window,
                cx,
            )
        }
    });

    window.on_mouse_event({
        let active_tooltip = active_tooltip.clone();
        move |_: &MouseDownEvent, _phase, window: &mut Window, _cx| {
            if !tooltip_id.is_some_and(|tooltip_id| tooltip_id.is_hovered(window)) {
                clear_active_tooltip_if_not_hoverable(&active_tooltip, window);
            }
        }
    });

    window.on_mouse_event({
        let active_tooltip = active_tooltip.clone();
        move |_: &ScrollWheelEvent, _phase, window: &mut Window, _cx| {
            if !tooltip_id.is_some_and(|tooltip_id| tooltip_id.is_hovered(window)) {
                clear_active_tooltip_if_not_hoverable(&active_tooltip, window);
            }
        }
    });
}

/// Handles displaying tooltips when an element is hovered.
///
/// The hover check also runs during window prepaint. At that point the element may not have a
/// hitbox, so `check_is_hovered_during_prepaint` uses the element's absolute bounds instead.
///
/// TODO: There's a minor bug due to the use of absolute bounds while checking during prepaint - it
/// does not know if the hitbox is occluded. In the case where a tooltip gets displayed and then
/// gets occluded after display, it will stick around until the mouse exits the hover bounds.
fn handle_tooltip_mouse_move(
    active_tooltip: &Rc<RefCell<Option<ActiveTooltip>>>,
    build_tooltip: &Rc<dyn Fn(&mut Window, &mut App) -> Option<(AnyView, bool)>>,
    check_is_hovered: &Rc<dyn Fn(&Window) -> bool>,
    check_is_hovered_during_prepaint: &Rc<dyn Fn(&Window) -> bool>,
    tooltip_id: Option<TooltipId>,
    current_view: EntityId,
    phase: DispatchPhase,
    show_delay: Duration,
    window: &mut Window,
    cx: &mut App,
) {
    // Separates logic for what mutation should occur from applying it, to avoid overlapping
    // RefCell borrows.
    enum Action {
        None,
        CancelShow,
        ScheduleShow,
        CheckVisible,
    }

    let action = match active_tooltip.borrow().as_ref() {
        None => {
            let is_hovered = check_is_hovered(window);
            if is_hovered && phase.bubble() {
                Action::ScheduleShow
            } else {
                Action::None
            }
        }
        Some(ActiveTooltip::WaitingForShow { .. }) => {
            let is_hovered = check_is_hovered(window);
            if is_hovered {
                Action::None
            } else {
                Action::CancelShow
            }
        }
        Some(ActiveTooltip::Visible { is_hoverable, .. }) => {
            if phase.capture()
                && !check_is_hovered(window)
                && (!*is_hoverable
                    || !tooltip_id.is_some_and(|tooltip_id| tooltip_id.is_hovered(window)))
            {
                Action::CheckVisible
            } else {
                Action::None
            }
        }
        Some(ActiveTooltip::WaitingForHide { .. }) => {
            if phase.capture()
                && (check_is_hovered(window)
                    || tooltip_id.is_some_and(|tooltip_id| tooltip_id.is_hovered(window)))
            {
                Action::CheckVisible
            } else {
                Action::None
            }
        }
    };

    match action {
        Action::None => {}
        Action::CancelShow => {
            // Cancel waiting to show tooltip when it is no longer hovered.
            active_tooltip.borrow_mut().take();
        }
        Action::ScheduleShow => {
            let delayed_show_task = window.spawn(cx, {
                let weak_active_tooltip = Rc::downgrade(active_tooltip);
                let build_tooltip = build_tooltip.clone();
                let check_is_hovered_during_prepaint = check_is_hovered_during_prepaint.clone();
                async move |cx| {
                    cx.background_executor().timer(show_delay).await;
                    let Some(active_tooltip) = weak_active_tooltip.upgrade() else {
                        return;
                    };
                    cx.update(|window, cx| {
                        let new_tooltip =
                            build_tooltip(window, cx).map(|(view, tooltip_is_hoverable)| {
                                let weak_active_tooltip = Rc::downgrade(&active_tooltip);
                                ActiveTooltip::Visible {
                                    tooltip: AnyTooltip {
                                        view,
                                        mouse_position: window.mouse_position(),
                                        check_visible_and_update: Rc::new(
                                            move |tooltip_bounds, window, cx| {
                                                let Some(active_tooltip) =
                                                    weak_active_tooltip.upgrade()
                                                else {
                                                    return false;
                                                };
                                                handle_tooltip_check_visible_and_update(
                                                    &active_tooltip,
                                                    tooltip_is_hoverable,
                                                    &check_is_hovered_during_prepaint,
                                                    tooltip_bounds,
                                                    window,
                                                    cx,
                                                )
                                            },
                                        ),
                                    },
                                    is_hoverable: tooltip_is_hoverable,
                                }
                            });
                        *active_tooltip.borrow_mut() = new_tooltip;
                        window.refresh();
                    })
                    .ok();
                }
            });
            active_tooltip
                .borrow_mut()
                .replace(ActiveTooltip::WaitingForShow {
                    _task: delayed_show_task,
                });
        }
        Action::CheckVisible => cx.notify(current_view),
    }
}

/// Returns a callback which will be called by window prepaint to update tooltip visibility. The
/// purpose of doing this logic here instead of the mouse move handler is that the mouse move
/// handler won't get called when the element is not painted (e.g. via use of `visible_on_hover`).
fn handle_tooltip_check_visible_and_update(
    active_tooltip: &Rc<RefCell<Option<ActiveTooltip>>>,
    tooltip_is_hoverable: bool,
    check_is_hovered: &Rc<dyn Fn(&Window) -> bool>,
    tooltip_bounds: Bounds<Pixels>,
    window: &mut Window,
    cx: &mut App,
) -> bool {
    // Separates logic for what mutation should occur from applying it, to avoid overlapping RefCell
    // borrows.
    enum Action {
        None,
        Hide,
        ScheduleHide(AnyTooltip),
        CancelHide(AnyTooltip),
    }

    let is_hovered = check_is_hovered(window)
        || (tooltip_is_hoverable && tooltip_bounds.contains(&window.mouse_position()));
    let action = match active_tooltip.borrow().as_ref() {
        Some(ActiveTooltip::Visible { tooltip, .. }) => {
            if is_hovered {
                Action::None
            } else {
                if tooltip_is_hoverable {
                    Action::ScheduleHide(tooltip.clone())
                } else {
                    Action::Hide
                }
            }
        }
        Some(ActiveTooltip::WaitingForHide { tooltip, .. }) => {
            if is_hovered {
                Action::CancelHide(tooltip.clone())
            } else {
                Action::None
            }
        }
        None | Some(ActiveTooltip::WaitingForShow { .. }) => Action::None,
    };

    match action {
        Action::None => {}
        Action::Hide => clear_active_tooltip(active_tooltip, window),
        Action::ScheduleHide(tooltip) => {
            let delayed_hide_task = window.spawn(cx, {
                let weak_active_tooltip = Rc::downgrade(active_tooltip);
                async move |cx| {
                    cx.background_executor()
                        .timer(HOVERABLE_TOOLTIP_HIDE_DELAY)
                        .await;
                    let Some(active_tooltip) = weak_active_tooltip.upgrade() else {
                        return;
                    };
                    if active_tooltip.borrow_mut().take().is_some() {
                        cx.update(|window, _cx| window.refresh()).ok();
                    }
                }
            });
            active_tooltip
                .borrow_mut()
                .replace(ActiveTooltip::WaitingForHide {
                    tooltip,
                    _task: delayed_hide_task,
                });
        }
        Action::CancelHide(tooltip) => {
            // Cancel waiting to hide tooltip when it becomes hovered.
            active_tooltip.borrow_mut().replace(ActiveTooltip::Visible {
                tooltip,
                is_hoverable: true,
            });
        }
    }

    active_tooltip.borrow().is_some()
}

/// Temporary cache of hitbox groups, written and read from during paint recursion.
#[derive(Default)]
pub(crate) struct GroupHitboxes(HashMap<SharedString, SmallVec<[HitboxId; 1]>>);

impl Global for GroupHitboxes {}

impl GroupHitboxes {
    pub fn get(name: &SharedString, cx: &mut App) -> Option<HitboxId> {
        cx.default_global::<Self>()
            .0
            .get(name)
            .and_then(|bounds_stack| bounds_stack.last())
            .cloned()
    }

    pub fn push(name: SharedString, hitbox_id: HitboxId, cx: &mut App) {
        cx.default_global::<Self>()
            .0
            .entry(name)
            .or_default()
            .push(hitbox_id);
    }

    pub fn pop(name: &SharedString, cx: &mut App) {
        cx.default_global::<Self>().0.get_mut(name).unwrap().pop();
    }
}

/// An element with an [`ElementId`], which lets it keep state between frames.
///
/// You get a `Stateful` by calling [`InteractiveElement::id`]. It implements the same traits as the element it wraps,
/// plus [`StatefulInteractiveElement`], which adds clicks, scrolling, starting a drag, and tooltips.
pub struct Stateful<E> {
    pub(crate) element: E,
}

impl<E> Styled for Stateful<E>
where
    E: Styled,
{
    fn style(&mut self) -> &mut StyleRefinement {
        self.element.style()
    }
}

impl<E> StatefulInteractiveElement for Stateful<E> where Self: InteractiveElement {}

impl<E> InteractiveElement for Stateful<E>
where
    E: InteractiveElement,
{
    fn interactivity(&mut self) -> &mut Interactivity {
        self.element.interactivity()
    }
}

impl<E> Element for Stateful<E>
where
    E: Element,
{
    type RequestLayoutState = E::RequestLayoutState;
    type PrepaintState = E::PrepaintState;

    fn id(&self) -> Option<ElementId> {
        self.element.id()
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        self.element.source_location()
    }

    fn a11y_role(&self) -> Option<accesskit::Role> {
        self.element.a11y_role()
    }

    fn is_a11y_hidden(&self) -> bool {
        self.element.is_a11y_hidden()
    }

    fn write_a11y_info(&self, node: &mut accesskit::Node) {
        self.element.write_a11y_info(node);
    }

    fn a11y_synthetic_children(
        &mut self,
        prepaint: &mut Self::PrepaintState,
        builder: &mut crate::A11ySubtreeBuilder,
    ) {
        self.element.a11y_synthetic_children(prepaint, builder);
    }

    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        self.element.request_layout(id, inspector_id, window, cx)
    }

    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        state: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> E::PrepaintState {
        self.element
            .prepaint(id, inspector_id, bounds, state, window, cx)
    }

    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.element.paint(
            id,
            inspector_id,
            bounds,
            request_layout,
            prepaint,
            window,
            cx,
        );
    }
}

impl<E: IntoElement> IntoElement for Stateful<E> {
    type Element = E::Element;

    fn into_element(self) -> Self::Element {
        self.element.into_element()
    }
}

impl<E> ParentElement for Stateful<E>
where
    E: ParentElement,
{
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.element.extend(elements)
    }
}

/// A reference to an element that a [`ScrollHandle`] can scroll to, even if it is not a direct child of the scrolling
/// element.
///
/// Create it with [`ScrollAnchor::for_handle`] from the parent's [`ScrollHandle`], which the parent tracks with
/// [`StatefulInteractiveElement::track_scroll`]. Attach the anchor to the target with
/// [`StatefulInteractiveElement::anchor_scroll`], then call [`ScrollAnchor::scroll_to`].
#[derive(Clone)]
pub struct ScrollAnchor {
    handle: ScrollHandle,
    last_origin: Rc<RefCell<Point<Pixels>>>,
}

impl ScrollAnchor {
    /// Creates an anchor that scrolls `handle`'s element when [`scroll_to`](Self::scroll_to) is called.
    pub fn for_handle(handle: ScrollHandle) -> Self {
        Self {
            handle,
            last_origin: Default::default(),
        }
    }
    /// Scrolls the anchored element's parent so that the anchored element is at the top of the viewport.
    ///
    /// The scroll happens on the next frame, after the anchored element has been painted and its position is known.
    pub fn scroll_to(&self, window: &mut Window, _cx: &mut App) {
        let this = self.clone();

        window.on_next_frame(move |_, _| {
            let viewport_bounds = this.handle.bounds();
            let self_bounds = *this.last_origin.borrow();
            this.handle.set_offset(viewport_bounds.origin - self_bounds);
        });
    }
}

#[derive(Default, Debug)]
struct ScrollHandleState {
    offset: Rc<RefCell<Point<Pixels>>>,
    ongoing_scroll: Rc<RefCell<OngoingScroll>>,
    bounds: Bounds<Pixels>,
    max_offset: Point<Pixels>,
    child_bounds: Vec<Bounds<Pixels>>,
    scroll_to_bottom: bool,
    overflow: Point<Overflow>,
    active_item: Option<ScrollActiveItem>,
}

#[derive(Default, Debug, Clone, Copy)]
struct ScrollActiveItem {
    index: usize,
    strategy: ScrollStrategy,
}

#[derive(Default, Debug, Clone, Copy)]
enum ScrollStrategy {
    #[default]
    FirstVisible,
    Top,
}

/// A handle to the scroll state of an element, used to read the scroll position and to scroll it.
///
/// Create it with [`ScrollHandle::new`], attach it with [`StatefulInteractiveElement::track_scroll`], and keep it across
/// renders, since the scroll state lives in the handle. Clones of a handle share the same state.
///
/// # Example
///
/// ```
/// # use gpui::{ScrollHandle, point, px};
/// let handle = ScrollHandle::new();
/// assert_eq!(handle.offset(), point(px(0.), px(0.)));
/// assert_eq!(handle.children_count(), 0);
/// ```
#[derive(Clone, Debug)]
pub struct ScrollHandle(Rc<RefCell<ScrollHandleState>>);

impl Default for ScrollHandle {
    fn default() -> Self {
        Self::new()
    }
}

impl ScrollHandle {
    /// Creates a new scroll handle with no scroll state. The offset is zero until an element is tracked with it.
    pub fn new() -> Self {
        Self(Rc::default())
    }

    /// Returns the current scroll offset.
    ///
    /// The offset is the distance from the top left of the scrolling element to the top left of its content. It is zero
    /// at the top and becomes more negative as the content scrolls up or to the left.
    pub fn offset(&self) -> Point<Pixels> {
        *self.0.borrow().offset.borrow()
    }

    /// Returns the largest offset the element can scroll to. The values are the distances the content can move, as positive values.
    pub fn max_offset(&self) -> Point<Pixels> {
        self.0.borrow().max_offset
    }

    /// Returns the index of the first child that is at least partly visible at the top of the viewport.
    ///
    /// Returns `0` if the element has no children.
    pub fn top_item(&self) -> usize {
        let state = self.0.borrow();
        let top = state.bounds.top() - state.offset.borrow().y;

        match state.child_bounds.binary_search_by(|bounds| {
            if top < bounds.top() {
                Ordering::Greater
            } else if top > bounds.bottom() {
                Ordering::Less
            } else {
                Ordering::Equal
            }
        }) {
            Ok(ix) => ix,
            Err(ix) => ix.min(state.child_bounds.len().saturating_sub(1)),
        }
    }

    /// Returns the index of the last child that is at least partly visible at the bottom of the viewport.
    ///
    /// Returns `0` if the element has no children.
    pub fn bottom_item(&self) -> usize {
        let state = self.0.borrow();
        let bottom = state.bounds.bottom() - state.offset.borrow().y;

        match state.child_bounds.binary_search_by(|bounds| {
            if bottom < bounds.top() {
                Ordering::Greater
            } else if bottom > bounds.bottom() {
                Ordering::Less
            } else {
                Ordering::Equal
            }
        }) {
            Ok(ix) => ix,
            Err(ix) => ix.min(state.child_bounds.len().saturating_sub(1)),
        }
    }

    /// Returns the bounds of the scrolling element's viewport, in window coordinates.
    pub fn bounds(&self) -> Bounds<Pixels> {
        self.0.borrow().bounds
    }

    /// Returns the bounds of the child at `ix`, in window coordinates, or `None` if there is no such child.
    pub fn bounds_for_item(&self, ix: usize) -> Option<Bounds<Pixels>> {
        self.0.borrow().child_bounds.get(ix).cloned()
    }

    /// Scrolls the smallest amount that makes the child at `ix` fully visible.
    ///
    /// The scroll happens during the next prepaint. If the element is not scrollable on the axis, nothing happens.
    pub fn scroll_to_item(&self, ix: usize) {
        let mut state = self.0.borrow_mut();
        state.active_item = Some(ScrollActiveItem {
            index: ix,
            strategy: ScrollStrategy::default(),
        });
    }

    /// Scrolls so that the child at `ix` is at the top of the viewport.
    ///
    /// The scroll happens during the next prepaint.
    pub fn scroll_to_top_of_item(&self, ix: usize) {
        let mut state = self.0.borrow_mut();
        state.active_item = Some(ScrollActiveItem {
            index: ix,
            strategy: ScrollStrategy::Top,
        });
    }

    /// Scrolls the minimal amount to either ensure that the child is
    /// fully visible or the top element of the view depends on the
    /// scroll strategy
    fn scroll_to_active_item(&self) {
        let mut state = self.0.borrow_mut();

        let Some(active_item) = state.active_item else {
            return;
        };

        let active_item = match state.child_bounds.get(active_item.index) {
            Some(bounds) => {
                let mut scroll_offset = state.offset.borrow_mut();

                match active_item.strategy {
                    ScrollStrategy::FirstVisible => {
                        if state.overflow.y == Overflow::Scroll {
                            let child_height = bounds.size.height;
                            let viewport_height = state.bounds.size.height;
                            if child_height > viewport_height {
                                scroll_offset.y = state.bounds.top() - bounds.top();
                            } else if bounds.top() + scroll_offset.y < state.bounds.top() {
                                scroll_offset.y = state.bounds.top() - bounds.top();
                            } else if bounds.bottom() + scroll_offset.y > state.bounds.bottom() {
                                scroll_offset.y = state.bounds.bottom() - bounds.bottom();
                            }
                        }
                    }
                    ScrollStrategy::Top => {
                        scroll_offset.y = state.bounds.top() - bounds.top();
                    }
                }

                if state.overflow.x == Overflow::Scroll {
                    let child_width = bounds.size.width;
                    let viewport_width = state.bounds.size.width;
                    if child_width > viewport_width {
                        scroll_offset.x = state.bounds.left() - bounds.left();
                    } else if bounds.left() + scroll_offset.x < state.bounds.left() {
                        scroll_offset.x = state.bounds.left() - bounds.left();
                    } else if bounds.right() + scroll_offset.x > state.bounds.right() {
                        scroll_offset.x = state.bounds.right() - bounds.right();
                    }
                }
                None
            }
            None => Some(active_item),
        };
        state.active_item = active_item;
    }

    /// Scrolls to the bottom of the content on the next prepaint.
    pub fn scroll_to_bottom(&self) {
        let mut state = self.0.borrow_mut();
        state.scroll_to_bottom = true;
    }

    /// Sets the scroll offset directly.
    ///
    /// The offset is the distance from the top left of the scrolling element to the top left of its content, so scrolling
    /// down makes the offset more negative. Values outside the scrollable range are clamped during the next prepaint.
    pub fn set_offset(&self, mut position: Point<Pixels>) {
        let state = self.0.borrow();
        *state.offset.borrow_mut() = position;
    }

    /// Returns the child at the top of the viewport, and how far its top edge is above the viewport's top edge.
    ///
    /// Use the pair to restore the same scroll position after the children change: scroll to the index, then offset by
    /// the pixels.
    pub fn logical_scroll_top(&self) -> (usize, Pixels) {
        let ix = self.top_item();
        let state = self.0.borrow();

        if let Some(child_bounds) = state.child_bounds.get(ix) {
            (
                ix,
                child_bounds.top() + state.offset.borrow().y - state.bounds.top(),
            )
        } else {
            (ix, px(0.))
        }
    }

    /// Returns the child at the bottom of the viewport, and how far its bottom edge is below the viewport's bottom edge.
    pub fn logical_scroll_bottom(&self) -> (usize, Pixels) {
        let ix = self.bottom_item();
        let state = self.0.borrow();

        if let Some(child_bounds) = state.child_bounds.get(ix) {
            (
                ix,
                child_bounds.bottom() + state.offset.borrow().y - state.bounds.bottom(),
            )
        } else {
            (ix, px(0.))
        }
    }

    /// Returns the number of children that were laid out in the most recent frame.
    pub fn children_count(&self) -> usize {
        self.0.borrow().child_bounds.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn tracked_scroll_reports_children_to_prepaint_listener(cx: &mut TestAppContext) {
        struct ScrollView {
            bounds: Rc<RefCell<Vec<Bounds<Pixels>>>>,
            handle: ScrollHandle,
        }

        impl Render for ScrollView {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                let bounds = self.bounds.clone();
                div()
                    .on_children_prepainted(move |children, _, _| {
                        *bounds.borrow_mut() = children;
                    })
                    .id("scrolling-container")
                    .flex()
                    .flex_col()
                    .size_full()
                    .overflow_y_scroll()
                    .track_scroll(&self.handle)
                    .child(div().h(px(20.)).flex_shrink_0())
                    .child(div().h(px(30.)).flex_shrink_0())
            }
        }

        let captured = Rc::new(RefCell::new(Vec::new()));
        let scroll_handle = ScrollHandle::new();
        let window = cx.open_window(size(px(100.), px(100.)), |_, _| ScrollView {
            bounds: captured.clone(),
            handle: scroll_handle.clone(),
        });
        cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))
            .unwrap();

        let bounds = captured.borrow();
        assert_eq!(bounds.len(), 2);
        assert_eq!(bounds[0].size.height, px(20.));
        assert_eq!(bounds[1].size.height, px(30.));
        assert_eq!(bounds[1].top(), bounds[0].bottom());
        assert_eq!(scroll_handle.bounds_for_item(0), Some(bounds[0]));
        assert_eq!(scroll_handle.bounds_for_item(1), Some(bounds[1]));
    }

    #[gpui::test]
    fn inline_div_places_element_children_in_text_flow(cx: &mut TestAppContext) {
        let placed_children = Rc::new(RefCell::new(Vec::new()));
        let captured_bounds = placed_children.clone();
        let window = cx.add_empty_window();
        let highlight_color = hsla(0.4, 0.6, 0.5, 1.0);

        window.draw(
            point(px(10.), px(20.)),
            size(px(160.), px(80.)),
            move |_, _| {
                div()
                    .block()
                    .w(px(160.))
                    .child(StyledText::new("prefix ").with_highlights([(
                        0.."prefix ".len(),
                        HighlightStyle {
                            background_color: Some(highlight_color),
                            ..Default::default()
                        },
                    )]))
                    .child(div().inline_flex().size(px(18.)).align_baseline())
                    .child(div().inline_flex().size(px(18.)).align_middle())
                    .child(" suffix")
                    .on_children_prepainted(move |bounds, _, _| {
                        *captured_bounds.borrow_mut() = bounds;
                    })
                    .into_any_element()
            },
        );

        let placed_children = placed_children.borrow();
        assert_eq!(placed_children.len(), 4);
        let baseline_box = placed_children[1];
        let middle_box = placed_children[2];
        assert_eq!(baseline_box.size, size(px(18.), px(18.)));
        assert_eq!(middle_box.size, baseline_box.size);
        assert!(baseline_box.origin.x > px(10.));
        assert!(middle_box.origin.x >= baseline_box.right());
        assert!(middle_box.right() <= px(170.));
        assert!(baseline_box.origin.y >= px(20.));
        assert!(middle_box.origin.y > baseline_box.origin.y);
        assert!(middle_box.bottom() > baseline_box.bottom());
        assert!(
            window.update(|window, _| {
                window
                    .rendered_frame
                    .scene
                    .quads
                    .iter()
                    .any(|quad| quad.background.solid == highlight_color.into())
            }),
            "the highlighted inline text did not paint its background"
        );
    }

    #[gpui::test]
    fn default_block_preserves_grid_flow_and_hides_none_descendants(cx: &mut TestAppContext) {
        let hidden_prepainted = Rc::new(Cell::new(false));
        let hidden_prepaint = hidden_prepainted.clone();
        let window = cx.add_empty_window();

        window.draw(
            point(px(10.), px(20.)),
            size(px(200.), px(100.)),
            move |_, _| {
                div()
                    .w(px(200.))
                    .child(
                        div()
                            .grid()
                            .w_full()
                            .grid_cols(2)
                            .debug_selector(|| "grid".into())
                            .child(div().h(px(20.)).debug_selector(|| "grid-cell-1".into()))
                            .child(div().h(px(30.)).debug_selector(|| "grid-cell-2".into())),
                    )
                    .child(div().hidden().h(px(300.)).child(canvas(
                        move |_, _, _| hidden_prepaint.set(true),
                        |_, _, _, _| {},
                    )))
                    .child(div().h(px(10.)).debug_selector(|| "following-block".into()))
                    .into_any_element()
            },
        );

        let [grid, first_cell, second_cell, following] =
            ["grid", "grid-cell-1", "grid-cell-2", "following-block"].map(|selector| {
                window
                    .update(|window, _| window.rendered_frame.debug_bounds.get(selector).copied())
                    .unwrap_or_else(|| panic!("{selector} was not rendered"))
            });

        assert_eq!(first_cell.origin.y, grid.origin.y);
        assert_eq!(second_cell.origin.y, grid.origin.y);
        assert!(second_cell.origin.x > first_cell.origin.x);
        assert_eq!(following.origin.y, grid.bottom());
        assert!(!hidden_prepainted.get());
    }

    struct ChildPrepaintTestView {
        custom_order: bool,
        visits: Rc<RefCell<Vec<(usize, Point<Pixels>)>>>,
        callback_offsets: Rc<RefCell<Vec<Point<Pixels>>>>,
        scroll_handle: ScrollHandle,
    }

    impl Render for ChildPrepaintTestView {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            let children = (0..3).map(|index| {
                let visits = self.visits.clone();

                canvas(
                    move |_bounds, window, _cx| {
                        visits.borrow_mut().push((index, window.element_offset()));
                    },
                    |_bounds, _state, _window, _cx| {},
                )
                .w_full()
                .h(px(40.))
                .flex_shrink_0()
            });
            let mut element = div().size_full().flex().flex_col().children(children);

            if self.custom_order {
                let visits = self.visits.clone();
                let callback_offsets = self.callback_offsets.clone();
                element = element.with_dynamic_prepaint_order(move |window, _cx| {
                    assert!(visits.borrow().is_empty());
                    callback_offsets.borrow_mut().push(window.element_offset());

                    SmallVec::from_slice(&[2, 9, 0, 1])
                });
            }

            element
                .id("prepaint-order")
                .overflow_y_scroll()
                .track_scroll(&self.scroll_handle)
        }
    }

    #[gpui::test]
    fn child_prepaint_preserves_order_and_scroll_offset(cx: &mut TestAppContext) {
        let cx = cx.add_empty_window();
        let origin = point(px(20.), px(30.));
        let scroll_offset = point(px(0.), px(-15.));
        let expected_offset = origin + scroll_offset;

        for custom_order in [false, true] {
            let visits = Rc::new(RefCell::new(Vec::new()));
            let callback_offsets = Rc::new(RefCell::new(Vec::new()));
            let scroll_handle = ScrollHandle::new();
            scroll_handle.set_offset(scroll_offset);

            cx.draw(origin, size(px(50.), px(50.)), |_window, cx| {
                cx.new(|_cx| ChildPrepaintTestView {
                    custom_order,
                    visits: visits.clone(),
                    callback_offsets: callback_offsets.clone(),
                    scroll_handle,
                })
                .into_any_element()
            });

            let expected_order = if custom_order { [2, 0, 1] } else { [0, 1, 2] };
            assert_eq!(
                *visits.borrow(),
                expected_order.map(|index| (index, expected_offset)),
            );

            if custom_order {
                assert_eq!(*callback_offsets.borrow(), [expected_offset]);
            } else {
                assert!(callback_offsets.borrow().is_empty());
            }
        }
    }

    struct GroupHoverTestView {
        render_count: Rc<Cell<usize>>,
        anonymous_paint_count: Rc<Cell<usize>>,
        stateful_width: Rc<Cell<Pixels>>,
    }

    impl Render for GroupHoverTestView {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            self.render_count.set(self.render_count.get() + 1);
            let anonymous_paint_count = self.anonymous_paint_count.clone();
            let stateful_width = self.stateful_width.clone();
            div().size_full().child(
                div()
                    .ml(px(20.))
                    .mt(px(20.))
                    .size(px(50.))
                    .relative()
                    .group("hover-group")
                    .child(
                        div()
                            .absolute()
                            .size_full()
                            .invisible()
                            .group_hover("hover-group", |style| style.visible())
                            .child(canvas(
                                |_, _, _| {},
                                move |_, _, _, _| {
                                    anonymous_paint_count.set(anonymous_paint_count.get() + 1)
                                },
                            )),
                    )
                    .child(
                        div()
                            .id("stateful-group-hover-target")
                            .absolute()
                            .top_0()
                            .left_0()
                            .size(px(10.))
                            .group_hover("hover-group", |style| style.size(px(20.)))
                            .child(
                                canvas(
                                    move |bounds, _, _| stateful_width.set(bounds.size.width),
                                    |_, _, _, _| {},
                                )
                                .size_full(),
                            ),
                    ),
            )
        }
    }

    #[gpui::test]
    fn group_hover_styles_update_only_on_transitions(cx: &mut TestAppContext) {
        let render_count = Rc::new(Cell::new(0));
        let anonymous_paint_count = Rc::new(Cell::new(0));
        let stateful_width = Rc::new(Cell::new(px(0.)));
        let window = cx.add_window({
            let render_count = render_count.clone();
            let anonymous_paint_count = anonymous_paint_count.clone();
            let stateful_width = stateful_width.clone();
            move |_, _| GroupHoverTestView {
                render_count,
                anonymous_paint_count,
                stateful_width,
            }
        });
        let window = AnyWindowHandle::from(window);

        cx.update_window(window, |_, window, cx| window.draw(cx).clear(cx))
            .unwrap();
        assert_eq!(anonymous_paint_count.get(), 0);
        assert_eq!(stateful_width.get(), px(10.));

        let move_mouse = |cx: &mut TestAppContext, position| {
            cx.update_window(window, |_, window, cx| {
                window.simulate_mouse_move(position, cx)
            })
            .unwrap();
        };

        let initial_render_count = render_count.get();
        move_mouse(cx, point(px(25.), px(25.)));
        assert_eq!(render_count.get(), initial_render_count + 1);
        assert_eq!(anonymous_paint_count.get(), 1);
        assert_eq!(stateful_width.get(), px(20.));

        move_mouse(cx, point(px(30.), px(30.)));
        assert_eq!(render_count.get(), initial_render_count + 1);
        assert_eq!(anonymous_paint_count.get(), 1);
        assert_eq!(stateful_width.get(), px(20.));

        move_mouse(cx, point(px(5.), px(5.)));
        assert_eq!(render_count.get(), initial_render_count + 2);
        assert_eq!(anonymous_paint_count.get(), 1);
        assert_eq!(stateful_width.get(), px(10.));

        move_mouse(cx, point(px(10.), px(10.)));
        assert_eq!(render_count.get(), initial_render_count + 2);
        assert_eq!(anonymous_paint_count.get(), 1);
        assert_eq!(stateful_width.get(), px(10.));
    }

    struct HoverListenerLayoutTestView {
        target_left: Pixels,
        hover_transitions: Rc<RefCell<Vec<bool>>>,
    }

    impl Render for HoverListenerLayoutTestView {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            let hover_transitions = self.hover_transitions.clone();
            div().relative().size_full().child(
                div()
                    .id("hover-target")
                    .absolute()
                    .left(self.target_left)
                    .top_0()
                    .size(px(20.))
                    .on_click(|_, _, _| {})
                    .on_hover(move |is_hovered, _, _| {
                        hover_transitions.borrow_mut().push(*is_hovered);
                    }),
            )
        }
    }

    #[gpui::test]
    fn hover_listeners_update_when_layout_changes_under_stationary_mouse(cx: &mut TestAppContext) {
        let hover_transitions = Rc::new(RefCell::new(Vec::new()));
        let window = cx.add_window({
            let hover_transitions = hover_transitions.clone();
            move |_, _| HoverListenerLayoutTestView {
                target_left: px(40.),
                hover_transitions,
            }
        });
        let any_window = AnyWindowHandle::from(window);

        cx.update_window(any_window, |_, window, cx| {
            window.draw(cx).clear(cx);
            window.simulate_mouse_move(point(px(10.), px(10.)), cx);
        })
        .unwrap();
        assert!(hover_transitions.borrow().is_empty());

        window
            .update(cx, |view, _, cx| {
                view.target_left = px(0.);
                cx.notify();
            })
            .unwrap();
        cx.update_window(any_window, |_, window, cx| window.draw(cx).clear(cx))
            .unwrap();
        assert_eq!(*hover_transitions.borrow(), [true]);

        window
            .update(cx, |view, _, cx| {
                view.target_left = px(40.);
                cx.notify();
            })
            .unwrap();
        cx.update_window(any_window, |_, window, cx| window.draw(cx).clear(cx))
            .unwrap();
        assert_eq!(*hover_transitions.borrow(), [true, false]);
    }

    #[gpui::test]
    fn hover_listeners_remain_hovered_during_stationary_mouse_press(cx: &mut TestAppContext) {
        let hover_transitions = Rc::new(RefCell::new(Vec::new()));
        let window = cx.add_window({
            let hover_transitions = hover_transitions.clone();
            move |_, _| HoverListenerLayoutTestView {
                target_left: px(0.),
                hover_transitions,
            }
        });
        let any_window = AnyWindowHandle::from(window);
        let mouse_position = point(px(10.), px(10.));

        cx.update_window(any_window, |_, window, cx| {
            window.draw(cx).clear(cx);
            window.simulate_mouse_move(mouse_position, cx);
        })
        .unwrap();
        assert_eq!(*hover_transitions.borrow(), [true]);

        cx.update_window(any_window, |_, window, cx| {
            window.dispatch_event(
                MouseDownEvent {
                    position: mouse_position,
                    button: MouseButton::Left,
                    modifiers: Default::default(),
                    click_count: 1,
                    first_mouse: false,
                }
                .to_platform_input(),
                cx,
            );
            window.draw(cx).clear(cx);
        })
        .unwrap();
        assert_eq!(*hover_transitions.borrow(), [true]);

        cx.update_window(any_window, |_, window, cx| {
            window.dispatch_event(
                MouseUpEvent {
                    position: mouse_position,
                    button: MouseButton::Left,
                    modifiers: Default::default(),
                    click_count: 1,
                }
                .to_platform_input(),
                cx,
            );
            window.draw(cx).clear(cx);
        })
        .unwrap();
        assert_eq!(*hover_transitions.borrow(), [true]);
    }

    struct HoverRegionView {
        events: Rc<RefCell<Vec<Option<&'static str>>>>,
        projections: Rc<Cell<usize>>,
    }

    impl Render for HoverRegionView {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            let events = self.events.clone();
            let projections = self.projections.clone();
            div().size_full().child(
                div()
                    .id("hover-region-target")
                    .ml(px(20.))
                    .mt(px(20.))
                    .size(px(40.))
                    .on_hover_region(
                        move |position, bounds| {
                            projections.set(projections.get() + 1);
                            let x = position.x - bounds.left();
                            (x < px(15.))
                                .then_some("left")
                                .or_else(|| (x >= px(25.)).then_some("right"))
                        },
                        move |region, _, _| events.borrow_mut().push(*region),
                    ),
            )
        }
    }

    #[gpui::test]
    fn hover_region_transitions_once_and_stays_none_after_exit(cx: &mut TestAppContext) {
        let events = Rc::new(RefCell::new(Vec::new()));
        let projections = Rc::new(Cell::new(0));
        let window = cx.add_window({
            let events = events.clone();
            let projections = projections.clone();
            move |_, _| HoverRegionView {
                events,
                projections,
            }
        });
        let any_window = AnyWindowHandle::from(window);

        let mut expected = Vec::new();
        for (x, transition) in [
            (25., Some(Some("left"))),
            (30., None),
            (40., Some(None)), // The projector can decline while the element stays hovered.
            (42., None),
            (50., Some(Some("right"))),
            (50., None),
        ] {
            cx.update_window(any_window, |_, window, cx| {
                window.simulate_mouse_move(point(px(x), px(25.)), cx);
                window.draw(cx).clear(cx);
            })
            .unwrap();
            if let Some(transition) = transition {
                expected.push(transition);
            }
            assert_eq!(*events.borrow(), expected, "x={x}");
        }
        let count = projections.get();
        cx.update_window(any_window, |_, window, cx| {
            window.dispatch_event(
                // No final move: the window still holds the last inside position.
                MouseExitEvent::default().to_platform_input(),
                cx,
            );
            window.draw(cx).clear(cx);
            window.dispatch_event(MouseExitEvent::default().to_platform_input(), cx);
            window.draw(cx).clear(cx);
        })
        .unwrap();

        expected.push(None);
        assert_eq!(*events.borrow(), expected);
        assert_eq!(
            projections.get(),
            count,
            "exit and subsequent draws must not run the projection"
        );
        cx.update_window(any_window, |_, window, cx| {
            window.simulate_mouse_move(point(px(50.), px(25.)), cx);
            window.simulate_mouse_move(point(px(60.), px(25.)), cx);
            window.simulate_mouse_move(point(px(65.), px(25.)), cx);
        })
        .unwrap();
        expected.extend([Some("right"), None]);
        assert_eq!(
            *events.borrow(),
            expected,
            "reentry and half-open element bounds"
        );
    }

    #[derive(Debug, PartialEq)]
    enum HoverRegionEvent {
        Number(Option<u8>),
        Flag(Option<bool>),
    }

    struct MultipleHoverRegionsView {
        events: Rc<RefCell<Vec<HoverRegionEvent>>>,
    }

    impl Render for MultipleHoverRegionsView {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            let events = self.events.clone();
            let events_second = events.clone();
            div()
                .id("multiple-hover-regions")
                .size(px(40.))
                .on_hover_region(
                    |position, _| (position.x < px(20.)).then_some(1u8),
                    move |value, _, _| events.borrow_mut().push(HoverRegionEvent::Number(*value)),
                )
                .on_hover_region(
                    |position, _| (position.x >= px(20.)).then_some(true),
                    move |value, _, _| {
                        // Keep this listener independently typed from the first one.
                        events_second
                            .borrow_mut()
                            .push(HoverRegionEvent::Flag(*value));
                    },
                )
        }
    }

    #[gpui::test]
    fn hover_regions_keep_independent_state_when_rebuilt(cx: &mut TestAppContext) {
        let events = Rc::new(RefCell::new(Vec::new()));
        let view = cx.add_window({
            let events = events.clone();
            move |_, _| MultipleHoverRegionsView { events }
        });
        let any_window = AnyWindowHandle::from(view);
        cx.update_window(any_window, |_, window, cx| {
            window.draw(cx).clear(cx);
            window.simulate_mouse_move(point(px(5.), px(5.)), cx);
            window.simulate_mouse_move(point(px(25.), px(5.)), cx);
        })
        .unwrap();
        use HoverRegionEvent::{Flag, Number};
        assert_eq!(
            *events.borrow(),
            [Number(Some(1)), Number(None), Flag(Some(true))]
        );
        for _ in 0..3 {
            view.update(cx, |_, _, cx| cx.notify()).unwrap();
            cx.update_window(any_window, |_, window, cx| window.draw(cx).clear(cx))
                .unwrap();
            assert_eq!(
                *events.borrow(),
                [Number(Some(1)), Number(None), Flag(Some(true))]
            );
        }
        cx.update_window(any_window, |_, window, cx| {
            window.simulate_mouse_move(point(px(5.), px(5.)), cx);
            window.simulate_mouse_move(point(px(50.), px(5.)), cx);
        })
        .unwrap();
        assert_eq!(
            *events.borrow(),
            [
                Number(Some(1)),
                Number(None),
                Flag(Some(true)),
                Number(Some(1)),
                Flag(None),
                Number(None)
            ]
        );
    }

    struct ChangingHoverRegionView {
        enabled: bool,
        alternate_type: bool,
        events: Rc<RefCell<Vec<HoverRegionEvent>>>,
    }

    impl Render for ChangingHoverRegionView {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let events = self.events.clone();
            let target = div().id("changing-regions").size(px(40.));
            if !self.enabled {
                target
            } else if self.alternate_type {
                target.on_hover_region(
                    |_, _| Some(true),
                    move |value, _, _| events.borrow_mut().push(HoverRegionEvent::Flag(*value)),
                )
            } else {
                target.on_hover_region(
                    |_, _| Some(1u8),
                    move |value, _, _| events.borrow_mut().push(HoverRegionEvent::Number(*value)),
                )
            }
        }
    }

    #[gpui::test]
    fn hover_region_registration_replaces_types_and_discards_removed_state(
        cx: &mut TestAppContext,
    ) {
        let events = Rc::new(RefCell::new(Vec::new()));
        let window = cx.add_window({
            let events = events.clone();
            move |_, _| ChangingHoverRegionView {
                enabled: true,
                alternate_type: false,
                events,
            }
        });
        let any_window = AnyWindowHandle::from(window);
        cx.update_window(any_window, |_, window, cx| {
            window.simulate_mouse_move(point(px(5.), px(5.)), cx);
        })
        .unwrap();
        assert_eq!(*events.borrow(), [HoverRegionEvent::Number(Some(1))]);
        window
            .update(cx, |view, _, cx| {
                view.alternate_type = true;
                cx.notify();
            })
            .unwrap();
        cx.update_window(any_window, |_, window, cx| window.draw(cx).clear(cx))
            .unwrap();
        assert_eq!(
            *events.borrow(),
            [
                HoverRegionEvent::Number(Some(1)),
                HoverRegionEvent::Flag(Some(true))
            ]
        );
        window
            .update(cx, |view, _, cx| {
                view.enabled = false;
                cx.notify();
            })
            .unwrap();
        cx.update_window(any_window, |_, window, cx| window.draw(cx).clear(cx))
            .unwrap();
        window
            .update(cx, |view, _, cx| {
                view.enabled = true;
                cx.notify();
            })
            .unwrap();
        cx.update_window(any_window, |_, window, cx| window.draw(cx).clear(cx))
            .unwrap();
        assert_eq!(
            *events.borrow(),
            [
                HoverRegionEvent::Number(Some(1)),
                HoverRegionEvent::Flag(Some(true)),
                HoverRegionEvent::Flag(Some(true))
            ]
        );
    }

    struct TestTooltipView;

    impl Render for TestTooltipView {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            div().w(px(20.)).h(px(20.)).child("tooltip")
        }
    }

    type CapturedActiveTooltip = Rc<RefCell<Option<Weak<RefCell<Option<ActiveTooltip>>>>>>;

    struct TooltipCaptureElement {
        child: AnyElement,
        captured_active_tooltip: CapturedActiveTooltip,
    }

    impl IntoElement for TooltipCaptureElement {
        type Element = Self;

        fn into_element(self) -> Self::Element {
            self
        }
    }

    impl Element for TooltipCaptureElement {
        type RequestLayoutState = ();
        type PrepaintState = ();

        fn id(&self) -> Option<ElementId> {
            None
        }

        fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
            None
        }

        fn request_layout(
            &mut self,
            _id: Option<&GlobalElementId>,
            _inspector_id: Option<&InspectorElementId>,
            window: &mut Window,
            cx: &mut App,
        ) -> (LayoutId, Self::RequestLayoutState) {
            (self.child.request_layout(window, cx), ())
        }

        fn prepaint(
            &mut self,
            _id: Option<&GlobalElementId>,
            _inspector_id: Option<&InspectorElementId>,
            _bounds: Bounds<Pixels>,
            _request_layout: &mut Self::RequestLayoutState,
            window: &mut Window,
            cx: &mut App,
        ) -> Self::PrepaintState {
            self.child.prepaint(window, cx);
        }

        fn paint(
            &mut self,
            _id: Option<&GlobalElementId>,
            _inspector_id: Option<&InspectorElementId>,
            _bounds: Bounds<Pixels>,
            _request_layout: &mut Self::RequestLayoutState,
            _prepaint: &mut Self::PrepaintState,
            window: &mut Window,
            cx: &mut App,
        ) {
            self.child.paint(window, cx);
            window.with_global_id("target".into(), |global_id, window| {
                window.with_element_state::<InteractiveElementState, _>(
                    global_id,
                    |state, _window| {
                        let state = state.unwrap();
                        *self.captured_active_tooltip.borrow_mut() =
                            state.active_tooltip.as_ref().map(Rc::downgrade);
                        ((), state)
                    },
                )
            });
        }
    }

    struct TooltipOwner {
        captured_active_tooltip: CapturedActiveTooltip,
        show_delay_override: Option<Duration>,
    }

    impl Render for TooltipOwner {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            TooltipCaptureElement {
                child: div()
                    .size_full()
                    .child(
                        div()
                            .id("target")
                            .w(px(50.))
                            .h(px(50.))
                            .tooltip(|_, cx| cx.new(|_| TestTooltipView).into())
                            .when_some(self.show_delay_override, |this, delay| {
                                this.tooltip_show_delay(delay)
                            }),
                    )
                    .into_any_element(),
                captured_active_tooltip: self.captured_active_tooltip.clone(),
            }
        }
    }

    #[test]
    fn scroll_handle_aligns_wide_children_to_left_edge() {
        let handle = ScrollHandle::new();
        {
            let mut state = handle.0.borrow_mut();
            state.bounds = Bounds::new(point(px(0.), px(0.)), size(px(80.), px(20.)));
            state.child_bounds = vec![Bounds::new(point(px(25.), px(0.)), size(px(200.), px(20.)))];
            state.overflow.x = Overflow::Scroll;
            state.active_item = Some(ScrollActiveItem {
                index: 0,
                strategy: ScrollStrategy::default(),
            });
        }

        handle.scroll_to_active_item();

        assert_eq!(handle.offset().x, px(-25.));
    }

    #[test]
    fn scroll_handle_aligns_tall_children_to_top_edge() {
        let handle = ScrollHandle::new();
        {
            let mut state = handle.0.borrow_mut();
            state.bounds = Bounds::new(point(px(0.), px(0.)), size(px(20.), px(80.)));
            state.child_bounds = vec![Bounds::new(point(px(0.), px(25.)), size(px(20.), px(200.)))];
            state.overflow.y = Overflow::Scroll;
            state.active_item = Some(ScrollActiveItem {
                index: 0,
                strategy: ScrollStrategy::default(),
            });
        }

        handle.scroll_to_active_item();

        assert_eq!(handle.offset().y, px(-25.));
    }

    fn setup_tooltip_owner_test(
        show_delay_override: Option<Duration>,
    ) -> (
        TestAppContext,
        crate::AnyWindowHandle,
        CapturedActiveTooltip,
    ) {
        let mut test_app = TestAppContext::single();
        let captured_active_tooltip: CapturedActiveTooltip = Rc::new(RefCell::new(None));
        let window = test_app.add_window({
            let captured_active_tooltip = captured_active_tooltip.clone();
            move |_, _| TooltipOwner {
                captured_active_tooltip,
                show_delay_override,
            }
        });
        let any_window = window.into();

        test_app
            .update_window(any_window, |_, window, cx| {
                window.draw(cx).clear(cx);
            })
            .unwrap();

        test_app
            .update_window(any_window, |_, window, cx| {
                window.dispatch_event(
                    MouseMoveEvent {
                        position: point(px(10.), px(10.)),
                        modifiers: Default::default(),
                        pressed_button: None,
                    }
                    .to_platform_input(),
                    cx,
                );
            })
            .unwrap();

        test_app
            .update_window(any_window, |_, window, cx| {
                window.draw(cx).clear(cx);
            })
            .unwrap();

        (test_app, any_window, captured_active_tooltip)
    }

    #[test]
    fn tooltip_waiting_for_show_is_released_when_its_owner_disappears() {
        let (mut test_app, any_window, captured_active_tooltip) = setup_tooltip_owner_test(None);

        let weak_active_tooltip = captured_active_tooltip.borrow().clone().unwrap();
        let active_tooltip = weak_active_tooltip.upgrade().unwrap();
        assert!(matches!(
            active_tooltip.borrow().as_ref(),
            Some(ActiveTooltip::WaitingForShow { .. })
        ));

        test_app
            .update_window(any_window, |_, window, _| {
                window.remove_window();
            })
            .unwrap();
        test_app.run_until_parked();
        drop(active_tooltip);

        assert!(weak_active_tooltip.upgrade().is_none());
    }

    #[test]
    fn tooltip_respects_custom_show_delay() {
        let extra_delay = Duration::from_secs(1);
        let show_delay_override = DEFAULT_TOOLTIP_SHOW_DELAY + extra_delay;
        let (mut test_app, _any_window, captured_active_tooltip) =
            setup_tooltip_owner_test(Some(show_delay_override));

        let weak_active_tooltip = captured_active_tooltip.borrow().clone().unwrap();
        let active_tooltip = weak_active_tooltip.upgrade().unwrap();

        test_app
            .dispatcher
            .advance_clock(DEFAULT_TOOLTIP_SHOW_DELAY);
        test_app.run_until_parked();

        assert!(matches!(
            active_tooltip.borrow().as_ref(),
            Some(ActiveTooltip::WaitingForShow { .. })
        ));

        test_app.dispatcher.advance_clock(extra_delay);
        test_app.run_until_parked();

        assert!(matches!(
            active_tooltip.borrow().as_ref(),
            Some(ActiveTooltip::Visible { .. })
        ));
    }

    #[test]
    fn tooltip_is_released_when_its_owner_disappears() {
        let (mut test_app, any_window, captured_active_tooltip) = setup_tooltip_owner_test(None);

        let weak_active_tooltip = captured_active_tooltip.borrow().clone().unwrap();
        let active_tooltip = weak_active_tooltip.upgrade().unwrap();

        test_app
            .dispatcher
            .advance_clock(DEFAULT_TOOLTIP_SHOW_DELAY);
        test_app.run_until_parked();

        assert!(matches!(
            active_tooltip.borrow().as_ref(),
            Some(ActiveTooltip::Visible { .. })
        ));

        test_app
            .update_window(any_window, |_, window, _| {
                window.remove_window();
            })
            .unwrap();
        test_app.run_until_parked();
        drop(active_tooltip);

        assert!(weak_active_tooltip.upgrade().is_none());
    }

    #[test]
    fn tooltip_hides_after_mouse_leaves_origin() {
        let (mut test_app, any_window, captured_active_tooltip) = setup_tooltip_owner_test(None);

        let weak_active_tooltip = captured_active_tooltip.borrow().clone().unwrap();
        let active_tooltip = weak_active_tooltip.upgrade().unwrap();

        test_app
            .dispatcher
            .advance_clock(DEFAULT_TOOLTIP_SHOW_DELAY);
        test_app.run_until_parked();

        assert!(matches!(
            active_tooltip.borrow().as_ref(),
            Some(ActiveTooltip::Visible { .. })
        ));

        test_app
            .update_window(any_window, |_, window, cx| {
                window.dispatch_event(
                    MouseMoveEvent {
                        position: point(px(75.), px(75.)),
                        modifiers: Default::default(),
                        pressed_button: None,
                    }
                    .to_platform_input(),
                    cx,
                );
            })
            .unwrap();

        assert!(active_tooltip.borrow().is_none());
    }

    struct MouseDownOutOwner {
        mouse_down_out_count: Rc<RefCell<usize>>,
    }

    impl Render for MouseDownOutOwner {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            let mouse_down_out_count = self.mouse_down_out_count.clone();
            div()
                .size_full()
                .child(div().id("target").w(px(50.)).h(px(50.)).on_mouse_down_out(
                    move |_, _, _| {
                        *mouse_down_out_count.borrow_mut() += 1;
                    },
                ))
        }
    }

    #[test]
    fn mouse_down_out_is_suppressed_while_window_prompt_is_active() {
        let mut test_app = TestAppContext::single();
        let mouse_down_out_count = Rc::new(RefCell::new(0));
        let window = test_app.add_window({
            let mouse_down_out_count = mouse_down_out_count.clone();
            move |_, _| MouseDownOutOwner {
                mouse_down_out_count,
            }
        });
        let any_window: AnyWindowHandle = window.into();

        fn dispatch_mouse_down_outside_target(
            test_app: &mut TestAppContext,
            any_window: AnyWindowHandle,
        ) {
            test_app
                .update_window(any_window, |_, window, cx| {
                    window.dispatch_event(
                        MouseDownEvent {
                            position: point(px(75.), px(75.)),
                            button: MouseButton::Left,
                            modifiers: Default::default(),
                            click_count: 1,
                            first_mouse: false,
                        }
                        .to_platform_input(),
                        cx,
                    );
                })
                .unwrap();
        }

        test_app
            .update_window(any_window, |_, window, cx| {
                window.draw(cx).clear(cx);
            })
            .unwrap();

        dispatch_mouse_down_outside_target(&mut test_app, any_window);
        assert_eq!(
            *mouse_down_out_count.borrow(),
            1,
            "mouse down outside the element should fire mouse-down-out listeners"
        );

        test_app
            .update_window(any_window, |_, window, cx| {
                cx.set_prompt_builder(crate::fallback_prompt_renderer);
                let _receiver =
                    window.prompt(crate::PromptLevel::Warning, "message", None, &["Ok"], cx);
                assert!(window.has_active_prompt());
                window.draw(cx).clear(cx);
            })
            .unwrap();

        dispatch_mouse_down_outside_target(&mut test_app, any_window);
        assert_eq!(
            *mouse_down_out_count.borrow(),
            1,
            "mouse down over an active prompt should not fire mouse-down-out listeners"
        );
    }

    #[test]
    fn test_accessibility_id_builder_writes_author_id() {
        let mut element = div()
            .id("buffer-font-size")
            .accessibility_id("settings.buffer-font-size");
        let mut node = accesskit::Node::new(accesskit::Role::SpinButton);

        element.interactivity().write_a11y_info(&mut node);

        assert_eq!(node.author_id(), Some("settings.buffer-font-size"));
    }

    #[test]
    fn test_write_a11y_info_string_and_numeric_properties() {
        let mut interactivity = Interactivity::default();
        interactivity.aria.author_id = Some("settings.buffer-font-size".into());
        interactivity.aria.label = Some("Buffer Font Size".into());
        interactivity.aria.value = Some("15".into());
        interactivity.aria.placeholder = Some("Search".into());
        interactivity.aria.numeric_value = Some(15.0);
        interactivity.aria.min_numeric_value = Some(6.0);
        interactivity.aria.max_numeric_value = Some(72.0);
        interactivity.aria.numeric_value_step = Some(1.0);

        let mut node = accesskit::Node::new(accesskit::Role::SpinButton);
        interactivity.write_a11y_info(&mut node);

        assert_eq!(node.author_id(), Some("settings.buffer-font-size"));
        assert_eq!(node.label(), Some("Buffer Font Size"));
        assert_eq!(node.value(), Some("15"));
        assert_eq!(node.placeholder(), Some("Search"));
        assert_eq!(node.numeric_value(), Some(15.0));
        assert_eq!(node.min_numeric_value(), Some(6.0));
        assert_eq!(node.max_numeric_value(), Some(72.0));
        assert_eq!(node.numeric_value_step(), Some(1.0));
    }

    #[test]
    fn test_write_a11y_info_disabled_modal_and_live_region() {
        let mut interactivity = Interactivity::default();
        interactivity.aria.disabled = Some(true);
        interactivity.aria.modal = Some(true);
        interactivity.aria.live = Some(accesskit::Live::Assertive);
        interactivity.aria.live_atomic = Some(true);

        let mut node = accesskit::Node::new(accesskit::Role::Dialog);
        interactivity.write_a11y_info(&mut node);

        assert!(node.is_disabled());
        assert!(node.is_modal());
        assert_eq!(node.live(), Some(accesskit::Live::Assertive));
        assert!(node.is_live_atomic());
    }

    #[test]
    fn test_write_a11y_info_disabled_and_modal_false_clear_the_flags() {
        // `Some(false)` must explicitly clear a flag-style AccessKit property,
        // not merely leave it at its (already-cleared) default: a caller that
        // re-renders a node from a previous state relies on `Some(false)`
        // overriding a flag that a prior frame set to `true`.
        let mut interactivity = Interactivity::default();
        interactivity.aria.disabled = Some(false);
        interactivity.aria.modal = Some(false);
        interactivity.aria.live_atomic = Some(false);

        let mut node = accesskit::Node::new(accesskit::Role::Dialog);
        node.set_disabled();
        node.set_modal();
        node.set_live_atomic();

        interactivity.write_a11y_info(&mut node);

        assert!(!node.is_disabled());
        assert!(!node.is_modal());
        assert!(!node.is_live_atomic());
    }

    /// Two focusable, clickable elements ("a" and "b") used to exercise the
    /// Enter/Space -> synthesized click press/release pairing.
    struct KeyboardActivationTest {
        focus_a: FocusHandle,
        focus_b: FocusHandle,
        clicks: Rc<RefCell<Vec<&'static str>>>,
    }

    impl Render for KeyboardActivationTest {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            let clicks_a = self.clicks.clone();
            let clicks_b = self.clicks.clone();
            div()
                .size_full()
                .child(
                    div()
                        .id("a")
                        .w(px(50.))
                        .h(px(50.))
                        .track_focus(&self.focus_a)
                        .on_click(move |_, _, _| clicks_a.borrow_mut().push("a")),
                )
                .child(
                    div()
                        .id("b")
                        .w(px(50.))
                        .h(px(50.))
                        .track_focus(&self.focus_b)
                        .on_click(move |_, _, _| clicks_b.borrow_mut().push("b")),
                )
        }
    }

    fn setup_keyboard_activation_test() -> (
        TestAppContext,
        AnyWindowHandle,
        Rc<RefCell<Vec<&'static str>>>,
        FocusHandle,
        FocusHandle,
    ) {
        let mut cx = TestAppContext::single();
        let (focus_a, focus_b) = cx.update(|cx| (cx.focus_handle(), cx.focus_handle()));
        let clicks: Rc<RefCell<Vec<&'static str>>> = Rc::new(RefCell::new(Vec::new()));
        let window = cx.add_window({
            let focus_a = focus_a.clone();
            let focus_b = focus_b.clone();
            let clicks = clicks.clone();
            move |_, _| KeyboardActivationTest {
                focus_a,
                focus_b,
                clicks,
            }
        });
        (cx, window.into(), clicks, focus_a, focus_b)
    }

    /// Move focus to `handle`, flush effects, then paint so the newly focused
    /// element registers its key handlers for the next dispatched event.
    fn focus_and_draw(cx: &mut TestAppContext, window: AnyWindowHandle, handle: &FocusHandle) {
        cx.update_window(window, |_, window, cx| window.focus(handle, cx))
            .unwrap();
        cx.run_until_parked();
        cx.update_window(window, |_, window, cx| {
            window.draw(cx).clear(cx);
        })
        .unwrap();
    }

    fn key_down(cx: &mut TestAppContext, window: AnyWindowHandle, key: &str) {
        let keystroke = Keystroke::parse(key).unwrap();
        cx.update_window(window, |_, window, cx| {
            window.dispatch_event(
                KeyDownEvent {
                    keystroke,
                    is_held: false,
                    prefer_character_input: false,
                }
                .to_platform_input(),
                cx,
            );
        })
        .unwrap();
    }

    fn key_up(cx: &mut TestAppContext, window: AnyWindowHandle, key: &str) {
        let keystroke = Keystroke::parse(key).unwrap();
        cx.update_window(window, |_, window, cx| {
            window.dispatch_event(KeyUpEvent { keystroke }.to_platform_input(), cx);
        })
        .unwrap();
    }

    /// Pressing and releasing Enter on the same focused element fires a click.
    #[test]
    fn keyboard_activation_fires_click_on_same_element() {
        let (mut cx, window, clicks, focus_a, _focus_b) = setup_keyboard_activation_test();

        focus_and_draw(&mut cx, window, &focus_a);
        key_down(&mut cx, window, "enter");
        key_up(&mut cx, window, "enter");

        assert_eq!(*clicks.borrow(), vec!["a"]);
    }

    /// A key-down whose key-up lands on a *different* element (because focus
    /// moved in between) must not leak a synthesized click onto the newly
    /// focused element. This is the core regression: previously the key-up
    /// handler fired unconditionally on whatever was focused at key-up time.
    #[test]
    fn keyboard_activation_does_not_leak_across_focus_change() {
        let (mut cx, window, clicks, focus_a, focus_b) = setup_keyboard_activation_test();

        // Enter pressed while "a" is focused...
        focus_and_draw(&mut cx, window, &focus_a);
        key_down(&mut cx, window, "enter");

        // ...focus moves to "b" before the release (as a confirm action would)...
        focus_and_draw(&mut cx, window, &focus_b);
        key_up(&mut cx, window, "enter");

        // ...so neither element is clicked: "a" never saw the up, and "b"
        // never saw the down.
        assert!(clicks.borrow().is_empty(), "clicks: {:?}", clicks.borrow());
    }

    /// A keydown whose flag is left pending because focus moved away before
    /// the keyup must not fire a click when focus later *returns* to the same
    /// element (the menu trigger reopening case). The stamped focus generation
    /// no longer matches, so the stale pending state is ignored.
    #[test]
    fn keyboard_activation_does_not_leak_when_focus_returns() {
        let (mut cx, window, clicks, focus_a, focus_b) = setup_keyboard_activation_test();

        // Enter pressed on "a"...
        focus_and_draw(&mut cx, window, &focus_a);
        key_down(&mut cx, window, "enter");

        // ...focus leaves "a" before its keyup (so the pending state is never
        // consumed), then comes back to "a"...
        focus_and_draw(&mut cx, window, &focus_b);
        focus_and_draw(&mut cx, window, &focus_a);
        key_up(&mut cx, window, "enter");

        // ...and the now-stale pending keydown must not fire a click.
        assert!(clicks.borrow().is_empty(), "clicks: {:?}", clicks.borrow());
    }

    /// A non-activation key *released* during the press must cancel the pending
    /// activation. For the sequence escape-down, space-down, escape-up,
    /// space-up the space forms a clean down/up pair, but the intervening
    /// escape-up means this isn't a plain space activation, so no click fires.
    #[test]
    fn keyboard_activation_cleared_by_intervening_key_release() {
        let (mut cx, window, clicks, focus_a, _focus_b) = setup_keyboard_activation_test();

        focus_and_draw(&mut cx, window, &focus_a);
        key_down(&mut cx, window, "escape");
        key_down(&mut cx, window, "space");
        key_up(&mut cx, window, "escape");
        key_up(&mut cx, window, "space");

        assert!(clicks.borrow().is_empty(), "clicks: {:?}", clicks.borrow());
    }

    /// The flag is a single activation marker, not keyed by which activation
    /// key was used, so a Space down paired with an Enter up on the same
    /// element still fires a click.
    #[test]
    fn keyboard_activation_does_not_distinguish_space_and_enter() {
        let (mut cx, window, clicks, focus_a, _focus_b) = setup_keyboard_activation_test();

        focus_and_draw(&mut cx, window, &focus_a);
        key_down(&mut cx, window, "space");
        key_up(&mut cx, window, "enter");

        assert_eq!(*clicks.borrow(), vec!["a"]);
    }

    /// A non-activation key pressed between the activation down and up clears
    /// the pending flag, suppressing the click.
    #[test]
    fn keyboard_activation_cleared_by_intervening_keydown() {
        let (mut cx, window, clicks, focus_a, _focus_b) = setup_keyboard_activation_test();

        focus_and_draw(&mut cx, window, &focus_a);
        key_down(&mut cx, window, "enter");
        key_down(&mut cx, window, "a");
        key_up(&mut cx, window, "enter");

        assert!(clicks.borrow().is_empty(), "clicks: {:?}", clicks.borrow());
    }

    /// A modified Enter (e.g. cmd-enter) is not treated as an activation key,
    /// so it neither sets the pending flag nor fires a click on release.
    #[test]
    fn keyboard_activation_ignores_modified_keys() {
        let (mut cx, window, clicks, focus_a, _focus_b) = setup_keyboard_activation_test();

        focus_and_draw(&mut cx, window, &focus_a);
        key_down(&mut cx, window, "cmd-enter");
        key_up(&mut cx, window, "cmd-enter");

        assert!(clicks.borrow().is_empty(), "clicks: {:?}", clicks.borrow());
    }

    /// Two sibling tab groups, each a focusable container that is *not* itself a
    /// tab stop and holds a single tab stop. Mirrors how the title bar and
    /// status bar expose their controls as ARIA toolbars.
    struct TabGroupFocus {
        group_a: FocusHandle,
        item_a: FocusHandle,
        group_b: FocusHandle,
        item_b: FocusHandle,
    }

    impl Render for TabGroupFocus {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            fn group(container: &FocusHandle, item: &FocusHandle) -> Div {
                div()
                    .track_focus(container)
                    .tab_group()
                    .child(div().track_focus(item))
            }
            div()
                .child(group(&self.group_a, &self.item_a))
                .child(group(&self.group_b, &self.item_b))
        }
    }

    /// Focusing a tab-group container and pressing Tab (`focus_next`) must move
    /// focus to the first tab stop *inside that container*, as documented on
    /// [`InteractiveElement::tab_stop`].
    #[test]
    fn focus_next_from_tab_group_container_enters_that_group() {
        let mut cx = TestAppContext::single();
        let (group_a, item_a, group_b, item_b) = cx.update(|cx| {
            (
                cx.focus_handle(),
                cx.focus_handle().tab_stop(true),
                cx.focus_handle(),
                cx.focus_handle().tab_stop(true),
            )
        });
        let window: AnyWindowHandle = cx
            .add_window({
                let (group_a, item_a, group_b, item_b) =
                    (group_a, item_a, group_b.clone(), item_b.clone());
                move |_, _| TabGroupFocus {
                    group_a,
                    item_a,
                    group_b,
                    item_b,
                }
            })
            .into();
        cx.update_window(window, |_, window, cx| window.draw(cx).clear(cx))
            .unwrap();

        // Focus the *second* group's container, then advance like Tab would.
        let focused = cx
            .update_window(window, |_, window, cx| {
                window.focus(&group_b, cx);
                window.focus_next(cx);
                window.focused(cx).map(|handle| handle.id)
            })
            .unwrap();

        assert_eq!(focused, Some(item_b.id));
    }

    #[gpui::test]
    fn test_fractional_padding_does_not_make_a_fitting_container_scrollable(
        cx: &mut TestAppContext,
    ) {
        struct PaddedContainer {
            scroll_handle: ScrollHandle,
        }

        impl Render for PaddedContainer {
            fn render(
                &mut self,
                _window: &mut Window,
                _cx: &mut Context<Self>,
            ) -> impl IntoElement {
                // 4.25px of padding snaps to 4px in layout, so a 42px child
                // fits the 50px box exactly.
                div().size_full().child(
                    div()
                        .id("container")
                        .h(px(50.))
                        .w(px(100.))
                        .py(px(4.25))
                        .overflow_y_scroll()
                        .track_scroll(&self.scroll_handle)
                        .child(div().w_full().h(px(42.))),
                )
            }
        }

        let scroll_handle = ScrollHandle::new();
        let window: AnyWindowHandle = cx
            .add_window({
                let scroll_handle = scroll_handle.clone();
                move |_, _| PaddedContainer { scroll_handle }
            })
            .into();
        cx.update_window(window, |_, window, cx| window.draw(cx).clear(cx))
            .unwrap();

        assert_eq!(scroll_handle.max_offset().y, px(0.));
    }

    struct ContentSizedGrid;

    impl Render for ContentSizedGrid {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            let widths = [px(100.), px(200.), px(50.)];
            div().size_full().child(
                div()
                    .w_full()
                    .grid()
                    .grid_cols_max_content(widths.len() as u16)
                    .children(widths.into_iter().enumerate().map(|(index, width)| {
                        div()
                            .debug_selector(move || format!("cell-{index}"))
                            .w(width)
                            .h(px(10.))
                    })),
            )
        }
    }

    #[gpui::test]
    fn grid_cols_max_content_sizes_columns_to_their_content(cx: &mut TestAppContext) {
        let window = cx.add_window(|_, _| ContentSizedGrid);
        cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))
            .unwrap();

        let mut bounds = |selector: &'static str| {
            cx.update_window(window.into(), |_, window, _| {
                window.rendered_frame.debug_bounds.get(selector).copied()
            })
            .unwrap()
            .unwrap_or_else(|| panic!("{selector} was not rendered"))
        };

        assert_eq!(bounds("cell-0").origin.x, px(0.));
        assert_eq!(bounds("cell-1").origin.x, px(100.));
        assert_eq!(bounds("cell-2").origin.x, px(300.));
    }
}
