use crate::{
    Bounds, Capslock, Context, Empty, IntoElement, Keystroke, LongPressEvent, Modifiers, Pixels,
    Point, Render, TouchDragEvent, Window, point, seal::Sealed,
};
use smallvec::SmallVec;
use std::{any::Any, fmt::Debug, ops::Deref, path::PathBuf};

/// A platform input event that GPUI can dispatch to a window.
///
/// Implemented by the event structs in this module. Each one converts into the
/// matching [`PlatformInput`] variant. The trait is sealed, so only GPUI can
/// implement it.
pub trait InputEvent: Sealed + 'static {
    /// Converts this event into the [`PlatformInput`] variant that wraps it.
    fn to_platform_input(self) -> PlatformInput;
}

/// An input event from the keyboard: [`KeyDownEvent`], [`KeyUpEvent`] or
/// [`ModifiersChangedEvent`].
pub trait KeyEvent: InputEvent {}

/// An input event from a pointer or touch: mouse buttons, movement, scrolling,
/// pinch, pressure, file drops and the gesture events.
pub trait MouseEvent: InputEvent {}

/// A gesture event recognized from touch input, such as [`PinchEvent`],
/// [`LongPressEvent`] or [`TouchDragEvent`].
pub trait GestureEvent: InputEvent {}

/// A key press, sent when the platform reports that a key went down.
///
/// GPUI first matches the keystroke against your key bindings and dispatches
/// matching actions. Unless an action stops propagation, the event then reaches
/// [`InteractiveElement::on_key_down`](crate::InteractiveElement::on_key_down)
/// listeners on the focus path. The [keyboard guide](crate::_documentation::guides::keyboard)
/// explains the order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KeyDownEvent {
    /// The key and modifiers that were pressed.
    pub keystroke: Keystroke,

    /// Whether this is an automatic repeat from a key that is still held down.
    pub is_held: bool,

    /// Whether the focused text input should receive this keystroke as text
    /// instead of matching key bindings.
    ///
    /// Set on a keystroke that GPUI replays after a multi-stroke binding fails to
    /// complete. Bindings are skipped only if the focused input handler accepts
    /// text input.
    pub prefer_character_input: bool,
}

impl Sealed for KeyDownEvent {}
impl InputEvent for KeyDownEvent {
    fn to_platform_input(self) -> PlatformInput {
        PlatformInput::KeyDown(self)
    }
}
impl KeyEvent for KeyDownEvent {}

/// A key release, sent when the platform reports that a key went up.
///
/// Dispatched to [`InteractiveElement::on_key_up`](crate::InteractiveElement::on_key_up) listeners on the focus path.
#[derive(Clone, Debug)]
pub struct KeyUpEvent {
    /// The key and modifiers that were released.
    pub keystroke: Keystroke,
}

impl Sealed for KeyUpEvent {}
impl InputEvent for KeyUpEvent {
    fn to_platform_input(self) -> PlatformInput {
        PlatformInput::KeyUp(self)
    }
}
impl KeyEvent for KeyUpEvent {}

/// A change to the modifier keys (Shift, Control, Alt, Platform, Function) or
/// Caps Lock, sent without a key press.
///
/// Dereferences to [`Modifiers`], so `event.shift` reads the new Shift state.
/// Modifier-only bindings, such as one bound to `"shift"`, are matched from
/// this event.
#[derive(Clone, Debug, Default)]
pub struct ModifiersChangedEvent {
    /// The modifier keys held after the change.
    pub modifiers: Modifiers,
    /// The Caps Lock state after the change.
    pub capslock: Capslock,
}

impl Sealed for ModifiersChangedEvent {}
impl InputEvent for ModifiersChangedEvent {
    fn to_platform_input(self) -> PlatformInput {
        PlatformInput::ModifiersChanged(self)
    }
}
impl KeyEvent for ModifiersChangedEvent {}

impl Deref for ModifiersChangedEvent {
    type Target = Modifiers;

    fn deref(&self) -> &Self::Target {
        &self.modifiers
    }
}

/// The lifecycle stage of a touch, scroll or pinch event.
///
/// Based on the winit enum of the same name. A gesture reports `Started` when it
/// begins, zero or more `Moved` events, and then `Ended` or `Cancelled`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TouchPhase {
    /// The touch, scroll or pinch began.
    Started,
    /// The touch, scroll or pinch changed position or scale.
    #[default]
    Moved,
    /// The touch, scroll or pinch finished normally.
    Ended,
    /// The system ended the gesture abnormally. Consumers must fully unwind any
    /// in-progress interaction, treating the touch as if it never committed.
    Cancelled,
}

/// Identifies one touch (finger or stylus contact) from its
/// [`TouchPhase::Started`] event through its [`TouchPhase::Ended`] or
/// [`TouchPhase::Cancelled`] event.
///
/// The value is assigned by the platform and is opaque. A platform window must
/// not reuse an identifier for a later touch.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TouchId(pub u64);

