use super::*;
use futures::{
    FutureExt,
    channel::{mpsc, oneshot},
    executor::block_on,
    future,
    sink::SinkExt,
    stream::{FuturesUnordered, StreamExt},
};
use std::{
    cell::RefCell,
    collections::{BTreeSet, HashSet},
    pin::Pin,
    rc::Rc,
    sync::Arc,
    task::{Context, Poll, Waker},
};

#[test]
fn test_executor_spawn() {
    TestScheduler::once(async |scheduler| {
        assert_eq!(scheduler.foreground().spawn(async { 42 }).await, 42);
    });

    TestScheduler::once(async |scheduler| {
        assert_eq!(scheduler.background().spawn(async { 42 }).await, 42);
    });

    TestScheduler::once(async |scheduler| {
        assert_eq!(
            DedicatedExecutor::new(&scheduler.background())
                .spawn(async { 42 })
                .await,
            42
        );
    });

    TestScheduler::once(async |scheduler| {
        assert_eq!(
            scheduler
                .background()
                .spawn_dedicated(|_executor| async { 42 })
                .await,
            42
        );
    });
}

#[test]
fn test_scheduler_drops_with_stalled_detached_foreground_task() {
    let scheduler = Arc::new(TestScheduler::new(TestSchedulerConfig::default()));
    let weak_scheduler = Arc::downgrade(&scheduler);
    let (sender, receiver) = oneshot::channel::<()>();

    scheduler
        .foreground()
        .spawn(async move {
            receiver.await.ok();
        })
        .detach();
    scheduler.run();

    drop(scheduler);
    assert!(weak_scheduler.upgrade().is_none());
    drop(sender);
}

#[test]
fn test_scheduler_drops_with_stalled_detached_background_task() {
    let scheduler = Arc::new(TestScheduler::new(TestSchedulerConfig::default()));
    let weak_scheduler = Arc::downgrade(&scheduler);
    let (sender, receiver) = oneshot::channel::<()>();

    scheduler
        .background()
        .spawn(async move {
            receiver.await.ok();
        })
        .detach();
    scheduler.run();

    drop(scheduler);
    assert!(weak_scheduler.upgrade().is_none());
    drop(sender);
}

/// A dedicated task that is never polled must not keep the scheduler alive:
/// its runnable sits in the scheduler's own queue, so any strong scheduler
/// handle captured by the future would form a reference cycle and leak both.
#[test]
fn test_scheduler_drops_with_never_polled_dedicated_task() {
    let scheduler = Arc::new(TestScheduler::new(TestSchedulerConfig::default()));
    let weak_scheduler = Arc::downgrade(&scheduler);

    scheduler
        .background()
        .spawn_dedicated(|_executor| async move {})
        .detach();

    drop(scheduler);
    assert!(weak_scheduler.upgrade().is_none());
}

#[test]
fn test_foreground_ordering() {
    let mut traces = HashSet::new();

    TestScheduler::many(if cfg!(miri) { 5 } else { 100 }, async |scheduler| {
        #[derive(Hash, PartialEq, Eq)]
        struct TraceEntry {
            session: usize,
            task: usize,
        }

        let trace = Rc::new(RefCell::new(Vec::new()));

        let foreground_1 = scheduler.foreground();
        for task in 0..10 {
            foreground_1
                .spawn({
                    let trace = trace.clone();
                    async move {
                        trace.borrow_mut().push(TraceEntry { session: 0, task });
                    }
                })
                .detach();
        }

        let foreground_2 = scheduler.foreground();
        for task in 0..10 {
            foreground_2
                .spawn({
                    let trace = trace.clone();
                    async move {
                        trace.borrow_mut().push(TraceEntry { session: 1, task });
                    }
                })
                .detach();
        }

        scheduler.run();

        assert_eq!(
            trace
                .borrow()
                .iter()
                .filter(|entry| entry.session == 0)
                .map(|entry| entry.task)
                .collect::<Vec<_>>(),
            (0..10).collect::<Vec<_>>()
        );
        assert_eq!(
            trace
                .borrow()
                .iter()
                .filter(|entry| entry.session == 1)
                .map(|entry| entry.task)
                .collect::<Vec<_>>(),
            (0..10).collect::<Vec<_>>()
        );

        traces.insert(trace.take());
    });

    assert!(traces.len() > 1, "Expected at least two traces");
}

