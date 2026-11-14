//! Ordinary rigid bodies only. A hammer contact produces one damage event.
use super::*;
use bevy_rapier3d::rapier::{math::Pose, prelude::*};

pub struct Hammer {
    pub body: RigidBodyHandle,
    collider: ColliderHandle,
    pub target: Aim,
    power: f32,
    life: f32,
    contacted: bool,
}

pub struct RigidWorld {
    pub world: PhysicsWorld,
    pub robot: RigidBodyHandle,
    collider: ColliderHandle,
    pub hammer: Option<Hammer>,
    pub step_ms: f64,
    pub detached: Vec<Detached>,
    owners: Vec<Option<usize>>,
    parents: Vec<Option<usize>>,
    shapes: Vec<Vec<(Pose, SharedShape)>>,
    masses: Vec<f32>,
}

pub struct Detached {
    pub body: RigidBodyHandle,
    collider: ColliderHandle,
    pub parts: Vec<usize>,
    pivot: Vec3,
    release: f32,
}

/// BOT parent links describe attachment ownership, not individual mesh nodes.
fn descends_from(mut part: usize, root: usize, parents: &[Option<usize>]) -> bool {
    for _ in 0..parents.len() {
        if part == root {
            return true;
        }
        let Some(parent) = parents[part] else {
            return false;
        };
        part = parent;
    }
    false
}