/// A raw touch sample from a touch screen, before gesture recognition.
///
/// GPUI does not deliver raw touches to element listeners. The gesture
/// recognizer reads them and reports what they mean: a tap becomes a
/// synthesized left mouse down and up, a pan becomes [`ScrollWheelEvent`]s, a
/// held touch becomes [`LongPressEvent`], and a drag claimed by an element
/// becomes [`TouchDragEvent`]. See the
/// [mouse and touch guide](crate::_documentation::guides::mouse_and_touch).
#[derive(Clone, Debug, Default)]
pub struct TouchEvent {
    /// The touch this sample belongs to.
    pub id: TouchId,
    /// Whether the touch started, moved, ended or was cancelled.
    pub phase: TouchPhase,
    /// The touch position in window coordinates, in logical pixels.
    pub position: Point<Pixels>,
    /// The position the platform predicts the touch will reach about one frame
    /// later, in the same coordinates as `position`. Only set for
    /// [`TouchPhase::Moved`] samples, and only when the platform can predict.
    ///
    /// This is latency compensation. It can change how far a pan scrolls within
    /// a frame. It never affects hit testing, gesture classification or
    /// velocity, and later samples correct any error it introduces.
    pub predicted_position: Option<Point<Pixels>>,
    /// The normalized touch force from `0.0` to `1.0`, if the hardware reports it.
    pub force: Option<f32>,
}

impl Sealed for TouchEvent {}
impl InputEvent for TouchEvent {
    fn to_platform_input(self) -> PlatformInput {
        PlatformInput::Touch(self)
    }
}

/// A mouse button press, sent to [`InteractiveElement::on_mouse_down`](crate::InteractiveElement::on_mouse_down) listeners.
///
/// Use it to react as soon as the button goes down. Use [`MouseClickEvent`]
/// instead when you need the completed click.
#[derive(Clone, Debug, Default)]
pub struct MouseDownEvent {
    /// The button that was pressed.
    pub button: MouseButton,

    /// The pointer position in window coordinates, in logical pixels.
    pub position: Point<Pixels>,

    /// The modifier keys held when the button was pressed.
    pub modifiers: Modifiers,

    /// The number of presses in a row at this spot, starting at 1. A double
    /// click has a count of 2.
    pub click_count: usize,

    /// Whether this press activated an inactive window. Set on macOS and Wayland
    /// when the window accepts the press that brings it forward.
    pub first_mouse: bool,
}

impl Sealed for MouseDownEvent {}
impl InputEvent for MouseDownEvent {
    fn to_platform_input(self) -> PlatformInput {
        PlatformInput::MouseDown(self)
    }
}
impl MouseEvent for MouseDownEvent {}

impl MouseDownEvent {
    /// Returns `true` if this press should focus the element.
    ///
    /// Only left button presses focus. Right and middle presses leave focus where
    /// it is.
    pub fn is_focusing(&self) -> bool {
        match self.button {
            MouseButton::Left => true,
            _ => false,
        }
    }
}

/// A mouse button release, sent to [`InteractiveElement::on_mouse_up`](crate::InteractiveElement::on_mouse_up) listeners.
///
/// A release alone is not a click. Use [`MouseClickEvent`] or
/// [`StatefulInteractiveElement::on_click`](crate::StatefulInteractiveElement::on_click) for a completed click.
#[derive(Clone, Debug, Default)]
pub struct MouseUpEvent {
    /// The button that was released.
    pub button: MouseButton,

    /// The pointer position in window coordinates, in logical pixels.
    pub position: Point<Pixels>,

    /// The modifier keys held when the button was released.
    pub modifiers: Modifiers,

    /// The number of presses in a row, as reported by the platform with this release.
    pub click_count: usize,
}

impl Sealed for MouseUpEvent {}
impl InputEvent for MouseUpEvent {
    fn to_platform_input(self) -> PlatformInput {
        PlatformInput::MouseUp(self)
    }
}

impl MouseEvent for MouseUpEvent {}

impl MouseUpEvent {
    /// Returns `true` if this release should focus the element.
    ///
    /// Only left button releases focus.
    pub fn is_focusing(&self) -> bool {
        match self.button {
            MouseButton::Left => true,
            _ => false,
        }
    }
}

/// A completed mouse click: the press and the release of one button on one
/// element.
///
/// GPUI sends it to [`StatefulInteractiveElement::on_click`](crate::StatefulInteractiveElement::on_click) for the left button and to
/// [`StatefulInteractiveElement::on_aux_click`](crate::StatefulInteractiveElement::on_aux_click) for every other button. It is wrapped in
/// [`ClickEvent::Mouse`].
#[derive(Clone, Debug, Default)]
pub struct MouseClickEvent {
    /// The press that started the click.
    pub down: MouseDownEvent,

    /// The release that completed the click.
    pub up: MouseUpEvent,
}

/// How hard a force-sensitive trackpad is pressed.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum PressureStage {
    /// No pressure.
    #[default]
    Zero,
    /// Normal click pressure.
    Normal,
    /// Firm pressure, enough to trigger a force click.
    Force,
}

