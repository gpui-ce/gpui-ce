use std::{ops::Sub, time::Duration};

use crate::{Lerp, Motion, Progress};

/// A sampled animated value and its activity state.
#[derive(Clone)]
pub struct AnimatedSample<T> {
    /// The interpolated value at the sampled time.
    pub value: T,

    /// Whether another sample may produce a different value.
    pub is_active: bool,

    /// Eased progress between the interruption anchor and logical value.
    pub progress: Progress,
}

/// A logical value together with the state required to animate its changes.
#[derive(Clone)]
pub struct Animated<T, Time = std::time::Instant> {
    value: T,
    initial_value: T,
    last_value: T,
    motion: Motion,
    settled_progress: Option<Progress>,
    started_at: Option<Time>,
}

impl<T, Time> Animated<T, Time>
where
    T: Lerp + Clone + PartialEq,
    Time: Copy + Sub<Time, Output = Duration>,
{
    /// Creates a completed animated value.
    pub fn new(value: T, motion: Motion) -> Self {
        Self {
            initial_value: value.clone(),
            last_value: value.clone(),
            value,
            motion,
            settled_progress: None,
            started_at: None,
        }
    }

    /// Returns the logical target value.
    pub fn value(&self) -> &T {
        &self.value
    }

    /// Returns eased progress at the supplied time without changing state.
    pub(crate) fn progress_at(&self, now: Time) -> Progress {
        self.started_at.map_or_else(
            || {
                self.settled_progress
                    .unwrap_or_else(|| self.motion.resting_progress())
            },
            |started_at| self.motion.sample_at(started_at, now).progress,
        )
    }

    /// Updates the logical value while preserving positional continuity.
    pub fn set(&mut self, value: T, motion: &Motion, now: Time) -> bool {
        self.retarget(value, motion, now, true)
    }

    /// Updates the logical value and restarts from the initial value.
    pub(crate) fn restart(&mut self, value: T, motion: &Motion, now: Time) -> bool {
        self.retarget(value, motion, now, false)
    }

    fn retarget(&mut self, value: T, motion: &Motion, now: Time, continuous: bool) -> bool {
        if self.value == value {
            return false;
        }

        let current = self.sample(now);
        self.last_value = if continuous {
            current.value
        } else {
            self.initial_value.clone()
        };
        self.value = value;
        self.motion = motion.clone();
        self.settled_progress = None;
        self.started_at = Some(now);
        true
    }

    /// Sets the logical and sampled value without animation.
    pub fn jump_to(&mut self, value: T) {
        self.value = value.clone();
        self.last_value = value;
        self.settled_progress = None;
        self.started_at = None;
    }

    /// Restores the value used to initialize this animation.
    pub fn reset(&mut self) {
        self.value = self.initial_value.clone();
        self.last_value = self.initial_value.clone();
        self.settled_progress = None;
        self.started_at = None;
    }

    /// Evaluates the interpolated value at the supplied time.
    pub fn sample(&mut self, now: Time) -> AnimatedSample<T> {
        let Some(started_at) = self.started_at else {
            return AnimatedSample {
                value: self.last_value.clone(),
                is_active: false,
                progress: self
                    .settled_progress
                    .unwrap_or_else(|| self.motion.resting_progress()),
            };
        };

        let sample = self.motion.sample_at(started_at, now);
        let value = self.last_value.lerp(&self.value, sample.progress.get());

        if !sample.is_active {
            self.last_value = value.clone();
            self.settled_progress = Some(sample.progress);
            self.started_at = None;
        }

        AnimatedSample {
            value,
            is_active: sample.is_active,
            progress: sample.progress,
        }
    }

    pub(crate) fn scale_by(&mut self, ratio: f32)
    where
        T: std::ops::Mul<f32, Output = T>,
    {
        self.last_value = self.last_value.clone() * ratio;
        self.value = self.value.clone() * ratio;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_sample(sample: AnimatedSample<f32>, value: f32, progress: Progress, is_active: bool) {
        assert_eq!(
            (sample.value, sample.progress, sample.is_active),
            (value, progress, is_active)
        );
    }

    #[test]
    fn animated_value_supports_a_complete_lifecycle() {
        let motion = Motion::new(Duration::from_secs(1));
        let mut animated = Animated::<f32, Duration>::new(2.0, motion.clone());

        assert_eq!(animated.value(), &2.0);
        assert_eq!(animated.progress_at(Duration::ZERO), Progress::END);
        assert_sample(animated.sample(Duration::ZERO), 2.0, Progress::END, false);

        assert!(animated.set(10.0, &motion, Duration::ZERO));
        assert_eq!(animated.sample(Duration::from_millis(500)).value, 6.0);
        assert_eq!(
            animated.progress_at(Duration::from_millis(500)),
            Progress::clamped(0.5)
        );

        assert_sample(
            animated.sample(Duration::from_secs(1)),
            10.0,
            Progress::END,
            false,
        );
        assert!(!animated.set(10.0, &motion, Duration::from_secs(2)));

        animated.jump_to(8.0);
        assert_eq!(animated.sample(Duration::from_secs(2)).value, 8.0);

        animated.scale_by(2.0);
        assert_eq!(animated.value(), &16.0);

        let immediate = Motion::default();
        assert!(animated.set(12.0, &immediate, Duration::from_secs(2)));
        assert_sample(
            animated.sample(Duration::from_secs(2)),
            12.0,
            Progress::END,
            false,
        );
        animated.reset();

        assert_eq!(animated.value(), &2.0);
        assert_sample(
            animated.sample(Duration::from_millis(500)),
            2.0,
            Progress::END,
            false,
        );
    }

    #[test]
    fn animated_exercises_retargeting_and_non_monotonic_motion() {
        let motion = Motion::new(Duration::from_secs(1));
        let mut continuous = Animated::<f32, Duration>::new(0.0, motion.clone());
        let mut restarting = Animated::<f32, Duration>::new(0.0, motion.clone());

        assert!(continuous.set(10.0, &motion, Duration::ZERO));
        assert!(restarting.set(10.0, &motion, Duration::ZERO));
        assert_eq!(continuous.sample(Duration::from_millis(500)).value, 5.0);
        assert_eq!(restarting.sample(Duration::from_millis(500)).value, 5.0);

        assert!(continuous.set(20.0, &motion, Duration::from_millis(500)));
        assert!(restarting.restart(20.0, &motion, Duration::from_millis(500)));

        let continuous_anchor = continuous.sample(Duration::from_millis(500));
        let restarting_anchor = restarting.sample(Duration::from_millis(500));
        assert_eq!(continuous_anchor.value, 5.0);
        assert_eq!(restarting_anchor.value, 0.0);
        assert!(continuous_anchor.is_active);
        assert!(restarting_anchor.is_active);

        assert_eq!(continuous.sample(Duration::from_secs(1)).value, 12.5);
        assert_eq!(restarting.sample(Duration::from_secs(1)).value, 10.0);
        assert!(!continuous.set(20.0, &motion, Duration::from_secs(1)));
        assert!(!restarting.restart(20.0, &motion, Duration::from_secs(1)));

        let non_monotonic = Motion::new(Duration::from_secs(1)).with_easing(|progress| {
            if progress < 0.5 {
                progress * 2.0
            } else {
                (1.0 - progress) * 2.0
            }
        });
        let mut animated = Animated::<f32, Duration>::new(0.0, non_monotonic.clone());

        assert!(animated.set(1.0, &non_monotonic, Duration::ZERO));
        assert_sample(animated.sample(Duration::ZERO), 0.0, Progress::START, true);
        assert_sample(
            animated.sample(Duration::from_millis(500)),
            1.0,
            Progress::END,
            true,
        );
        assert_sample(
            animated.sample(Duration::from_secs(1)),
            0.0,
            Progress::START,
            false,
        );
        assert_sample(
            animated.sample(Duration::from_secs(2)),
            0.0,
            Progress::START,
            false,
        );
        assert_eq!(
            animated.progress_at(Duration::from_secs(2)),
            Progress::START
        );
        assert_eq!(animated.value(), &1.0);
    }

    #[test]
    fn animated_keeps_the_motion_that_started_each_run() {
        let one_second = Motion::new(Duration::from_secs(1));
        let two_seconds = Motion::new(Duration::from_secs(2));
        let mut animated = Animated::<f32, Duration>::new(0.0, one_second.clone());

        assert!(animated.set(10.0, &one_second, Duration::ZERO));
        assert_eq!(animated.sample(Duration::from_millis(500)).value, 5.0);
        assert_eq!(
            animated.progress_at(Duration::from_millis(750)),
            Progress::clamped(0.75)
        );

        assert!(animated.set(20.0, &two_seconds, Duration::from_millis(500)));
        assert_eq!(animated.sample(Duration::from_millis(500)).value, 5.0);
        assert_eq!(animated.sample(Duration::from_millis(1_500)).value, 12.5);
    }
}
