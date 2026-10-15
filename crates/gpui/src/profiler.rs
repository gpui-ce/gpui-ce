//! Timing for tasks, actions and window frames.
//!
//! Recording is compiled in with the `profiler` feature. Without it, the functions
//! that return timings return empty results, and the recording functions do nothing.
//! Even with the feature, task and frame events in the trace buffers are kept only
//! while tracing is on; [`set_trace_enabled`] controls that state. The foreground journal,
//! which `App::foreground_journal` reads, records them regardless. With the feature, each
//! window also keeps its histograms, whether or not tracing is on.
//!
//! To see frame times on screen, set the debug frame overlay mode with
//! `Window::set_debug_frame_overlay_mode`. To read the numbers in code, call
//! `Window::frame_duration_snapshot` and `Window::input_latency_snapshot`. Those
//! three window methods exist only with the `profiler` feature. The
//! [performance guide](crate::_documentation::guides::performance) walks through them.

#[cfg(feature = "profiler")]
use hdrhistogram::Histogram;
use itertools::Itertools;
use scheduler::{Instant, SpawnTime};
#[cfg(feature = "profiler")]
use smallvec::SmallVec;
use std::{
    cell::LazyCell,
    collections::{HashMap, VecDeque},
    hash::{DefaultHasher, Hash, Hasher},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    thread::ThreadId,
    time::Duration,
};

mod actions;
#[cfg(feature = "profiler")]
pub mod hang;
#[cfg(feature = "profiler")]
pub mod journal;
pub use actions::{ActionStatistics, ActionTiming, take_action_stats};

/// Hints that the calling branch is unlikely to be taken.
///
/// Stands in for `std::hint::cold_path`, which is still unstable on our MSRV
/// (rust-lang/rust#136873); calling a `#[cold]` function is the equivalent hint.
#[cold]
#[inline(always)]
pub(super) fn cold_path() {}

use serde::{Deserialize, Serialize};

#[cfg(feature = "profiler")]
use crate::{Action, App, WindowId};
use crate::{SharedString, TasksIncluded};

/// Returns the task timings of every live thread.
#[cfg(feature = "profiler")]
#[doc(hidden)]
pub fn get_all_timings(included: gpui::TasksIncluded) -> Vec<gpui::ThreadTaskTimings> {
    ThreadTaskTimings::collect(upgraded_thread_timings(), included)
}

/// Returns the task timings of the calling thread.
#[cfg(feature = "profiler")]
#[doc(hidden)]
pub fn get_current_thread_timings(included: TasksIncluded) -> gpui::ThreadTaskTimings {
    gpui::profiler::get_current_thread_task_timings(included)
}

/// Returns and resets the task statistics of every live thread.
#[cfg(feature = "profiler")]
#[doc(hidden)]
pub fn take_all_stats(included: TasksIncluded) -> Vec<gpui::ThreadTaskStatistics> {
    ThreadTaskStatistics::collect_and_reset(upgraded_thread_timings(), included)
}

/// Returns no timings, because the `profiler` feature is off.
#[cfg(not(feature = "profiler"))]
#[doc(hidden)]
pub fn get_all_timings(_included: gpui::TasksIncluded) -> Vec<gpui::ThreadTaskTimings> {
    Vec::new()
}
/// Returns an empty timing for the calling thread, because the `profiler` feature is off.
#[cfg(not(feature = "profiler"))]
#[doc(hidden)]
pub fn get_current_thread_timings(_included: TasksIncluded) -> gpui::ThreadTaskTimings {
    gpui::ThreadTaskTimings {
        thread_name: None,
        thread_id: std::thread::current().id(),
        timings: Vec::new(),
        stats: TaskStatistics::default(),
        total_pushed: 0,
    }
}
/// Returns no statistics, because the `profiler` feature is off.
#[cfg(not(feature = "profiler"))]
#[doc(hidden)]
pub fn take_all_stats(_included: TasksIncluded) -> Vec<gpui::ThreadTaskStatistics> {
    Vec::new()
}

/// The instant a task poll ended, either by yielding or by finishing.
#[doc(hidden)]
#[derive(Debug, Copy, Clone)]
pub struct YieldTime(pub Instant);

/// One completed poll of a task.
#[doc(hidden)]
#[derive(Copy, Clone)]
pub struct TaskTiming {
    /// Where the task was spawned.
    pub location: &'static core::panic::Location<'static>,
    /// When the task was spawned.
    pub spawned: SpawnTime,
    /// When this poll started.
    pub start: Instant,
    /// When this poll yielded or finished.
    pub end: YieldTime,
}

impl std::fmt::Debug for TaskTiming {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TaskTiming")
            .field("location", &self.location)
            .field("since_spawned", &self.spawned.0.elapsed())
            .field("last_poll_duration", &self.poll_duration())
            .field("total_runtime", &self.since_spawn())
            .finish()
    }
}

/// A task poll that is still running on its thread.
#[doc(hidden)]
#[derive(Debug, Copy, Clone)]
pub struct ActiveTiming {
    /// Where the task was spawned.
    pub location: &'static core::panic::Location<'static>,
    /// When the task was spawned.
    pub spawned: SpawnTime,
    /// When this poll started.
    pub start: Instant,
}

impl TaskTiming {
    /// Returns a timing with zero duration, used to fill the statistics before any task has run.
    pub fn placeholder() -> Self {
        let now = Instant::now();
        Self {
            location: std::panic::Location::caller(),
            spawned: SpawnTime(now),
            start: now,
            end: YieldTime(now),
        }
    }

    /// Returns how long this poll ran, from start until it yielded or finished.
    #[inline(always)]
    pub fn poll_duration(&self) -> Duration {
        self.end.0 - self.start
    }

    #[inline(always)]
    fn since_spawn(&self) -> Duration {
        self.end.0 - self.spawned.0
    }
}

/// The recorded task polls of one thread.
#[doc(hidden)]
#[derive(Debug, Clone)]
pub struct ThreadTaskTimings {
    /// The thread's name, if it has one.
    pub thread_name: Option<String>,
    /// The thread's ID.
    pub thread_id: ThreadId,
    /// Completed polls, oldest first, and the running poll if it was requested.
    pub timings: Vec<TaskTiming>,
    /// The longest polls and runtimes seen on this thread.
    pub stats: TaskStatistics,
    /// The number of polls recorded since the thread started. This can be larger than
    /// `timings.len()`, because the buffer keeps only the most recent polls.
    pub total_pushed: u64,
}

impl ThreadTaskTimings {
    /// Copies the given threads' recorded polls into [`ThreadTaskTimings`] values.
    ///
    /// The running poll is included only when `included` is [`TasksIncluded::CompletedAndRunning`].
    pub fn collect(
        timings: Vec<(ThreadId, Arc<GuardedTaskTimings>)>,
        included: TasksIncluded,
    ) -> Vec<Self> {
        timings
            .into_iter()
            .map(|(thread_id, timings)| {
                let timings = timings.lock();
                let thread_name = timings.thread_name.clone();
                let total_pushed = timings.total_pushed;
                let completed = &timings.timings;

                let mut vec = Vec::with_capacity(completed.len() + 1); // +1 for running task
                let (s1, s2) = completed.as_slices();
                vec.extend_from_slice(s1);
                vec.extend_from_slice(s2);
                if let TasksIncluded::CompletedAndRunning = included
                    && let Some(running) = timings.running
                {
                    vec.push(TaskTiming {
                        location: running.location,
                        spawned: running.spawned,
                        start: running.start,
                        end: YieldTime(Instant::now()),
                    })
                }

                ThreadTaskTimings {
                    thread_name,
                    thread_id,
                    timings: vec,
                    stats: timings.stats.clone(),
                    total_pushed,
                }
            })
            .collect()
    }
}

