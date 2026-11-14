//! Freeze a native articulated robot in its configured initial pose.
//! Authored mounting frames and saved BOT data are never modified. Consumers use
//! the same posed models for rendering, picking, bounds and rigid collision.
use crate::{
    assembly::{self, ResolvedComponent},
    components::ComponentDefinition,
    mechanisms,
};
use arena_core::{
    mechanism::{ActuatorKind, Rig},
    robot::{RobotAssembly, RobotDefinition},
    *,
};
use std::sync::Arc;

pub struct FrozenRobot {
    pub assembly: RobotAssembly,
    pub models: Vec<ResolvedComponent>,
}

/// Convert display/proxy geometry into the master body's rigid frame. Native
/// component geometry is attached to that body; its export TM is not a second
/// body rotation. Attachment records already describe connector-local frames.
/// Preserve mesh scale and source vertices/UVs. Scene imports do not use this.
pub fn bind_model(model: &Model, definition: &ComponentDefinition) -> Option<Model> {
    let bodies = model.all_bodies();
    let master = if definition.value("master").is_empty() {
        model
            .joints
            .first()
            .and_then(|j| {
                bodies
                    .iter()
                    .copied()
                    .find(|b| b.name.eq_ignore_ascii_case(&j.bodies[0]))
            })
            .or_else(|| model.bodies.first())
    } else {
        bodies
            .iter()
            .copied()
            .find(|b| b.name.eq_ignore_ascii_case(definition.value("master")))
    }?;
    // Scale belongs to the mesh, not the rigid mounting frame. In particular the
    // shipped long round extender has a non-unit master-node Y scale.
    let (_, rotation, translation) = master.transform.to_scale_rotation_translation();
    let frame = Mat4::from_rotation_translation(rotation.normalize(), translation);
    if frame.abs_diff_eq(Mat4::IDENTITY, 0.00001) {
        return None;
    }
    let inverse = frame.inverse();
    let mut result = model.clone();
    for object in &mut result.objects {
        object.transform = inverse * object.transform;
    }
    fn bodies_in_frame(bodies: &mut [Body], inverse: Mat4) {
        for body in bodies {
            body.transform = inverse * body.transform;
            bodies_in_frame(&mut body.children, inverse);
        }
    }
    bodies_in_frame(&mut result.bodies, inverse);
    for joint in &mut result.joints {
        joint.pivot = inverse.transform_point3(joint.pivot);
        joint.axis = inverse.transform_vector3(joint.axis).normalize_or_zero();
    }
    Some(result)
}

/// Absolute output orientation used by native hinge setup (525830 / 4808a0).
/// An angular axis is a pseudovector: reflection reverses its X/Y, not its Z.
/// Build the reference in the original coordinates, then reflect the rotation.
pub fn native_hinge_rotation(axis: Vec3, angle: f32) -> Quat {
    let axis = Vec3::new(-axis.x, -axis.y, axis.z).normalize();
    let seed = if axis.x.abs() < 0.57 {
        Vec3::X
    } else {
        Vec3::Y
    };
    let y = axis.cross(seed).normalize();
    let z = -axis;
    let zero = Mat3::from_cols(y.cross(z), y, z);
    let reflection = Mat3::from_diagonal(Vec3::new(1.0, 1.0, -1.0));
    Quat::from_mat3(&(reflection * Mat3::from_axis_angle(axis, angle) * zero * reflection))
        .normalize()
}

pub fn body_frames(rig: &Rig) -> Result<Vec<Mat4>, String> {
    let mut incoming = vec![None; rig.bodies.len()];
    for (i, j) in rig.joints.iter().enumerate() {
        if j.child >= incoming.len() || j.parent >= incoming.len() {
            return Err("Joint body out of range".into());
        }
        if incoming[j.child].replace(i).is_some() {
            return Err("Multiple parent joints for rigid body".into());
        }
    }
    let mut frames = rig.bodies.iter().map(|b| b.frame).collect::<Vec<_>>();
    let mut state = vec![0; frames.len()];
    fn solve(
        i: usize,
        rig: &Rig,
        incoming: &[Option<usize>],
        frames: &mut [Mat4],
        state: &mut [u8],
    ) -> Result<(), String> {
        if state[i] == 2 {
            return Ok(());
        }
        if state[i] == 1 {
            return Err("Cyclic initial-pose joints".into());
        }
        state[i] = 1;
        if let Some(index) = incoming[i] {
            let j = &rig.joints[index];
            solve(j.parent, rig, incoming, frames, state)?;
            let inherited = frames[j.parent] * rig.bodies[j.parent].frame.inverse();
            let delta = if j.kind == ActuatorKind::Burst {
                let anchor = inherited.transform_point3(j.anchor);
                let axis = inherited.transform_vector3(j.axis).normalize();
                // Native setup replaces the output's absolute orientation; it
                // does not add a universal quarter-turn to the exported mesh.
                let rotation = native_hinge_rotation(axis, j.start) * j.child_rotation.inverse();
                Mat4::from_translation(anchor)
                    * Mat4::from_quat(rotation)
                    * Mat4::from_translation(-j.anchor)
            } else {
                inherited
            };
            frames[i] = delta * rig.bodies[i].frame;
        }
        if !frames[i].is_finite() {
            return Err("Nonfinite initial pose".into());
        }
        state[i] = 2;
        Ok(())
    }
    for i in 0..frames.len() {
        solve(i, rig, &incoming, &mut frames, &mut state)?;
    }
    Ok(frames)
}