#[test]
fn test_timer_ordering() {
    TestScheduler::many(1, async |scheduler| {
        let background = scheduler.background();
        let futures = FuturesUnordered::new();
        futures.push(
            async {
                background.timer(Duration::from_millis(100)).await;
                2
            }
            .boxed(),
        );
        futures.push(
            async {
                background.timer(Duration::from_millis(50)).await;
                1
            }
            .boxed(),
        );
        futures.push(
            async {
                background.timer(Duration::from_millis(150)).await;
                3
            }
            .boxed(),
        );
        assert_eq!(futures.collect::<Vec<_>>().await, vec![1, 2, 3]);
    });
}

#[test]
fn test_foreground_task_can_hold_mut_borrow_across_await() {
    TestScheduler::once(async |scheduler| {
        let foreground = scheduler.foreground();
        let (sender, mut receiver) = mpsc::unbounded::<()>();

        foreground
            .spawn(async move {
                receiver.next().await;
            })
            .detach();

        scheduler.run();
        sender.unbounded_send(()).unwrap();
        scheduler.run();
    });
}

#[test]
fn test_send_from_bg_to_fg() {
    TestScheduler::once(async |scheduler| {
        let foreground = scheduler.foreground();
        let background = scheduler.background();

        let (sender, receiver) = oneshot::channel::<i32>();

        background
            .spawn(async move {
                sender.send(42).unwrap();
            })
            .detach();

        let task = foreground.spawn(async move { receiver.await.unwrap() });
        let result = task.await;
        assert_eq!(result, 42);
    });
}

#[test]
fn test_randomize_order() {
    let mut deterministic_results = HashSet::new();
    for seed in 0..10 {
        let config = TestSchedulerConfig {
            seed,
            randomize_order: false,
            ..Default::default()
        };
        let order = block_on(capture_execution_order(config));
        assert_eq!(order.len(), 6);
        deterministic_results.insert(order);
    }

    assert_eq!(
        deterministic_results.len(),
        1,
        "Deterministic mode should always produce same execution order"
    );

    let mut randomized_results = HashSet::new();
    for seed in 0..20 {
        let config = TestSchedulerConfig::with_seed(seed);
        let order = block_on(capture_execution_order(config));
        assert_eq!(order.len(), 6);
        randomized_results.insert(order);
    }

    assert!(
        randomized_results.len() > 1,
        "Randomized mode should produce multiple different orders"
    );
}

async fn capture_execution_order(config: TestSchedulerConfig) -> Vec<String> {
    let scheduler = Arc::new(TestScheduler::new(config));
    let foreground = scheduler.foreground();
    let background = scheduler.background();

    let (sender, receiver) = mpsc::unbounded::<String>();

    for i in 0..3 {
        let mut sender = sender.clone();
        foreground
            .spawn(async move {
                sender.send(format!("fg-{}", i)).await.ok();
            })
            .detach();
    }

    for i in 0..3 {
        let mut sender = sender.clone();
        background
            .spawn(async move {
                sender.send(format!("bg-{}", i)).await.ok();
            })
            .detach();
    }

    drop(sender); // Close sender to signal no more messages
    scheduler.run();

    receiver.collect().await
}

#[test]
fn test_block() {
    let scheduler = Arc::new(TestScheduler::new(TestSchedulerConfig::default()));
    let (tx, rx) = oneshot::channel();

    scheduler
        .background()
        .spawn(async move {
            tx.send(42).unwrap();
        })
        .detach();

    let result = scheduler.foreground().block_on(async { rx.await.unwrap() });
    assert_eq!(result, 42);
}

#[test]
#[should_panic(expected = "Parking forbidden.")]
fn test_parking_panics() {
    let config = TestSchedulerConfig {
        capture_pending_traces: true,
        ..Default::default()
    };
    let scheduler = Arc::new(TestScheduler::new(config));
    scheduler.foreground().block_on(async {
        let (_tx, rx) = oneshot::channel::<()>();
        rx.await.unwrap(); // This will never complete
    });
}

#[test]
fn test_block_with_parking() {
    let config = TestSchedulerConfig {
        allow_parking: true,
        ..Default::default()
    };
    let scheduler = Arc::new(TestScheduler::new(config));
    let (tx, rx) = oneshot::channel();

    scheduler
        .background()
        .spawn(async move {
            tx.send(42).unwrap();
        })
        .detach();

    let result = scheduler.foreground().block_on(async { rx.await.unwrap() });
    assert_eq!(result, 42);
}