/// The task statistics of one thread, taken from the thread's recorder.
#[doc(hidden)]
#[derive(Debug)]
pub struct ThreadTaskStatistics {
    /// The thread's name, if it has one.
    pub thread_name: Option<String>,
    /// The thread's ID.
    pub thread_id: ThreadId,
    /// The statistics that were taken.
    pub stats: TaskStatistics,
}

impl ThreadTaskStatistics {
    /// Takes each thread's statistics and resets them to [`TaskStatistics::default`].
    ///
    /// The running poll is folded into the statistics when `include_running` is
    /// [`TasksIncluded::CompletedAndRunning`].
    pub fn collect_and_reset(
        timings: Vec<(ThreadId, Arc<GuardedTaskTimings>)>,
        include_running: TasksIncluded,
    ) -> Vec<Self> {
        timings
            .into_iter()
            .map(|(thread_id, timings)| {
                let mut timings = timings.lock();
                let thread_name = timings.thread_name.clone();

                let mut stats = std::mem::take(&mut timings.stats);
                if let TasksIncluded::CompletedAndRunning = include_running
                    && let Some(ActiveTiming {
                        location,
                        spawned,
                        start,
                    }) = timings.running
                {
                    let end = YieldTime(Instant::now());
                    let timing = TaskTiming {
                        location,
                        spawned,
                        start,
                        end,
                    };
                    stats.add_runtime(timing);
                    stats.add_yield_timing(timing);
                }

                Self {
                    thread_name,
                    thread_id,
                    stats,
                }
            })
            .collect()
    }
}

/// Serializable variant of [`core::panic::Location`]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializedLocation {
    /// Name of the source file
    pub file: SharedString,
    /// Line in the source file
    pub line: u32,
    /// Column in the source file
    pub column: u32,
}

impl From<&core::panic::Location<'static>> for SerializedLocation {
    fn from(value: &core::panic::Location<'static>) -> Self {
        SerializedLocation {
            file: value.file().into(),
            line: value.line(),
            column: value.column(),
        }
    }
}

/// Serializable variant of [`TaskTiming`]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializedTaskTiming {
    /// Where the task was spawned
    pub location: SerializedLocation,
    /// Start of the poll, in nanoseconds after the anchor passed to [`Self::convert`]
    pub start: u128,
    /// How long the poll ran, in nanoseconds
    pub duration: u128,
}

impl SerializedTaskTiming {
    /// Converts a slice of [`TaskTiming`] values into their serializable format.
    ///
    /// # Params
    ///
    /// `anchor` - [`Instant`] that should be earlier than all timings to use as base anchor
    pub fn convert(anchor: Instant, timings: &[TaskTiming]) -> Vec<SerializedTaskTiming> {
        let serialized = timings
            .iter()
            .map(|timing| {
                let start = timing.start.duration_since(anchor).as_nanos();
                let duration = timing.end.0.duration_since(timing.start).as_nanos();
                SerializedTaskTiming {
                    location: timing.location.into(),
                    start,
                    duration,
                }
            })
            .collect::<Vec<_>>();

        serialized
    }

    /// Converts one [`TaskTiming`] into its serializable format.
    ///
    /// `anchor` is the [`Instant`] that `start` is measured from. It should be no later than
    /// the timing's start.
    pub fn from(anchor: Instant, timing: TaskTiming) -> SerializedTaskTiming {
        let start = timing.start.duration_since(anchor).as_nanos();
        let duration = timing.end.0.duration_since(timing.start).as_nanos();
        SerializedTaskTiming {
            location: timing.location.into(),
            start,
            duration,
        }
    }
}

/// Serializable variant of [`ThreadTaskTimings`]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializedThreadTaskTimings {
    /// Thread name
    pub thread_name: Option<String>,
    /// Hash of the thread id
    pub thread_id: u64,
    /// Timing records for this thread
    pub timings: Vec<SerializedTaskTiming>,
}

impl SerializedThreadTaskTimings {
    /// Convert [`ThreadTaskTimings`] into their serializable format
    ///
    /// # Params
    ///
    /// `anchor` - [`Instant`] that should be earlier than all timings to use as base anchor
    pub fn convert(anchor: Instant, timings: ThreadTaskTimings) -> SerializedThreadTaskTimings {
        let serialized_timings = SerializedTaskTiming::convert(anchor, &timings.timings);

        let mut hasher = DefaultHasher::new();
        timings.thread_id.hash(&mut hasher);
        let thread_id = hasher.finish();

        SerializedThreadTaskTimings {
            thread_name: timings.thread_name,
            thread_id,
            timings: serialized_timings,
        }
    }
}

/// The polls one thread recorded since the previous [`ProfilingCollector::collect_unseen`] call.
#[doc(hidden)]
#[derive(Debug, Clone)]
pub struct ThreadTimingsDelta {
    /// Hashed thread id
    pub thread_id: u64,
    /// Thread name, if known
    pub thread_name: Option<String>,
    /// New timings since the last call. If the circular buffer wrapped around
    /// since the previous poll, some entries may have been lost.
    pub new_timings: Vec<SerializedTaskTiming>,
}

/// Tracks which timing events have already been seen so that callers can request only unseen events.
#[doc(hidden)]
pub struct ProfilingCollector {
    startup_time: Instant,
    cursors: HashMap<ThreadId, u64>,
}

impl ProfilingCollector {
    /// Creates a collector that reports timings relative to `startup_time`.
    pub fn new(startup_time: Instant) -> Self {
        Self {
            startup_time,
            cursors: HashMap::default(),
        }
    }

    /// Returns the instant that serialized timings are measured from.
    pub fn startup_time(&self) -> Instant {
        self.startup_time
    }

    /// Returns the polls recorded since the previous call, one entry per thread that has new polls.
    ///
    /// Polls that were evicted from a thread's buffer before this call are skipped.
    pub fn collect_unseen(
        &mut self,
        all_timings: Vec<ThreadTaskTimings>,
    ) -> Vec<ThreadTimingsDelta> {
        let mut deltas = Vec::with_capacity(all_timings.len());

        for thread in all_timings {
            let mut hasher = DefaultHasher::new();
            thread.thread_id.hash(&mut hasher);
            let hashed_id = hasher.finish();

            let prev_cursor = self.cursors.get(&thread.thread_id).copied().unwrap_or(0);
            let buffer_len = thread.timings.len() as u64;
            let buffer_start = thread.total_pushed.saturating_sub(buffer_len);

            let mut slice = if prev_cursor < buffer_start {
                // Cursor fell behind the buffer — some entries were evicted.
                // Return everything still in the buffer.
                thread.timings.as_slice()
            } else {
                let skip = (prev_cursor - buffer_start) as usize;
                &thread.timings[skip.min(thread.timings.len())..]
            };

            let cursor_advance = thread.total_pushed;
            self.cursors.insert(thread.thread_id, cursor_advance);

            if slice.is_empty() {
                continue;
            }

            let new_timings = SerializedTaskTiming::convert(self.startup_time, slice);

            deltas.push(ThreadTimingsDelta {
                thread_id: hashed_id,
                thread_name: thread.thread_name,
                new_timings,
            });
        }

        deltas
    }