/// A pressure update from a force-sensitive trackpad, sent as the user presses
/// harder or eases off. Only macOS trackpads report it today.
#[derive(Debug, Clone, Default)]
pub struct MousePressureEvent {
    /// The pressure within the current stage, from `0.0` to `1.0`.
    pub pressure: f32,
    /// The pressure stage the trackpad is in.
    pub stage: PressureStage,
    /// The pointer position in window coordinates, in logical pixels.
    pub position: Point<Pixels>,
    /// The modifier keys held when the pressure changed.
    pub modifiers: Modifiers,
}

impl Sealed for MousePressureEvent {}
impl InputEvent for MousePressureEvent {
    fn to_platform_input(self) -> PlatformInput {
        PlatformInput::MousePressure(self)
    }
}
impl MouseEvent for MousePressureEvent {}

/// A click triggered from the keyboard: Enter or Space on a focused element.
///
/// Keyboard clicks carry no pointer data. The element's bounds stand in for the
/// position.
#[derive(Clone, Debug, Default)]
pub struct KeyboardClickEvent {
    /// The key that triggered the click.
    pub button: KeyboardButton,

    /// The bounds of the clicked element in window coordinates, in logical pixels.
    pub bounds: Bounds<Pixels>,
}

/// A tap, or a long press, recognized on a touch screen.
///
/// GPUI does not produce this event yet. Taps currently reach click listeners
/// as [`ClickEvent::Mouse`] with a left button press and release.
#[derive(Clone, Debug, Default)]
pub struct TouchClickEvent {
    /// The position of the tap in window coordinates, in logical pixels.
    pub position: Point<Pixels>,
    /// The number of consecutive taps at this spot. A double tap has a count of 2.
    pub tap_count: usize,
    /// Whether this was a long press rather than a tap. A long press is the touch
    /// equivalent of a right click.
    pub long_press: bool,
}

/// A completed click from the mouse, keyboard or touch.
///
/// Click listeners receive this type. [`StatefulInteractiveElement::on_click`](crate::StatefulInteractiveElement::on_click) handles
/// the primary action, and [`StatefulInteractiveElement::on_aux_click`](crate::StatefulInteractiveElement::on_aux_click) handles the
/// other mouse buttons. Use the methods on this type, such as
/// [`ClickEvent::is_secondary`], to tell the cases apart.
///
/// # Examples
///
/// ```
/// use gpui::{ClickEvent, KeyboardButton, KeyboardClickEvent, Modifiers, MouseButton,
///     MouseClickEvent, MouseDownEvent, MouseUpEvent};
///
/// let right_click = ClickEvent::Mouse(MouseClickEvent {
///     down: MouseDownEvent { button: MouseButton::Right, ..Default::default() },
///     up: MouseUpEvent { button: MouseButton::Right, ..Default::default() },
/// });
/// assert!(right_click.is_right_click());
/// assert!(right_click.is_secondary());
/// assert!(!right_click.standard_click());
///
/// let enter = ClickEvent::Keyboard(KeyboardClickEvent {
///     button: KeyboardButton::Enter,
///     ..Default::default()
/// });
/// assert!(enter.is_keyboard());
/// assert_eq!(enter.click_count(), 1);
/// assert_eq!(enter.modifiers(), Modifiers::default());
/// ```
#[derive(Clone, Debug)]
pub enum ClickEvent {
    /// A click from a mouse button, a press and a release on the same element.
    Mouse(MouseClickEvent),
    /// A click from Enter or Space on a focused element.
    Keyboard(KeyboardClickEvent),
    /// A touch tap or long press. GPUI reserves this variant for direct touch
    /// click delivery; the recognizer currently reports those gestures through
    /// mouse events instead.
    Touch(TouchClickEvent),
}

impl Default for ClickEvent {
    fn default() -> Self {
        ClickEvent::Keyboard(KeyboardClickEvent::default())
    }
}

impl ClickEvent {
    /// Returns the modifier keys held during the click.
    ///
    /// Keyboard and touch clicks have none, so this returns the default
    /// [`Modifiers`]. A mouse click returns the modifiers held when the button was
    /// released.
    pub fn modifiers(&self) -> Modifiers {
        match self {
            // Click events are only generated from keyboard events _without any modifiers_, so we know the modifiers are always Default
            ClickEvent::Keyboard(_) => Modifiers::default(),
            // Click events on the web only reflect the modifiers for the keyup event,
            // tested via observing the behavior of the `ClickEvent.shiftKey` field in Chrome 138
            // under various combinations of modifiers and keyUp / keyDown events.
            ClickEvent::Mouse(event) => event.up.modifiers,
            // Touch screens have no modifier keys.
            ClickEvent::Touch(_) => Modifiers::default(),
        }
    }

    /// Returns the window position of the click, in logical pixels.
    ///
    /// For a mouse click this is where the button was released. For a touch it
    /// is where the tap happened. For a keyboard click, which has no pointer,
    /// it is the bottom-left corner of the clicked element.
    pub fn position(&self) -> Point<Pixels> {
        match self {
            ClickEvent::Keyboard(event) => event.bounds.bottom_left(),
            ClickEvent::Mouse(event) => event.up.position,
            ClickEvent::Touch(event) => event.position,
        }
    }

