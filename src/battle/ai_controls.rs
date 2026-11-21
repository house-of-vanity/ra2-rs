//! Wiring-aware input generation for a robot AI.
//!
//! Construct [`WiringAnalysis`] once from the same document and resolved rig used
//! by the simulation, then keep one [`AiControls`] per robot. Call
//! [`AiControls::samples`] exactly once per control tick and pass its result to
//! `arena_core::mechanism::Controls::evaluate` in the same order. Both controls
//! instances must start at their default state and advance together. Rebuild the
//! analysis and reset both instances when the document or rig changes.
//!
//! This module infers wheel drives from rigid-body ownership and joint axes. Other
//! actuators are treated as weapons; no claim is made that geometry can distinguish
//! a weapon from a self-righting mechanism. Shared wires and unavailable actions
//! are resolved as a best-effort compromise, not bypassed with direct commands.

use arena_core::{
    Vec3,
    mechanism::{ActuatorKind, Command, Controls, Rig},
    robot::RobotDefinition,
};

/// Requested chassis motion and weapon use. Motion is clamped to -1..=1;
/// positive turn means left. `fire` is a sustained request: edge-triggered
/// weapons are retriggered after their estimated stroke and return time.
#[derive(Clone, Copy, Debug, Default)]
pub struct Intent {
    pub forward: f32,
    pub turn: f32,
    pub fire: bool,
}

#[derive(Clone, Copy, Debug)]
struct Wheel {
    component: usize,
    forward_sign: f32,
    side: f32,
}

/// Immutable wiring and geometry derived from one robot and its resolved rig.
#[derive(Clone)]
pub struct WiringAnalysis {
    robot: RobotDefinition,
    wheels: Vec<Wheel>,
    steering: Vec<(usize, f32)>,
    weapons: Vec<usize>,
    slots: Vec<(usize, usize)>,
    reload_ticks: Vec<u32>,
    continuous: Vec<bool>,
    side_scale: f32,
}

impl WiringAnalysis {
    /// Analyze actual component bindings and rigid bodies. Invalid bindings and
    /// degenerate joint axes are ignored, as in the native control evaluator.
    pub fn new(robot: &RobotDefinition, rig: &Rig) -> Self {
        let heading = if robot.heading.is_finite() {
            robot.heading
        } else {
            0.0
        };
        let rotation = arena_core::Quat::from_rotation_y(heading);
        let forward = rotation * Vec3::NEG_Z;
        let left = Vec3::Y.cross(forward);
        let mut wheels = Vec::new();
        let mut steering = Vec::new();
        let mut weapons = Vec::new();
        let mut reload_ticks = vec![DEFAULT_RELOAD_TICKS; robot.components.len()];
        let mut continuous = vec![false; robot.components.len()];
        for joint in &rig.joints {
            if joint.component >= robot.components.len() {
                continue;
            }
            if joint.kind == ActuatorKind::Steering {
                if !steering.iter().any(|&(i, _)| i == joint.component) {
                    let sign = joint.axis.dot(Vec3::Y);
                    if sign.is_finite() && sign.abs() > 0.05 {
                        steering.push((joint.component, sign.signum()));
                    }
                }
                continue;
            }
            let wheel_body = robot
                .components
                .iter()
                .enumerate()
                .filter(|(_, c)| c.class.eq_ignore_ascii_case("Wheel"))
                .flat_map(|(i, _)| {
                    rig.component_bodies
                        .get(i)
                        .copied()
                        .into_iter()
                        .chain(rig.objects.get(i).into_iter().flatten().copied())
                })
                .find(|&body| descendant(rig, joint.child, body));
            if joint.kind == ActuatorKind::Spin
                && let Some(body) = wheel_body
            {
                // Rolling without slip makes the centre velocity opposite to
                // the contact point velocity ω × (-up): v = ω × up.
                // The tyre solver uses the same axle × ground-normal direction.
                let tangent = (rotation * joint.axis).cross(Vec3::Y).dot(forward);
                if tangent.is_finite() && tangent.abs() > 0.05 {
                    let position = rig
                        .bodies
                        .get(body)
                        .map_or(joint.anchor, |b| b.frame.w_axis.truncate());
                    let side = (rotation * position).dot(left);
                    wheels.push(Wheel {
                        component: joint.component,
                        forward_sign: tangent.signum(),
                        side: if side.is_finite() { side } else { 0.0 },
                    });
                    continue;
                }
            }
            if matches!(joint.kind, ActuatorKind::Burst | ActuatorKind::BurstPiston) {
                let duration = 1.5 + (joint.stop - joint.start).abs() / joint.return_speed.max(0.1);
                if duration.is_finite() {
                    reload_ticks[joint.component] = (duration * CONTROL_HZ).ceil().max(1.0) as u32;
                }
            }
            if joint.kind != ActuatorKind::Passive && !weapons.contains(&joint.component) {
                weapons.push(joint.component);
            }
            if joint.kind == ActuatorKind::Spin {
                continuous[joint.component] = true;
            }
        }
        // Cannon and other directly controlled components need not have a joint.
        for binding in &robot.bindings {
            if let Some(part) = robot.components.get(binding.component)
                && matches!(binding.action.as_str(), "Fire" | "Activate")
                && !part.class.eq_ignore_ascii_case("Wheel")
                && !weapons.contains(&binding.component)
            {
                weapons.push(binding.component);
            }
        }
        let mut slots = Vec::new();
        for b in &robot.bindings {
            if b.component >= robot.components.len() {
                continue;
            }
            if let Some(i) = robot
                .controllers
                .iter()
                .position(|c| c.position == b.controller)
                && b.key < robot.controllers[i].keys.len()
                && !slots.contains(&(i, b.key))
            {
                slots.push((i, b.key));
            }
        }
        let side_scale = wheels.iter().map(|w| w.side.abs()).fold(0.0_f32, f32::max);
        Self {
            robot: robot.clone(),
            wheels,
            steering,
            weapons,
            slots,
            reload_ticks,
            continuous,
            side_scale,
        }
    }