    /// Forgets every cursor, so the next [`Self::collect_unseen`] returns all buffered polls.
    pub fn reset(&mut self) {
        self.cursors.clear();
    }
}

// Allow 16MiB of task timing entries.
// VecDeque grows by doubling its capacity when full, so keep this a power of 2 to avoid wasting
// memory.
#[cfg(feature = "profiler")]
const MAX_TASK_TIMINGS: usize = (16 * 1024 * 1024) / core::mem::size_of::<TaskTiming>();

#[doc(hidden)]
pub(crate) type TaskTimings = VecDeque<TaskTiming>;

/// A thread's timings behind a spinlock, shared between the thread and the profiler.
#[doc(hidden)]
pub type GuardedTaskTimings = spin::Mutex<ThreadTimings>;

/// A weak handle to one thread's timings, kept in [`GLOBAL_THREAD_TIMINGS`].
#[doc(hidden)]
pub struct GlobalThreadTimings {
    /// The thread's ID.
    pub thread_id: ThreadId,
    /// A weak handle, so the registry doesn't keep a finished thread's timings alive.
    pub timings: std::sync::Weak<GuardedTaskTimings>,
}

/// The five longest polls and the five longest runtimes seen on one thread.
#[doc(hidden)]
#[derive(Debug, Clone)]
pub struct TaskStatistics {
    /// The poll duration a poll must reach to be recorded in `longest_poll_times`.
    pub poll_time_to_beat: Duration,
    /// The runtime a task must reach to be recorded in `longest_runtimes`.
    pub runtime_to_beat: Duration,
    /// The five polls that ran the longest before yielding.
    pub longest_poll_times: [TaskTiming; 5],
    /// The five tasks that ran the longest from spawn to their last poll.
    pub longest_runtimes: [TaskTiming; 5],
}

impl std::fmt::Display for TaskStatistics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Tasks that blocked the longest before yielding\n")?;
        for timing in self.longest_poll_times {
            f.write_fmt(format_args!(
                "{:<20} - {}:{}\n",
                format!("{:?}", timing.poll_duration()),
                timing.location.file(),
                timing.location.column()
            ))?;
        }
        f.write_str("Tasks that ran the longest\n")?;
        for timing in self.longest_runtimes {
            f.write_fmt(format_args!(
                "{:<20} - {}:{}\n",
                format!("{:?}", timing.since_spawn()),
                timing.location.file(),
                timing.location.column()
            ))?;
        }
        Ok(())
    }
}

impl Default for TaskStatistics {
    fn default() -> Self {
        Self {
            // Do not track polls that are not problematic
            // this keeps more calls on the fast path
            poll_time_to_beat: Duration::from_micros(100),
            runtime_to_beat: Duration::from_micros(100),
            longest_poll_times: [TaskTiming::placeholder(); 5],
            longest_runtimes: [TaskTiming::placeholder(); 5],
        }
    }
}

impl TaskStatistics {
    #[inline(always)]
    fn add_yield_timing(&mut self, task: TaskTiming) {
        let yielded_after = task.poll_duration();
        if yielded_after >= self.poll_time_to_beat {
            cold_path(); // most tasks are not the worst, optimize for that
            let to_replace = self
                .longest_poll_times
                .iter()
                .position_min_by_key(|task| task.poll_duration())
                .expect("guarded by the comparison with nth_longest_yield_time");
            self.longest_poll_times[to_replace] = task;

            self.poll_time_to_beat = self
                .longest_poll_times
                .iter()
                .map(|task| task.poll_duration())
                .min()
                .expect("never empty")
                .max(Duration::from_micros(100));
        }
    }

    #[inline(always)]
    fn add_runtime(&mut self, task: TaskTiming) {
        let runtime = task.since_spawn();
        if runtime >= self.runtime_to_beat {
            cold_path(); // most tasks are not the worst, optimize for that
            let to_replace = self
                .longest_runtimes
                .iter()
                .position_min_by_key(|task| task.since_spawn())
                .expect("guarded by the comparison with nth_longest_yield_time");
            self.longest_runtimes[to_replace] = task;

            self.runtime_to_beat = self
                .longest_runtimes
                .iter()
                .map(|task| task.since_spawn())
                .min()
                .expect("never empty")
                .max(Duration::from_micros(100));
        }
    }
}

/// Every thread that has recorded timings, so the profiler can read them from any thread.
#[doc(hidden)]
pub static GLOBAL_THREAD_TIMINGS: spin::Mutex<Vec<GlobalThreadTimings>> =
    spin::Mutex::new(Vec::new());

/// Upgrades all live per-thread timing handles, holding the global registry
/// lock only for the duration of the upgrades.
///
/// The upgraded `Arc`s must never be dropped while `GLOBAL_THREAD_TIMINGS` is
/// locked: dropping the last strong reference runs [`ThreadTimings::drop`],
/// which locks `GLOBAL_THREAD_TIMINGS` again and would deadlock the
/// non-reentrant spinlock. A thread exiting concurrently can hand off its last
/// reference to us at any time, so callers of this function process (lock,
/// read, drop) the returned handles only after the global lock is released.
fn upgraded_thread_timings() -> Vec<(ThreadId, Arc<GuardedTaskTimings>)> {
    let global_thread_timings = GLOBAL_THREAD_TIMINGS.lock();
    global_thread_timings
        .iter()
        .filter_map(|t| Some((t.thread_id, t.timings.upgrade()?)))
        .collect()
}

thread_local! {
    /// This thread's timings. Created on first use and registered in [`GLOBAL_THREAD_TIMINGS`].
    #[doc(hidden)]
    pub static THREAD_TIMINGS: LazyCell<Arc<GuardedTaskTimings>> = LazyCell::new(|| {
        let current_thread = std::thread::current();
        let thread_name = current_thread.name();
        let thread_id = current_thread.id();
        let timings = ThreadTimings::new(thread_name.map(|e| e.to_string()), thread_id);
        let timings = Arc::new(spin::Mutex::new(timings));

        {
            let timings = Arc::downgrade(&timings);
            let global_timings = GlobalThreadTimings {
                thread_id: std::thread::current().id(),
                timings,
            };
            GLOBAL_THREAD_TIMINGS.lock().push(global_timings);
        }

        timings
    });
}

/// The timing state of one thread: its completed polls, the running poll and its statistics.
#[doc(hidden)]
pub struct ThreadTimings {
    /// The thread's name, if it has one.
    pub thread_name: Option<String>,
    /// The thread's ID.
    pub thread_id: ThreadId,
    /// Completed polls, oldest first. Holds about 16 MiB of entries and drops the oldest first.
    pub timings: TaskTimings,
    /// The poll that is running now, if any.
    pub running: Option<ActiveTiming>,
    /// The longest polls and runtimes seen on this thread.
    pub stats: TaskStatistics,
    /// The number of polls pushed to `timings` since the thread started, including evicted ones.
    /// Counts only while tracing is on.
    pub total_pushed: u64,
}

impl ThreadTimings {
    /// Creates empty timing state for a thread.
    pub fn new(thread_name: Option<String>, thread_id: ThreadId) -> Self {
        ThreadTimings {
            thread_name,
            thread_id,
            timings: TaskTimings::new(),
            stats: TaskStatistics::default(),
            total_pushed: 0,
            running: None,
        }
    }

