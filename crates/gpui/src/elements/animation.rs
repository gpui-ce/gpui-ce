use scheduler::Instant;
use std::{cell::Cell, rc::Rc, time::Duration};

use crate::{
    AnyElement, App, AppContext, Element, ElementId, GlobalElementId, InspectorElementId,
    IntoElement, Motion, MotionExtent, ParentElement, SpringAnimation, SpringConfig,
    SpringDescription, SpringPlayback, SpringState, SpringTarget, Window,
};

pub use easing::*;
use smallvec::SmallVec;

/// An animation that can be applied to an element.
#[derive(Clone)]
pub struct Animation {
    /// The timing, repetition, and easing applied to this animation.
    pub motion: Motion,
    /// Whether to derive the phase from a shared clock. See [`Animation::repeat_synced`].
    pub synced: bool,
    /// The maximum number of times per second this animation re-renders.
    /// When `None`, the animation re-renders on every frame.
    pub max_fps: Option<f32>,
}

impl Animation {
    /// Creates an animation with the given motion.
    ///
    /// Duration inputs create one linear motion pass.
    pub fn new(motion: impl Into<Motion>) -> Self {
        Self {
            motion: motion.into(),
            synced: false,
            max_fps: None,
        }
    }

    /// Set the animation to loop when it finishes.
    pub fn repeat(mut self) -> Self {
        self.motion = self.motion.repeat_forever();
        self
    }

    /// Set the animation to loop when it finishes, phase-locked to a clock shared by the whole [`App`].
    pub fn repeat_synced(mut self) -> Self {
        self.motion = self.motion.repeat_forever();
        self.synced = true;
        self
    }

    /// Sets easing without clamping the output, so curves may overshoot.
    pub fn with_easing(mut self, easing: impl Fn(f32) -> f32 + 'static) -> Self {
        self.motion = self.motion.with_easing(easing);
        self
    }

    /// Limits re-renders to `max_fps` by scheduling a timer between frames.
    /// Non-finite and non-positive values are ignored.
    pub fn with_max_fps(mut self, max_fps: f32) -> Self {
        self.max_fps = Some(max_fps);
        self
    }
}

/// An extension trait for adding the animation wrapper to both Elements and Components
///
/// Animations rendered through this trait automatically respect
/// [`App::reduce_motion`](crate::App::reduce_motion): when it is set,
/// the element is rendered in a static state (the end state for oneshot
/// animations, the start state for repeating ones) and no animation frames are
/// scheduled.
pub trait AnimationExt {
    /// Render this component or element with an animation
    fn with_animation(
        self,
        id: impl Into<ElementId>,
        animation: Animation,
        animator: impl Fn(Self, f32) -> Self + 'static,
    ) -> AnimationElement<Self>
    where
        Self: Sized,
    {
        let timing = AnimationSchedule::new(std::slice::from_ref(&animation));
        AnimationElement {
            id: id.into(),
            element: Some(self),
            animator: Box::new(move |this, _, value| animator(this, value)),
            animations: smallvec::smallvec![animation],
            timing,
            replay_key: None,
        }
    }

    /// Runs animations sequentially. An exact handoff samples the incoming
    /// ordinary animation at local elapsed time zero; reverse motion or custom
    /// easing can make its presented progress nonzero. Late frames skip elapsed
    /// animations and carry remaining elapsed time into the selected animation.
    /// Chains with zero total duration settle in one render; empty chains leave
    /// the element unchanged. Only the currently selected callback runs.
    ///
    /// Placement uses the element's mount clock. A synchronized child's value
    /// still uses the App epoch independently of its placement, including at
    /// handoff. Frame throttling follows the selected animation.
    ///
    /// An unbounded animation or overflowing duration makes later animations
    /// unreachable.
    fn with_animations(
        self,
        id: impl Into<ElementId>,
        animations: Vec<Animation>,
        animator: impl Fn(Self, usize, f32) -> Self + 'static,
    ) -> AnimationElement<Self>
    where
        Self: Sized,
    {
        let timing = AnimationSchedule::new(&animations);
        AnimationElement {
            id: id.into(),
            element: Some(self),
            animator: Box::new(animator),
            animations: animations.into(),
            timing,
            replay_key: None,
        }
    }

