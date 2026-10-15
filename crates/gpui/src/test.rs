//! Support for the `#[gpui::test]` macro, and streams for observing entities in tests.
//!
//! `#[gpui::test]` runs a test function once per seed. Each `&TestAppContext`
//! parameter gets its own app, and all the apps share one deterministic scheduler,
//! so a failing seed replays the same interleaving. The functions in this module
//! are used by the macro; you rarely need to call them directly. See the
//! [testing guide](crate::_documentation::guides::testing).
//!
//! # Example
//!
//! ```
//! use gpui::{AppContext, Global, TestAppContext};
//!
//! struct Name(&'static str);
//! impl Global for Name {}
//!
//! #[gpui::test]
//! fn test_counts_are_positive(cx: &mut TestAppContext) {
//!     let count = cx.new(|_| 1usize);
//!     cx.run_until_parked();
//!     cx.read(|cx| assert!(*count.read(cx) > 0));
//! }
//!
//! #[gpui::test]
//! fn test_each_app_has_its_own_globals(cx_a: &mut TestAppContext, cx_b: &mut TestAppContext) {
//!     cx_a.update(|cx| cx.set_global(Name("alice")));
//!     cx_b.update(|cx| cx.set_global(Name("bob")));
//!     cx_a.read_global::<Name, _>(|name, _| assert_eq!(name.0, "alice"));
//!     cx_b.read_global::<Name, _>(|name, _| assert_eq!(name.0, "bob"));
//! }
//! ```
use crate::{Entity, Subscription, TestAppContext, TestDispatcher};
use futures::StreamExt as _;
use proptest::prelude::{Just, Strategy, any};
use std::{
    env,
    panic::{self, RefUnwindSafe, UnwindSafe},
    pin::Pin,
};

/// Returns a proptest strategy for scheduler seeds.
///
/// When `SEED` is set, the strategy always produces that value. Otherwise it
/// produces any `u64`. Seeds are not shrunk, so a failing seed is replayed as
/// recorded rather than replaced with a smaller seed.
/// This is for the `#[gpui::property_test]` macro.
///
/// # Panics
///
/// Panics if `SEED` is set but isn't a `u64`.
pub fn seed_strategy() -> impl Strategy<Value = u64> {
    match std::env::var("SEED") {
        Ok(val) => Just(val.parse().unwrap()).boxed(),
        Err(_) => any::<u64>().no_shrink().boxed(),
    }
}

/// Sets the RNG seed of a proptest config from `SEED`, or from `0` when it's unset.
///
/// The same variable then controls both the scheduler seed and proptest's case
/// generation. The `#[gpui::property_test]` macro uses this helper.
///
/// If `SEED` is not a `u64`, this function uses `0`. [`seed_strategy`] panics for
/// the same input.
pub fn apply_seed_to_proptest_config(
    mut config: proptest::test_runner::Config,
) -> proptest::test_runner::Config {
    let seed = env::var("SEED")
        .ok()
        .and_then(|val| val.parse::<u64>().ok())
        .unwrap_or(0);
    config.rng_seed = proptest::test_runner::RngSeed::Fixed(seed);
    config
}

/// Runs `test_fn` once on a dispatcher for `seed`, and returns its result.
///
/// Unlike [`run_test`], this runs once without retries, and `test_fn` can be an
/// `FnOnce`. The `#[gpui::property_test]` macro uses it. If `test_fn` panics,
/// the panic is re-raised after the dispatcher is dropped.
pub fn run_test_once<R>(
    seed: u64,
    test_fn: Box<dyn UnwindSafe + FnOnce(TestDispatcher) -> R>,
) -> R {
    let result = panic::catch_unwind(|| {
        let dispatcher = TestDispatcher::new(seed);
        let scheduler = dispatcher.scheduler().clone();
        let res = test_fn(dispatcher);
        scheduler.end_test();
        res
    });

    match result {
        Ok(r) => r,
        Err(e) => panic::resume_unwind(e),
    }
}

