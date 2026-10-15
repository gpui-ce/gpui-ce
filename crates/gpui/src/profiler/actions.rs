//! Timing for action handlers: the slowest handlers seen, and the one running now.
//!
//! Timing is recorded only with the `profiler` feature. Without it, the statistics stay
//! empty.

use std::time::Duration;

use itertools::Itertools;
use scheduler::Instant;

#[cfg(feature = "profiler")]
use crate::action::Action;

/// The five slowest action handlers seen so far, and the handler that is running now.
///
/// Read them with [`take_action_stats`]. Only handlers that run for at least 100
/// microseconds are considered for the list, so most calls take the fast path.
#[doc(hidden)]
#[derive(Clone)]
pub struct ActionStatistics {
    runtime_to_beat: Duration,

    longest_runtimes: heapless::Vec<ActionTiming, 5>,
    running: Option<(&'static str, Instant)>,
}

impl std::fmt::Debug for ActionStatistics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActionStatistics")
            .field("runtime_to_beat", &self.runtime_to_beat)
            .field("longest_runtimes", &self.longest_runtimes)
            .field(
                "running",
                &self.running.map(|(id, started)| (id, started.elapsed())),
            )
            .finish()
    }
}

impl std::fmt::Display for ActionStatistics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Actions that blocked the longest\n")?;
        for action in self
            .longest_runtimes(true)
            .sorted_by_key(|action| action.runtime())
            .rev()
        {
            f.write_fmt(format_args!(
                "{:<20} - {}",
                format!("{:?}", action.runtime()), // impl dbg does not support alignment
                action.name
            ))?;
            writeln!(f)?;
        }
        Ok(())
    }
}

impl Default for ActionStatistics {
    fn default() -> Self {
        Self::new()
    }
}

impl ActionStatistics {
    const fn new() -> Self {
        Self {
            // This keeps more calls on the fast path by only tracking
            // problematic polls
            runtime_to_beat: Duration::from_micros(100),
            longest_runtimes: heapless::Vec::new(),
            running: None,
        }
    }

    /// Returns the statistics gathered so far and resets this one. The running action stays
    /// tracked, so it is still reported by the next snapshot.
    pub fn take(&mut self) -> Self {
        let taken = std::mem::take(self);
        self.running = taken.running;
        taken
    }

    /// Returns `true` if no handler has been recorded as one of the longest.
    pub fn is_empty(&self) -> bool {
        self.longest_runtimes.is_empty()
    }

    /// Records that `action` started at `started`, so a long-running handler can be reported
    /// before it finishes.
    #[cfg(feature = "profiler")]
    pub fn update_running_action(&mut self, action: &'static str, started: Instant) {
        self.running = Some((action, started));
    }
    /// Does nothing, because the `profiler` feature is off.
    #[cfg(not(feature = "profiler"))]
    pub fn update_running_action(&mut self, _action: &'static str, _started: Instant) {}

    /// Ends the running action and keeps its timing if it is among the longest.
    #[cfg(feature = "profiler")]
    pub fn save_action_timing(&mut self) {
        self.save_action_timing_at(Instant::now());
    }

    #[cfg(feature = "profiler")]
    fn save_action_timing_at(&mut self, now: Instant) {
        let Some((action, started)) = self.running.take() else {
            // Actions run only on the foreground executor and therefore
            // sequentially _except_ in tests where they can run concurrently.
            //
            // When ran sequentially self.running will always be Some. When ran
            // concurrently that is no longer true. But that is fine, we do not
            // need to track action timings in tests.
            super::cold_path();
            return;
        };

        let timing = ActionTiming {
            name: action,
            start: started,
            end: now,
        };

        let runtime = now.duration_since(started);
        if runtime >= self.runtime_to_beat {
            super::cold_path(); // most actions are not the worst, optimize for that

            if self.longest_runtimes.is_full()
                && let Some(to_replace) = self
                    .longest_runtimes
                    .iter_mut()
                    .min_by_key(|action| action.runtime())
            {
                *to_replace = timing;
            } else {
                self.longest_runtimes
                    .push(timing)
                    .expect("just checked it is not full");
            };

            // Until the list is full, any action above the floor is worth keeping.
            self.runtime_to_beat = if self.longest_runtimes.is_full() {
                self.longest_runtimes
                    .iter()
                    .map(|action| action.runtime())
                    .min()
                    .expect("never empty")
            } else {
                Duration::from_micros(100)
            };
        }
    }
    /// Does nothing, because the `profiler` feature is off.
    #[cfg(not(feature = "profiler"))]
    pub fn save_action_timing(&mut self) {}

    /// Returns the recorded handlers, and the running one if `include_running` is `true`.
    ///
    /// The running handler's `end` is the time of this call.
    pub fn longest_runtimes(&self, include_running: bool) -> impl Iterator<Item = ActionTiming> {
        self.longest_runtimes.iter().copied().chain(
            self.running
                .into_iter()
                .filter(move |_| include_running)
                .map(|(name, start)| ActionTiming {
                    name,
                    start,
                    end: Instant::now(),
                }),
        )
    }
}

/// The timing of one action handler.
///
/// UNSTABLE: only for use in the profiler and zed-reliability.
#[doc(hidden)]
#[derive(Copy, Clone)]
pub struct ActionTiming {
    /// The action's name, or `"un-named"` if it has none.
    pub name: &'static str,
    /// When the handler started.
    pub start: Instant,
    /// When the handler finished, or the time of the snapshot for a running handler.
    pub end: Instant,
}

impl core::fmt::Debug for ActionTiming {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActionTiming")
            .field("name", &self.name)
            .field("runtime", &self.runtime())
            .finish()
    }
}

