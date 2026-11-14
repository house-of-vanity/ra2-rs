//! Rapier contact -> absolute native HP. Contact attribution follows compound
//! subshape identities, not proximity to whichever component happens to be near.
use super::*;
use arena_core::health::{Health, impact_damage, impact_inverse_mass};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone)]
pub(super) struct Solid {
    pub component: usize,
    pub position: Vec3,
    pub rotation: Quat,
    pub collider: Collider,
}
#[derive(Default)]
pub(super) struct BodyParts {
    pub solids: Vec<Solid>,
    pub visuals: Vec<(usize, Entity)>,
    pub masses: Vec<(usize, f32)>,
    pub wheels: Vec<(usize, f32)>,
}
#[derive(Clone, Copy)]
struct ImpactBody {
    velocity: Velocity,
    properties: ReadWorldMassProperties,
}
impl Default for ImpactBody {
    fn default() -> Self {
        Self {
            velocity: Velocity::zero(),
            properties: ReadWorldMassProperties {
                center_of_mass: Vec3::ZERO,
                effective_inv_mass: Vec3::ZERO,
                effective_world_inv_inertia: Mat3::ZERO,
            },
        }
    }
}
impl ImpactBody {
    fn velocity(&self, point: Vec3) -> Vec3 {
        self.velocity
            .linear_velocity_at_point(point, self.properties.center_of_mass)
    }
    fn inverse_mass(&self, point: Vec3, normal: Vec3) -> f32 {
        let r = (point - self.properties.center_of_mass).cross(normal);
        let inverse_linear = normal.dot(self.properties.effective_inv_mass * normal);
        impact_inverse_mass(
            if inverse_linear > 0. {
                inverse_linear.recip()
            } else {
                0.
            },
            r.dot(self.properties.effective_world_inv_inertia * r),
        )
    }
}
pub(super) struct Supply {
    pub component: usize,
    pub electricity: f32,
    pub electric_rate: f32,
    pub air: f32,
    pub air_rate: f32,
}
#[derive(Default)]
pub(super) struct State {
    pub health: Health,
    pub shell: Option<super::surface::Shell>,
    pub supplies: Vec<Supply>,
    /// Other damage sources (hazards, scripted weapons) use the same HP path.
    pub pending: Vec<(usize, f32, Vec3)>,
    pub bodies: Vec<BodyParts>,
    pub fragments: Vec<Entity>,
    before: BTreeMap<Entity, ImpactBody>,
    touching: BTreeSet<(Entity, [i8; 3])>,
    assembly: ImpactBody,
    age: f32,
    pub view: Mat4,
    pub aspect: f32,
}
impl State {
    pub(super) fn connected_properties(&self) -> ReadWorldMassProperties {
        self.assembly.properties
    }
}
#[allow(clippy::type_complexity)]
pub(super) fn capture(
    mut garage: ResMut<Garage>,
    bodies: Query<(
        Entity,
        &Velocity,
        &ReadWorldMassProperties,
        &ReadMassProperties,
        &Transform,
        &RapierContextEntityLink,
    )>,
) {
    if !garage.active {
        return;
    }
    let context = garage.context;
    garage.damage.before.clear();
    garage.damage.before.extend(
        bodies
            .iter()
            .filter(|(_, _, _, _, _, c)| c.0 == context)
            .map(|(e, v, p, _, _, _)| {
                (
                    e,
                    ImpactBody {
                        velocity: *v,
                        properties: *p,
                    },
                )
            }),
    );
    // Connected bodies react together. Using only a light wheel/hammer's inverse
    // inertia can reduce a whole robot striking a wall to < 1 joule.
    let whole: bevy_rapier3d::rapier::dynamics::MassProperties =
        garage
            .bodies
            .iter()
            .enumerate()
            .filter(|(i, _)| !garage.damage.bodies[*i].masses.is_empty())
            .filter_map(|(_, e)| bodies.get(*e).ok())
            .map(|(_, _, _, local, t, _)| {
                local.get().into_rapier().transform_by(
                    &bevy_rapier3d::rapier::math::Pose::from_parts(t.translation, t.rotation),
                )
            })
            .sum();
    garage.damage.assembly = ImpactBody {
        velocity: Velocity::zero(),
        properties: ReadWorldMassProperties {
            center_of_mass: whole.local_com,
            effective_inv_mass: Vec3::splat(whole.inv_mass),
            effective_world_inv_inertia: whole.reconstruct_inverse_inertia_matrix(),
        },
    };
}
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(super) fn contacts(
    mut commands: Commands,
    mut garage: ResMut<Garage>,
    editor: Res<Editor>,
    nav: Res<Navigation>,
    contexts: Query<(
        &RapierContextSimulation,
        &RapierContextColliders,
        &RapierRigidBodySet,
    )>,
    poses: Query<(&Transform, &Velocity, &ReadWorldMassProperties), With<GarageBody>>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    if !garage.active || editor.mode != Mode::Test || nav.page != Page::Workshop {
        return;
    }
    let Ok((simulation, colliders, rigid_bodies)) = contexts.get(garage.context) else {
        return;
    };
    let dt = crate::STEP as f32;
    garage.damage.age += dt;
    garage.damage.health.advance(dt);
    let mut touching = BTreeSet::new();
    // One physical impact against one surface, independent of the number of
    // wheels, compound children, contact points and arena triangles involved.
    let mut hits = BTreeMap::<(Entity, [i8; 3]), (usize, f32, Vec3, Vec3)>::new();
    for pair in simulation
        .contact_pairs(colliders, rigid_bodies)
        .filter(|p| p.has_any_active_contact())
    {
        let (Some(a), Some(b)) = (pair.collider1(), pair.collider2()) else {
            continue;
        };
        let body_a = garage.bodies.iter().position(|e| *e == a);
        let body_b = garage.bodies.iter().position(|e| *e == b);
        if body_a.is_none() && body_b.is_none() {
            continue;
        }
        let va = garage.damage.before.get(&a).copied().unwrap_or_default();
        let vb = garage.damage.before.get(&b).copied().unwrap_or_default();
        for manifold in pair.manifolds() {
            let n = manifold.normal();
            let pose1 = colliders.colliders[pair.raw.collider1].position();
            let pose2 = colliders.colliders[pair.raw.collider2].position();
            for contact in manifold.points() {
                let p1 = manifold.subshape_pose1().map_or(contact.local_p1(), |t| {
                    t.transform_point(contact.local_p1())
                });
                let p2 = manifold.subshape_pose2().map_or(contact.local_p2(), |t| {
                    t.transform_point(contact.local_p2())
                });
                let p = (*pose1 * p1 + *pose2 * p2) * 0.5;
                let speed = (va.velocity(p) - vb.velocity(p)).dot(n).max(0.);
                // Include speculative/CCD contacts reached during this step.
                if contact.dist() > speed * dt + 0.005 {
                    continue;
                }
                let ia = if body_a.is_some() {
                    garage.damage.assembly.inverse_mass(p, n)
                } else {
                    va.inverse_mass(p, n)
                };
                let ib = if body_b.is_some() {
                    garage.damage.assembly.inverse_mass(p, n)
                } else {
                    vb.inverse_mass(p, n)
                };
                let amount = impact_damage(speed, ia + ib, garage.tuning.impact_damage_scale);
                for (body, subshape, entity, other) in [
                    (body_a, manifold.subshape1(), a, b),
                    (body_b, manifold.subshape2(), b, a),
                ] {
                    let Some(body) = body else {
                        continue;
                    };
                    let Some(solid) = garage.damage.bodies[body].solids.get(subshape as usize)
                    else {
                        continue;
                    };
                    let outward = if entity == a { n } else { -n };
                    // Floor and walls share one collider in the original garage.
                    let sector = outward.to_array().map(|v| (v * 4.).round() as i8);
                    let key = (other, sector);
                    touching.insert(key);
                    if amount > 0. && !garage.damage.touching.contains(&key) {
                        let hit = hits.entry(key).or_insert((solid.component, 0., p, outward));
                        if amount > hit.1 {
                            *hit = (solid.component, amount, p, outward);
                        }
                    }
                }
            }
        }
    }
    garage.damage.touching = touching;
    if garage.damage.age < 0.6 {
        return;
    }
    let mut broken = vec![];
    for (component, amount, point) in std::mem::take(&mut garage.damage.pending) {
        if let Some(e) = garage.damage.health.apply(component, amount, point)
            && e.destroyed
            && component > 0
        {
            broken.push(component);
        }
    }
    for (_, (component, amount, point, normal)) in hits {
        // A mounted part transmits structural shock to the hull. Split the same
        // damage budget; never count a wheel hit as two full-strength impacts.
        let hull = if component == 0 { amount } else { amount * 0.5 };
        if component > 0
            && let Some(e) = garage.damage.health.apply(component, amount - hull, point)
            && e.destroyed
        {
            broken.push(component);
        }
        if let Some(e) = garage.damage.health.apply(0, hull, point)
            && let Some(root) = garage.bodies.first()
            && let Ok((pose, _, _)) = poses.get(*root)
        {
            let local = pose.to_matrix().inverse();
            if let Some(shell) = &mut garage.damage.shell {
                shell.impact(
                    local.transform_point3(point),
                    local.transform_vector3(normal).normalize(),
                    e.amount,
                    &mut meshes,
                );
            }
        }
    }
    for component in broken {
        detach(&mut commands, &mut garage, &poses, component);
    }
}