impl RigidWorld {
    pub fn new(loaded: &content::Loaded) -> Result<Self, String> {
        let content = &loaded.content;
        let mut world = PhysicsWorld::new();
        world.configure_thread_pool(1).map_err(|e| e.to_string())?;
        world.integration_parameters.num_solver_iterations = 4;
        let arena = content
            .arena
            .object("h_box")
            .ok_or("Garage collision mesh missing")?;
        let terrain = SharedShape::trimesh(
            arena
                .positions()
                .map(|p| p + content.arena_offset)
                .collect(),
            arena
                .faces
                .iter()
                .map(|f| f.vertices.map(|i| i as u32))
                .collect(),
        )
        .map_err(|e| e.to_string())?;
        world.insert(
            RigidBodyBuilder::fixed(),
            ColliderBuilder::new(terrain).friction(0.7),
        );
        let mut shapes = vec![Vec::new(); content.robot.components.len()];
        for i in std::iter::once(0).chain(content.part_indices.iter().copied()) {
            let transform = content.assembly.parts[i].transform;
            let (_, rotation, translation) = transform.to_scale_rotation_translation();
            for (_, collider) in arena_runtime::robot::collision_parts(&content.models[i]) {
                shapes[i].push((
                    Pose::from_parts(translation, rotation.normalize()),
                    collider.raw.clone(),
                ));
            }
        }
        // Very thin authored parts sometimes have no volumetric proxy. Their
        // local bounds provide a conservative thin collider, never a new mesh.
        for b in &loaded.boxes {
            if shapes[b.component].is_empty() {
                let center = b.transform.transform_point3((b.min + b.max) * 0.5);
                let (_, rotation, _) = b.transform.to_scale_rotation_translation();
                let half = (b.max - b.min) * 0.5;
                shapes[b.component].push((
                    Pose::from_parts(center, rotation.normalize()),
                    SharedShape::cuboid(half.x, half.y, half.z),
                ));
            }
        }
        let compound: Vec<_> = shapes.iter().flatten().cloned().collect();
        if compound.is_empty() {
            return Err("Robot has no rigid collision geometry".into());
        }
        let masses: Vec<_> = content
            .assembly
            .parts
            .iter()
            .map(|p| p.mass.max(0.01))
            .collect();
        let mass = masses.iter().sum::<f32>();
        let (robot, collider) = world.insert(
            RigidBodyBuilder::dynamic()
                .translation(content.origin)
                .linear_damping(0.4)
                .angular_damping(0.7)
                .ccd_enabled(true),
            ColliderBuilder::new(SharedShape::compound(compound))
                .mass(mass)
                .collision_groups(InteractionGroups::new(
                    Group::GROUP_1,
                    Group::ALL,
                    InteractionTestMode::And,
                ))
                .friction(0.8)
                .restitution(0.02),
        );
        Ok(Self {
            world,
            robot,
            collider,
            hammer: None,
            step_ms: 0.0,
            detached: Vec::new(),
            owners: vec![None; masses.len()],
            parents: content.robot.components.iter().map(|p| p.parent).collect(),
            shapes,
            masses,
        })
    }
    pub fn transform(&self) -> Transform {
        let body = &self.world.bodies[self.robot];
        Transform::from_translation(body.translation()).with_rotation(*body.rotation())
    }
    pub fn attached(&self, component: usize) -> bool {
        self.owners[component].is_none()
    }
    /// Map the original assembled chassis coordinates into the current world.
    pub fn frame(&self, component: usize) -> Transform {
        if let Some(group) = self.owners[component] {
            let group = &self.detached[group];
            let body = &self.world.bodies[group.body];
            Transform::from_translation(body.translation() - *body.rotation() * group.pivot)
                .with_rotation(*body.rotation())
        } else {
            self.transform()
        }
    }
    pub fn aim_frame(&self, aim: Aim) -> Transform {
        self.frame(match aim.target {
            Target::Shell(_) => 0,
            Target::Component(i) => i,
        })
    }
    fn target_collider(&self, aim: Aim) -> ColliderHandle {
        match aim.target {
            Target::Component(i) if self.owners[i].is_some() => {
                self.detached[self.owners[i].unwrap()].collider
            }
            _ => self.collider,
        }
    }
    /// Ancestors fail first when one dent damages both a mount and its child.
    /// A previously detached branch is never split as a side effect.
    pub fn detach_failed(
        &mut self,
        boxes: &[arena_core::damage::DamageBox],
        aim: Aim,
        power: f32,
    ) -> usize {
        let failed: Vec<_> = boxes
            .iter()
            .filter(|b| b.component != 0 && b.damage >= 100. && self.attached(b.component))
            .map(|b| b.component)
            .collect();
        let roots: Vec<_> = failed
            .iter()
            .copied()
            .filter(|&i| {
                !failed
                    .iter()
                    .any(|&other| other != i && descends_from(i, other, &self.parents))
            })
            .collect();
        let frame = self.aim_frame(aim);
        let point = frame.transform_point(aim.point);
        let impulse_direction = -(frame.rotation * aim.normal);
        for root in &roots {
            let parts: Vec<_> = (1..self.owners.len())
                .filter(|&i| self.attached(i) && descends_from(i, *root, &self.parents))
                .collect();
            let pivot = self.shapes[*root]
                .first()
                .map_or(Vec3::ZERO, |(p, _)| p.translation);
            let compound: Vec<_> = parts
                .iter()
                .flat_map(|&i| {
                    self.shapes[i].iter().map(|(pose, shape)| {
                        (
                            Pose::from_parts(pose.translation - pivot, pose.rotation),
                            shape.clone(),
                        )
                    })
                })
                .collect();
            // A damaged visible component always has a proxy or bounds fallback.
            if compound.is_empty() {
                continue;
            }
            let robot = &self.world.bodies[self.robot];
            let pose = *robot.position();
            let old_com = robot.center_of_mass();
            let velocity = robot.linvel();
            let angular = robot.angvel();
            let mass: f32 = parts.iter().map(|&i| self.masses[i]).sum();
            let (body, collider) = self.world.insert(
                RigidBodyBuilder::dynamic()
                    .pose(Pose::from_parts(pose.transform_point(pivot), pose.rotation))
                    .angular_damping(0.3)
                    .linear_damping(0.15)
                    .ccd_enabled(true),
                ColliderBuilder::new(SharedShape::compound(compound))
                    .mass(mass)
                    .friction(0.8)
                    .restitution(0.05)
                    // Mount proxies may overlap at their socket while welded.
                    // Briefly ignore only the host, retaining floor/hammer contacts.
                    .collision_groups(InteractionGroups::new(
                        Group::GROUP_2,
                        Group::ALL & !Group::GROUP_1,
                        InteractionTestMode::And,
                    )),
            );
            self.world.bodies[body].recompute_mass_properties_from_colliders(&self.world.colliders);
            let com = self.world.bodies[body].center_of_mass();
            self.world.bodies[body].set_linvel(velocity + angular.cross(com - old_com), true);
            self.world.bodies[body].set_angvel(angular, true);
            self.world.bodies[body].apply_impulse_at_point(
                impulse_direction * mass * power * 0.5,
                point,
                true,
            );
            let owner = self.detached.len();
            for &i in &parts {
                self.owners[i] = Some(owner);
            }
            self.detached.push(Detached {
                body,
                collider,
                parts,
                pivot,
                release: 0.15,
            });
            let remaining: Vec<_> = self
                .shapes
                .iter()
                .enumerate()
                .filter(|(i, _)| self.attached(*i))
                .flat_map(|(_, p)| p.iter().cloned())
                .collect();
            let remaining_mass: f32 = self
                .masses
                .iter()
                .enumerate()
                .filter(|(i, _)| self.attached(*i))
                .map(|(_, m)| *m)
                .sum();
            let collider = &mut self.world.colliders[self.collider];
            collider.set_shape(SharedShape::compound(remaining));
            collider.set_mass(remaining_mass);
            let robot = &mut self.world.bodies[self.robot];
            robot.recompute_mass_properties_from_colliders(&self.world.colliders);
            robot.set_linvel(
                velocity + angular.cross(robot.center_of_mass() - old_com),
                true,
            );
            robot.wake_up(true);
        }
        roots.len()
    }
    pub fn strike(&mut self, aim: Aim, power: f32) {
        if self.hammer.is_some() {
            return;
        }
        let t = self.aim_frame(aim);
        let point = t.transform_point(aim.point);
        let normal = t.rotation * aim.normal;
        let rotation = Quat::from_rotation_arc(Vec3::Y, normal);
        let (body, collider) = self.world.insert(
            RigidBodyBuilder::dynamic()
                .pose(Pose::from_parts(point + normal * 0.65, rotation))
                .linvel(-normal * (7.0 + power * 3.0))
                .gravity_scale(0.0)
                .ccd_enabled(true),
            ColliderBuilder::cuboid(0.17, 0.10, 0.13).mass(12.0 * power),
        );
        self.hammer = Some(Hammer {
            body,
            collider,
            target: aim,
            power,
            life: 0.7,
            contacted: false,
        });
    }
    pub fn step(&mut self, dt: f32) -> Option<(Aim, f32)> {
        for group in &mut self.detached {
            if group.release > 0. {
                group.release -= dt;
                if group.release <= 0. {
                    self.world.colliders[group.collider].set_collision_groups(
                        InteractionGroups::new(
                            Group::GROUP_2,
                            Group::ALL,
                            InteractionTestMode::And,
                        ),
                    );
                }
            }
        }
        self.world.integration_parameters.dt = dt;
        let start = std::time::Instant::now();
        self.world.step();
        self.step_ms = start.elapsed().as_secs_f64() * 1000.0;
        let mut result = None;
        let target = self.hammer.as_ref().map(|h| self.target_collider(h.target));
        if let Some(hammer) = &mut self.hammer {
            hammer.life -= dt;
            if !hammer.contacted
                && self
                    .world
                    .narrow_phase
                    .contact_pairs_with(hammer.collider)
                    .any(|pair| {
                        pair.has_any_active_contact()
                            && (Some(pair.collider1) == target || Some(pair.collider2) == target)
                    })
            {
                hammer.contacted = true;
                result = Some((hammer.target, hammer.power));
            }
            if hammer.life <= 0.0 {
                let hammer = self.hammer.take().unwrap();
                self.world.remove_body(hammer.body);
            }
        }
        result
    }
}

