use scheduler::Instant;
use std::{cell::Cell, rc::Rc, time::Duration};

use crate::{
    AnyElement, App, AppContext, Element, ElementId, GlobalElementId, InspectorElementId,
    IntoElement, Motion, ParentElement, SpringAnimation, SpringConfig, SpringDescription,
    SpringPlayback, SpringState, SpringTarget, Window,
};

pub use easing::*;
use smallvec::SmallVec;

/// A timed change to an element, applied with [`AnimationExt::with_animation`].
///
/// An animation is a [`Motion`] with two more options: whether its phase follows the app's shared
/// clock, and an optional cap on how often it redraws. A [`Duration`](std::time::Duration) turns
/// into one linear pass.
///
/// # Examples
///
/// ```
/// use std::time::Duration;
///
/// use gpui::{Animation, Direction, Iterations, Motion, ease_out_quint, millis};
///
/// let once = Animation::new(Duration::from_millis(300)).with_easing(ease_out_quint());
/// assert!(!once.synced);
/// assert_eq!(once.motion.playback().iterations(), Iterations::default());
///
/// // Pass an alternating motion, then repeat it, to ping-pong forever.
/// let ping_pong = Animation::new(Motion::new(millis(300)).alternate()).repeat();
/// assert_eq!(ping_pong.motion.playback().iterations(), Iterations::Forever);
/// assert_eq!(ping_pong.motion.playback().direction(), Direction::Alternate);
///
/// assert_eq!(Animation::new(millis(16)).with_max_fps(30.).max_fps, Some(30.0));
/// ```
#[derive(Clone)]
pub struct Animation {
    /// The timing, repetition and easing of this animation.
    pub motion: Motion,
    /// Whether the phase follows the app's shared clock rather than the time this element began
    /// animating. See [`Animation::repeat_synced`].
    pub synced: bool,
    /// The maximum number of re-renders per second. `None` re-renders on every frame.
    /// See [`Animation::with_max_fps`].
    pub max_fps: Option<f32>,
}

impl Animation {
    /// Creates an animation from a [`Motion`] or a [`Duration`](std::time::Duration).
    ///
    /// A duration creates one linear pass that runs once.
    pub fn new(motion: impl Into<Motion>) -> Self {
        Self {
            motion: motion.into(),
            synced: false,
            max_fps: None,
        }
    }

    /// Repeats the animation forever.
    ///
    /// Each cycle starts again from the beginning. The direction doesn't reverse. To ping-pong,
    /// build the motion with [`Motion::alternate`] and pass it to [`Animation::new`].
    pub fn repeat(mut self) -> Self {
        self.motion = self.motion.repeat_forever();
        self
    }

    /// Repeats the animation forever, with its phase set by a clock shared by the whole [`App`](crate::App).
    ///
    /// Elements that use it stay in step with each other. An element that mounts later joins the
    /// shared phase, rather than starting from zero.
    pub fn repeat_synced(mut self) -> Self {
        self.motion = self.motion.repeat_forever();
        self.synced = true;
        self
    }

    /// Sets the easing curve.
    ///
    /// The output isn't clamped. An overshooting curve gives values outside `0.0..=1.0`, so clamp
    /// in your animator if the property can't take them. A curve such as [`ease_out_quint`] stays
    /// within that range.
    pub fn with_easing(mut self, easing: impl Fn(f32) -> f32 + 'static) -> Self {
        self.motion = self.motion.with_easing(easing);
        self
    }

    /// Caps re-renders at `max_fps` per second, using a timer between frames.
    ///
    /// A value that is not finite or not positive is ignored, and the animation re-renders on every
    /// frame.
    pub fn with_max_fps(mut self, max_fps: f32) -> Self {
        self.max_fps = Some(max_fps);
        self
    }
}

