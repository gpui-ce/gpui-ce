use std::{ops::RangeInclusive, time::Duration};

use crate::{Hsla, Motion, Pixels, Rems, Rgba, SpringDescription};

const CRITICAL_DAMPING_TOLERANCE: f32 = 1e-4;
pub(crate) const DEFAULT_SPRING_EPSILON: f32 = 0.001;

/// The physical parameters of a damped spring that moves a scalar toward a target.
///
/// Stiffness pulls the value toward its target, damping removes energy, and mass resists both.
/// Keep one set of parameters for every step of an animation. `stiffness` and `mass` must be
/// finite and positive; `damping` must be finite and non-negative. The constructor does not
/// validate them, so invalid values can produce NaN or meaningless motion.
///
/// Create one with [`spring`]. Pass it to [`SpringAnimation::new`] to animate an element, or
/// step it yourself with [`SpringConfig::step`].
///
/// # Examples
///
/// ```
/// use gpui::spring;
///
/// let config = spring(100., 10., 1.);
/// let (natural_frequency, damping_ratio) = config.canonical();
/// assert_eq!(natural_frequency, 10.0);
/// assert_eq!(damping_ratio, 0.5);
/// ```
///
/// See the [animation guide](crate::_documentation::guides::animation#use-springs-for-interactive-values)
/// for how spring state is retained between frames.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpringConfig {
    /// The spring constant, written $k$. A larger value pulls the spring toward its target more
    /// strongly.
    pub stiffness: f32,
    /// The viscous damping coefficient, written $c$. With zero damping, a spring set in motion
    /// keeps oscillating instead of losing energy.
    pub damping: f32,
    /// The moving mass, written $m$. A larger mass responds more slowly.
    pub mass: f32,
}

/// Creates a [`SpringConfig`] from stiffness, damping and mass.
///
/// This is the same as [`SpringConfig::new`]. Use the short name where the spring reads well
/// next to its animation.
pub const fn spring(stiffness: f32, damping: f32, mass: f32) -> SpringConfig {
    SpringConfig::new(stiffness, damping, mass)
}

impl SpringConfig {
    /// Creates a spring from its stiffness, damping and mass. See [`spring`].
    pub const fn new(stiffness: f32, damping: f32, mass: f32) -> Self {
        Self {
            stiffness,
            damping,
            mass,
        }
    }

    /// Returns the natural angular frequency $\omega_0$ and the damping ratio $\zeta$.
    ///
    /// A ratio below one describes an underdamped spring, which oscillates about its target.
    /// One is critical damping; above one is overdamping. For a displaced spring released from
    /// rest, critical damping gives the fastest approach to a fixed target without oscillation.
    /// A spring that already has velocity can cross the target even with critical damping.
    pub fn canonical(&self) -> (f32, f32) {
        let natural_frequency = (self.stiffness / self.mass).sqrt();
        let damping_ratio = self.damping / (2.0 * (self.stiffness * self.mass).sqrt());
        (natural_frequency, damping_ratio)
    }

    /// Advances the spring by `delta_time` seconds toward a fixed `target`.
    ///
    /// The solver assumes `target` stays fixed during `delta_time` and computes motion from
    /// elapsed time rather than a fixed frame duration. Splitting an interval into smaller steps
    /// gives the same motion apart from numerical error. Velocity carries over, so retargeting
    /// an interrupted spring does not restart it from rest.
    ///
    /// # Examples
    ///
    /// ```
    /// use gpui::{SpringState, spring};
    ///
    /// let config = spring(320., 24., 1.);
    /// let start = SpringState::default();
    /// let one_step = config.step(start, 1.0, 1.0 / 60.0);
    /// let two_steps = config.step(config.step(start, 1.0, 1.0 / 120.0), 1.0, 1.0 / 120.0);
    /// assert!((one_step.position - two_steps.position).abs() < 1e-3);
    /// assert!(one_step.position > 0.0 && one_step.position < 1.0);
    /// ```
    pub fn step(&self, state: SpringState, target: f32, delta_time: f32) -> SpringState {
        let propagator = self.propagator(delta_time);
        let displacement = state.position - target;

        SpringState {
            position: target + propagator[0][0] * displacement + propagator[0][1] * state.velocity,
            velocity: propagator[1][0] * displacement + propagator[1][1] * state.velocity,
        }
    }