/// Exercises real authored proxies and BOT links in the headless catalogue check.
pub(super) fn check_detachment(loaded: &content::Loaded) -> Result<(), String> {
    let mut physics = RigidWorld::new(loaded)?;
    let mut boxes = loaded.boxes.clone();
    for b in &mut boxes {
        b.damage = 0.;
    }
    let Some(first) = boxes
        .iter()
        .find(|b| {
            !boxes.iter().any(|other| {
                other.component != b.component
                    && descends_from(other.component, b.component, &physics.parents)
            })
        })
        .map(|b| b.component)
    else {
        return Ok(());
    };
    physics.world.bodies[physics.robot].set_position(
        Pose::from_parts(
            Vec3::new(1., 4., -1.),
            Quat::from_euler(EulerRot::YXZ, 0.4, 0.2, -0.3),
        ),
        true,
    );
    physics.world.bodies[physics.robot].set_linvel(Vec3::new(1., 0.2, -0.3), true);
    physics.world.bodies[physics.robot].set_angvel(Vec3::new(0.1, -0.25, 0.3), true);
    let old_mass = physics.world.bodies[physics.robot].mass();
    let old_com = physics.world.bodies[physics.robot].center_of_mass();
    let old_velocity = physics.world.bodies[physics.robot].linvel();
    let old_angular = physics.world.bodies[physics.robot].angvel();
    let frame = physics.frame(first).to_matrix();
    let aim = Aim {
        target: Target::Component(first),
        point: Vec3::ZERO,
        normal: Vec3::Y,
        distance: 1.,
    };
    boxes
        .iter_mut()
        .find(|b| b.component == first)
        .unwrap()
        .damage = 99.;
    assert_eq!(
        physics.detach_failed(&boxes, aim, 0.),
        0,
        "Detached before 100% damage"
    );
    boxes
        .iter_mut()
        .find(|b| b.component == first)
        .unwrap()
        .damage = 100.;
    assert_eq!(physics.detach_failed(&boxes, aim, 0.), 1);
    assert_eq!(
        physics.detached[0].parts,
        [first],
        "Breaking a leaf removed its parent or siblings"
    );
    let body = physics.detached[0].body;
    assert!(
        physics.frame(first).to_matrix().abs_diff_eq(frame, 1e-4),
        "Detachment teleported the component"
    );
    let rigid = &physics.world.bodies[body];
    assert!(
        rigid.linvel().abs_diff_eq(
            old_velocity + old_angular.cross(rigid.center_of_mass() - old_com),
            1e-4
        ),
        "Detached body did not inherit velocity at its COM"
    );
    let preserved = physics.detached[0].parts.clone();
    if let Some(root) = boxes
        .iter()
        .find(|b| {
            physics.attached(b.component)
                && boxes.iter().any(|child| {
                    child.component != b.component
                        && physics.attached(child.component)
                        && descends_from(child.component, b.component, &physics.parents)
                })
        })
        .map(|b| b.component)
    {
        let expected: Vec<_> = (1..physics.owners.len())
            .filter(|&i| physics.attached(i) && descends_from(i, root, &physics.parents))
            .collect();
        boxes
            .iter_mut()
            .find(|b| b.component == root)
            .unwrap()
            .damage = 100.;
        assert_eq!(physics.detach_failed(&boxes, aim, 0.), 1);
        assert_eq!(
            physics.detached.last().unwrap().parts,
            expected,
            "Failed mount did not retain its entire branch as one body"
        );
        assert!(
            boxes
                .iter()
                .filter(|b| b.component != root && expected.contains(&b.component))
                .all(|b| b.damage == 0.),
            "Parent destruction changed healthy children's HP"
        );
    }
    // Fail all remaining attachments simultaneously. Each highest failed
    // ancestor creates one body; healthy descendants retain their HP and links.
    for b in &mut boxes {
        if physics.attached(b.component) {
            b.damage = 100.;
        }
    }
    let expected_roots: Vec<_> = boxes
        .iter()
        .filter(|b| {
            physics.attached(b.component)
                && !boxes.iter().any(|p| {
                    p.component != b.component
                        && physics.attached(p.component)
                        && descends_from(b.component, p.component, &physics.parents)
                })
        })
        .map(|b| b.component)
        .collect();
    assert_eq!(physics.detach_failed(&boxes, aim, 0.), expected_roots.len());
    assert_eq!(physics.detached[0].body, body);
    assert_eq!(
        physics.detached[0].parts, preserved,
        "Breaking a parent split an already detached branch"
    );
    assert!(boxes.iter().all(|b| !physics.attached(b.component)));
    assert_eq!(
        physics.detach_failed(&boxes, aim, 0.),
        0,
        "Repeated damage duplicated detached bodies"
    );
    let remaining = physics.world.colliders[physics.collider]
        .shape()
        .as_compound()
        .unwrap()
        .shapes()
        .len();
    assert_eq!(
        remaining,
        physics.shapes[0].len(),
        "Robot retained ghost collision shapes"
    );
    let mass = physics.world.bodies[physics.robot].mass()
        + physics
            .detached
            .iter()
            .map(|g| physics.world.bodies[g.body].mass())
            .sum::<f32>();
    assert!(
        (mass - old_mass).abs() < old_mass * 0.0001,
        "Mass was duplicated during detachment"
    );
    assert_eq!(physics.world.bodies.len(), 2 + physics.detached.len());
    assert_eq!(physics.world.impulse_joints.len(), 0);
    for _ in 0..240 {
        physics.step(1. / 120.);
    }
    assert!(
        physics.world.quarantine().is_empty(),
        "Detached assembly became unstable"
    );
    for group in &physics.detached {
        let body = &physics.world.bodies[group.body];
        assert!(body.translation().is_finite() && body.linvel().is_finite());
        assert!(
            body.translation().y > -1.,
            "Detached assembly fell through the native floor"
        );
        assert!(
            body.translation().length() < 100.,
            "Detached body exploded away from the robot"
        );
        assert!(
            physics.world.colliders[group.collider]
                .collision_groups()
                .test(physics.world.colliders[physics.collider].collision_groups()),
            "Host collisions were not restored"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn attachment_ownership_follows_parent_chain_not_component_order() {
        let parents = [None, Some(4), Some(1), Some(0), Some(0), Some(4)];
        let branch: Vec<_> = (1..parents.len())
            .filter(|&i| descends_from(i, 4, &parents))
            .collect();
        assert_eq!(branch, [1, 2, 4, 5]);
        assert!(!descends_from(4, 1, &parents));
        assert!(!descends_from(3, 4, &parents));
        assert!(!descends_from(1, 0, &[None, Some(2), Some(1)]));
    }
}
