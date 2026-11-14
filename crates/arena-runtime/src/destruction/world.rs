//! Small reusable destruction world. Tools supply geometry/impulses; rendering and input are external.
use super::ArmourShell;
use arena_core::{Mat4, Vec3};
use bevy_rapier3d::rapier::{math::Pose, prelude::*};
use std::{sync::Mutex, time::Instant};

pub struct AttachedPart {
    pub transform: Mat4,
    pub shape: SharedShape,
    pub mass: f32,
    pub parent: Option<usize>,
}
pub struct Mount {
    pub joint: ImpulseJointHandle,
    pub body: RigidBodyHandle,
    pub strength: f32,
    pub broken: bool,
}
pub struct Hammer {
    pub body: RigidBodyHandle,
    pub collider: ColliderHandle,
    pub life: f32,
    pub contacted: bool,
}
#[derive(Clone, Copy)]
pub struct Impact {
    pub point: Vec3,
    pub normal: Vec3,
    pub power: f32,
}
pub struct DestructionWorld {
    pub world: PhysicsWorld,
    pub shell: ArmourShell,
    pub origin: Vec3,
    pub root: SoftBodyHandle,
    pub parts: Vec<RigidBodyHandle>,
    pub mounts: Vec<Mount>,
    pub hammer: Option<Hammer>,
    pub impacts: Vec<Impact>,
    pub cuts: usize,
    pub fractures: usize,
    pub elapsed: f32,
    pub last_step_ms: f64,
    pub unstable: bool,
}
#[derive(Default)]
struct Tears(Mutex<usize>);
impl EventHandler for Tears {
    fn handle_collision_event(
        &self,
        _: &RigidBodySet,
        _: &ColliderSet,
        _: CollisionEvent,
        _: Option<&ContactPair>,
    ) {
    }
    fn handle_contact_force_event(
        &self,
        _: f32,
        _: &RigidBodySet,
        _: &ColliderSet,
        _: &ContactPair,
        _: f32,
    ) {
    }
    fn handle_soft_body_tear_event(&self, _: &SoftBodySet, event: &SoftBodyTearEvent) {
        *self.0.lock().unwrap() += event.torn_cells.len() + event.torn_edges.len();
    }
}
impl DestructionWorld {
    pub fn new(
        shell: ArmourShell,
        origin: Vec3,
        mass: f32,
        parts: &[AttachedPart],
        fem: bool,
        iterations: usize,
    ) -> Result<Self, String> {
        let mut world = PhysicsWorld::new();
        world.configure_thread_pool(1).map_err(|e| e.to_string())?;
        world.integration_parameters.soft_bodies.fem.max_dense_dofs = 1400;
        world
            .integration_parameters
            .soft_bodies
            .fem
            .max_linear_iterations = 12;
        world.integration_parameters.soft_bodies.max_extra_substeps = 2;
        world.integration_parameters.soft_bodies.contact_stiffening = 1.0;
        let root = world.insert_soft_body(shell.builder(origin, mass, fem, iterations));
        let mut handles = Vec::new();
        let mut mounts = Vec::new();
        for (i, part) in parts.iter().enumerate() {
            let (_, rotation, position) = part.transform.to_scale_rotation_translation();
            let pose = Pose::from_parts(position + origin, rotation);
            let (body, _) = world.insert(
                RigidBodyBuilder::dynamic()
                    .pose(pose)
                    .can_sleep(false)
                    .linear_damping(0.08)
                    .angular_damping(0.15)
                    .ccd_enabled(true),
                ColliderBuilder::new(part.shape.clone())
                    .mass(part.mass.max(0.1))
                    .friction(0.7)
                    .collision_groups(InteractionGroups::new(
                        Group::GROUP_2,
                        Group::GROUP_1,
                        InteractionTestMode::And,
                    )),
            );
            let parent = if let Some(parent) = part.parent.filter(|p| *p < i) {
                handles[parent]
            } else {
                let mut nearest: Vec<_> = shell
                    .positions
                    .iter()
                    .enumerate()
                    .map(|(i, p)| (i as u32, p.distance_squared(position)))
                    .collect();
                nearest.sort_by(|a, b| a.1.total_cmp(&b.1));
                let mut particles: Vec<_> = nearest.iter().take(8).map(|(i, _)| *i).collect();
                particles.sort_unstable();
                let cluster = world
                    .soft_bodies
                    .add_cluster(root, &particles, &mut world.bodies, &mut world.colliders)
                    .ok_or("Cannot create armour mount")?;
                world.soft_bodies[root]
                    .cluster_proxy(cluster)
                    .ok_or("Armour mount has no frame")?
            };
            let frame = world.bodies[parent].position().inverse() * pose;
            let joint = world.insert_impulse_joint(
                parent,
                body,
                FixedJointBuilder::new()
                    .local_frame1(frame)
                    .contacts_enabled(false),
            );
            mounts.push(Mount {
                joint,
                body,
                strength: 45.0 + part.mass.sqrt() * 12.0,
                broken: false,
            });
            handles.push(body);
        }
        Ok(Self {
            world,
            shell,
            origin,
            root,
            parts: handles,
            mounts,
            hammer: None,
            impacts: vec![],
            cuts: 0,
            fractures: 0,
            elapsed: 0.0,
            last_step_ms: 0.0,
            unstable: false,
        })
    }