#[test]
fn test_helper_methods() {
    let result = TestScheduler::once(async |scheduler: Arc<TestScheduler>| {
        let background = scheduler.background();
        background.spawn(async { 42 }).await
    });
    assert_eq!(result, 42);

    let results = TestScheduler::many(3, async |scheduler: Arc<TestScheduler>| {
        let background = scheduler.background();
        background.spawn(async { 10 }).await
    });
    assert_eq!(results, vec![10, 10, 10]);
}

#[test]
fn test_block_with_timeout() {
    TestScheduler::once(async |scheduler| {
        let foreground = scheduler.foreground();
        let future = future::ready(42);
        let output = foreground.block_with_timeout(Duration::from_millis(100), future);
        assert_eq!(output.ok(), Some(42));
    });

    TestScheduler::once(async |scheduler| {
        // A zero tick budget prevents `block_with_timeout` from progressing the scheduler
        // and accidentally completing work that we expect to time out.
        scheduler.set_timeout_ticks(0..=0);

        let foreground = scheduler.foreground();
        let future = future::pending::<()>();
        let output = foreground.block_with_timeout(Duration::from_millis(50), future);
        assert!(output.is_err(), "future should not have finished");
    });

    let mut results = BTreeSet::new();
    TestScheduler::many(if cfg!(miri) { 5 } else { 100 }, async |scheduler| {
        // Keep random tick budgets so some seeds complete and others time out.
        let task = scheduler.background().spawn(async move {
            Yield { polls: 10 }.await;
            42
        });
        let output = scheduler
            .foreground()
            .block_with_timeout(Duration::from_millis(50), task);
        results.insert(output.ok());
    });
    assert_eq!(
        results.into_iter().collect::<Vec<_>>(),
        if cfg!(miri) {
            vec![Some(42)]
        } else {
            vec![None, Some(42)]
        }
    );

    // Timing out must not cancel the future. It must remain pollable to completion.
    // Time advances only when explicitly advanced, not when the future yields.
    TestScheduler::once(async |scheduler| {
        // Zero ticks prevent scheduler steps and timer advancement during the timeout.
        scheduler.set_timeout_ticks(0..=0);

        let background = scheduler.background();

        // This task should only complete once time is explicitly advanced.
        let task = background.spawn({
            let scheduler = scheduler.clone();
            async move {
                scheduler.timer(Duration::from_millis(100)).await;
                123
            }
        });

        // This should time out before we advance time enough for the timer to fire.
        let timed_out = scheduler
            .foreground()
            .block_with_timeout(Duration::from_millis(50), task);
        assert!(
            timed_out.is_err(),
            "expected timeout before advancing the clock enough for the timer"
        );

        // Now explicitly advance time and ensure the returned future can complete.
        let mut task = timed_out.err().unwrap();
        scheduler.advance_clock(Duration::from_millis(100));
        scheduler.run();

        let output = scheduler.foreground().block_on(&mut task);
        assert_eq!(output, 123);
    });
}

// When calling block, we shouldn't make progress on foreground-spawned futures with the same session id.
#[test]
fn test_block_does_not_progress_same_session_foreground() {
    let mut task2_made_progress_once = false;
    TestScheduler::many(if cfg!(miri) { 5 } else { 1000 }, async |scheduler| {
        let foreground1 = scheduler.foreground();
        let foreground2 = scheduler.foreground();

        let task1 = foreground1.spawn(async move {});
        let task2 = foreground2.spawn(async move {});

        foreground1.block_on(async {
            scheduler.yield_random().await;
            assert!(!task1.is_ready());
            task2_made_progress_once |= task2.is_ready();
        });

        task1.await;
        task2.await;
    });

    assert!(
        task2_made_progress_once,
        "Expected task from different foreground executor to make progress (at least once)"
    );
}

struct Yield {
    polls: usize,
}

impl Future for Yield {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.polls -= 1;
        if self.polls == 0 {
            Poll::Ready(())
        } else {
            cx.waker().wake_by_ref();
            Poll::Pending
        }
    }
}

