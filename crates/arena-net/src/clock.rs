//! Four-timestamp clock estimate. Physics ticks and packet arrival time are different clocks.
use crate::*;
#[derive(Clone, Debug, Default)]
pub struct ClockSync {
    offset_us: f64,
    best_rtt_us: Option<u64>,
    pub samples: u64,
}
impl ClockSync {
    /// t1/t4: local send/receive; t2/t3: host receive/send, all monotonic microseconds.
    pub fn observe(&mut self, t1: u64, t2: u64, t3: u64, t4: u64) -> Result<()> {
        if t4 < t1 || t3 < t2 || t3 - t2 > t4 - t1 {
            return Err(Error::Malformed);
        }
        let rtt = (t4 - t1) - (t3 - t2);
        if rtt > 2_000_000 {
            return Err(Error::Limit);
        }
        let offset = ((t2 as f64 - t1 as f64) + (t3 as f64 - t4 as f64)) * 0.5;
        self.samples += 1;
        // Prefer low-queue samples; do not mistake network jitter for clock motion.
        if self.best_rtt_us.is_none_or(|old| rtt <= old) {
            self.offset_us = offset;
            self.best_rtt_us = Some(rtt);
        }
        Ok(())
    }
    pub fn host_time_us(&self, local_us: u64) -> f64 {
        local_us as f64 + self.offset_us
    }
    pub fn rtt_us(&self) -> Option<u64> {
        self.best_rtt_us
    }
}
