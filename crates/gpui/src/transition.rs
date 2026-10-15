use std::{
    cell::{Ref, RefCell},
    time::Instant,
};

use crate::{Animated, App, Entity, EntityId, Motion, Progress, Window, lerp::Lerp};

#[derive(Clone)]
struct TransitionCache<T> {
    value: Option<T>,
    progress: Progress,
    is_active: bool,
}

impl<T> TransitionCache<T> {
    fn empty() -> Self {
        Self {
            value: None,
            progress: Progress::END,
            is_active: false,
        }
    }

    fn clear(&mut self) {
        self.value = None;
    }
}

/// A value that moves toward a goal over time and changes course when that goal changes.
///
/// A `Transition` pairs an entity that holds a [`TransitionState`] with a [`Motion`]. Read the
/// value for the current frame with [`evaluate`](Self::evaluate) during render. Change the goal
/// with [`update`](Self::update) from an event handler, then call `cx.notify()` so the view
/// redraws. `T` must implement [`Lerp`], [`Clone`] and [`PartialEq`].
///
/// Clones share the goal and motion state, but each has its own per-frame cache. Read the value
/// through the clone that is rendering it.
///
/// Create one during render with [`Window::use_keyed_transition`]. Its key keeps the state across
/// frames. [`Window::use_transition`] keys state by call site instead, so that state lasts only
/// while the element renders at that call site in consecutive frames.
///
/// # Continuous and non-continuous mode
///
/// Transitions are continuous by default. When the goal changes:
///
/// - **Continuous** (the default): the value continues from where it is now toward the new goal.
/// - **Non-continuous** ([`continuous(false)`](Self::continuous)): the value restarts from the
///   initial value and moves toward the new goal.
///
/// # Examples
///
/// The goal can change from an event handler, and the value follows it. This example uses an
/// easing of `|_| 0.5`, so the value sits halfway between its start and its goal, and the
/// assertions are exact.
///
/// ```
/// use gpui::{
///     AppContext, Context, IntoElement, Render, TestApp, Transition, TransitionState, Window,
///     div, prelude::*, secs,
/// };
///
/// struct Fade {
///     transition: Transition<f32>,
/// }
///
/// impl Render for Fade {
///     fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
///         let opacity = *self.transition.evaluate(window, cx);
///         div().opacity(opacity)
///     }
/// }
///
/// let mut app = TestApp::new();
/// let mut window = app.open_window(|_, cx| {
///     let state = cx.new(|_| TransitionState::new(0.0_f32, secs(10)));
///     Fade {
///         transition: Transition::new(state, secs(10)).with_easing(|_| 0.5),
///     }
/// });
///
/// window.update(|view, _, cx| {
///     view.transition.update(cx, |goal, cx| {
///         *goal = 1.0;
///         cx.notify();
///     });
/// });
/// window.draw();
///
/// window.update(|view, _, cx| {
///     assert_eq!(*view.transition.read_cache(), Some(0.5));
///     assert_eq!(*view.transition.read_goal(cx), 1.0);
/// });
/// ```
///
/// See the [animation guide](crate::_documentation::guides::animation#transition-a-value-that-changes)
/// for the keyed hook.
#[derive(Clone)]
pub struct Transition<T: Lerp + Clone + PartialEq + 'static> {
    motion: Motion,

    state: Entity<TransitionState<T>>,

    /// The transition sample cached for this render.
    cache: RefCell<TransitionCache<T>>,

    /// Whether to continue the transition from the current value when the goal changes.
    /// If true, transitions smoothly from current animated value to new goal.
    /// If false, restarts from the original start value.
    continuous: bool,
}

