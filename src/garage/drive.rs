//! Fixed-step actuation, electricity and pneumatic budgets. No edits to the workshop robot.
use super::*;
#[derive(Component)]
pub(super) struct Motor(pub usize);
pub(super) fn motors(
    mut garage: ResMut<Garage>,
    editor: Res<Editor>,
    nav: Res<Navigation>,
    mut joints: Query<(&Motor, &mut ImpulseJoint)>,
    poses: Query<&Transform, With<GarageBody>>,
) {
    if editor.mode != Mode::Test || nav.page != Page::Workshop {
        return;
    }
    let Garage {
        tuning,
        bodies,
        rig,
        states,
        commands,
        damage,
        document,
        ..
    } = &mut *garage;
    let Some(rig) = rig else {
        return;
    };
    let dt = crate::STEP as f32;
    let mut electric_capacity = 0.;
    let mut electric_rate = 0.;
    let mut air_capacity = 0.;
    let mut air_rate = 0.;
    for s in damage
        .supplies
        .iter()
        .filter(|s| damage.health.operational(s.component))
    {
        electric_capacity += s.electricity;
        electric_rate += s.electric_rate;
        air_capacity += s.air;
        air_rate += s.air_rate;
    }
    rig.electrical.capacity = electric_capacity;
    rig.electrical.max_rate = electric_rate;
    rig.electrical.remaining = rig.electrical.remaining.min(electric_capacity);
    rig.pneumatic.capacity = air_capacity;
    rig.pneumatic.max_rate = air_rate;
    rig.pneumatic.remaining = rig.pneumatic.remaining.min(air_capacity);
    let mut measured = vec![None; rig.joints.len()];
    let mut axes = rig.joints.iter().map(|j| j.axis).collect::<Vec<_>>();
    for (Motor(i), joint) in &joints {
        let spec = &rig.joints[*i];
        if let Ok([parent, child]) = poses.get_many([bodies[spec.parent], bodies[spec.child]]) {
            axes[*i] = parent.rotation * joint.data.as_ref().local_basis1() * Vec3::X;
            measured[*i] = Some(joint_position(
                parent,
                child,
                joint.data.as_ref(),
                matches!(spec.kind, ActuatorKind::Piston | ActuatorKind::BurstPiston),
            ));
        }
    }
    let board_alive = document.as_ref().is_some_and(|robot| {
        robot
            .components
            .iter()
            .enumerate()
            .any(|(i, c)| c.class == "ControlBoard" && damage.health.operational(i))
    });
    let has_control = rig.control_board && board_alive && damage.health.operational(0);
    let targets = rig
        .joints
        .iter()
        .enumerate()
        .map(|(i, j)| {
            if !has_control
                || !damage.health.operational(j.component)
                || (j.kind == ActuatorKind::Burst
                    && damage.health.parts[j.component].fraction() <= 0.5)
            {
                return (None, 0., 0.);
            }
            states[i].step(
                j,
                commands.get(j.component).copied().unwrap_or_default(),
                dt,
                measured[i],
            )
        })
        .collect::<Vec<_>>();
    let electric: f32 = rig
        .joints
        .iter()
        .zip(&targets)
        .map(|(j, t)| j.electricity * t.2)
        .sum();
    let air: f32 = rig
        .joints
        .iter()
        .zip(&targets)
        .map(|(j, t)| j.air * t.2)
        .sum();
    let e = if electric > 0. && rig.electrical.max_rate <= 0. {
        0.
    } else if electric > 0.0 {
        rig.electrical
            .consume(electric.min(rig.electrical.max_rate), dt)
    } else {
        f32::from(rig.electrical.remaining > 0.0 && rig.electrical.max_rate > 0.0)
    };
    let a = if air > 0. && rig.pneumatic.max_rate <= 0. {
        0.
    } else if air > 0.0 {
        rig.pneumatic.consume(air.min(rig.pneumatic.max_rate), dt)
    } else {
        f32::from(rig.pneumatic.remaining > 0.0 && rig.pneumatic.max_rate > 0.0)
    };
    for command in commands.iter_mut() {
        command.fire = false;
    }
    let wheel_radii: Vec<_> = rig
        .joints
        .iter()
        .map(|j| {
            if j.kind == ActuatorKind::Spin && damage.health.operational(j.component) {
                damage.bodies[j.child]
                    .wheels
                    .iter()
                    .filter(|(i, _)| damage.health.operational(*i))
                    .map(|(_, r)| *r)
                    .reduce(f32::max)
            } else {
                None
            }
        })
        .collect();
    let wheel_drives = wheel_radii.iter().flatten().count().max(1) as f32;
    // Opposed wheel commands used for tank steering cancel the pitch
    // reaction. Preserve their differential torque instead of weakening turns.
    let mut reaction = Vec3::ZERO;
    let mut demand = 0_f32;
    for (i, j) in rig
        .joints
        .iter()
        .enumerate()
        .filter(|(i, _)| wheel_radii[*i].is_some())
    {
        let drive = targets[i].1 / j.speed.abs().max(0.001);
        reaction += axes[i] * drive;
        demand += drive.abs();
    }
    let coherence = if demand > 0. {
        reaction.length() / demand
    } else {
        0.
    };
    let connected_mass: f32 = damage
        .bodies
        .iter()
        .flat_map(|b| &b.masses)
        .map(|(_, m)| m)
        .sum();
    for (Motor(i), mut joint) in &mut joints {
        let spec = &rig.joints[*i];
        let (position, velocity, activity) = targets[*i];
        let linear = matches!(spec.kind, ActuatorKind::Piston | ActuatorKind::BurstPiston);
        let axis = if linear {
            JointAxis::LinX
        } else {
            JointAxis::AngX
        };
        let health = damage.health.parts.get(spec.component);
        let fraction = health.map_or(0., |p| p.fraction());
        let enabled = damage.health.operational(0)
            && health.is_some_and(|p| p.alive())
            && !(spec.kind == ActuatorKind::Burst && fraction <= 0.5);
        let supply = if !has_control || !enabled {
            0.0
        } else {
            (if spec.electricity > 0.0 { e } else { 1.0 }) * (if spec.air > 0.0 { a } else { 1.0 })
        };
        let data = joint.data.as_mut();
        if spec.kind == ActuatorKind::Spin {
            data.set_motor_model(
                axis,
                if wheel_radii[*i].is_some() {
                    MotorModel::ForceBased
                } else {
                    MotorModel::AccelerationBased
                },
            );
        }
        let burst = matches!(spec.kind, ActuatorKind::Burst | ActuatorKind::BurstPiston);
        if burst {
            let target = position.unwrap_or(spec.start);
            let error = target - measured[*i].unwrap_or(spec.start);
            let striking = states[*i].burst_time > 0. && !states[*i].returning;
            let speed = velocity.abs()
                * if striking {
                    tuning.strike_speed_scale
                } else {
                    1.
                };
            // Native velocity motor with a torque cap, fast stroke and slower return.
            data.set_motor_velocity(
                axis,
                (error * if striking { 60. } else { 20. }).clamp(-speed, speed),
                tuning.motor_damping * if striking { 1.5 } else { 1. },
            );
            if let Some(limits) = spec.limits {
                data.set_limits(
                    axis,
                    if states[*i].burst_time <= 0. && error.abs() < 0.05 {
                        [spec.start, spec.start]
                    } else {
                        limits
                    },
                );
            }
        } else if let Some(position) = position {
            data.set_motor_position(axis, position, tuning.servo_stiffness, tuning.servo_damping);
        } else if wheel_radii[*i].is_some() {
            // Wheel control uses a torque response, independent of the tyre's
            // inertia. A mass-normalized gain makes small wheels stall in turns.
            data.set_motor_velocity(axis, velocity, tuning.wheel_motor_damping);
        } else {
            data.set_motor_velocity(axis, velocity, tuning.motor_damping);
        }
        let native_torque = if burst && !states[*i].returning && activity > 0. {
            spec.burst_force * tuning.strike_force_scale
        } else {
            spec.force
        };
        let torque = (native_torque * tuning.motor_force_scale).min(wheel_radii[*i].map_or(
            f32::INFINITY,
            |radius| {
                connected_mass * tuning.wheel_drive_acceleration * radius
                    / (wheel_drives * coherence.max(0.001))
            },
        ));
        data.set_motor_max_force(
            axis,
            if spec.kind == ActuatorKind::Passive {
                0.
            } else {
                torque
                    * supply
                    * (0.5 + 0.5 * fraction)
                    * if position.is_some() { 1. } else { activity }
            },
        );
    }
}

