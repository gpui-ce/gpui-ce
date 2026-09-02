use scheduler::Instant;

use crate::{
    AnyElement, App, Element, ElementId, GlobalElementId, InspectorElementId, IntoElement, Motion,
    ParentElement, Repeat, Window,
};

pub use easing::*;
use smallvec::SmallVec;

/// An animation that can be applied to an element.
#[derive(Clone)]
pub struct Animation {
    /// The timing and easing applied to this animation.
    pub motion: Motion,
}

impl Animation {
    /// Create a new animation with the given motion.
    /// The animation runs once unless [`Self::repeat`] is called. Duration inputs
    /// use linear easing.
    pub fn new(motion: impl Into<Motion>) -> Self {
        Self {
            motion: motion.into(),
        }
    }

    /// Set the animation to loop when it finishes.
    pub fn repeat(mut self) -> Self {
        self.motion.repeat = Repeat::Forever;
        self
    }

    /// Set the easing function to use for this animation.
    /// The easing function will take a time delta between 0 and 1 and return a new delta
    /// between 0 and 1
    pub fn with_easing(mut self, easing: impl Fn(f32) -> f32 + 'static) -> Self {
        self.motion = self.motion.with_easing(easing);
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
        AnimationElement {
            id: id.into(),
            element: Some(self),
            animator: Box::new(move |this, _, value| animator(this, value)),
            animations: smallvec::smallvec![animation],
        }
    }

    /// Render this component or element with a chain of animations
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
}

impl<E: IntoElement + 'static> AnimationExt for E {}

/// A GPUI element that applies an animation to another element
pub struct AnimationElement<E> {
    id: ElementId,
    element: Option<E>,
    animations: SmallVec<[Animation; 1]>,
    animator: Box<dyn Fn(E, usize, f32) -> E + 'static>,
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
            let mut state = state.unwrap_or_else(|| AnimationState {
                start: Instant::now(),
                animation_ix: 0,
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
                let now = Instant::now();

                let sample = self.animations[animation_ix]
                    .motion
                    .sample_at(state.start, now);
                let mut done = !sample.is_active;
                if done && animation_ix < self.animations.len() - 1 {
                    state.start = now;
                    state.animation_ix += 1;
                    done = false;
                }
                (animation_ix, sample.progress.get(), done)
            };

            let element = self.element.take().expect("should only be called once");
            let mut element = (self.animator)(element, animation_ix, delta).into_any_element();

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
mod tests {
    use std::{cell::RefCell, rc::Rc, time::Duration};

    use crate::{
        Animation, Context, Render, TestAppContext, WindowHandle, div, prelude::*, px, size,
    };

    use super::*;

    struct AnimationTestView {
        rendered_deltas: Rc<RefCell<Vec<f32>>>,
    }

    struct AnimationSequenceTestView {
        rendered_samples: Rc<RefCell<Vec<(usize, f32)>>>,
    }

    impl Render for AnimationTestView {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            let rendered_deltas = self.rendered_deltas.clone();
            div().size_full().child(
                div()
                    .with_animation(
                        "repeating-animation",
                        Animation::new(Duration::from_secs(1))
                            .repeat()
                            .with_easing(|progress| progress),
                        move |this, delta| {
                            rendered_deltas.borrow_mut().push(delta);
                            this
                        },
                    )
                    .map_element(|element| element.child(div()))
                    .child(div()),
            )
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
        let rendered_deltas = Rc::new(RefCell::new(Vec::new()));
        let window = cx.open_window(size(px(100.), px(100.)), {
            let rendered_deltas = rendered_deltas.clone();
            move |_, _| AnimationTestView { rendered_deltas }
        });
        cx.run_until_parked();
        (rendered_deltas, window)
    }

    fn simulate_next_frame(
        window: &WindowHandle<AnimationTestView>,
        cx: &mut TestAppContext,
    ) -> usize {
        let callback_count = window
            .update(cx, |_, window, cx| window.simulate_next_frame(cx))
            .unwrap();
        cx.run_until_parked();
        callback_count
    }
    #[gpui::test]
    fn animation_element_exercises_layout_state_paths(cx: &mut TestAppContext) {
        let (rendered_deltas, window) = open_test_window(cx);

        assert_eq!(rendered_deltas.borrow().len(), 1);

        for expected_frames in 2..=3 {
            assert_eq!(simulate_next_frame(&window, cx), 1);
            assert_eq!(rendered_deltas.borrow().len(), expected_frames);
        }

        let rendered_samples = Rc::new(RefCell::new(Vec::new()));
        let sequence_window = cx.open_window(size(px(100.), px(100.)), {
            let rendered_samples = rendered_samples.clone();
            move |_, _| AnimationSequenceTestView { rendered_samples }
        });
        cx.run_until_parked();

        assert_eq!(*rendered_samples.borrow(), vec![(0, 1.0)]);
        assert_eq!(
            sequence_window
                .update(cx, |_, window, cx| window.simulate_next_frame(cx))
                .unwrap(),
            1
        );
        cx.run_until_parked();
        assert_eq!(*rendered_samples.borrow(), vec![(0, 1.0), (1, 1.0)]);
        assert_eq!(
            sequence_window
                .update(cx, |_, window, cx| window.simulate_next_frame(cx))
                .unwrap(),
            0
        );

        cx.update(|cx| cx.set_reduce_motion(true));
        let (reduced_deltas, reduced_window) = open_test_window(cx);

        assert_eq!(*reduced_deltas.borrow(), vec![0.0]);

        assert_eq!(simulate_next_frame(&reduced_window, cx), 0);
        assert_eq!(*reduced_deltas.borrow(), vec![0.0]);

        let reduced_samples = Rc::new(RefCell::new(Vec::new()));
        let reduced_sequence_window = cx.open_window(size(px(100.), px(100.)), {
            let reduced_samples = reduced_samples.clone();
            move |_, _| AnimationSequenceTestView {
                rendered_samples: reduced_samples,
            }
        });
        cx.run_until_parked();

        assert_eq!(*reduced_samples.borrow(), vec![(1, 1.0)]);
        assert_eq!(
            reduced_sequence_window
                .update(cx, |_, window, cx| window.simulate_next_frame(cx))
                .unwrap(),
            0
        );
    }
}