pub fn freeze(
    robot: &RobotDefinition,
    models: &[ResolvedComponent],
    definitions: &[ComponentDefinition],
) -> Result<FrozenRobot, String> {
    if definitions.len() != models.len() {
        return Err("Pose component/definition count mismatch".into());
    }
    let mut rebound = Vec::with_capacity(models.len());
    let models: Vec<_> = models
        .iter()
        .enumerate()
        .map(|(i, source)| {
            let bound = if i == 0 {
                None
            } else {
                bind_model(&source.model, &definitions[i])
            };
            rebound.push(bound.is_some());
            ResolvedComponent {
                key: source.key.clone(),
                model: bound.map(Arc::new).unwrap_or_else(|| source.model.clone()),
                hidden: source.hidden,
            }
        })
        .collect();
    let models = &models;
    let mut assembly = assembly::assemble(robot, models)?;
    let rig = mechanisms::build(robot, &assembly, models, definitions)?;
    let frames = body_frames(&rig)?;
    let deltas: Vec<_> = frames
        .iter()
        .zip(&rig.bodies)
        .map(|(frame, body)| *frame * body.frame.inverse())
        .collect();
    let mut posed = Vec::with_capacity(models.len());
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    for (i, source) in models.iter().enumerate() {
        let authored = assembly.parts[i].transform;
        let transform = deltas[rig.component_bodies[i]] * authored;
        let inverse = transform.inverse();
        let mut model = (*source.model).clone();
        let mut changed = rebound[i];
        for (o, &body) in model.objects.iter_mut().zip(&rig.objects[i]) {
            let relative = inverse * deltas[body] * authored;
            if !relative.abs_diff_eq(Mat4::IDENTITY, 0.00001) {
                o.transform = relative * o.transform;
                changed = true;
            }
        }
        for (a, &body) in model.attachments.iter_mut().zip(&rig.attachment_bodies[i]) {
            let relative = inverse * deltas[body] * authored;
            if !relative.abs_diff_eq(Mat4::IDENTITY, 0.00001) {
                a.transform = relative * a.transform;
                changed = true;
            }
        }
        // Collision proxies are model objects and receive exactly the same delta
        // as their owning body. Body frames are not re-used as a second node transform.
        let key = if changed {
            let mut hash = 0xcbf29ce484222325_u64;
            for object in &model.objects {
                for word in object.transform.to_cols_array() {
                    hash = (hash ^ u64::from(word.to_bits())).wrapping_mul(0x100000001b3);
                }
            }
            for attachment in &model.attachments {
                for word in attachment.transform.to_cols_array() {
                    hash = (hash ^ u64::from(word.to_bits())).wrapping_mul(0x100000001b3);
                }
            }
            format!("{}#pose-{hash:016x}", source.key)
        } else {
            source.key.clone()
        };
        if !source.hidden {
            for p in model
                .objects
                .iter()
                .filter(|o| o.visible)
                .flat_map(|o| o.positions())
            {
                let p = transform.transform_point3(p);
                min = min.min(p);
                max = max.max(p);
            }
        }
        assembly.parts[i].transform = transform;
        assembly.parts[i].model = key.clone();
        posed.push(ResolvedComponent {
            key,
            model: if changed {
                Arc::new(model)
            } else {
                source.model.clone()
            },
            hidden: source.hidden,
        });
    }
    assembly.bounds = (min, max);
    Ok(FrozenRobot {
        assembly,
        models: posed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn absolute_hinge_orientation_matches_original_executable_measurements() {
        // Original 525830 + 4808a0, angle 5.03237, WXYZ float bits. These
        // fixtures include both motor handednesses and all three world axes.
        let fixtures = [
            (Vec3::X, [0x3f32b51f, 0x3de6b432, 0xbf32b51e, 0xbde6b432]),
            (-Vec3::X, [0x3f32b51f, 0xbde6b432, 0x3f32b51e, 0xbde6b432]),
            (Vec3::Y, [0x3ed3f28f, 0x3ed3f28f, 0x3f12c1bf, 0xbf12c1bf]),
            (-Vec3::Y, [0xbed3f28f, 0x3ed3f28f, 0x3f12c1bf, 0x3f12c1bf]),
            (Vec3::Z, [0x80000000, 0x3f15de98, 0x3f4f8ba5, 0x00000000]),
            (-Vec3::Z, [0x3f15de98, 0x00000000, 0x00000000, 0xbf4f8ba5]),
            (
                Vec3::new(0.8, 0.0, 0.6),
                [0xbee20c91, 0xbe11e8f3, 0x3f620c91, 0x3d91e8f3],
            ),
            (
                Vec3::new(0.4, 0.8, 0.4472136),
                [0x3e16a1cd, 0x3f13ff57, 0x3f1fbf32, 0xbf01361c],
            ),
        ];
        for (native_axis, bits) in fixtures {
            let [w, x, y, z] = bits.map(f32::from_bits);
            let expected = Quat::from_xyzw(-x, -y, z, w).normalize();
            let axis = Vec3::new(-native_axis.x, -native_axis.y, native_axis.z);
            let actual = native_hinge_rotation(axis, 5.03237);
            for v in [Vec3::X, Vec3::Y, Vec3::Z] {
                assert!(
                    (actual * v).abs_diff_eq(expected * v, 0.00001),
                    "axis {axis:?}"
                );
            }
        }
    }

    #[test]
    fn component_geometry_uses_master_frame_without_changing_connectors_or_source_data() {
        let frame = Mat4::from_rotation_x(0.7) * Mat4::from_translation(Vec3::new(0.2, 0.3, 0.4));
        let mut model = Model::default();
        model.bodies.push(Body {
            name: "master".into(),
            transform: frame,
            mass: 1.0,
            geometry: CollisionGeometry::Convex,
            proxy: "proxy".into(),
            display_proxy: String::new(),
            use_display_proxy: false,
            disabled: false,
            initially_sleeping: false,
            friction: 0.7,
            restitution: 0.0,
            children: vec![],
        });
        for (name, local) in [
            ("master", Mat4::IDENTITY),
            ("proxy", Mat4::from_translation(Vec3::X)),
        ] {
            model.objects.push(Object {
                name: name.into(),
                visible: true,
                parent: None,
                transform: frame * local,
                vertices: vec![Vec3::ZERO, Vec3::X, Vec3::Y],
                faces: vec![],
                material: 0,
                uv_channels: Default::default(),
            });
        }
        model.attachments.push(Attachment {
            name: "input".into(),
            transform: Mat4::from_translation(Vec3::Z),
            properties: Default::default(),
        });
        let posed = bind_model(&model, &ComponentDefinition::parse("master=master")).unwrap();
        assert!(
            posed.objects[0]
                .transform
                .abs_diff_eq(Mat4::IDENTITY, 0.00001)
        );
        assert!(
            posed.objects[1]
                .transform
                .abs_diff_eq(Mat4::from_translation(Vec3::X), 0.00001)
        );
        assert!(
            posed.attachments[0]
                .transform
                .abs_diff_eq(model.attachments[0].transform, 0.00001)
        );
        assert!(
            posed.bodies[0]
                .transform
                .abs_diff_eq(Mat4::IDENTITY, 0.00001)
        );
        assert_eq!(model.objects[0].transform, frame);
        assert!(bind_model(&posed, &ComponentDefinition::parse("master=master")).is_none());

        // A stretched export must keep both its actual dimensions and sockets.
        let mut stretched = model.clone();
        stretched.bodies[0].transform = Mat4::from_scale(Vec3::new(1.0, 1.7704918, 1.0));
        stretched.objects[0].transform = stretched.bodies[0].transform;
        assert!(bind_model(&stretched, &ComponentDefinition::parse("master=master")).is_none());
    }

    #[test]
    fn initial_pose_follows_output_body_and_nested_joints_independent_of_list_order() {
        use arena_core::mechanism::{RigBody, RigJoint};
        let joint = |parent, child, anchor| RigJoint {
            component: 1,
            parent,
            child,
            anchor,
            axis: Vec3::Z,
            child_rotation: Quat::IDENTITY,
            kind: ActuatorKind::Burst,
            motion: JointKind::Hinge {
                limited: false,
                friction: 0.0,
                limits: [-3.0, 3.0],
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
        };
        let rig = Rig {
            bodies: (0..4)
                .map(|i| RigBody {
                    name: i.to_string(),
                    frame: Mat4::IDENTITY,
                    mass: 1.0,
                })
                .collect(),
            joints: vec![joint(1, 2, Vec3::X), joint(0, 1, Vec3::ZERO)],
            ..Default::default()
        };
        let frames = body_frames(&rig).unwrap();
        assert!(frames[1].is_finite());
        assert!(
            frames[2]
                .transform_point3(Vec3::X)
                .abs_diff_eq(frames[1].transform_point3(Vec3::X), 0.00001)
        );
        assert_eq!(frames[0], Mat4::IDENTITY);
        assert_eq!(frames[3], Mat4::IDENTITY);
    }
}