    /// Renders this component or element at the value produced by a spring.
    ///
    /// The element ID preserves position and velocity across target changes.
    /// A newly mounted spring starts at its target unless configured with
    /// [`SpringAnimation::from`].
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

/// A GPUI element that applies an animation to another element
pub struct AnimationElement<E> {
    id: ElementId,
    element: Option<E>,
    animations: SmallVec<[Animation; 1]>,
    timing: AnimationSchedule,
    replay_key: Option<ElementId>,
    animator: Box<dyn Fn(E, usize, f32) -> E + 'static>,
}

/// A GPUI element driven by a stateful spring.
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
    /// Returns a new [`SpringAnimationElement<E>`] after applying the given function
    /// to the element being animated.
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
    /// Restarts the animation when `event_id` changes between renders.
    ///
    /// Keep the animation's element ID stable and change this key for each
    /// application event, including events with identical payloads. An unchanged
    /// key leaves the run's elapsed clock unchanged. The first mount still
    /// animates normally.
    /// Replay restarts at the configured origin and delay, even during an active
    /// run. It does not remount the element or its children.
    ///
    /// Reduced motion resolves a replay to its resting value and consumes the
    /// event, even if the preference is disabled before the next layout. A new
    /// key observed after disabling the preference can start another run.
    /// Synchronized animations retain their App epoch;
    /// use an ordinary [`Animation`] for an event-local phase.
    pub fn replay_on(mut self, event_id: impl Into<ElementId>) -> Self {
        self.replay_key = Some(event_id.into());
        self
    }

    /// Returns a new [`AnimationElement<E>`] after applying the given function
    /// to the element being animated.
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
    replay_key: Option<ElementId>,
    reduced_replay: bool,
    preference_epoch: u64,
    /// Whether a throttled re-render (see [`Animation::with_max_fps`]) is
    /// already scheduled, so overlapping renders don't stack extra timers.
    delayed_frame_pending: Rc<Cell<bool>>,
    delayed_frame: Option<crate::Task<()>>,
    delayed_frame_interval: Option<Duration>,
}

// Delays and every configured pass contribute to placement. An unbounded or
// overflowing extent prevents placement of later animations.
struct AnimationSchedule {
    starts: SmallVec<[Duration; 1]>,
}

impl AnimationSchedule {
    fn new(animations: &[Animation]) -> Self {
        let mut starts = SmallVec::new();
        let mut start = Duration::ZERO;
        for animation in animations {
            starts.push(start);
            let Some(MotionExtent::Finite(duration)) = animation.motion.checked_extent() else {
                break;
            };
            let Some(next) = start.checked_add(duration) else {
                break;
            };
            start = next;
        }
        Self { starts }
    }

    fn select(&self, animations: &[Animation], elapsed: Duration) -> Option<(usize, Duration)> {
        let index = self
            .starts
            .partition_point(|start| *start <= elapsed)
            .checked_sub(1)?;
        let local = elapsed.saturating_sub(self.starts[index]);
        // The last reachable step holds its actual terminal sample.
        let local = match animations[index].motion.checked_extent() {
            Some(MotionExtent::Finite(end)) => local.min(end),
            _ => local,
        };
        Some((index, local))
    }
}

fn animation_sequence_sample(
    animations: &[Animation],
    timing: &AnimationSchedule,
    elapsed: Duration,
    epoch_elapsed: Duration,
    reduced: bool,
) -> Option<(usize, f32, bool)> {
    let (index, local) = if reduced {
        (animations.len().checked_sub(1)?, Duration::ZERO)
    } else {
        timing.select(animations, elapsed)?
    };
    let animation = &animations[index];
    let value = if reduced {
        animation.motion.resting_progress().get()
    } else {
        animation
            .motion
            .sample(if animation.synced {
                epoch_elapsed
            } else {
                local
            })
            .progress
            .get()
    };
    // A single synchronized animation uses epoch-based completion.
    // A sequence uses its mount-local placement while sampling synced values at the epoch.
    let completion_time = if animations.len() == 1 && animation.synced {
        epoch_elapsed
    } else {
        local
    };
    let done = reduced || !animation.motion.sample(completion_time).is_active;
    Some((index, value, done))
}