    /// Number of wheel joints whose direction could be inferred.
    #[cfg(test)]
    pub fn wheel_count(&self) -> usize {
        self.wheels.len()
    }
    /// Component indices identified as weapon or auxiliary actuators.
    #[cfg(test)]
    pub fn weapons(&self) -> &[usize] {
        &self.weapons
    }
}

fn descendant(rig: &Rig, ancestor: usize, body: usize) -> bool {
    if ancestor >= rig.bodies.len() || body >= rig.bodies.len() {
        return false;
    }
    let mut pending = vec![ancestor];
    let mut visited = vec![false; rig.bodies.len()];
    while let Some(current) = pending.pop() {
        if current == body {
            return true;
        }
        if visited[current] {
            continue;
        }
        visited[current] = true;
        for j in &rig.joints {
            if j.parent == current && j.child < visited.len() && !visited[j.child] {
                pending.push(j.child);
            }
        }
    }
    false
}

const CONTROL_HZ: f32 = 120.0;
// Directly controlled cannons lack a RigJoint from which to estimate a cycle.
const DEFAULT_RELOAD_TICKS: u32 = 120;

/// Stateful input generator. Clone together with the simulation controls for
/// deterministic rollback; do not use a single instance for multiple robots.
#[derive(Clone)]
pub struct AiControls {
    analysis: WiringAnalysis,
    controls: Controls,
    cooldown: Vec<u32>,
    cached: Option<Cached>,
}

#[derive(Clone)]
struct Cached {
    forward: f32,
    turn: f32,
    fire: bool,
    spin_up: bool,
    ready: Vec<bool>,
    samples: Vec<Vec<f32>>,
}

impl AiControls {
    pub fn new(analysis: WiringAnalysis) -> Self {
        let cooldown = vec![0; analysis.robot.components.len()];
        Self {
            analysis,
            controls: Controls::default(),
            cooldown,
            cached: None,
        }
    }

