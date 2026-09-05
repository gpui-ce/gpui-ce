use std::{ops::Sub, rc::Rc, time::Duration};

/// Creates a duration from a number of whole seconds.
pub const fn secs(seconds: u64) -> Duration {
    Duration::from_secs(seconds)
}

/// Creates a duration from a number of whole milliseconds.
pub const fn millis(milliseconds: u64) -> Duration {
    Duration::from_millis(milliseconds)
}

/// Normalized animation progress.
#[derive(Clone, Copy, Debug, Default, PartialEq, PartialOrd)]
pub struct Progress(f32);

impl Progress {
    /// The beginning of an animation.
    pub const START: Self = Self(0.0);

    /// The end of an animation.
    pub const END: Self = Self(1.0);

    /// Returns progress clamped to the normalized range.
    pub fn clamped(value: f32) -> Self {
        assert!(!value.is_nan(), "progress must not be NaN");
        Self(value.clamp(Self::START.0, Self::END.0))
    }

    /// Returns the underlying normalized value.
    pub const fn get(self) -> f32 {
        self.0
    }

    /// Returns whether this progress has reached the end.
    pub const fn is_complete(self) -> bool {
        self.0 >= Self::END.0
    }

    fn contains(value: f32) -> bool {
        value >= Self::START.0 && value <= Self::END.0
    }
}

/// Creates motion from a duration and an easing function.
pub trait DurationWithEasing {
    /// Creates motion with this duration and the supplied easing function.
    fn with_easing(self, easing: impl Fn(f32) -> f32 + 'static) -> Motion;
}

impl DurationWithEasing for Duration {
    fn with_easing(self, easing: impl Fn(f32) -> f32 + 'static) -> Motion {
        Motion::new(self).with_easing(easing)
    }
}

/// Maps linear progress to eased progress.
#[derive(Clone)]
pub struct Easing {
    function: Rc<dyn Fn(f32) -> f32>,
    options: MotionOptions,
}

#[derive(Clone, Copy, Default)]
struct MotionOptions {
    delay: Duration,
    repeat_count: Option<u32>,
    auto_reverse: bool,
}

impl Easing {
    /// Creates an easing function.
    pub fn new(easing: impl Fn(f32) -> f32 + 'static) -> Self {
        Self {
            function: Rc::new(easing),
            options: MotionOptions::default(),
        }
    }

    /// Evaluates this easing function with normalized progress.
    pub fn sample(&self, progress: Progress) -> Progress {
        let eased = (self.function)(progress.get());

        debug_assert!(
            Progress::contains(eased),
            "easing must return a value between 0 and 1"
        );

        Progress::clamped(eased)
    }
}

impl Default for Easing {
    fn default() -> Self {
        Self::new(crate::linear)
    }
}

/// Controls whether a motion runs once or indefinitely.
///
/// Use [`Motion::with_repeat_count`] to configure a finite number of passes.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Repeat {
    /// Run once.
    #[default]
    Once,

    /// Repeat and remain active until the owner removes the animation.
    Forever,
}

/// The result of evaluating motion at a point in time.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MotionSample {
    /// Eased progress between zero and one.
    pub progress: Progress,

    /// Whether another sample may produce a different value.
    pub is_active: bool,
}

/// Configuration for one-shot or repeating motion.
#[derive(Clone)]
pub struct Motion {
    /// How long each pass takes, excluding the initial delay.
    pub duration: Duration,

    /// Maps linear progress to eased progress.
    pub easing: Easing,

    /// Whether the motion runs once or indefinitely.
    ///
    /// Use [`Motion::with_repeat_count`] to configure a finite number of passes.
    pub repeat: Repeat,
}

impl Motion {
    /// Creates one linear motion pass with the supplied duration.
    pub fn new(duration: Duration) -> Self {
        Self {
            duration,
            easing: Easing::default(),
            repeat: Repeat::Once,
        }
    }

    /// Replaces the linear easing function.
    pub fn with_easing(mut self, easing: impl Fn(f32) -> f32 + 'static) -> Self {
        let options = self.easing.options;
        self.easing = Easing {
            function: Rc::new(easing),
            options,
        };
        self
    }

    /// Holds the starting value before the first pass of each run.
    /// Retargeting an animated value starts a new run with this delay.
    pub fn with_delay(mut self, delay: Duration) -> Self {
        self.easing.options.delay = delay;
        self
    }

    /// Sets whether the motion runs once or indefinitely.
    pub fn with_repeat(mut self, repeat: Repeat) -> Self {
        self.repeat = repeat;
        self.easing.options.repeat_count = None;
        self
    }

    /// Sets the total number of passes, including the first one.
    ///
    /// A count of zero leaves the motion at its starting value and inactive.
    pub fn with_repeat_count(mut self, count: u32) -> Self {
        self.repeat = Repeat::Once;
        self.easing.options.repeat_count = Some(count);
        self
    }

