//! Shared tick/replay orchestration. The physics adapter owns solver state and side effects.
use crate::{input::*, snapshot::*, *};
use std::collections::{BTreeMap, VecDeque};
pub trait Simulation {
    fn snapshot(&self) -> Snapshot;
    /// Validate before mutation; on error the previous simulation must remain intact.
    /// A successful restore must also discard stale solver/contact/constraint caches.
    fn restore(&mut self, snapshot: &Snapshot) -> Result<()>;
    /// Prediction must not commit HP, score, destruction or external effects.
    fn step(
        &mut self,
        tick: Tick,
        inputs: &BTreeMap<PeerId, AppliedInput>,
        authoritative: bool,
    ) -> Vec<EventKind>;
    fn set_paused(&mut self, paused: bool);
}
pub struct Host<S> {
    pub sim: S,
    pub inputs: BTreeMap<PeerId, InputBuffer>,
    pub events: Vec<Event>,
    next_event: u64,
}
impl<S: Simulation> Host<S> {
    pub fn new(sim: S, owners: impl IntoIterator<Item = PeerId>) -> Self {
        Self {
            sim,
            inputs: owners
                .into_iter()
                .map(|p| (p, InputBuffer::default()))
                .collect(),
            events: vec![],
            next_event: 1,
        }
    }
    /// The transport-authenticated sender selects the slot. Packets cannot select a victim/owner.
    pub fn receive(&mut self, sender: PeerId, frames: &[Input]) -> Result<()> {
        if frames.len() > 16 {
            return Err(Error::Limit);
        }
        let now = self.sim.snapshot().tick;
        let input = self.inputs.get_mut(&sender).ok_or(Error::Unauthorized)?;
        let mut candidate = input.clone();
        for &frame in frames {
            candidate.receive(now, frame)?;
        }
        *input = candidate;
        Ok(())
    }
    pub fn advance(&mut self) {
        let tick = self.sim.snapshot().tick + 1;
        let inputs = self
            .inputs
            .iter_mut()
            .map(|(p, b)| (*p, b.consume(tick)))
            .collect();
        let events = self.sim.step(tick, &inputs, true);
        for kind in events {
            self.emit(tick, kind);
        }
    }
    fn emit(&mut self, tick: Tick, kind: EventKind) {
        self.events.push(Event {
            id: self.next_event,
            tick,
            kind,
        });
        self.next_event += 1;
    }
    pub fn pause(&mut self, sender: PeerId, paused: bool) -> Result<()> {
        if sender != HOST {
            return Err(Error::Unauthorized);
        }
        self.sim.set_paused(paused);
        for input in self.inputs.values_mut() {
            input.clear();
        }
        self.emit(self.sim.snapshot().tick, EventKind::Paused(paused));
        Ok(())
    }
}