    /// Generate native input levels indexed by controller and key, each in 0..=1.
    /// The internal native evaluator advances once; the caller must evaluate the
    /// returned samples once as well, including when the robot is disabled.
    /// Call at the simulation fixed rate of 120 Hz. Cooldowns are conservative
    /// estimates; physical actuator state and available resources are not observed.
    #[cfg(test)]
    pub fn samples(&mut self, intent: Intent) -> Vec<Vec<f32>> {
        self.samples_with_spin(intent, intent.fire)
    }

    /// The spinner may be started before the opponent enters striking range.
    pub fn samples_with_spin(&mut self, intent: Intent, spin_up: bool) -> Vec<Vec<f32>> {
        let robot = &self.analysis.robot;
        let mut samples: Vec<Vec<f32>> = robot
            .controllers
            .iter()
            .map(|c| vec![0.0; c.keys.len()])
            .collect();
        let forward = finite_unit(intent.forward);
        let turn = finite_unit(intent.turn);
        let side_scale = self.analysis.side_scale;
        for remaining in &mut self.cooldown {
            *remaining = remaining.saturating_sub(1);
        }
        let ready: Vec<bool> = self
            .analysis
            .weapons
            .iter()
            .map(|&i| self.cooldown[i] == 0)
            .collect();
        if let Some(cache) = &self.cached
            && cache.forward == forward
            && cache.turn == turn
            && cache.fire == intent.fire
            && cache.spin_up == spin_up
            && cache.ready == ready
        {
            let samples = cache.samples.clone();
            let commands = self.controls.evaluate(robot, &samples);
            for &i in &self.analysis.weapons {
                if commands[i].fire {
                    self.cooldown[i] = self.analysis.reload_ticks[i];
                }
            }
            return samples;
        }
        let mut desired = vec![Command::default(); robot.components.len()];
        let mut weights = vec![0.0_f32; robot.components.len()];
        for wheel in &self.analysis.wheels {
            let side = if side_scale > 1e-5 {
                wheel.side / side_scale
            } else {
                0.0
            };
            desired[wheel.component].drive = (forward
                - if self.analysis.steering.is_empty() {
                    turn * side
                } else {
                    0.0
                })
            .clamp(-1.0, 1.0)
                * wheel.forward_sign;
            weights[wheel.component] = 1.0;
        }
        for &(i, sign) in &self.analysis.steering {
            desired[i].steer = turn * sign;
            weights[i] = 1.0;
        }
        for &i in &self.analysis.weapons {
            let active = if self.analysis.continuous[i] {
                spin_up
            } else {
                intent.fire
            };
            let positive = robot.bindings.iter().any(|binding| {
                binding.component == i
                    && matches!(
                        binding.action.as_str(),
                        "Spin Clockwise" | "On/Off Clockwise" | "Forward" | "Extend"
                    )
            });
            let negative = robot.bindings.iter().any(|binding| {
                binding.component == i
                    && matches!(
                        binding.action.as_str(),
                        "Spin Counter-clockwise"
                            | "On/Off Counter-clockwise"
                            | "Backward"
                            | "Retract"
                    )
            });
            desired[i].drive = if active && positive {
                1.0
            } else if active && negative {
                -1.0
            } else {
                0.0
            };
            desired[i].fire = active && self.cooldown[i] == 0;
            weights[i] = 0.6;
        }
        // Coordinate descent evaluates the complete native wiring each time. This
        // respects shared bindings, switch latches and action-level toggles.
        for _ in 0..3 {
            let mut changed = false;
            for &(ci, key) in &self.analysis.slots {
                let original = samples[ci][key];
                let mut best = original;
                let mut best_score = score(&self.controls, robot, &samples, &desired, &weights);
                let analog = robot.controllers[ci].kind != 2;
                let levels = [
                    0.0,
                    1.0,
                    forward.abs(),
                    turn.abs(),
                    (forward + turn).abs().min(1.0),
                    (forward - turn).abs().min(1.0),
                ];
                for level in levels {
                    if !analog && level != 0.0 && level != 1.0 {
                        continue;
                    }
                    samples[ci][key] = level;
                    let s = score(&self.controls, robot, &samples, &desired, &weights);
                    if s + 1e-5 < best_score {
                        best_score = s;
                        best = level;
                    }
                }
                samples[ci][key] = best;
                changed |= (best - original).abs() > 1e-5;
            }
            if !changed {
                break;
            }
        }
        let before = self.controls.snapshot();
        let actual = self.controls.evaluate(robot, &samples);
        for &i in &self.analysis.weapons {
            if actual[i].fire {
                self.cooldown[i] = self.analysis.reload_ticks[i];
            }
        }
        // A repeated level is safe to reuse only after all edge/latch state is stable.
        self.cached = if before == self.controls.snapshot() {
            Some(Cached {
                forward,
                turn,
                fire: intent.fire,
                spin_up,
                ready,
                samples: samples.clone(),
            })
        } else {
            None
        };
        samples
    }
}

