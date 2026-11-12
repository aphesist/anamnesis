//! Hybrid logical clocks (Kulkarni et al., 2014): wall-clock milliseconds plus a counter,
//! so every event gets a timestamp that is close to real time, never goes backwards on a
//! device, and is later than anything that device had already received. Ties between
//! devices break on the device id, so all devices sort the merged log the same way.

use serde::{Deserialize, Serialize};

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default,
)]
pub struct Hlc {
    /// Milliseconds since the unix epoch.
    pub t: i64,
    /// Logical counter within the same millisecond.
    pub c: u32,
}

impl std::fmt::Display for Hlc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}", self.t, self.c)
    }
}

/// A device's clock: the last timestamp it issued or received.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Clock {
    pub last: Hlc,
}

impl Clock {
    /// Timestamp a local event that happened at wall time `wall_ms`.
    pub fn tick(&mut self, wall_ms: i64) -> Hlc {
        let next = if wall_ms > self.last.t {
            Hlc { t: wall_ms, c: 0 }
        } else {
            Hlc {
                t: self.last.t,
                c: self.last.c + 1,
            }
        };
        self.last = next;
        next
    }

    /// Take in a timestamp received from another device.
    pub fn observe(&mut self, remote: Hlc) {
        if remote > self.last {
            self.last = remote;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monotonic_and_after_what_was_received() {
        let mut c = Clock::default();
        let a = c.tick(1000);
        let b = c.tick(1000);
        let d = c.tick(900); // the wall clock went back
        assert!(a < b && b < d);
        assert_eq!(d.t, 1000);
        c.observe(Hlc { t: 5000, c: 3 });
        let e = c.tick(1001);
        assert_eq!(e, Hlc { t: 5000, c: 4 }, "later than what it received");
        let f = c.tick(6000);
        assert_eq!(f, Hlc { t: 6000, c: 0 });
    }
}
