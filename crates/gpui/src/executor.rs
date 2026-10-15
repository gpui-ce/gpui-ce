use crate::{App, PlatformDispatcher, PlatformScheduler};
#[cfg(not(target_family = "wasm"))]
use futures::channel::mpsc;
use futures::prelude::*;
use gpui_util::{TryFutureExt, TryFutureExtBacktrace};
use scheduler::Instant;
use scheduler::Scheduler;
use std::{future::Future, marker::PhantomData, rc::Rc, sync::Arc, time::Duration};
#[cfg(not(target_family = "wasm"))]
use std::{mem, pin::Pin};

pub use scheduler::{
    DedicatedExecutor, FallibleTask, LocalExecutor as SchedulerLocalExecutor, Priority, Task,
};

/// Runs `Send` futures on GPUI's background executor.
///
/// Get one from [`App::background_executor`] or [`crate::AsyncApp::background_executor`].
/// Its futures must be `Send` and `'static`, so they cannot borrow entities or contexts. Pass in
/// plain data and return plain data. Native backends use worker threads; a single-threaded
/// web build runs these futures on the browser's main thread. See
/// [executors](crate::_documentation::concepts::executors).
#[derive(Clone)]
pub struct BackgroundExecutor {
    inner: scheduler::BackgroundExecutor,
    dispatcher: Arc<dyn PlatformDispatcher>,
}

/// Runs futures on the main thread, interleaved with event handling and drawing.
///
/// Get one from [`App::foreground_executor`] or [`crate::AsyncApp::foreground_executor`].
/// Futures need only be `'static`, so they can hold entity handles. Each poll
/// runs until the future returns `Pending` or `Ready`; an await can complete
/// immediately. Keep CPU-heavy work out of a foreground poll and move it to a
/// [`BackgroundExecutor`]. See
/// [executors](crate::_documentation::concepts::executors).
#[derive(Clone)]
pub struct ForegroundExecutor {
    inner: scheduler::LocalExecutor,
    dispatcher: Arc<dyn PlatformDispatcher>,
    #[cfg(feature = "profiler")]
    foreground_runnables: Option<crate::profiler::journal::ForegroundRunnableCounter>,
    not_send: PhantomData<Rc<()>>,
}

/// Extension methods for tasks that produce a [`Result`].
///
/// The methods log a failure instead of dropping it silently, so use them for work
/// that you start and don't wait for.
///
/// # Examples
///
/// ```
/// use gpui::{TaskExt, TestApp};
///
/// let mut app = TestApp::new();
/// app.update(|cx| {
///     let save = cx.background_executor().spawn(async { Err::<(), _>("disk full") });
///     save.detach_and_log_err(cx);
/// });
/// ```
pub trait TaskExt<T, E> {
    /// Runs the task to completion in the background. If it fails, the error is written
    /// to the log.
    fn detach_and_log_err(self, cx: &App);
    /// Like [`TaskExt::detach_and_log_err`], but logs the error with `Debug` formatting,
    /// so an `anyhow::Error` includes its backtrace. Use it when you need the backtrace,
    /// and the other method otherwise.
    fn detach_and_log_err_with_backtrace(self, cx: &App);
}

impl<T, E> TaskExt<T, E> for Task<Result<T, E>>
where
    T: 'static,
    E: 'static + std::fmt::Display + std::fmt::Debug,
{
    #[track_caller]
    fn detach_and_log_err(self, cx: &App) {
        let location = core::panic::Location::caller();
        cx.foreground_executor()
            .spawn(self.log_tracked_err(*location))
            .detach();
    }

    #[track_caller]
    fn detach_and_log_err_with_backtrace(self, cx: &App) {
        let location = *core::panic::Location::caller();
        cx.foreground_executor()
            .spawn(self.log_tracked_err_with_backtrace(location))
            .detach();
    }
}