/// Adds animation methods to every element and component.
///
/// It is implemented for any [`IntoElement`] type, so you can call these methods on a
/// [`Div`](crate::Div) or on your own component.
///
/// Animations honor [`App::reduce_motion`](crate::App::reduce_motion). When it is set, the element
/// renders in a static state: the end state for a one-pass animation, and the start state for a
/// repeating one. No animation frames are scheduled. GPUI doesn't read the operating system's
/// setting, so set it yourself with [`App::set_reduce_motion`](crate::App::set_reduce_motion).
pub trait AnimationExt {
    /// Renders this element with `animation`, passing `animator` the eased progress on every frame.
    ///
    /// `animator` receives the element and a value from `0.0` to `1.0`, and returns the styled
    /// element. The `id` identifies the animation across frames. Change it to start the animation
    /// again, and keep it stable otherwise, or the animation restarts on every render.
    ///
    /// # Examples
    ///
    /// Under reduce motion, a one-pass animation renders at its end state, `1.0`.
    ///
    /// ```
    /// use std::{cell::RefCell, rc::Rc, time::Duration};
    ///
    /// use gpui::{
    ///     Animation, AnimationExt, Context, IntoElement, Render, TestApp, Window, div, prelude::*, px,
    /// };
    ///
    /// struct Toast {
    ///     deltas: Rc<RefCell<Vec<f32>>>,
    /// }
    ///
    /// impl Render for Toast {
    ///     fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
    ///         let deltas = self.deltas.clone();
    ///         div().child("Saved").with_animation(
    ///             "toast-enter",
    ///             Animation::new(Duration::from_millis(300)),
    ///             move |toast, delta| {
    ///                 deltas.borrow_mut().push(delta);
    ///                 toast.opacity(delta).mt(px(8. * (1. - delta)))
    ///             },
    ///         )
    ///     }
    /// }
    ///
    /// let deltas = Rc::new(RefCell::new(Vec::new()));
    /// let mut app = TestApp::new();
    /// app.update(|cx| cx.set_reduce_motion(true));
    /// let mut window = app.open_window(|_, _| Toast { deltas: deltas.clone() });
    /// window.draw();
    /// assert_eq!(deltas.borrow().last(), Some(&1.0));
    /// ```
    fn with_animation(
        self,
        id: impl Into<ElementId>,
        animation: Animation,
        animator: impl Fn(Self, f32) -> Self + 'static,
    ) -> AnimationElement<Self>
    where
        Self: Sized,
    {
        AnimationElement {
            id: id.into(),
            element: Some(self),
            animator: Box::new(move |this, _, value| animator(this, value)),
            animations: smallvec::smallvec![animation],
        }
    }

    /// Renders this element with a sequence of animations, one after another.
    ///
    /// `animator` receives the index of the running step and its progress from `0.0` to `1.0`.
    /// Each step starts when the one before it finishes. A repeating step never finishes, so only
    /// the last step should repeat.
    ///
    /// Under reduce motion, the element renders at the end of the last step.
    ///
    /// # Panics
    ///
    /// Panics if `animations` is empty. Debug builds assert this when the element is created.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::{cell::RefCell, rc::Rc, time::Duration};
    ///
    /// use gpui::{
    ///     Animation, AnimationExt, Context, IntoElement, Render, TestApp, Window, div, ease_out_quint,
    ///     prelude::*, px,
    /// };
    ///
    /// struct Pop {
    ///     steps: Rc<RefCell<Vec<(usize, f32)>>>,
    /// }
    ///
    /// impl Render for Pop {
    ///     fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
    ///         let steps = self.steps.clone();
    ///         div().child("Done").with_animations(
    ///             "pop",
    ///             vec![
    ///                 Animation::new(Duration::from_millis(150)),
    ///                 Animation::new(Duration::from_millis(300)).with_easing(ease_out_quint()),
    ///             ],
    ///             move |el, step, delta| {
    ///                 steps.borrow_mut().push((step, delta));
    ///                 match step {
    ///                     0 => el.opacity(delta),
    ///                     _ => el.opacity(1.0).mt(px(8. * (1. - delta))),
    ///                 }
    ///             },
    ///         )
    ///     }
    /// }
    ///
    /// let steps = Rc::new(RefCell::new(Vec::new()));
    /// let mut app = TestApp::new();
    /// app.update(|cx| cx.set_reduce_motion(true));
    /// let mut window = app.open_window(|_, _| Pop { steps: steps.clone() });
    /// window.draw();
    /// assert_eq!(steps.borrow().last(), Some(&(1, 1.0)));
    /// ```
    fn with_animations(
        self,
        id: impl Into<ElementId>,
        animations: Vec<Animation>,
        animator: impl Fn(Self, usize, f32) -> Self + 'static,
    ) -> AnimationElement<Self>
    where
        Self: Sized,
    {
        debug_assert!(!animations.is_empty(), "animations must not be empty");

        AnimationElement {
            id: id.into(),
            element: Some(self),
            animator: Box::new(animator),
            animations: animations.into(),
        }
    }

    /// Renders this element at the value of a spring, passed to `animator`.
    ///
    /// The `id` keeps the spring's position and velocity across frames, so a retargeted spring
    /// bends toward the new target. A newly mounted spring starts at its target, unless you set
    /// a start with [`SpringAnimation::from`]. Under reduce motion, the spring jumps to its target.
    /// See [`SpringAnimation`] for an example.
    fn with_spring<T>(
        self,
        id: impl Into<ElementId>,
        animation: SpringAnimation<T>,
        animator: impl FnOnce(Self, T::Output) -> Self + 'static,
    ) -> SpringAnimationElement<Self>
    where
        Self: Sized,
        T: SpringTarget,
        T::Output: 'static,
    {
        let SpringAnimation {
            motion,
            target,
            initial,
            playback,
        } = animation;
        let scalar_target = target.target();
        SpringAnimationElement {
            id: id.into(),
            element: Some(self),
            motion,
            target: scalar_target,
            initial,
            playback,
            animator: Some(Box::new(move |this, value| {
                animator(this, target.resolve(value))
            })),
        }
    }
}

