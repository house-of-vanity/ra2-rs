//! Native match adapter. Only the host commits physics, damage, traps and match rules.
//! Clients present a buffered authoritative world; visual interpolation never writes HP.
use super::*;
use crate::gameplay::{RobotBody, RobotOwner};
use bevy::prelude::Real;
use bevy_rapier3d::prelude::*;
#[derive(Component)]
pub(crate) struct ArenaBody(pub u32);
impl Multiplayer {
    pub(crate) fn sounds(&mut self, mut events: Vec<(String, [f32; 3])>) {
        events.truncate(64);
        let epoch = self.epoch;
        self.send(Request::Sounds(epoch, events));
    }
    pub(crate) fn return_lobby(&mut self) {
        if self.host {
            self.send(Request::ReturnLobby);
        } else {
            self.error = Some("Waiting for the host to return to the lobby".into());
        }
    }
    pub(crate) fn pause(&mut self, tick: u64, paused: bool) {
        self.send(Request::Pause(tick, paused));
    }
}
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(super) fn prepare(
    mut commands: Commands,
    mut net: ResMut<Multiplayer>,
    mut s: ResMut<battle::Session>,
    mut nav: ResMut<Navigation>,
    bodies: Query<(Entity, &Name, &RapierContextEntityLink, Option<&RobotOwner>), With<RigidBody>>,
    mut rigid: Query<&mut RigidBody>,
    mut assets: ResMut<legacy_ui::assets::LegacyAssets>,
) {
    for p in &net.players {
        let key = thumbnail(p);
        if !assets.images.contains_key(&key)
            && let Some(bot) = net.bots.get(&p.digest)
            && let Some(bytes) = &bot.snapshot
            && let Ok(image) = legacy_ui::assets::decode_image(bytes, "tga")
        {
            assets.images.insert(key, image);
        }
    }
    if net.screen != Screen::Loading || net.loaded_sent {
        return;
    }
    if !net.loaded {
        if net
            .players
            .iter()
            .any(|p| !net.bots.contains_key(&p.digest))
        {
            return;
        }
        let Some(arena) = s.arenas.iter().position(|a| {
            a.key == net.settings.arena && a.modes.contains(&ui::mode(net.settings.mode))
        }) else {
            net.error = Some("The selected arena/mode is unavailable".into());
            net.send(Request::Leave);
            return;
        };
        s.selected = [None; 4];
        for p in &net.players {
            let bot = net.bots[&p.digest].clone();
            let i = s.library.len();
            s.library.push(crate::content::robots::Entry {
                path: std::path::PathBuf::new(),
                team: p.team.clone(),
                team_key: format!("network/{}", p.slot),
                logo: String::new(),
                motto: String::new(),
                robot: bot,
            });
            s.selected[p.slot as usize] = Some(i);
        }
        s.mode = ui::mode(net.settings.mode);
        s.arena = arena;
        s.minutes = net.settings.minutes as u32;
        s.hazards = net.settings.hazards;
        s.weight = net.settings.weight.map(u32::from);
        s.network = true;
        s.authority = net.host;
        s.local_slot = net.local;
        s.net_waiting = true;
        s.net_paused = false;
        s.start = true;
        net.loaded = true;
        nav.page = Page::Match;
        return;
    }
    if let Some(e) = s.error.clone() {
        net.disconnect();
        s.network = false;
        nav.page = Page::Multiplayer;
        net.error = Some(e);
        return;
    }
    if s.start || s.robots.is_empty() {
        return;
    }
    let mut arena = bodies
        .iter()
        .filter(|(_, _, ctx, owner)| ctx.0 == s.context && owner.is_none())
        .map(|(e, n, _, _)| (n.as_str().to_string(), e))
        .collect::<Vec<_>>();
    arena.sort_by(|a, b| a.0.cmp(&b.0));
    for (i, (_, e)) in arena.into_iter().enumerate() {
        commands.entity(e).insert(ArenaBody(0x8000_0000 + i as u32));
    }
    if !net.host {
        for robot in &s.robots {
            if robot.id as usize == net.local + 1 {
                continue;
            }
            for &e in &robot.bodies {
                if let Ok(mut body) = rigid.get_mut(e) {
                    *body = RigidBody::KinematicPositionBased;
                }
            }
        }
    }
    let epoch = net.epoch;
    net.loaded_sent = true;
    net.send(Request::Loaded(epoch));
}
pub(crate) fn id(slot: usize, body: usize, fragment: bool) -> u32 {
    (slot as u32) << 16
        | if fragment {
            0x8000 + body as u32
        } else {
            body as u32
        }
}
fn pose(id: u32, t: &Transform, v: &Velocity) -> Pose {
    Pose {
        id,
        position: t.translation.to_array(),
        rotation: t.rotation.to_array(),
        linear: v.linear.to_array(),
        angular: v.angular.to_array(),
    }
}
#[allow(clippy::too_many_arguments)]
pub(super) fn capture(
    mut net: ResMut<Multiplayer>,
    s: Res<battle::Session>,
    poses: Query<(&Transform, &Velocity)>,
    arena: Query<(Entity, &ArenaBody)>,
    inputs: Res<AuthoritativeInputs>,
) {
    if !s.network || !net.host || net.screen != Screen::Playing {
        return;
    }
    net.tick += 1;
    if net.tick % 4 != 0 {
        return;
    }
    let mut bodies = vec![];
    let mut robots = vec![];
    for r in &s.robots {
        let slot = r.id as usize - 1;
        for (fragment, list) in [(false, &r.bodies), (true, &r.damage.fragments)] {
            for (i, &e) in list.iter().enumerate() {
                if let Ok((t, v)) = poses.get(e) {
                    bodies.push(pose(id(slot, i, fragment), t, v));
                }
            }
        }
        robots.push(Robot {
            slot: slot as u8,
            hp: r.damage.health.parts.iter().map(|p| p.current).collect(),
            energy: r.rig.as_ref().map_or(0., |r| r.electrical.remaining),
            air: r.rig.as_ref().map_or(0., |r| r.pneumatic.remaining),
            motors: r
                .states
                .iter()
                .map(|s| {
                    [
                        s.position,
                        s.burst_time,
                        f32::from(s.initialized),
                        f32::from(s.returning),
                    ]
                })
                .collect(),
            commands: r
                .commands
                .iter()
                .map(|c| [c.drive, c.steer, f32::from(c.fire)])
                .collect(),
            detached: r.damage.detached_roots.iter().map(|n| *n as u16).collect(),
            numbers: r
                .damage
                .health
                .numbers
                .iter()
                .map(|n| Number {
                    hp: n.amount,
                    position: n.position.to_array(),
                    age: n.age,
                    source: n.source,
                })
                .collect(),
            shell_revision: r.damage.shell.as_ref().map_or(0, |s| s.revision),
            controls: r.controls.snapshot(),
            acknowledged: inputs
                .peers
                .get(&(slot as u8))
                .map_or(0, |p| p.buffer.acknowledged),
            edges: inputs
                .peers
                .get(&(slot as u8))
                .map_or_else(Vec::new, |p| p.edges.clone()),
        });
        if let Some(shell) = &r.damage.shell
            && net.shell_sent.get(&(slot as u8)).copied().unwrap_or(0) < shell.revision
        {
            let skin = Skin {
                epoch: net.epoch,
                slot: slot as u8,
                revision: shell.revision,
                positions: shell
                    .geometry
                    .positions
                    .iter()
                    .map(|p| p.to_array())
                    .collect(),
            };
            net.send(Request::Skin(skin));
            net.shell_sent.insert(slot as u8, shell.revision);
        }
    }
    for (e, body) in &arena {
        if let Ok((t, v)) = poses.get(e) {
            bodies.push(pose(body.0, t, v));
        }
    }
    let (phase, phase_time) = match s.phase {
        battle::Phase::Setup => (0, 0.),
        battle::Phase::Introduction(t) => (0, t),
        battle::Phase::Countdown(t) => (1, t),
        battle::Phase::Playing => (2, 0.),
        battle::Phase::Finished(_) => (3, 0.),
    };
    let f = Frame {
        input_generation: net.input_generation,
        epoch: net.epoch,
        tick: net.tick,
        phase,
        phase_time,
        paused: net.paused,
        remaining: s.remaining,
        scores: s.score,
        eliminated: s.eliminated,
        winners: s.winners,
        bodies,
        robots,
        traps: s
            .hazards_runtime
            .iter()
            .map(|t| Trap {
                index: t.index as u16,
                clock: t.clock,
                active: t.active,
                occupied: t.occupied,
                zapping: t.zapping.clone(),
            })
            .collect(),
    };
    net.send(Request::Frame(f));
}
/// Native commands are consumed once per fixed physics step. The exact processed
/// sequence and Wiring state are included in the snapshot captured after writeback.
#[derive(Default)]
pub(crate) struct PeerInput {
    pub buffer: arena_net::native_input::CommandBuffer,
    pub edges: Vec<u32>,
    last_packet: u64,
}
#[derive(Resource, Default)]
pub(crate) struct AuthoritativeInputs {
    pub generation: u64,
    pub epoch: u64,
    pub peers: BTreeMap<u8, PeerInput>,
}
#[allow(clippy::too_many_arguments)]
pub(super) fn controls(
    net: Res<Multiplayer>,
    mut s: ResMut<battle::Session>,
    nav: Res<Navigation>,
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time<Real>>,
    mut input: ResMut<AuthoritativeInputs>,
) {
    if !s.network || !net.host {
        return;
    }
    if input.generation != net.input_generation || input.epoch != net.epoch {
        input.peers.clear();
        input.generation = net.input_generation;
        input.epoch = net.epoch;
    }
    if s.net_waiting || s.net_paused {
        return;
    }
    let playing = matches!(s.phase, battle::Phase::Playing);
    let local = s.local_slot;
    let eliminated = s.eliminated;
    for r in &mut s.robots {
        let slot = r.id as usize - 1;
        r.running = playing
            && !eliminated[slot]
            && r.damage.health.parts.first().is_some_and(|p| p.alive());
        let Some(doc) = &r.document else {
            continue;
        };
        if slot == local {
            let samples =
                crate::editor::controls::samples(doc, &keys, nav.page == Page::Match && r.running);
            r.commands = r.controls.evaluate(doc, &samples);
            continue;
        }
        if !playing {
            continue;
        }
        let peer = input.peers.entry(slot as u8).or_default();
        let mut live = false;
        if let Some((w, at)) = net.inputs.get(&(slot as u8))
            && w.generation == net.input_generation
        {
            live = time.elapsed_secs_f64() - at < 0.25;
            if w.sequence > peer.last_packet {
                if peer.buffer.receive(w).is_ok() {
                    peer.last_packet = w.sequence;
                }
            }
        }
        let c = peer.buffer.advance(live && r.running);
        r.commands = r
            .controls
            .evaluate_wire(doc, &c.levels, &c.edges, &mut peer.edges);
    }
}
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(super) fn apply(
    mut commands: Commands,
    mut net: ResMut<Multiplayer>,
    mut s: ResMut<battle::Session>,
    time: Res<Time<Real>>,
    mut nav: ResMut<Navigation>,
    mut params: ParamSet<(
        Query<(&Transform, &Velocity, &ReadWorldMassProperties), With<RobotBody>>,
        Query<(&mut Transform, &mut Velocity)>,
    )>,
    arena: Query<(Entity, &ArenaBody)>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    if !s.network || net.host || !net.loaded_sent {
        return;
    }
    let Some((latest, at)) = net.frames.back().cloned() else {
        return;
    };
    let target = latest.tick as f64 + ((time.elapsed_secs_f64() - at) * 120.).min(6.) - 8.;
    let mut shown = latest.clone();
    if let Some((upper, _)) = net.frames.iter().find(|(f, _)| f.tick as f64 >= target) {
        let lower = net
            .frames
            .iter()
            .rev()
            .find(|(f, _)| f.tick as f64 <= target)
            .map(|(f, _)| f)
            .unwrap_or(upper);
        let t = if upper.tick == lower.tick {
            1.
        } else {
            ((target - lower.tick as f64) / (upper.tick - lower.tick) as f64).clamp(0., 1.) as f32
        };
        shown = upper.clone();
        for b in &mut shown.bodies {
            if let Some(a) = lower.bodies.iter().find(|a| a.id == b.id) {
                b.position = Vec3::from_array(a.position)
                    .lerp(Vec3::from_array(b.position), t)
                    .to_array();
                b.rotation = Quat::from_array(a.rotation)
                    .slerp(Quat::from_array(b.rotation), t)
                    .to_array();
            }
        }
    }
    if !latest.paused {
        for r in &mut s.robots {
            r.damage.health.advance(time.delta_secs());
        }
    }
    if latest.tick > net.last_frame_tick {
        s.phase = match latest.phase {
            0 => battle::Phase::Setup,
            1 => battle::Phase::Countdown(latest.phase_time),
            2 => battle::Phase::Playing,
            _ => battle::Phase::Finished(latest.winners.iter().position(|v| *v)),
        };
        s.remaining = latest.remaining;
        s.score = latest.scores;
        s.eliminated = latest.eliminated;
        s.winners = latest.winners;
        if latest.tick >= net.pause_barrier_tick {
            s.net_paused = latest.paused;
            net.paused = latest.paused;
        }
        let playing = matches!(s.phase, battle::Phase::Playing);
        let has_new_detach = latest.robots.iter().any(|r| {
            s.robots
                .iter()
                .find(|b| b.id == r.slot as u32 + 1)
                .is_some_and(|b| {
                    r.detached
                        .iter()
                        .any(|i| !b.damage.detached_roots.contains(&(*i as usize)))
                })
        });
        if has_new_detach {
            for r in &s.robots {
                for (i, e) in r.bodies.iter().enumerate() {
                    if let Some(p) = latest
                        .bodies
                        .iter()
                        .find(|p| p.id == id(r.id as usize - 1, i, false))
                        && let Ok((mut t, mut v)) = params.p1().get_mut(*e)
                    {
                        t.translation = Vec3::from_array(p.position);
                        t.rotation = Quat::from_array(p.rotation);
                        v.linear = Vec3::from_array(p.linear);
                        v.angular = Vec3::from_array(p.angular);
                    }
                }
            }
        }
        for state in &latest.robots {
            let Some(robot) = s.robots.iter_mut().find(|r| r.id == state.slot as u32 + 1) else {
                continue;
            };
            for &root in &state.detached {
                if !robot.damage.detached_roots.contains(&(root as usize)) {
                    crate::gameplay::damage::detach(
                        &mut commands,
                        robot,
                        &params.p0(),
                        root as usize,
                    );
                }
            }
            for (p, hp) in robot.damage.health.parts.iter_mut().zip(&state.hp) {
                p.current = hp.min(p.maximum);
            }
            if let Some(rig) = &mut robot.rig {
                rig.electrical.remaining = state.energy.min(rig.electrical.capacity);
                rig.pneumatic.remaining = state.air.min(rig.pneumatic.capacity);
            }
            for (m, v) in robot.states.iter_mut().zip(&state.motors) {
                m.position = v[0];
                m.burst_time = v[1];
                m.initialized = v[2] > 0.;
                m.returning = v[3] > 0.;
            }
            robot.commands = state
                .commands
                .iter()
                .map(|c| arena_core::mechanism::Command {
                    drive: c[0],
                    steer: c[1],
                    fire: c[2] > 0.,
                })
                .collect();
            robot.damage.health.numbers = state
                .numbers
                .iter()
                .map(|n| arena_core::health::DamageNumber {
                    amount: n.hp,
                    position: Vec3::from_array(n.position),
                    age: n.age,
                    source: n.source,
                })
                .collect();
            robot.running = playing && !latest.eliminated[state.slot as usize] && !latest.paused;
        }
        for t in &latest.traps {
            if let Some(trap) = s
                .hazards_runtime
                .iter_mut()
                .find(|trap| trap.index == t.index as usize)
            {
                trap.clock = t.clock;
                trap.active = t.active;
                trap.occupied = t.occupied;
                trap.zapping = t.zapping.clone();
            }
        }
        if latest.phase == 3 {
            nav.page = Page::Results;
        }
        net.last_frame_tick = latest.tick;
        s.revision += 1;
    }
    // Match stable IDs to local entities; no Bevy entity ID crosses the connection.
    let mut entities = BTreeMap::new();
    for r in &s.robots {
        for (fragment, list) in [(false, &r.bodies), (true, &r.damage.fragments)] {
            for (i, &e) in list.iter().enumerate() {
                entities.insert(id(r.id as usize - 1, i, fragment), e);
            }
        }
    }
    entities.extend(arena.iter().map(|(e, n)| (n.0, e)));
    let mut target_poses = params.p1();
    for b in &shown.bodies {
        if let Some(e) = entities.get(&b.id)
            && let Ok((mut t, mut v)) = target_poses.get_mut(*e)
        {
            t.translation = Vec3::from_array(b.position);
            t.rotation = Quat::from_array(b.rotation);
            v.linear = Vec3::from_array(b.linear);
            v.angular = Vec3::from_array(b.angular);
        }
    }
    for skin in std::mem::take(&mut net.skins) {
        if skin.epoch == net.epoch
            && let Some(shell) = s
                .robots
                .iter_mut()
                .find(|r| r.id == skin.slot as u32 + 1)
                .and_then(|r| r.damage.shell.as_mut())
        {
            if let Err(e) = shell.restore_positions(skin.revision, &skin.positions, &mut meshes) {
                net.error = Some(e);
            }
        }
    }
}