    /// Returns the mouse position of the click, or `None` if it did not come from
    /// a mouse.
    ///
    /// Keyboard and touch clicks return `None`. Use [`ClickEvent::position`] when
    /// you need a position for every kind.
    pub fn mouse_position(&self) -> Option<Point<Pixels>> {
        match self {
            ClickEvent::Keyboard(_) => None,
            ClickEvent::Mouse(event) => Some(event.up.position),
            ClickEvent::Touch(_) => None,
        }
    }

    /// Returns `true` if this is a mouse click made with the right button.
    ///
    /// Keyboard and touch clicks always return `false`. Use
    /// [`ClickEvent::is_secondary`] to also match long presses.
    pub fn is_right_click(&self) -> bool {
        match self {
            ClickEvent::Keyboard(_) => false,
            ClickEvent::Mouse(event) => {
                event.down.button == MouseButton::Right && event.up.button == MouseButton::Right
            }
            ClickEvent::Touch(_) => false,
        }
    }

    /// Returns `true` if this is a mouse click made with the middle button.
    ///
    /// Keyboard and touch clicks always return `false`.
    pub fn is_middle_click(&self) -> bool {
        match self {
            ClickEvent::Keyboard(_) => false,
            ClickEvent::Mouse(event) => {
                event.down.button == MouseButton::Middle && event.up.button == MouseButton::Middle
            }
            ClickEvent::Touch(_) => false,
        }
    }

    /// Returns `true` if this click is a secondary activation, the kind that
    /// usually opens a context menu.
    ///
    /// That is a right-button mouse click, or a long press when a future direct
    /// touch click is delivered. On macOS, Control-clicks already arrive as right
    /// clicks. Keyboard clicks always return `false`.
    pub fn is_secondary(&self) -> bool {
        match self {
            ClickEvent::Keyboard(_) => false,
            ClickEvent::Mouse(event) => {
                event.down.button == MouseButton::Right && event.up.button == MouseButton::Right
            }
            ClickEvent::Touch(event) => event.long_press,
        }
    }

    /// Returns `true` for a primary activation: a left-button click, a keyboard
    /// click, or a direct touch tap that is not a long press.
    ///
    /// Right, middle and navigation button clicks return `false`. Use
    /// [`ClickEvent::is_secondary`] to test for the right button and long presses.
    pub fn standard_click(&self) -> bool {
        match self {
            ClickEvent::Keyboard(_) => true,
            ClickEvent::Mouse(event) => {
                event.down.button == MouseButton::Left && event.up.button == MouseButton::Left
            }
            ClickEvent::Touch(event) => !event.long_press,
        }
    }

    /// Returns `true` if this click activated the window, as reported by the
    /// platform in [`MouseDownEvent::first_mouse`].
    ///
    /// Keyboard and touch clicks always return `false`. A keyboard click needs
    /// the element to be focused already.
    pub fn first_focus(&self) -> bool {
        match self {
            ClickEvent::Keyboard(_) => false,
            ClickEvent::Mouse(event) => event.down.first_mouse,
            ClickEvent::Touch(_) => false,
        }
    }

    /// Returns how many clicks in a row this click completes. A double click is 2.
    ///
    /// A keyboard click always returns 1. A mouse click returns the count from its
    /// release. A touch click returns its tap count.
    pub fn click_count(&self) -> usize {
        match self {
            ClickEvent::Keyboard(_) => 1,
            ClickEvent::Mouse(event) => event.up.click_count,
            ClickEvent::Touch(event) => event.tap_count,
        }
    }

    /// Returns `true` if this click came from the keyboard.
    pub fn is_keyboard(&self) -> bool {
        match self {
            ClickEvent::Mouse(_) | ClickEvent::Touch(_) => false,
            ClickEvent::Keyboard(_) => true,
        }
    }
}

/// The key that triggered a keyboard click: Enter or Space.
#[derive(Hash, PartialEq, Eq, Copy, Clone, Debug, Default)]
pub enum KeyboardButton {
    /// The Enter key.
    #[default]
    Enter,
    /// The Space bar.
    Space,
}

/// A mouse button.
///
/// Compare buttons with `==` to choose a handler, as in
/// `event.button == MouseButton::Right`. The default is [`MouseButton::Left`].
#[derive(Hash, Default, PartialEq, Eq, Copy, Clone, Debug)]
pub enum MouseButton {
    /// The primary button, usually on the left.
    #[default]
    Left,

    /// The secondary button, usually on the right.
    Right,

    /// The middle button, usually the scroll wheel.
    Middle,

    /// A navigation button, such as back or forward on a side button.
    Navigate(NavigationDirection),
}

