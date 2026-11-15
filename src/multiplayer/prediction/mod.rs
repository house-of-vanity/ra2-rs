//! Client-side prediction of the real native assembly, with command acknowledgements.
//! Remote rendering remains interpolated; this world never confirms damage or effects.
mod scene;
#[cfg(test)]
mod tests;
use super::{Multiplayer, Screen};
use crate::{
    battle,
    frontend::{Navigation, Page},
};
use arena_net::{
    HISTORY,
    session::{Frame, InputCommand, Pose, Wiring},
};
use arena_net_io::session::Request;
use bevy::prelude::*;
use scene::{Blueprint, Scene};
use std::{collections::VecDeque, time::Instant};
#[derive(Resource, Default, Debug)]
pub(crate) struct Stats {
    pub corrections: VecDeque<f32>,
    pub maximum: f32,
    pub replayed: u64,
    pub hard_resyncs: u64,
    pub topology_resets: u64,
    pub phase_resets: u64,
    pub cpu_ms: f64,
    pub confirmed: u64,
    pub pending: usize,
    pub checkpoints: u64,
    pub steps: u64,
    pub step_ms: f64,
    pub last_replay: usize,
}
impl Stats {
    pub fn p95(&self) -> f32 {
        let mut v = self.corrections.iter().copied().collect::<Vec<_>>();
        v.sort_by(f32::total_cmp);
        v.get(((v.len().saturating_sub(1)) as f32 * 0.95) as usize)
            .copied()
            .unwrap_or(0.)
    }
}
#[derive(Default)]
pub(crate) struct Prediction {
    scene: Option<Scene>,
    generation: Option<(u64, u64)>,
    confirmed_tick: u64,
    sequence: u64,
    pending: VecDeque<InputCommand>,
    history: VecDeque<(u64, Vec<Pose>)>,
    edges: Vec<u32>,
    levels: Vec<u8>,
    offset: Vec3,
    rotation: Quat,
    shown_root: Option<Transform>,
    active: bool,
    last_sent: f64,
}
impl Prediction {
    fn confirm(
        &mut self,
        blueprint: Option<Blueprint>,
        frame: &Frame,
        slot: usize,
        stats: &mut Stats,
    ) -> Result<(), String> {
        let r = frame
            .robots
            .iter()
            .find(|r| r.slot as usize == slot)
            .ok_or("Local checkpoint missing")?;
        let generation = (frame.epoch, frame.input_generation);
        let changed = self.generation != Some(generation);
        if !changed && frame.tick <= self.confirmed_tick {
            return Ok(());
        }
        let before = Instant::now();
        let expired = !changed
            && (r.acknowledged.saturating_add(HISTORY as u64) < self.sequence
                || self
                    .pending
                    .front()
                    .is_some_and(|c| c.sequence > r.acknowledged.saturating_add(1)));
        if expired {
            stats.hard_resyncs += 1;
        }
        if let Some((_, old)) = self.history.iter().find(|(seq, _)| *seq == r.acknowledged) {
            let error = frame
                .bodies
                .iter()
                .filter(|p| p.id >> 16 == slot as u32 && p.id & 0xffff < 0x8000)
                .filter_map(|p| {
                    old.iter().find(|b| b.id == p.id).map(|b| {
                        Vec3::from_array(b.position).distance(Vec3::from_array(p.position))
                    })
                })
                .fold(0_f32, f32::max);
            stats.maximum = stats.maximum.max(error);
            stats.corrections.push_back(error);
            if stats.corrections.len() > 4096 {
                stats.corrections.pop_front();
            }
        }
        let reset = changed
            || expired
            || self.scene.is_none()
            || frame.paused
            || frame.phase != 2
            || frame.eliminated[slot];
        if let Some(source) = blueprint {
            if self.scene.is_some() {
                stats.topology_resets += 1;
            }
            self.scene = Some(Scene::new(source)?);
        }
        if reset {
            if self.generation.is_some() && changed {
                stats.phase_resets += 1;
            }
            self.pending.clear();
            self.history.clear();
            self.sequence = if changed {
                r.acknowledged
            } else {
                self.sequence.max(r.acknowledged)
            };
            self.edges = r.edges.clone();
            self.offset = Vec3::ZERO;
            self.rotation = Quat::IDENTITY;
            self.shown_root = None;
        }
        self.generation = Some(generation);
        self.confirmed_tick = frame.tick;
        self.active = frame.phase == 2 && !frame.paused && !frame.eliminated[slot];
        let scene = self.scene.as_mut().ok_or("Prediction scene missing")?;
        scene.restore(frame)?;
        self.pending.retain(|c| c.sequence > r.acknowledged);
        self.history.clear();
        if r.acknowledged > self.sequence {
            self.sequence = r.acknowledged;
            stats.hard_resyncs += 1;
        }
        stats.last_replay = self.pending.len();
        for command in &self.pending {
            scene.step(command);
            self.history.push_back((command.sequence, scene.poses()));
            stats.replayed += 1;
        }
        if self.active
            && !reset
            && let Some(old) = self.shown_root
            && let Some(new) = scene
                .poses()
                .iter()
                .find(|p| p.id == super::world::id(slot, 0, false))
        {
            let position = Vec3::from_array(new.position);
            let delta = old.translation - position;
            if delta.length() < 1. {
                self.offset = delta;
                self.rotation = old.rotation * Quat::from_array(new.rotation).inverse();
            } else {
                self.offset = Vec3::ZERO;
                self.rotation = Quat::IDENTITY;
            }
        }
        stats.confirmed = r.acknowledged;
        stats.pending = self.pending.len();
        stats.checkpoints += 1;
        stats.cpu_ms += before.elapsed().as_secs_f64() * 1000.;
        Ok(())
    }
    fn advance(&mut self, levels: Vec<u8>, stats: &mut Stats) {
        if !self.active {
            return;
        }
        let before = Instant::now();
        self.edges.resize(levels.len(), 0);
        self.levels.resize(levels.len(), 0);
        for (i, &v) in levels.iter().enumerate() {
            if v > 127 && self.levels[i] <= 127 {
                self.edges[i] = self.edges[i].saturating_add(1);
            }
        }
        self.levels = levels;
        self.sequence += 1;
        let c = InputCommand {
            sequence: self.sequence,
            levels: self.levels.clone(),
            edges: self.edges.clone(),
        };
        if let Some(scene) = &mut self.scene {
            scene.step(&c);
            self.history.push_back((c.sequence, scene.poses()));
        }
        self.pending.push_back(c);
        while self.pending.len() > HISTORY {
            self.pending.pop_front();
        }
        while self.history.len() > HISTORY {
            self.history.pop_front();
        }
        stats.pending = self.pending.len();
        stats.steps += 1;
        stats.step_ms += before.elapsed().as_secs_f64() * 1000.;
    }
    pub(super) fn wire(&self) -> Option<Wiring> {
        let (epoch, generation) = self.generation?;
        let latest = self.pending.back()?;
        let history = self
            .pending
            .iter()
            .rev()
            .skip(1)
            .take(32)
            .collect::<Vec<_>>();
        let mut redundant: Vec<InputCommand> = vec![];
        for c in history.into_iter().rev() {
            if redundant
                .last()
                .is_none_or(|previous| previous.levels != c.levels || previous.edges != c.edges)
            {
                redundant.push(c.clone());
            }
        }
        Some(Wiring {
            epoch,
            generation,
            sequence: latest.sequence,
            levels: latest.levels.clone(),
            edges: latest.edges.clone(),
            redundant,
        })
    }
}
fn levels(w: &World) -> Vec<u8> {
    let s = w.resource::<battle::Session>();
    let net = w.resource::<Multiplayer>();
    let nav = w.resource::<Navigation>();
    s.robots
        .iter()
        .find(|r| r.id as usize == net.local + 1)
        .and_then(|r| r.document.as_ref())
        .map(|r| {
            crate::editor::controls::samples(
                r,
                w.resource::<ButtonInput<KeyCode>>(),
                nav.page == Page::Match && !net.paused,
            )
        })
        .unwrap_or_default()
        .into_iter()
        .flatten()
        .map(|v| (v * 255.) as u8)
        .collect()
}
pub(super) fn tick(w: &mut World) {
    let net = w.resource::<Multiplayer>();
    let s = w.resource::<battle::Session>();
    if !s.network || net.host || net.screen != Screen::Playing || net.paused || s.net_waiting {
        return;
    }
    let levels = levels(w);
    let mut p = w.remove_non_send::<Prediction>().unwrap_or_default();
    let mut stats = w.remove_resource::<Stats>().unwrap_or_default();
    p.advance(levels, &mut stats);
    present(w, &mut p, 0.);
    w.insert_resource(stats);
    w.insert_non_send(p);
}
pub(super) fn reconcile(w: &mut World) {
    let net = w.resource::<Multiplayer>();
    let s = w.resource::<battle::Session>();
    if !s.network || net.host || !net.loaded_sent {
        if w.non_send::<Prediction>().generation.is_some() {
            w.insert_non_send(Prediction::default());
            w.insert_resource(Stats::default());
        }
        return;
    }
    let local = net.local;
    let Some((latest, _)) = net.frames.back().cloned() else {
        return;
    };
    let dt = w.resource::<Time<Real>>().delta_secs();
    let mut p = w.remove_non_send::<Prediction>().unwrap_or_default();
    let mut stats = w.remove_resource::<Stats>().unwrap_or_default();
    let topology = Blueprint::topology(w);
    let rebuild = p
        .scene
        .as_ref()
        .is_none_or(|scene| scene.source.world_topology != topology)
        || p.generation.is_some_and(|(epoch, _)| epoch != latest.epoch);
    if latest.tick > p.confirmed_tick || p.generation.is_none() {
        let result = (|| {
            let blueprint = if rebuild {
                Some(Blueprint::capture(w)?)
            } else {
                None
            };
            p.confirm(blueprint, &latest, local, &mut stats)
        })();
        if let Err(e) = result {
            w.resource_mut::<Multiplayer>().error = Some(e);
            p.active = false;
        }
    }
    if w.resource::<Multiplayer>().paused {
        p.active = false;
    }
    present(w, &mut p, dt);
    w.insert_resource(stats);
    w.insert_non_send(p);
}
fn present(w: &mut World, p: &mut Prediction, dt: f32) {
    let Some(scene) = &p.scene else {
        return;
    };
    let poses = scene.poses();
    let slot = scene.source.slot;
    let Some(root) = poses
        .iter()
        .find(|v| v.id == super::world::id(slot, 0, false))
    else {
        return;
    };
    let root_pos = Vec3::from_array(root.position);
    let decay = (-dt / 0.08).exp();
    p.offset *= decay;
    p.rotation = Quat::IDENTITY.slerp(p.rotation, decay);
    let entities = w
        .resource::<battle::Session>()
        .robots
        .iter()
        .find(|r| r.id as usize == slot + 1)
        .map(|r| r.bodies.clone())
        .unwrap_or_default();
    for pose in &poses {
        if let Some(&e) = entities.get((pose.id & 0xffff) as usize) {
            if let Some(mut t) = w.get_mut::<Transform>(e) {
                t.translation =
                    root_pos + p.offset + p.rotation * (Vec3::from_array(pose.position) - root_pos);
                t.rotation = p.rotation * Quat::from_array(pose.rotation);
                if pose.id & 0xffff == 0 {
                    p.shown_root = Some(*t);
                }
            }
            if let Some(mut v) = w.get_mut::<bevy_rapier3d::prelude::Velocity>(e) {
                v.linear = Vec3::from_array(pose.linear);
                v.angular = Vec3::from_array(pose.angular);
            }
        }
    }
    let predicted = scene.robot();
    if let Some(r) = w
        .resource_mut::<battle::Session>()
        .robots
        .iter_mut()
        .find(|r| r.id as usize == slot + 1)
    {
        if let (Some(target), Some(source)) = (&mut r.rig, &predicted.rig) {
            target.electrical = source.electrical;
            target.pneumatic = source.pneumatic;
        }
        r.states = predicted.states.clone();
        r.commands = predicted.commands.clone();
    }
}
pub(super) fn send_input(w: &mut World) {
    let net = w.resource::<Multiplayer>();
    if !w.resource::<battle::Session>().network || net.host || net.screen != Screen::Playing {
        return;
    }
    let now = w.resource::<Time<Real>>().elapsed_secs_f64();
    let mut p = w.remove_non_send::<Prediction>().unwrap_or_default();
    if now - p.last_sent >= 1. / 60. {
        p.last_sent = now;
        if let Some(wire) = p.wire() {
            w.resource_mut::<Multiplayer>().send(Request::Wiring(wire));
        }
    }
    w.insert_non_send(p);
}
