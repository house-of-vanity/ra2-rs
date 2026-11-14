use super::*;
use bevy::{
    input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll},
    window::PrimaryWindow,
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Tool {
    Hammer,
    Saw,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn input(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut camera: Single<(&Camera, &GlobalTransform, &mut Transform), With<LabCamera>>,
    mut orbit: ResMut<Orbit>,
    mut lab: ResMut<Lab>,
) {
    if lab.physics.unstable {
        lab.target = None;
        return;
    }
    if mouse.pressed(MouseButton::Right) {
        if keys.pressed(KeyCode::ShiftLeft) {
            let right = camera.1.right().as_vec3();
            let distance = orbit.distance;
            orbit.focus += (-right * motion.delta.x + Vec3::Y * motion.delta.y) * distance * 0.0015;
        } else {
            orbit.yaw -= motion.delta.x * 0.006;
            orbit.pitch = (orbit.pitch + motion.delta.y * 0.005).clamp(0.08, 1.5);
        }
    }
    orbit.distance = (orbit.distance * (-scroll.delta.y * 0.1).exp()).clamp(1.3, 11.0);
    let (sy, cy) = orbit.yaw.sin_cos();
    let (sp, cp) = orbit.pitch.sin_cos();
    camera.2.translation = orbit.focus + Vec3::new(sy * cp, sp, cy * cp) * orbit.distance;
    camera.2.look_at(orbit.focus, Vec3::Y);
    if keys.pressed(KeyCode::KeyQ) {
        lab.cut_angle += time.delta_secs() * 1.5;
    }
    if keys.pressed(KeyCode::KeyE) {
        lab.cut_angle -= time.delta_secs() * 1.5;
    }
    lab.cooldown = (lab.cooldown - time.delta_secs()).max(0.0);
    lab.saw_active = false;
    let Some(position) = window
        .cursor_position()
        .filter(|p| !ui::over_ui(*p, window.size()))
    else {
        lab.target = None;
        return;
    };
    let Ok(ray) = camera.0.viewport_to_world(camera.1, position) else {
        return;
    };
    lab.target = lab.physics.hit(ray.origin, *ray.direction);
    let Some((collider, point, normal)) = lab.target else {
        return;
    };
    if lab.paused || !mouse.pressed(MouseButton::Left) || mouse.pressed(MouseButton::Right) {
        return;
    }
    let power = lab.power;
    match lab.tool {
        Tool::Hammer => {
            if lab.cooldown == 0.0 && lab.physics.hammer.is_none() {
                lab.physics.strike(point, normal, power);
                lab.cooldown = 0.7;
                lab.strikes += 1;
            }
        }
        Tool::Saw => {
            lab.saw_active = true;
            if lab.cooldown > 0.0 {
                return;
            }
            let tangent = Quat::from_axis_angle(normal, lab.cut_angle)
                * (Quat::from_rotation_arc(Vec3::Y, normal) * Vec3::X);
            let changed = lab.physics.cut(point, normal, tangent, power);
            if !changed {
                // Cutting a component's mount releases the native rigid component as debris.
                let body = lab.physics.world.colliders[collider].parent();
                if let Some(mount) = lab
                    .physics
                    .mounts
                    .iter()
                    .position(|m| !m.broken && Some(m.body) == body)
                {
                    let handle = lab.physics.mounts[mount].joint;
                    lab.physics.world.remove_impulse_joint(handle);
                    lab.physics.mounts[mount].broken = true;
                }
            }
            lab.physics
                .impacts
                .push(arena_runtime::destruction::world::Impact {
                    point,
                    normal,
                    power: 0.13,
                });
            lab.cooldown = 0.09;
        }
    }
}