impl MouseButton {
    /// Returns every mouse button GPUI knows, in a fixed order.
    ///
    /// The order is `Left`, `Right`, `Middle`, `Navigate(Back)` and
    /// `Navigate(Forward)`. Use it to iterate over all buttons, for example to
    /// build a table of handlers.
    ///
    /// # Examples
    ///
    /// ```
    /// use gpui::{MouseButton, NavigationDirection};
    ///
    /// assert_eq!(MouseButton::all().len(), 5);
    /// assert_eq!(MouseButton::all()[3], MouseButton::Navigate(NavigationDirection::Back));
    /// ```
    pub fn all() -> Vec<Self> {
        vec![
            MouseButton::Left,
            MouseButton::Right,
            MouseButton::Middle,
            MouseButton::Navigate(NavigationDirection::Back),
            MouseButton::Navigate(NavigationDirection::Forward),
        ]
    }
}

/// The direction of a navigation button, the side buttons on some mice.
#[derive(Hash, Default, PartialEq, Eq, Copy, Clone, Debug)]
pub enum NavigationDirection {
    /// The back button.
    #[default]
    Back,

    /// The forward button.
    Forward,
}

/// The pointer moved over the window, with or without a button held.
///
/// Use [`MouseMoveEvent::dragging`] to tell a left-button drag from a hover.
#[derive(Clone, Debug, Default)]
pub struct MouseMoveEvent {
    /// The pointer position in window coordinates, in logical pixels.
    pub position: Point<Pixels>,

    /// The button held down during the move, or `None` if no button is held.
    pub pressed_button: Option<MouseButton>,

    /// The modifier keys held during the move.
    pub modifiers: Modifiers,
}

impl Sealed for MouseMoveEvent {}
impl InputEvent for MouseMoveEvent {
    fn to_platform_input(self) -> PlatformInput {
        PlatformInput::MouseMove(self)
    }
}
impl MouseEvent for MouseMoveEvent {}

impl MouseMoveEvent {
    /// Returns `true` if the left button is held down during this move.
    ///
    /// Other buttons, such as the right button, return `false`.
    pub fn dragging(&self) -> bool {
        self.pressed_button == Some(MouseButton::Left)
    }
}

/// A scroll from a mouse wheel, a trackpad, or a touch pan.
///
/// Sent to [`InteractiveElement::on_scroll_wheel`](crate::InteractiveElement::on_scroll_wheel) listeners at the pointer
/// position. Scroll containers use the `delta` to move their content.
///
/// Dereferences to [`Modifiers`], so `event.shift` reads the Shift state.
#[derive(Clone, Debug, Default)]
pub struct ScrollWheelEvent {
    /// The pointer position in window coordinates, in logical pixels. For a
    /// touch pan, the position where the pan started.
    pub position: Point<Pixels>,

    /// How far the content should scroll for this event.
    pub delta: ScrollDelta,

    /// The modifier keys held during the scroll.
    pub modifiers: Modifiers,

    /// Where this event falls in its scroll gesture. Trackpad and touch scrolls
    /// report `Started`, `Moved` and `Ended`. Wheels that don't report phases
    /// leave it at `Moved`.
    pub touch_phase: TouchPhase,
}

impl Sealed for ScrollWheelEvent {}
impl InputEvent for ScrollWheelEvent {
    fn to_platform_input(self) -> PlatformInput {
        PlatformInput::ScrollWheel(self)
    }
}
impl MouseEvent for ScrollWheelEvent {}

impl Deref for ScrollWheelEvent {
    type Target = Modifiers;

    fn deref(&self) -> &Self::Target {
        &self.modifiers
    }
}

/// The amount a scroll event moves the content.
///
/// Trackpads and touch pans report [`ScrollDelta::Pixels`]. Line-based mouse
/// wheels report [`ScrollDelta::Lines`]. Use [`ScrollDelta::pixel_delta`] to get
/// pixels for either.
#[derive(Clone, Copy, Debug)]
pub enum ScrollDelta {
    /// An exact amount in logical pixels.
    Pixels(Point<Pixels>),
    /// An approximate amount in lines of text. Multiply by the line height to get
    /// pixels.
    Lines(Point<f32>),
}

impl Default for ScrollDelta {
    fn default() -> Self {
        Self::Lines(Default::default())
    }
}

/// A pinch-to-zoom gesture, typically from a trackpad.
///
/// Sent to [`InteractiveElement::on_pinch`](crate::InteractiveElement::on_pinch) listeners. Pinch recognition from
/// touch input is not implemented yet, so pinch events come from the platform.
///
/// Dereferences to [`Modifiers`].
#[derive(Clone, Debug, Default)]
pub struct PinchEvent {
    /// The center of the pinch in window coordinates, in logical pixels.
    pub position: Point<Pixels>,

    /// The change in scale since the previous event. `0.1` is a 10% zoom in and
    /// `-0.1` is a 10% zoom out.
    pub delta: f32,

    /// The modifier keys held during the pinch.
    pub modifiers: Modifiers,

    /// Where this event falls in the pinch gesture.
    pub phase: TouchPhase,
}

impl Sealed for PinchEvent {}
impl InputEvent for PinchEvent {
    fn to_platform_input(self) -> PlatformInput {
        PlatformInput::Pinch(self)
    }
}
impl GestureEvent for PinchEvent {}
impl MouseEvent for PinchEvent {}