fn finite_unit(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(-1.0, 1.0)
    } else {
        0.0
    }
}

fn score(
    controls: &Controls,
    robot: &RobotDefinition,
    samples: &[Vec<f32>],
    desired: &[Command],
    weights: &[f32],
) -> f32 {
    let actual = controls.clone().evaluate(robot, samples);
    actual
        .iter()
        .zip(desired)
        .zip(weights)
        .map(|((a, d), w)| {
            let drive = a.drive - d.drive;
            let steer = a.steer - d.steer;
            let fire = f32::from(a.fire != d.fire);
            w * (drive * drive + steer * steer + fire * 2.0)
        })
        .sum::<f32>()
        + samples.iter().flatten().sum::<f32>() * 0.0001
}

#[cfg(test)]
mod tests {
    use super::*;
    use arena_core::{
        JointKind, Mat4, Quat,
        mechanism::{RigBody, RigJoint},
        robot::{ControlBinding, Controller, RobotComponent},
    };

    fn component(class: &str) -> RobotComponent {
        RobotComponent {
            parent: Some(0),
            parent_mount: 0,
            own_mount: 0,
            mounting: Default::default(),
            class: class.into(),
            definition: String::new(),
            settings: Default::default(),
        }
    }
    fn joint(component: usize, child: usize, axis: Vec3, kind: ActuatorKind) -> RigJoint {
        RigJoint {
            component,
            parent: 0,
            child,
            anchor: Vec3::ZERO,
            axis,
            child_rotation: Quat::IDENTITY,
            kind,
            motion: JointKind::Slider {
                limits: [-1.0, 1.0],
            },
            limits: None,
            speed: 1.0,
            return_speed: 1.0,
            force: 1.0,
            burst_force: 1.0,
            electricity: 0.0,
            air: 0.0,
            start: 0.0,
            stop: 1.0,
        }
    }
    fn fixture() -> (RobotDefinition, Rig) {
        let mut r = arena_core::editor::Workshop::new().robot;
        r.components.extend([
            component("SpinMotor"),
            component("Wheel"),
            component("SpinMotor"),
            component("Wheel"),
            component("BurstPiston"),
        ]);
        r.controllers = (0..3)
            .map(|i| Controller {
                name: String::new(),
                position: [i, 0],
                kind: 1,
                keys: vec![vec![], vec![]],
            })
            .collect();
        for (c, key, part, action) in [
            (0, 0, 1, "Spin Clockwise"),
            (0, 1, 1, "Spin Counter-clockwise"),
            (1, 0, 3, "Spin Clockwise"),
            (1, 1, 3, "Spin Counter-clockwise"),
            (2, 0, 5, "Fire"),
        ] {
            r.bindings.push(ControlBinding {
                component: part,
                controller: [c, 0],
                key,
                action: action.into(),
            });
        }
        let mut rig = Rig::default();
        rig.bodies = [-1.0, 1.0, 0.0]
            .into_iter()
            .map(|x| RigBody {
                name: String::new(),
                frame: Mat4::from_translation(Vec3::new(x, 0.0, 0.0)),
                mass: 1.0,
            })
            .collect();
        rig.component_bodies = vec![0, 0, 0, 0, 1, 2];
        rig.objects = vec![vec![], vec![], vec![0], vec![], vec![1], vec![]];
        // Motors drive opposite-facing wheel axes; their child bodies carry the wheels.
        rig.joints = vec![
            joint(1, 0, Vec3::X, ActuatorKind::Spin),
            joint(3, 1, -Vec3::X, ActuatorKind::Spin),
            joint(5, 2, Vec3::Y, ActuatorKind::BurstPiston),
        ];
        (r, rig)
    }
    #[test]
    fn differential_drive_and_burst_use_native_controls() {
        let (r, rig) = fixture();
        let analysis = WiringAnalysis::new(&r, &rig);
        assert_eq!(analysis.wheel_count(), 2);
        assert_eq!(analysis.weapons(), &[5]);
        let mut ai = AiControls::new(analysis);
        let mut native = Controls::default();
        let out = native.evaluate(
            &r,
            &ai.samples(Intent {
                forward: 1.0,
                turn: 0.0,
                fire: true,
            }),
        );
        assert_eq!((out[1].drive, out[3].drive), (-1.0, 1.0));
        assert!(out[5].fire);
        let out = native.evaluate(
            &r,
            &ai.samples(Intent {
                forward: 0.0,
                turn: 1.0,
                fire: true,
            }),
        );
        assert_eq!((out[1].drive, out[3].drive), (1.0, 1.0));
        assert!(!out[5].fire);
        let out = native.evaluate(
            &r,
            &ai.samples(Intent {
                fire: true,
                ..Default::default()
            }),
        );
        assert!(!out[5].fire);
    }
    #[test]
    fn switches_and_on_off_actions_release_and_toggle() {
        let (mut r, rig) = fixture();
        r.controllers[0].kind = 2;
        r.bindings[3].action = "On/Off Counter-clockwise".into();
        let mut ai = AiControls::new(WiringAnalysis::new(&r, &rig));
        let mut native = Controls::default();
        let go = Intent {
            forward: 1.0,
            ..Default::default()
        };
        for _ in 0..3 {
            let out = native.evaluate(&r, &ai.samples(go));
            assert_eq!((out[1].drive, out[3].drive), (-1.0, 1.0));
        }
        let out = native.evaluate(&r, &ai.samples(Intent::default()));
        assert_eq!((out[1].drive, out[3].drive), (0.0, 0.0));
    }
    #[test]
    fn heading_rotates_geometry_and_invalid_bindings_are_ignored() {
        let (mut r, rig) = fixture();
        r.heading = std::f32::consts::FRAC_PI_2;
        r.bindings.push(ControlBinding {
            component: 999,
            controller: [99, 99],
            key: 100,
            action: "Fire".into(),
        });
        let mut ai = AiControls::new(WiringAnalysis::new(&r, &rig));
        let out = Controls::default().evaluate(
            &r,
            &ai.samples(Intent {
                forward: 1.0,
                turn: f32::NAN,
                fire: false,
            }),
        );
        assert_eq!((out[1].drive, out[3].drive), (-1.0, 1.0));
    }
    /// Run with RA2_GAME_DIR pointing at the original game data.
    #[test]
    #[ignore = "requires original RA2 models"]
    fn original_models_produce_wired_drive_signals() {
        use ra2_formats::{
            ModelImporters,
            assembly::{ResolvedComponent, assemble},
            components::{ComponentCatalog, ComponentDefinition, normalize},
        };
        use std::{
            collections::BTreeMap,
            path::{Path, PathBuf},
            sync::Arc,
        };
        fn scan(path: &Path, index: &mut BTreeMap<String, PathBuf>, root: &Path) {
            if path.is_dir() {
                for entry in std::fs::read_dir(path).unwrap().flatten() {
                    scan(&entry.path(), index, root);
                }
            } else if let Ok(relative) = path.strip_prefix(root) {
                index.insert(normalize(&relative.to_string_lossy()), path.to_owned());
            }
        }
        let root = PathBuf::from(std::env::var_os("RA2_GAME_DIR").expect("set RA2_GAME_DIR"));
        let mut index = BTreeMap::new();
        scan(&root, &mut index, &root);
        let catalog =
            ComponentCatalog::from_cfz(&std::fs::read(root.join("Components.cfz")).unwrap())
                .unwrap();
        let decoder = ModelImporters::with_ra2();
        for path in [
            "AI/Team0/Bot0.bot",
            "AI/Team0/Bot2.bot",
            "AI/Team2/Bot2.bot",
            "Teams/Team0/Bot0.bot",
            "AI/Team13/Bot2.bot",
        ] {
            let robot = ra2_formats::bot::decode(&std::fs::read(root.join(path)).unwrap()).unwrap();
            let mut models = vec![ResolvedComponent {
                key: "chassis".into(),
                model: Arc::new(robot.chassis.model.clone().unwrap()),
                hidden: false,
            }];
            let mut definitions = vec![ComponentDefinition::parse("")];
            for part in robot.components.iter().skip(1) {
                let def = catalog.get(&part.definition).unwrap().clone();
                let key = normalize(&def.model_path().unwrap());
                let model = decoder
                    .decode(&std::fs::read(&index[&key]).unwrap())
                    .unwrap()
                    .model;
                models.push(ResolvedComponent {
                    key,
                    model: Arc::new(model),
                    hidden: part.class == "SmartZone",
                });
                definitions.push(def);
            }
            let assembly = assemble(&robot, &models).unwrap();
            let rig =
                ra2_formats::mechanisms::build(&robot, &assembly, &models, &definitions).unwrap();
            let analysis = WiringAnalysis::new(&robot, &rig);
            assert!(analysis.wheel_count() > 0, "{path}: no wheel drives");
            let mut ai = AiControls::new(analysis.clone());
            let samples = ai.samples(Intent {
                forward: 1.0,
                ..Default::default()
            });
            let commands = Controls::default().evaluate(&robot, &samples);
            assert!(
                analysis.wheels.iter().any(|w| {
                    let joint = rig
                        .joints
                        .iter()
                        .find(|j| j.component == w.component)
                        .unwrap();
                    let traction = joint.axis.cross(Vec3::Y) * commands[w.component].drive;
                    traction.dot(Vec3::NEG_Z) > 0.5
                }),
                "{path}: no forward wheel command"
            );
        }
    }
    #[test]
    fn weapon_with_only_reverse_binding_is_driven() {
        let (mut robot, mut rig) = fixture();
        rig.joints[2].kind = ActuatorKind::Spin;
        let binding = robot
            .bindings
            .iter_mut()
            .find(|binding| binding.component == 5)
            .unwrap();
        binding.action = "Spin Counter-clockwise".into();
        let mut ai = AiControls::new(WiringAnalysis::new(&robot, &rig));
        let mut controls = Controls::default();
        let commands = controls.evaluate(
            &robot,
            &ai.samples(Intent {
                fire: true,
                ..Default::default()
            }),
        );
        assert_eq!(commands[5].drive, -1.0);
    }

