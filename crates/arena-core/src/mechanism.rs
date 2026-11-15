//! Live robot state. Editing documents and saved BOT data never carry simulation state.
use crate::{JointKind, Mat4, Quat, Vec3, robot::RobotDefinition};

#[derive(Clone, Debug)]
pub struct RigBody {
    pub name: String,
    pub frame: Mat4,
    pub mass: f32,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ActuatorKind {
    Spin,
    Servo,
    Burst,
    Piston,
    BurstPiston,
    Steering,
    Passive,
}
#[derive(Clone, Debug)]
pub struct RigJoint {
    pub component: usize,
    pub parent: usize,
    pub child: usize,
    pub anchor: Vec3,
    pub axis: Vec3,
    /// Authored orientation of the output body in assembly coordinates.
    pub child_rotation: Quat,
    pub kind: ActuatorKind,
    pub motion: JointKind,
    pub limits: Option<[f32; 2]>,
    pub speed: f32,
    pub return_speed: f32,
    pub force: f32,
    pub burst_force: f32,
    pub electricity: f32,
    pub air: f32,
    pub start: f32,
    pub stop: f32,
}
#[derive(Clone, Debug, Default)]
pub struct Rig {
    pub bodies: Vec<RigBody>,
    /// Body carrying the fixed/input frame of each component.
    pub component_bodies: Vec<usize>,
    /// Body owning each attachment point, including moving output sockets.
    pub attachment_bodies: Vec<Vec<usize>>,
    /// For each component, the rigid body owning each model object.
    pub objects: Vec<Vec<usize>>,
    pub joints: Vec<RigJoint>,
    pub electrical: Reservoir,
    pub pneumatic: Reservoir,
    pub control_board: bool,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct Reservoir {
    pub capacity: f32,
    pub remaining: f32,
    pub max_rate: f32,
}
impl Reservoir {
    pub fn fraction(&self) -> f32 {
        if self.capacity > 0.0 {
            (self.remaining / self.capacity).clamp(0.0, 1.0)
        } else {
            0.0
        }
    }
    pub fn consume(&mut self, demand: f32, dt: f32) -> f32 {
        if demand <= 0.0 {
            return 1.0;
        }
        let rate = demand.min(self.max_rate).min(self.remaining / dt.max(1e-6));
        self.remaining = (self.remaining - rate * dt).max(0.0);
        (rate / demand).clamp(0.0, 1.0)
    }
    pub fn refill(&mut self) {
        self.remaining = self.capacity;
    }
}
#[derive(Clone, Copy, Debug, Default)]
pub struct Command {
    pub drive: f32,
    pub steer: f32,
    pub fire: bool,
}
#[derive(Default, Clone)]
pub struct Controls {
    previous: Vec<Vec<f32>>,
    switches: Vec<Vec<f32>>,
    toggles: Vec<bool>,
    binding_previous: Vec<bool>,
}
/// Complete state of native switches and per-binding edges for authoritative rollback.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ControlsSnapshot {
    pub previous: Vec<Vec<f32>>,
    pub switches: Vec<Vec<f32>>,
    pub toggles: Vec<bool>,
    pub binding_previous: Vec<bool>,
}
impl ControlsSnapshot {
    pub fn valid(&self) -> bool {
        self.previous.len() <= 128
            && self.previous.len() == self.switches.len()
            && self
                .previous
                .iter()
                .zip(&self.switches)
                .all(|(p, s)| p.len() == s.len())
            && self.previous.iter().map(Vec::len).sum::<usize>() <= 128
            && self.switches.iter().map(Vec::len).sum::<usize>() <= 128
            && self.toggles.len() <= 4096
            && self.binding_previous.len() == self.toggles.len()
            && self
                .previous
                .iter()
                .chain(&self.switches)
                .flatten()
                .all(|v| v.is_finite() && (0. ..=1.).contains(v))
    }
}
impl Controls {
    fn prepare(&mut self, robot: &RobotDefinition) {
        if self.previous.len() != robot.controllers.len()
            || self
                .previous
                .iter()
                .zip(&robot.controllers)
                .any(|(row, c)| row.len() != c.keys.len())
        {
            self.previous = robot
                .controllers
                .iter()
                .map(|c| vec![0.; c.keys.len()])
                .collect();
            self.switches = self.previous.clone();
        }
        self.toggles.resize(robot.bindings.len(), false);
        self.binding_previous.resize(robot.bindings.len(), false);
    }
    pub fn snapshot(&self) -> ControlsSnapshot {
        ControlsSnapshot {
            previous: self.previous.clone(),
            switches: self.switches.clone(),
            toggles: self.toggles.clone(),
            binding_previous: self.binding_previous.clone(),
        }
    }
    pub fn restore(&mut self, s: &ControlsSnapshot) -> Result<(), String> {
        if !s.valid() {
            return Err("Invalid native controls checkpoint".into());
        }
        self.previous = s.previous.clone();
        self.switches = s.switches.clone();
        self.toggles = s.toggles.clone();
        self.binding_previous = s.binding_previous.clone();
        Ok(())
    }
    /// Preserve lost short presses without evaluating a controller twice. A replay
    /// restores both this state and the acknowledged edge counters before stepping.
    pub fn evaluate_wire(
        &mut self,
        robot: &RobotDefinition,
        levels: &[u8],
        edges: &[u32],
        seen: &mut Vec<u32>,
    ) -> Vec<Command> {
        self.prepare(robot);
        let mut samples = Vec::with_capacity(robot.controllers.len());
        let mut flat = 0;
        seen.resize(robot.controllers.iter().map(|c| c.keys.len()).sum(), 0);
        for (ci, c) in robot.controllers.iter().enumerate() {
            let mut row = Vec::with_capacity(c.keys.len());
            for key in 0..c.keys.len() {
                let presses = edges
                    .get(flat)
                    .copied()
                    .unwrap_or(0)
                    .saturating_sub(seen[flat]);
                let mut level = levels.get(flat).copied().unwrap_or(0) as f32 / 255.;
                if presses > 0 {
                    // evaluate() handles the final edge. Preserve parity of any
                    // earlier presses whose individual commands were lost.
                    if c.kind == 2 && presses % 2 == 0 {
                        self.switches[ci][key] = 1. - self.switches[ci][key];
                    }
                    if let Some(previous) = self.previous.get_mut(ci).and_then(|v| v.get_mut(key)) {
                        *previous = 0.;
                    }
                    for (bi, b) in robot.bindings.iter().enumerate() {
                        if b.controller == c.position
                            && b.key == key
                            && let Some(previous) = self.binding_previous.get_mut(bi)
                        {
                            *previous = false;
                            if c.kind != 2 && b.action.starts_with("On/Off") && presses % 2 == 0 {
                                self.toggles[bi] = !self.toggles[bi];
                            }
                        }
                    }
                    level = level.max(1.);
                }
                seen[flat] = seen[flat].max(edges.get(flat).copied().unwrap_or(0));
                row.push(level);
                flat += 1;
            }
            samples.push(row);
        }
        self.evaluate(robot, &samples)
    }

