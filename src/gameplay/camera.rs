//! Reusable player framing. Scene adapters resolve walls after sampling the rig.
use bevy::prelude::*;

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
    #[default]
    Automatic,
    Player,
}
impl Mode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Automatic => "Automatic Camera",
            Self::Player => "Player Camera",
        }
    }
}

pub(crate) struct Frame {
    pub focus: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub distance: f32,
}
impl Frame {
    pub fn eye(&self, pitch: f32) -> Vec3 {
        self.focus
            + Vec3::new(
                self.yaw.sin() * pitch.cos(),
                pitch.sin(),
                self.yaw.cos() * pitch.cos(),
            ) * self.distance
    }
}

pub(crate) struct Rig {
    pub mode: Mode,
    yaw: f32,
    pitch: f32,
    distance: f32,
    auto_distance: f32,
    follow: Option<(Vec3, f32)>,
}
impl Default for Rig {
    fn default() -> Self {
        Self {
            mode: Mode::Automatic,
            yaw: 0.,
            pitch: 0.45,
            distance: 6.,
            auto_distance: 10.5,
            follow: None,
        }
    }
}
impl Rig {
    pub fn cycle(&mut self) {
        self.mode = match self.mode {
            Mode::Automatic => Mode::Player,
            Mode::Player => Mode::Automatic,
        };
        self.follow = None;
    }
    pub fn orbit(&mut self, delta: Vec2) {
        if self.mode == Mode::Player {
            self.yaw -= delta.x * 0.008;
            self.pitch = (self.pitch + delta.y * 0.008).clamp(0.1, 1.3);
        }
    }
    pub fn zoom(&mut self, steps: f32) {
        match self.mode {
            Mode::Automatic => {
                self.auto_distance = (self.auto_distance - steps * 0.6).clamp(6., 24.)
            }
            Mode::Player => self.distance = (self.distance - steps * 0.4).clamp(3., 16.),
        }
    }
    pub fn sample(&mut self, robot: &Transform, velocity: Vec3, dt: f32) -> Frame {
        // Project the authored front onto the floor, retaining the last heading
        // when the robot tips onto its nose. Roll never rolls the camera.
        let forward = robot.rotation * Vec3::NEG_Z;
        let heading = if Vec2::new(forward.x, forward.z).length_squared() > 0.01 {
            forward.x.atan2(forward.z) + std::f32::consts::PI
        } else {
            self.follow.map_or(0., |(_, yaw)| yaw)
        };
        let focus = robot.translation + Vec3::Y * 0.45;
        if self.mode == Mode::Player {
            return Frame {
                focus,
                yaw: heading + self.yaw,
                pitch: self.pitch,
                distance: self.distance,
            };
        }
        let ahead = (Vec3::new(velocity.x, 0., velocity.z) * 0.16).clamp_length_max(0.8);
        let focus = focus + ahead;
        let (previous, yaw) = self.follow.unwrap_or((focus, heading));
        let (focus, yaw) = if previous.distance_squared(focus) > 20_f32.powi(2) {
            (focus, heading)
        } else {
            let dt = dt.clamp(0., 0.1);
            let turn = (heading - yaw).sin().atan2((heading - yaw).cos());
            (
                previous.lerp(focus, 1. - (-10. * dt).exp()),
                yaw + turn * (1. - (-2.5 * dt).exp()),
            )
        };
        self.follow = Some((focus, yaw));
        Frame {
            focus,
            yaw,
            pitch: 0.62,
            distance: self.auto_distance,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn modes_cycle_with_independent_zoom_and_preserve_the_old_orbit() {
        let mut rig = Rig::default();
        let robot = Transform::default();
        assert_eq!(rig.mode, Mode::Automatic);
        assert_eq!(rig.sample(&robot, Vec3::ZERO, 0.).distance, 10.5);
        rig.orbit(Vec2::splat(10.));
        rig.zoom(1.);
        rig.cycle();
        assert_eq!(rig.mode, Mode::Player);
        assert_eq!(rig.sample(&robot, Vec3::ZERO, 0.).distance, 6.);
        rig.orbit(Vec2::new(25., 10.));
        rig.zoom(1.);
        let manual = rig.sample(&robot, Vec3::ZERO, 0.);
        rig.cycle();
        assert!((rig.sample(&robot, Vec3::ZERO, 0.).distance - 9.9).abs() < 1e-5);
        rig.cycle();
        let restored = rig.sample(&robot, Vec3::ZERO, 0.);
        assert_eq!(
            (manual.yaw, manual.pitch, manual.distance),
            (restored.yaw, restored.pitch, restored.distance)
        );
    }
    #[test]
    fn automatic_follow_is_frame_rate_independent_and_takes_short_turns() {
        fn run(hz: u32) -> Frame {
            let mut rig = Rig::default();
            let mut pose = Transform::from_rotation(Quat::from_rotation_y(3.1));
            rig.sample(&pose, Vec3::ZERO, 0.);
            pose.translation.x = 4.;
            pose.rotation = Quat::from_rotation_y(-3.1);
            let mut frame = rig.sample(&pose, Vec3::ZERO, 1. / hz as f32);
            for _ in 1..hz {
                frame = rig.sample(&pose, Vec3::ZERO, 1. / hz as f32);
            }
            frame
        }
        let a = run(30);
        let b = run(144);
        assert!(a.focus.abs_diff_eq(b.focus, 1e-4));
        assert!((a.yaw - b.yaw).abs() < 1e-4);
        assert!((a.yaw - 3.1).abs() < 0.1);
    }
}