    /// Advances the spring by `delta_time` seconds toward a target that moves at `target_velocity`.
    ///
    /// Use this for a target that moves steadily, such as a dragged pointer. With positive
    /// damping, the transient motion dies away and the spring approaches a constant lag behind
    /// the moving target. Using [`step`](Self::step) would instead treat the target as fixed
    /// within each frame, introducing a lag that depends on frame duration.
    ///
    /// # Examples
    ///
    /// ```
    /// use gpui::{SpringState, spring};
    ///
    /// // A natural frequency of 10 and a damping ratio of 0.8.
    /// let config = spring(100., 16., 1.);
    /// let state = SpringState { position: -0.48, velocity: 3.0 };
    /// let next = config.step_ramp(state, 0.0, 3.0, 0.25);
    /// assert!((next.position - 0.27).abs() < 1e-4);
    /// assert!((next.velocity - 3.0).abs() < 1e-4);
    /// ```
    pub fn step_ramp(
        &self,
        state: SpringState,
        target: f32,
        target_velocity: f32,
        delta_time: f32,
    ) -> SpringState {
        let (natural_frequency, damping_ratio) = self.canonical();
        let steady_state_lag = -2.0 * damping_ratio * target_velocity / natural_frequency;
        let displacement = state.position - target - steady_state_lag;
        let velocity = state.velocity - target_velocity;
        let propagator = self.propagator(delta_time);
        let target = target + target_velocity * delta_time;

        SpringState {
            position: target
                + steady_state_lag
                + propagator[0][0] * displacement
                + propagator[0][1] * velocity,
            velocity: target_velocity
                + propagator[1][0] * displacement
                + propagator[1][1] * velocity,
        }
    }

    /// Returns the exact state-transition matrix for a fixed target over `delta_time` seconds.
    ///
    /// The matrix maps the displacement from the target and the velocity to their values after
    /// `delta_time`, which is what [`step`](Self::step) does. Build it once when many springs share
    /// this config and frame delta. Rebuild it when the delta changes.
    ///
    /// # Examples
    ///
    /// ```
    /// use gpui::spring;
    ///
    /// // With no time elapsed, nothing changes.
    /// let matrix = spring(100., 10., 1.).propagator(0.0);
    /// assert_eq!(matrix, [[1.0, 0.0], [0.0, 1.0]]);
    /// ```
    pub fn propagator(&self, delta_time: f32) -> [[f32; 2]; 2] {
        let (natural_frequency, damping_ratio) = self.canonical();

        if damping_ratio < 1.0 - CRITICAL_DAMPING_TOLERANCE {
            let decay = damping_ratio * natural_frequency;
            let damped_frequency = natural_frequency * (1.0 - damping_ratio * damping_ratio).sqrt();
            let exponential = (-decay * delta_time).exp();
            let (sine, cosine) = (damped_frequency * delta_time).sin_cos();
            let sine_over_frequency = sine / damped_frequency;

            [
                [
                    exponential * (cosine + decay * sine_over_frequency),
                    exponential * sine_over_frequency,
                ],
                [
                    -exponential * natural_frequency * natural_frequency * sine_over_frequency,
                    exponential * (cosine - decay * sine_over_frequency),
                ],
            ]
        } else if damping_ratio > 1.0 + CRITICAL_DAMPING_TOLERANCE {
            let root = (damping_ratio * damping_ratio - 1.0).sqrt();
            let root_sum = damping_ratio + root;
            let slow_root = -natural_frequency / root_sum;
            let fast_root = -natural_frequency * root_sum;
            let denominator = slow_root - fast_root;
            let slow_exponential = (slow_root * delta_time).exp();
            let fast_exponential = (fast_root * delta_time).exp();

            [
                [
                    (-fast_root * slow_exponential + slow_root * fast_exponential) / denominator,
                    (slow_exponential - fast_exponential) / denominator,
                ],
                [
                    slow_root * fast_root * (fast_exponential - slow_exponential) / denominator,
                    (slow_root * slow_exponential - fast_root * fast_exponential) / denominator,
                ],
            ]
        } else {
            let exponential = (-natural_frequency * delta_time).exp();

            [
                [
                    exponential * (1.0 + natural_frequency * delta_time),
                    exponential * delta_time,
                ],
                [
                    -exponential * natural_frequency * natural_frequency * delta_time,
                    exponential * (1.0 - natural_frequency * delta_time),
                ],
            ]
        }
    }

    /// Returns whether the spring is within `epsilon` of `target`, in position and velocity.
    ///
    /// The position must be within `epsilon` of `target`. The velocity must be at most
    /// `epsilon * natural_frequency`, which puts it on the same scale as position. A negative or
    /// non-finite `epsilon` never settles.
    ///
    /// # Examples
    ///
    /// ```
    /// use gpui::{SpringState, spring};
    ///
    /// let config = spring(100., 10., 1.);
    /// let moving = SpringState { position: 1.0, velocity: 1.0 };
    /// let nearly_still = SpringState { position: 1.005, velocity: 0.05 };
    /// assert!(!config.is_settled(moving, 1.0, 0.01));
    /// assert!(config.is_settled(nearly_still, 1.0, 0.01));
    /// ```
    pub fn is_settled(&self, state: SpringState, target: f32, epsilon: f32) -> bool {
        let (natural_frequency, _) = self.canonical();
        epsilon.is_finite()
            && epsilon >= 0.0
            && (state.position - target).abs() <= epsilon
            && state.velocity.abs() <= epsilon * natural_frequency
    }

