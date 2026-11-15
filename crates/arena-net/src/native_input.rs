//! Fixed-duration native commands. Acknowledgements refer to simulation steps,
//! not packet receipt; holding a lost command still consumes one sequence number.
use crate::{
    session::{InputCommand, Wiring},
    *,
};
use std::collections::BTreeMap;
#[derive(Clone, Debug, Default)]
pub struct CommandBuffer {
    pending: BTreeMap<u64, InputCommand>,
    pub acknowledged: u64,
    pub last: InputCommand,
    pub late: u64,
    pub missing: u64,
    pub resyncs: u64,
    active: bool,
}
impl CommandBuffer {
    pub fn receive(&mut self, w: &Wiring) -> Result<()> {
        let commands = w.commands()?;
        let mut next = self.clone();
        if next.active && w.sequence > next.acknowledged.saturating_add(HISTORY as u64) {
            // Sequence numbers identify inputs, not a right to simulate extra time.
            // After a history-sized clock gap discard unsimulated commands and
            // resume at the new bounded batch, still doing only one step per tick.
            if w.sequence > next.acknowledged.saturating_add(4 * HISTORY as u64) {
                return Err(Error::Limit);
            }
            let first = commands.first().ok_or(Error::Malformed)?.sequence;
            next.pending.clear();
            next.acknowledged = first - 1;
            next.resyncs += 1;
        }
        for c in commands {
            if c.sequence <= next.acknowledged {
                next.late += 1;
                continue;
            }
            if next.active && c.sequence > next.acknowledged.saturating_add(HISTORY as u64) {
                return Err(Error::Limit);
            }
            if let Some(old) = next.pending.get(&c.sequence) {
                if old != &c {
                    return Err(Error::Conflict);
                }
            } else {
                next.pending.insert(c.sequence, c);
            }
        }
        if next.pending.len() > HISTORY {
            return Err(Error::Limit);
        }
        *self = next;
        Ok(())
    }
    pub fn advance(&mut self, live: bool) -> InputCommand {
        if !self.active {
            if let Some((&first, _)) = self.pending.first_key_value() {
                self.acknowledged = first - 1;
                self.active = true;
            } else {
                return InputCommand::default();
            }
        }
        self.acknowledged += 1;
        if let Some(c) = self.pending.remove(&self.acknowledged) {
            self.last = c;
        } else {
            self.missing += 1;
        }
        let mut c = self.last.clone();
        c.sequence = self.acknowledged;
        if !live {
            c.levels.fill(0);
        }
        c
    }
    pub fn len(&self) -> usize {
        self.pending.len()
    }
    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }
}
