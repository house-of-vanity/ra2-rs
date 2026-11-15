//! The production drive/contact systems over a render-free clone of the loaded
//! native collision scene. No test cart, custom wheel forces, HP or effect systems.
use crate::{
    gameplay::{self, RobotBody, RobotInstance},
    garage::Garage,
};
use arena_core::{
    health::Health,
    mechanism::{ActuatorState, Rig},
};
#[cfg(test)]
use arena_net::session::Robot;
use arena_net::session::{Frame, InputCommand, Pose};
use bevy::{prelude::*, time::TimeUpdateStrategy};
use bevy_rapier3d::prelude::*;
use std::collections::BTreeMap;
#[derive(Clone)]
struct Body {
    id: u32,
    source: Entity,
    transform: Transform,
    kind: RigidBody,
    collider: Option<Collider>,
    mass: Option<ColliderMassProperties>,
    friction: Option<Friction>,
    restitution: Option<Restitution>,
    groups: Option<CollisionGroups>,
    damping: Option<Damping>,
    sleeping: Option<Sleeping>,
    ccd: Option<Ccd>,
    iterations: Option<AdditionalSolverIterations>,
    sensor: bool,
    disabled: bool,
    gravity: Option<GravityScale>,
    axes: Option<LockedAxes>,
    joint: Option<ImpulseJoint>,
    motor: Option<usize>,
    tyres: Option<gameplay::tyres::Tyres>,
}
#[derive(Clone)]
pub(super) struct Blueprint {
    bodies: Vec<Body>,
    pub slot: usize,
    rig: Rig,
    document: arena_core::robot::RobotDefinition,
    tuning: crate::config::GarageConfig,
    parts: Vec<gameplay::damage::BodyParts>,
    supplies: Vec<gameplay::damage::Supply>,
    health: Vec<arena_core::health::PartHealth>,
    configuration: RapierConfiguration,
    integration: bevy_rapier3d::rapier::dynamics::IntegrationParameters,
    pub topology: Vec<usize>,
    pub world_topology: Vec<(u32, Vec<usize>, usize)>,
}
impl Blueprint {
    pub(super) fn topology(w: &World) -> Vec<(u32, Vec<usize>, usize)> {
        w.get_resource::<crate::battle::Session>()
            .map(|s| {
                s.robots
                    .iter()
                    .map(|r| {
                        (
                            r.id,
                            r.damage.detached_roots.clone(),
                            r.damage.fragments.len(),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
    pub(super) fn capture(w: &mut World) -> Result<Self, String> {
        let s = w.resource::<crate::battle::Session>();
        let local = s.local_slot;
        let r = s
            .robots
            .iter()
            .find(|r| r.id as usize == local + 1)
            .ok_or("Local assembly missing")?;
        Self::capture_robot(w, r)
    }
    pub(super) fn capture_robot(w: &World, r: &RobotInstance) -> Result<Self, String> {
        let rig = r.rig.clone().ok_or("Native rig missing")?;
        let doc = r.document.clone().ok_or("Native BOT missing")?;
        let mut ids = BTreeMap::new();
        if let Some(s) = w.get_resource::<crate::battle::Session>() {
            for other in &s.robots {
                for (fragment, list) in [(false, &other.bodies), (true, &other.damage.fragments)] {
                    for (i, e) in list.iter().enumerate() {
                        ids.insert(
                            *e,
                            super::super::world::id(other.id as usize - 1, i, fragment),
                        );
                    }
                }
            }
        } else {
            for (fragment, list) in [(false, &r.bodies), (true, &r.damage.fragments)] {
                for (i, e) in list.iter().enumerate() {
                    ids.insert(*e, super::super::world::id(r.id as usize - 1, i, fragment));
                }
            }
        }
        let mut bodies = vec![];
        let mut static_index = 0;
        for entity in w.iter_entities() {
            if entity
                .get::<RapierContextEntityLink>()
                .is_none_or(|c| c.0 != r.context)
            {
                continue;
            }
            let Some(kind) = entity.get::<RigidBody>() else {
                continue;
            };
            let e = entity.id();
            let id = ids
                .get(&e)
                .copied()
                .or_else(|| entity.get::<super::super::world::ArenaBody>().map(|b| b.0))
                .unwrap_or_else(|| {
                    let n = 0xc000_0000 + static_index;
                    static_index += 1;
                    n
                });
            let local = r.bodies.contains(&e);
            let transform = entity
                .get::<GlobalTransform>()
                .map(|g| Transform::from_matrix(g.to_matrix()))
                .or_else(|| entity.get::<Transform>().copied())
                .unwrap_or_default();
            bodies.push(Body {
                id,
                source: e,
                transform,
                kind: if local {
                    *kind
                } else {
                    RigidBody::KinematicPositionBased
                },
                collider: entity.get::<Collider>().cloned(),
                mass: entity.get::<ColliderMassProperties>().copied(),
                friction: entity.get::<Friction>().copied(),
                restitution: entity.get::<Restitution>().copied(),
                groups: entity.get::<CollisionGroups>().copied(),
                damping: entity.get::<Damping>().copied(),
                sleeping: entity.get::<Sleeping>().copied(),
                ccd: entity.get::<Ccd>().copied(),
                iterations: entity.get::<AdditionalSolverIterations>().copied(),
                sensor: entity.contains::<Sensor>(),
                disabled: entity.contains::<ColliderDisabled>(),
                gravity: entity.get::<GravityScale>().copied(),
                axes: entity.get::<LockedAxes>().copied(),
                joint: if local {
                    entity.get::<ImpulseJoint>().copied()
                } else {
                    None
                },
                motor: if local {
                    entity.get::<gameplay::drive::Motor>().map(|m| m.0)
                } else {
                    None
                },
                tyres: if local {
                    entity.get::<gameplay::tyres::Tyres>().cloned()
                } else {
                    None
                },
            });
        }
        bodies.sort_by_key(|b| b.id);
        if bodies.is_empty() || bodies.len() > 1024 {
            return Err("Invalid native prediction scene size".into());
        }
        Ok(Self {
            bodies,
            slot: r.id as usize - 1,
            rig,
            document: doc,
            tuning: r.tuning.clone(),
            parts: r.damage.bodies.clone(),
            supplies: r.damage.supplies.clone(),
            health: r.damage.health.parts.clone(),
            configuration: *w
                .get::<RapierConfiguration>(r.context)
                .ok_or("Native physics configuration missing")?,
            integration: w
                .get::<RapierContextSimulation>(r.context)
                .ok_or("Native physics context missing")?
                .integration_parameters,
            topology: r.damage.detached_roots.clone(),
            world_topology: Self::topology(w),
        })
    }
}
pub(super) struct Scene {
    pub app: App,
    context: Entity,
    ids: BTreeMap<u32, Entity>,
    pub edges: Vec<u32>,
    proxies: Vec<(Entity, Pose)>,
    steps: u64,
    pub source: Blueprint,
}
impl Scene {
    pub(super) fn new(source: Blueprint) -> Result<Self, String> {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, TransformPlugin))
            .insert_resource(TimeUpdateStrategy::ManualDuration(
                std::time::Duration::from_secs_f64(crate::STEP),
            ))
            .insert_resource(TimestepMode::Fixed {
                dt: crate::STEP as f32,
                substeps: 1,
            })
            .add_plugins(RapierPhysicsPlugin::<NoUserData>::default().in_schedule(Update))
            .add_systems(
                Update,
                (
                    gameplay::drive::motors,
                    gameplay::damage::capture,
                    gameplay::tyres::contacts,
                )
                    .chain()
                    .before(PhysicsSet::SyncBackend),
            );
        let mut sim = RapierContextSimulation::default();
        sim.integration_parameters = source.integration;
        let mut config = source.configuration;
        config.physics_pipeline_active = true;
        config.num_threads = Some(1);
        let context = app.world_mut().spawn((sim, config)).id();
        let mut ids = BTreeMap::new();
        let mut entities = BTreeMap::new();
        for b in &source.bodies {
            let mut e = app.world_mut().spawn((
                b.kind,
                b.transform,
                Velocity::zero(),
                ReadWorldMassProperties::default(),
                RapierContextEntityLink(context),
            ));
            macro_rules! component {
                ($field:ident) => {
                    if let Some(v) = &b.$field {
                        e.insert(v.clone());
                    }
                };
            }
            component!(collider);
            component!(mass);
            component!(friction);
            component!(restitution);
            component!(groups);
            component!(damping);
            component!(sleeping);
            component!(ccd);
            component!(iterations);
            component!(gravity);
            component!(axes);
            component!(tyres);
            if b.sensor {
                e.insert(Sensor);
            }
            if b.disabled {
                e.insert(ColliderDisabled);
            }
            if b.id >> 16 == source.slot as u32 && b.id & 0xffff < 0x8000 {
                e.insert((RobotBody, ExternalImpulse::default()));
            }
            if let Some(m) = b.motor {
                e.insert(gameplay::drive::Motor(m));
            }
            ids.insert(b.id, e.id());
            entities.insert(b.source, e.id());
        }
        for b in &source.bodies {
            if let Some(j) = &b.joint {
                let parent = *entities
                    .get(&j.parent)
                    .ok_or("Native joint parent missing")?;
                app.world_mut().entity_mut(ids[&b.id]).insert(ImpulseJoint {
                    parent,
                    data: j.data,
                });
            }
        }
        let mut robot = RobotInstance::new(source.slot as u32 + 1, context, source.tuning.clone());
        robot.active = true;
        robot.document = Some(source.document.clone());
        robot.rig = Some(source.rig.clone());
        robot.states = (0..source.rig.joints.len())
            .map(|_| ActuatorState::default())
            .collect();
        robot.bodies = (0..source.rig.bodies.len())
            .map(|i| ids[&super::super::world::id(source.slot, i, false)])
            .collect();
        robot.damage.bodies = source.parts.clone();
        robot.damage.supplies = source.supplies.clone();
        robot.damage.health = Health {
            parts: source.health.clone(),
            numbers: vec![],
            received: 0.,
        };
        // Main-world render entities have no identity in this collision-only world.
        for parts in &mut robot.damage.bodies {
            parts.visuals.clear();
        }
        robot.damage.detached_roots = source.topology.clone();
        app.insert_resource(Garage::simulation(robot));
        app.finish();
        app.cleanup();
        Ok(Self {
            app,
            context,
            ids,
            edges: vec![],
            proxies: vec![],
            steps: 0,
            source,
        })
    }
    pub(super) fn restore(&mut self, frame: &Frame) -> Result<(), String> {
        frame
            .validate()
            .map_err(|e| format!("Invalid native checkpoint: {e:?}"))?;
        let r = frame
            .robots
            .iter()
            .find(|r| r.slot as usize == self.source.slot)
            .ok_or("Own checkpoint missing")?;
        if r.hp.len() != self.source.health.len() || r.motors.len() != self.source.rig.joints.len()
        {
            return Err("Incompatible native robot checkpoint".into());
        }
        let control = &r.controls;
        if !control.previous.is_empty()
            && (control.previous.len() != self.source.document.controllers.len()
                || control
                    .previous
                    .iter()
                    .zip(&self.source.document.controllers)
                    .any(|(row, c)| row.len() != c.keys.len()))
        {
            return Err("Incompatible native controls checkpoint".into());
        }
        if self.ids.keys().any(|id| {
            *id >> 16 == self.source.slot as u32
                && *id & 0xffff < 0x8000
                && !frame.bodies.iter().any(|b| b.id == *id)
        }) {
            return Err("Incomplete native body checkpoint".into());
        }
        let w = self.app.world_mut();
        self.proxies.clear();
        self.steps = 0;
        for b in &self.source.bodies {
            let pose = frame.bodies.iter().find(|p| p.id == b.id);
            if let Some(p) = pose {
                let mut e = w.entity_mut(self.ids[&b.id]);
                e.insert((
                    Transform::from_translation(Vec3::from_array(p.position))
                        .with_rotation(Quat::from_array(p.rotation)),
                    Velocity {
                        linear: Vec3::from_array(p.linear),
                        angular: Vec3::from_array(p.angular),
                    },
                    ExternalImpulse::default(),
                ));
                if b.kind == RigidBody::KinematicPositionBased {
                    self.proxies.push((e.id(), p.clone()));
                }
            }
            w.entity_mut(self.ids[&b.id]).remove::<(
                RapierRigidBodyHandle,
                RapierColliderHandle,
                RapierImpulseJointHandle,
            )>();
        }
        let mut sim = RapierContextSimulation::default();
        sim.integration_parameters = self.source.integration;
        w.entity_mut(self.context).insert((
            sim,
            RapierRigidBodySet::default(),
            RapierContextColliders::default(),
            RapierContextJoints::default(),
            SimulationToRenderTime::default(),
        ));
        {
            let mut garage = w.resource_mut::<Garage>();
            let rig = garage.rig.as_mut().unwrap();
            rig.electrical.remaining = r.energy;
            rig.pneumatic.remaining = r.air;
            for (s, m) in garage.states.iter_mut().zip(&r.motors) {
                *s = ActuatorState {
                    position: m[0],
                    burst_time: m[1],
                    initialized: m[2] > 0.,
                    returning: m[3] > 0.,
                };
            }
            garage.controls.restore(&r.controls)?;
            garage.commands = r
                .commands
                .iter()
                .map(|c| arena_core::mechanism::Command {
                    drive: c[0],
                    steer: c[1],
                    fire: c[2] > 0.,
                })
                .collect();
            for (p, hp) in garage.damage.health.parts.iter_mut().zip(&r.hp) {
                p.current = *hp;
            }
            garage.running = false;
        }
        self.edges = r.edges.clone();
        // Collision-only bootstrap rebuilds mass properties and the ground sensor BVH
        // without advancing bodies, actuators or resources by an extra simulation step.
        w.get_mut::<RapierConfiguration>(self.context)
            .unwrap()
            .simulation_mode = SimulationMode::CollisionOnly;
        self.app.update();
        self.app
            .world_mut()
            .get_mut::<RapierConfiguration>(self.context)
            .unwrap()
            .simulation_mode = SimulationMode::Full;
        // Collision-only consumes the "inserted/modified" flags without registering
        // dynamic/kinematic bodies with the island manager. Re-mark the complete set
        // before returning to dynamics, including fixed opponent/platform proxies.
        for (_, body) in self
            .app
            .world_mut()
            .get_mut::<RapierRigidBodySet>(self.context)
            .unwrap()
            .bodies
            .iter_mut()
        {
            body.wake_up(true);
        }

        Ok(())
    }
    pub(super) fn step(&mut self, c: &InputCommand) {
        self.steps += 1;
        let dt = (self.steps as f32 * crate::STEP as f32).min(0.05);
        for (e, p) in &self.proxies {
            let rotation = (Quat::from_scaled_axis(Vec3::from_array(p.angular) * dt)
                * Quat::from_array(p.rotation))
            .normalize();
            self.app.world_mut().entity_mut(*e).insert(
                Transform::from_translation(
                    Vec3::from_array(p.position) + Vec3::from_array(p.linear) * dt,
                )
                .with_rotation(rotation),
            );
        }
        {
            let mut g = self.app.world_mut().resource_mut::<Garage>();
            g.running = true;
            let doc = g.document.as_ref().unwrap().clone();
            g.commands = g
                .controls
                .evaluate_wire(&doc, &c.levels, &c.edges, &mut self.edges);
        }
        self.app.update();
    }
    pub(super) fn poses(&self) -> Vec<Pose> {
        self.ids
            .iter()
            .filter(|(id, _)| **id >> 16 == self.source.slot as u32 && **id & 0xffff < 0x8000)
            .filter_map(|(id, e)| {
                let t = self.app.world().get::<Transform>(*e)?;
                let v = self.app.world().get::<Velocity>(*e)?;
                Some(Pose {
                    id: *id,
                    position: t.translation.to_array(),
                    rotation: t.rotation.to_array(),
                    linear: v.linear.to_array(),
                    angular: v.angular.to_array(),
                })
            })
            .collect()
    }
    pub(super) fn robot(&self) -> &RobotInstance {
        &self.app.world().resource::<Garage>().instance
    }
    #[cfg(test)]
    pub(super) fn checkpoint(&self, ack: u64) -> Robot {
        let r = self.robot();
        Robot {
            slot: self.source.slot as u8,
            hp: r.damage.health.parts.iter().map(|p| p.current).collect(),
            energy: r.rig.as_ref().unwrap().electrical.remaining,
            air: r.rig.as_ref().unwrap().pneumatic.remaining,
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
            detached: self.source.topology.iter().map(|i| *i as u16).collect(),
            numbers: vec![],
            shell_revision: 0,
            controls: r.controls.snapshot(),
            acknowledged: ack,
            edges: self.edges.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires original BOT/CFZ/garage assets; no window or GPU"]
    fn native_prediction_moving_proxy_is_bounded_and_cannot_commit_damage() {
        let (mut source, mut frame) = crate::multiplayer::prediction::tests::source("Walrus");
        let root = Vec3::from_array(frame.bodies[0].position);
        let start = root + Vec3::NEG_Z * 3.;
        let mut proxy = source
            .bodies
            .iter()
            .find(|b| b.id >= 0xc000_0000)
            .unwrap()
            .clone();
        proxy.id = 0x10000;
        proxy.kind = RigidBody::KinematicPositionBased;
        proxy.collider = Some(Collider::cuboid(5., 2., 0.1));
        proxy.transform = Transform::from_translation(start);
        proxy.groups = Some(CollisionGroups::new(Group::GROUP_3, Group::ALL));
        proxy.joint = None;
        proxy.motor = None;
        proxy.tyres = None;
        source.bodies.push(proxy);
        let mut p = Pose {
            id: 0x10000,
            position: start.to_array(),
            rotation: Quat::IDENTITY.to_array(),
            linear: [0., 0., 0.5],
            angular: [0.; 3],
        };
        frame.bodies.push(p.clone());
        let mut scene = Scene::new(source).unwrap();
        let hp = frame.robots[0].hp.clone();
        for _ in 0..10 {
            scene.restore(&frame).unwrap();
            for sequence in 1..=24 {
                scene.step(&InputCommand {
                    sequence,
                    levels: vec![],
                    edges: vec![],
                });
            }
            let t = scene
                .app
                .world()
                .get::<Transform>(scene.ids[&p.id])
                .unwrap();
            assert!(
                t.translation
                    .distance(Vec3::from_array(p.position) + Vec3::Z * 0.025)
                    < 0.00001
            );
            assert_eq!(scene.checkpoint(24).hp, hp);
            assert!(
                scene
                    .poses()
                    .iter()
                    .all(|p| p.position.iter().all(|v| v.is_finite()))
            );
            p.position[0] += 0.01;
            frame.bodies.last_mut().unwrap().position = p.position;
        }
    }
}