    /// Returns a time after which the spring stays settled, measured from `state` in seconds.
    ///
    /// The result is conservative. Stepping the spring to this time, or any later time, leaves it
    /// settled by [`is_settled`](Self::is_settled) with the same `target` and `epsilon`. An
    /// undamped spring that isn't already at rest never settles, so this returns
    /// [`Duration::MAX`]. So does a non-positive or non-finite `epsilon`, unless the spring is
    /// already at rest.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::time::Duration;
    ///
    /// use gpui::{SpringState, spring};
    ///
    /// let config = spring(100., 10., 1.);
    /// let start = SpringState::default();
    /// let time = config.settle_time(start, 1.0, 0.001);
    /// assert!(time > Duration::ZERO && time < Duration::MAX);
    /// let later = config.step(start, 1.0, time.as_secs_f32() + 0.1);
    /// assert!(config.is_settled(later, 1.0, 0.001));
    ///
    /// assert_eq!(spring(100., 0., 1.).settle_time(start, 1.0, 0.001), Duration::MAX);
    /// ```
    pub fn settle_time(&self, state: SpringState, target: f32, epsilon: f32) -> Duration {
        let displacement = state.position - target;
        if displacement == 0.0 && state.velocity == 0.0 {
            return Duration::ZERO;
        }

        let (natural_frequency, damping_ratio) = self.canonical();
        if !natural_frequency.is_finite()
            || natural_frequency <= 0.0
            || !damping_ratio.is_finite()
            || damping_ratio <= 0.0
            || !epsilon.is_finite()
            || epsilon <= 0.0
        {
            return Duration::MAX;
        }

        let velocity_threshold = epsilon * natural_frequency;

        if damping_ratio < 1.0 - CRITICAL_DAMPING_TOLERANCE {
            let decay = damping_ratio * natural_frequency;
            let damped_frequency = natural_frequency * (1.0 - damping_ratio * damping_ratio).sqrt();
            let sine_coefficient = (state.velocity + decay * displacement) / damped_frequency;
            let position_envelope = displacement.hypot(sine_coefficient);
            let velocity_cosine = damped_frequency * sine_coefficient - decay * displacement;
            let velocity_sine = -damped_frequency * displacement - decay * sine_coefficient;
            let velocity_envelope = velocity_cosine.hypot(velocity_sine);

            find_settle_time(
                epsilon,
                velocity_threshold,
                0.0,
                natural_frequency,
                move |time| {
                    let exponential = (-decay * time).exp();
                    (
                        position_envelope * exponential,
                        velocity_envelope * exponential,
                    )
                },
            )
        } else if damping_ratio > 1.0 + CRITICAL_DAMPING_TOLERANCE {
            let root = (damping_ratio * damping_ratio - 1.0).sqrt();
            let root_sum = damping_ratio + root;
            let slow_root = -natural_frequency / root_sum;
            let fast_root = -natural_frequency * root_sum;
            let denominator = slow_root - fast_root;
            let slow_coefficient = (state.velocity - fast_root * displacement) / denominator;
            let fast_coefficient = (slow_root * displacement - state.velocity) / denominator;

            find_settle_time(
                epsilon,
                velocity_threshold,
                0.0,
                natural_frequency,
                move |time| {
                    let slow_term = slow_coefficient.abs() * (slow_root * time).exp();
                    let fast_term = fast_coefficient.abs() * (fast_root * time).exp();
                    (
                        slow_term + fast_term,
                        slow_root.abs() * slow_term + fast_root.abs() * fast_term,
                    )
                },
            )
        } else {
            let linear_coefficient = state.velocity + natural_frequency * displacement;
            let position_constant = displacement.abs();
            let position_linear = linear_coefficient.abs();
            let velocity_constant = (linear_coefficient - natural_frequency * displacement).abs();
            let velocity_linear = natural_frequency * linear_coefficient.abs();
            let position_decay_start =
                envelope_decay_start(position_constant, position_linear, natural_frequency);
            let velocity_decay_start =
                envelope_decay_start(velocity_constant, velocity_linear, natural_frequency);

            find_settle_time(
                epsilon,
                velocity_threshold,
                position_decay_start.max(velocity_decay_start),
                natural_frequency,
                move |time| {
                    let exponential = (-natural_frequency * time).exp();
                    (
                        (position_constant + position_linear * time) * exponential,
                        (velocity_constant + velocity_linear * time) * exponential,
                    )
                },
            )
        }
    }
}

