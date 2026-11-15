//! Shared rendering and Rapier adapters for a native articulated robot.
use super::*;
use crate::editor::catalog::Catalog;
use arena_core::{Model, robot::RobotDefinition};
use arena_runtime::{
    content::{FileTextures, TextureBytes, TextureSource},
    render::{self, RenderSettings, TextureCache},
};
use ra2_formats::{
    assembly::{self, ResolvedComponent},
    components::{ComponentDefinition, normalize},
};
use std::sync::Arc;
struct Paint<'a>(&'a [u8]);
impl TextureSource for Paint<'_> {
    fn read(&self, _: &str) -> Result<TextureBytes, String> {
        let mut bytes = self.0.to_vec();
        if bytes.len() < 18 {
            return Err("Invalid chassis texture".into());
        }
        for p in bytes[18..].as_chunks_mut::<4>().0 {
            p[3] = 255;
        }
        Ok(TextureBytes {
            key: "garage#chassis".into(),
            extension: "tga".into(),
            bytes,
        })
    }
}
#[allow(clippy::too_many_arguments)]
pub(crate) fn robot(
    commands: &mut Commands,
    garage: &mut RobotInstance,
    robot: &RobotDefinition,
    catalog: &Catalog,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    images: &mut Assets<Image>,
) -> Result<(), String> {
    let mut models = vec![ResolvedComponent {
        key: "@chassis".into(),
        model: Arc::new(robot.chassis.model.clone().ok_or("Build a chassis first")?),
        hidden: false,
    }];
    let mut definitions = vec![ComponentDefinition::parse("")];
    for c in robot.components.iter().skip(1) {
        let a = catalog
            .components
            .get(&normalize(&c.definition))
            .ok_or_else(|| format!("Missing component {}", c.definition))?;
        models.push(ResolvedComponent {
            key: a.key.clone(),
            model: ra2_formats::pose::bind_model(&a.model, &a.definition)
                .map(Arc::new)
                .unwrap_or_else(|| a.model.clone()),
            hidden: c.class == "SmartZone",
        });
        definitions.push(a.definition.clone());
    }
    let assembly = assembly::assemble(robot, &models)?;
    let mut rig = ra2_formats::mechanisms::build(robot, &assembly, &models, &definitions)?;
    let mut placement = garage
        .placement
        .ok_or("Robot ground placement was not set")?;
    placement.translation.y += -assembly.bounds.0.y + 0.12;
    let placement = placement.to_matrix();
    let rest = ra2_formats::pose::body_frames(&rig)?;
    for joint in &mut rig.joints {
        if joint.kind == ActuatorKind::Burst {
            // Native editor end pose is start_angle + stop_angle. Havok's
            // internal limit angle has the opposite sign to Rapier's relative
            // rotation about this reflected axis; keep the BOT's signed stroke.
            joint.start = 0.0;
            joint.limits = Some([joint.start.min(joint.stop), joint.start.max(joint.stop)]);
        }
    }

    garage.damage = damage::State::default();
    garage.damage.health = ra2_formats::health::build(robot, &definitions)?;
    garage.damage.shell = Some(super::surface::Shell::new(&models[0].model)?);
    garage.damage.supplies = definitions
        .iter()
        .enumerate()
        .map(|(i, d)| {
            let number = |key| d.value(key).parse::<f32>().unwrap_or(0.).max(0.);
            damage::Supply {
                component: i,
                electricity: number("electotal"),
                electric_rate: number("elecmaxinoutrate"),
                air: number("airtotal"),
                air_rate: number("airmaxinoutrate"),
            }
        })
        .collect();
    let mut cache = TextureCache::default();
    for (body_index, body) in rig.bodies.iter().enumerate() {
        let mut shapes = vec![];
        let mut parts = damage::BodyParts::default();
        let inverse = body.frame.inverse();
        let mut visual_models = vec![];
        let mut friction = 0.3_f32;
        let mut wheel_contact = false;
        let mut treads = tyres::Tyres::default();
        for (i, asset) in models.iter().enumerate().filter(|(_, a)| !a.hidden) {
            if robot.components[i].class == "Wheel" && rig.objects[i].contains(&body_index) {
                wheel_contact = true;
                if let Some((_, center, rotation, shape)) =
                    arena_runtime::robot::wheel_collider(&asset.model)
                    && let Some(cylinder) = shape.raw.as_round_cylinder()
                {
                    parts
                        .wheels
                        .push((i, cylinder.inner_shape.radius + cylinder.border_radius));
                    // Passive/caster wheels keep Rapier friction. In particular,
                    // a caster may share the chassis body: disabling its whole
                    // compound material would also remove hull-floor friction.
                    if rig
                        .joints
                        .iter()
                        .any(|j| j.child == body_index && j.kind == ActuatorKind::Spin)
                    {
                        let ground_reach = asset
                            .model
                            .object(definitions[i].value("contact"))
                            .and_then(|o| {
                                o.positions()
                                    .map(|p| {
                                        let d = rotation.inverse() * (p - center);
                                        Vec3::new(d.x, 0., d.z).length()
                                    })
                                    .reduce(f32::max)
                            })
                            .unwrap_or(
                                cylinder.inner_shape.radius + cylinder.border_radius + 0.025,
                            );
                        treads.0.push(tyres::Tread {
                            component: i,
                            ground_reach,
                            center: (inverse * assembly.parts[i].transform)
                                .transform_point3(center),
                            axle: (inverse * assembly.parts[i].transform)
                                .transform_vector3(rotation * Vec3::Y)
                                .normalize(),
                            grip: definitions[i]
                                .value("grip")
                                .parse::<f32>()
                                .unwrap_or(1.)
                                .max(0.),
                            resistance: definitions[i]
                                .value("resistance")
                                .parse::<f32>()
                                .unwrap_or(0.2)
                                .max(0.),
                        });
                    }
                }
                friction = definitions[i]
                    .value("grip")
                    .parse::<f32>()
                    .unwrap_or(1.0)
                    .max(0.0);
            }
            let mass = if i == 0 {
                if body_index == 0 {
                    assembly.parts[0].mass
                } else {
                    0.
                }
            } else {
                asset
                    .model
                    .all_bodies()
                    .iter()
                    .filter(|b| !b.disabled)
                    .filter(|b| {
                        asset
                            .model
                            .objects
                            .iter()
                            .position(|o| {
                                o.name.eq_ignore_ascii_case(&b.proxy)
                                    || o.name.eq_ignore_ascii_case(&b.name)
                            })
                            .map_or(rig.component_bodies[i], |n| rig.objects[i][n])
                            == body_index
                    })
                    .map(|b| b.mass)
                    .sum()
            };
            if mass > 0. {
                parts.masses.push((i, mass));
            }
            let mut model = (*asset.model).clone();
            model.objects = model
                .objects
                .into_iter()
                .enumerate()
                .filter_map(|(j, mut o)| {
                    if rig.objects[i][j] != body_index {
                        return None;
                    }
                    o.transform = inverse * assembly.parts[i].transform * o.transform;
                    Some(o)
                })
                .collect();
            let colliders =
                if robot.components[i].class == "Wheel" && garage.tuning.smooth_wheel_contacts {
                    arena_runtime::robot::wheel_collider(&asset.model)
                        .map(|(name, position, rotation, shape)| {
                            vec![(
                                name,
                                Mat4::from_rotation_translation(rotation, position),
                                shape,
                            )]
                        })
                        .unwrap_or_else(|| {
                            arena_runtime::robot::collision_parts(&asset.model)
                                .into_iter()
                                .map(|(n, c)| (n, Mat4::IDENTITY, c))
                                .collect()
                        })
                } else {
                    arena_runtime::robot::collision_parts(&asset.model)
                        .into_iter()
                        .map(|(n, c)| (n, Mat4::IDENTITY, c))
                        .collect()
                };
            for (name, local, collider) in colliders {
                if let Some(object) = asset
                    .model
                    .objects
                    .iter()
                    .position(|o| o.name.eq_ignore_ascii_case(&name))
                    && rig.objects[i][object] == body_index
                {
                    let (_, rotation, translation) =
                        (inverse * assembly.parts[i].transform * local)
                            .to_scale_rotation_translation();
                    parts.solids.push(damage::Solid {
                        component: i,
                        tool: definitions[i].value("base").eq_ignore_ascii_case("weapon"),
                        profile: ra2_formats::health::contact_profile(
                            &definitions[i],
                            asset
                                .model
                                .materials
                                .get(&asset.model.objects[object].material)
                                .map_or("", |m| m.name.as_str()),
                        ),
                        position: translation,
                        rotation,
                        collider: collider.clone(),
                    });
                    shapes.push((translation, rotation, collider));
                }
            }
            if model.objects.iter().any(|o| o.visible) {
                visual_models.push((i, model));
            }
        }
        // Keep attribution consistent with the mass split used by the rig,
        // including virtual steering knuckles that have no display/proxy object.
        let attributed: f32 = parts.masses.iter().map(|(_, m)| m).sum();
        if attributed > 0. {
            for (_, mass) in &mut parts.masses {
                *mass *= body.mass.max(0.01) / attributed;
            }
        } else {
            let owner = rig
                .joints
                .iter()
                .find(|j| j.child == body_index)
                .map_or(0, |j| j.component);
            parts.masses.push((owner, body.mass.max(0.01)));
        }
        // Zero-volume axle bodies are valid; attached wheels add geometry to the same group.
        let collider = if shapes.is_empty() {
            Collider::ball(0.005)
        } else {
            Collider::compound(shapes)
        };
        let transform = Transform::from_matrix(
            placement * Mat4::from_rotation_y(robot.heading) * rest[body_index],
        );
        let mass_properties = if let Some(hub) = rig
            .joints
            .iter()
            .find(|j| j.kind == ActuatorKind::Spin && j.parent == body_index)
            .filter(|_| {
                rig.joints
                    .iter()
                    .any(|j| j.kind == ActuatorKind::Steering && j.child == body_index)
            }) {
            let mut radius_squared = 0.0001_f32;
            for (i, asset) in models.iter().enumerate() {
                for (n, object) in asset.model.objects.iter().enumerate() {
                    if object.visible && rig.objects[i][n] == hub.child {
                        for p in object.positions() {
                            radius_squared = radius_squared.max(
                                (assembly.parts[i].transform.transform_point3(p) - hub.anchor)
                                    .length_squared(),
                            );
                        }
                    }
                }
            }
            ColliderMassProperties::MassProperties(MassProperties {
                mass: body.mass.max(0.01),
                principal_inertia: Vec3::splat(body.mass.max(0.01) * radius_squared * 0.4),
                ..default()
            })
        } else {
            ColliderMassProperties::MassProperties(super::mass::properties(
                &parts.solids,
                &parts.masses,
            ))
        };
        let entity = commands
            .spawn((
                Name::new(body.name.clone()),
                RobotBody,
                RigidBody::Dynamic,
                collider,
                mass_properties,
                transform,
                Velocity::zero(),
                Sleeping::disabled(),
                Ccd::enabled(),
                AdditionalSolverIterations(16),
                Friction {
                    coefficient: if treads.0.is_empty() { friction } else { 0. },
                    combine_rule: if !treads.0.is_empty() {
                        CoefficientCombineRule::Min
                    } else if wheel_contact {
                        CoefficientCombineRule::Max
                    } else {
                        CoefficientCombineRule::Average
                    },
                },
                Restitution::coefficient(0.03),
                Damping {
                    linear_damping: 0.03,
                    angular_damping: 0.1,
                },
                CollisionGroups::new(garage.group, Group::ALL & !garage.group),
                RapierContextEntityLink(garage.context),
            ))
            .id();
        // Track ownership immediately so a failed texture/model load is cleaned up.
        garage.bodies.push(entity);
        commands.entity(entity).insert((
            ReadWorldMassProperties::default(),
            treads,
            RobotOwner(garage.id),
        ));
        for (i, model) in visual_models {
            let model = if i == 0 {
                garage
                    .damage
                    .shell
                    .as_ref()
                    .map_or(model.clone(), |s| s.model(&model))
            } else {
                model
            };
            let file = if i > 0 {
                Some(FileTextures::new(
                    catalog.components[&models[i].key].source.parent().unwrap(),
                ))
            } else {
                None
            };
            let paint = Paint(robot.chassis.texture.as_deref().unwrap_or(&[]));
            let source: &dyn TextureSource = if let Some(file) = &file { file } else { &paint };
            let settings = RenderSettings {
                roughness: if i == 0 {
                    (1.0 - robot.chassis.material_parameters[4] / 120.0).clamp(0.08, 0.85)
                } else {
                    0.8
                },
                ..Default::default()
            };
            let prepared = render::prepare_model(
                &model, source, &settings, &mut cache, meshes, materials, images,
            )?;
            if i == 0
                && let Some(shell) = &mut garage.damage.shell
            {
                shell.meshes = prepared.mesh_handles().cloned().collect();
            }
            let visual = prepared.spawn_on_layer(
                commands,
                &format!("Robot component {i}"),
                Transform::default(),
                garage.layer,
            );
            commands.entity(visual).insert(ChildOf(entity));
            parts.visuals.push((i, visual));
        }
        garage.damage.bodies.push(parts);
    }
    for (i, j) in rig.joints.iter().enumerate() {
        let parent = rig.bodies[j.parent]
            .frame
            .inverse()
            .transform_point3(j.anchor);
        let child = rig.bodies[j.child]
            .frame
            .inverse()
            .transform_point3(j.anchor);
        let axis = if j.axis.length_squared() > 0.5 {
            j.axis
        } else {
            Vec3::X
        };
        let parent_delta = rest[j.parent] * rig.bodies[j.parent].frame.inverse();
        let posed_axis = parent_delta.transform_vector3(axis).normalize();
        let world_basis = Quat::from_rotation_arc(Vec3::X, posed_axis);
        let parent_rotation = rest[j.parent].to_scale_rotation_translation().1;
        let child_rotation = rest[j.child].to_scale_rotation_translation().1;
        let linear = matches!(j.kind, ActuatorKind::Piston | ActuatorKind::BurstPiston);
        let mut joint = GenericJointBuilder::new(if linear {
            JointAxesMask::LOCKED_PRISMATIC_AXES
        } else if matches!(
            j.motion,
            arena_core::JointKind::Ball | arena_core::JointKind::Ragdoll { .. }
        ) {
            JointAxesMask::LOCKED_SPHERICAL_AXES
        } else {
            JointAxesMask::LOCKED_REVOLUTE_AXES
        })
        // Both frames must coincide in the configured pose. Reusing an
        // authored axis for both rotated bodies lets locked axes flip the bot.
        .local_basis1(parent_rotation.inverse() * world_basis)
        .local_basis2(child_rotation.inverse() * world_basis)
        .local_anchor1(parent)
        .local_anchor2(child)
        .contacts_enabled(false)
        .build();
        let motor_axis = if linear {
            JointAxis::LinX
        } else {
            JointAxis::AngX
        };
        if let Some(limits) = j.limits {
            joint.set_limits(motor_axis, limits);
        }
        joint.set_motor_model(
            motor_axis,
            if matches!(
                j.kind,
                ActuatorKind::Spin
                    | ActuatorKind::Steering
                    | ActuatorKind::Burst
                    | ActuatorKind::BurstPiston
            ) {
                MotorModel::AccelerationBased
            } else {
                MotorModel::ForceBased
            },
        );
        joint.set_motor_max_force(motor_axis, 0.0);
        commands.entity(garage.bodies[j.child]).insert((
            ImpulseJoint::new(garage.bodies[j.parent], joint),
            drive::Motor(i),
        ));
    }
    info!(
        "Robot runtime: {} components, {} bodies, {} joints, battery {}, air {}",
        robot.components.len(),
        rig.bodies.len(),
        rig.joints.len(),
        rig.electrical.capacity,
        rig.pneumatic.capacity
    );
    garage.states = rig
        .joints
        .iter()
        .map(|_| ActuatorState::default())
        .collect();
    garage.commands = vec![Command::default(); robot.components.len()];
    garage.controls = Controls::default();
    garage.document = Some(robot.clone());
    garage.rig = Some(rig);
    Ok(())
}
pub(crate) fn read(catalog: &Catalog, path: &str) -> Result<(Model, FileTextures), String> {
    let file = catalog
        .index
        .get(&normalize(path))
        .ok_or_else(|| format!("Missing original asset {path}"))?;
    let model = ra2_formats::ModelImporters::with_ra2()
        .decode(&std::fs::read(file).map_err(|e| e.to_string())?)?
        .model;
    Ok((model, FileTextures::new(file.parent().unwrap())))
}