#[test]
fn test_nondeterministic_wake_and_drop_detection() {
    for action in [Waker::wake as fn(Waker), drop::<Waker>] {
        let config = TestSchedulerConfig {
            allow_parking: false,
            ..Default::default()
        };
        let scheduler = Arc::new(TestScheduler::new(config));

        let (waker_tx, waker_rx) = std::sync::mpsc::channel::<Waker>();

        scheduler.foreground().block_on(future::poll_fn(|cx| {
            waker_tx.send(cx.waker().clone()).unwrap();

            Poll::Ready(())
        }));

        let handle = std::thread::spawn(move || {
            if let Ok(waker) = waker_rx.recv() {
                action(waker);
            }
        });

        handle.join().ok();

        // The non-determinism error should be detected when end_test is called
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            scheduler.end_test();
        }));
        assert!(result.is_err(), "Expected end_test to panic");
        let panic_payload = result.unwrap_err();
        let panic_message = panic_payload
            .downcast_ref::<String>()
            .map(|s| s.as_str())
            .or_else(|| panic_payload.downcast_ref::<&str>().copied())
            .unwrap_or("<unknown panic>");
        assert!(
            panic_message.contains("Your test is not deterministic"),
            "Expected panic message to contain non-determinism error, got: {}",
            panic_message
        );
    }
}

#[test]
fn test_nondeterministic_wake_allowed_with_parking() {
    let config = TestSchedulerConfig {
        allow_parking: true,
        ..Default::default()
    };
    let scheduler = Arc::new(TestScheduler::new(config));

    let (waker_tx, waker_rx) = std::sync::mpsc::channel::<Waker>();

    std::thread::spawn(move || {
        if let Ok(waker) = waker_rx.recv() {
            // With allow_parking, this should NOT panic
            waker.wake();
        }
    });

    let mut waker_tx = Some(waker_tx);
    scheduler.foreground().block_on(future::poll_fn(|cx| {
        if let Some(sender) = waker_tx.take() {
            sender.send(cx.waker().clone()).unwrap();

            return Poll::Pending;
        }

        Poll::Ready(())
    }));
}

#[test]
fn test_background_priority_scheduling() {
    use parking_lot::Mutex;

    let mut high_before_low_count = 0;
    let iterations = if cfg!(miri) { 5 } else { 100 };

    for seed in 0..iterations {
        let config = TestSchedulerConfig::with_seed(seed);
        let scheduler = Arc::new(TestScheduler::new(config));
        let background = scheduler.background();

        let execution_order = Arc::new(Mutex::new(Vec::new()));

        for i in 0..3 {
            let order = execution_order.clone();
            background
                .spawn_with_priority(Priority::Low, async move {
                    order.lock().push(format!("low-{}", i));
                })
                .detach();
        }

        for i in 0..3 {
            let order = execution_order.clone();
            background
                .spawn_with_priority(Priority::High, async move {
                    order.lock().push(format!("high-{}", i));
                })
                .detach();
        }

        scheduler.run();

        let order = execution_order.lock();
        let high_in_first_half = order
            .iter()
            .take(3)
            .filter(|s| s.starts_with("high"))
            .count();

        if high_in_first_half >= 2 {
            high_before_low_count += 1;
        }
    }

    // Weights of 60 versus 10 should favor high-priority tasks early.
    assert!(
        high_before_low_count > iterations / 2,
        "Expected high priority tasks to run before low priority tasks more often. \
         Got {} out of {} iterations",
        high_before_low_count,
        iterations
    );
}

#[test]
fn test_spawn_dedicated_not_send_future() {
    let result = TestScheduler::once(async |scheduler| {
        scheduler
            .background()
            .spawn_dedicated(|_executor| async move {
                // `Rc<RefCell<_>>` is `!Send`. If `spawn_dedicated` required
                // the returned future to be `Send`, this wouldn't compile.
                let state = Rc::new(RefCell::new(0_i32));
                for _ in 0..5 {
                    *state.borrow_mut() += 1;
                }
                *state.borrow()
            })
            .await
    });
    assert_eq!(result, 5);
}

#[test]
fn test_spawn_dedicated_send_closure_captures() {
    use parking_lot::Mutex;

    let observed = TestScheduler::once(async |scheduler| {
        let shared = Arc::new(Mutex::new(0_i32));
        let shared_for_closure = shared.clone();
        let returned = scheduler
            .background()
            .spawn_dedicated(move |_executor| {
                // Mutate the shared value inside the !Send future after crossing the Send boundary.
                let local = shared_for_closure;
                async move {
                    *local.lock() = 7;
                }
            })
            .await;
        let _: () = returned;
        *shared.lock()
    });
    assert_eq!(observed, 7);
}