    /// Marks a poll as started now. Called before each task poll.
    #[cfg(feature = "profiler")]
    pub fn update_running_task(
        &mut self,
        spawned: SpawnTime,
        location: &'static std::panic::Location<'_>,
    ) {
        let start = Instant::now();
        self.running = Some(ActiveTiming {
            spawned,
            location,
            start,
        });
    }
    /// Does nothing, because the `profiler` feature is off.
    #[cfg(not(feature = "profiler"))]
    pub fn update_running_task(&mut self, _: SpawnTime, _: &'static std::panic::Location<'_>) {}

    /// Ends the running poll at `ended`, records it in the statistics and, while tracing is on, in the buffer.
    ///
    /// # Panics
    ///
    /// Panics if no poll is running. It must follow a call to [`Self::update_running_task`].
    #[cfg(feature = "profiler")]
    pub fn save_task_timing(&mut self, ended: YieldTime) -> TaskTiming {
        let ActiveTiming {
            location,
            start,
            spawned,
        } = self
            .running
            .take()
            .expect("this function is only ever called after register_task_start");

        let timing = TaskTiming {
            location,
            spawned,
            start,
            end: ended,
        };
        self.stats.add_yield_timing(timing);
        self.stats.add_runtime(timing);

        if trace_enabled() {
            cold_path(); // optimize for when the profiling is off
            if self.timings.len() >= MAX_TASK_TIMINGS {
                self.timings.pop_front();
            }
            self.timings.push_back(timing);
            self.total_pushed += 1;
        }
        timing
    }
    /// Does nothing, because the `profiler` feature is off.
    #[cfg(not(feature = "profiler"))]
    pub fn save_task_timing(&mut self, _: YieldTime) {}

    /// Returns a copy of this thread's recorded polls and statistics.
    ///
    /// The running poll is included when `includes` is [`TasksIncluded::CompletedAndRunning`],
    /// with its end set to the time of this call.
    pub fn get_thread_task_timings(&self, includes: TasksIncluded) -> ThreadTaskTimings {
        ThreadTaskTimings {
            thread_name: self.thread_name.clone(),
            thread_id: self.thread_id,
            timings: self
                .timings
                .iter()
                .cloned()
                .chain(
                    self.running
                        .filter(|_| matches!(includes, TasksIncluded::CompletedAndRunning))
                        .map(|running| TaskTiming {
                            spawned: running.spawned,
                            location: running.location,
                            start: running.start,
                            end: YieldTime(Instant::now()),
                        }),
                )
                .collect(),
            stats: self.stats.clone(),
            total_pushed: self.total_pushed,
        }
    }
}

impl Drop for ThreadTimings {
    fn drop(&mut self) {
        let mut thread_timings = GLOBAL_THREAD_TIMINGS.lock();

        let Some((index, _)) = thread_timings
            .iter()
            .enumerate()
            .find(|(_, t)| t.thread_id == self.thread_id)
        else {
            return;
        };
        thread_timings.swap_remove(index);
    }
}

/// Marks a task poll as started on the calling thread. Called by the scheduler before each poll.
#[doc(hidden)]
pub fn update_running_task(spawned: SpawnTime, location: &'static std::panic::Location<'_>) {
    #[cfg(feature = "profiler")]
    journal::begin_foreground_turn();
    THREAD_TIMINGS.with(|timings| {
        timings.lock().update_running_task(spawned, location);
    });
}

/// Ends the task poll that [`update_running_task`] started on the calling thread.
#[doc(hidden)]
pub fn save_task_timing() {
    let yielded_at = YieldTime(Instant::now());
    #[cfg(feature = "profiler")]
    {
        let timing = THREAD_TIMINGS.with(|timings| timings.lock().save_task_timing(yielded_at));
        journal::record_task_poll(timing);
    }
    #[cfg(not(feature = "profiler"))]
    THREAD_TIMINGS.with(|timings| {
        timings.lock().save_task_timing(yielded_at);
    });
}

/// Returns the calling thread's recorded polls and statistics.
#[doc(hidden)]
pub fn get_current_thread_task_timings(include_running: TasksIncluded) -> ThreadTaskTimings {
    THREAD_TIMINGS.with(|timings| timings.lock().get_thread_task_timings(include_running))
}

const TRACE_SETTING_ENABLED: u64 = 1 << 63;
const TRACE_SCOPE_COUNT_MASK: u64 = TRACE_SETTING_ENABLED - 1;
static TRACE_STATE: AtomicU64 = AtomicU64::new(0);

/// Turns trace collection on or off, and returns `true` if the setting changed.
///
/// While tracing is off, task polls and frame events update only the cheap
/// statistics, and nothing is added to the trace buffers. Turning tracing off
/// clears the per-thread task buffers and the frame-event buffer, unless a trace
/// scope is still active, so a later re-enable doesn't report stale data. Active
/// trace scopes keep collection on until the last scope ends, whatever this
/// setting is. Passing the current setting changes nothing and returns `false`.
///
/// # Examples
///
/// ```
/// use gpui::profiler::{set_trace_enabled, trace_enabled};
///
/// assert!(set_trace_enabled(true));
/// assert!(trace_enabled());
/// assert!(!set_trace_enabled(true), "already on, so nothing changed");
/// set_trace_enabled(false);
/// ```
pub fn set_trace_enabled(enabled: bool) -> bool {
    let mut state = TRACE_STATE.load(Ordering::Acquire);
    loop {
        let was_enabled = state & TRACE_SETTING_ENABLED != 0;
        if was_enabled == enabled {
            return false;
        }

        let next_state = if enabled {
            state | TRACE_SETTING_ENABLED
        } else {
            state & TRACE_SCOPE_COUNT_MASK
        };
        match TRACE_STATE.compare_exchange_weak(
            state,
            next_state,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => {
                if next_state == 0 {
                    clear_trace_buffers();
                }
                return true;
            }
            Err(updated_state) => state = updated_state,
        }
    }
}

/// Keeps trace collection on while it lives. Created by [`trace_scope`].
#[cfg(any(feature = "bench-support", all(test, feature = "profiler")))]
pub(crate) struct TraceGuard;

/// Starts a trace scope, so tracing stays on until the returned guard is dropped.
#[cfg(any(feature = "bench-support", all(test, feature = "profiler")))]
pub(crate) fn trace_scope() -> TraceGuard {
    let incremented = TRACE_STATE.fetch_update(Ordering::AcqRel, Ordering::Acquire, |state| {
        (state & TRACE_SCOPE_COUNT_MASK < TRACE_SCOPE_COUNT_MASK).then_some(state + 1)
    });
    assert!(incremented.is_ok(), "too many active profiler trace scopes");
    TraceGuard
}

#[cfg(any(feature = "bench-support", all(test, feature = "profiler")))]
impl Drop for TraceGuard {
    fn drop(&mut self) {
        let previous_state =
            TRACE_STATE.fetch_update(Ordering::AcqRel, Ordering::Acquire, |state| {
                (state & TRACE_SCOPE_COUNT_MASK > 0).then_some(state - 1)
            });
        match previous_state {
            Ok(1) => clear_trace_buffers(),
            Ok(_) => {}
            Err(_) => debug_assert!(false, "profiler trace scope count underflowed"),
        }
    }
}

/// Returns `true` while trace collection is on, either from [`set_trace_enabled`] or from an active trace scope.
pub fn trace_enabled() -> bool {
    TRACE_STATE.load(Ordering::Relaxed) != 0
}

