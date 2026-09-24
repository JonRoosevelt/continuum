use std::time::{Duration, Instant};

use continuum_core::{ClipboardItem, Clock, DeviceId};

use continuum_platform::backend::{ClipboardBackend, ClipboardError};

const ACTIVE_INTERVAL: Duration = Duration::from_millis(100);
const IDLE_INTERVAL: Duration = Duration::from_millis(1000);
const ACTIVE_WINDOW: Duration = Duration::from_secs(2);

pub struct Monitor {
    backend: Box<dyn ClipboardBackend>,
    clock: Clock,
    device_id: DeviceId,
    active_until: Option<Instant>,
}

impl Monitor {
    pub fn open(device_id: DeviceId) -> Result<Self, ClipboardError> {
        Ok(Self {
            backend: continuum_platform::open()?,
            clock: Clock::new(device_id),
            device_id,
            active_until: None,
        })
    }

    #[must_use]
    #[allow(dead_code)]
    pub const fn device_id(&self) -> DeviceId {
        self.device_id
    }

    /// Polls the clipboard. Returns a stamped item only when new content appeared.
    pub fn poll(&mut self) -> Result<Option<ClipboardItem>, ClipboardError> {
        let Some(items) = self.backend.read()? else {
            return Ok(None);
        };
        self.active_until = Some(Instant::now() + ACTIVE_WINDOW);
        let version = self.clock.tick();
        Ok(Some(ClipboardItem::new(self.device_id, version, items)?))
    }

    #[allow(dead_code)]
    pub fn write(&mut self, items: &[continuum_core::ClipItem]) -> Result<(), ClipboardError> {
        self.backend.write(items)
    }

    #[must_use]
    pub fn next_delay(&self) -> Duration {
        match self.active_until {
            Some(deadline) if Instant::now() < deadline => ACTIVE_INTERVAL,
            _ => IDLE_INTERVAL,
        }
    }
}
