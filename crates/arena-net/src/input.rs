//! Tick-addressed redundant input. Repeated packets never repeat a weapon edge.
use crate::*;
use std::collections::BTreeMap;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Input {
    pub tick: Tick,
    pub throttle: i16,
    pub steer: i16,
    /// Monotonic activation counter, not a level that can stick during packet loss.
    pub action: u32,
}
impl Input {
    pub fn axes(self) -> (f32, f32) {
        (self.throttle as f32 / 32767., self.steer as f32 / 32767.)
    }
    pub fn valid(self) -> bool {
        self.throttle != i16::MIN && self.steer != i16::MIN
    }
}
#[derive(Clone, Copy, Debug, Default)]
pub struct AppliedInput {
    pub input: Input,
    pub fire: bool,
}
#[derive(Default, Clone)]
pub struct InputBuffer {
    pending: BTreeMap<Tick, Input>,
    previous: Input,
    action: u32,
    last_received: Tick,
    seen: std::collections::BTreeSet<Tick>,
    pub late: u64,
    pub missing: u64,
}
impl InputBuffer {
    pub fn receive(&mut self, now: Tick, input: Input) -> Result<()> {
        if !input.valid() {
            return Err(Error::Malformed);
        }
        if input.tick > now.saturating_add(HZ) {
            return Err(Error::Limit);
        }
        self.seen.retain(|tick| *tick >= now.saturating_sub(HZ));
        if input.tick <= now {
            if self.seen.insert(input.tick) {
                self.late += 1;
            }
            return Ok(());
        }
        if let Some(old) = self.pending.get(&input.tick) {
            return if *old == input {
                Ok(())
            } else {
                Err(Error::Conflict)
            };
        }
        self.pending.insert(input.tick, input);
        self.seen.insert(input.tick);
        Ok(())
    }
    pub fn consume(&mut self, tick: Tick) -> AppliedInput {
        self.pending.retain(|t, _| *t >= tick);
        let input = if let Some(input) = self.pending.remove(&tick) {
            self.last_received = tick;
            self.previous = input;
            input
        } else {
            self.missing += 1;
            if tick.saturating_sub(self.last_received) > HZ / 4 {
                Input {
                    tick,
                    ..Default::default()
                }
            } else {
                Input {
                    tick,
                    ..self.previous
                }
            }
        };
        let fire = input.action > self.action;
        self.action = self.action.max(input.action);
        AppliedInput { input, fire }
    }
    pub fn clear(&mut self) {
        self.pending.clear();
        self.previous = Input::default();
    }
    pub fn len(&self) -> usize {
        self.pending.len()
    }
    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }
}
