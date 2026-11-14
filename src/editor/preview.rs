//! Kinematic Wiring preview. Shares input decoding and actuator state machines
//! with the garage, but never changes the document, supplies or physics world.
use super::{catalog::Catalog, controls};
use arena_core::{mechanism::*, robot::RobotDefinition};
use bevy::prelude::*;
use ra2_formats::{
    assembly::{self, ResolvedComponent},
    components::{ComponentDefinition, normalize},
};
use std::sync::Arc;

pub struct Preview {
    pub rig: Rig,
    rest: Vec<Mat4>,
    controls: Controls,
    states: Vec<ActuatorState>,
    positions: Vec<f32>,
    pub objects: Vec<(Entity, usize, Mat4, Mat4)>,
}
impl Preview {
    pub fn new(catalog: &Catalog, robot: &RobotDefinition) -> Result<Self, String> {
        let mut models = vec![ResolvedComponent {
            key: "@chassis".into(),
            model: Arc::new(robot.chassis.model.clone().ok_or("Missing chassis")?),
            hidden: false,
        }];
        let mut definitions = vec![ComponentDefinition::parse("")];
        for c in robot.components.iter().skip(1) {
            let a = catalog
                .components
                .get(&normalize(&c.definition))
                .ok_or("Missing component")?;
            let model = ra2_formats::pose::bind_model(&a.model, &a.definition)
                .map(Arc::new)
                .unwrap_or_else(|| a.model.clone());
            models.push(ResolvedComponent {
                key: a.key.clone(),
                model,
                hidden: c.class == "SmartZone",
            });
            definitions.push(a.definition.clone());
        }
        let assembly = assembly::assemble(robot, &models)?;
        let mut rig = ra2_formats::mechanisms::build(robot, &assembly, &models, &definitions)?;
        let rest = ra2_formats::pose::body_frames(&rig)?;
        for j in &mut rig.joints {
            if j.kind == ActuatorKind::Burst {
                j.start = 0.;
                j.limits = Some([0_f32.min(j.stop), 0_f32.max(j.stop)]);
            }
        }
        let states = rig
            .joints
            .iter()
            .map(|_| ActuatorState::default())
            .collect();
        let positions = rig.joints.iter().map(|j| j.start).collect();
        Ok(Self {
            rig,
            rest,
            controls: Controls::default(),
            states,
            positions,
            objects: vec![],
        })
    }
    pub fn update(
        &mut self,
        commands: &mut Commands,
        robot: &RobotDefinition,
        keys: &ButtonInput<KeyCode>,
        enabled: bool,
        dt: f32,
    ) {
        let samples = controls::samples(robot, keys, enabled);
        let inputs = self.controls.evaluate(robot, &samples);
        let dt = dt.clamp(0., 0.05);
        for (i, j) in self.rig.joints.iter().enumerate() {
            let (target, speed, _) =
                self.states[i].step(j, inputs[j.component], dt, Some(self.positions[i]));
            self.positions[i] = if let Some(target) = target {
                if matches!(j.kind, ActuatorKind::Burst | ActuatorKind::BurstPiston) {
                    self.positions[i]
                        + (target - self.positions[i])
                            .clamp(-speed.abs() * dt * 1.35, speed.abs() * dt * 1.35)
                } else {
                    target
                }
            } else {
                (self.positions[i] + speed * dt).rem_euclid(std::f32::consts::TAU)
            };
        }
        let mut delta = vec![None; self.rest.len()];
        fn solve(i: usize, preview: &Preview, delta: &mut [Option<Mat4>]) -> Mat4 {
            if let Some(d) = delta[i] {
                return d;
            }
            let d = if let Some((n, j)) = preview
                .rig
                .joints
                .iter()
                .enumerate()
                .find(|(_, j)| j.child == i)
            {
                let inherited = solve(j.parent, preview, delta);
                let p = preview.positions[n] - j.start;
                let axis = (preview.rest[j.parent] * preview.rig.bodies[j.parent].frame.inverse())
                    .transform_vector3(j.axis)
                    .normalize();
                let anchor = preview.rest[j.child].transform_point3(
                    preview.rig.bodies[j.child]
                        .frame
                        .inverse()
                        .transform_point3(j.anchor),
                );
                let motion = if matches!(j.kind, ActuatorKind::Piston | ActuatorKind::BurstPiston) {
                    Mat4::from_translation(axis * p)
                } else {
                    Mat4::from_translation(anchor)
                        * Mat4::from_quat(Quat::from_axis_angle(axis, p))
                        * Mat4::from_translation(-anchor)
                };
                inherited * motion
            } else {
                Mat4::IDENTITY
            };
            delta[i] = Some(d);
            d
        }
        for i in 0..delta.len() {
            solve(i, self, &mut delta);
        }
        for &(entity, body, parent_inverse, original) in &self.objects {
            commands.entity(entity).insert(Transform::from_matrix(
                parent_inverse * delta[body].unwrap() * original,
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires native robot models"]
    fn native_wiring_preview_spins_wheels_and_cycles_weapons_without_editing() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../RobotArena2");
        let catalog = Catalog::load(
            &root,
            &mut crate::legacy_ui::assets::LegacyAssets::default(),
        )
        .unwrap();
        for file in [
            "Teams/Team1/Bot0.bot",
            "Teams/Team0/Bot0.bot",
            "AI/Team6/Bot1.bot",
        ] {
            let robot = ra2_formats::bot::decode(&std::fs::read(root.join(file)).unwrap()).unwrap();
            let saved = ra2_formats::bot::encode(&robot).unwrap();
            let mut p = Preview::new(&catalog, &robot).unwrap();
            let mut keys = ButtonInput::default();
            keys.press(KeyCode::KeyW);
            keys.press(KeyCode::Space);
            let mut world = World::new();
            let mut queue = bevy::ecs::world::CommandQueue::default();
            let mut peak = vec![0_f32; p.rig.joints.len()];
            for _ in 0..6 {
                p.update(
                    &mut Commands::new(&mut queue, &world),
                    &robot,
                    &keys,
                    true,
                    1. / 120.,
                );
                for (i, j) in p.rig.joints.iter().enumerate() {
                    peak[i] = peak[i].max((p.positions[i] - j.start).abs());
                }
            }
            assert!(
                p.rig
                    .joints
                    .iter()
                    .enumerate()
                    .any(|(i, j)| j.kind == ActuatorKind::Spin
                        && (p.positions[i] - j.start).abs() > 0.1),
                "{} wheels did not preview",
                robot.name
            );
            let fired: Vec<_> = p
                .rig
                .joints
                .iter()
                .enumerate()
                .filter(|(_, j)| {
                    matches!(j.kind, ActuatorKind::Burst | ActuatorKind::BurstPiston)
                        && robot
                            .bindings
                            .iter()
                            .any(|b| b.component == j.component && b.action == "Fire")
                })
                .map(|(i, _j)| {
                    assert!(peak[i] > 0.05, "{} striker did not preview", robot.name);
                    i
                })
                .collect();
            keys.release_all();
            for _ in 0..480 {
                p.update(
                    &mut Commands::new(&mut queue, &world),
                    &robot,
                    &keys,
                    true,
                    1. / 120.,
                );
            }
            for i in fired {
                assert!(
                    (p.positions[i] - p.rig.joints[i].start).abs() < 0.05,
                    "{} did not return",
                    robot.name
                );
            }
            queue.apply(&mut world);
            assert_eq!(ra2_formats::bot::encode(&robot).unwrap(), saved);
            println!(
                "Wiring live preview {}: drive, fire, return; BOT unchanged",
                robot.name
            );
        }
    }
}