    pub fn hit(&self, origin: Vec3, direction: Vec3) -> Option<(ColliderHandle, Vec3, Vec3)> {
        let filter = QueryFilter {
            groups: Some(InteractionGroups::new(
                Group::GROUP_1,
                Group::GROUP_2,
                InteractionTestMode::And,
            )),
            ..Default::default()
        };
        let ray = Ray::new(origin, direction);
        let (collider, hit) = self
            .world
            .query_pipeline_with_filter(filter)
            .cast_ray_and_get_normal(&ray, 100.0, false)?;
        let normal = if hit.normal.dot(direction) > 0.0 {
            -hit.normal
        } else {
            hit.normal
        };
        Some((collider, origin + direction * hit.time_of_impact, normal))
    }

    pub fn strike(&mut self, point: Vec3, normal: Vec3, power: f32) {
        if self.hammer.is_some() {
            return;
        }
        let rotation = arena_core::Quat::from_rotation_arc(Vec3::Y, normal);
        let (body, collider) = self.world.insert(
            RigidBodyBuilder::dynamic()
                .pose(Pose::from_parts(point + normal * 0.65, rotation))
                .linvel(-normal * (7.0 + power * 4.0))
                .ccd_enabled(true)
                .gravity_scale(0.0),
            ColliderBuilder::cuboid(0.18, 0.12, 0.14)
                .mass(18.0 * power)
                .collision_groups(InteractionGroups::new(
                    Group::GROUP_1,
                    Group::GROUP_2,
                    InteractionTestMode::And,
                )),
        );
        self.hammer = Some(Hammer {
            body,
            collider,
            life: 0.65,
            contacted: false,
        });
    }

    /// The saw cuts only the material its finite blade crosses. A cut changes topology, not health.
    pub fn cut(&mut self, point: Vec3, normal: Vec3, tangent: Vec3, power: f32) -> bool {
        let particles: usize = self
            .world
            .soft_bodies
            .iter()
            .map(|(_, b)| b.num_particles())
            .sum();
        if self.unstable
            || self.cuts >= 192
            || particles > self.shell.positions.len() * 4 + 256
            || self.world.soft_bodies.len() >= 48
        {
            return false;
        }
        let side = tangent.normalize_or_zero();
        let depth = 0.12 + power * 0.07;
        let blade = [
            point + side * 0.28 + normal * 0.04,
            point - side * 0.28 + normal * 0.04,
            point - normal * depth,
        ];
        let handles: Vec<_> = self
            .world
            .soft_bodies
            .iter()
            .filter(|(_, b)| {
                b.particle_positions()
                    .any(|p| p.distance_squared(point) < 0.65 * 0.65)
            })
            .map(|(h, _)| h)
            .collect();
        let mut changed = false;
        for h in handles {
            if let Some(event) = self.world.cut_soft_body(h, &blade) {
                self.fractures += event.torn_cells.len() + event.torn_edges.len();
                self.cuts += 1;
                changed = true;
                for piece in event.bodies() {
                    if let Some(body) = self.world.soft_bodies.get_mut(piece) {
                        let nearby: Vec<_> = body
                            .particle_positions()
                            .enumerate()
                            .filter(|(_, p)| p.distance_squared(point) < 0.12 * 0.12)
                            .map(|(i, _)| i)
                            .collect();
                        for i in nearby {
                            body.apply_particle_impulse(i, normal * 0.2 + side * 0.03, true);
                        }
                    }
                }
            }
        }
        if changed {
            self.impacts.push(Impact {
                point,
                normal,
                power: 0.25,
            });
        }
        changed
    }

    pub fn step(&mut self, dt: f32, iterations: usize) {
        if self.unstable {
            return;
        }
        self.world.integration_parameters.dt = dt;
        self.world.integration_parameters.num_solver_iterations = iterations;
        let events = Tears::default();
        let start = Instant::now();
        self.world.step_with_events(&(), &events);
        self.last_step_ms = start.elapsed().as_secs_f64() * 1000.0;
        self.unstable = !self.world.quarantine().is_empty()
            || self.world.soft_bodies.iter().any(|(_, b)| {
                b.particles().iter().any(|p| {
                    !p.position().is_finite() || !p.velocity().length_squared().is_finite()
                })
            });
        if self.unstable {
            return;
        }
        self.fractures += events.0.into_inner().unwrap();
        self.elapsed += dt;
        if self.elapsed > 0.5 {
            for mount in &mut self.mounts {
                if !mount.broken
                    && self.world.impulse_joints.get(mount.joint).is_some_and(|j| {
                        j.impulses.iter().take(3).map(|v| v * v).sum::<f32>().sqrt()
                            > mount.strength
                    })
                {
                    self.world.remove_impulse_joint(mount.joint);
                    mount.broken = true;
                    if let Some(body) = self.world.bodies.get(mount.body) {
                        self.impacts.push(Impact {
                            point: body.translation(),
                            normal: Vec3::Y,
                            power: 0.5,
                        });
                    }
                }
            }
        }
        if let Some(hammer) = &mut self.hammer {
            hammer.life -= dt;
            if !hammer.contacted
                && self
                    .world
                    .narrow_phase
                    .contact_pairs_with(hammer.collider)
                    .any(|c| c.has_any_active_contact())
            {
                hammer.contacted = true;
                let body = &self.world.bodies[hammer.body];
                self.impacts.push(Impact {
                    point: body.translation(),
                    normal: body.rotation() * Vec3::Y,
                    power: 1.0,
                });
            }
            if hammer.life <= 0.0 {
                let h = self.hammer.take().unwrap();
                self.world.remove_body(h.body);
            }
        }
        if self.world.soft_bodies.len() >= 48 {
            for (_, b) in self.world.soft_bodies.iter_mut() {
                b.material_mut().tear_strain = None;
            }
        }
    }
}