    #[test]
    fn spinup_does_not_trigger_short_range_weapon() {
        let (mut robot, mut rig) = fixture();
        robot.components.push(component("SpinMotor"));
        let spin = robot.components.len() - 1;
        robot.controllers.push(Controller {
            name: "spinner".into(),
            position: [6, 0],
            kind: 1,
            keys: vec![vec![]],
        });
        robot.bindings.push(ControlBinding {
            component: spin,
            controller: [6, 0],
            key: 0,
            action: "Spin Clockwise".into(),
        });
        rig.joints.push(joint(spin, 2, Vec3::Y, ActuatorKind::Spin));
        let mut ai = AiControls::new(WiringAnalysis::new(&robot, &rig));
        let mut controls = Controls::default();
        let commands = controls.evaluate(&robot, &ai.samples_with_spin(Intent::default(), true));
        assert!(commands[spin].drive > 0.5);
        assert!(
            !commands[5].fire,
            "distant enemy must not trigger the hammer"
        );
        let commands = controls.evaluate(
            &robot,
            &ai.samples_with_spin(
                Intent {
                    fire: true,
                    ..Default::default()
                },
                true,
            ),
        );
        assert!(commands[5].fire, "nearby enemy should trigger the hammer");
    }

    #[test]
    fn burst_cooldown_tracks_stroke_and_does_not_reset_on_release() {
        let (r, rig) = fixture();
        let mut ai = AiControls::new(WiringAnalysis::new(&r, &rig));
        let mut native = Controls::default();
        let mut fired = Vec::new();
        for tick in 0..600 {
            let want = tick != 30;
            let commands = native.evaluate(
                &r,
                &ai.samples(Intent {
                    fire: want,
                    ..Default::default()
                }),
            );
            if commands[5].fire {
                fired.push(tick);
            }
        }
        // The fixture stroke takes 1.5 + 1/1 = 2.5 seconds.
        assert_eq!(fired, vec![0, 300]);
    }