impl<T: Lerp + Clone + PartialEq + 'static> Transition<T> {
    /// Creates a transition that stores its state in `state` and animates with `motion`.
    ///
    /// Inside render, prefer [`Window::use_keyed_transition`], which creates the state for you.
    pub fn new(state: Entity<TransitionState<T>>, motion: impl Into<Motion>) -> Self {
        Self {
            motion: motion.into(),
            state,
            cache: RefCell::new(TransitionCache::empty()),
            continuous: true,
        }
    }

    /// Sets the easing of the forward pass. See [`Motion::with_easing`].
    ///
    /// The easing maps normalized pass time to presentation progress. It may be non-monotonic, and
    /// it may overshoot zero through one.
    pub fn with_easing(mut self, easing: impl Fn(f32) -> f32 + 'static) -> Self {
        self.motion = self.motion.with_easing(easing);
        self.clear_cache();
        self
    }

    /// Sets whether a new goal continues from the current value.
    ///
    /// Continuous is the default. Pass `false` to restart from the initial value on every goal
    /// change instead.
    pub fn continuous(mut self, continuous: bool) -> Self {
        self.continuous = continuous;
        self
    }

    fn sample(&self, cx: &mut App) -> Ref<'_, TransitionCache<T>> {
        if self.cache.borrow().value.is_none() {
            let mut state = self.state.as_mut(cx);
            let sample = state.sample(Instant::now());

            *self.cache.borrow_mut() = TransitionCache {
                value: Some(sample.value),
                progress: sample.progress,
                is_active: sample.is_active,
            };
        }

        self.cache.borrow()
    }

    fn clear_cache(&self) {
        self.cache.borrow_mut().clear();
    }

    /// Returns the value for the current frame, and requests another frame while it moves.
    ///
    /// The value depends on the time since the last goal change, and on the easing. The first call
    /// samples the clock and caches the result. Later calls return the cached value until the goal
    /// changes or the cache is cleared, so a `Transition` kept across frames returns the same value.
    /// [`Window::use_keyed_transition`] returns a new `Transition` on every render, which avoids
    /// this. Call this during render or paint.
    pub fn evaluate(&self, window: &mut Window, cx: &mut App) -> Ref<'_, T> {
        let sample = self.sample(cx);
        if sample.is_active {
            window.request_animation_frame();
        }

        Ref::map(sample, |sample| sample.value.as_ref().unwrap())
    }

    /// Returns the goal: the value most recently set by [`update`](Self::update).
    ///
    /// This isn't the value on screen while the transition moves. Use [`evaluate`](Self::evaluate)
    /// for that.
    pub fn read_goal<'b>(&'b self, cx: &'b mut App) -> &'b T {
        self.state.read(cx).value()
    }

    /// Returns the value cached by the last [`evaluate`](Self::evaluate), or `None` if the cache
    /// was cleared.
    ///
    /// The cache is cleared when the goal changes, and by [`jump_to`](Self::jump_to),
    /// [`scale_by`](Self::scale_by), [`reset`](Self::reset) and [`with_easing`](Self::with_easing).
    pub fn read_cache(&self) -> Ref<'_, Option<T>> {
        Ref::map(self.cache.borrow(), |cache| &cache.value)
    }

    /// Returns the eased progress for the current frame, without requesting another frame.
    ///
    /// Progress may overshoot zero through one. It doesn't mean the run has finished: restarting
    /// iterations begin again, and alternating iterations run in reverse.
    pub fn evaluate_delta(&self, cx: &App) -> f32 {
        if self.cache.borrow().value.is_some() {
            return self.cache.borrow().progress.get();
        }

        self.state.read(cx).progress_at(Instant::now()).get()
    }

    /// Changes the goal, and starts a run toward the new goal if it changed.
    ///
    /// `update` passes the current goal to the closure, which can change it. If the new goal
    /// differs from the old one, the value moves toward it. In continuous mode the run starts from
    /// the current value. Otherwise it starts from the initial value. The goal isn't read during
    /// render, so change it from an event handler.
    ///
    /// Returns `true` if the goal changed, and `false` if it stayed the same.
    ///
    /// This doesn't notify GPUI. Call `cx.notify()` inside the closure to redraw the view.
    pub fn update<R>(
        &self,
        cx: &mut App,
        update: impl FnOnce(&mut T, &mut crate::Context<TransitionState<T>>) -> R,
    ) -> bool {
        let mut was_updated = false;

        self.state.update(cx, |state, cx| {
            let mut value = state.value().clone();
            update(&mut value, cx);
            was_updated = if self.continuous {
                state.set(value, &self.motion, Instant::now())
            } else {
                state.restart(value, &self.motion, Instant::now())
            };
        });

        if was_updated {
            self.clear_cache();
        }

        was_updated
    }

    /// Sets the value and the goal to `target`, with no animation.
    ///
    /// Later calls to [`evaluate`](Self::evaluate) return `target` until the goal changes again.
    /// Use this when the next animation should start from a different value.
    pub fn jump_to(&self, target: T, cx: &mut App) {
        self.state.update(cx, |state, _cx| state.jump_to(target));
        self.clear_cache();
    }

    /// Multiplies the start value and the goal by `ratio`.
    ///
    /// An in-flight transition keeps its progress when the coordinate space changes, such as a
    /// window resize. Pass the ratio of the new scale to the old one.
    pub fn scale_by(&self, ratio: f32, cx: &mut App)
    where
        T: std::ops::Mul<f32, Output = T>,
    {
        self.state.update(cx, |state, _cx| state.scale_by(ratio));
        self.clear_cache();
    }

    /// Returns the ID of the entity that holds this transition's state.
    ///
    /// Use it to tell two transitions apart.
    pub fn entity_id(&self) -> EntityId {
        self.state.entity_id()
    }

    /// Returns the transition to its initial value, with no animation.
    ///
    /// This clears any progress and sets the goal back to the value the transition was created
    /// with. The cache is cleared too.
    pub fn reset(&self, cx: &mut App) {
        self.state.update(cx, |state, _cx| state.reset());
        self.clear_cache();
    }
}

