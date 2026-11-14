//! Grounded wheel forces: Rapier resolves normal contact and rigid joints.
//! Longitudinal drive, passive rolling and lateral slip are independent.
//! Native RA2 also uses a separate wheel-contact force pass (4dc13e/4856f0),
//! rather than relying on an isotropic rubber material for tank steering.
use super::*;
use std::collections::{BTreeMap, BTreeSet};

pub(super) struct Tread {
    pub component: usize,
    pub axle: Vec3,
    pub center: Vec3,
    pub ground_reach: f32,
    pub grip: f32,
    pub resistance: f32,
}
#[derive(Component, Default)]
pub(super) struct Tyres(pub Vec<Tread>);
#[derive(Default)]
struct Patch {
    point: Vec3,
    normal: Vec3,
    samples: f32,
}
fn inverse(properties: &ReadWorldMassProperties, p: Vec3, axis: Vec3) -> f32 {
    let r = (p - properties.center_of_mass).cross(axis);
    axis.dot(properties.effective_inv_mass * axis)
        + r.dot(properties.effective_world_inv_inertia * r)
}
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(super) fn contacts(
    mut commands: Commands,
    garage: Res<Garage>,
    editor: Res<Editor>,
    nav: Res<Navigation>,
    contexts: Query<(
        &RapierConfiguration,
        &RapierContextSimulation,
        &RapierContextColliders,
        &RapierRigidBodySet,
    )>,
    tyres: Query<&Tyres>,
    bodies: Query<(
        &Transform,
        Option<&Velocity>,
        Option<&ReadWorldMassProperties>,
    )>,
    mut impulses: Query<&mut ExternalImpulse>,
    mut joints: Query<&mut ImpulseJoint>,
) {
    if !garage.active || editor.mode != Mode::Test || nav.page != Page::Workshop {
        return;
    }
    let Ok((configuration, sim, colliders, rigids)) = contexts.get(garage.context) else {
        return;
    };
    // Contact geometry survives solver contact clustering. It also provides
    // edge support when the centre ray misses a narrow ramp or obstacle.
    if !configuration.physics_pipeline_active {
        return;
    }
    let mut patches = BTreeMap::<(Entity, usize, Entity), Patch>::new();
    for pair in sim.contact_pairs(colliders, rigids) {
        let (Some(a), Some(b)) = (pair.collider1(), pair.collider2()) else {
            continue;
        };
        for manifold in pair.manifolds() {
            for (wheel, other, subshape, up) in [
                (a, b, manifold.subshape1(), -manifold.normal()),
                (b, a, manifold.subshape2(), manifold.normal()),
            ] {
                if up.y < 0.4 {
                    continue;
                }
                let Ok(treads) = tyres.get(wheel) else {
                    continue;
                };

                let Some(body) = garage.bodies.iter().position(|e| *e == wheel) else {
                    continue;
                };
                let Some(solid) = garage.damage.bodies[body].solids.get(subshape as usize) else {
                    continue;
                };
                if !treads.0.iter().any(|t| t.component == solid.component)
                    || !garage.damage.health.operational(solid.component)
                {
                    continue;
                }
                for contact in manifold.points().filter(|c| c.dist() < 0.025) {
                    let p1 = manifold.subshape_pose1().map_or(contact.local_p1(), |t| {
                        t.transform_point(contact.local_p1())
                    });
                    let p2 = manifold.subshape_pose2().map_or(contact.local_p2(), |t| {
                        t.transform_point(contact.local_p2())
                    });
                    let p = (*colliders.colliders[pair.raw.collider1].position() * p1
                        + *colliders.colliders[pair.raw.collider2].position() * p2)
                        * 0.5;
                    let patch = patches.entry((wheel, solid.component, other)).or_default();
                    patch.point += p;
                    patch.normal += up;
                    patch.samples += 1.;
                }
            }
        }
    }
    // Native Wheel has a separate ground sensor (507f90/507ff0), not a
    // per-frame normal-impulse threshold. Use the original `contact` mesh's
    // radial reach for a centre probe; solid cylinders retain visible sizes.
    // This keeps traction continuous as support alternates between contacts.
    let predicate = |e, _: &bevy_rapier3d::rapier::geometry::Collider| !garage.bodies.contains(&e);
    let filter = QueryFilter::default()
        .exclude_sensors()
        .predicate(&predicate);
    RapierQueryPipeline::new_scoped(
        &sim.broad_phase,
        colliders,
        rigids,
        &filter,
        sim.query_dispatcher(),
        |pipeline| {
            for (body, entity) in garage.bodies.iter().enumerate() {
                let Ok(treads) = tyres.get(*entity) else {
                    continue;
                };
                let Ok((pose, _, _)) = bodies.get(*entity) else {
                    continue;
                };
                for tread in &treads.0 {
                    if !garage.damage.health.operational(tread.component)
                        || !garage.damage.bodies[body]
                            .solids
                            .iter()
                            .any(|s| s.component == tread.component)
                    {
                        continue;
                    }
                    if (pose.rotation * tread.axle).y.abs() > 0.4 {
                        continue;
                    }
                    let origin = pose.transform_point(tread.center);
                    if let Some((other, hit)) = pipeline.cast_ray_and_get_normal(
                        origin,
                        Vec3::NEG_Y,
                        tread.ground_reach,
                        true,
                    ) && hit.normal.y > 0.4
                    {
                        patches.retain(|(e, i, _), _| *e != *entity || *i != tread.component);
                        patches.insert(
                            (*entity, tread.component, other),
                            Patch {
                                point: hit.point,
                                normal: hit.normal,
                                samples: 1.,
                            },
                        );
                    }
                }
            }
        },
    );
    if patches.is_empty() {
        return;
    }
    let count = patches.len() as f32;

    let driven = garage.rig.as_ref().map_or(1, |r| {
        r.joints
            .iter()
            .filter(|j| {
                j.kind == ActuatorKind::Spin && !garage.damage.bodies[j.child].wheels.is_empty()
            })
            .count()
    }) as f32;
    let connected = garage.damage.connected_properties();
    let mut sums = BTreeMap::<Entity, ExternalImpulse>::new();
    let mut grounded = BTreeSet::new();
    let mut supports = BTreeMap::<Entity, u32>::new();
    for (entity, _, _) in patches.keys() {
        *supports.entry(*entity).or_default() += 1;
    }
    for ((entity, component, other), patch) in patches {
        let Ok((pose, Some(velocity), Some(properties))) = bodies.get(entity) else {
            continue;
        };
        let tread = tyres
            .get(entity)
            .unwrap()
            .0
            .iter()
            .find(|t| t.component == component)
            .unwrap();
        let body = garage.bodies.iter().position(|e| *e == entity).unwrap();
        let drive = garage.rig.as_ref().and_then(|r| {
            let spec = r
                .joints
                .iter()
                .find(|j| j.child == body && j.kind == ActuatorKind::Spin)?;
            let data = joints.get(entity).ok()?.data.as_ref();
            let motor = data.motor(JointAxis::AngX)?;
            let parent = bodies.get(garage.bodies[spec.parent]).ok()?.0;
            let radius = garage.damage.bodies[body]
                .wheels
                .iter()
                .find(|(i, _)| *i == component)?
                .1;
            Some((data, motor, parent, radius))
        });
        let mut point = patch.point / patch.samples;
        let up = patch.normal.normalize_or_zero();
        let load = configuration.gravity.length() * crate::STEP as f32
            / (connected.effective_inv_mass.x * count).max(0.00001);
        let axle = drive.map_or_else(
            || (pose.rotation * tread.axle).normalize_or_zero(),
            |(data, _, parent, _)| parent.rotation * data.local_basis1() * Vec3::X,
        );
        let forward = axle.cross(up).normalize_or_zero();
        let lateral = up.cross(forward).normalize_or_zero();
        if forward == Vec3::ZERO {
            continue;
        }
        let other_motion = bodies.get(other).ok().and_then(|(_, v, p)| Some((v?, p?)));
        let relative = velocity.linear_velocity_at_point(point, properties.center_of_mass)
            - other_motion.map_or(Vec3::ZERO, |(v, p)| {
                v.linear_velocity_at_point(point, p.center_of_mass)
            });
        let mut recipient = entity;
        let mut receiver = properties;
        let mut impulse = Vec3::ZERO;
        if let Some((data, motor, parent, radius)) = drive {
            // Native 4dbf90/4856f0 applies wheel forces to the host at the
            // wheel anchor. Driving through the rotating low-poly proxy's
            // instantaneous friction and inertia makes tank steering stall.
            // Use the nonspinning joint frame. Authored tyre/axle axes can
            // differ slightly; rotating that error with the tyre would make
            // the traction direction precess once per wheel revolution.
            point = parent.transform_point(data.local_anchor1());
            let root = garage.bodies[0];
            let Ok((_, Some(root_velocity), Some(root_properties))) = bodies.get(root) else {
                continue;
            };
            recipient = root;
            receiver = root_properties;
            let axis = parent.rotation * data.local_basis1() * Vec3::X;
            let target = if motor.max_force > 0. {
                motor.target_vel * axis.dot(axle) * radius
            } else {
                0.
            };
            let relative = root_velocity
                .linear_velocity_at_point(point, root_properties.center_of_mass)
                - other_motion.map_or(Vec3::ZERO, |(v, p)| {
                    v.linear_velocity_at_point(point, p.center_of_mass)
                });
            if tread.grip > 0. && load > 0. {
                // Drive RPM requests traction, not visible free spin. Couple
                // actual axle rotation to the ground with an external tyre
                // torque. Using an axle motor for this would incorrectly send
                // the braking reaction into the chassis instead of the ground.
                let centre_velocity = bodies
                    .get(joints.get(entity).unwrap().parent)
                    .ok()
                    .and_then(|(_, v, p)| Some((v?, p?)))
                    .map_or_else(
                        || {
                            root_velocity
                                .linear_velocity_at_point(point, root_properties.center_of_mass)
                        },
                        |(v, p)| v.linear_velocity_at_point(point, p.center_of_mass),
                    );
                let ground_velocity = other_motion.map_or(Vec3::ZERO, |(v, p)| {
                    v.linear_velocity_at_point(patch.point / patch.samples, p.center_of_mass)
                });
                let speed = (centre_velocity - ground_velocity).dot(forward)
                    / (radius * axis.cross(up).length().max(0.001));
                let inv = axis.dot(properties.effective_world_inv_inertia * axis)
                    + other_motion
                        .map_or(0., |(_, p)| axis.dot(p.effective_world_inv_inertia * axis));
                if inv > 0. {
                    let limit = tread.grip * load * radius;
                    let torque = axis
                        * ((speed - velocity.angular.dot(axis)) / inv / supports[&entity] as f32)
                            .clamp(-limit, limit);
                    sums.entry(entity).or_default().torque_impulse += torque;
                    if other_motion.is_some() {
                        sums.entry(other).or_default().torque_impulse -= torque;
                    }
                    grounded.insert(entity);
                }
            }
            let inv = inverse(&connected, point, forward) * count
                + other_motion.map_or(0., |(_, p)| inverse(p, point, forward));
            let limit = (tread.grip * load).min(if motor.max_force > 0. {
                motor.max_force / radius * crate::STEP as f32 * driven / count
            } else {
                tread.grip * load
            });
            // Solver adaptation: RPM sets the surface-speed goal; force and
            // resource budgets still cap traction. Zero input keeps passive
            // rolling drag instead of disabling ground resistance entirely.
            let response = if motor.max_force > 0. { 8. } else { 4. };
            if inv > 0. {
                impulse += forward
                    * ((target - relative.dot(forward))
                        * (1. - (-response * crate::STEP as f32).exp())
                        / inv)
                        .clamp(-limit, limit);
            }
            let inv = inverse(&connected, point, lateral) * count
                + other_motion.map_or(0., |(_, p)| inverse(p, point, lateral));
            if inv > 0. {
                impulse += lateral
                    * (-relative.dot(lateral)
                        * (1. - (-30. * tread.resistance * crate::STEP as f32).exp())
                        / inv)
                        .clamp(-tread.grip * load, tread.grip * load);
            }
        } else {
            for (direction, grip, spin) in [
                (forward, tread.grip, true),
                (lateral, tread.resistance, false),
            ] {
                let lever = (point - properties.center_of_mass)
                    .cross(direction)
                    .dot(axle);
                let spin_inverse = if spin {
                    axle.dot(properties.effective_world_inv_inertia * axle) * lever * lever
                } else {
                    0.
                };
                let inv = inverse(&connected, point, direction) * count
                    + spin_inverse
                    + other_motion.map_or(0., |(_, p)| inverse(p, point, direction));
                if inv > 0. {
                    let limit = grip * load;
                    impulse += direction * (-relative.dot(direction) / inv).clamp(-limit, limit);
                }
            }
        }
        impulse = impulse.clamp_length_max(tread.grip * load);
        *sums.entry(recipient).or_default() +=
            ExternalImpulse::at_point(impulse, point, receiver.center_of_mass);
        if let Some((_, p)) = other_motion {
            *sums.entry(other).or_default() +=
                ExternalImpulse::at_point(-impulse, point, p.center_of_mass);
        }
    }
    for entity in grounded {
        if let Ok(mut joint) = joints.get_mut(entity) {
            // Traction already used the original powered torque budget. Wheel
            // spin now comes from passive ground coupling; airborne wheels
            // still use the regular drive motor from the next drive step.
            joint.data.as_mut().set_motor_max_force(JointAxis::AngX, 0.);
        }
    }
    // Native 485ca5: grounded yaw drag is 0.05 * mass, scaled by the
    // supported-wheel fraction. An implicit impulse cannot reverse the spin.
    if let Ok((_, Some(velocity), _)) = bodies.get(garage.bodies[0]) {
        let mass = 1. / connected.effective_inv_mass.x.max(0.00001);
        let total = tyres.iter().map(|t| t.0.len()).sum::<usize>().max(1) as f32;
        let damping = 0.05 * mass * crate::STEP as f32 * (count / total).min(1.);
        let inv = Vec3::Y.dot(connected.effective_world_inv_inertia * Vec3::Y);
        let torque = -velocity.angular.y * damping / (1. + damping * inv);
        sums.entry(garage.bodies[0]).or_default().torque_impulse += Vec3::Y * torque;
        // Solver adaptation: dissipate pitch/roll motor reaction while on the
        // ground. This is damping, not an orientation constraint. The original
        // also has grounded angular resistance (485e52), with a different law.
        for axis in [Vec3::X, Vec3::Z] {
            let damping = mass * crate::STEP as f32;
            let inv = axis.dot(connected.effective_world_inv_inertia * axis);
            sums.entry(garage.bodies[0]).or_default().torque_impulse +=
                axis * (-velocity.angular.dot(axis) * damping / (1. + damping * inv));
        }
    }
    for (entity, impulse) in sums {
        if let Ok(mut existing) = impulses.get_mut(entity) {
            *existing += impulse;
        } else {
            commands.entity(entity).insert(impulse);
        }
    }
}
