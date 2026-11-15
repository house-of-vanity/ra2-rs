//! Stable wire identities and complete gameplay state. Render smoothing never changes this state.
use crate::*;
use arena_core::{Quat, Vec3};
use std::collections::{BTreeMap, VecDeque};
#[derive(Clone, Debug, PartialEq)]
pub struct BodyState {
    pub id: BodyId,
    pub owner: PeerId,
    pub parent: Option<BodyId>,
    pub position: Vec3,
    pub rotation: Quat,
    pub linear: Vec3,
    pub angular: Vec3,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MotorState {
    pub position: f32,
    pub burst_time: f32,
    pub initialized: bool,
    pub returning: bool,
}
#[derive(Clone, Debug, PartialEq)]
pub struct RobotState {
    pub owner: PeerId,
    pub hp: Vec<f32>,
    pub energy: f32,
    pub air: f32,
    pub score: f32,
    pub revision: u32,
    pub action: u32,
    pub motors: Vec<MotorState>,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    pub tick: Tick,
    pub sim_tick: Tick,
    pub paused: bool,
    pub bodies: Vec<BodyState>,
    pub robots: Vec<RobotState>,
}
impl Snapshot {
    pub fn validate(&self) -> Result<()> {
        if self.tick > (1_u64 << 48) || self.sim_tick > self.tick {
            return Err(Error::Limit);
        }
        if self.bodies.len() > MAX_BODIES || self.robots.len() > MAX_PLAYERS {
            return Err(Error::Limit);
        }
        let robots = self
            .robots
            .iter()
            .map(|r| (r.owner, r))
            .collect::<BTreeMap<_, _>>();
        let bodies = self
            .bodies
            .iter()
            .map(|b| (b.id, b))
            .collect::<BTreeMap<_, _>>();
        if robots.len() != self.robots.len() || bodies.len() != self.bodies.len() {
            return Err(Error::Conflict);
        }
        for r in &self.robots {
            if r.hp.len() > 64 || r.motors.len() > 64 {
                return Err(Error::Limit);
            }
            if r.hp
                .iter()
                .chain([&r.energy, &r.air, &r.score])
                .any(|v| !v.is_finite() || *v < 0.)
                || r.motors.iter().any(|m| {
                    !m.position.is_finite() || !m.burst_time.is_finite() || m.burst_time < 0.
                })
            {
                return Err(Error::Malformed);
            }
        }
        for b in &self.bodies {
            if !robots.contains_key(&b.owner)
                || !b.position.is_finite()
                || !b.linear.is_finite()
                || !b.angular.is_finite()
                || b.position.abs().max_element() > 1_000_000.
                || b.linear.abs().max_element() > 10_000.
                || b.angular.abs().max_element() > 100_000.
                || !b.rotation.is_finite()
                || (b.rotation.length_squared() - 1.).abs() > 0.01
            {
                return Err(Error::Malformed);
            }
            let mut parent = b.parent;
            let mut depth = 0;
            while let Some(id) = parent {
                let p = bodies.get(&id).ok_or(Error::Malformed)?;
                if p.owner != b.owner || id == b.id || depth >= MAX_BODIES {
                    return Err(Error::Malformed);
                }
                parent = p.parent;
                depth += 1;
            }
        }
        Ok(())
    }
    pub fn body(&self, id: BodyId) -> Option<&BodyState> {
        self.bodies.iter().find(|b| b.id == id)
    }
}

#[derive(Default)]
pub struct Interpolation {
    frames: VecDeque<Snapshot>,
    joints: BTreeMap<BodyId, JointFrames>,
    pub underruns: u64,
    pub stale: u64,
}
/// Authored anchors for fixed/revolute joints, sent with the assembly, not every snapshot.
/// Sliders require their own position-dependent anchors and must not use this constraint.
#[derive(Clone, Copy, Debug)]
pub struct JointFrames {
    pub child: BodyId,
    pub parent: BodyId,
    pub parent_anchor: Vec3,
    pub child_anchor: Vec3,
}
impl Interpolation {
    pub fn with_joints(joints: impl IntoIterator<Item = JointFrames>) -> Result<Self> {
        let mut this = Self::default();
        for j in joints {
            if this.joints.len() >= MAX_BODIES
                || j.child == j.parent
                || !j.parent_anchor.is_finite()
                || !j.child_anchor.is_finite()
            {
                return Err(Error::Malformed);
            }
            if this.joints.insert(j.child, j).is_some() {
                return Err(Error::Conflict);
            }
        }
        Ok(this)
    }
    fn constrain(&self, s: &mut Snapshot) {
        let mut done = std::collections::BTreeSet::new();
        for _ in 0..s.bodies.len() {
            let mut changed = false;
            for i in 0..s.bodies.len() {
                let id = s.bodies[i].id;
                if done.contains(&id) {
                    continue;
                }
                let Some(parent) = s.bodies[i].parent else {
                    done.insert(id);
                    changed = true;
                    continue;
                };
                if !done.contains(&parent) {
                    continue;
                }
                if let Some(j) = self.joints.get(&id).filter(|j| j.parent == parent)
                    && let Some(p) = s.body(parent)
                {
                    s.bodies[i].position = p.position + p.rotation * j.parent_anchor
                        - s.bodies[i].rotation * j.child_anchor;
                }
                done.insert(id);
                changed = true;
            }
            if !changed {
                break;
            }
        }
    }
    pub fn push(&mut self, snapshot: Snapshot) -> Result<bool> {
        snapshot.validate()?;
        if self.frames.back().is_some_and(|s| s.tick >= snapshot.tick) {
            self.stale += 1;
            return Ok(false);
        }
        self.frames.push_back(snapshot);
        while self.frames.len() > 64 {
            self.frames.pop_front();
        }
        Ok(true)
    }
    /// Time is in host ticks, not local receipt time. Never interpolates across a broken joint.
    pub fn sample(&mut self, tick: f64) -> Option<Snapshot> {
        let first = self.frames.front()?;
        let last = self.frames.back()?;
        if tick < first.tick as f64 {
            self.underruns += 1;
            return Some(first.clone());
        }
        if tick > last.tick as f64 {
            self.underruns += 1;
            let mut result = last.clone();
            let dt = ((tick - last.tick as f64) / HZ as f64).min(0.05) as f32;
            if !last.paused {
                for b in &mut result.bodies {
                    b.position += b.linear * dt;
                    b.rotation = (Quat::from_scaled_axis(b.angular * dt) * b.rotation).normalize();
                }
            }
            self.constrain(&mut result);
            return Some(result);
        }
        let b_index = self.frames.iter().position(|s| s.tick as f64 >= tick)?;
        let b = &self.frames[b_index];
        let a = &self.frames[b_index.saturating_sub(1)];
        let t = if b.tick == a.tick {
            1.
        } else {
            ((tick - a.tick as f64) / (b.tick - a.tick) as f64) as f32
        };
        let mut result = a.clone();
        for body in &mut result.bodies {
            if let Some(next) = b.body(body.id)
                && body.parent == next.parent
                && a.robots
                    .iter()
                    .find(|r| r.owner == body.owner)
                    .map(|r| r.revision)
                    == b.robots
                        .iter()
                        .find(|r| r.owner == body.owner)
                        .map(|r| r.revision)
            {
                body.position = body.position.lerp(next.position, t);
                body.rotation = body.rotation.slerp(next.rotation, t);
            }
        }
        if t >= 1. {
            result = b.clone();
        }
        self.constrain(&mut result);
        Some(result)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum EventKind {
    Damage { victim: PeerId, part: u16, hp: f32 },
    Detached { body: BodyId },
    Paused(bool),
    Finished(Option<PeerId>),
}
#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    pub id: u64,
    pub tick: Tick,
    pub kind: EventKind,
}
#[derive(Default)]
pub struct EventCursor {
    last: u64,
    pub accepted: u64,
    pub duplicates: u64,
}
impl EventCursor {
    /// Must be delivered on the ordered reliable event lane. Absolute snapshots own HP.
    pub fn accept(&mut self, event: &Event) -> Result<bool> {
        if event.id <= self.last {
            self.duplicates += 1;
            return Ok(false);
        }
        if event.id != self.last + 1 {
            return Err(Error::Conflict);
        }
        self.last = event.id;
        self.accepted += 1;
        Ok(true)
    }
}