/// The position and velocity of a spring at one instant.
///
/// Pass the state from one step to the next with [`SpringConfig::step`]. The default represents a
/// spring at rest at zero.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SpringState {
    /// The current value, in the units of the animated value.
    pub position: f32,
    /// The change in position, per second.
    pub velocity: f32,
}

/// A value that a one-dimensional spring can target.
///
/// The spring solver works on one `f32` coordinate. A target maps its value into that coordinate
/// and maps the sampled coordinate back into the output consumed by the animator. Built-in targets
/// are [`f32`], [`Pixels`](crate::Pixels), [`Rems`](crate::Rems), [`AnimationPhase`] and [`bool`]
/// (which maps to `0.0` or `1.0`).
///
/// # Examples
///
/// ```
/// use gpui::{SpringTarget, px};
///
/// assert_eq!(px(12.).target(), 12.0);
/// assert_eq!(px(12.).resolve(14.0), px(14.));
/// assert_eq!(true.target(), 1.0);
/// ```
pub trait SpringTarget: 'static {
    /// The value that the spring animator receives.
    type Output;

    /// Returns this target as a coordinate for the spring.
    fn target(&self) -> f32;

    /// Converts a spring coordinate into the output the animator receives.
    fn resolve(&self, value: f32) -> Self::Output;
}

impl SpringTarget for f32 {
    type Output = f32;

    fn target(&self) -> f32 {
        *self
    }

    fn resolve(&self, value: f32) -> Self::Output {
        value
    }
}

impl SpringTarget for Pixels {
    type Output = Pixels;

    fn target(&self) -> f32 {
        self.as_f32()
    }

    fn resolve(&self, value: f32) -> Self::Output {
        Pixels::from(value)
    }
}

impl SpringTarget for Rems {
    type Output = Rems;

    fn target(&self) -> f32 {
        self.0
    }

    fn resolve(&self, value: f32) -> Self::Output {
        Rems(value)
    }
}

impl SpringTarget for bool {
    type Output = AnimationPhase;

    fn target(&self) -> f32 {
        if *self { 1.0 } else { 0.0 }
    }

    fn resolve(&self, value: f32) -> Self::Output {
        AnimationPhase(value)
    }
}

/// An animation coordinate that can go past zero and one.
///
/// A phase is not limited to `0.0..=1.0`. A multi-stage animation can assign each stage its own
/// range, such as `1.0..=2.0`, and interpolate values across it. A [`bool`] target produces a
/// phase that settles at `0.0` or `1.0`.
///
/// # Examples
///
/// ```
/// use gpui::AnimationPhase;
///
/// let phase = AnimationPhase(1.5);
/// assert_eq!(phase.interpolate(10.0, 20.0), 25.0);
/// assert_eq!(phase.interpolate_clamped(10.0, 20.0), 20.0);
/// assert_eq!(phase.interpolate_between(1.0..=2.0, 10.0, 20.0), 15.0);
/// assert_eq!(phase.interpolate_between_clamped(2.0..=3.0, 10.0, 20.0), 10.0);
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, PartialOrd)]
pub struct AnimationPhase(
    /// The phase coordinate, which is not bounded.
    pub f32,
);

impl AnimationPhase {
    /// Limits this phase to `range`.
    ///
    /// The bounds may be given in either order.
    pub fn clamp(self, range: RangeInclusive<f32>) -> Self {
        let (first, second) = range.into_inner();
        Self(self.0.clamp(first.min(second), first.max(second)))
    }

    /// Interpolates from `from` at phase `0.0` to `to` at phase `1.0`.
    ///
    /// Phases outside `0.0..=1.0` extrapolate past the endpoints. Use
    /// [`interpolate_clamped`](Self::interpolate_clamped) to hold the endpoints instead.
    pub fn interpolate<T: Interpolate>(self, from: T, to: T) -> T {
        T::interpolate(from, to, self.0)
    }

    /// Interpolates from `from` to `to` like [`interpolate`](Self::interpolate), clamping at the
    /// endpoints.
    ///
    /// A phase below `0.0` gives `from`, and a phase above `1.0` gives `to`.
    pub fn interpolate_clamped<T: Interpolate>(self, from: T, to: T) -> T {
        T::interpolate(from, to, self.0.clamp(0.0, 1.0))
    }

