//! RA2 body/constraint/CFZ conventions -> backend-independent articulated rig.
use crate::{assembly::ResolvedComponent, components::ComponentDefinition};
use arena_core::{mechanism::*, robot::*, *};
use std::collections::BTreeMap;
fn numbers(s: &str) -> Vec<f32> {
    s.split([',', ' '])
        .filter_map(|s| s.trim().parse().ok())
        .collect()
}
fn scalar(d: &ComponentDefinition, key: &str) -> f32 {
    d.value(key).parse().unwrap_or(0.0)
}
fn key(s: &str) -> String {
    s.to_ascii_lowercase()
}
fn id(a: &Attachment) -> Option<u32> {
    a.properties.get("id")?.parse().ok()
}

pub fn build(
    robot: &RobotDefinition,
    assembly: &RobotAssembly,
    models: &[ResolvedComponent],
    definitions: &[ComponentDefinition],
) -> Result<Rig, String> {
    if models.len() != robot.components.len() || definitions.len() != models.len() {
        return Err("Rig component count mismatch".into());
    }
    let mut rig = Rig {
        bodies: vec![RigBody {
            name: "Chassis".into(),
            frame: Mat4::IDENTITY,
            mass: assembly.parts[0].mass,
        }],
        objects: vec![vec![]; models.len()],
        component_bodies: vec![0; models.len()],
        attachment_bodies: vec![vec![]; models.len()],
        ..Default::default()
    };
    let mut owners: Vec<Option<BTreeMap<String, usize>>> = vec![None; models.len()];
    let mut visiting = vec![false; models.len()];
    #[allow(clippy::too_many_arguments)]
    fn component(
        i: usize,
        robot: &RobotDefinition,
        assembly: &RobotAssembly,
        models: &[ResolvedComponent],
        defs: &[ComponentDefinition],
        rig: &mut Rig,
        owners: &mut [Option<BTreeMap<String, usize>>],
        visiting: &mut [bool],
    ) -> Result<(), String> {
        if owners[i].is_some() {
            return Ok(());
        }
        if visiting[i] {
            return Err("Cyclic rig".into());
        }
        visiting[i] = true;
        let model = &models[i].model;
        let definition = &defs[i];
        let part = &assembly.parts[i];
        let base = if i == 0 {
            0
        } else {
            let p = robot.components[i].parent.ok_or("Detached part")?;
            component(p, robot, assembly, models, defs, rig, owners, visiting)?;
            let socket = models[p]
                .model
                .attachments
                .iter()
                .find(|a| id(a) == Some(robot.components[i].parent_mount));
            let host = socket
                .and_then(|a| a.properties.get("host"))
                .cloned()
                .unwrap_or_else(|| {
                    if socket
                        .and_then(|a| a.properties.get("attach"))
                        .is_some_and(|s| s.to_ascii_lowercase().ends_with("_m"))
                    {
                        defs[p].value("attaching").to_string()
                    } else {
                        String::new()
                    }
                });
            owners[p]
                .as_ref()
                .and_then(|m| m.get(&key(&host)))
                .copied()
                .unwrap_or_else(|| owners[p].as_ref().unwrap()[""])
        };
        let mut map = BTreeMap::from([(String::new(), base)]);
        rig.component_bodies[i] = base;
        let mut source_joints = model.joints.clone();
        let prism = definition
            .value("prismatic")
            .split(',')
            .map(str::trim)
            .collect::<Vec<_>>();
        if prism.len() >= 8 {
            let axis = Vec3::new(
                prism[2].parse().unwrap_or(0.0),
                prism[3].parse().unwrap_or(0.0),
                -prism[4].parse::<f32>().unwrap_or(1.0),
            );
            // Native pistons extend towards their male connector, opposite the authored slider axis.
            let limits = [
                -prism[7].parse::<f32>().unwrap_or(0.2),
                -prism[6].parse::<f32>().unwrap_or(-1.0),
            ];
            source_joints.push(ModelJoint {
                name: "Prismatic".into(),
                bodies: [prism[0].into(), prism[1].into()],
                pivot: Vec3::ZERO,
                axis: -axis,
                kind: JointKind::Slider { limits },
            });
        }
        let master = key(definition.value("master"));
        if !master.is_empty() {
            map.insert(master.clone(), base);
        }
        if let Some(first) = source_joints.first() {
            map.entry(key(&first.bodies[0])).or_insert(base);
        }
        let power = numbers(definition.value("power"));
        let kind = match robot.components[i].class.as_str() {
            "SpinMotor" => ActuatorKind::Spin,
            "ServoMotor" => ActuatorKind::Servo,
            "BurstMotor" => ActuatorKind::Burst,
            "ServoPiston" => ActuatorKind::Piston,
            "BurstPiston" => ActuatorKind::BurstPiston,
            "Steering" => ActuatorKind::Steering,
            _ => ActuatorKind::Passive,
        };
        let mut pending = source_joints.iter().collect::<Vec<_>>();
        while !pending.is_empty() {
            let before = pending.len();
            pending.retain(|j| {
                let Some(&parent) = map.get(&key(&j.bodies[0])) else {
                    return true;
                };
                if map.contains_key(&key(&j.bodies[1])) {
                    return false;
                }
                let child = rig.bodies.len();
                let anchor = part.transform.transform_point3(j.pivot);
                rig.bodies.push(RigBody {
                    name: format!("{} / {}", i, j.bodies[1]),
                    frame: Mat4::from_translation(anchor),
                    mass: 0.0,
                });
                map.insert(key(&j.bodies[1]), child);
                let limits = match j.kind {
                    JointKind::Hinge {
                        limited: true,
                        limits,
                        ..
                    }
                    | JointKind::Slider { limits } => Some(limits),
                    _ => None,
                };
                let settings = &robot.components[i].settings;
                let start = settings
                    .get("start_angle")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0.0);
                let stop = settings
                    .get("stop_angle")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or_else(|| limits.map_or(std::f32::consts::PI, |l| l[1]));
                let burst = numbers(definition.value("burst"));
                // Clockwise is defined at the output connector. Shipped motors
                // use opposite GMF hinge windings (e.g. Z-Tek versus Red Bird).
                let mut axis = j.axis;
                if kind == ActuatorKind::Spin
                    && let Some(socket) = model.attachments.iter().find(|a| {
                        a.properties
                            .get("attach")
                            .is_some_and(|t| t.to_ascii_lowercase().ends_with("_m"))
                    })
                    && axis.dot(socket.transform.z_axis.truncate()) < 0.
                {
                    axis = -axis;
                }
                let mut spec = RigJoint {
                    component: i,
                    parent,
                    child,
                    anchor,
                    axis: part.transform.transform_vector3(axis).normalize_or_zero(),
                    child_rotation: (part.transform
                        * model
                            .all_bodies()
                            .into_iter()
                            .find(|b| b.name.eq_ignore_ascii_case(&j.bodies[1]))
                            .map_or(Mat4::IDENTITY, |b| b.transform))
                    .to_scale_rotation_translation()
                    .1
                    .normalize(),
                    kind,
                    motion: j.kind.clone(),
                    limits,
                    speed: if kind == ActuatorKind::Burst {
                        burst.get(1).copied().unwrap_or(40.0)
                    } else {
                        power.first().copied().unwrap_or(12.0)
                    },
                    return_speed: power.first().copied().unwrap_or(5.0),
                    force: power.get(1).copied().unwrap_or(40.0),
                    burst_force: burst
                        .get(2)
                        .copied()
                        .or_else(|| power.get(1).copied())
                        .unwrap_or(40.0),
                    electricity: (-scalar(definition, "elecmaxinoutrate")).max(0.0),
                    air: (-scalar(definition, "airmaxinoutrate")).max(0.0),
                    start,
                    stop,
                };
                if kind == ActuatorKind::Steering {
                    // A car wheel has two independent hinges: steering knuckle,
                    // then rolling hub. A single two-axis Euler motor couples
                    // yaw to wheel revolutions and can launch the whole robot.
                    let knuckle = rig.bodies.len();
                    rig.bodies.push(RigBody {
                        name: format!("{} / {} steering knuckle", i, j.bodies[1]),
                        frame: Mat4::from_translation(anchor),
                        mass: 0.0,
                    });
                    let local_axis = if let JointKind::Ragdoll { parameters } = &j.kind {
                        Vec3::new(parameters[0], parameters[1], parameters[2]).normalize_or(Vec3::Y)
                    } else {
                        Vec3::Y
                    };
                    let mut pivot = spec.clone();
                    pivot.child = knuckle;
                    pivot.axis = part.transform.transform_vector3(local_axis).normalize();
                    pivot.child_rotation = Quat::IDENTITY;
                    pivot.start = 0.;
                    pivot.stop = 0.6;
                    pivot.limits = Some([-0.6, 0.6]);
                    pivot.motion = JointKind::Hinge {
                        limited: true,
                        friction: 0.,
                        limits: [-0.6, 0.6],
                    };
                    pivot.electricity *= 0.1;
                    rig.joints.push(pivot);
                    spec.parent = knuckle;
                    spec.kind = ActuatorKind::Spin;
                    spec.limits = None;
                    spec.motion = JointKind::Hinge {
                        limited: false,
                        friction: 0.,
                        limits: [0., 0.],
                    };
                    spec.electricity *= 0.9;
                }
                rig.joints.push(spec);
                false
            });
            if pending.len() == before {
                return Err(format!(
                    "Unresolved body constraints in {}",
                    robot.components[i].definition
                ));
            }
        }
        // Body groups own their proxies and display nodes; child objects inherit that ownership.
        for body in model.all_bodies() {
            let group = map.get(&key(&body.name)).copied().unwrap_or(base);
            map.insert(key(&body.name), group);
            for name in [&body.proxy, &body.display_proxy] {
                if !name.is_empty() {
                    map.insert(key(name), group);
                }
            }
            if i > 0 && !body.disabled {
                rig.bodies[group].mass += body.mass;
            }
        }
        for _ in 0..model.objects.len() {
            let mut changed = false;
            for o in &model.objects {
                if !map.contains_key(&key(&o.name))
                    && let Some(g) = o.parent.as_ref().and_then(|p| map.get(&key(p))).copied()
                {
                    map.insert(key(&o.name), g);
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        rig.objects[i] = model
            .objects
            .iter()
            .map(|o| map.get(&key(&o.name)).copied().unwrap_or(base))
            .collect();
        rig.attachment_bodies[i] = model
            .attachments
            .iter()
            .map(|a| {
                let host = a
                    .properties
                    .get("host")
                    .map(String::as_str)
                    .unwrap_or_else(|| {
                        if a.properties
                            .get("attach")
                            .is_some_and(|s| s.to_ascii_lowercase().ends_with("_m"))
                        {
                            definition.value("attaching")
                        } else {
                            ""
                        }
                    });
                map.get(&key(host)).copied().unwrap_or(base)
            })
            .collect();
        owners[i] = Some(map);
        visiting[i] = false;
        Ok(())
    }
    for i in 0..models.len() {
        component(
            i,
            robot,
            assembly,
            models,
            definitions,
            &mut rig,
            &mut owners,
            &mut visiting,
        )?;
        let d = &definitions[i];
        rig.electrical.capacity += scalar(d, "electotal").max(0.0);
        rig.electrical.max_rate += scalar(d, "elecmaxinoutrate").max(0.0);
        rig.pneumatic.capacity += scalar(d, "airtotal").max(0.0);
        rig.pneumatic.max_rate += scalar(d, "airmaxinoutrate").max(0.0);
        rig.control_board |= robot.components[i].class == "ControlBoard";
    }
    // Split existing hub/wheel mass across the two hinge bodies. A nearly
    // massless knuckle makes an impulse solver ill-conditioned; preserve the
    // total original assembly mass instead of adding artificial ballast.
    for pivot in rig
        .joints
        .iter()
        .filter(|j| j.kind == ActuatorKind::Steering)
    {
        if let Some(hub) = rig
            .joints
            .iter()
            .find(|j| j.parent == pivot.child && j.kind == ActuatorKind::Spin)
        {
            let mass = rig.bodies[hub.child].mass * 0.25;
            rig.bodies[pivot.child].mass = mass;
            rig.bodies[hub.child].mass -= mass;
        }
    }
    rig.electrical.refill();
    rig.pneumatic.refill();
    Ok(rig)
}
