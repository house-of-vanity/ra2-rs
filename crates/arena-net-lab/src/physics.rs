//! Real Rapier stress fixture, using the game's shared actuator and health arithmetic.
//! This is an instrumented articulated rig, not yet the native BOT/Bevy adapter.
use arena_core::{
    JointKind, Quat, Vec3,
    mechanism::{ActuatorKind, ActuatorState, Command, RigJoint},
};
use arena_net::{input::AppliedInput, prediction::Simulation, snapshot::*, *};
use rapier3d::prelude::*;
use std::collections::BTreeMap;
struct Robot {
    root: RigidBodyHandle,
    arm: RigidBodyHandle,
    tip: RigidBodyHandle,
    joint: Option<ImpulseJointHandle>,
    state: RobotState,
    actuator: ActuatorState,
}
pub struct Physics {
    pub world: PhysicsWorld,
    robots: Vec<Robot>,
    tick: Tick,
    sim_tick: Tick,
    paused: bool,
    touching: std::collections::BTreeSet<(usize, usize)>,
    pub contacts: u64,
    pub shots: u64,
    pub breaks: u64,
}
fn motor() -> RigJoint {
    RigJoint {
        component: 1,
        parent: 0,
        child: 1,
        anchor: Vec3::ZERO,
        axis: Vec3::Y,
        child_rotation: Quat::IDENTITY,
        kind: ActuatorKind::Burst,
        motion: JointKind::Hinge {
            limited: true,
            limits: [-0.9, 0.9],
            friction: 0.,
        },
        limits: Some([-0.9, 0.9]),
        speed: 14.,
        return_speed: 3.,
        force: 100.,
        burst_force: 400.,
        electricity: 2.,
        air: 0.,
        start: -0.9,
        stop: 0.9,
    }
}
impl Physics {
    pub fn new(players: usize) -> Self {
        assert!((2..=4).contains(&players));
        let mut world = PhysicsWorld::default();
        world.integration_parameters.dt = DT;
        world.insert(
            RigidBodyBuilder::fixed().translation(Vec3::new(0., -0.25, 0.)),
            ColliderBuilder::cuboid(10., 0.25, 10.).friction(0.6),
        );
        for (p, shape) in [
            (Vec3::new(0., 1., -8.), Vec3::new(10., 1., 0.15)),
            (Vec3::new(0., 1., 8.), Vec3::new(10., 1., 0.15)),
            (Vec3::new(-8., 1., 0.), Vec3::new(0.15, 1., 10.)),
            (Vec3::new(8., 1., 0.), Vec3::new(0.15, 1., 10.)),
        ] {
            world.insert(
                RigidBodyBuilder::fixed().translation(p),
                ColliderBuilder::cuboid(shape.x, shape.y, shape.z).friction(0.4),
            );
        }
        let mut robots = vec![];
        for i in 0..players {
            let p = Vec3::new(
                if i % 2 == 0 { -2.5 } else { 2.5 },
                0.35,
                (i / 2) as f32 * 2.5,
            );
            let (root, _) = world.insert(
                RigidBodyBuilder::dynamic()
                    .translation(p)
                    .linear_damping(0.5)
                    .angular_damping(3.)
                    .ccd_enabled(true),
                ColliderBuilder::cuboid(0.55, 0.3, 0.65)
                    .mass(30.)
                    .friction(0.35),
            );
            let (arm, _) = world.insert(
                RigidBodyBuilder::dynamic()
                    .translation(p + Vec3::new(0., 0.15, -0.95))
                    .ccd_enabled(true),
                ColliderBuilder::cuboid(0.15, 0.15, 0.5)
                    .mass(3.)
                    .friction(0.3),
            );
            let joint = world.impulse_joints.insert(
                root,
                arm,
                RevoluteJointBuilder::new(Vec3::Y)
                    .local_anchor1(Vec3::new(0., 0.15, -0.55))
                    .local_anchor2(Vec3::new(0., 0., 0.4))
                    .limits([-0.9, 0.9])
                    .motor_position(-0.9, 150., 20.)
                    .motor_max_force(400.)
                    .contacts_enabled(false),
                true,
            );
            let (tip, _) = world.insert(
                RigidBodyBuilder::dynamic()
                    .translation(p + Vec3::new(0., 0.15, -1.6))
                    .ccd_enabled(true),
                ColliderBuilder::cuboid(0.35, 0.18, 0.18).mass(2.),
            );
            world.impulse_joints.insert(
                arm,
                tip,
                FixedJointBuilder::new()
                    .local_anchor1(Vec3::new(0., 0., -0.5))
                    .local_anchor2(Vec3::new(0., 0., 0.15))
                    .contacts_enabled(false),
                true,
            );
            robots.push(Robot {
                root,
                arm,
                tip,
                joint: Some(joint),
                state: RobotState {
                    owner: PeerId(i as u64),
                    hp: vec![500., 1.],
                    energy: 10000.,
                    air: 1000.,
                    score: 0.,
                    revision: 0,
                    action: 0,
                    motors: vec![MotorState::default()],
                },
                actuator: Default::default(),
            });
        }
        Self {
            world,
            robots,
            tick: 0,
            sim_tick: 0,
            paused: false,
            touching: Default::default(),
            contacts: 0,
            shots: 0,
            breaks: 0,
        }
    }
    fn snapshot_body(&self, i: usize, part: usize) -> BodyState {
        let r = &self.robots[i];
        let b = &self.world.bodies[match part {
            0 => r.root,
            1 => r.arm,
            _ => r.tip,
        }];
        BodyState {
            id: BodyId((i * 3 + part) as u32),
            owner: r.state.owner,
            parent: match part {
                1 if r.joint.is_some() => Some(BodyId((i * 3) as u32)),
                2 => Some(BodyId((i * 3 + 1) as u32)),
                _ => None,
            },
            position: b.translation(),
            rotation: *b.rotation(),
            linear: b.linvel(),
            angular: b.angvel(),
        }
    }
    pub fn joint_frames(&self) -> Vec<JointFrames> {
        (0..self.robots.len())
            .flat_map(|i| {
                [
                    JointFrames {
                        child: BodyId((i * 3 + 1) as u32),
                        parent: BodyId((i * 3) as u32),
                        parent_anchor: Vec3::new(0., 0.15, -0.55),
                        child_anchor: Vec3::new(0., 0., 0.4),
                    },
                    JointFrames {
                        child: BodyId((i * 3 + 2) as u32),
                        parent: BodyId((i * 3 + 1) as u32),
                        parent_anchor: Vec3::new(0., 0., -0.5),
                        child_anchor: Vec3::new(0., 0., 0.15),
                    },
                ]
            })
            .collect()
    }
}
impl Simulation for Physics {
    fn snapshot(&self) -> Snapshot {
        Snapshot {
            tick: self.tick,
            sim_tick: self.sim_tick,
            paused: self.paused,
            bodies: (0..self.robots.len())
                .flat_map(|i| {
                    [
                        self.snapshot_body(i, 0),
                        self.snapshot_body(i, 1),
                        self.snapshot_body(i, 2),
                    ]
                })
                .collect(),
            robots: self.robots.iter().map(|r| r.state.clone()).collect(),
        }
    }
    fn restore(&mut self, s: &Snapshot) -> Result<()> {
        s.validate()?;
        if s.robots.len() != self.robots.len() || s.bodies.len() != self.robots.len() * 3 {
            return Err(Error::Incompatible);
        }
        for i in 0..self.robots.len() {
            let owner = PeerId(i as u64);
            let r = s
                .robots
                .iter()
                .find(|r| r.owner == owner)
                .ok_or(Error::Incompatible)?;
            if r.motors.len() != 1 || r.hp.len() != 2 {
                return Err(Error::Incompatible);
            }
            if s.body(BodyId((i * 3 + 2) as u32))
                .is_none_or(|b| b.parent != Some(BodyId((i * 3 + 1) as u32)))
            {
                return Err(Error::Incompatible);
            }
            for id in [i * 3, i * 3 + 1, i * 3 + 2] {
                let b = s.body(BodyId(id as u32)).ok_or(Error::Incompatible)?;
                if b.owner != owner
                    || b.parent.is_some_and(|p| {
                        id == i * 3
                            || p != BodyId((if id == i * 3 + 1 { i * 3 } else { i * 3 + 1 }) as u32)
                    })
                {
                    return Err(Error::Incompatible);
                }
            }
        }
        // Solver islands, contacts and joint caches form one unit. Rebuild all of them.
        *self = Self::new(self.robots.len());
        self.tick = s.tick;
        self.sim_tick = s.sim_tick;
        self.paused = s.paused;
        for (i, r) in self.robots.iter_mut().enumerate() {
            r.state = s
                .robots
                .iter()
                .find(|p| p.owner == PeerId(i as u64))
                .ok_or(Error::Incompatible)?
                .clone();
            let m = &r.state.motors[0];
            r.actuator = ActuatorState {
                position: m.position,
                burst_time: m.burst_time,
                initialized: m.initialized,
                returning: m.returning,
            };
            let broken = s
                .body(BodyId((i * 3 + 1) as u32))
                .ok_or(Error::Incompatible)?
                .parent
                .is_none();
            if broken && let Some(j) = r.joint.take() {
                self.world.impulse_joints.remove(j, true);
            }
            if !broken && r.joint.is_none() {
                r.joint = Some(
                    self.world.impulse_joints.insert(
                        r.root,
                        r.arm,
                        RevoluteJointBuilder::new(Vec3::Y)
                            .local_anchor1(Vec3::new(0., 0.15, -0.55))
                            .local_anchor2(Vec3::new(0., 0., 0.4))
                            .limits([-0.9, 0.9])
                            .contacts_enabled(false),
                        true,
                    ),
                );
            }
            for (id, h) in [
                (BodyId((i * 3) as u32), r.root),
                (BodyId((i * 3 + 1) as u32), r.arm),
                (BodyId((i * 3 + 2) as u32), r.tip),
            ] {
                let b = s.body(id).ok_or(Error::Incompatible)?;
                let body = &mut self.world.bodies[h];
                body.set_position(Pose::from_parts(b.position, b.rotation), true);
                body.set_linvel(b.linear, true);
                body.set_angvel(b.angular, true);
                body.reset_forces(true);
                body.reset_torques(true);
            }
        }
        // Discard stale contacts/solver impulses after an authoritative correction.
        self.touching.clear();
        Ok(())
    }
    fn set_paused(&mut self, p: bool) {
        self.paused = p;
    }
    fn step(
        &mut self,
        tick: Tick,
        inputs: &BTreeMap<PeerId, AppliedInput>,
        authoritative: bool,
    ) -> Vec<EventKind> {
        self.tick = tick;
        if self.paused {
            return vec![];
        }
        self.sim_tick += 1;
        for r in &mut self.robots {
            let command = inputs.get(&r.state.owner).copied().unwrap_or_default();
            let (throttle, steer) = command.input.axes();
            r.state.action = r.state.action.max(command.input.action);
            let root = &mut self.world.bodies[r.root];
            let known = authoritative || inputs.contains_key(&r.state.owner);
            let forward = *root.rotation() * Vec3::X;
            let target = forward * (throttle * 6.);
            let current = root.linvel();
            let correction =
                (target - Vec3::new(current.x, 0., current.z)).clamp_length_max(12. * DT);
            if known {
                root.set_linvel(current + correction, true);
                root.set_angvel(
                    Vec3::new(root.angvel().x, steer * 2.5, root.angvel().z),
                    true,
                );
            }
            r.state.energy = (r.state.energy - (throttle.abs() + steer.abs()) * DT).max(0.);
            if let Some(joint) = r.joint {
                let a = self.world.bodies[r.root].rotation().inverse()
                    * self.world.bodies[r.arm].rotation();
                let measured = a.to_scaled_axis().y;
                let (position, velocity, _) = r.actuator.step(
                    &motor(),
                    Command {
                        fire: command.fire,
                        ..Default::default()
                    },
                    DT,
                    Some(measured),
                );
                if command.fire {
                    self.shots += 1;
                    r.state.energy = (r.state.energy - 5.).max(0.);
                }
                let data = &mut self.world.impulse_joints.get_mut(joint, true).unwrap().data;
                data.set_motor(JointAxis::AngX, position.unwrap_or(0.), velocity, 150., 20.);
                data.set_motor_max_force(JointAxis::AngX, 400.);
            }
            r.state.motors[0] = MotorState {
                position: r.actuator.position,
                burst_time: r.actuator.burst_time,
                initialized: r.actuator.initialized,
                returning: r.actuator.returning,
            };
        }
        let before = self
            .robots
            .iter()
            .map(|r| self.world.bodies[r.root].linvel())
            .collect::<Vec<_>>();
        self.world.step();
        let mut events = vec![];
        let mut touching = std::collections::BTreeSet::new();
        if authoritative {
            for pair in self
                .world
                .narrow_phase
                .contact_pairs()
                .filter(|p| p.has_any_active_contact())
            {
                let ca = self.world.colliders[pair.collider1].parent();
                let cb = self.world.colliders[pair.collider2].parent();
                let ia = self
                    .robots
                    .iter()
                    .position(|r| Some(r.root) == ca || Some(r.arm) == ca || Some(r.tip) == ca);
                let ib = self
                    .robots
                    .iter()
                    .position(|r| Some(r.root) == cb || Some(r.arm) == cb || Some(r.tip) == cb);
                if ia == ib {
                    continue;
                }
                for i in [ia, ib].into_iter().flatten() {
                    let other = if Some(i) == ia { ib } else { ia };
                    let key = (i, other.unwrap_or(usize::MAX));
                    touching.insert(key);
                    if self.touching.contains(&key) {
                        continue;
                    }
                    let speed = (before[i] - other.map_or(Vec3::ZERO, |j| before[j])).length();
                    let amount = arena_core::health::impact_damage(
                        speed,
                        if other.is_some() { 0.06 } else { 0.12 },
                        0.1,
                    );
                    if amount > 0.5 {
                        let r = &mut self.robots[i];
                        let own = if Some(i) == ia { ca } else { cb };
                        let part = usize::from(own != Some(r.root));
                        let hp = amount.min(r.state.hp[part]);
                        if hp <= 0. {
                            continue;
                        }
                        r.state.hp[part] -= hp;
                        self.contacts += 1;
                        events.push(EventKind::Damage {
                            victim: r.state.owner,
                            part: part as u16,
                            hp,
                        });
                        if let Some(j) = other {
                            self.robots[j].state.score += hp.floor();
                        }
                    }
                }
            }
            // A physical tool striking a wall can break its attachment; descendants remain rigid bodies.
            for i in 0..self.robots.len() {
                let r = &mut self.robots[i];
                if r.state.hp[1] <= 0.
                    && let Some(j) = r.joint.take()
                {
                    self.world.impulse_joints.remove(j, true);
                    r.state.hp[1] = 0.;
                    r.state.revision += 1;
                    self.breaks += 1;
                    events.push(EventKind::Detached {
                        body: BodyId((i * 3 + 1) as u32),
                    });
                }
            }
        }
        self.touching = touching;
        events
    }
}