fn clear_trace_buffers() {
    for (_, timings) in upgraded_thread_timings() {
        let mut timings = timings.lock();
        timings.timings.clear();
        timings.timings.shrink_to_fit();
        timings.total_pushed = 0;
    }
    #[cfg(feature = "profiler")]
    {
        let mut frames = FRAME_TIMINGS.lock();
        frames.timings.clear();
        frames.timings.shrink_to_fit();
        frames.total_pushed = 0;
    }
}

/// The timing of one drawn window frame. Available with the `profiler` feature.
#[cfg(feature = "profiler")]
#[derive(Debug, Copy, Clone)]
pub struct FrameTiming {
    /// The window that was drawn.
    pub window_id: WindowId,
    /// When the frame first became dirty (its first invalidation). `None` if
    /// nothing invalidated the window since its previous draw.
    pub dirty_at: Option<Instant>,
    /// Number of invalidations coalesced into this frame.
    pub invalidations: u64,
    /// When `Window::draw` started.
    pub draw_start: Instant,
    /// When `Window::draw` finished.
    pub draw_end: Instant,
}

#[cfg(feature = "profiler")]
impl FrameTiming {
    /// Time spent inside `Window::draw`.
    pub fn draw_duration(&self) -> Duration {
        self.draw_end.duration_since(self.draw_start)
    }

    /// Time from the frame's first invalidation to the end of its draw, if the
    /// first invalidation was observed.
    pub fn dirty_to_draw_duration(&self) -> Option<Duration> {
        self.dirty_at
            .map(|dirty_at| self.draw_end.duration_since(dirty_at))
    }
}

/// The work spent submitting a window frame to the platform. Available with the `profiler` feature.
#[cfg(feature = "profiler")]
#[derive(Debug, Copy, Clone)]
pub struct PresentTiming {
    /// The window whose frame was submitted.
    pub window_id: WindowId,
    /// When the platform submission began.
    pub present_start: Instant,
    /// When the platform submission completed.
    pub present_end: Instant,
    /// The interval since the previous newly drawn frame was submitted, when
    /// both frames belong to an active animation.
    pub animation_interval: Option<Duration>,
}

#[cfg(feature = "profiler")]
impl PresentTiming {
    /// Time spent submitting the frame to the platform.
    pub fn present_duration(&self) -> Duration {
        self.present_end.duration_since(self.present_start)
    }
}

/// A frame event observed by the profiler. Available with the `profiler` feature.
#[cfg(feature = "profiler")]
#[derive(Debug, Copy, Clone)]
pub enum FrameEvent {
    /// A window frame was drawn.
    Draw(FrameTiming),
    /// A newly drawn window frame was presented.
    Present(PresentTiming),
}

/// A point-in-time copy of a window's frame-duration histograms, for formatting or logging.
///
/// Get one with [`Window::frame_duration_snapshot`]. Available with the `profiler` feature.
#[cfg(feature = "profiler")]
#[derive(Clone)]
pub struct FrameDurationSnapshot {
    /// Histogram of durations from the first invalidation through presentation, in nanoseconds.
    pub dirty_to_present_histogram: Histogram<u64>,
    /// Histogram of `Window::draw` durations, in nanoseconds.
    pub draw_duration_histogram: Histogram<u64>,
    /// Histogram of intervals between consecutively presented frames while the
    /// window was animating, in nanoseconds.
    pub present_interval_histogram: Histogram<u64>,
}

/// A point-in-time copy of a window's input-latency histograms, for formatting or logging.
///
/// Get one with [`Window::input_latency_snapshot`]. Available with the `profiler` feature.
#[cfg(feature = "profiler")]
#[derive(Clone)]
pub struct InputLatencySnapshot {
    /// Histogram of input-to-frame latency samples, in nanoseconds.
    pub latency_histogram: Histogram<u64>,
    /// Histogram of input events coalesced per rendered frame.
    pub events_per_frame_histogram: Histogram<u64>,
    /// Count of input events that arrived mid-draw and were excluded from
    /// latency recording.
    pub mid_draw_events_dropped: u64,
}

#[cfg(feature = "profiler")]
enum WindowActivity {
    Input {
        started_at: Instant,
        kind: &'static str,
    },
    Draw {
        started_at: Instant,
    },
}