    #[test]
    fn steering_joint_controls_heading_without_tank_mixing() {
        let (mut r, mut rig) = fixture();
        r.components.push(component("Steering"));
        let i = r.components.len() - 1;
        r.controllers.push(Controller {
            name: String::new(),
            position: [4, 0],
            kind: 1,
            keys: vec![vec![], vec![]],
        });
        for (key, action) in [(0, "Left"), (1, "Right")] {
            r.bindings.push(ControlBinding {
                component: i,
                controller: [4, 0],
                key,
                action: action.into(),
            });
        }
        rig.joints
            .push(joint(i, 2, -Vec3::Y, ActuatorKind::Steering));
        let mut ai = AiControls::new(WiringAnalysis::new(&r, &rig));
        let commands = Controls::default().evaluate(
            &r,
            &ai.samples(Intent {
                forward: 1.0,
                turn: 1.0,
                fire: false,
            }),
        );
        assert_eq!((commands[1].drive, commands[3].drive), (-1.0, 1.0));
        assert_eq!(commands[i].steer, -1.0);
    }

    #[test]
    fn steady_inputs_use_cache_and_still_allow_switch_changes() {
        let (mut r, rig) = fixture();
        r.controllers[0].kind = 2;
        let mut ai = AiControls::new(WiringAnalysis::new(&r, &rig));
        let mut native = Controls::default();
        let intent = Intent {
            forward: 1.0,
            ..Default::default()
        };
        for _ in 0..10 {
            native.evaluate(&r, &ai.samples(intent));
        }
        assert!(ai.cached.is_some());
        for _ in 0..120 {
            let out = native.evaluate(&r, &ai.samples(intent));
            assert_eq!((out[1].drive, out[3].drive), (-1.0, 1.0));
            assert!(ai.cached.is_some());
        }
        let out = native.evaluate(&r, &ai.samples(Intent::default()));
        assert_eq!((out[1].drive, out[3].drive), (0.0, 0.0));
    }

