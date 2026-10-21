mod clock;
mod executor;
mod test_scheduler;
#[cfg(test)]
mod tests;

pub use clock::*;
pub use executor::*;
pub use test_scheduler::*;

use async_task::Runnable;
use futures::channel::oneshot;
use std::{
    any::Any,
    future::Future,
    panic::Location,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};

/// Task priority for background tasks.
///
/// Higher-priority tasks are more likely to run before lower-priority tasks.
/// The scheduler may interleave priorities to prevent starvation, so ordering is not guaranteed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Priority {
    /// Runs on a dedicated thread. Use only for audio.
    RealtimeAudio,
    /// High priority - use for tasks critical to user experience/responsiveness.
    High,
    /// Medium priority - suitable for most use cases.
    #[default]
    Medium,
    /// Low priority - use for background work that can be deprioritized.
    Low,
}

impl Priority {
    /// Returns the relative probability weight used to select tasks.
    pub const fn weight(self) -> u32 {
        match self {
            Priority::High => 60,
            Priority::Medium => 30,
            Priority::Low => 10,
            // realtime priorities are not considered for probability scheduling
            Priority::RealtimeAudio => 0,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct SpawnTime(pub Instant);

/// Metadata attached to runnables for debugging and profiling.
#[derive(Clone, Debug)]
pub struct RunnableMeta {
    /// The source location where the task was spawned.
    pub location: &'static Location<'static>,
    /// The moment the task was spawned.
    pub spawned: SpawnTime,
}

impl RunnableMeta {
    #[track_caller]
    pub fn new_with_callers_location() -> Self {
        Self {
            location: core::panic::Location::caller(),
            spawned: SpawnTime(Instant::now()),
        }
    }
}

pub trait Scheduler: Send + Sync {
    /// Block until the given future completes or timeout occurs.
    ///
    /// Returns `true` if the future completed, `false` if it timed out.
    /// The future is passed as a pinned mutable reference so the caller
    /// retains ownership and can continue polling or return it on timeout.
    #[cfg(not(target_family = "wasm"))]
    fn block(
        &self,
        session_id: Option<SessionId>,
        future: Pin<&mut dyn Future<Output = ()>>,
        timeout: Option<Duration>,
    ) -> bool;

    /// Queues a runnable for `session_id`. Runnables execute in order on the session's
    /// thread, usually the main thread or one created by `spawn_dedicated_thread`.
    fn schedule_local(&self, session_id: SessionId, runnable: Runnable<RunnableMeta>);

    /// Schedule a background task with the given priority.
    fn schedule_background_with_priority(
        &self,
        runnable: Runnable<RunnableMeta>,
        priority: Priority,
    );

    /// Spawn a closure on a dedicated realtime thread for audio processing.
    fn spawn_realtime(&self, f: Box<dyn FnOnce() + Send>);

    /// Schedule a background task with default (medium) priority.
    fn schedule_background(&self, runnable: Runnable<RunnableMeta>) {
        self.schedule_background_with_priority(runnable, Priority::default());
    }

    #[track_caller]
    fn timer(&self, timeout: Duration) -> Timer;
    fn clock(&self) -> Arc<dyn Clock>;

    /// Spawn a closure on a fresh session pinned to its own [`LocalExecutor`].
    ///
    /// `PlatformScheduler` uses a new OS thread via [`spawn_dedicated_thread`].
    /// `TestScheduler` uses its test loop to preserve determinism under `TestScheduler::many`.
    ///
    /// Erases the output as `Box<dyn Any + Send + Sync>` to keep this trait object-safe.
    /// Prefer [`LocalExecutor::spawn_dedicated`] or [`BackgroundExecutor::spawn_dedicated`],
    /// which use [`Task::downcast`] to recover the concrete return type.
    fn spawn_dedicated(self: Arc<Self>, f: DedicatedFn) -> Task<Box<dyn Any + Send + Sync>>;

    fn as_test(&self) -> Option<&TestScheduler> {
        None
    }
}

/// Spawn work on a fresh OS thread that's exclusive to the returned task and
/// anything spawned on the executor it provides. Blocking syscalls inside that
/// work don't disturb any other executor in the process.
///
/// `f` is called on the dedicated thread with a [`LocalExecutor`] pinned
/// to it. The future `f` returns may freely be `!Send`. The returned `Task`
/// resolves to that future's output: dropping it cancels the root, but
/// detached children keep running until they finish. The thread shuts down
/// once the executor and every task on it are gone.
///
/// Never blocks, so it can run on the web main thread. The returned task waits
/// asynchronously for the dedicated thread to deliver the root task's handle,
/// then behaves like that handle.
///
/// Wasm uses a web worker and requires the `wasm-threads` feature and a shared-memory
/// build, otherwise this panics.
///
/// Reuse web sessions across jobs because worker creation instantiates a module
/// and a fresh function table, costing more than an OS thread.
///
/// The caller is responsible for supplying a `session_id` that's distinct from
/// every other live session on `scheduler`. Concrete schedulers typically wrap
/// this in an inherent method that allocates the id from their own counter.
pub fn spawn_dedicated_thread<F, Fut>(
    session_id: SessionId,
    scheduler: Arc<dyn Scheduler>,
    f: F,
) -> Task<Fut::Output>
where
    F: FnOnce(LocalExecutor) -> Fut + Send + 'static,
    Fut: Future + 'static,
    Fut::Output: Send + 'static,
{
    let (task, delivery) = Task::rendezvous();
    let thread_body = move || {
        let (runnable_sender, runnable_receiver) = flume::unbounded::<Runnable<RunnableMeta>>();
        let dispatch = move |runnable: Runnable<RunnableMeta>| {
            let _ = runnable_sender.send(runnable);
        };
        let executor = LocalExecutor::new(session_id, scheduler, dispatch);
        let root_task = executor.spawn(f(executor.clone()));
        // If the caller already dropped or detached the rendezvous task,
        // delivery applies that disposition to the root task here.
        delivery.deliver(root_task);
        // Only spawned tasks and user-held executors retain the sender after this drop.
        // The receive loop exits when they are gone.
        drop(executor);

        while let Ok(runnable) = runnable_receiver.recv() {
            runnable.run();
        }
    };
    spawn_dedicated_os_thread(session_id, thread_body);
    task
}

fn spawn_dedicated_os_thread(session_id: SessionId, thread_body: impl FnOnce() + Send + 'static) {
    let thread_name = format!("spawn_dedicated session {:?}", session_id);
    #[cfg(not(target_family = "wasm"))]
    std::thread::Builder::new()
        .name(thread_name)
        .spawn(thread_body)
        .expect("failed to spawn dedicated thread");
    #[cfg(all(target_family = "wasm", feature = "wasm-threads"))]
    wasm_thread::Builder::new()
        .name(thread_name)
        .spawn(thread_body)
        .expect("failed to spawn dedicated thread");
    #[cfg(all(target_family = "wasm", not(feature = "wasm-threads")))]
    {
        let _ = (thread_name, thread_body);
        panic!("spawn_dedicated on wasm requires the scheduler crate's `wasm-threads` feature");
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct SessionId(u16);

impl SessionId {
    pub fn new(id: u16) -> Self {
        SessionId(id)
    }
}

pub struct Timer(oneshot::Receiver<()>);

impl Timer {
    pub fn new(rx: oneshot::Receiver<()>) -> Self {
        Timer(rx)
    }
}

impl Future for Timer {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<()> {
        match Pin::new(&mut self.0).poll(cx) {
            Poll::Ready(_) => Poll::Ready(()),
            Poll::Pending => Poll::Pending,
        }
    }
}