impl<E: IntoElement + 'static> AnimationExt for E {}

/// The element returned by [`AnimationExt::with_animation`] and [`AnimationExt::with_animations`].
///
/// It wraps the element being animated and runs the animator on each frame.
pub struct AnimationElement<E> {
    id: ElementId,
    element: Option<E>,
    animations: SmallVec<[Animation; 1]>,
    animator: Box<dyn Fn(E, usize, f32) -> E + 'static>,
}

/// The element returned by [`AnimationExt::with_spring`].
///
/// It wraps the element being animated and renders it at the spring's current value.
pub struct SpringAnimationElement<E> {
    id: ElementId,
    element: Option<E>,
    motion: Motion<SpringDescription>,
    target: f32,
    initial: Option<f32>,
    playback: SpringPlayback,
    animator: Option<Box<dyn FnOnce(E, f32) -> E + 'static>>,
}

impl<E: ParentElement> ParentElement for SpringAnimationElement<E> {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        let Some(element) = &mut self.element else {
            return;
        };

        element.extend(elements);
    }
}

impl<E> SpringAnimationElement<E> {
    /// Applies `f` to the element being animated, and returns the animation with the result.
    ///
    /// The spring's animator still runs on the element that `f` returns.
    pub fn map_element(mut self, f: impl FnOnce(E) -> E) -> SpringAnimationElement<E> {
        self.element = self.element.map(f);
        self
    }
}

impl<E: IntoElement + 'static> IntoElement for SpringAnimationElement<E> {
    type Element = SpringAnimationElement<E>;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl<E: ParentElement> ParentElement for AnimationElement<E> {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        let Some(element) = &mut self.element else {
            return;
        };

        element.extend(elements);
    }
}

impl<E> AnimationElement<E> {
    /// Applies `f` to the element being animated, and returns the animation with the result.
    ///
    /// The animator still runs on the element that `f` returns.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::time::Duration;
    ///
    /// use gpui::{Animation, AnimationExt, div, prelude::*, px};
    ///
    /// let _fade = div()
    ///     .child("Saved")
    ///     .with_animation(
    ///         "fade",
    ///         Animation::new(Duration::from_millis(200)),
    ///         |el, delta| el.opacity(delta),
    ///     )
    ///     .map_element(|el| el.w(px(120.)));
    /// ```
    pub fn map_element(mut self, f: impl FnOnce(E) -> E) -> AnimationElement<E> {
        self.element = self.element.map(f);
        self
    }
}

impl<E: IntoElement + 'static> IntoElement for AnimationElement<E> {
    type Element = AnimationElement<E>;

    fn into_element(self) -> Self::Element {
        self
    }
}

struct AnimationState {
    start: Instant,
    animation_ix: usize,
    /// Whether a throttled re-render (see [`Animation::with_max_fps`]) is
    /// already scheduled, so overlapping renders don't stack extra timers.
    delayed_frame_pending: Rc<Cell<bool>>,
}

struct SpringElementState {
    spring: SpringState,
    target: f32,
    config: SpringConfig,
    initial: f32,
    playback: SpringPlayback,
    updated_at: Instant,
}

impl<E: IntoElement + 'static> Element for SpringAnimationElement<E> {
    type RequestLayoutState = AnyElement;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (crate::LayoutId, Self::RequestLayoutState) {
        window.with_element_state(global_id.unwrap(), |state, window| {
            // Use the executor clock so spring progression is deterministic in
            // tests and remains consistent with scheduled animation work.
            let now = cx.background_executor().now();
            let initial = self.initial.unwrap_or(self.target);
            let mut state = state.unwrap_or_else(|| SpringElementState {
                spring: SpringState {
                    position: initial,
                    velocity: 0.0,
                },
                target: self.target,
                config: self.motion.config,
                initial,
                playback: self.playback,
                updated_at: now,
            });

            let elapsed = now.duration_since(state.updated_at).as_secs_f32();
            match state.playback {
                SpringPlayback::Running => {
                    state.spring = state.config.step(state.spring, state.target, elapsed);
                }
                SpringPlayback::Paused
                | SpringPlayback::Stopped
                | SpringPlayback::Completed
                | SpringPlayback::Cancelled => {}
            }

            state.config = self.motion.config;
            state.target = self.target;

            let done = match self.playback {
                SpringPlayback::Running => {
                    if cx.reduce_motion() {
                        state.spring = SpringState {
                            position: state.target,
                            velocity: 0.0,
                        };
                        true
                    } else {
                        let done = state.config.is_settled(
                            state.spring,
                            state.target,
                            self.motion.epsilon,
                        );
                        if done {
                            state.spring = SpringState {
                                position: state.target,
                                velocity: 0.0,
                            };
                        }
                        done
                    }
                }
                SpringPlayback::Paused => true,
                SpringPlayback::Stopped => {
                    state.spring.velocity = 0.0;
                    true
                }
                SpringPlayback::Completed => {
                    state.spring = SpringState {
                        position: state.target,
                        velocity: 0.0,
                    };
                    true
                }
                SpringPlayback::Cancelled => {
                    state.spring = SpringState {
                        position: state.initial,
                        velocity: 0.0,
                    };
                    true
                }
            };
            state.playback = self.playback;
            state.updated_at = now;

            let element = self.element.take().expect("should only be called once");
            let animator = self.animator.take().expect("should only be called once");
            let mut element = animator(element, state.spring.position).into_any_element();

            if !done {
                window.request_animation_frame();
            }

            ((element.request_layout(window, cx), element), state)
        })
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: crate::Bounds<crate::Pixels>,
        element: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        element.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: crate::Bounds<crate::Pixels>,
        element: &mut Self::RequestLayoutState,
        _: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        element.paint(window, cx);
    }
}