    /// Reverses every other pass. Two passes make one out-and-back cycle.
    /// Before easing, finite motion ends at progress zero after an even number
    /// of alternating passes, and at progress one after an odd number.
    pub fn with_auto_reverse(mut self, auto_reverse: bool) -> Self {
        self.easing.options.auto_reverse = auto_reverse;
        self
    }

    /// Evaluates this motion after the supplied elapsed time.
    /// During the delay, progress is zero and the motion remains active.
    /// Zero-duration motion settles immediately after the delay, even when
    /// repeating forever. A zero pass count is always inactive.
    pub fn sample(&self, elapsed: Duration) -> MotionSample {
        let repeat_count = self.repeat_count();
        if repeat_count == Some(0) {
            return MotionSample {
                progress: Progress::START,
                is_active: false,
            };
        }

        let Some(elapsed) = elapsed.checked_sub(self.easing.options.delay) else {
            return MotionSample {
                progress: Progress::START,
                is_active: true,
            };
        };

        if self.duration.is_zero() {
            return MotionSample {
                progress: self.resting_progress(),
                is_active: false,
            };
        }

        // Keep pass boundaries and parity exact, including after long runs or
        // skipped frames. Only the fraction within one pass needs floating point.
        let duration_nanos = self.duration.as_nanos();
        let elapsed_nanos = elapsed.as_nanos();
        let pass = elapsed_nanos / duration_nanos;
        let finished = match (self.repeat, repeat_count) {
            (Repeat::Once, Some(count)) => pass >= u128::from(count),
            (Repeat::Once, None) => pass >= 1,
            (Repeat::Forever, _) => false,
        };

        let linear_progress = if finished {
            self.resting_progress()
        } else {
            let fraction = (elapsed_nanos % duration_nanos) as f64 / duration_nanos as f64;
            let progress = if self.easing.options.auto_reverse && pass % 2 == 1 {
                1.0 - fraction
            } else {
                fraction
            };
            Progress::clamped(progress as f32)
        };

        MotionSample {
            progress: self.easing.sample(linear_progress),
            is_active: !finished,
        }
    }

    /// Evaluates this motion between two timestamps.
    pub fn sample_at<Time>(&self, started_at: Time, now: Time) -> MotionSample
    where
        Time: Sub<Time, Output = Duration>,
    {
        self.sample(now - started_at)
    }

    pub(crate) fn resting_progress(&self) -> Progress {
        match self.repeat {
            Repeat::Once => match self.repeat_count() {
                Some(0) => Progress::START,
                Some(count) if self.easing.options.auto_reverse && count % 2 == 0 => {
                    Progress::START
                }
                _ => Progress::END,
            },
            Repeat::Forever => Progress::START,
        }
    }

    fn repeat_count(&self) -> Option<u32> {
        (self.repeat == Repeat::Once)
            .then_some(self.easing.options.repeat_count)
            .flatten()
    }
}

impl Default for Motion {
    fn default() -> Self {
        Self::new(Duration::ZERO)
    }
}

impl From<Duration> for Motion {
    fn from(duration: Duration) -> Self {
        Self::new(duration)
    }
}