/// Collects profiling information for one window.
///
/// Aggregate histograms are always populated when the `profiler` feature is
/// compiled in. Individual draw and present events are added to the global
/// profiler buffer only while tracing is enabled. Each window owns one. Read its
/// results with [`Window::frame_duration_snapshot`] and [`Window::input_latency_snapshot`].
#[cfg(feature = "profiler")]
pub struct WindowProfiler {
    window_id: WindowId,
    active_activities: SmallVec<[WindowActivity; 4]>,
    active_actions: SmallVec<[(&'static str, Instant); 2]>,
    dirty_to_present_histogram: Histogram<u64>,
    draw_duration_histogram: Histogram<u64>,
    present_interval_histogram: Histogram<u64>,
    first_input_at: Option<Instant>,
    pending_input_count: u64,
    input_latency_histogram: Histogram<u64>,
    events_per_frame_histogram: Histogram<u64>,
    mid_draw_events_dropped: u64,
    last_present_at: Option<Instant>,
    animating_at_last_present: bool,
    pending_frame: Option<FrameTiming>,
}

#[cfg(feature = "profiler")]
impl WindowProfiler {
    /// Creates a profiler for the window `window_id`.
    ///
    /// # Errors
    ///
    /// Returns an error if one of the histograms can't be allocated.
    pub fn new(window_id: WindowId) -> anyhow::Result<Self> {
        let profiler = Self {
            window_id,
            active_activities: SmallVec::new(),
            active_actions: SmallVec::new(),
            dirty_to_present_histogram: Histogram::new(3).map_err(|error| {
                anyhow::anyhow!("Failed to create dirty-to-present histogram: {error}")
            })?,
            draw_duration_histogram: Histogram::new(3).map_err(|error| {
                anyhow::anyhow!("Failed to create draw duration histogram: {error}")
            })?,
            present_interval_histogram: Histogram::new(3).map_err(|error| {
                anyhow::anyhow!("Failed to create present interval histogram: {error}")
            })?,
            first_input_at: None,
            pending_input_count: 0,
            input_latency_histogram: Histogram::new(3).map_err(|error| {
                anyhow::anyhow!("Failed to create input latency histogram: {error}")
            })?,
            events_per_frame_histogram: Histogram::new(3).map_err(|error| {
                anyhow::anyhow!("Failed to create events per frame histogram: {error}")
            })?,
            mid_draw_events_dropped: 0,
            last_present_at: None,
            animating_at_last_present: false,
            pending_frame: None,
        };
        journal::record_frame_pending(window_id, Instant::now());
        Ok(profiler)
    }

    /// Records the start of an input dispatch. `kind` names the platform
    /// input variant being dispatched (see [`crate::PlatformInput::kind_name`]).
    ///
    /// Each call needs a matching [`Self::end_input`].
    pub fn begin_input(&mut self, kind: &'static str) {
        journal::begin_foreground_turn();
        self.active_activities.push(WindowActivity::Input {
            started_at: Instant::now(),
            kind,
        });
    }

    /// Records the end of the input dispatch that [`Self::begin_input`] started.
    ///
    /// If the input caused an invalidation and didn't arrive during a draw, it counts
    /// toward the input-latency histograms at the next present. Input that arrives
    /// during a draw is counted in `mid_draw_events_dropped` instead.
    pub fn end_input(&mut self, caused_invalidation: bool) {
        let Some(WindowActivity::Input { started_at, kind }) = self.active_activities.pop() else {
            debug_assert!(false, "input activity must be the current window activity");
            journal::end_foreground_turn();
            return;
        };

        journal::record_input(journal::InputTiming {
            kind,
            start: started_at,
            end: Instant::now(),
            caused_invalidation,
        });
        journal::end_foreground_turn();

        if !caused_invalidation {
            return;
        }

        let arrived_during_draw = self
            .active_activities
            .iter()
            .any(|activity| matches!(activity, WindowActivity::Draw { .. }));
        if arrived_during_draw {
            self.mid_draw_events_dropped += 1;
        } else {
            self.first_input_at.get_or_insert(started_at);
            self.pending_input_count += 1;
        }
    }

    /// Records the start of the handler for `action`. Pair it with [`Self::end_action_handler`].
    pub fn begin_action_handler(&mut self, action: &(dyn Action + 'static), cx: &mut App) {
        journal::begin_foreground_turn();
        let name = actions::update_running_action(action, cx);
        self.active_actions.push((name, Instant::now()));
    }

    /// Records the end of the current action handler.
    pub fn end_action_handler(&mut self) {
        // Dual-write to the legacy aggregate store; its single global running
        // slot misbehaves when tests run actions concurrently, which is why
        // the journal entry is tracked here on the window instead.
        actions::save_action_timing();
        let Some((name, start)) = self.active_actions.pop() else {
            debug_assert!(false, "action handler must be begun before it ends");
            journal::end_foreground_turn();
            return;
        };
        journal::record_action(ActionTiming {
            name,
            start,
            end: Instant::now(),
        });
        journal::end_foreground_turn();
    }

    /// Records the start of a window draw. Pair it with [`Self::end_draw`].
    pub fn begin_draw(&mut self) {
        journal::begin_foreground_turn();
        let started_at = Instant::now();
        journal::record_frame_pending(self.window_id, started_at);
        self.active_activities
            .push(WindowActivity::Draw { started_at });
    }

    /// Records the end of the draw that [`Self::begin_draw`] started, and returns how long it took.
    ///
    /// `dirty_at` is when the frame was first invalidated, and `invalidations` is how many
    /// invalidations this frame covers. Returns [`Duration::ZERO`] if no draw was begun.
    pub fn end_draw(&mut self, dirty_at: Option<Instant>, invalidations: u64) -> Duration {
        let Some(WindowActivity::Draw {
            started_at: draw_start,
        }) = self.active_activities.pop()
        else {
            debug_assert!(false, "draw activity must be the current window activity");
            journal::end_foreground_turn();
            return Duration::ZERO;
        };

        let draw_end = Instant::now();
        let frame_timing = FrameTiming {
            window_id: self.window_id,
            dirty_at,
            invalidations,
            draw_start,
            draw_end,
        };
        let draw_duration = frame_timing.draw_duration();
        self.record_draw_timing(frame_timing);
        journal::end_foreground_turn();
        draw_duration
    }

    /// Records that the window's latest frame was presented between `present_start`
    /// and `present_end`.
    ///
    /// `window_active` says whether the window is active, and `next_frame_scheduled`
    /// says whether another frame is already requested. Together they decide whether
    /// the gap until the next newly drawn frame counts as an animation interval.
    pub fn record_present(
        &mut self,
        present_start: Instant,
        present_end: Instant,
        window_active: bool,
        next_frame_scheduled: bool,
    ) {
        self.record_present_at(
            present_start,
            present_end,
            window_active,
            next_frame_scheduled,
        );
    }

    /// Returns a copy of the input-latency histograms. Latency runs from the first input
    /// that caused an invalidation to the present of the frame it caused.
    pub fn input_latency_snapshot(&self) -> InputLatencySnapshot {
        InputLatencySnapshot {
            latency_histogram: self.input_latency_histogram.clone(),
            events_per_frame_histogram: self.events_per_frame_histogram.clone(),
            mid_draw_events_dropped: self.mid_draw_events_dropped,
        }
    }

    /// Returns a copy of the frame-duration histograms, in nanoseconds.
    pub fn frame_duration_snapshot(&self) -> FrameDurationSnapshot {
        FrameDurationSnapshot {
            dirty_to_present_histogram: self.dirty_to_present_histogram.clone(),
            draw_duration_histogram: self.draw_duration_histogram.clone(),
            present_interval_histogram: self.present_interval_histogram.clone(),
        }
    }

    fn record_present_at(
        &mut self,
        present_start: Instant,
        present_end: Instant,
        window_active: bool,
        next_frame_scheduled: bool,
    ) {
        if let Some(first_input_at) = self.first_input_at.take() {
            let latency_nanos = present_end.duration_since(first_input_at).as_nanos() as u64;
            self.input_latency_histogram.record(latency_nanos).ok();
        }
        if self.pending_input_count > 0 {
            self.events_per_frame_histogram
                .record(self.pending_input_count)
                .ok();
            self.pending_input_count = 0;
        }

        let frame = self.pending_frame.take();
        let animation_interval =
            if frame.is_some() && self.animating_at_last_present && window_active {
                self.last_present_at
                    .map(|last_present_at| present_end.duration_since(last_present_at))
            } else {
                None
            };
        let present_timing = PresentTiming {
            window_id: self.window_id,
            present_start,
            present_end,
            animation_interval,
        };
        journal::record_present(present_timing, frame);

        let Some(frame) = frame else {
            return;
        };

        if let Some(dirty_at) = frame.dirty_at
            && let Err(error) = self
                .dirty_to_present_histogram
                .record(present_end.duration_since(dirty_at).as_nanos() as u64)
        {
            log::error!("failed to record dirty-to-present frame timing: {error}");
        }

        if let Some(animation_interval) = animation_interval {
            self.present_interval_histogram
                .record(animation_interval.as_nanos() as u64)
                .ok();
        }
        record_frame_event(FrameEvent::Present(present_timing));

        self.last_present_at = Some(present_end);
        self.animating_at_last_present = next_frame_scheduled && window_active;
    }

    fn record_draw_timing(&mut self, timing: FrameTiming) {
        self.record_draw_duration(timing.draw_duration());
        self.pending_frame = Some(timing);
        record_frame_event(FrameEvent::Draw(timing));
        journal::record_draw(timing);
    }

    fn record_draw_duration(&mut self, duration: Duration) {
        self.draw_duration_histogram
            .record(duration.as_nanos() as u64)
            .ok();
    }
}

#[cfg(feature = "profiler")]
impl Drop for WindowProfiler {
    fn drop(&mut self) {
        journal::record_window_closed(self.window_id);
    }
}

// Allow 16MiB of frame event entries.
#[cfg(feature = "profiler")]
const MAX_FRAME_TIMINGS: usize = (16 * 1024 * 1024) / core::mem::size_of::<FrameEvent>();

#[cfg(feature = "profiler")]
struct FrameTimings {
    timings: VecDeque<FrameEvent>,
    total_pushed: u64,
}

#[cfg(feature = "profiler")]
static FRAME_TIMINGS: spin::Mutex<FrameTimings> = spin::Mutex::new(FrameTimings {
    timings: VecDeque::new(),
    total_pushed: 0,
});

/// Adds a frame event to the global trace buffer.
///
/// Does nothing unless tracing is enabled with [`set_trace_enabled`]. Available with the
/// `profiler` feature.
#[cfg(feature = "profiler")]
pub fn record_frame_event(event: FrameEvent) {
    if !trace_enabled() {
        return;
    }
    cold_path(); // optimize for when profiling is off

    let mut frames = FRAME_TIMINGS.lock();
    if frames.timings.len() >= MAX_FRAME_TIMINGS {
        frames.timings.pop_front();
    }
    frames.timings.push_back(event);
    frames.total_pushed += 1;
}

/// Reads the frame events recorded after this collector was created.
///
/// Each call to [`Self::collect_unseen`] returns only the events recorded since the
/// previous call. Available with the `profiler` feature.
#[cfg(feature = "profiler")]
pub struct FrameTimingCollector {
    cursor: u64,
}

#[cfg(feature = "profiler")]
impl Default for FrameTimingCollector {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "profiler")]
impl FrameTimingCollector {
    /// Creates a collector that only sees frame events recorded from this point on.
    pub fn new() -> Self {
        Self {
            cursor: FRAME_TIMINGS.lock().total_pushed,
        }
    }

    /// Returns frame events recorded since the previous call (or since the
    /// collector was created). If the ring buffer wrapped around since the
    /// previous poll, the evicted entries are lost.
    pub fn collect_unseen(&mut self) -> Vec<FrameEvent> {
        let frames = FRAME_TIMINGS.lock();
        let buffer_len = frames.timings.len() as u64;
        let buffer_start = frames.total_pushed.saturating_sub(buffer_len);
        let skip = self.cursor.saturating_sub(buffer_start) as usize;
        let unseen = frames
            .timings
            .iter()
            .skip(skip.min(frames.timings.len()))
            .copied()
            .collect();
        self.cursor = frames.total_pushed;
        unseen
    }
}

#[cfg(all(test, feature = "profiler"))]
mod tests {
    use super::*;
    use std::sync::{Mutex, MutexGuard};

    #[test]
    fn long_poll_sample_is_ranked_by_poll_time_not_wait_time() {
        let spawned = Instant::now();
        let mut stats = TaskStatistics::default();
        for microseconds in [500, 100, 400, 200, 300, 50, 350, 150, 600] {
            // Every task waited 10 seconds to start, which must not affect its ranking.
            let start = spawned + Duration::from_secs(10);
            stats.add_yield_timing(TaskTiming {
                location: std::panic::Location::caller(),
                spawned: SpawnTime(spawned),
                start,
                end: YieldTime(start + Duration::from_micros(microseconds)),
            });
        }

        let mut retained = stats.longest_poll_times.map(|task| task.poll_duration());
        retained.sort();
        assert_eq!(
            retained,
            [300, 350, 400, 500, 600].map(Duration::from_micros)
        );
        assert_eq!(stats.poll_time_to_beat, Duration::from_micros(300));
    }

    #[test]
    fn short_polls_do_not_fill_unused_long_poll_slots() {
        let spawned = Instant::now();
        let mut stats = TaskStatistics::default();
        for microseconds in [200, 50, 100, 20] {
            stats.add_yield_timing(TaskTiming {
                location: std::panic::Location::caller(),
                spawned: SpawnTime(spawned),
                start: spawned,
                end: YieldTime(spawned + Duration::from_micros(microseconds)),
            });
        }

        assert_eq!(stats.poll_time_to_beat, Duration::from_micros(100));
        let retained = stats
            .longest_poll_times
            .iter()
            .filter(|task| !task.poll_duration().is_zero())
            .count();
        assert_eq!(retained, 2, "only the 200 and 100 microsecond polls count");
    }

    #[test]
    fn records_draw_events_only_while_tracing() {
        let _trace_test_guard = TraceTestGuard::new();
        let window_id = WindowId::from(0xD0A0);
        let mut window_profiler =
            WindowProfiler::new(window_id).expect("window profiler should initialize");
        let dirty_at = Instant::now();
        let mut collector = FrameTimingCollector::new();

        window_profiler.begin_draw();
        window_profiler.end_draw(Some(dirty_at), 3);
        assert!(
            collector
                .collect_unseen()
                .iter()
                .all(|event| !event_matches_window(*event, window_id))
        );

        set_trace_enabled(true);
        let mut collector = FrameTimingCollector::new();
        window_profiler.begin_draw();
        window_profiler.end_draw(Some(dirty_at), 3);

        let timing = collector
            .collect_unseen()
            .into_iter()
            .find_map(|event| match event {
                FrameEvent::Draw(timing) if timing.window_id == window_id => Some(timing),
                _ => None,
            })
            .expect("draw event should be recorded while tracing");
        assert_eq!(timing.dirty_at, Some(dirty_at));
        assert_eq!(timing.invalidations, 3);
        assert!(timing.draw_start >= dirty_at);
    }

    #[test]
    fn records_present_events_for_newly_drawn_frames() {
        let _trace_test_guard = TraceTestGuard::new();
        set_trace_enabled(true);
        let window_id = WindowId::from(0xA11E);
        let mut window_profiler =
            WindowProfiler::new(window_id).expect("window profiler should initialize");
        let start = Instant::now();
        let mut collector = FrameTimingCollector::new();

        record_test_draw(&mut window_profiler, start);
        window_profiler.record_present_at(start, start, true, true);
        record_test_draw(&mut window_profiler, start + FRAME);
        window_profiler.record_present_at(start + FRAME, start + FRAME, true, true);
        window_profiler.record_present_at(
            start + FRAME + FRAME / 2,
            start + FRAME + FRAME / 2,
            true,
            true,
        );

        let present_timings = collector
            .collect_unseen()
            .into_iter()
            .filter_map(|event| match event {
                FrameEvent::Present(timing) if timing.window_id == window_id => Some(timing),
                _ => None,
            })
            .collect::<Vec<_>>();
        let [first_present, second_present] = present_timings.as_slice() else {
            panic!("expected exactly two present events, got {present_timings:?}");
        };
        assert_eq!(first_present.animation_interval, None);
        assert_eq!(second_present.animation_interval, Some(FRAME));

        #[cfg(feature = "profiler")]
        {
            assert_eq!(window_profiler.present_interval_histogram.len(), 1);
            assert!(
                window_profiler.present_interval_histogram.max()
                    >= second_present
                        .animation_interval
                        .expect("second present should have an animation interval")
                        .as_nanos() as u64
            );
        }
    }

    #[test]
    fn disabling_tracing_clears_frame_events() {
        let _trace_test_guard = TraceTestGuard::new();
        set_trace_enabled(true);
        let window_id = WindowId::from(0xC1EA);
        let mut window_profiler =
            WindowProfiler::new(window_id).expect("window profiler should initialize");
        let mut collector = FrameTimingCollector::new();

        window_profiler.begin_draw();
        window_profiler.end_draw(None, 0);
        assert!(
            FRAME_TIMINGS
                .lock()
                .timings
                .iter()
                .copied()
                .any(|event| event_matches_window(event, window_id))
        );

        set_trace_enabled(false);
        assert!(
            collector
                .collect_unseen()
                .iter()
                .all(|event| !event_matches_window(*event, window_id))
        );
    }

    #[cfg(feature = "profiler")]
    #[test]
    fn records_intervals_only_between_animation_frames() {
        let mut window_profiler =
            WindowProfiler::new(WindowId::from(1)).expect("window profiler should initialize");
        let start = Instant::now();

        draw_and_present(&mut window_profiler, start, true, true);
        assert_eq!(window_profiler.present_interval_histogram.len(), 0);

        draw_and_present(&mut window_profiler, start + FRAME, true, true);
        assert_eq!(window_profiler.present_interval_histogram.len(), 1);

        draw_and_present(&mut window_profiler, start + FRAME * 2, true, false);
        assert_eq!(window_profiler.present_interval_histogram.len(), 2);

        draw_and_present(&mut window_profiler, start + FRAME * 100, true, true);
        assert_eq!(window_profiler.present_interval_histogram.len(), 2);
    }

    #[cfg(feature = "profiler")]
    #[test]
    fn missed_frames_stretch_the_recorded_interval() {
        let mut window_profiler =
            WindowProfiler::new(WindowId::from(2)).expect("window profiler should initialize");
        let start = Instant::now();

        draw_and_present(&mut window_profiler, start, true, true);
        draw_and_present(&mut window_profiler, start + FRAME * 5, true, true);

        let recorded = window_profiler.present_interval_histogram.max();
        assert!(recorded >= (FRAME * 4).as_nanos() as u64);
    }

    #[cfg(feature = "profiler")]
    #[test]
    fn ignores_re_presents_of_unchanged_frames() {
        let mut window_profiler =
            WindowProfiler::new(WindowId::from(3)).expect("window profiler should initialize");
        let start = Instant::now();

        draw_and_present(&mut window_profiler, start, true, true);
        window_profiler.record_present_at(start + FRAME / 2, start + FRAME / 2, true, true);
        draw_and_present(&mut window_profiler, start + FRAME, true, true);

        assert_eq!(window_profiler.present_interval_histogram.len(), 1);
        assert!(
            window_profiler.present_interval_histogram.max() >= (FRAME * 3 / 4).as_nanos() as u64
        );
    }

    #[cfg(feature = "profiler")]
    #[test]
    fn skips_intervals_for_inactive_windows() {
        let mut window_profiler =
            WindowProfiler::new(WindowId::from(4)).expect("window profiler should initialize");
        let start = Instant::now();

        draw_and_present(&mut window_profiler, start, false, true);
        draw_and_present(&mut window_profiler, start + FRAME, false, true);
        assert_eq!(window_profiler.present_interval_histogram.len(), 0);

        draw_and_present(&mut window_profiler, start + FRAME * 2, true, true);
        assert_eq!(window_profiler.present_interval_histogram.len(), 0);

        draw_and_present(&mut window_profiler, start + FRAME * 3, true, true);
        assert_eq!(window_profiler.present_interval_histogram.len(), 1);
    }

    #[test]
    fn records_dirty_to_present_durations() {
        let mut window_profiler =
            WindowProfiler::new(WindowId::from(8)).expect("window profiler should initialize");
        let draw_end = Instant::now();
        let present_end = draw_end + Duration::from_millis(6);

        record_test_draw(&mut window_profiler, draw_end);
        window_profiler.record_present_at(present_end, present_end, true, false);

        let snapshot = window_profiler.frame_duration_snapshot();
        let histogram = snapshot.dirty_to_present_histogram;
        assert_eq!(histogram.len(), 1);
        assert!(histogram.max() >= Duration::from_millis(10).as_nanos() as u64);
    }

    #[cfg(feature = "profiler")]
    #[test]
    fn records_every_draw_duration() {
        let mut window_profiler =
            WindowProfiler::new(WindowId::from(5)).expect("window profiler should initialize");

        window_profiler.record_draw_duration(Duration::from_millis(2));
        window_profiler.record_draw_duration(Duration::from_millis(40));

        let snapshot = window_profiler.frame_duration_snapshot();
        assert_eq!(snapshot.draw_duration_histogram.len(), 2);
        assert!(snapshot.draw_duration_histogram.max() >= 39_000_000);
    }

    #[test]
    fn records_input_latency_at_the_frame_presentation_timestamp() {
        let mut window_profiler =
            WindowProfiler::new(WindowId::from(6)).expect("window profiler should initialize");
        let first_input_at = Instant::now();
        let presented_at = first_input_at + Duration::from_millis(12);

        begin_input_at(&mut window_profiler, first_input_at);
        window_profiler.end_input(true);
        begin_input_at(
            &mut window_profiler,
            first_input_at + Duration::from_millis(2),
        );
        window_profiler.end_input(true);
        record_test_draw(&mut window_profiler, presented_at);
        window_profiler.record_present_at(presented_at, presented_at, true, false);

        let snapshot = window_profiler.input_latency_snapshot();
        assert_eq!(snapshot.latency_histogram.len(), 1);
        assert!(snapshot.latency_histogram.max() >= Duration::from_millis(12).as_nanos() as u64);
        assert_eq!(snapshot.events_per_frame_histogram.len(), 1);
        assert_eq!(snapshot.events_per_frame_histogram.max(), 2);
        assert_eq!(snapshot.mid_draw_events_dropped, 0);
    }

    #[test]
    fn excludes_input_that_arrives_during_a_draw() {
        let mut window_profiler =
            WindowProfiler::new(WindowId::from(7)).expect("window profiler should initialize");

        window_profiler.begin_draw();
        begin_input_at(&mut window_profiler, Instant::now());
        window_profiler.end_input(true);
        window_profiler.end_draw(None, 0);

        let snapshot = window_profiler.input_latency_snapshot();
        assert!(snapshot.latency_histogram.is_empty());
        assert!(snapshot.events_per_frame_histogram.is_empty());
        assert_eq!(snapshot.mid_draw_events_dropped, 1);
    }

    #[test]
    fn overlapping_trace_scopes_keep_tracing_enabled() {
        let _trace_test_guard = TraceTestGuard::new();
        let first_scope = trace_scope();
        let second_scope = trace_scope();

        assert!(trace_enabled());
        drop(first_scope);
        assert!(trace_enabled());
        drop(second_scope);
        assert!(!trace_enabled());
    }

    const FRAME: Duration = Duration::from_millis(16);
    static TRACE_TEST_LOCK: Mutex<()> = Mutex::new(());

    struct TraceTestGuard {
        was_enabled: bool,
        _lock: MutexGuard<'static, ()>,
    }

    impl TraceTestGuard {
        fn new() -> Self {
            let lock = TRACE_TEST_LOCK
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let was_enabled = trace_enabled();
            set_trace_enabled(false);
            Self {
                was_enabled,
                _lock: lock,
            }
        }
    }

    impl Drop for TraceTestGuard {
        fn drop(&mut self) {
            set_trace_enabled(false);
            if self.was_enabled {
                set_trace_enabled(true);
            }
        }
    }

    fn event_matches_window(event: FrameEvent, window_id: WindowId) -> bool {
        match event {
            FrameEvent::Draw(timing) => timing.window_id == window_id,
            FrameEvent::Present(timing) => timing.window_id == window_id,
        }
    }

    fn begin_input_at(window_profiler: &mut WindowProfiler, started_at: Instant) {
        window_profiler
            .active_activities
            .push(WindowActivity::Input {
                started_at,
                kind: "test",
            });
    }

    #[cfg(feature = "profiler")]
    fn draw_and_present(
        window_profiler: &mut WindowProfiler,
        presented_at: Instant,
        window_active: bool,
        next_frame_scheduled: bool,
    ) {
        record_test_draw(window_profiler, presented_at);
        window_profiler.record_present_at(
            presented_at,
            presented_at,
            window_active,
            next_frame_scheduled,
        );
    }

    fn record_test_draw(window_profiler: &mut WindowProfiler, draw_end: Instant) {
        window_profiler.record_draw_timing(FrameTiming {
            window_id: window_profiler.window_id,
            dirty_at: Some(draw_end - Duration::from_millis(4)),
            invalidations: 1,
            draw_start: draw_end - Duration::from_millis(2),
            draw_end,
        });
    }
}