fn detach(
    commands: &mut Commands,
    garage: &mut Garage,
    poses: &Query<(&Transform, &Velocity, &ReadWorldMassProperties), With<GarageBody>>,
    root: usize,
) {
    let Some(robot) = &garage.document else {
        return;
    };
    let detached: BTreeSet<_> = garage
        .damage
        .health
        .disconnect(robot, root)
        .into_iter()
        .collect();
    if detached.is_empty() {
        return;
    }
    let owner = garage
        .rig
        .as_ref()
        .map_or(0, |rig| rig.component_bodies[root]);
    let Some((pose, velocity, properties)) =
        garage.bodies.get(owner).and_then(|e| poses.get(*e).ok())
    else {
        return;
    };
    let pose = *pose;
    let mut velocity = *velocity;
    let inverse = pose.to_matrix().inverse();
    let mut solids = vec![];
    let mut visuals = vec![];
    let mut mass = 0_f32;
    let mut detached_properties = bevy_rapier3d::rapier::dynamics::MassProperties::default();
    let mut motions = Vec::new();
    let mut owned_bodies = BTreeSet::new();
    for (index, parts) in garage.damage.bodies.iter_mut().enumerate() {
        let changed = parts.solids.iter().any(|s| detached.contains(&s.component))
            || parts.visuals.iter().any(|(c, _)| detached.contains(c))
            || parts.masses.iter().any(|(c, _)| detached.contains(c));
        if !changed {
            continue;
        }
        let entity = garage.bodies[index];
        let Ok((body_pose, body_velocity, body_properties)) = poses.get(entity) else {
            continue;
        };
        let delta = inverse * body_pose.to_matrix();
        let lost_solids = parts
            .solids
            .iter()
            .filter(|s| detached.contains(&s.component))
            .cloned()
            .collect::<Vec<_>>();
        let lost_masses = parts
            .masses
            .iter()
            .filter(|(c, _)| detached.contains(c))
            .copied()
            .collect::<Vec<_>>();
        let (_, r, t) = delta.to_scale_rotation_translation();
        detached_properties += super::mass::properties(&lost_solids, &lost_masses)
            .into_rapier()
            .transform_by(&bevy_rapier3d::rapier::math::Pose::from_parts(t, r));
        parts.solids.retain(|s| {
            if !detached.contains(&s.component) {
                return true;
            }
            let (_, r, p) = (delta * Mat4::from_rotation_translation(s.rotation, s.position))
                .to_scale_rotation_translation();
            solids.push((p, r, s.collider.clone()));
            false
        });
        parts.visuals.retain(|&(component, visual)| {
            if !detached.contains(&component) {
                return true;
            }
            visuals.push((visual, Transform::from_matrix(delta)));
            false
        });
        parts.masses.retain(|&(component, m)| {
            if detached.contains(&component) {
                mass += m;
                motions.push((m, *body_velocity, body_properties.center_of_mass));
                false
            } else {
                true
            }
        });
        // Rebuild the surviving compound once on break, never per frame.
        if parts.solids.is_empty()
            && parts.visuals.is_empty()
            && parts.masses.is_empty()
            && index > 0
        {
            owned_bodies.insert(index);
            commands.entity(entity).remove::<Collider>();
        } else {
            commands.entity(entity).insert((
                if parts.solids.is_empty() {
                    Collider::ball(0.005)
                } else {
                    Collider::compound(
                        parts
                            .solids
                            .iter()
                            .map(|s| (s.position, s.rotation, s.collider.clone()))
                            .collect(),
                    )
                },
                ColliderMassProperties::MassProperties(super::mass::properties(
                    &parts.solids,
                    &parts.masses,
                )),
            ));
        }
    }
    if solids.is_empty() {
        return;
    }
    let collider = Collider::compound(solids);
    let centre = pose.transform_point(detached_properties.local_com);
    velocity.linear = velocity.linear_velocity_at_point(centre, properties.center_of_mass);
    if mass > 0. {
        // Rigidify the broken branch once. A moving output/hammer contributes
        // its motion too; inheriting only the chassis velocity would freeze it.
        velocity.linear = motions
            .iter()
            .map(|(m, v, c)| v.linear_velocity_at_point(centre, *c) * *m)
            .sum::<Vec3>()
            / mass;
        velocity.angular = motions.iter().map(|(m, v, _)| v.angular * *m).sum::<Vec3>() / mass;
    }
    let fragment = commands
        .spawn((
            Name::new(format!("Detached assembly {root}")),
            GarageBody,
            RigidBody::Dynamic,
            collider,
            ColliderMassProperties::MassProperties(MassProperties::from_rapier(
                detached_properties,
            )),
            pose,
            velocity,
            ReadWorldMassProperties::default(),
            Ccd::enabled(),
            Sleeping::default(),
            Friction::coefficient(0.5),
            CollisionGroups::new(Group::GROUP_1, Group::ALL),
            RapierContextEntityLink(garage.context),
        ))
        .id();
    for (visual, transform) in visuals {
        commands
            .entity(visual)
            .insert((ChildOf(fragment), transform));
    }
    if let Some(rig) = &garage.rig {
        for j in &rig.joints {
            if detached.contains(&j.component) || owned_bodies.contains(&j.child) {
                commands
                    .entity(garage.bodies[j.child])
                    .remove::<(ImpulseJoint, drive::Motor)>();
            }
        }
    }
    for index in owned_bodies {
        commands
            .entity(garage.bodies[index])
            .insert(RigidBody::Fixed);
    }
    garage.damage.fragments.push(fragment);
}
