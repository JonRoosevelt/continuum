use std::cmp::Ordering;

use serde::{Deserialize, Serialize};

use crate::id::DeviceId;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Version {
    pub counter: u64,
    pub device: DeviceId,
}

impl Version {
    #[must_use]
    pub const fn new(counter: u64, device: DeviceId) -> Self {
        Self { counter, device }
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        self.counter
            .cmp(&other.counter)
            .then_with(|| self.device.cmp(&other.device))
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Debug, Clone)]
pub struct Clock {
    device: DeviceId,
    counter: u64,
}

impl Clock {
    #[must_use]
    pub const fn new(device: DeviceId) -> Self {
        Self { device, counter: 0 }
    }

    #[must_use]
    pub const fn device(&self) -> DeviceId {
        self.device
    }

    #[must_use]
    pub const fn counter(&self) -> u64 {
        self.counter
    }

    pub fn tick(&mut self) -> Version {
        self.counter = self.counter.saturating_add(1);
        Version::new(self.counter, self.device)
    }

    pub fn observe(&mut self, remote: Version) {
        self.counter = self.counter.max(remote.counter);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ticking_is_monotonic() {
        let mut clock = Clock::new(DeviceId::from_bytes([1; 32]));
        let a = clock.tick();
        let b = clock.tick();
        assert!(b > a);
    }

    #[test]
    fn observe_advances_past_remote() {
        let local = DeviceId::from_bytes([1; 32]);
        let remote = DeviceId::from_bytes([2; 32]);
        let mut clock = Clock::new(local);
        clock.tick();
        clock.observe(Version::new(41, remote));
        let next = clock.tick();
        assert_eq!(next.counter, 42);
        assert_eq!(next.device, local);
    }

    #[test]
    fn equal_counter_breaks_ties_by_device() {
        let a = Version::new(7, DeviceId::from_bytes([1; 32]));
        let b = Version::new(7, DeviceId::from_bytes([2; 32]));
        assert!(a < b);
    }
}