    #[test]
    #[ignore = "requires original robot and garage assets"]
    fn native_scout_weapon_actually_moves_under_ai_control() {
        use crate::garage::{Garage, tests::native_garage};
        use bevy::prelude::Transform;
        let (mut app, robot) = native_garage("AI/Team0/Bot0.bot");
        for _ in 0..240 {
            app.update();
        }
        let (rig, bodies) = {
            let garage = app.world().resource::<Garage>();
            (garage.rig.clone().unwrap(), garage.bodies.clone())
        };
        let (joint_index, joint) = rig
            .joints
            .iter()
            .enumerate()
            .find(|(_, joint)| {
                joint.kind == ActuatorKind::Burst
                    && robot.bindings.iter().any(|binding| {
                        binding.component == joint.component && binding.action == "Fire"
                    })
            })
            .expect("Scout must have a wired burst weapon");
        let relative = |app: &bevy::prelude::App| {
            let parent = app.world().get::<Transform>(bodies[joint.parent]).unwrap();
            let child = app.world().get::<Transform>(bodies[joint.child]).unwrap();
            parent.rotation.inverse() * child.rotation
        };
        let initial = relative(&app);
        let mut ai = AiControls::new(WiringAnalysis::new(&robot, &rig));
        let mut largest_angle = 0.0_f32;
        let mut activated = false;
        for _ in 0..240 {
            let samples = ai.samples(Intent {
                fire: true,
                ..Default::default()
            });
            let mut garage = app.world_mut().resource_mut::<Garage>();
            garage.commands = garage.controls.evaluate(&robot, &samples);
            drop(garage);
            app.update();
            activated |= app.world().resource::<Garage>().states[joint_index].burst_time > 0.0;
            largest_angle = largest_angle.max(initial.angle_between(relative(&app)));
        }
        assert!(activated, "AI never triggered the burst actuator");
        assert!(
            largest_angle > 0.05,
            "AI fired but the weapon did not move ({largest_angle})"
        );
    }