    /// Interpolates across `range`, where its start maps to `from` and its end maps to `to`.
    ///
    /// Phases outside the range extrapolate. If the range is empty (`start == end`), the result
    /// is `from` below `start` and `to` at or above it.
    pub fn interpolate_between<T: Interpolate>(
        self,
        range: RangeInclusive<f32>,
        from: T,
        to: T,
    ) -> T {
        let (start, end) = range.into_inner();
        let phase = if start == end {
            if self.0 < start { 0.0 } else { 1.0 }
        } else {
            (self.0 - start) / (end - start)
        };
        T::interpolate(from, to, phase)
    }

    /// Interpolates across `range` like [`interpolate_between`](Self::interpolate_between),
    /// clamping at the endpoints.
    ///
    /// Phases outside the range give `from` or `to`. An empty range behaves as it does in
    /// [`interpolate_between`](Self::interpolate_between).
    pub fn interpolate_between_clamped<T: Interpolate>(
        self,
        range: RangeInclusive<f32>,
        from: T,
        to: T,
    ) -> T {
        let (start, end) = range.into_inner();
        let phase = if start == end {
            if self.0 < start { 0.0 } else { 1.0 }
        } else {
            ((self.0 - start) / (end - start)).clamp(0.0, 1.0)
        };
        T::interpolate(from, to, phase)
    }
}

impl From<f32> for AnimationPhase {
    fn from(value: f32) -> Self {
        Self(value)
    }
}

impl From<bool> for AnimationPhase {
    fn from(value: bool) -> Self {
        Self(if value { 1.0 } else { 0.0 })
    }
}

impl SpringTarget for AnimationPhase {
    type Output = AnimationPhase;

    fn target(&self) -> f32 {
        self.0
    }

    fn resolve(&self, value: f32) -> Self::Output {
        Self(value)
    }
}

/// A value that [`AnimationPhase`] can interpolate.
///
/// Implemented for [`f32`], [`Pixels`](crate::Pixels), [`Rems`](crate::Rems), [`Rgba`](crate::Rgba)
/// and [`Hsla`](crate::Hsla). RGB colors interpolate channel by channel; [`Hsla`](crate::Hsla)
/// takes the shorter path around the hue wheel.
///
/// # Examples
///
/// ```
/// use gpui::{Interpolate, Pixels, px};
///
/// assert_eq!(f32::interpolate(10.0, 20.0, 0.25), 12.5);
/// assert_eq!(Pixels::interpolate(px(0.), px(8.), 0.5), px(4.));
/// ```
pub trait Interpolate: Sized {
    /// Returns the value at `phase`, where `0.0` is `from` and `1.0` is `to`.
    ///
    /// Phases outside `0.0..=1.0` extrapolate.
    fn interpolate(from: Self, to: Self, phase: f32) -> Self;
}

impl Interpolate for f32 {
    fn interpolate(from: Self, to: Self, phase: f32) -> Self {
        from + (to - from) * phase
    }
}

impl Interpolate for Pixels {
    fn interpolate(from: Self, to: Self, phase: f32) -> Self {
        from + (to - from) * phase
    }
}

impl Interpolate for Rems {
    fn interpolate(from: Self, to: Self, phase: f32) -> Self {
        from + (to - from) * phase
    }
}

impl Interpolate for Rgba {
    fn interpolate(from: Self, to: Self, phase: f32) -> Self {
        Self::new(
            f32::interpolate(from.color.red, to.color.red, phase),
            f32::interpolate(from.color.green, to.color.green, phase),
            f32::interpolate(from.color.blue, to.color.blue, phase),
            f32::interpolate(from.alpha, to.alpha, phase),
        )
    }
}

impl Interpolate for Hsla {
    fn interpolate(from: Self, to: Self, phase: f32) -> Self {
        use palette::RgbHue;
        let from_h = from.color.hue.into_degrees() / 360.0;
        let to_h = to.color.hue.into_degrees() / 360.0;
        let hue_delta = (to_h - from_h + 0.5).rem_euclid(1.0) - 0.5;
        Self::new(
            RgbHue::from_degrees((from_h + hue_delta * phase).rem_euclid(1.0) * 360.0),
            f32::interpolate(from.color.saturation, to.color.saturation, phase),
            f32::interpolate(from.color.lightness, to.color.lightness, phase),
            f32::interpolate(from.alpha, to.alpha, phase),
        )
    }
}

/// How a spring advances, and where it rests, when its animation changes state.
///
/// Set it with [`SpringAnimation::playback`]. The default is [`SpringPlayback::Running`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SpringPlayback {
    /// The spring advances toward the latest target and keeps its velocity across retargets.
    #[default]
    Running,
    /// The spring holds its position and velocity. It resumes from them when playback runs again.
    Paused,
    /// The spring holds its position and sets its velocity to zero.
    Stopped,
    /// The spring jumps to the latest target and sets its velocity to zero.
    Completed,
    /// The spring returns to its initial value and sets its velocity to zero.
    Cancelled,
}

