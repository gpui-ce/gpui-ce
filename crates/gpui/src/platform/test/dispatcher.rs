use crate::{PlatformDispatcher, Priority, RunnableVariant};
use scheduler::Instant;
use scheduler::{Clock, Scheduler, SessionId, TestScheduler, TestSchedulerConfig, Yield};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

/// Runs all async work on one thread, in a seeded order, with a virtual clock.
///
/// Tests get deterministic scheduling from this type. The order of tasks comes from the seed,
/// and time moves only when the test advances it. [`TestApp`](crate::TestApp) creates one for
/// you, so you rarely construct it directly. Reach the clock and random numbers through
/// [`Self::scheduler`].
///
/// Background and foreground work share the scheduler. A timer does not fire
/// until the virtual clock reaches its deadline; the dispatcher does not
/// implement `dispatch_after`. Use [`BackgroundExecutor::timer`](crate::BackgroundExecutor::timer),
/// which schedules through the test scheduler.
#[doc(hidden)]
pub struct TestDispatcher {
    session_id: SessionId,
    scheduler: Arc<TestScheduler>,
    num_cpus_override: Arc<AtomicUsize>,
}

impl TestDispatcher {
    /// Creates a dispatcher whose random choices follow `seed`.
    ///
    /// The same seed gives the same task order. Set `PENDING_TRACES` to `1` or
    /// `true` to make the scheduler capture traces of pending tasks.
    pub fn new(seed: u64) -> Self {
        let scheduler = Arc::new(TestScheduler::new(TestSchedulerConfig {
            seed,
            randomize_order: true,
            allow_parking: false,
            capture_pending_traces: std::env::var("PENDING_TRACES")
                .map_or(false, |var| var == "1" || var == "true"),
            timeout_ticks: 0..=1000,
        }));
        Self::from_scheduler(scheduler)
    }

    /// Creates a dispatcher that runs on an existing `scheduler`.
    ///
    /// Use it to give several dispatchers one clock and one random number generator. The
    /// dispatcher gets its own session id.
    pub fn from_scheduler(scheduler: Arc<TestScheduler>) -> Self {
        TestDispatcher {
            session_id: scheduler.allocate_session_id(),
            scheduler,
            num_cpus_override: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Returns the underlying scheduler, for direct control of its clock, random numbers and
    /// parking.
    pub fn scheduler(&self) -> &Arc<TestScheduler> {
        &self.scheduler
    }

    /// Returns the session id that this dispatcher's foreground tasks run under.
    ///
    /// Each clone of a dispatcher has its own session id.
    pub fn session_id(&self) -> SessionId {
        self.session_id
    }

    /// Drops every queued task and timer without running any of them.
    ///
    /// The leak detector calls this at the end of a test, because a queued task can keep an
    /// entity alive. Tasks you drop this way never complete.
    pub fn drain_tasks(&self) {
        self.scheduler.drain_tasks();
    }

    /// Moves the virtual clock forward by `by`.
    ///
    /// Tasks that are ready run, and timers whose due time falls within the interval fire, in
    /// due-time order, as the clock passes them. A timer due after the interval doesn't fire.
    pub fn advance_clock(&self, by: Duration) {
        self.scheduler.advance_clock(by);
    }

    /// Moves the virtual clock to when the next timer is due.
    ///
    /// Returns `false`, and leaves the clock alone, when no timer is armed. This doesn't run any
    /// tasks. Call [`Self::run_until_parked`] afterwards to run the timer's work.
    pub fn advance_clock_to_next_timer(&self) -> bool {
        self.scheduler.advance_clock_to_next_timer()
    }

    /// Returns a random `Yield` amount, drawn from the seeded random number generator.
    ///
    /// Most draws are 0 or 1. About one in ten is between 10 and 19. Use it to inject variable
    /// delays into a test that should tolerate them.
    pub fn simulate_random_delay(&self) -> Yield {
        self.scheduler.yield_random()
    }

    /// Runs one ready task, and returns whether any work was done.
    ///
    /// With `background_only` set to `true`, foreground tasks are skipped. Timers that aren't due
    /// yet don't fire. This doesn't advance the clock.
    pub fn tick(&self, background_only: bool) -> bool {
        if background_only {
            self.scheduler.tick_background_only()
        } else {
            self.scheduler.tick()
        }
    }

    /// Runs ready tasks until none is left, without advancing the clock.
    ///
    /// Timers that aren't due yet stay armed. Use [`Self::advance_clock`] to make time pass.
    pub fn run_until_parked(&self) {
        while self.tick(false) {}
    }

    /// Lets the scheduler wait in real time when nothing is ready, so another thread can wake a task.
    ///
    /// While parking is allowed, the clock doesn't jump to the next timer. Use it when a test
    /// waits on work from a real thread. [`Self::forbid_parking`] restores the default.
    pub fn allow_parking(&self) {
        self.scheduler.allow_parking();
    }

    /// Restores the default, which forbids waiting in real time.
    ///
    /// With parking forbidden, the scheduler jumps the virtual clock to the next
    /// timer when nothing is ready. Activity on another thread then fails the
    /// test because it would make the schedule nondeterministic.
    pub fn forbid_parking(&self) {
        self.scheduler.forbid_parking();
    }

    /// Overrides the value that [`BackgroundExecutor::num_cpus`](crate::BackgroundExecutor::num_cpus) returns in tests.
    ///
    /// A value of 0 removes the override, and `num_cpus` then returns 4.
    pub fn set_num_cpus(&self, count: usize) {
        self.num_cpus_override.store(count, Ordering::SeqCst);
    }

    /// Returns the CPU count set with [`Self::set_num_cpus`], or `None` if no override is set.
    pub fn num_cpus_override(&self) -> Option<usize> {
        match self.num_cpus_override.load(Ordering::SeqCst) {
            0 => None,
            n => Some(n),
        }
    }
}

impl Clone for TestDispatcher {
    fn clone(&self) -> Self {
        let session_id = self.scheduler.allocate_session_id();
        Self {
            session_id,
            scheduler: self.scheduler.clone(),
            num_cpus_override: self.num_cpus_override.clone(),
        }
    }
}

impl PlatformDispatcher for TestDispatcher {
    fn is_main_thread(&self) -> bool {
        self.scheduler.is_main_thread()
    }

    fn now(&self) -> Instant {
        self.scheduler.clock().now()
    }

    fn dispatch(&self, runnable: RunnableVariant, priority: Priority) {
        self.scheduler
            .schedule_background_with_priority(runnable, priority);
    }

    fn dispatch_on_main_thread(&self, runnable: RunnableVariant, _priority: Priority) {
        self.scheduler.schedule_local(self.session_id, runnable);
    }

    fn dispatch_after(&self, _duration: Duration, _runnable: RunnableVariant) {
        panic!(
            "dispatch_after should not be called in tests. \
            Use BackgroundExecutor::timer() which uses the scheduler's native timer."
        );
    }

    fn as_test(&self) -> Option<&TestDispatcher> {
        Some(self)
    }

    fn spawn_realtime(&self, f: Box<dyn FnOnce() + Send>) {
        std::thread::spawn(move || {
            f();
        });
    }
}