impl Deref for PinchEvent {
    type Target = Modifiers;

    fn deref(&self) -> &Self::Target {
        &self.modifiers
    }
}

impl ScrollDelta {
    /// Returns `true` if this delta is exact, in pixels, rather than in lines.
    pub fn precise(&self) -> bool {
        match self {
            ScrollDelta::Pixels(_) => true,
            ScrollDelta::Lines(_) => false,
        }
    }

    /// Returns this delta in logical pixels.
    ///
    /// A pixel delta is returned unchanged. A line delta is multiplied by
    /// `line_height` on each axis, so pass the line height of the text you are
    /// scrolling.
    ///
    /// # Examples
    ///
    /// ```
    /// use gpui::{ScrollDelta, point, px};
    ///
    /// let lines = ScrollDelta::Lines(point(0., 2.));
    /// assert_eq!(lines.pixel_delta(px(20.)), point(px(0.), px(40.)));
    /// ```
    pub fn pixel_delta(&self, line_height: Pixels) -> Point<Pixels> {
        match self {
            ScrollDelta::Pixels(delta) => *delta,
            ScrollDelta::Lines(delta) => point(line_height * delta.x, line_height * delta.y),
        }
    }

    /// Merges this delta with a later one, as when two scroll events arrive in
    /// the same frame.
    ///
    /// Each axis is handled separately. If both deltas move that axis the same
    /// way, the amounts add. If they move it in opposite directions, `other`
    /// wins on that axis. Two deltas of different kinds, one in pixels and one
    /// in lines, return `other` unchanged.
    ///
    /// A component of zero counts as positive, so a zero in `other` replaces a
    /// negative component of `self`, and a zero in `self` is replaced by a
    /// negative component of `other`.
    ///
    /// # Examples
    ///
    /// ```
    /// use gpui::{ScrollDelta, point, px};
    ///
    /// // Same direction on x adds (10 + 5); opposite direction on y takes the later 3.
    /// let merged = ScrollDelta::Pixels(point(px(10.), px(-5.)))
    ///     .coalesce(ScrollDelta::Pixels(point(px(5.), px(3.))));
    /// assert_eq!(merged.pixel_delta(px(16.)), point(px(15.), px(3.)));
    /// ```
    pub fn coalesce(self, other: ScrollDelta) -> ScrollDelta {
        match (self, other) {
            (ScrollDelta::Pixels(a), ScrollDelta::Pixels(b)) => {
                let x = if a.x.signum() == b.x.signum() {
                    a.x + b.x
                } else {
                    b.x
                };

                let y = if a.y.signum() == b.y.signum() {
                    a.y + b.y
                } else {
                    b.y
                };

                ScrollDelta::Pixels(point(x, y))
            }

            (ScrollDelta::Lines(a), ScrollDelta::Lines(b)) => {
                let x = if a.x.signum() == b.x.signum() {
                    a.x + b.x
                } else {
                    b.x
                };

                let y = if a.y.signum() == b.y.signum() {
                    a.y + b.y
                } else {
                    b.y
                };

                ScrollDelta::Lines(point(x, y))
            }

            _ => other,
        }
    }
}

/// The pointer left the window.
///
/// Use it to end hover state. Dereferences to [`Modifiers`].
#[derive(Clone, Debug, Default)]
pub struct MouseExitEvent {
    /// The last pointer position in window coordinates, in logical pixels.
    pub position: Point<Pixels>,
    /// The button held when the pointer left, or `None` if none was held.
    pub pressed_button: Option<MouseButton>,
    /// The modifier keys held when the pointer left.
    pub modifiers: Modifiers,
}

impl Sealed for MouseExitEvent {}
impl InputEvent for MouseExitEvent {
    fn to_platform_input(self) -> PlatformInput {
        PlatformInput::MouseExited(self)
    }
}

impl MouseEvent for MouseExitEvent {}

impl Deref for MouseExitEvent {
    type Target = Modifiers;

    fn deref(&self) -> &Self::Target {
        &self.modifiers
    }
}

/// The file paths carried by a file drop.
///
/// Wraps the paths the platform reports when files are dragged onto the window.
#[derive(Debug, Clone, Default, Eq, PartialEq)]
pub struct ExternalPaths(pub SmallVec<[PathBuf; 2]>);

impl ExternalPaths {
    /// Returns the dropped paths in the order the platform reported them.
    pub fn paths(&self) -> &[PathBuf] {
        &self.0
    }
}

/// The content of a drag that leaves the window and becomes a native drag
/// session handed to the operating system.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum ExternalDragPayload {
    /// Files on disk, offered to the platform as an outbound file drag.
    Files(FileDragPaths),
}

/// The files for a native file drag, each paired with whether it is a directory.
///
/// The caller supplies the directory flags up front, so the platform does not
/// have to query the file system when the drag starts.
#[derive(Debug, Clone, Default, Eq, PartialEq)]
pub struct FileDragPaths(SmallVec<[(PathBuf, bool); 2]>);