impl<E: IntoElement + 'static> Element for AnimationElement<E> {
    type RequestLayoutState = AnyElement;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (crate::LayoutId, Self::RequestLayoutState) {
        window.with_element_state(global_id.unwrap(), |state, window| {
            let now = cx.background_executor().now();
            let mut state = state.unwrap_or_else(|| AnimationState {
                start: now,
                animation_ix: 0,
                delayed_frame_pending: Rc::new(Cell::new(false)),
            });
            let (animation_ix, delta, done) = if cx.reduce_motion() {
                let animation_ix = self.animations.len() - 1;
                let delta = self.animations[animation_ix]
                    .motion
                    .resting_progress()
                    .get();
                (animation_ix, delta, true)
            } else {
                let animation_ix = state.animation_ix;
                let animation = &self.animations[animation_ix];
                let elapsed = if animation.synced {
                    now - cx.synced_animation_epoch
                } else {
                    now - state.start
                };
                let sample = animation.motion.sample(elapsed);
                let mut done = !sample.is_active;
                if done && animation_ix < self.animations.len() - 1 {
                    state.start = now;
                    state.animation_ix += 1;
                    done = false;
                }
                (animation_ix, sample.progress.get(), done)
            };

            debug_assert!(delta.is_finite(), "animated value should be finite");

            let element = self.element.take().expect("should only be called once");
            let mut element = (self.animator)(element, animation_ix, delta).into_any_element();

            if !done {
                match self.animations[animation_ix].max_fps {
                    Some(max_fps) if max_fps.is_finite() && max_fps > 0.0 => {
                        if !state.delayed_frame_pending.get() {
                            state.delayed_frame_pending.set(true);
                            let delayed_frame_pending = state.delayed_frame_pending.clone();
                            let view = window.current_view();
                            let interval = Duration::from_secs_f32(1.0 / max_fps);
                            window
                                .spawn(cx, async move |cx| {
                                    cx.background_executor().timer(interval).await;
                                    delayed_frame_pending.set(false);
                                    cx.update(move |_, cx| cx.notify(view)).ok();
                                })
                                .detach();
                        }
                    }
                    _ => window.request_animation_frame(),
                }
            }

            ((element.request_layout(window, cx), element), state)
        })
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: crate::Bounds<crate::Pixels>,
        element: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        element.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: crate::Bounds<crate::Pixels>,
        element: &mut Self::RequestLayoutState,
        _: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        element.paint(window, cx);
    }
}

mod easing {
    use std::f32::consts::PI;

    /// Returns `delta` unchanged. This is the default easing.
    ///
    /// # Examples
    ///
    /// ```
    /// use gpui::linear;
    ///
    /// assert_eq!(linear(0.25), 0.25);
    /// ```
    pub fn linear(delta: f32) -> f32 {
        delta
    }

    /// Accelerates from rest. Returns `delta * delta`.
    ///
    /// # Examples
    ///
    /// ```
    /// use gpui::quadratic;
    ///
    /// assert_eq!(quadratic(0.5), 0.25);
    /// ```
    pub fn quadratic(delta: f32) -> f32 {
        delta * delta
    }

    /// Starts and ends slowly, and moves fastest at the midpoint.
    ///
    /// # Examples
    ///
    /// ```
    /// use gpui::ease_in_out;
    ///
    /// assert_eq!(ease_in_out(0.25), 0.125);
    /// assert_eq!(ease_in_out(0.5), 0.5);
    /// assert_eq!(ease_in_out(0.75), 0.875);
    /// ```
    pub fn ease_in_out(delta: f32) -> f32 {
        if delta < 0.5 {
            2.0 * delta * delta
        } else {
            let x = -2.0 * delta + 2.0;
            1.0 - x * x / 2.0
        }
    }

