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

    fn repeating(value: f32) -> Self {
        Self::clamped(value % Self::END.0)
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
pub struct Easing(Rc<dyn Fn(f32) -> f32>);

impl Easing {
    /// Creates an easing function.
    pub fn new(easing: impl Fn(f32) -> f32 + 'static) -> Self {
        Self(Rc::new(easing))
    }

    /// Evaluates this easing function with normalized progress.
    pub fn sample(&self, progress: Progress) -> Progress {
        let eased = (self.0)(progress.get());

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

/// Whether motion runs once or repeats indefinitely.
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
    /// How long this motion takes.
    pub duration: Duration,

    /// Maps linear progress to eased progress.
    pub easing: Easing,

    /// Whether this motion runs once or forever.
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
        self.easing = Easing::new(easing);
        self
    }

    /// Evaluates this motion after the supplied elapsed time.
    pub fn sample(&self, elapsed: Duration) -> MotionSample {
        if self.duration.is_zero() {
            return MotionSample {
                progress: self.resting_progress(),
                is_active: false,
            };
        }

        let linear_progress = elapsed.as_secs_f32() / self.duration.as_secs_f32();
        let (linear_progress, is_active) = match self.repeat {
            Repeat::Once => {
                let progress = Progress::clamped(linear_progress);
                (progress, !progress.is_complete())
            }
            Repeat::Forever => (Progress::repeating(linear_progress), true),
        };

        MotionSample {
            progress: self.easing.sample(linear_progress),
            is_active,
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
            Repeat::Once => Progress::END,
            Repeat::Forever => Progress::START,
        }
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