impl FileDragPaths {
    /// Creates a drag payload from paths, each paired with whether it is a directory.
    pub fn new(entries: impl IntoIterator<Item = (PathBuf, bool)>) -> Self {
        Self(entries.into_iter().collect())
    }

    /// Returns the dragged paths, each paired with whether it is a directory.
    pub fn entries(&self) -> &[(PathBuf, bool)] {
        &self.0
    }
}

impl Render for ExternalPaths {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        // the platform will render icons for the dragged files
        Empty
    }
}

/// A stage of a file drag from the operating system into the window.
///
/// `Entered` starts a drag, `Pending` reports movement during it, `Submit` drops
/// the files, and `Exited` ends a drag that did not drop. `Ended` reports that the
/// platform's drag session has finished.
#[derive(Debug, Clone)]
pub enum FileDropEvent {
    /// The drag entered the window.
    Entered {
        /// The pointer position in window coordinates, in logical pixels.
        position: Point<Pixels>,
        /// The paths of the files being dragged.
        paths: ExternalPaths,
    },
    /// The drag moved over the window.
    Pending {
        /// The pointer position in window coordinates, in logical pixels.
        position: Point<Pixels>,
    },
    /// The user dropped the files on the window.
    Submit {
        /// The pointer position in window coordinates, in logical pixels.
        position: Point<Pixels>,
    },
    /// The drag left the window without a drop.
    Exited,
    /// The platform's drag session ended.
    Ended,
}

impl Sealed for FileDropEvent {}
impl InputEvent for FileDropEvent {
    fn to_platform_input(self) -> PlatformInput {
        PlatformInput::FileDrop(self)
    }
}
impl MouseEvent for FileDropEvent {}

/// Any input event the platform sends to a window.
///
/// Platform backends build this enum and hand it to the window, which routes
/// each variant to the matching listeners. You rarely construct it yourself.
/// Match on it to see the raw event, or call [`PlatformInput::kind_name`] for a
/// label.
#[derive(Clone, Debug)]
pub enum PlatformInput {
    /// A key was pressed. See [`KeyDownEvent`].
    KeyDown(KeyDownEvent),
    /// A key was released. See [`KeyUpEvent`].
    KeyUp(KeyUpEvent),
    /// The modifier keys or Caps Lock changed. See [`ModifiersChangedEvent`].
    ModifiersChanged(ModifiersChangedEvent),
    /// A mouse button was pressed. See [`MouseDownEvent`].
    MouseDown(MouseDownEvent),
    /// A mouse button was released. See [`MouseUpEvent`].
    MouseUp(MouseUpEvent),
    /// A force-sensitive trackpad changed pressure. See [`MousePressureEvent`].
    MousePressure(MousePressureEvent),
    /// The pointer moved. See [`MouseMoveEvent`].
    MouseMove(MouseMoveEvent),
    /// The pointer left the window. See [`MouseExitEvent`].
    MouseExited(MouseExitEvent),
    /// A scroll from a wheel, trackpad or touch pan. See [`ScrollWheelEvent`].
    ScrollWheel(ScrollWheelEvent),
    /// A pinch-to-zoom gesture. See [`PinchEvent`].
    Pinch(PinchEvent),
    /// A long press recognized from touch input. See [`LongPressEvent`].
    LongPress(LongPressEvent),
    /// A touch drag claimed by an element. See [`TouchDragEvent`].
    TouchDrag(TouchDragEvent),
    /// Files were dragged onto, or dropped on, the window. See [`FileDropEvent`].
    FileDrop(FileDropEvent),
    /// A raw sample from a touch screen, before gesture recognition. See [`TouchEvent`].
    Touch(TouchEvent),
}

impl PlatformInput {
    pub(crate) fn mouse_event(&self) -> Option<&dyn Any> {
        match self {
            PlatformInput::KeyDown { .. } => None,
            PlatformInput::KeyUp { .. } => None,
            PlatformInput::ModifiersChanged { .. } => None,
            PlatformInput::MouseDown(event) => Some(event),
            PlatformInput::MouseUp(event) => Some(event),
            PlatformInput::MouseMove(event) => Some(event),
            PlatformInput::MousePressure(event) => Some(event),
            PlatformInput::MouseExited(event) => Some(event),
            PlatformInput::ScrollWheel(event) => Some(event),
            PlatformInput::Pinch(event) => Some(event),
            PlatformInput::LongPress(event) => Some(event),
            PlatformInput::TouchDrag(event) => Some(event),
            PlatformInput::FileDrop(event) => Some(event),
            PlatformInput::Touch(_) => None,
        }
    }