/// A spring animation for an element: a [`SpringConfig`], a target and a starting value.
///
/// Build one with [`SpringAnimation::new`] and [`SpringAnimation::to`], then pass it to
/// [`AnimationExt::with_spring`](crate::AnimationExt::with_spring). The element keeps the spring's
/// position and velocity across frames, so a retargeted spring bends toward the new target
/// instead of jumping. Under [`App::reduce_motion`](crate::App::reduce_motion), the spring jumps
/// straight to its target.
///
/// # Examples
///
/// ```
/// use std::{cell::Cell, rc::Rc};
///
/// use gpui::{
///     AnimationExt, Context, IntoElement, Render, SpringAnimation, TestApp, Window, div,
///     prelude::*, spring,
/// };
///
/// struct Knob {
///     last: Rc<Cell<f32>>,
/// }
///
/// impl Render for Knob {
///     fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
///         let last = self.last.clone();
///         div().child("knob").with_spring(
///             "knob",
///             SpringAnimation::new(spring(320., 24., 1.)).to(1.0_f32),
///             move |el, position| {
///                 last.set(position);
///                 el.opacity(position)
///             },
///         )
///     }
/// }
///
/// let last = Rc::new(Cell::new(0.0));
/// let mut app = TestApp::new();
/// app.update(|cx| cx.set_reduce_motion(true));
/// let mut window = app.open_window(|_, _| Knob { last: last.clone() });
/// window.draw();
/// assert_eq!(last.get(), 1.0);
/// ```
#[derive(Clone, Debug)]
pub struct SpringAnimation<T = ()> {
    pub(crate) motion: Motion<SpringDescription>,
    pub(crate) target: T,
    pub(crate) initial: Option<f32>,
    pub(crate) playback: SpringPlayback,
}

impl SpringAnimation<()> {
    /// Creates a spring animation with `config`, which has no target yet.
    ///
    /// Call [`to`](Self::to) to set the target before passing the animation to an element.
    pub fn new(config: SpringConfig) -> Self {
        Self {
            motion: Motion::spring(config),
            target: (),
            initial: None,
            playback: SpringPlayback::Running,
        }
    }

    /// Sets the target of this spring.
    ///
    /// An `f32` targets a number, a [`Pixels`](crate::Pixels) or [`Rems`](crate::Rems) targets a
    /// length, a `bool` targets `0.0` or `1.0`, and an [`AnimationPhase`] targets a phase.
    /// The animator receives the target's [`SpringTarget::Output`].
    pub fn to<T: SpringTarget>(self, target: T) -> SpringAnimation<T> {
        let SpringAnimation {
            motion,
            target: (),
            initial,
            playback,
        } = self;
        SpringAnimation {
            motion,
            target,
            initial,
            playback,
        }
    }
}

impl<T> SpringAnimation<T> {
    /// Sets how close the spring must be to its target before it counts as settled.
    ///
    /// The tolerance is in the target's scalar coordinate. For a [`Pixels`](crate::Pixels) target,
    /// it is in pixels. See [`Motion::with_epsilon`](crate::Motion::with_epsilon).
    pub fn with_epsilon(mut self, epsilon: f32) -> Self {
        self.motion = self.motion.with_epsilon(epsilon);
        self
    }

    /// Sets how the spring advances, and where it rests, when its animation changes state.
    ///
    /// See [`SpringPlayback`].
    pub fn playback(mut self, playback: SpringPlayback) -> Self {
        self.playback = playback;
        self
    }
}

impl<T: SpringTarget> SpringAnimation<T> {
    /// Sets the value a newly mounted spring starts from.
    ///
    /// Without this, a new spring starts at its target and doesn't animate on mount. The value is
    /// converted with [`SpringTarget::target`], so it must be the same kind of value as the target.
    pub fn from(mut self, initial: T) -> Self {
        self.initial = Some(initial.target());
        self
    }
}

/// Returns the time a spring takes to settle from rest, and an easing that samples it over that time.
///
/// The spring starts at rest at zero and moves to `1.0`. The easing maps progress `0.0..=1.0`
/// across the settling time, so it fits APIs that take a duration and an easing, such as
/// [`Animation::with_easing`](crate::Animation::with_easing). The easing can overshoot `1.0`.
/// Retargeting a duration-based animation restarts it. Use [`SpringConfig::step`] when keeping
/// velocity matters.
///
/// # Examples
///
/// ```
/// use std::time::Duration;
///
/// use gpui::{sampled_easing, spring};
///
/// let (duration, ease) = sampled_easing(spring(100., 6., 1.), 0.001);
/// assert!(duration > Duration::ZERO);
/// assert_eq!(ease(0.0), 0.0);
/// assert_eq!(ease(1.0), 1.0);
/// assert!((1..100).any(|step| ease(step as f32 / 100.0) > 1.0));
/// ```
pub fn sampled_easing(config: SpringConfig, epsilon: f32) -> (Duration, impl Fn(f32) -> f32) {
    let initial_state = SpringState {
        position: 0.0,
        velocity: 0.0,
    };
    let duration = config.settle_time(initial_state, 1.0, epsilon);
    let duration_seconds = duration.as_secs_f32();

    (duration, move |progress| {
        if progress <= 0.0 {
            0.0
        } else if progress >= 1.0 {
            1.0
        } else {
            config
                .step(initial_state, 1.0, progress * duration_seconds)
                .position
        }
    })
}