    /// Samples are indexed by controller, then native input slot. Axis values are 0..1.
    pub fn evaluate(&mut self, robot: &RobotDefinition, samples: &[Vec<f32>]) -> Vec<Command> {
        self.prepare(robot);
        let mut result = vec![Command::default(); robot.components.len()];
        let mut values = self.previous.clone();
        for (i, c) in robot.controllers.iter().enumerate() {
            for (k, output) in values[i].iter_mut().enumerate() {
                let value = samples
                    .get(i)
                    .and_then(|s| s.get(k))
                    .copied()
                    .unwrap_or(0.0)
                    .clamp(0.0, 1.0);
                if c.kind == 2 && value > 0.5 && self.previous[i][k] <= 0.5 {
                    self.switches[i][k] = 1.0 - self.switches[i][k];
                }
                *output = if c.kind == 2 {
                    self.switches[i][k]
                } else {
                    value
                };
                self.previous[i][k] = value;
            }
        }
        for (i, b) in robot.bindings.iter().enumerate() {
            let Some(c) = robot
                .controllers
                .iter()
                .position(|c| c.position == b.controller)
            else {
                continue;
            };
            let value = values
                .get(c)
                .and_then(|v| v.get(b.key))
                .copied()
                .unwrap_or(0.0);
            let down = value > 0.5;
            let edge = down && !self.binding_previous[i];
            self.binding_previous[i] = down;
            let Some(command) = result.get_mut(b.component) else {
                continue;
            };
            let mut value = value;
            if b.action.starts_with("On/Off") && robot.controllers[c].kind != 2 {
                if edge {
                    self.toggles[i] = !self.toggles[i];
                }
                value = f32::from(self.toggles[i]);
            }
            match b.action.as_str() {
                "Spin Clockwise" | "On/Off Clockwise" | "Forward" | "Extend" => {
                    command.drive += value
                }
                "Spin Counter-clockwise" | "On/Off Counter-clockwise" | "Backward" | "Retract" => {
                    command.drive -= value
                }
                "Left" => command.steer += value,
                "Right" => command.steer -= value,
                "Fire" | "Activate" => command.fire |= edge,
                _ => (),
            }
        }
        for c in &mut result {
            c.drive = c.drive.clamp(-1.0, 1.0);
            c.steer = c.steer.clamp(-1.0, 1.0);
        }
        result
    }
}
#[derive(Default, Clone, Debug, PartialEq)]
pub struct ActuatorState {
    pub position: f32,
    pub burst_time: f32,
    pub initialized: bool,
    pub returning: bool,
}
impl ActuatorState {
    /// Target position, target velocity, activity. Burst returns without another key press.
    pub fn step(
        &mut self,
        joint: &RigJoint,
        command: Command,
        dt: f32,
        measured: Option<f32>,
    ) -> (Option<f32>, f32, f32) {
        if !self.initialized {
            self.position = joint.start;
            self.initialized = true;
        }
        match joint.kind {
            ActuatorKind::Servo if joint.limits.is_none() => {
                (None, command.drive * joint.speed, command.drive.abs())
            }
            ActuatorKind::Spin => (
                None,
                command.drive * joint.speed,
                command.drive.abs().max(command.steer.abs()),
            ),
            ActuatorKind::Steering => (Some(command.steer * 0.6), 0.0, command.steer.abs()),
            ActuatorKind::Servo | ActuatorKind::Piston => {
                self.position += command.drive * joint.speed * dt;
                if let Some([min, max]) = joint.limits {
                    self.position = self.position.clamp(min, max);
                }
                (Some(self.position), 0.0, command.drive.abs())
            }
            ActuatorKind::Burst | ActuatorKind::BurstPiston => {
                if command.fire && self.burst_time <= 0.0 {
                    self.burst_time =
                        1.5 + (joint.stop - joint.start).abs() / joint.return_speed.max(0.1);
                    self.returning = false;
                }
                if self.burst_time > 0.0 {
                    let target = if self.returning {
                        joint.start
                    } else {
                        joint.stop
                    };
                    if measured.is_some_and(|p| (p - target).abs() < 0.05) {
                        if self.returning {
                            self.burst_time = 0.0;
                        } else {
                            self.returning = true;
                        }
                    }
                    // Return even if a weapon is pinned against an obstacle.
                    if self.burst_time
                        < 0.5 + (joint.stop - joint.start).abs() / joint.return_speed.max(0.1)
                    {
                        self.returning = true;
                    }
                }
                let active = self.burst_time > 0.0;
                let target = if active && !self.returning {
                    joint.stop
                } else {
                    joint.start
                };
                self.burst_time = (self.burst_time - dt).max(0.0);
                let speed = if active && !self.returning {
                    joint.speed
                } else {
                    joint.return_speed
                };
                let velocity = (target - measured.unwrap_or(self.position)).signum() * speed;
                let activity = if joint.kind == ActuatorKind::BurstPiston {
                    // Air use follows displaced piston volume, not the time spent
                    // waiting at a stop or stalled under load.
                    measured
                        .map_or(0., |p| {
                            (p - self.position).abs() / (dt.max(1e-6) * speed.max(0.1))
                        })
                        .clamp(0., 1.)
                } else {
                    f32::from(active)
                };
                if let Some(p) = measured {
                    self.position = p;
                }
                (Some(target), velocity, activity)
            }
            ActuatorKind::Passive => (None, 0.0, 0.0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reservoir_limits_rate_and_depletion() {
        let mut r = Reservoir {
            capacity: 10.0,
            remaining: 10.0,
            max_rate: 3.0,
        };
        assert_eq!(r.consume(6.0, 2.0), 0.5);
        assert_eq!(r.remaining, 4.0);
        assert!((r.consume(6.0, 2.0) - 1.0 / 3.0).abs() < 1e-6);
        assert_eq!(r.remaining, 0.0);
        assert_eq!(r.consume(6.0, 1.0), 0.0);
        r.refill();
        assert_eq!(r.fraction(), 1.0);
    }
    #[test]
    fn analog_switch_and_action_toggles_use_edges() {
        use crate::robot::{ControlBinding, Controller};
        let mut r = crate::editor::Workshop::new().robot;
        r.controllers.push(Controller {
            name: "Drive".into(),
            position: [0, 0],
            kind: 1,
            keys: vec![vec![], vec![]],
        });
        r.bindings.push(ControlBinding {
            component: 0,
            action: "Spin Clockwise".into(),
            controller: [0, 0],
            key: 0,
        });
        r.bindings.push(ControlBinding {
            component: 0,
            action: "Spin Counter-clockwise".into(),
            controller: [0, 0],
            key: 1,
        });
        let mut c = Controls::default();
        assert_eq!(c.evaluate(&r, &[vec![1.0, 1.0]])[0].drive, 0.0);
        assert_eq!(c.evaluate(&r, &[vec![0.5, 0.0]])[0].drive, 0.5);
        r.controllers[0].kind = 2;
        c = Controls::default();
        assert_eq!(c.evaluate(&r, &[vec![1.0, 0.0]])[0].drive, 1.0);
        assert_eq!(c.evaluate(&r, &[vec![1.0, 0.0]])[0].drive, 1.0);
        c.evaluate(&r, &[vec![0.0, 0.0]]);
        assert_eq!(c.evaluate(&r, &[vec![1.0, 0.0]])[0].drive, 0.0);
    }
}

#[cfg(test)]
mod actuator_tests {
    use super::*;
    fn joint(kind: ActuatorKind) -> RigJoint {
        RigJoint {
            component: 0,
            parent: 0,
            child: 1,
            anchor: Vec3::ZERO,
            axis: Vec3::X,
            child_rotation: Quat::IDENTITY,
            kind,
            motion: JointKind::Slider { limits: [0.0, 1.0] },
            limits: Some([0.0, 1.0]),
            speed: 1.0,
            return_speed: 1.0,
            force: 20.0,
            burst_force: 80.0,
            electricity: 0.0,
            air: 10.0,
            start: 0.0,
            stop: 1.0,
        }
    }
    #[test]
    fn servo_range_and_burst_feedback_return_without_retrigger() {
        let j = joint(ActuatorKind::Piston);
        let mut s = ActuatorState::default();
        for _ in 0..20 {
            s.step(
                &j,
                Command {
                    drive: 1.0,
                    ..Default::default()
                },
                0.1,
                None,
            );
        }
        assert_eq!(s.position, 1.0);
        let j = joint(ActuatorKind::BurstPiston);
        let mut s = ActuatorState::default();
        assert_eq!(
            s.step(
                &j,
                Command {
                    fire: true,
                    ..Default::default()
                },
                0.01,
                Some(0.0)
            )
            .0,
            Some(1.0)
        );
        assert_eq!(s.step(&j, Command::default(), 0.01, Some(1.0)).0, Some(0.0));
        s.step(&j, Command::default(), 0.01, Some(0.0));
        assert_eq!(s.burst_time, 0.0);
    }
}
#[cfg(test)]
mod rollback_controls_tests {
    use super::*;
    use crate::robot::{ControlBinding, Controller};
    #[test]
    fn switch_state_and_lost_short_presses_survive_restore_without_duplicate_fire() {
        let mut r = crate::editor::Workshop::new().robot;
        r.controllers.push(Controller {
            name: "Switch".into(),
            position: [0, 0],
            kind: 2,
            keys: vec![vec![]],
        });
        r.controllers.push(Controller {
            name: "Burst".into(),
            position: [1, 0],
            kind: 1,
            keys: vec![vec![]],
        });
        r.bindings.push(ControlBinding {
            component: 0,
            controller: [0, 0],
            key: 0,
            action: "Spin Clockwise".into(),
        });
        r.bindings.push(ControlBinding {
            component: 0,
            controller: [1, 0],
            key: 0,
            action: "Fire".into(),
        });
        let mut c = Controls::default();
        let mut edges = vec![];
        let command = c.evaluate_wire(&r, &[0, 0], &[1, 1], &mut edges)[0];
        assert_eq!(command.drive, 1.);
        assert!(command.fire);
        let checkpoint = c.snapshot();
        let acknowledged = edges.clone();
        assert!(!c.evaluate_wire(&r, &[0, 0], &[1, 1], &mut edges)[0].fire);
        let expected = c.evaluate_wire(&r, &[0, 0], &[2, 2], &mut edges)[0];
        assert_eq!(expected.drive, 0.);
        assert!(expected.fire);
        c.restore(&checkpoint).unwrap();
        edges = acknowledged;
        c.evaluate_wire(&r, &[0, 0], &[1, 1], &mut edges);
        let replay = c.evaluate_wire(&r, &[0, 0], &[2, 2], &mut edges)[0];
        assert_eq!(replay.drive, expected.drive);
        assert_eq!(replay.fire, expected.fire);
        let current = c.evaluate_wire(&r, &[0, 0], &[4, 4], &mut edges)[0];
        assert_eq!(
            current.drive, replay.drive,
            "two missed switch presses must preserve parity"
        );
        assert!(current.fire);
        assert!(!c.evaluate_wire(&r, &[0, 0], &[4, 4], &mut edges)[0].fire);
    }
}