#[test]
fn test_spawn_dedicated_inner_spawn_local() {
    let result = TestScheduler::once(async |scheduler| {
        scheduler
            .background()
            .spawn_dedicated(|executor| async move {
                // The provided executor can spawn additional `!Send` work
                // onto the same dedicated session.
                let inner = Rc::new(RefCell::new(0_i32));
                let inner_for_child = inner.clone();
                let child = executor.spawn(async move {
                    *inner_for_child.borrow_mut() = 99;
                    *inner_for_child.borrow()
                });
                child.await
            })
            .await
    });
    assert_eq!(result, 99);
}

#[test]
fn test_spawn_dedicated_determinism_under_many() {
    use parking_lot::Mutex;

    let iterations = if cfg!(miri) { 4 } else { 20 };
    let mut run_tasks = async |scheduler: Arc<TestScheduler>| {
        let trace = Arc::new(Mutex::new(Vec::<u32>::new()));

        let background = scheduler.background();
        let mut tasks = Vec::new();
        for id in 0..4_u32 {
            let trace = trace.clone();
            let task = background.spawn_dedicated(move |executor| async move {
                for step in 0..3 {
                    trace.lock().push(id * 100 + step);
                    executor.spawn(async {}).await;
                }
                id
            });
            tasks.push(task);
        }

        let mut outputs = Vec::new();
        for task in tasks {
            outputs.push(task.await);
        }

        (trace.lock().clone(), outputs)
    };
    let outcomes = TestScheduler::many(iterations, &mut run_tasks);
    let outcomes_replay = TestScheduler::many(iterations, &mut run_tasks);

    assert_eq!(
        outcomes, outcomes_replay,
        "per-seed outcomes should be reproducible"
    );

    // At least one seed must interleave dedicated tasks.
    let any_interleaved = outcomes.iter().any(|(trace, _)| {
        trace
            .windows(2)
            .any(|window| window[0] / 100 != window[1] / 100)
    });
    assert!(
        any_interleaved,
        "expected at least one seed to interleave dedicated tasks"
    );
}

#[test]
fn test_spawn_dedicated_dropping_task_cancels_future() {
    use parking_lot::Mutex;

    let counter_after = TestScheduler::once(async |scheduler| {
        let counter = Arc::new(Mutex::new(0_u32));
        let (resume_tx, resume_rx) = oneshot::channel::<()>();

        let task = {
            let counter = counter.clone();
            scheduler
                .background()
                .spawn_dedicated(move |_executor| async move {
                    *counter.lock() = 1;
                    // Park here until the test resumes us. If the task is
                    // dropped before this resolves, the second assignment
                    // below must never happen.
                    let _ = resume_rx.await;
                    *counter.lock() = 2;
                })
        };

        scheduler.run();
        assert_eq!(*counter.lock(), 1);

        // Cancel by dropping the root task, then unblock the parked oneshot.
        // The future must not advance past the await: counter stays at 1.
        drop(task);
        let _ = resume_tx.send(());
        scheduler.run();

        *counter.lock()
    });

    assert_eq!(
        counter_after, 1,
        "dropping the dedicated task must cancel the root future before its second write"
    );
}

#[test]
fn test_spawn_dedicated_detached_child_runs_after_root_completes() {
    use parking_lot::Mutex;

    let child_ran = TestScheduler::once(async |scheduler| {
        let child_ran = Arc::new(Mutex::new(false));

        let task = {
            let child_ran = child_ran.clone();
            scheduler
                .background()
                .spawn_dedicated(move |executor| async move {
                    executor
                        .spawn(async move {
                            *child_ran.lock() = true;
                        })
                        .detach();
                    // Root returns immediately, before the child has had a
                    // chance to run.
                })
        };

        task.await;

        // Drain the dedicated session. The detached child must run.
        scheduler.run();

        *child_ran.lock()
    });

    assert!(
        child_ran,
        "detached child must complete after the root, not be cancelled with it"
    );
}

// The production `spawn_dedicated` smoke test is in `crates/gpui/src/platform_scheduler.rs`.