/// The state behind a [`Transition`]: an [`Animated`] that reads the clock from [`Instant`].
///
/// Create one with `cx.new(|_| TransitionState::new(initial, motion))` and pass it to
/// [`Transition::new`]. [`Window::use_keyed_transition`] does that for you.
pub type TransitionState<T> = Animated<T, Instant>;

#[cfg(all(test, feature = "test-support"))]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::time::Duration;

    use crate::{AppContext, Context, IntoElement, Render, canvas, px, size};

    use super::*;
    use gpui::TestAppContext;

    /// Helper to create a Transition directly without using window hooks.
    /// This bypasses the render-phase restriction of use_transition/use_keyed_transition.
    fn create_transition<T: Lerp + Clone + PartialEq + 'static>(
        cx: &mut App,
        motion: impl Into<Motion>,
        initial: T,
    ) -> Transition<T> {
        let motion = motion.into();
        let state = cx.new(|_| TransitionState::new(initial, motion.clone()));
        Transition::new(state, motion)
    }

    #[gpui::test]
    fn unkeyed_transitions_have_distinct_state_that_persists(cx: &mut TestAppContext) {
        struct Hooks {
            seen: Rc<RefCell<Vec<[EntityId; 2]>>>,
        }

        impl Render for Hooks {
            fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
                let first = window.use_transition(cx, Duration::from_secs(1), |_, _| 2.0_f32);
                let second = window.use_transition(cx, Duration::from_secs(1), |_, _| 7.0_f32);
                assert_eq!(*first.read_goal(cx), 2.0);
                assert_eq!(*second.read_goal(cx), 7.0);
                self.seen
                    .borrow_mut()
                    .push([first.entity_id(), second.entity_id()]);
                crate::Empty
            }
        }

        let seen = Rc::new(RefCell::new(Vec::new()));
        let window = cx.open_window(size(px(100.), px(100.)), {
            let seen = seen.clone();
            move |_, _| Hooks { seen }
        });
        cx.run_until_parked();
        window.update(cx, |_, _, cx| cx.notify()).unwrap();
        cx.run_until_parked();

        let seen = seen.borrow();
        assert!(seen.len() >= 2);
        assert_ne!(seen[0][0], seen[0][1]);
        assert!(seen.iter().all(|ids| *ids == seen[0]));
    }

    struct TransitionTestView {
        transition: Transition<f32>,
        rendered_values: Rc<RefCell<Vec<f32>>>,
    }

    impl Render for TransitionTestView {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            let transition = self.transition.clone();
            let rendered_values = self.rendered_values.clone();
            canvas(
                move |_, window, cx| {
                    rendered_values
                        .borrow_mut()
                        .push(*transition.evaluate(window, cx));
                },
                |_, _, _, _| {},
            )
        }
    }

    #[gpui::test]
    fn transition_exercises_state_updates_and_cache(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let transition =
                create_transition(cx, Duration::from_secs(1), 0.0_f32).with_easing(|_| 0.5);

            assert!(transition.read_cache().is_none());
            assert!(transition.update(cx, |value, _| *value = 100.0));
            assert_eq!(*transition.read_goal(cx), 100.0);
            let entity_id = transition.entity_id();
            assert_eq!(entity_id, transition.entity_id());

            let value1 = *transition.sample(cx).value.as_ref().unwrap();
            let value2 = *transition.sample(cx).value.as_ref().unwrap();
            assert_eq!((value1, value2), (50.0, 50.0));
            assert_eq!(transition.evaluate_delta(cx), 0.5);

            assert!(!transition.update(cx, |value, _| *value = 100.0));
            assert!(transition.read_cache().is_some());
            assert!(transition.update(cx, |value, _| *value = 200.0));
            assert!(transition.read_cache().is_none());

            let transition = transition.with_easing(|_| 0.75);
            assert!(transition.read_cache().is_none());
            assert_eq!(*transition.sample(cx).value.as_ref().unwrap(), 125.0);
            assert_eq!(transition.evaluate_delta(cx), 0.5);

            let motion = Motion::new(Duration::from_secs(1)).with_easing(|_| 0.5);
            let continuous = create_transition(cx, motion.clone(), 0.0_f32);
            let restarting = create_transition(cx, motion, 0.0_f32).continuous(false);

            for transition in [&continuous, &restarting] {
                assert!(transition.update(cx, |value, _| *value = 100.0));
                assert_eq!(*transition.sample(cx).value.as_ref().unwrap(), 50.0);
            }

            assert!(continuous.update(cx, |value, _| *value = 200.0));
            assert!(restarting.update(cx, |value, _| *value = 200.0));

            assert_eq!(*continuous.sample(cx).value.as_ref().unwrap(), 125.0);
            assert_eq!(*restarting.sample(cx).value.as_ref().unwrap(), 100.0);

            let motion = Motion::new(Duration::from_secs(1)).with_easing(|_| 0.5);
            let mutators = create_transition(cx, motion, 2.0_f32);

            assert!(mutators.update(cx, |value, _| *value = 10.0));
            assert_eq!(*mutators.sample(cx).value.as_ref().unwrap(), 6.0);

            mutators.scale_by(2.0, cx);
            assert_eq!(*mutators.read_goal(cx), 20.0);
            assert_eq!(*mutators.sample(cx).value.as_ref().unwrap(), 12.0);

            mutators.jump_to(7.0, cx);
            assert_eq!(*mutators.read_goal(cx), 7.0);
            assert_eq!(*mutators.sample(cx).value.as_ref().unwrap(), 7.0);

            mutators.reset(cx);
            assert_eq!(*mutators.read_goal(cx), 2.0);
            assert!(mutators.read_cache().is_none());

            let alternating = create_transition(
                cx,
                Motion::new(Duration::ZERO).iterations(2).alternate(),
                0.0_f32,
            );
            assert!(alternating.update(cx, |value, _| *value = 100.0));
            let sample = alternating.sample(cx);
            assert_eq!(sample.value, Some(0.0));
            assert_eq!(sample.progress, Progress::START);
            assert!(!sample.is_active);
            drop(sample);
            assert_eq!(alternating.evaluate_delta(cx), 0.0);
        });
    }

    #[gpui::test]
    fn evaluate_uses_samples_and_requests_frames_while_active(cx: &mut TestAppContext) {
        let transition = cx.update(|cx| {
            create_transition(
                cx,
                Motion::new(Duration::from_secs(1)).with_easing(|_| 0.5),
                0.0_f32,
            )
        });
        cx.update(|cx| {
            assert!(transition.update(cx, |value, _| *value = 100.0));
        });
        let rendered_values = Rc::new(RefCell::new(Vec::new()));
        let window = cx.open_window(size(px(100.0), px(100.0)), {
            let transition = transition.clone();
            let rendered_values = rendered_values.clone();
            move |_, _| TransitionTestView {
                transition,
                rendered_values,
            }
        });
        cx.run_until_parked();

        assert_eq!(*rendered_values.borrow(), vec![50.0]);
        cx.update(|cx| transition.jump_to(100.0, cx));
        assert_eq!(
            window
                .update(cx, |_, window, cx| window.simulate_next_frame(cx))
                .unwrap(),
            1
        );
        cx.run_until_parked();

        assert_eq!(*rendered_values.borrow(), vec![50.0, 100.0]);
        assert_eq!(
            window
                .update(cx, |_, window, cx| window.simulate_next_frame(cx))
                .unwrap(),
            0
        );
    }
}