/// Runs `test_fn` for each seed, retrying a failing seed up to `max_retries` times.
///
/// The `#[gpui::test]` macro calls this function. It chooses seeds from the
/// macro's arguments and these environment variables:
///
/// - `ITERATIONS=n` overrides the number of iterations.
/// - `SEED=n` runs only seed `n`, or, with more than one iteration, the seeds
///   `n` to `n + iterations - 1`. It replaces `explicit_seeds`.
///
/// When more than one seed runs, each seed is printed before it runs, and a
/// failing seed is printed when its last attempt fails. After the last failure,
/// `on_fail_fn` runs, then the panic is re-raised.
pub fn run_test(
    num_iterations: usize,
    explicit_seeds: &[u64],
    max_retries: usize,
    test_fn: &mut (dyn RefUnwindSafe + Fn(TestDispatcher, u64)),
    on_fail_fn: Option<fn()>,
) {
    let (seeds, is_multiple_runs) = calculate_seeds(num_iterations as u64, explicit_seeds);

    for seed in seeds {
        let mut attempt = 0;
        loop {
            if is_multiple_runs {
                eprintln!("seed = {seed}");
            }
            let result = panic::catch_unwind(|| {
                let dispatcher = TestDispatcher::new(seed);
                let scheduler = dispatcher.scheduler().clone();
                test_fn(dispatcher, seed);
                scheduler.end_test();
            });

            match result {
                Ok(_) => break,
                Err(error) => {
                    if attempt < max_retries {
                        println!("attempt {} failed, retrying", attempt);
                        attempt += 1;
                        // The panic payload might itself trigger an unwind on drop:
                        // https://doc.rust-lang.org/std/panic/fn.catch_unwind.html#notes
                        std::mem::forget(error);
                    } else {
                        if is_multiple_runs {
                            eprintln!("failing seed: {seed}");
                            eprintln!(
                                "You can rerun from this seed by setting the environmental variable SEED to {seed}"
                            );
                        }
                        if let Some(on_fail_fn) = on_fail_fn {
                            on_fail_fn()
                        }
                        panic::resume_unwind(error);
                    }
                }
            }
        }
    }
}

fn calculate_seeds(
    iterations: u64,
    explicit_seeds: &[u64],
) -> (impl Iterator<Item = u64> + '_, bool) {
    let iterations = env::var("ITERATIONS")
        .ok()
        .map(|var| var.parse().expect("invalid ITERATIONS variable"))
        .unwrap_or(iterations);

    let env_num = env::var("SEED")
        .map(|seed| seed.parse().expect("invalid SEED variable as integer"))
        .ok();

    let iter = seed_sequence(iterations, explicit_seeds, env_num);
    let is_multiple_runs = iter.clone().nth(1).is_some();
    (iter, is_multiple_runs)
}

fn seed_sequence(
    iterations: u64,
    explicit_seeds: &[u64],
    env_num: Option<u64>,
) -> impl Iterator<Item = u64> + Clone + '_ {
    // if `iterations` is 1 and `explicit_seeds` is non-empty, discard the run `0`
    // if `SEED` is set, do `SEED..SEED+iterations` and ignore `explicit_seeds`
    // otherwise, do `0..iterations` followed by `explicit_seeds`
    let range = match (iterations, env_num) {
        (1, None) if !explicit_seeds.is_empty() => 0..0,
        (iterations, Some(env)) => env..env + iterations,
        (iterations, None) => 0..iterations,
    };
    let explicit_seeds = if env_num.is_some() {
        &[]
    } else {
        explicit_seeds
    };
    range.chain(explicit_seeds.iter().copied())
}

#[cfg(test)]
mod seed_tests {
    use super::seed_sequence;

    #[test]
    fn environment_seed_starts_one_sequence_without_duplicate_first_run() {
        assert_eq!(
            seed_sequence(3, &[80], Some(17)).collect::<Vec<_>>(),
            [17, 18, 19]
        );
        assert_eq!(seed_sequence(1, &[80], Some(17)).collect::<Vec<_>>(), [17]);
    }

    #[test]
    fn explicit_seeds_replace_default_run_or_extend_iteration_range() {
        assert_eq!(seed_sequence(1, &[], None).collect::<Vec<_>>(), [0]);
        assert_eq!(
            seed_sequence(1, &[17, 80], None).collect::<Vec<_>>(),
            [17, 80]
        );
        assert_eq!(
            seed_sequence(3, &[80], None).collect::<Vec<_>>(),
            [0, 1, 2, 80]
        );
    }
}

/// A stream of unit values from an entity observation, created by [`observe`].
///
/// Dropping the stream stops the observation.
pub struct Observation<T> {
    rx: Pin<Box<async_channel::Receiver<T>>>,
    _subscription: Subscription,
}

impl<T: 'static> futures::Stream for Observation<T> {
    type Item = T;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.rx.poll_next_unpin(cx)
    }
}

/// Returns a stream that yields `()` each time `entity` notifies.
///
/// Dropping the returned stream unsubscribes. For a stream that also ends when the
/// entity is released, use [`TestAppContext::notifications`].
pub fn observe<T: 'static>(entity: &Entity<T>, cx: &mut TestAppContext) -> Observation<()> {
    let (tx, rx) = async_channel::unbounded();
    let _subscription = cx.update(|cx| {
        cx.observe(entity, move |_, _| {
            let _ = gpui::block_on(tx.send(()));
        })
    });
    let rx = Box::pin(rx);

    Observation { rx, _subscription }
}