    /// Starts quickly and decelerates to a stop.
    ///
    /// This returns the easing as a closure, so call it as `ease_out_quint()`.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::time::Duration;
    ///
    /// use gpui::{Animation, ease_out_quint};
    ///
    /// let easing = ease_out_quint();
    /// assert_eq!(easing(0.5), 0.96875);
    /// assert_eq!(easing(1.0), 1.0);
    ///
    /// let animation = Animation::new(Duration::from_millis(300)).with_easing(ease_out_quint());
    /// assert!(!animation.synced);
    /// ```
    pub fn ease_out_quint() -> impl Fn(f32) -> f32 {
        move |delta| 1.0 - (1.0 - delta).powi(5)
    }

    /// Runs `easing` forward over the first half, then backward over the second half.
    ///
    /// The motion returns to its start value. The result is a closure, so call it as `bounce(...)`.
    ///
    /// # Examples
    ///
    /// ```
    /// use gpui::{bounce, linear, quadratic};
    ///
    /// let easing = bounce(linear);
    /// assert_eq!(easing(0.0), 0.0);
    /// assert_eq!(easing(0.5), 1.0);
    /// assert_eq!(easing(1.0), 0.0);
    ///
    /// assert_eq!(bounce(quadratic)(0.25), 0.25);
    /// ```
    pub fn bounce(easing: impl Fn(f32) -> f32) -> impl Fn(f32) -> f32 {
        move |delta| {
            if delta < 0.5 {
                easing(delta * 2.0)
            } else {
                easing((1.0 - delta) * 2.0)
            }
        }
    }

    /// Breathes between `min` and `max`, for idle indicators.
    ///
    /// Over one cycle the value is at `max` near a quarter of the way through, at `min` near three
    /// quarters, and at the midpoint at the start, the middle and the end. Use it with
    /// [`Animation::repeat_synced`](crate::Animation::repeat_synced) so that several indicators pulse together. The result is a
    /// closure, so call it as `pulsating_between(min, max)`.
    ///
    /// # Examples
    ///
    /// ```
    /// use gpui::pulsating_between;
    ///
    /// let breathe = pulsating_between(0.3, 1.0);
    /// assert!((breathe(0.25) - 1.0).abs() < 1e-6);
    /// assert!((breathe(0.75) - 0.3).abs() < 1e-6);
    /// assert!((breathe(0.0) - 0.65).abs() < 1e-6);
    /// ```
    pub fn pulsating_between(min: f32, max: f32) -> impl Fn(f32) -> f32 {
        let range = max - min;

        move |delta| {
            // Use a combination of sine and cubic functions for a more natural breathing rhythm
            let t = (delta * 2.0 * PI).sin();
            let breath = (t * t * t + t) / 2.0;

            // Map the breath to our desired alpha range
            let normalized_alpha = (breath + 1.0) / 2.0;

            min + (normalized_alpha * range)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, rc::Rc, time::Duration};

    use crate::{
        Animation, Context, InteractiveElement, Pixels, Render, SpringAnimation, SpringConfig,
        TestAppContext, WindowHandle, div, prelude::*, px, size,
    };

    use super::*;

    struct AnimationTestView {
        rendered_deltas: Rc<RefCell<Vec<f32>>>,
        max_fps: Option<f32>,
    }

    struct AnimationSequenceTestView {
        rendered_samples: Rc<RefCell<Vec<(usize, f32)>>>,
    }

    struct SyncedAnimationTestView {
        show_second: bool,
        first_deltas: Rc<RefCell<Vec<f32>>>,
        second_deltas: Rc<RefCell<Vec<f32>>>,
    }

    struct SpringAnimationTestView {
        target: Pixels,
        initial: Option<Pixels>,
        playback: SpringPlayback,
        rendered_values: Rc<RefCell<Vec<Pixels>>>,
    }

    impl Render for SpringAnimationTestView {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            let rendered_values = self.rendered_values.clone();
            let mut animation = Motion::spring(SpringConfig::new(100.0, 2.0, 1.0))
                .with_epsilon(0.01)
                .to(self.target)
                .playback(self.playback);
            if let Some(initial) = self.initial {
                animation = animation.from(initial);
            }
            div().with_spring("spring-animation", animation, move |this, value| {
                rendered_values.borrow_mut().push(value);
                this.left(value)
            })
        }
    }