impl BackgroundExecutor {
    /// Creates an executor that sends its work through `dispatcher`.
    ///
    /// Platform code creates the executor. Apps get one from [`App::background_executor`].
    pub fn new(dispatcher: Arc<dyn PlatformDispatcher>) -> Self {
        #[cfg(any(test, feature = "test-support"))]
        let scheduler: Arc<dyn Scheduler> = if let Some(test_dispatcher) = dispatcher.as_test() {
            test_dispatcher.scheduler().clone()
        } else {
            Arc::new(PlatformScheduler::new(dispatcher.clone()))
        };

        #[cfg(not(any(test, feature = "test-support")))]
        let scheduler: Arc<dyn Scheduler> = Arc::new(PlatformScheduler::new(dispatcher.clone()));

        Self {
            inner: scheduler::BackgroundExecutor::new(scheduler),
            dispatcher,
        }
    }

    /// Returns the underlying scheduler executor, for code that takes one directly.
    pub fn scheduler_executor(&self) -> scheduler::BackgroundExecutor {
        self.inner.clone()
    }

    /// Runs the future that `f` returns on a thread of its own, and returns a task for
    /// its result.
    ///
    /// Use this instead of [`BackgroundExecutor::spawn`] for work whose polls need more
    /// stack than the shared background threads provide. A dedicated thread gets the
    /// standard library's default stack of 2 MiB. Dropping the task cancels the work, as it
    /// does for any task.
    #[track_caller]
    pub fn spawn_dedicated<F, Fut>(&self, f: F) -> Task<Fut::Output>
    where
        F: FnOnce(SchedulerLocalExecutor) -> Fut + Send + 'static,
        Fut: Future + 'static,
        Fut::Output: Send + Sync + 'static,
    {
        self.inner.spawn_dedicated(f)
    }