    pub(crate) fn keyboard_event(&self) -> Option<&dyn Any> {
        match self {
            PlatformInput::KeyDown(event) => Some(event),
            PlatformInput::KeyUp(event) => Some(event),
            PlatformInput::ModifiersChanged(event) => Some(event),
            PlatformInput::MouseDown(_) => None,
            PlatformInput::MouseUp(_) => None,
            PlatformInput::MouseMove(_) => None,
            PlatformInput::MousePressure(_) => None,
            PlatformInput::MouseExited(_) => None,
            PlatformInput::ScrollWheel(_) => None,
            PlatformInput::Pinch(_) => None,
            PlatformInput::LongPress(_) => None,
            PlatformInput::TouchDrag(_) => None,
            PlatformInput::FileDrop(_) => None,
            PlatformInput::Touch(_) => None,
        }
    }

    /// Returns a short, fixed name for this variant, such as `"mouse_down"`.
    ///
    /// Use it in logs and telemetry. The names are stable snake_case strings.
    pub fn kind_name(&self) -> &'static str {
        match self {
            PlatformInput::KeyDown(_) => "key_down",
            PlatformInput::KeyUp(_) => "key_up",
            PlatformInput::ModifiersChanged(_) => "modifiers_changed",
            PlatformInput::MouseDown(_) => "mouse_down",
            PlatformInput::MouseUp(_) => "mouse_up",
            PlatformInput::MousePressure(_) => "mouse_pressure",
            PlatformInput::MouseMove(_) => "mouse_move",
            PlatformInput::MouseExited(_) => "mouse_exited",
            PlatformInput::ScrollWheel(_) => "scroll_wheel",
            PlatformInput::Pinch(_) => "pinch",
            PlatformInput::LongPress(_) => "long_press",
            PlatformInput::TouchDrag(_) => "touch_drag",
            PlatformInput::FileDrop(_) => "file_drop",
            PlatformInput::Touch(_) => "touch",
        }
    }

    /// Returns the raw [`TouchEvent`] if this input is [`PlatformInput::Touch`],
    /// or `None` for every other variant.
    pub fn touch_event(&self) -> Option<&TouchEvent> {
        match self {
            PlatformInput::Touch(event) => Some(event),
            _ => None,
        }
    }
}

#[cfg(test)]
mod test {

    use crate::{
        self as gpui, AppContext as _, Context, FocusHandle, InteractiveElement, IntoElement,
        KeyBinding, Keystroke, Modifiers, ParentElement, Render, TestAppContext, Window, div,
    };

    struct TestView {
        saw_key_down: bool,
        saw_action: bool,
        focus_handle: FocusHandle,
    }

    actions!(test_only, [TestAction]);

    impl Render for TestView {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            div().id("testview").child(
                div()
                    .key_context("parent")
                    .on_key_down(cx.listener(|this, _, _, cx| {
                        cx.stop_propagation();
                        this.saw_key_down = true
                    }))
                    .on_action(cx.listener(|this: &mut TestView, _: &TestAction, _, _| {
                        this.saw_action = true
                    }))
                    .child(
                        div()
                            .key_context("nested")
                            .track_focus(&self.focus_handle)
                            .into_element(),
                    ),
            )
        }
    }

    #[gpui::test]
    fn test_on_events(cx: &mut TestAppContext) {
        let window = cx.update(|cx| {
            cx.open_window(Default::default(), |_, cx| {
                cx.new(|cx| TestView {
                    saw_key_down: false,
                    saw_action: false,
                    focus_handle: cx.focus_handle(),
                })
            })
            .unwrap()
        });

        cx.update(|cx| {
            cx.bind_keys(vec![KeyBinding::new("ctrl-g", TestAction, Some("parent"))]);
        });

        window
            .update(cx, |test_view, window, cx| {
                window.focus(&test_view.focus_handle, cx)
            })
            .unwrap();

        cx.dispatch_keystroke(*window, Keystroke::parse("a").unwrap());
        cx.dispatch_keystroke(*window, Keystroke::parse("ctrl-g").unwrap());

        window
            .update(cx, |test_view, _, _| {
                assert!(test_view.saw_key_down || test_view.saw_action);
                assert!(test_view.saw_key_down);
                assert!(test_view.saw_action);
            })
            .unwrap();
    }

    #[gpui::test]
    fn test_multi_modifier_gesture_does_not_dispatch_standalone_modifier_binding(
        cx: &mut TestAppContext,
    ) {
        let (test_view, cx) = cx.add_window_view(|_, cx| TestView {
            saw_key_down: false,
            saw_action: false,
            focus_handle: cx.focus_handle(),
        });

        cx.update(|_, cx| {
            cx.bind_keys(vec![KeyBinding::new("shift", TestAction, None)]);
        });
        test_view.update_in(cx, |test_view, window, cx| {
            window.focus(&test_view.focus_handle, cx);
        });

        cx.simulate_modifiers_change(Modifiers::alt());
        cx.simulate_modifiers_change(Modifiers::alt() | Modifiers::shift());
        cx.simulate_modifiers_change(Modifiers::shift());
        cx.simulate_modifiers_change(Modifiers::none());
        assert!(!test_view.read_with(cx, |test_view, _| test_view.saw_action));

        cx.simulate_modifiers_change(Modifiers::shift());
        cx.simulate_modifiers_change(Modifiers::none());
        assert!(test_view.read_with(cx, |test_view, _| test_view.saw_action));
    }
}