/// Feedback uses the same local frames as the solver, so a nonidentity rest
/// orientation measures zero. Comparing raw body quaternions loses that offset.
pub(super) fn joint_position(
    parent: &Transform,
    child: &Transform,
    joint: &GenericJoint,
    linear: bool,
) -> f32 {
    let frame = parent.rotation * joint.local_basis1();
    if linear {
        return (child.transform_point(joint.local_anchor2())
            - parent.transform_point(joint.local_anchor1()))
        .dot(frame * Vec3::X);
    }
    let relative = (frame.inverse() * child.rotation * joint.local_basis2()).normalize();
    let angle = 2.0 * relative.x.atan2(relative.w);
    (angle + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn feedback_is_relative_to_the_joint_rest_frames() {
        let parent = Transform::from_rotation(Quat::from_rotation_y(0.7));
        let rest = Quat::from_rotation_z(-1.3) * Quat::from_rotation_x(2.1);
        let world = Quat::from_rotation_y(-0.4);
        let joint = GenericJointBuilder::new(JointAxesMask::LOCKED_REVOLUTE_AXES)
            .local_basis1(parent.rotation.inverse() * world)
            .local_basis2(rest.inverse() * world)
            .build();
        for angle in [0.0, -2.4, 2.4] {
            let child =
                Transform::from_rotation(Quat::from_axis_angle(world * Vec3::X, angle) * rest);
            assert!((joint_position(&parent, &child, &joint, false) - angle).abs() < 0.00001);
        }
    }
}