/// The former name of [`Motion`].
#[deprecated(note = "use Motion")]
pub type MotionInfo = Motion;

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_sample(motion: &Motion, elapsed: Duration, progress: f32, is_active: bool) {
        assert_eq!(
            motion.sample(elapsed),
            MotionSample {
                progress: Progress::clamped(progress),
                is_active,
            },
            "at {elapsed:?}"
        );
    }

    #[test]
    fn delayed_counted_motion_waits_only_before_the_first_pass() {
        let motion = Motion::new(secs(1))
            .with_delay(millis(500))
            .with_repeat_count(3);

        for (ms, progress, active) in [
            (0, 0.0, true),
            (499, 0.0, true),
            (500, 0.0, true),
            (750, 0.25, true),
            (1_500, 0.0, true),
            (1_750, 0.25, true),
            (2_500, 0.0, true),
            (3_250, 0.75, true),
            (3_500, 1.0, false),
            (9_000, 1.0, false),
        ] {
            assert_sample(&motion, millis(ms), progress, active);
        }
    }

    #[test]
    fn alternating_passes_reverse_the_easing_curve_and_settle_by_parity() {
        let motion = Motion::new(secs(1))
            .with_easing(|progress| progress * progress)
            .with_repeat_count(4)
            .with_auto_reverse(true);

        for (ms, progress, active) in [
            (250, 0.0625, true),
            (1_000, 1.0, true),
            (1_250, 0.5625, true),
            (2_000, 0.0, true),
            (3_000, 1.0, true),
            (4_000, 0.0, false),
            (9_000, 0.0, false),
        ] {
            assert_sample(&motion, millis(ms), progress, active);
        }

        let odd = motion.clone().with_repeat_count(3);
        assert_sample(&odd, secs(3), 1.0, false);
        let forever = motion.with_repeat(Repeat::Forever);
        assert_sample(&forever, secs(4), 0.0, true);
        assert_sample(&forever, secs(5), 1.0, true);
    }

    #[test]
    fn zero_passes_and_zero_duration_have_finite_activity() {
        let motion = Motion::new(Duration::ZERO)
            .with_delay(secs(1))
            .with_auto_reverse(true);
        for (repeat, end) in [(Repeat::Once, 1.0), (Repeat::Forever, 0.0)] {
            let motion = motion.clone().with_repeat(repeat);
            assert_sample(&motion, millis(999), 0.0, true);
            assert_sample(&motion, secs(1), end, false);
            assert_sample(&motion, Duration::MAX, end, false);
        }
        for (count, end) in [(1, 1.0), (2, 0.0), (3, 1.0)] {
            let motion = motion.clone().with_repeat_count(count);
            assert_sample(&motion, millis(999), 0.0, true);
            assert_sample(&motion, secs(1), end, false);
            assert_sample(&motion, Duration::MAX, end, false);
        }
        for duration in [Duration::ZERO, secs(1)] {
            let motion = Motion::new(duration)
                .with_delay(secs(1))
                .with_repeat_count(0);
            assert_sample(&motion, Duration::ZERO, 0.0, false);
            assert_sample(&motion, Duration::MAX, 0.0, false);
        }
    }

    #[test]
    fn delay_holds_the_origin_even_with_a_nonzero_easing_start() {
        let motion = Motion::new(secs(1))
            .with_delay(secs(1))
            .with_easing(|_| 0.5);
        assert_sample(&motion, millis(999), 0.0, true);
        assert_sample(&motion, secs(1), 0.5, true);
    }

    #[test]
    fn pass_boundaries_remain_exact_for_long_runs_and_tiny_durations() {
        let motion = Motion::new(Duration::from_nanos(2))
            .with_repeat(Repeat::Forever)
            .with_auto_reverse(true);
        // The elapsed nanoseconds exceed the integer precision of f32 and f64.
        assert_sample(&motion, Duration::new(100_000_000, 1), 0.5, true);
        assert_sample(&motion, Duration::new(100_000_000, 2), 1.0, true);
        assert_sample(&motion, Duration::MAX, 0.5, true);

        let finite = motion.with_repeat_count(u32::MAX);
        assert_sample(&finite, Duration::MAX, 1.0, false);
        let long = Motion::new(Duration::MAX).with_repeat_count(u32::MAX);
        assert_sample(&long, Duration::MAX, 0.0, true);
    }

    #[test]
    fn creates_durations() {
        assert_eq!(secs(2), Duration::from_secs(2));
        assert_eq!(millis(250), Duration::from_millis(250));
    }

    #[test]
    fn samples_one_shot_and_eased_motion() {
        let motion = Duration::from_secs(2).with_easing(|progress| progress * progress);

        let cases = [
            (
                Duration::from_secs(1),
                MotionSample {
                    progress: Progress::clamped(0.25),
                    is_active: true,
                },
            ),
            (
                Duration::from_secs(3),
                MotionSample {
                    progress: Progress::END,
                    is_active: false,
                },
            ),
        ];

        for (elapsed, expected) in cases {
            assert_eq!(motion.sample(elapsed), expected);
        }
        assert_eq!(
            motion.sample_at(Duration::from_secs(3), Duration::from_secs(5)),
            MotionSample {
                progress: Progress::END,
                is_active: false,
            }
        );

        assert_eq!(Progress::clamped(-1.0), Progress::START);
        assert_eq!(Progress::clamped(2.0), Progress::END);
    }

    #[test]
    fn repeating_and_zero_duration_motion_use_their_resting_progress() {
        let once = Motion::new(Duration::ZERO).sample(Duration::from_secs(10));
        assert_eq!(once.progress, Progress::END);
        assert!(!once.is_active);

        let mut repeating = Motion::new(Duration::ZERO);
        repeating.repeat = Repeat::Forever;
        let sample = repeating.sample(Duration::from_secs(10));
        assert_eq!(sample.progress, Progress::START);
        assert!(!sample.is_active);

        let duration = Duration::from_secs(1);
        let mut motion = Motion::new(duration);
        motion.repeat = Repeat::Forever;

        assert_eq!(
            motion.sample(Duration::from_millis(250)),
            MotionSample {
                progress: Progress::clamped(0.25),
                is_active: true,
            }
        );
        assert_eq!(motion.sample(duration).progress, Progress::START);
        assert_eq!(
            motion.sample(duration * 2 + Duration::from_millis(500)),
            MotionSample {
                progress: Progress::clamped(0.5),
                is_active: true,
            }
        );
    }
}
