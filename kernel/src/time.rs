mod pit;
use crate::time::pit::TICKS;
use core::sync::atomic::Ordering;
pub use pit::{do_tick, init};
pub type TimerId = usize;
/// A Represents a length of time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Duration {
    ticks: u64,
}

impl Duration {
    pub fn as_micros(&self) -> u64 {
        self.ticks / 1000
    }
    pub fn as_millis(&self) -> u64 {
        self.ticks / 1000 / 1000
    }
    pub fn as_secs(&self) -> u64 {
        self.ticks / 1000 / 1000 / 1000
    }
}

/// Represents a point in time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Instant {
    ticks: u64,
}
impl Instant {
    pub fn now() -> Self {
        use Ordering::Relaxed;
        Instant {
            ticks: TICKS.load(Relaxed),
        }
    }
    /// Returns the number of ticks since the `Instant` was created.
    pub fn elapsed(&self) -> Duration {
        use Ordering::Relaxed;
        Duration {
            ticks: TICKS.load(Relaxed) - self.ticks,
        }
    }
}
impl core::ops::Add<Duration> for Instant {
    type Output = Instant;
    fn add(self, rhs: Duration) -> Self::Output {
        use Ordering::Relaxed;
        Instant {
            ticks: TICKS.load(Relaxed) + rhs.ticks,
        }
    }
}

impl core::ops::Sub<Duration> for Instant {
    type Output = Instant;
    fn sub(self, rhs: Duration) -> Self::Output {
        use Ordering::Relaxed;
        Instant {
            ticks: TICKS.load(Relaxed) - rhs.ticks,
        }
    }
}
