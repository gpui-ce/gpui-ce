use chrono::{DateTime, Utc};
use parking_lot::Mutex;
use std::time::Duration;

pub use web_time::Instant;

/// Interface for providing current time and monotonic instants.
pub trait Clock {
    /// Returns the current UTC date and time.
    fn utc_now(&self) -> DateTime<Utc>;

    /// Returns the current monotonic instant.
    fn now(&self) -> Instant;
}

/// A mock clock implementation for use in tests.
pub struct TestClock(Mutex<TestClockState>);

struct TestClockState {
    now: Instant,
    utc_now: DateTime<Utc>,
}

impl TestClock {
    /// Creates a new TestClock initialized to a fixed start time.
    pub fn new() -> Self {
        const START_TIME: &str = "2025-07-01T23:59:58-00:00";
        let utc_now = DateTime::parse_from_rfc3339(START_TIME).unwrap().to_utc();
        Self(Mutex::new(TestClockState {
            now: Instant::now(),
            utc_now,
        }))
    }

    /// Sets the current UTC time for the clock.
    pub fn set_utc_now(&self, now: DateTime<Utc>) {
        let mut state = self.0.lock();
        state.utc_now = now;
    }

    /// Advances the clock by the given duration.
    pub fn advance(&self, duration: Duration) {
        let mut state = self.0.lock();
        state.now += duration;
        state.utc_now += duration;
    }
}

impl Clock for TestClock {
    fn utc_now(&self) -> DateTime<Utc> {
        self.0.lock().utc_now
    }

    fn now(&self) -> Instant {
        self.0.lock().now
    }
}