    impl Render for SyncedAnimationTestView {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            let record_deltas = |deltas: Rc<RefCell<Vec<f32>>>| {
                move |this, delta| {
                    deltas.borrow_mut().push(delta);
                    this
                }
            };
            div()
                .size_full()
                .child(div().with_animation(
                    "first-synced-animation",
                    Animation::new(Duration::from_secs(1)).repeat_synced(),
                    record_deltas(self.first_deltas.clone()),
                ))
                .when(self.show_second, |this| {
                    this.child(div().with_animation(
                        "second-synced-animation",
                        Animation::new(Duration::from_secs(1)).repeat_synced(),
                        record_deltas(self.second_deltas.clone()),
                    ))
                })
        }
    }

    impl Render for AnimationTestView {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            let rendered_deltas = self.rendered_deltas.clone();
            // The throttled variant syncs to the shared clock so the deltas
            // follow the test scheduler's clock rather than wall time.
            let mut animation = Animation::new(Motion::new(Duration::from_secs(1)));
            if let Some(max_fps) = self.max_fps {
                animation = animation.repeat_synced().with_max_fps(max_fps);
            } else {
                animation = animation.repeat();
            }
            div().size_full().child(div().with_animation(
                "repeating-animation",
                animation,
                move |this, delta| {
                    rendered_deltas.borrow_mut().push(delta);
                    this
                },
            ))
        }
    }

    impl Render for AnimationSequenceTestView {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            let rendered_samples = self.rendered_samples.clone();
            div().with_animations(
                "animation-sequence",
                vec![
                    Animation::new(Duration::ZERO),
                    Animation::new(Duration::ZERO),
                ],
                move |this, animation_ix, delta| {
                    rendered_samples.borrow_mut().push((animation_ix, delta));
                    this
                },
            )
        }
    }

    fn open_test_window(
        cx: &mut TestAppContext,
    ) -> (Rc<RefCell<Vec<f32>>>, WindowHandle<AnimationTestView>) {
        open_test_window_with_max_fps(cx, None)
    }

    fn open_test_window_with_max_fps(
        cx: &mut TestAppContext,
        max_fps: Option<f32>,
    ) -> (Rc<RefCell<Vec<f32>>>, WindowHandle<AnimationTestView>) {
        let rendered_deltas = Rc::new(RefCell::new(Vec::new()));
        let window = cx.open_window(size(px(100.), px(100.)), {
            let rendered_deltas = rendered_deltas.clone();
            move |_, _| AnimationTestView {
                rendered_deltas,
                max_fps,
            }
        });
        cx.run_until_parked();
        (rendered_deltas, window)
    }

    fn simulate_next_frame<V: Render>(window: &WindowHandle<V>, cx: &mut TestAppContext) -> usize {
        let callback_count = window
            .update(cx, |_, window, cx| window.simulate_next_frame(cx))
            .unwrap();
        cx.run_until_parked();
        callback_count
    }

    #[test]
    fn test_animation_wrappers_accept_children() {
        div()
            .id("id")
            .with_animation(
                "animation",
                Animation::new(Duration::from_secs(1)),
                |element, _progress| element,
            )
            .child(div());

        div()
            .id("id")
            .with_spring(
                "spring-animation",
                SpringAnimation::new(SpringConfig::new(100.0, 10.0, 1.0))
                    .to(px(10.0))
                    .from(px(0.0)),
                |element, value| element.left(value),
            )
            .child(div());
    }

    #[gpui::test]
    fn test_spring_animation_preserves_velocity_when_retargeted(cx: &mut TestAppContext) {
        let rendered_values = Rc::new(RefCell::new(Vec::new()));
        let window = cx.open_window(size(px(100.0), px(100.0)), {
            let rendered_values = rendered_values.clone();
            move |_, _| SpringAnimationTestView {
                target: px(0.0),
                initial: None,
                playback: SpringPlayback::Running,
                rendered_values,
            }
        });
        cx.run_until_parked();
        assert_eq!(*rendered_values.borrow(), vec![px(0.0)]);

        window
            .update(cx, |view, _, cx| {
                view.target = px(100.0);
                cx.notify();
            })
            .unwrap();
        cx.run_until_parked();

        cx.executor().advance_clock(Duration::from_millis(50));
        assert!(simulate_next_frame(&window, cx) > 0);
        // Delivering the frame callback only notifies the view. Explicitly
        // flush that invalidation so randomized test-scheduler ordering cannot
        // leave the assertion observing the pre-frame value.
        cx.refresh().unwrap();
        cx.run_until_parked();
        let value_before_retargeting = *rendered_values.borrow().last().unwrap();
        assert!(value_before_retargeting > px(0.0));
        assert!(value_before_retargeting < px(100.0));

        window
            .update(cx, |view, _, cx| {
                view.target = px(0.0);
                cx.notify();
            })
            .unwrap();
        cx.run_until_parked();

        cx.executor().advance_clock(Duration::from_millis(5));
        assert!(simulate_next_frame(&window, cx) > 0);
        cx.refresh().unwrap();
        cx.run_until_parked();
        let value_after_retargeting = *rendered_values.borrow().last().unwrap();
        assert!(value_after_retargeting > value_before_retargeting);
    }

    #[gpui::test]
    fn test_paused_spring_resumes_with_its_velocity(cx: &mut TestAppContext) {
        let rendered_values = Rc::new(RefCell::new(Vec::new()));
        let window = cx.open_window(size(px(100.0), px(100.0)), {
            let rendered_values = rendered_values.clone();
            move |_, _| SpringAnimationTestView {
                target: px(0.0),
                initial: None,
                playback: SpringPlayback::Running,
                rendered_values,
            }
        });
        cx.run_until_parked();

        window
            .update(cx, |view, _, cx| {
                view.target = px(100.0);
                cx.notify();
            })
            .unwrap();
        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_millis(50));
        assert!(simulate_next_frame(&window, cx) > 0);

        window
            .update(cx, |view, _, cx| {
                view.target = px(0.0);
                view.playback = SpringPlayback::Paused;
                cx.notify();
            })
            .unwrap();
        cx.run_until_parked();
        let paused_value = *rendered_values.borrow().last().unwrap();

        cx.executor().advance_clock(Duration::from_millis(500));
        assert!(simulate_next_frame(&window, cx) > 0);
        assert_eq!(*rendered_values.borrow().last().unwrap(), paused_value);
        assert_eq!(simulate_next_frame(&window, cx), 0);

        window
            .update(cx, |view, _, cx| {
                view.playback = SpringPlayback::Running;
                cx.notify();
            })
            .unwrap();
        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_millis(5));
        assert!(simulate_next_frame(&window, cx) > 0);
        assert!(*rendered_values.borrow().last().unwrap() > paused_value);
    }

    #[gpui::test]
    fn test_stopped_spring_resumes_without_velocity(cx: &mut TestAppContext) {
        let rendered_values = Rc::new(RefCell::new(Vec::new()));
        let window = cx.open_window(size(px(100.0), px(100.0)), {
            let rendered_values = rendered_values.clone();
            move |_, _| SpringAnimationTestView {
                target: px(0.0),
                initial: None,
                playback: SpringPlayback::Running,
                rendered_values,
            }
        });
        cx.run_until_parked();

        window
            .update(cx, |view, _, cx| {
                view.target = px(1_000_000.0);
                cx.notify();
            })
            .unwrap();
        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_millis(50));
        assert!(simulate_next_frame(&window, cx) > 0);

        window
            .update(cx, |view, _, cx| {
                view.target = px(0.0);
                view.playback = SpringPlayback::Stopped;
                cx.notify();
            })
            .unwrap();
        cx.run_until_parked();
        let stopped_value = *rendered_values.borrow().last().unwrap();

        cx.executor().advance_clock(Duration::from_millis(500));
        assert!(simulate_next_frame(&window, cx) > 0);
        assert_eq!(*rendered_values.borrow().last().unwrap(), stopped_value);
        assert_eq!(simulate_next_frame(&window, cx), 0);

        window
            .update(cx, |view, _, cx| {
                view.target = stopped_value;
                view.playback = SpringPlayback::Running;
                cx.notify();
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(*rendered_values.borrow().last().unwrap(), stopped_value);
        assert_eq!(simulate_next_frame(&window, cx), 0);
    }

    #[gpui::test]
    fn test_cancelled_and_completed_springs_resolve_their_endpoints(cx: &mut TestAppContext) {
        let rendered_values = Rc::new(RefCell::new(Vec::new()));
        let window = cx.open_window(size(px(100.0), px(100.0)), {
            let rendered_values = rendered_values.clone();
            move |_, _| SpringAnimationTestView {
                target: px(100.0),
                initial: Some(px(20.0)),
                playback: SpringPlayback::Running,
                rendered_values,
            }
        });
        cx.run_until_parked();
        assert_eq!(*rendered_values.borrow(), vec![px(20.0)]);

        cx.executor().advance_clock(Duration::from_millis(50));
        assert!(simulate_next_frame(&window, cx) > 0);
        assert!(*rendered_values.borrow().last().unwrap() > px(20.0));

        window
            .update(cx, |view, _, cx| {
                view.playback = SpringPlayback::Cancelled;
                cx.notify();
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(*rendered_values.borrow().last().unwrap(), px(20.0));
        assert!(simulate_next_frame(&window, cx) > 0);
        assert_eq!(simulate_next_frame(&window, cx), 0);

        window
            .update(cx, |view, _, cx| {
                view.playback = SpringPlayback::Completed;
                cx.notify();
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(*rendered_values.borrow().last().unwrap(), px(100.0));
        assert_eq!(simulate_next_frame(&window, cx), 0);
    }

    #[gpui::test]
    fn test_animations_respect_reduced_motion(cx: &mut TestAppContext) {
        cx.update(|cx| cx.set_reduce_motion(true));
        let rendered_values = Rc::new(RefCell::new(Vec::new()));
        let window = cx.open_window(size(px(100.0), px(100.0)), {
            let rendered_values = rendered_values.clone();
            move |_, _| SpringAnimationTestView {
                target: px(100.0),
                initial: None,
                playback: SpringPlayback::Running,
                rendered_values,
            }
        });
        cx.run_until_parked();

        assert_eq!(*rendered_values.borrow(), vec![px(100.0)]);
        assert_eq!(simulate_next_frame(&window, cx), 0);

        let (rendered_deltas, window) = open_test_window(cx);
        assert_eq!(*rendered_deltas.borrow(), vec![0.0]);
        assert_eq!(simulate_next_frame(&window, cx), 0);
    }

    #[gpui::test]
    fn test_zero_duration_animation_sequence_advances_without_nan(cx: &mut TestAppContext) {
        let rendered_samples = Rc::new(RefCell::new(Vec::new()));
        let window = cx.open_window(size(px(100.0), px(100.0)), {
            let rendered_samples = rendered_samples.clone();
            move |_, _| AnimationSequenceTestView { rendered_samples }
        });
        cx.run_until_parked();

        assert_eq!(*rendered_samples.borrow(), vec![(0, 1.0)]);
        assert_eq!(simulate_next_frame(&window, cx), 1);
        assert_eq!(*rendered_samples.borrow(), vec![(0, 1.0), (1, 1.0)]);
        assert_eq!(simulate_next_frame(&window, cx), 0);
    }

    #[gpui::test]
    fn test_repeating_animation_schedules_animation_frames(cx: &mut TestAppContext) {
        let (rendered_deltas, window) = open_test_window(cx);

        assert_eq!(rendered_deltas.borrow().len(), 1);

        for expected_frames in 2..=3 {
            assert_eq!(simulate_next_frame(&window, cx), 1);
            assert_eq!(rendered_deltas.borrow().len(), expected_frames);
        }
    }

    #[gpui::test]
    fn test_max_fps_schedules_timer_driven_frames(cx: &mut TestAppContext) {
        let (rendered_deltas, window) = open_test_window_with_max_fps(cx, Some(10.0));

        // The test scheduler's clock jitters forward slightly on each poll,
        // so compare against expectations loosely.
        let assert_deltas_approx_eq = |expected: &[f32]| {
            let actual = rendered_deltas.borrow();
            assert_eq!(actual.len(), expected.len(), "deltas: {actual:?}");
            for (actual, expected) in actual.iter().zip(expected) {
                assert!(
                    (actual - expected).abs() < 1e-2,
                    "expected {expected}, got {actual}"
                );
            }
        };

        assert_deltas_approx_eq(&[0.0]);

        // No per-frame callback is scheduled; re-renders are timer-driven.
        assert_eq!(simulate_next_frame(&window, cx), 0);
        assert_deltas_approx_eq(&[0.0]);

        cx.executor().advance_clock(Duration::from_millis(105));
        cx.run_until_parked();
        assert_deltas_approx_eq(&[0.0, 0.105]);

        cx.executor().advance_clock(Duration::from_millis(105));
        cx.run_until_parked();
        assert_deltas_approx_eq(&[0.0, 0.105, 0.21]);
    }

    #[gpui::test]
    fn test_synced_animations_share_phase_across_elements(cx: &mut TestAppContext) {
        let first_deltas = Rc::new(RefCell::new(Vec::new()));
        let second_deltas = Rc::new(RefCell::new(Vec::new()));
        let window = cx.open_window(size(px(100.), px(100.)), {
            let first_deltas = first_deltas.clone();
            let second_deltas = second_deltas.clone();
            move |_, _| SyncedAnimationTestView {
                show_second: false,
                first_deltas,
                second_deltas,
            }
        });
        cx.run_until_parked();

        assert_eq!(*first_deltas.borrow(), vec![0.0]);

        cx.executor().advance_clock(Duration::from_millis(250));
        simulate_next_frame(&window, cx);
        assert_eq!(*first_deltas.borrow(), vec![0.0, 0.25]);

        // The second element mounts a quarter through the cycle, yet renders
        // the shared phase rather than starting at zero.
        window
            .update(cx, |view, _, cx| {
                view.show_second = true;
                cx.notify();
            })
            .unwrap();
        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_millis(250));
        simulate_next_frame(&window, cx);

        assert_eq!(*second_deltas.borrow().last().unwrap(), 0.5);
        assert_eq!(
            *first_deltas.borrow().last().unwrap(),
            *second_deltas.borrow().last().unwrap()
        );
        assert!(second_deltas.borrow().iter().all(|delta| *delta > 0.0));

        // The phase wraps around each full cycle.
        cx.executor().advance_clock(Duration::from_millis(2250));
        simulate_next_frame(&window, cx);
        assert_eq!(*first_deltas.borrow().last().unwrap(), 0.75);

        // Sub-second precision survives months of uptime: converting the raw
        // elapsed time to f32 would round 0.25 away entirely.
        cx.executor()
            .advance_clock(Duration::from_secs(300 * 24 * 60 * 60) + Duration::from_millis(500));
        simulate_next_frame(&window, cx);
        assert_eq!(*first_deltas.borrow().last().unwrap(), 0.25);
    }
}
