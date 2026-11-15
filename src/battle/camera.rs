//! Exhibition input and arena occlusion around the shared follow rig.
use super::*;

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(super) fn update(
    mut s: ResMut<Session>,
    nav: Res<Navigation>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<bevy::input::mouse::AccumulatedMouseMotion>,
    buttons: Res<ButtonInput<MouseButton>>,
    mut wheel: MessageReader<bevy::input::mouse::MouseWheel>,
    time: Res<Time>,
    mut cameras: Query<(&Camera, &mut Transform, &mut Projection), With<MatchCamera>>,
    poses: Query<(&Transform, &Velocity), (With<RobotBody>, Without<MatchCamera>)>,
    context: ReadRapierContext<With<MatchContext>>,
) {
    if let Phase::Introduction(age) = s.phase {
        wheel.clear();
        if let Some((position, rotation, fov)) = s.presentation.camera(age)
            && let Ok((camera, mut pose, mut projection)) = cameras.get_mut(s.camera)
            && camera.is_active
        {
            *pose = Transform::from_translation(position).with_rotation(rotation);
            if let Projection::Perspective(p) = &mut *projection {
                p.fov = fov;
            }
        }
        return;
    }
    if nav.page == Page::Match {
        if keys.just_pressed(KeyCode::F3) {
            s.follow.cycle();
            s.revision += 1;
        }
        if buttons.pressed(MouseButton::Right) {
            s.follow.orbit(mouse.delta);
        }
        for e in wheel.read() {
            s.follow.zoom(e.y);
        }
    } else {
        wheel.clear();
    }
    if let Some(root) = s
        .robots
        .iter()
        .find(|r| r.id as usize == s.local_slot + 1)
        .and_then(|r| r.bodies.first())
        && let Ok((p, v)) = poses.get(*root)
        && let Ok((camera, mut camera_pose, mut projection)) = cameras.get_mut(s.camera)
        && camera.is_active
    {
        if let Projection::Perspective(p) = &mut *projection {
            p.fov = std::f32::consts::FRAC_PI_4;
        }
        let dt = if nav.page == Page::Match {
            time.delta_secs()
        } else {
            0.
        };
        let frame = s.follow.sample(p, v.linear, dt);
        let focus = frame.focus;
        let context = context.single().ok();
        let clip = |eye: Vec3| {
            let eye = if let Some((min, max)) = s.arena_state.bounds {
                Vec3::new(
                    eye.x.clamp(min.x + 0.25, max.x - 0.25),
                    eye.y,
                    eye.z.clamp(min.z + 0.25, max.z - 0.25),
                )
            } else {
                eye
            };
            let offset = eye - focus;
            let direction = offset.normalize_or_zero();
            let mut distance = offset.length();
            if let Some(context) = &context
                && let Some((_, hit)) = context.cast_ray(
                    focus,
                    direction,
                    distance,
                    true,
                    QueryFilter::default()
                        .exclude_sensors()
                        .groups(CollisionGroups::new(Group::ALL, Group::GROUP_1)),
                )
            {
                distance = (hit - 0.25).max(0.2);
            }
            if let Some(shell) = &s.arena_state.camera_shell
                && let Some(hit) = shell.cast_ray(
                    Vec3::ZERO,
                    Quat::IDENTITY,
                    focus,
                    direction,
                    distance,
                    false,
                )
            {
                distance = (hit - 0.25).max(0.2);
            }
            focus + direction * distance
        };
        // Raise the boom at walls rather than forcing it into a close-up inside
        // the robot. Try a bounded number of BVH rays; never add world colliders.
        let mut best = focus + Vec3::Y * 0.2;
        let mut best_distance = 0.;
        for attempt in 0..5 {
            let pitch = (frame.pitch + attempt as f32 * 0.3).min(1.5);
            let eye = clip(frame.eye(pitch));
            let distance = eye.distance(focus);
            if distance > best_distance {
                best = eye;
                best_distance = distance;
            }
            if distance >= frame.distance * 0.9 {
                break;
            }
        }
        *camera_pose = Transform::from_translation(best).looking_at(focus, Vec3::Y);
    }
}