    /// Schedules `future` on this executor and returns a task for its result.
    ///
    /// On native targets, use this for computation or blocking I/O that should leave
    /// the main thread free. On a single-threaded web build it uses the main thread,
    /// so each poll must remain short.
    /// The future must be `Send` and `'static`. To get the result on the main thread,
    /// await the task from a foreground future. Dropping the task cancels the future.
    ///
    /// # Examples
    ///
    /// ```
    /// use gpui::TestApp;
    ///
    /// let app = TestApp::new();
    /// let task = app.background_executor().spawn(async { 6 * 7 });
    /// assert_eq!(app.foreground_executor().block_test(task), 42);
    /// ```
    #[track_caller]
    pub fn spawn<R>(&self, future: impl Future<Output = R> + Send + 'static) -> Task<R>
    where
        R: Send + 'static,
    {
        self.spawn_with_priority(Priority::default(), future.boxed())
    }

    /// Like [`BackgroundExecutor::spawn`], but with a [`Priority`].
    ///
    /// Higher priorities are more likely to run first, but the order isn't guaranteed.
    /// [`Priority::RealtimeAudio`] runs the future on a dedicated thread with realtime
    /// scheduling. Use it only for audio work.
    #[track_caller]
    pub fn spawn_with_priority<R>(
        &self,
        priority: Priority,
        future: impl Future<Output = R> + Send + 'static,
    ) -> Task<R>
    where
        R: Send + 'static,
    {
        if priority == Priority::RealtimeAudio {
            self.inner.spawn_realtime(future)
        } else {
            self.inner.spawn_with_priority(priority, future)
        }
    }

    /// Runs futures that may borrow from the caller in parallel, and waits for all of
    /// them to finish.
    ///
    /// `scheduler` receives a [`Scope`] and spawns the futures on it. Unlike
    /// [`BackgroundExecutor::spawn`], those futures needn't be `'static`, because the
    /// returned future doesn't complete until they have. Dropping the returned future
    /// cancels the futures, and blocks until they are destroyed. If `scheduler` panics, the
    /// futures that haven't started are dropped, and the panic unwinds without hanging.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::sync::atomic::{AtomicUsize, Ordering};
    /// use gpui::TestApp;
    ///
    /// let app = TestApp::new();
    /// let total = AtomicUsize::new(0);
    /// app.foreground_executor().block_test(app.background_executor().scoped(|scope| {
    ///     for n in 1..=3 {
    ///         let total = &total;
    ///         scope.spawn(async move {
    ///             total.fetch_add(n, Ordering::SeqCst);
    ///         });
    ///     }
    /// }));
    /// assert_eq!(total.into_inner(), 6);
    /// ```
    #[cfg(not(target_family = "wasm"))]
    pub async fn scoped<'scope, F>(&self, scheduler: F)
    where
        F: FnOnce(&mut Scope<'scope>),
    {
        let mut scope = Scope::new(self.clone(), Priority::default());
        (scheduler)(&mut scope);
        let spawned = mem::take(&mut scope.futures)
            .into_iter()
            .map(|f| self.spawn_with_priority(scope.priority, f))
            .collect::<Vec<_>>();
        for task in spawned {
            task.await;
        }
    }

    /// Like [`BackgroundExecutor::scoped`], but runs every future in the scope at
    /// `priority`.
    #[cfg(not(target_family = "wasm"))]
    pub async fn scoped_priority<'scope, F>(&self, priority: Priority, scheduler: F)
    where
        F: FnOnce(&mut Scope<'scope>),
    {
        let mut scope = Scope::new(self.clone(), priority);
        (scheduler)(&mut scope);
        let spawned = mem::take(&mut scope.futures)
            .into_iter()
            .map(|f| self.spawn_with_priority(scope.priority, f))
            .collect::<Vec<_>>();
        for task in spawned {
            task.await;
        }
    }

    /// Returns the current time.
    ///
    /// Use this in place of `std::time::Instant::now` so that tests can control time. In
    /// tests it reads the simulated clock, which moves only when the test advances it.
    pub fn now(&self) -> Instant {
        self.inner.scheduler().clock().now()
    }

    /// Returns a task that completes after `duration`.
    ///
    /// Await it to delay work or to repeat it on an interval. In tests, time moves only
    /// when the test calls [`BackgroundExecutor::advance_clock`]. Other work can delay
    /// the wake-up, so the real wait can be longer than `duration`. A zero duration
    /// returns a task that is already complete.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::time::Duration;
    /// use gpui::TestApp;
    ///
    /// let app = TestApp::new();
    /// let timer = app.background_executor().timer(Duration::from_millis(500));
    /// app.advance_clock(Duration::from_millis(500));
    /// assert!(timer.is_ready());
    /// ```
    #[track_caller]
    pub fn timer(&self, duration: Duration) -> Task<()> {
        if duration.is_zero() {
            return Task::ready(());
        }
        self.spawn(self.inner.scheduler().timer(duration))
    }

    /// Yields a random number of times, allowing other ready tasks to run between steps.
    ///
    /// Await it between steps of a test to shake out ordering bugs. Only available in
    /// tests.
    ///
    /// # Panics
    ///
    /// Panics if this executor isn't a test executor.
    #[cfg(any(test, feature = "test-support"))]
    pub fn simulate_random_delay(&self) -> impl Future<Output = ()> + use<> {
        self.dispatcher.as_test().unwrap().simulate_random_delay()
    }

    /// Moves the simulated clock forward by `duration`.
    ///
    /// Ready tasks run as the clock passes each timer, in time order, so a task that
    /// waits on a timer resumes at the right moment. When the call returns, the clock
    /// reads its earlier value plus `duration`. Only available in tests.
    ///
    /// # Panics
    ///
    /// Panics if this executor isn't a test executor.
    #[cfg(any(test, feature = "test-support"))]
    pub fn advance_clock(&self, duration: Duration) {
        self.dispatcher.as_test().unwrap().advance_clock(duration)
    }

    /// Runs one unit of ready work in tests.
    ///
    /// Returns `true` if it ran a task or fired a timer, and `false` if there was nothing
    /// to do. Only available in tests.
    ///
    /// # Panics
    ///
    /// Panics if this executor isn't a test executor.
    #[cfg(any(test, feature = "test-support"))]
    pub fn tick(&self) -> bool {
        self.dispatcher.as_test().unwrap().scheduler().tick()
    }

    /// Runs ready tasks in tests until none can make progress.
    ///
    /// This doesn't move the simulated clock. A timer that hasn't expired stays pending,
    /// until [`BackgroundExecutor::advance_clock`] moves time past it. Only available in
    /// tests.
    #[cfg(any(test, feature = "test-support"))]
    pub fn run_until_parked(&self) {
        let scheduler = self.dispatcher.as_test().unwrap().scheduler();
        scheduler.run();
    }

    /// Lets blocking waits in tests sleep in real time when no test work can make progress.
    ///
    /// By default, a blocking call that cannot progress panics. With parking allowed, it waits
    /// in real time instead, and the test fails if the wait lasts 15 seconds. Use this for tests
    /// that wait on real I/O. Undo it with [`BackgroundExecutor::forbid_parking`].
    #[cfg(any(test, feature = "test-support"))]
    pub fn allow_parking(&self) {
        self.dispatcher
            .as_test()
            .unwrap()
            .scheduler()
            .allow_parking();

        if std::env::var("GPUI_RUN_UNTIL_PARKED_LOG").ok().as_deref() == Some("1") {
            log::warn!("[gpui::executor] allow_parking: enabled");
        }
    }

    /// Sets the range that the tick budget of a [`ForegroundExecutor::block_with_timeout`]
    /// call is drawn from, in tests.
    ///
    /// The default is 1 to 1000 ticks. Only available in tests.
    #[cfg(any(test, feature = "test-support"))]
    pub fn set_block_on_ticks(&self, range: std::ops::RangeInclusive<usize>) {
        self.dispatcher
            .as_test()
            .unwrap()
            .scheduler()
            .set_timeout_ticks(range);
    }

    /// Undoes [`BackgroundExecutor::allow_parking`], so a blocking call that can't
    /// progress panics again. Only available in tests.
    #[cfg(any(test, feature = "test-support"))]
    pub fn forbid_parking(&self) {
        self.dispatcher
            .as_test()
            .unwrap()
            .scheduler()
            .forbid_parking();
    }

    /// Returns the seeded random number generator that the test scheduler uses.
    ///
    /// Draw from it when a test makes its own random choices, so that the choices
    /// repeat with the same `SEED`. Only available in tests.
    #[cfg(any(test, feature = "test-support"))]
    pub fn rng(&self) -> scheduler::SharedRng {
        self.dispatcher.as_test().unwrap().scheduler().rng()
    }

    /// Returns the number of CPUs the dispatcher reports.
    ///
    /// In tests this is 4 unless it was changed with [`BackgroundExecutor::set_num_cpus`].
    pub fn num_cpus(&self) -> usize {
        #[cfg(any(test, feature = "test-support"))]
        if let Some(test) = self.dispatcher.as_test() {
            return test.num_cpus_override().unwrap_or(4);
        }
        num_cpus::get()
    }

    /// Sets the value that [`BackgroundExecutor::num_cpus`] returns, in tests.
    ///
    /// # Panics
    ///
    /// Panics if this executor isn't a test executor.
    #[cfg(any(test, feature = "test-support"))]
    pub fn set_num_cpus(&self, count: usize) {
        self.dispatcher
            .as_test()
            .expect("set_num_cpus can only be called on a test executor")
            .set_num_cpus(count);
    }

    /// Returns whether the current thread is the main thread.
    pub fn is_main_thread(&self) -> bool {
        self.dispatcher.is_main_thread()
    }

    /// Returns the platform dispatcher this executor sends work through. GPUI uses this
    /// internally.
    #[doc(hidden)]
    pub fn dispatcher(&self) -> &Arc<dyn PlatformDispatcher> {
        &self.dispatcher
    }
}

impl ForegroundExecutor {
    /// Creates an executor that runs futures on the main thread through `dispatcher`.
    ///
    /// Platform code creates the executor. Apps get one from
    /// [`App::foreground_executor`] or [`crate::AsyncApp::foreground_executor`].
    pub fn new(dispatcher: Arc<dyn PlatformDispatcher>) -> Self {
        #[cfg(any(test, feature = "test-support"))]
        let (scheduler, session_id): (Arc<dyn Scheduler>, _) =
            if let Some(test_dispatcher) = dispatcher.as_test() {
                (
                    test_dispatcher.scheduler().clone(),
                    test_dispatcher.session_id(),
                )
            } else {
                let platform_scheduler = Arc::new(PlatformScheduler::new(dispatcher.clone()));
                let inner = platform_scheduler.foreground_executor();
                return Self {
                    inner,
                    dispatcher,
                    #[cfg(feature = "profiler")]
                    foreground_runnables: Some(platform_scheduler.foreground_runnable_counter()),
                    not_send: PhantomData,
                };
            };

        #[cfg(not(any(test, feature = "test-support")))]
        let platform_scheduler = Arc::new(PlatformScheduler::new(dispatcher.clone()));
        #[cfg(not(any(test, feature = "test-support")))]
        let inner = platform_scheduler.foreground_executor();
        #[cfg(all(not(any(test, feature = "test-support")), feature = "profiler"))]
        let foreground_runnables = Some(platform_scheduler.foreground_runnable_counter());

        #[cfg(any(test, feature = "test-support"))]
        let inner = {
            let scheduler_for_dispatch = Arc::downgrade(&scheduler);
            scheduler::LocalExecutor::new(session_id, scheduler, move |runnable| {
                if let Some(scheduler) = scheduler_for_dispatch.upgrade() {
                    scheduler.schedule_local(session_id, runnable);
                }
            })
        };

        #[cfg(all(any(test, feature = "test-support"), feature = "profiler"))]
        // The deterministic test scheduler does not invoke GPUI's task profiler
        // hooks, so an increment here would have no matching decrement.
        let foreground_runnables = None;

        Self {
            inner,
            dispatcher,
            #[cfg(feature = "profiler")]
            foreground_runnables,
            not_send: PhantomData,
        }
    }

    /// Runs `future` on the main thread, and returns a task for its result.
    ///
    /// The future needn't be `Send`, so it can hold entity handles and other state that
    /// lives on the main thread. Each poll runs until the future returns `Pending` or
    /// `Ready`; an await can complete immediately. Keep long computations out of a poll
    /// and use [`BackgroundExecutor::spawn`] for them. Dropping the task cancels the
    /// future, but cannot interrupt synchronous code already running on this thread.
    ///
    /// # Examples
    ///
    /// ```
    /// use gpui::TestApp;
    ///
    /// let app = TestApp::new();
    /// let answer = app.foreground_executor().spawn(async { 6 * 7 });
    /// assert_eq!(app.foreground_executor().block_test(answer), 42);
    /// ```
    #[track_caller]
    pub fn spawn<R>(&self, future: impl Future<Output = R> + 'static) -> Task<R>
    where
        R: 'static,
    {
        self.inner.spawn(future.boxed_local())
    }

    /// Like [`ForegroundExecutor::spawn`]. The priority is ignored: foreground futures run
    /// in the order they were spawned.
    #[track_caller]
    pub fn spawn_with_priority<R>(
        &self,
        _priority: Priority,
        future: impl Future<Output = R> + 'static,
    ) -> Task<R>
    where
        R: 'static,
    {
        // Priority is ignored for foreground tasks - they run in order on the main thread
        self.inner.spawn(future)
    }

    /// Runs `future` on the main thread when the platform is idle.
    ///
    /// Use this for low-priority UI work that can wait. `timeout` sets how long the
    /// future may wait. `None` lets it wait as long as the platform stays busy. With a
    /// timeout, a poll that is still waiting after that long runs as ordinary main-thread
    /// work. On platforms without idle scheduling the future runs at low priority, and
    /// `timeout` is ignored.
    ///
    /// Each poll uses part of one idle slice. Long synchronous stretches should check
    /// [`ForegroundExecutor::idle_time_remaining`] and yield when it runs low.
    #[track_caller]
    pub fn spawn_when_idle<R>(
        &self,
        timeout: Option<Duration>,
        future: impl Future<Output = R> + 'static,
    ) -> Task<R>
    where
        R: 'static,
    {
        let dispatcher = self.dispatcher.clone();
        #[cfg(feature = "profiler")]
        let foreground_runnables = self.foreground_runnables.clone();
        self.inner
            .spawn_with_dispatch(future.boxed_local(), move |runnable| {
                #[cfg(feature = "profiler")]
                if let Some(foreground_runnables) = &foreground_runnables {
                    foreground_runnables.queued();
                }
                dispatcher.dispatch_on_main_thread_when_idle(runnable, timeout);
            })
    }

    /// Returns how much of the current idle slice is left.
    ///
    /// Call it from a future that was spawned with
    /// [`ForegroundExecutor::spawn_when_idle`]. Returns `None` on platforms that don't
    /// meter idle time, and when the call isn't inside an idle slice. In that case,
    /// fall back to a budget of your own.
    pub fn idle_time_remaining(&self) -> Option<Duration> {
        self.dispatcher.idle_time_remaining()
    }

    /// Runs `future` on the main thread until it completes, and returns its output. Only
    /// available in tests.
    ///
    /// If the future is waiting and no other test work can run, the simulated clock
    /// moves to the next timer.
    ///
    /// # Panics
    ///
    /// Panics if `future` is waiting, no timer is pending, and parking isn't allowed.
    #[cfg(all(not(target_family = "wasm"), any(test, feature = "test-support")))]
    #[track_caller]
    pub fn block_test<R>(&self, future: impl Future<Output = R>) -> R {
        use std::cell::Cell;

        let scheduler = self.inner.scheduler();

        let output = Cell::new(None);
        let future = async {
            output.set(Some(future.await));
        };
        let mut future = std::pin::pin!(future);

        // In async GPUI tests, we must allow foreground tasks scheduled by the test itself
        // (which are associated with the test session) to make progress while we block.
        // Otherwise, awaiting futures that depend on same-session foreground work can deadlock.
        scheduler.block(None, future.as_mut(), None);

        output.take().expect("block_test future did not complete")
    }

    /// Blocks the current thread until `future` completes, and returns its output.
    ///
    /// Prefer [`ForegroundExecutor::block_with_timeout`], which gives up instead of
    /// waiting indefinitely.
    #[cfg(not(target_family = "wasm"))]
    pub fn block_on<R>(&self, future: impl Future<Output = R>) -> R {
        self.inner.block_on(future)
    }

    /// Blocks the current thread until `future` completes or `duration` passes.
    ///
    /// Returns `Ok` with the output if the future finished. Otherwise returns `Err` with
    /// the unfinished future, which you can poll later.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::time::Duration;
    /// use gpui::TestApp;
    ///
    /// let app = TestApp::new();
    /// let result = app
    ///     .foreground_executor()
    ///     .block_with_timeout(Duration::from_secs(1), async { 5 });
    /// assert_eq!(result.ok(), Some(5));
    /// ```
    #[cfg(not(target_family = "wasm"))]
    pub fn block_with_timeout<R, Fut: Future<Output = R>>(
        &self,
        duration: Duration,
        future: Fut,
    ) -> Result<R, impl Future<Output = R> + use<R, Fut>> {
        self.inner.block_with_timeout(duration, future)
    }

    /// Returns the platform dispatcher this executor sends work through. GPUI uses this
    /// internally.
    #[doc(hidden)]
    pub fn dispatcher(&self) -> &Arc<dyn PlatformDispatcher> {
        &self.dispatcher
    }

    /// Returns the scheduler's local executor, for code that takes one directly.
    #[doc(hidden)]
    pub fn scheduler_executor(&self) -> SchedulerLocalExecutor {
        self.inner.clone()
    }
}

/// A set of futures that run together and are awaited together.
///
/// [`BackgroundExecutor::scoped`] and [`BackgroundExecutor::scoped_priority`] pass
/// you a `Scope`. Spawn futures on it inside the closure. Not available on wasm.
#[cfg(not(target_family = "wasm"))]
pub struct Scope<'a> {
    executor: BackgroundExecutor,
    priority: Priority,
    futures: Vec<Pin<Box<dyn Future<Output = ()> + Send + 'static>>>,
    tx: Option<mpsc::Sender<()>>,
    rx: mpsc::Receiver<()>,
    lifetime: PhantomData<&'a ()>,
}

#[cfg(not(target_family = "wasm"))]
impl<'a> Scope<'a> {
    fn new(executor: BackgroundExecutor, priority: Priority) -> Self {
        let (tx, rx) = mpsc::channel(1);
        Self {
            executor,
            priority,
            tx: Some(tx),
            rx,
            futures: Default::default(),
            lifetime: PhantomData,
        }
    }

    /// Returns the number of CPUs the dispatcher reports. See
    /// [`BackgroundExecutor::num_cpus`].
    pub fn num_cpus(&self) -> usize {
        self.executor.num_cpus()
    }

    /// Adds `f` to the scope.
    ///
    /// The futures start when the scope's future is first polled, and run in parallel
    /// with the others. The scope waits for every future, so `f` may borrow data that
    /// lives as long as the scope.
    #[track_caller]
    pub fn spawn<F>(&mut self, f: F)
    where
        F: Future<Output = ()> + Send + 'a,
    {
        let tx = self.tx.clone().unwrap();

        // SAFETY: The 'a lifetime is guaranteed to outlive any of these futures because
        // dropping this `Scope` blocks until all of the futures have resolved.
        let f = unsafe {
            mem::transmute::<
                Pin<Box<dyn Future<Output = ()> + Send + 'a>>,
                Pin<Box<dyn Future<Output = ()> + Send + 'static>>,
            >(Box::pin(async move {
                f.await;
                drop(tx);
            }))
        };
        self.futures.push(f);
    }
}

#[cfg(not(target_family = "wasm"))]
impl Drop for Scope<'_> {
    fn drop(&mut self) {
        // Futures that were never submitted hold clones of `tx`, which would keep the
        // channel open forever when the scope is dropped while unwinding.
        self.futures.clear();
        self.tx.take().unwrap();

        // Wait until the channel is closed, which means that all of the spawned
        // futures have resolved.
        let future = async {
            self.rx.next().await;
        };
        let mut future = std::pin::pin!(future);
        self.executor
            .inner
            .scheduler()
            .block(None, future.as_mut(), None);
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::{App, AssetRegistry, TestDispatcher, TestPlatform};
    use std::cell::RefCell;

    /// Helper to create test infrastructure.
    /// Returns (dispatcher, background_executor, app).
    fn create_test_app() -> (TestDispatcher, BackgroundExecutor, Rc<crate::AppCell>) {
        let dispatcher = TestDispatcher::new(0);
        let arc_dispatcher = Arc::new(dispatcher.clone());
        let background_executor = BackgroundExecutor::new(arc_dispatcher.clone());
        let foreground_executor = ForegroundExecutor::new(arc_dispatcher);

        let platform = TestPlatform::new(background_executor.clone(), foreground_executor);
        let http_client = crate::http_client::FakeHttpClient::with_404_response();

        let app = App::new_app(platform, AssetRegistry::default().into(), http_client);
        (dispatcher, background_executor, app)
    }

    #[test]
    #[cfg(not(target_family = "wasm"))]
    fn scoped_registration_panic_does_not_block_on_unstarted_futures() {
        use std::sync::mpsc;

        // A broken cleanup path blocks forever in `Scope::drop`, so run it on a
        // separate thread and fail with a timeout instead of hanging the test.
        let (completed_tx, completed_rx) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            let dispatcher = Arc::new(TestDispatcher::new(0));
            let executor = BackgroundExecutor::new(dispatcher);
            let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                futures::executor::block_on(executor.scoped(|scope| {
                    scope.spawn(async { unreachable!("an unsubmitted future must not run") });
                    panic!("registration failed");
                }));
            }));
            completed_tx.send(unwound.is_err()).unwrap();
        });

        assert_eq!(
            completed_rx.recv_timeout(Duration::from_secs(5)),
            Ok(true),
            "the scope must unwind after dropping its unstarted futures"
        );
        worker.join().unwrap();
    }

    #[test]
    fn sanity_test_tasks_run() {
        let (dispatcher, _background_executor, app) = create_test_app();
        let foreground_executor = app.borrow().foreground_executor.clone();

        let task_ran = Rc::new(RefCell::new(false));

        foreground_executor
            .spawn({
                let task_ran = Rc::clone(&task_ran);
                async move {
                    *task_ran.borrow_mut() = true;
                }
            })
            .detach();

        // Run dispatcher while app is still alive
        dispatcher.run_until_parked();

        // Task should have run
        assert!(
            *task_ran.borrow(),
            "Task should run normally when app is alive"
        );
    }
}