fn frame_interval(max_fps: Option<f32>) -> Option<Duration> {
    let fps = max_fps.filter(|fps| fps.is_finite() && *fps > 0.0)?;
    Duration::try_from_secs_f64(1.0 / f64::from(fps))
        .ok()
        .filter(|interval| !interval.is_zero())
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
            let preference_epoch = cx
                .try_global::<crate::AnimationPreference>()
                .map_or(0, |preference| preference.0);
            let mut state = state.unwrap_or_else(|| AnimationState {
                start: now,
                animation_ix: 0,
                replay_key: self.replay_key.clone(),
                reduced_replay: false,
                preference_epoch,
                delayed_frame_pending: Rc::new(Cell::new(false)),
                delayed_frame: None,
                delayed_frame_interval: None,
            });
            if state.replay_key != self.replay_key {
                state.reduced_replay = false;
                state.preference_epoch = preference_epoch;
                if self.replay_key.is_some() {
                    state.start = now;
                    state.animation_ix = 0;
                    state.delayed_frame = None;
                    state.delayed_frame_pending = Rc::new(Cell::new(false));
                }
                state.replay_key = self.replay_key.clone();
            }
            if self.replay_key.is_some()
                && (cx.reduce_motion() || state.preference_epoch != preference_epoch)
            {
                state.reduced_replay = true;
            }
            state.preference_epoch = preference_epoch;
            let selected = animation_sequence_sample(
                &self.animations,
                &self.timing,
                now - state.start,
                now - cx.synced_animation_epoch,
                cx.reduce_motion() || state.reduced_replay,
            );
            let (animation_ix, delta, done) = if let Some((animation_ix, delta, done)) = selected {
                if state.animation_ix != animation_ix {
                    state.animation_ix = animation_ix;
                    state.delayed_frame = None;
                    state.delayed_frame_pending = Rc::new(Cell::new(false));
                }
                (Some(animation_ix), delta, done)
            } else {
                (None, 0.0, true)
            };

            debug_assert!(delta.is_finite(), "animated value should be finite");

            let element = self.element.take().expect("should only be called once");
            let mut element = if let Some(animation_ix) = animation_ix {
                (self.animator)(element, animation_ix, delta)
            } else {
                element
            }
            .into_any_element();

            let interval = if done {
                None
            } else {
                frame_interval(animation_ix.and_then(|ix| self.animations[ix].max_fps))
            };
            if state.delayed_frame_interval != interval {
                state.delayed_frame = None;
                state.delayed_frame_pending = Rc::new(Cell::new(false));
                state.delayed_frame_interval = interval;
            }
            if done {
                state.delayed_frame = None;
                state.delayed_frame_pending.set(false);
            }
            if !done {
                match interval {
                    Some(interval) => {
                        if !state.delayed_frame_pending.get() {
                            state.delayed_frame_pending.set(true);
                            let delayed_frame_pending = state.delayed_frame_pending.clone();
                            let view = window.current_view();
                            state.delayed_frame = Some(window.spawn(cx, async move |cx| {
                                cx.background_executor().timer(interval).await;
                                delayed_frame_pending.set(false);
                                cx.update(move |_, cx| cx.notify(view)).ok();
                            }));
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

    /// The linear easing function, or delta itself
    pub fn linear(delta: f32) -> f32 {
        delta
    }

    /// The quadratic easing function, delta * delta
    pub fn quadratic(delta: f32) -> f32 {
        delta * delta
    }

    /// The quadratic ease-in-out function, which starts and ends slowly but speeds up in the middle
    pub fn ease_in_out(delta: f32) -> f32 {
        if delta < 0.5 {
            2.0 * delta * delta
        } else {
            let x = -2.0 * delta + 2.0;
            1.0 - x * x / 2.0
        }
    }

    /// The Quint ease-out function, which starts quickly and decelerates to a stop
    pub fn ease_out_quint() -> impl Fn(f32) -> f32 {
        move |delta| 1.0 - (1.0 - delta).powi(5)
    }

    /// Apply the given easing function, first in the forward direction and then in the reverse direction
    pub fn bounce(easing: impl Fn(f32) -> f32) -> impl Fn(f32) -> f32 {
        move |delta| {
            if delta < 0.5 {
                easing(delta * 2.0)
            } else {
                easing((1.0 - delta) * 2.0)
            }
        }
    }

    /// A custom easing function for pulsating alpha that slows down as it approaches 0.1
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
#[path = "animation/replay_tests.rs"]
mod replay_tests;

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

    struct TimedSequenceTestView {
        animations: Vec<Animation>,
        rendered_samples: Rc<RefCell<Vec<(usize, f32)>>>,
    }

    impl Render for TimedSequenceTestView {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let samples = self.rendered_samples.clone();
            div().with_animations(
                "timed-sequence",
                self.animations.clone(),
                move |element, index, value| {
                    samples.borrow_mut().push((index, value));
                    element
                },
            )
        }
    }

    #[test]
    fn sequence_adapter_exact_boundaries_late_frames_sync_and_throttle_selection() {
        let animations = vec![
            Animation::new(Duration::from_millis(100)).with_max_fps(10.0),
            Animation::new(Duration::from_millis(100)),
            Animation::new(Duration::from_millis(1000))
                .repeat_synced()
                .with_max_fps(20.0),
        ];
        let timing = AnimationSchedule::new(&animations);
        assert_eq!(
            animation_sequence_sample(
                &animations,
                &timing,
                Duration::from_millis(100),
                Duration::from_millis(500),
                false
            ),
            Some((1, 0.0, false))
        );
        assert_eq!(
            animation_sequence_sample(
                &animations,
                &timing,
                Duration::from_millis(150),
                Duration::from_millis(550),
                false
            ),
            Some((1, 0.5, false))
        );
        let sample = animation_sequence_sample(
            &animations,
            &timing,
            Duration::from_millis(250),
            Duration::from_millis(650),
            false,
        )
        .unwrap();
        assert_eq!(sample, (2, 0.65, false));
        assert_eq!(
            frame_interval(animations[sample.0].max_fps),
            Some(Duration::from_millis(50))
        );
        assert_eq!(
            animation_sequence_sample(
                &[],
                &AnimationSchedule::new(&[]),
                Duration::ZERO,
                Duration::ZERO,
                false
            ),
            None
        );
        for fps in [
            0.0,
            -1.0,
            f32::INFINITY,
            f32::NAN,
            f32::from_bits(1),
            f32::MAX,
        ] {
            assert_eq!(frame_interval(Some(fps)), None);
        }
        assert_eq!(
            animation_sequence_sample(
                &animations,
                &timing,
                Duration::from_millis(250),
                Duration::ZERO,
                true
            ),
            Some((2, 0.0, true))
        );
    }

    #[gpui::test]
    fn sequence_adapter_carries_late_elapsed_through_real_element(cx: &mut TestAppContext) {
        let rendered_samples = Rc::new(RefCell::new(Vec::new()));
        let window = cx.open_window(size(px(100.0), px(100.0)), {
            let rendered_samples = rendered_samples.clone();
            move |_, _| TimedSequenceTestView {
                animations: vec![
                    Animation::new(Duration::from_millis(100)),
                    Animation::new(Duration::from_millis(100)),
                    Animation::new(Duration::from_millis(400)),
                ],
                rendered_samples,
            }
        });
        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_millis(300));
        simulate_next_frame(&window, cx);
        cx.refresh().unwrap();
        cx.run_until_parked();
        let (index, value) = *rendered_samples.borrow().last().unwrap();
        assert_eq!(index, 2);
        assert!((value - 0.25).abs() < 1e-3);
        assert!(
            rendered_samples
                .borrow()
                .iter()
                .all(|(index, _)| *index != 1)
        );
    }

    #[gpui::test]
    fn sequence_adapter_handoff_changes_frame_throttle(cx: &mut TestAppContext) {
        let rendered_samples = Rc::new(RefCell::new(Vec::new()));
        let window = cx.open_window(size(px(100.0), px(100.0)), {
            let rendered_samples = rendered_samples.clone();
            move |_, _| TimedSequenceTestView {
                animations: vec![
                    Animation::new(Duration::from_millis(100)).with_max_fps(10.0),
                    Animation::new(Duration::from_secs(1)).with_max_fps(20.0),
                ],
                rendered_samples,
            }
        });
        cx.run_until_parked();
        assert_eq!(simulate_next_frame(&window, cx), 0);
        cx.executor().advance_clock(Duration::from_millis(105));
        cx.run_until_parked();
        assert_eq!(rendered_samples.borrow().last().unwrap().0, 1);
        let renders = rendered_samples.borrow().len();
        cx.executor().advance_clock(Duration::from_millis(55));
        cx.run_until_parked();
        assert_eq!(rendered_samples.borrow().len(), renders + 1);
        assert!(
            (rendered_samples.borrow().last().unwrap().1 - 0.05).abs() < 0.005,
            "samples: {:?}",
            rendered_samples.borrow()
        );
    }

    #[gpui::test]
    fn sequence_handoff_cancels_pending_throttle(cx: &mut TestAppContext) {
        let samples = Rc::new(RefCell::new(Vec::new()));
        let window = cx.open_window(size(px(100.0), px(100.0)), {
            let samples = samples.clone();
            move |_, _| TimedSequenceTestView {
                animations: vec![
                    Animation::new(Duration::from_millis(100)).with_max_fps(0.5),
                    Animation::new(Duration::from_millis(100)),
                ],
                rendered_samples: samples,
            }
        });
        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_millis(210));
        cx.update_window(window.into(), |_, window, cx| {
            window.refresh();
            window.draw(cx).clear(cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(samples.borrow().last().unwrap().0, 1);
        assert_eq!(samples.borrow().last().unwrap().1, 1.0);
        let renders = samples.borrow().len();
        cx.executor().advance_clock(Duration::from_secs(3));
        cx.run_until_parked();
        assert_eq!(
            samples.borrow().len(),
            renders,
            "obsolete detached timer must not wake resting view"
        );
        assert_eq!(simulate_next_frame(&window, cx), 0);
    }

    #[gpui::test]
    fn reduced_motion_and_empty_sequence_cancel_pending_throttle(cx: &mut TestAppContext) {
        for reduced in [false, true] {
            let samples = Rc::new(RefCell::new(Vec::new()));
            let window = cx.open_window(size(px(100.0), px(100.0)), {
                let samples = samples.clone();
                move |_, _| TimedSequenceTestView {
                    animations: vec![Animation::new(Duration::from_secs(1)).with_max_fps(0.5)],
                    rendered_samples: samples,
                }
            });
            cx.run_until_parked();
            if reduced {
                cx.update(|cx| cx.set_reduce_motion(true));
                cx.refresh().unwrap();
            } else {
                window
                    .update(cx, |v, _, cx| {
                        v.animations.clear();
                        cx.notify();
                    })
                    .unwrap();
            }
            cx.run_until_parked();
            let renders = samples.borrow().len();
            cx.executor().advance_clock(Duration::from_secs(3));
            cx.run_until_parked();
            assert_eq!(
                samples.borrow().len(),
                renders,
                "cancelled throttle cannot redraw resting content"
            );
            assert_eq!(simulate_next_frame(&window, cx), 0);
        }
    }

    #[gpui::test]
    fn finite_synchronized_animation_mounted_after_epoch_completion_rests(cx: &mut TestAppContext) {
        cx.executor().advance_clock(Duration::from_millis(300));
        let samples = Rc::new(RefCell::new(Vec::new()));
        let window = cx.open_window(size(px(100.0), px(100.0)), {
            let samples = samples.clone();
            move |_, _| TimedSequenceTestView {
                animations: vec![Animation {
                    motion: Motion::new(Duration::from_millis(100)),
                    synced: true,
                    max_fps: None,
                }],
                rendered_samples: samples,
            }
        });
        cx.run_until_parked();
        assert_eq!(*samples.borrow().last().unwrap(), (0, 1.0));
        assert_eq!(simulate_next_frame(&window, cx), 0);
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

        assert_eq!(*rendered_samples.borrow(), vec![(1, 1.0)]);
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