    #[test]
    #[ignore = "requires original robot and garage assets"]
    fn native_robots_drive_headlessly() {
        use crate::garage::{Garage, tests::native_garage};
        use bevy::prelude::{Transform, Vec3 as BevyVec3};
        for path in [
            "AI/Team0/Bot0.bot",
            "AI/Team0/Bot2.bot",
            "AI/Team2/Bot2.bot",
        ] {
            let (mut app, robot) = native_garage(path);
            for _ in 0..240 {
                app.update();
            }
            let (body, rig) = {
                let g = app.world().resource::<Garage>();
                (g.bodies[0], g.rig.clone().unwrap())
            };
            let analysis = WiringAnalysis::new(&robot, &rig);
            let mut bench = AiControls::new(analysis.clone());
            let now = std::time::Instant::now();
            for tick in 0..1200 {
                std::hint::black_box(bench.samples(Intent {
                    forward: 0.7,
                    turn: (tick as f32 * 0.01).sin(),
                    fire: true,
                }));
            }
            println!("{path}: 1200 changing samples in {:?}", now.elapsed());
            let mut ai = AiControls::new(analysis);
            let start = *app.world().get::<Transform>(body).unwrap();
            let mut shots = 0;
            for _ in 0..240 {
                let samples = ai.samples(Intent {
                    forward: 1.0,
                    turn: 0.0,
                    fire: true,
                });
                let mut g = app.world_mut().resource_mut::<Garage>();
                g.commands = g.controls.evaluate(&robot, &samples);
                shots += g.commands.iter().filter(|c| c.fire).count();
                drop(g);
                app.update();
            }
            let end = app.world().get::<Transform>(body).unwrap().translation;
            let distance = (end - start.translation).dot(start.rotation * BevyVec3::NEG_Z);
            println!("{path}: distance {distance:.2} m, burst commands {shots}");
            assert!(
                shots > 0 && shots <= 4,
                "{path}: invalid burst frequency {shots}"
            );
            if path == "AI/Team0/Bot2.bot" {
                let before = *app.world().get::<Transform>(body).unwrap();
                for _ in 0..240 {
                    let samples = ai.samples(Intent {
                        forward: 0.7,
                        turn: 1.0,
                        fire: false,
                    });
                    let mut g = app.world_mut().resource_mut::<Garage>();
                    g.commands = g.controls.evaluate(&robot, &samples);
                    drop(g);
                    app.update();
                }
                let after = app.world().get::<Transform>(body).unwrap();
                let from = (before.rotation * BevyVec3::NEG_Z)
                    .with_y(0.0)
                    .normalize_or_zero();
                let to = (after.rotation * BevyVec3::NEG_Z)
                    .with_y(0.0)
                    .normalize_or_zero();
                let left_turn = from.cross(to).y;
                println!("{path}: left turn {left_turn:.3}");
                assert!(
                    left_turn > 0.05,
                    "{path}: steering did not turn left ({left_turn})"
                );
            }
            assert!(
                distance > 0.1,
                "{path}: forward displacement {distance}, start {:?}, end {:?}",
                start.translation,
                end
            );
        }
    }
}