#[derive(Default, Debug)]
pub struct PredictionMetrics {
    pub corrections: VecDeque<f32>,
    pub angles: VecDeque<f32>,
    pub stale: u64,
    pub replayed_ticks: u64,
    pub hard_resyncs: u64,
    pub reconciliations: u64,
    pub motion_max: f32,
    pub topology_max: f32,
    pub topology_changes: u64,
    pub last_topology_change: bool,
    pub phase_max: f32,
    pub phase_changes: u64,
    pub last_phase_change: bool,
}
pub struct Predictor<S> {
    pub sim: S,
    pub owner: PeerId,
    pub metrics: PredictionMetrics,
    inputs: BTreeMap<Tick, Input>,
    history: VecDeque<Snapshot>,
    confirmed: Option<Tick>,
    action: u32,
}
impl<S: Simulation> Predictor<S> {
    pub fn new(sim: S, owner: PeerId) -> Self {
        let initial = sim.snapshot();
        Self {
            sim,
            owner,
            metrics: Default::default(),
            inputs: BTreeMap::new(),
            history: VecDeque::from([initial]),
            confirmed: None,
            action: 0,
        }
    }
    pub fn advance(&mut self, input: Input) -> Result<()> {
        if input.tick != self.sim.snapshot().tick + 1 || !input.valid() {
            return Err(Error::Conflict);
        }
        let fire = input.action > self.action;
        self.action = self.action.max(input.action);
        self.inputs.insert(input.tick, input);
        self.sim.step(
            input.tick,
            &BTreeMap::from([(self.owner, AppliedInput { input, fire })]),
            false,
        );
        self.history.push_back(self.sim.snapshot());
        while self.history.len() > HISTORY {
            self.history.pop_front();
        }
        while self.inputs.len() > HISTORY {
            self.inputs.pop_first();
        }
        Ok(())
    }
    pub fn recent_inputs(&self, count: usize) -> Vec<Input> {
        self.inputs
            .values()
            .rev()
            .take(count.min(16))
            .copied()
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect()
    }
    /// Reliable phase changes cannot wait for the next lossy snapshot. Rewind to
    /// the event tick, retain predicted pose there, and replay with the new phase.
    pub fn control_pause(&mut self, tick: Tick, paused: bool) -> Result<bool> {
        if self.confirmed.is_some_and(|confirmed| confirmed >= tick) {
            return Ok(false);
        }
        let target = self.sim.snapshot().tick;
        let mut checkpoint = self
            .history
            .iter()
            .find(|s| s.tick == tick)
            .cloned()
            .ok_or(Error::MissingHistory)?;
        checkpoint.paused = paused;
        self.sim.restore(&checkpoint)?;
        self.action = checkpoint
            .robots
            .iter()
            .find(|r| r.owner == self.owner)
            .map_or(0, |r| r.action);
        self.history.retain(|s| s.tick < tick);
        self.history.push_back(checkpoint);
        for next in tick + 1..=target {
            let input = self.inputs.get(&next).copied().unwrap_or(Input {
                tick: next,
                action: self.action,
                ..Default::default()
            });
            let fire = input.action > self.action;
            self.action = self.action.max(input.action);
            self.sim.step(
                next,
                &BTreeMap::from([(self.owner, AppliedInput { input, fire })]),
                false,
            );
            self.history.push_back(self.sim.snapshot());
            self.metrics.replayed_ticks += 1;
        }
        while self.history.len() > HISTORY {
            self.history.pop_front();
        }
        Ok(true)
    }
    /// Compare at the same authoritative tick, then restore and replay pending commands.
    /// Measuring against the newest host pose would misreport network delay as prediction error.
    pub fn reconcile(&mut self, snapshot: &Snapshot) -> Result<bool> {
        snapshot.validate()?;
        if self.confirmed.is_some_and(|tick| snapshot.tick <= tick) {
            self.metrics.stale += 1;
            return Ok(false);
        }
        let target = self.sim.snapshot().tick.max(snapshot.tick);
        if let Some(predicted) = self.history.iter().find(|s| s.tick == snapshot.tick) {
            let phase_change = predicted.paused != snapshot.paused;
            let topology = predicted
                .robots
                .iter()
                .find(|r| r.owner == self.owner)
                .map(|r| r.revision)
                != snapshot
                    .robots
                    .iter()
                    .find(|r| r.owner == self.owner)
                    .map(|r| r.revision);
            let mut distance = 0_f32;
            let mut angle = 0_f32;
            for actual in snapshot.bodies.iter().filter(|b| b.owner == self.owner) {
                if let Some(old) = predicted.body(actual.id) {
                    distance = distance.max(actual.position.distance(old.position));
                    angle = angle.max(actual.rotation.angle_between(old.rotation).abs());
                }
            }
            self.metrics.corrections.push_back(distance);
            self.metrics.angles.push_back(angle);
            self.metrics.last_topology_change = topology;
            self.metrics.last_phase_change = phase_change;
            if topology {
                self.metrics.topology_max = self.metrics.topology_max.max(distance);
                self.metrics.topology_changes += 1;
            } else if phase_change {
                self.metrics.phase_max = self.metrics.phase_max.max(distance);
                self.metrics.phase_changes += 1;
            } else {
                self.metrics.motion_max = self.metrics.motion_max.max(distance);
            }
            if self.metrics.corrections.len() > 4096 {
                self.metrics.corrections.pop_front();
                self.metrics.angles.pop_front();
            }
        } else if target > snapshot.tick {
            self.metrics.hard_resyncs += 1;
        }
        self.sim.restore(snapshot)?;
        self.action = snapshot
            .robots
            .iter()
            .find(|r| r.owner == self.owner)
            .map_or(0, |r| r.action);
        self.history.retain(|s| s.tick <= snapshot.tick);
        self.inputs.retain(|tick, _| *tick > snapshot.tick);
        // Bounded replay. A long outage is explicitly a hard resync, never an unbounded CPU spike.
        if target - snapshot.tick > HISTORY as u64 {
            self.inputs.clear();
            self.history.clear();
        } else {
            for tick in snapshot.tick + 1..=target {
                let input = self.inputs.get(&tick).copied().unwrap_or(Input {
                    tick,
                    action: self.action,
                    ..Default::default()
                });
                let fire = input.action > self.action;
                self.action = self.action.max(input.action);
                self.sim.step(
                    tick,
                    &BTreeMap::from([(self.owner, AppliedInput { input, fire })]),
                    false,
                );
                self.history.push_back(self.sim.snapshot());
                self.metrics.replayed_ticks += 1;
            }
        }
        while self.history.len() > HISTORY {
            self.history.pop_front();
        }
        self.confirmed = Some(snapshot.tick);
        self.metrics.reconciliations += 1;
        Ok(true)
    }
}