fn envelope_decay_start(constant: f32, linear: f32, decay: f32) -> f32 {
    if linear == 0.0 {
        0.0
    } else {
        (1.0 / decay - constant / linear).max(0.0)
    }
}

fn find_settle_time(
    position_threshold: f32,
    velocity_threshold: f32,
    decay_start: f32,
    natural_frequency: f32,
    envelope: impl Fn(f32) -> (f32, f32),
) -> Duration {
    let is_below_threshold = |time| {
        let (position, velocity) = envelope(time);
        position <= position_threshold && velocity <= velocity_threshold
    };

    if is_below_threshold(decay_start) {
        return duration_from_secs(decay_start);
    }

    let mut lower_bound = decay_start;
    let mut upper_bound = decay_start.max(natural_frequency.recip());
    while !is_below_threshold(upper_bound) {
        lower_bound = upper_bound;
        upper_bound *= 2.0;
        if !upper_bound.is_finite() {
            return Duration::MAX;
        }
    }

    for _ in 0..32 {
        let midpoint = (lower_bound + upper_bound) / 2.0;
        if is_below_threshold(midpoint) {
            upper_bound = midpoint;
        } else {
            lower_bound = midpoint;
        }
    }

    duration_from_secs(upper_bound)
}

fn duration_from_secs(seconds: f32) -> Duration {
    if !seconds.is_finite() || seconds >= Duration::MAX.as_secs_f32() {
        Duration::MAX
    } else if seconds <= 0.0 {
        Duration::ZERO
    } else {
        Duration::from_secs_f32(seconds)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPSILON: f32 = 1e-4;

    #[test]
    fn spring_targets_resolve_typed_outputs() {
        assert_eq!(12.0_f32.target(), 12.0);
        assert_eq!(12.0_f32.resolve(14.0), 14.0);
        assert_eq!(Pixels::from(12.0).target(), 12.0);
        assert_eq!(Pixels::from(12.0).resolve(14.0), Pixels::from(14.0));
        assert_eq!(false.target(), 0.0);
        assert_eq!(true.target(), 1.0);
        assert_eq!(true.resolve(1.25), AnimationPhase(1.25));
    }

    #[test]
    fn animation_phases_interpolate_over_arbitrary_ranges() {
        let phase = AnimationPhase(1.5);
        assert_eq!(phase.interpolate_between(1.0..=2.0, 10.0, 20.0), 15.0);
        assert_eq!(
            phase.interpolate_between_clamped(2.0..=3.0, 10.0, 20.0),
            10.0
        );
        assert_eq!(
            AnimationPhase(3.5).interpolate_between(2.0..=3.0, 10.0, 20.0),
            25.0
        );
    }

    #[test]
    fn hsla_interpolation_takes_the_shortest_hue_path() {
        let from = crate::hsla(0.9, 0.5, 0.5, 1.0);
        let to = crate::hsla(0.1, 1.0, 0.75, 0.5);
        let result = AnimationPhase(0.5).interpolate(from, to);

        let h = result.color.hue.into_degrees() / 360.0;
        assert!(h < EPSILON || (1.0 - h) < EPSILON);
        assert!((result.color.saturation - 0.75).abs() < EPSILON);
        assert!((result.color.lightness - 0.625).abs() < EPSILON);
        assert!((result.alpha - 0.75).abs() < EPSILON);
    }

    #[test]
    fn propagators_compose_and_have_expected_determinant() {
        for damping_ratio in [0.4, 1.0, 1.5] {
            let natural_frequency = 12.0;
            let config = SpringConfig::new(
                natural_frequency * natural_frequency,
                2.0 * damping_ratio * natural_frequency,
                1.0,
            );
            let first = config.propagator(0.013);
            let second = config.propagator(0.021);
            let combined = multiply(second, first);
            let direct = config.propagator(0.034);

            for row in 0..2 {
                for column in 0..2 {
                    assert!(
                        (combined[row][column] - direct[row][column]).abs() < 2e-4,
                        "{damping_ratio}: {combined:?} != {direct:?}"
                    );
                }
            }

            let determinant = direct[0][0] * direct[1][1] - direct[0][1] * direct[1][0];
            let expected = (-2.0 * damping_ratio * natural_frequency * 0.034).exp();
            assert!((determinant - expected).abs() < 2e-4);
        }
    }

    #[test]
    fn step_preserves_semigroup_for_every_damping_regime() {
        let state = SpringState {
            position: -3.0,
            velocity: 5.0,
        };
        for damping in [4.0, 20.0, 40.0] {
            let config = SpringConfig::new(100.0, damping, 1.0);
            let stepped = config.step(config.step(state, 7.0, 0.013), 7.0, 0.021);
            let direct = config.step(state, 7.0, 0.034);

            assert!((stepped.position - direct.position).abs() < 2e-4);
            assert!((stepped.velocity - direct.velocity).abs() < 2e-4);
        }
    }

    #[test]
    fn ramp_tracks_steady_state_lag() {
        let natural_frequency = 10.0;
        let damping_ratio = 0.8;
        let target_velocity = 3.0;
        let config = SpringConfig::new(
            natural_frequency * natural_frequency,
            2.0 * damping_ratio * natural_frequency,
            1.0,
        );
        let lag = -2.0 * damping_ratio * target_velocity / natural_frequency;
        let state = SpringState {
            position: lag,
            velocity: target_velocity,
        };
        let next = config.step_ramp(state, 0.0, target_velocity, 0.25);

        assert!((next.position - (target_velocity * 0.25 + lag)).abs() < EPSILON);
        assert!((next.velocity - target_velocity).abs() < EPSILON);
    }

    #[test]
    fn settling_requires_low_velocity() {
        let config = SpringConfig::new(100.0, 10.0, 1.0);
        assert!(!config.is_settled(
            SpringState {
                position: 1.0,
                velocity: 1.0,
            },
            1.0,
            0.01,
        ));
        assert!(config.is_settled(
            SpringState {
                position: 1.005,
                velocity: 0.05,
            },
            1.0,
            0.01,
        ));
    }

    #[test]
    fn settle_time_is_conservative_for_every_damping_regime() {
        let initial_state = SpringState {
            position: -2.0,
            velocity: 4.0,
        };
        for damping in [4.0, 20.0, 40.0] {
            let config = SpringConfig::new(100.0, damping, 1.0);
            let duration = config.settle_time(initial_state, 3.0, 0.001);
            assert_ne!(duration, Duration::MAX);

            for additional_time in [0.0, 0.1, 1.0] {
                let state =
                    config.step(initial_state, 3.0, duration.as_secs_f32() + additional_time);
                assert!(
                    config.is_settled(state, 3.0, 0.001),
                    "{damping}: {duration:?} produced {state:?}"
                );
            }
        }
    }

    #[test]
    fn settle_time_accounts_for_motion_outside_an_instantaneous_tolerance() {
        let config = SpringConfig::new(100.0, 2.0, 1.0);
        let state = SpringState {
            position: 1.125,
            velocity: 1.25,
        };
        assert!(config.is_settled(state, 1.0, 0.125));

        let duration = config.settle_time(state, 1.0, 0.125);
        assert!(duration > Duration::ZERO);
        assert!(config.is_settled(config.step(state, 1.0, duration.as_secs_f32()), 1.0, 0.125,));
    }

    #[test]
    fn undamped_spring_never_settles() {
        let config = SpringConfig::new(100.0, 0.0, 1.0);
        assert_eq!(
            config.settle_time(
                SpringState {
                    position: 0.0,
                    velocity: 0.0,
                },
                1.0,
                0.001,
            ),
            Duration::MAX
        );
    }

    #[test]
    fn sampled_easing_has_exact_endpoints_and_can_overshoot() {
        let config = SpringConfig::new(100.0, 6.0, 1.0);
        let (duration, easing) = sampled_easing(config, 0.001);

        assert_ne!(duration, Duration::MAX);
        assert_eq!(easing(0.0), 0.0);
        assert_eq!(easing(1.0), 1.0);
        assert!((1..100).any(|step| easing(step as f32 / 100.0) > 1.0));
    }

    fn multiply(left: [[f32; 2]; 2], right: [[f32; 2]; 2]) -> [[f32; 2]; 2] {
        [
            [
                left[0][0] * right[0][0] + left[0][1] * right[1][0],
                left[0][0] * right[0][1] + left[0][1] * right[1][1],
            ],
            [
                left[1][0] * right[0][0] + left[1][1] * right[1][0],
                left[1][0] * right[0][1] + left[1][1] * right[1][1],
            ],
        ]
    }
}