impl ActionTiming {
    /// Returns how long the handler ran, from `start` to `end`. Returns zero if `end` is before `start`.
    pub fn duration(&self) -> Duration {
        self.end.saturating_duration_since(self.start)
    }
}

impl ActionTiming {
    /// Returns how long the handler ran, computed with `end - start`. Prefer [`Self::duration`].
    #[doc(hidden)]
    pub fn runtime(&self) -> Duration {
        self.end - self.start
    }
}

// The profiler is careful to never block when the lock is held, therefore a
// spinlock is optimal.
#[cfg(feature = "profiler")]
static ACTION_STATISTICS: spin::Mutex<ActionStatistics> =
    const { spin::Mutex::new(ActionStatistics::new()) };

/// Records the start of the action `action` on the foreground thread, and returns its name.
#[doc(hidden)]
#[cfg(feature = "profiler")]
pub(crate) fn update_running_action(
    action: &(dyn Action + 'static),
    cx: &mut crate::App,
) -> &'static str {
    let now = Instant::now();
    let action = action.type_id();
    let action = cx.actions.try_resolve_action(&action).unwrap_or("un-named");
    ACTION_STATISTICS.lock().update_running_action(action, now);
    action
}

/// Ends the running action and records its timing.
#[doc(hidden)]
#[cfg(feature = "profiler")]
pub(crate) fn save_action_timing() {
    ACTION_STATISTICS.lock().save_action_timing();
}

/// Returns the action statistics gathered since the previous call, and resets them.
///
/// Available with the `profiler` feature. Without it, this returns empty statistics.
#[doc(hidden)]
#[cfg(feature = "profiler")]
pub fn take_action_stats() -> ActionStatistics {
    ACTION_STATISTICS.lock().take()
}

/// Returns empty action statistics, because the `profiler` feature is off.
#[doc(hidden)]
#[cfg(not(feature = "profiler"))]
pub fn take_action_stats() -> ActionStatistics {
    ActionStatistics::default()
}

#[cfg(all(test, feature = "profiler"))]
mod tests {
    use super::*;

    #[test]
    fn retains_the_five_longest_actions_regardless_of_arrival_order() {
        let start = Instant::now();
        let mut stats = ActionStatistics::new();
        for microseconds in [500, 100, 400, 200, 300, 50, 350, 150, 600] {
            stats.update_running_action("test", start);
            stats.save_action_timing_at(start + Duration::from_micros(microseconds));
        }

        let mut retained = stats
            .longest_runtimes(false)
            .map(|action| action.duration())
            .collect::<Vec<_>>();
        retained.sort();
        assert_eq!(
            retained,
            [300, 350, 400, 500, 600].map(Duration::from_micros)
        );
        assert_eq!(stats.runtime_to_beat, Duration::from_micros(300));
    }
}
