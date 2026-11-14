//! Attachment compatibility and geometry, shared by the placement preview and commit.
use super::{
    Editor,
    catalog::Catalog,
    viewport::{Pick, View},
};
use arena_core::{Attachment, Mat4, Model, Quat, Vec3, robot::*};
use ra2_formats::components::normalize;
use std::{collections::BTreeMap, sync::Arc};
pub fn compatible(a: &str, b: &str) -> bool {
    let (Some((a, ag)), Some((b, bg))) = (a.rsplit_once('_'), b.rsplit_once('_')) else {
        return false;
    };
    a.eq_ignore_ascii_case(b)
        && ((ag.eq_ignore_ascii_case("M") && bg.eq_ignore_ascii_case("F"))
            || (ag.eq_ignore_ascii_case("F") && bg.eq_ignore_ascii_case("M")))
}

pub fn socket_id(socket: &Attachment) -> Option<u32> {
    socket.properties.get("id")?.parse().ok()
}

/// Prefer a chassis mount, then a receiving socket. GMF record order is not ID order.
pub fn default_socket(model: &Model) -> u32 {
    model
        .attachments
        .iter()
        .filter_map(|socket| {
            let id = socket_id(socket)?;
            let kind = socket
                .properties
                .get("attach")
                .map(String::as_str)
                .unwrap_or("");
            let priority = if kind.eq_ignore_ascii_case("Base_F") {
                0
            } else if kind
                .rsplit_once('_')
                .is_some_and(|(_, gender)| gender.eq_ignore_ascii_case("F"))
            {
                1
            } else {
                2
            };
            Some((priority, id))
        })
        .min()
        .map(|(_, id)| id)
        .unwrap_or(1)
}

/// Only free, matching connectors participate in placement picking.
pub fn available_target(
    editor: &Editor,
    own_type: &str,
    component: usize,
    socket: &Attachment,
) -> bool {
    let Some(id) = socket_id(socket) else {
        return false;
    };
    socket
        .properties
        .get("attach")
        .is_some_and(|kind| compatible(own_type, kind))
        && editor
            .document
            .robot
            .components
            .get(component)
            .is_some_and(|c| c.own_mount != id)
        && !editor
            .document
            .robot
            .components
            .iter()
            .any(|c| c.parent == Some(component) && c.parent_mount == id)
}
pub fn pending_base_mount(editor: &Editor, catalog: &Catalog) -> bool {
    editor
        .pending
        .as_ref()
        .and_then(|key| catalog.components.get(key))
        .and_then(|asset| selected_socket(&asset.model, editor.selected_socket))
        .and_then(|s| s.properties.get("attach"))
        .is_some_and(|kind| kind.eq_ignore_ascii_case("Base_F"))
}

pub fn selected_socket(model: &Model, id: u32) -> Option<&Attachment> {
    model
        .attachments
        .iter()
        .find(|socket| socket_id(socket) == Some(id))
        .or_else(|| (model.attachments.len() == 1).then(|| &model.attachments[0]))
}
/// A candidate is frozen through the same solver as the displayed assembly.
/// The result includes articulated geometry and the final connector frame.
pub fn pose(editor: &Editor, catalog: &Catalog, pick: &Pick, view: &View) -> Result<Mat4, String> {
    let component = candidate_on_view(editor, catalog, pick, view)?;
    let (pose, model) = freeze_candidate(editor, catalog, component.clone())?;
    validate_on_view(editor, catalog, &component, pose, &model, view)?;
    Ok(pose)
}
pub fn update_preview(editor: &mut Editor, catalog: &Catalog, view: &View) {
    let Some(pick) = editor.ghost_pick.clone() else {
        return;
    };
    let strict = candidate_on_view(editor, catalog, &pick, view);
    let candidate_valid = strict.is_ok();
    let candidate = strict.or_else(|_| {
        candidate_impl(
            editor,
            catalog,
            &pick,
            view.assembly.as_ref(),
            Some(&view.models),
            false,
        )
    });
    if let Ok(component) = candidate
        && let Ok((pose, model)) = freeze_candidate(editor, catalog, component.clone())
    {
        editor.ghost_valid = candidate_valid
            && validate_on_view(editor, catalog, &component, pose, &model, view).is_ok();
        editor.ghost_pose = Some(pose);
        editor.ghost_model = Some(model);
    } else {
        editor.ghost_valid = false;
    }
}
fn freeze_candidate(
    editor: &Editor,
    catalog: &Catalog,
    component: RobotComponent,
) -> Result<(Mat4, ra2_formats::assembly::ResolvedComponent), String> {
    let mut robot = editor.document.robot.clone();
    robot.components.push(component);
    let mut frozen = catalog.freeze(&robot)?;
    Ok((
        frozen
            .assembly
            .parts
            .last()
            .ok_or("Missing placement transform")?
            .transform,
        frozen.models.pop().ok_or("Missing placement model")?,
    ))
}
fn validate_on_view(
    editor: &Editor,
    catalog: &Catalog,
    component: &RobotComponent,
    pose: Mat4,
    model: &ra2_formats::assembly::ResolvedComponent,
    view: &View,
) -> Result<(), String> {
    let mut collisions = view.collisions.clone();
    let key = normalize(&component.definition);
    let shapes = if model.key == key {
        catalog.collisions.get(&key).cloned()
    } else {
        None
    }
    .unwrap_or_else(|| arena_runtime::robot::collision_parts(&model.model));
    collisions.insert(key, shapes);
    validate_using(
        editor,
        catalog,
        component,
        pose,
        view.assembly.as_ref(),
        &collisions,
    )
}
#[cfg(test)]
pub fn candidate(
    editor: &Editor,
    catalog: &Catalog,
    pick: &Pick,
    assembly: Option<&RobotAssembly>,
) -> Result<RobotComponent, String> {
    candidate_impl(editor, catalog, pick, assembly, None, true)
}
pub fn candidate_on_view(
    editor: &Editor,
    catalog: &Catalog,
    pick: &Pick,
    view: &View,
) -> Result<RobotComponent, String> {
    candidate_impl(
        editor,
        catalog,
        pick,
        view.assembly.as_ref(),
        Some(&view.models),
        true,
    )
}
fn candidate_impl(
    editor: &Editor,
    catalog: &Catalog,
    pick: &Pick,
    assembly: Option<&RobotAssembly>,
    posed_models: Option<&BTreeMap<String, Arc<Model>>>,
    strict: bool,
) -> Result<RobotComponent, String> {
    if strict && pick.face == usize::MAX {
        return Err("Point at a chassis panel or matching attachment".into());
    }
    let key = editor
        .pending
        .clone()
        .ok_or("Select a component and click Attach")?;
    let asset = catalog
        .components
        .get(&key)
        .ok_or("Missing component template")?;
    let mut settings = BTreeMap::new();
    let class = asset.definition.value("base").to_string();
    if class == "BurstMotor" {
        settings.insert("start_angle".into(), "0".into());
        settings.insert("stop_angle".into(), "2.4".into());
    }
    if class == "SmartZone" {
        settings.insert("zone".into(), "Zone".into());
    }
    let own = selected_socket(&asset.model, editor.selected_socket)
        .ok_or("Select an attachment point in the component preview")?;
    let own_mount = socket_id(own).ok_or("Attachment has no ID")?;
    let own_type = own
        .properties
        .get("attach")
        .map(String::as_str)
        .unwrap_or("");
    let (parent_mount, mounting) = if pick.component == 0 {
        if strict && !own_type.eq_ignore_ascii_case("Base_F") {
            return Err("This attachment requires a matching component socket".into());
        }
        let normal = if pick.normal.y.abs() > 0.7 {
            Vec3::Y
        } else {
            pick.normal
        };
        let mut position = pick.position;
        if normal == Vec3::Y {
            position.y = 0.0;
        }
        (
            0,
            MountPlacement {
                position,
                rotation: Quat::from_rotation_arc(Vec3::Z, normal),
                twist: std::f32::consts::PI - editor.attach_angle,
                offset: editor.attach_offset,
            },
        )
    } else {
        if strict && pick.attachment.is_none() {
            return Err("Point at the baseplate or an explicit matching attachment".into());
        }
        let assembly = assembly.ok_or("No robot assembly")?;
        let part = &assembly.parts[pick.component];
        let parent = catalog
            .components
            .get(&normalize(
                &editor.document.robot.components[pick.component].definition,
            ))
            .ok_or("Missing parent component")?;
        let model = posed_models
            .and_then(|models| models.get(&part.model))
            .unwrap_or(&parent.model);
        let socket = model
            .attachments
            .iter()
            .filter(|a| {
                let compatible = a
                    .properties
                    .get("attach")
                    .is_some_and(|s| compatible(own_type, s));
                let id = a.properties.get("id").and_then(|s| s.parse::<u32>().ok());
                pick.attachment.is_none_or(|selected| id == Some(selected))
                    && (!strict
                        || (compatible
                            && id
                                != Some(
                                    editor.document.robot.components[pick.component].own_mount,
                                )
                            && !editor.document.robot.components.iter().any(|p| {
                                p.parent == Some(pick.component) && Some(p.parent_mount) == id
                            })))
            })
            .filter_map(|a| {
                Some((
                    a.properties.get("id")?.parse::<u32>().ok()?,
                    part.transform
                        .transform_point3(a.transform.transform_point3(Vec3::ZERO)),
                ))
            })
            .min_by(|a, b| {
                a.1.distance_squared(pick.position)
                    .total_cmp(&b.1.distance_squared(pick.position))
            })
            .ok_or("Parent has no available attachment points")?;
        (
            socket.0,
            MountPlacement {
                twist: std::f32::consts::PI - editor.attach_angle,
                offset: editor.attach_offset,
                ..Default::default()
            },
        )
    };
    Ok(RobotComponent {
        parent: Some(pick.component),
        parent_mount,
        own_mount,
        mounting,
        class,
        definition: asset.reference.clone(),
        settings,
    })
}

/// Reject overlaps of native collision proxies. Socket mates can interlock;
/// source `passthru` proxies are explicitly allowed to cross chassis panels.
#[cfg(test)]
pub fn validate(
    editor: &Editor,
    catalog: &Catalog,
    component: &RobotComponent,
    pose: Mat4,
    assembly: Option<&RobotAssembly>,
) -> Result<(), String> {
    validate_using(
        editor,
        catalog,
        component,
        pose,
        assembly,
        &catalog.collisions,
    )
}
fn validate_using(
    editor: &Editor,
    catalog: &Catalog,
    component: &RobotComponent,
    pose: Mat4,
    assembly: Option<&RobotAssembly>,
    collisions: &BTreeMap<String, Vec<(String, bevy_rapier3d::prelude::Collider)>>,
) -> Result<(), String> {
    use bevy_rapier3d::{parry::query, prelude::Collider, rapier::math::Pose};
    let Some(assembly) = assembly else {
        return Ok(());
    };
    let key = normalize(&component.definition);
    let Some(shapes) = collisions.get(&key) else {
        return Ok(());
    };
    let asset = &catalog.components[&key];
    let passthru = asset
        .definition
        .value("passthru")
        .split(',')
        .map(|s| s.trim().to_ascii_lowercase())
        .collect::<Vec<_>>();
    let (_, rotation, translation) = pose.to_scale_rotation_translation();
    let position = Pose::from_parts(translation, rotation);
    let chassis = editor
        .document
        .robot
        .chassis
        .model
        .as_ref()
        .and_then(|m| m.objects.first());
    // A chassis is a hollow shell. A closed trimesh query can interpret the
    // entire interior as solid; test its panels separately instead.
    let chassis_shapes = chassis
        .map(|o| {
            o.faces
                .iter()
                .map(|f| {
                    let [a, b, c] = f
                        .vertices
                        .map(|v| o.transform.transform_point3(o.vertices[v]));
                    ("chassis panel".to_string(), Collider::triangle(a, b, c))
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    for part in &assembly.parts {
        if !part.visible || (part.component > 0 && component.parent == Some(part.component)) {
            continue;
        }
        let targets = if part.component == 0 {
            chassis_shapes.clone()
        } else {
            collisions.get(&part.model).cloned().unwrap_or_default()
        };
        let (_, r, t) = part.transform.to_scale_rotation_translation();
        let other = Pose::from_parts(t, r);
        for (name, shape) in shapes {
            if part.component == 0 && passthru.contains(name) {
                continue;
            }
            for (_, target) in &targets {
                if query::contact(
                    &position,
                    shape.raw.as_ref(),
                    &other,
                    target.raw.as_ref(),
                    0.0,
                )
                .ok()
                .flatten()
                .is_some_and(|c| c.dist < -0.008)
                {
                    return Err(if part.component == 0 {
                        format!("{} intersects a chassis panel", asset.name())
                    } else {
                        format!("Component overlaps part {}", part.component)
                    });
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::catalog::ComponentAsset;
    use std::sync::Arc;

    fn mount(id: u32, kind: &str) -> Attachment {
        Attachment {
            name: format!("point{id}"),
            transform: Mat4::IDENTITY,
            properties: [
                ("id".into(), id.to_string()),
                ("attach".into(), kind.into()),
            ]
            .into(),
        }
    }

    #[test]
    fn connection_types_ignore_case_but_preserve_gender_and_family() {
        assert!(compatible("axle_M", "Axle_F"));
        assert!(compatible("BASE_f", "base_m"));
        assert!(!compatible("Base_F", "Base_F"));
        assert!(!compatible("Axle_M", "Base_F"));
        assert!(!compatible("Base", "Base_F"));
    }

    #[test]
    fn placement_markers_exclude_used_and_incompatible_connectors() {
        let mut editor = Editor::default();
        let mut motor = editor.document.robot.components[0].clone();
        motor.parent = Some(0);
        motor.own_mount = 1;
        editor.document.robot.components.push(motor.clone());
        assert!(!available_target(&editor, "Base_F", 1, &mount(3, "axle_M")));
        assert!(!available_target(&editor, "Base_M", 1, &mount(1, "Base_F")));
        assert!(available_target(&editor, "Axle_F", 1, &mount(3, "axle_M")));
        motor.parent = Some(1);
        motor.parent_mount = 3;
        editor.document.robot.components.push(motor);
        assert!(!available_target(&editor, "Axle_F", 1, &mount(3, "axle_M")));
    }

    #[test]
    fn socket_ids_are_not_gmf_record_indices() {
        let mut model = Model {
            attachments: vec![mount(3, "axle_M"), mount(2, "base_F"), mount(1, "Base_F")],
            ..Default::default()
        };
        assert_eq!(default_socket(&model), 1);
        assert_eq!(socket_id(selected_socket(&model, 2).unwrap()), Some(2));
        assert!(selected_socket(&model, 99).is_none());
        model.attachments = vec![mount(2, "Base_F")];
        assert_eq!(socket_id(selected_socket(&model, 1).unwrap()), Some(2));
    }

    #[test]
    fn lowercase_base_mount_is_accepted_and_selected_id_is_saved() {
        let key = "components/test.txt";
        let asset = ComponentAsset {
            key: key.into(),
            reference: "Components\\test.txt".into(),
            definition: ra2_formats::components::ComponentDefinition::parse("base=SpinMotor"),
            model: Arc::new(Model {
                attachments: vec![mount(2, "base_F"), mount(1, "Base_F"), mount(3, "axle_M")],
                ..Default::default()
            }),
            source: Default::default(),
            preview: String::new(),
        };
        let catalog = Catalog {
            components: [(key.into(), asset)].into(),
            ..Default::default()
        };
        let mut editor = Editor {
            pending: Some(key.into()),
            selected_socket: 2,
            ..Default::default()
        };
        let pick = Pick {
            attachment: None,
            component: 0,
            position: Vec3::ZERO,
            normal: Vec3::Y,
            face: 0,
            uv: arena_core::Vec2::ZERO,
        };
        let part = candidate(&editor, &catalog, &pick, None).unwrap();
        assert_eq!(part.own_mount, 2);
        assert_eq!(part.definition, "Components\\test.txt");
        editor.selected_socket = 3;
        assert!(candidate(&editor, &catalog, &pick, None).is_err());
        editor.selected_socket = 99;
        assert!(candidate(&editor, &catalog, &pick, None).is_err());
    }

    #[test]
    fn collision_proxies_reject_overlap_and_allow_clearance() {
        use bevy_rapier3d::prelude::Collider;
        let key = "components/test.txt";
        let asset = ComponentAsset {
            key: key.into(),
            reference: key.into(),
            definition: ra2_formats::components::ComponentDefinition::parse(""),
            model: Arc::new(Model::default()),
            source: Default::default(),
            preview: String::new(),
        };
        let catalog = Catalog {
            components: [(key.into(), asset)].into(),
            collisions: [(
                key.into(),
                vec![("body".into(), Collider::cuboid(0.2, 0.2, 0.2))],
            )]
            .into(),
            ..Default::default()
        };
        let editor = Editor::default();
        let part = RobotComponent {
            parent: Some(0),
            parent_mount: 0,
            own_mount: 1,
            mounting: Default::default(),
            class: "Component".into(),
            definition: key.into(),
            settings: Default::default(),
        };
        let assembly = RobotAssembly {
            name: String::new(),
            parts: vec![AssemblyPart {
                component: 1,
                model: key.into(),
                transform: Mat4::IDENTITY,
                mass: 1.0,
                visible: true,
            }],
            mass: 1.0,
            bounds: (Vec3::ZERO, Vec3::ONE),
        };
        assert!(validate(&editor, &catalog, &part, Mat4::IDENTITY, Some(&assembly)).is_err());
        assert!(
            validate(
                &editor,
                &catalog,
                &part,
                Mat4::from_translation(Vec3::X),
                Some(&assembly)
            )
            .is_ok()
        );
    }

    #[test]
    #[ignore = "requires a local RA2 installation"]
    fn original_mounts_place_and_roundtrip_in_native_bot() {
        use crate::legacy_ui::assets::LegacyAssets;
        use ra2_formats::assembly::{ResolvedComponent, assemble};
        let root = std::env::var_os("RA2_GAME_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../RobotArena2")
            });
        let catalog = Catalog::load(&root, &mut LegacyAssets::default()).unwrap();
        let mut editor = Editor::default();
        editor.document.robot.chassis.bottom = vec![
            arena_core::Vec2::new(200.0, 200.0),
            arena_core::Vec2::new(800.0, 200.0),
            arena_core::Vec2::new(800.0, 800.0),
            arena_core::Vec2::new(200.0, 800.0),
        ];
        editor.document.robot.chassis.top = editor.document.robot.chassis.bottom.clone();
        arena_core::editor::chassis::rebuild(&mut editor.document.robot.chassis).unwrap();
        let pick = Pick {
            attachment: None,
            component: 0,
            position: Vec3::ZERO,
            normal: Vec3::Y,
            face: 0,
            uv: arena_core::Vec2::ZERO,
        };
        let mut checked = 0;
        for asset in catalog.components.values() {
            for socket in &asset.model.attachments {
                if !socket
                    .properties
                    .get("attach")
                    .is_some_and(|s| s.eq_ignore_ascii_case("Base_F"))
                {
                    continue;
                }
                editor.pending = Some(asset.key.clone());
                editor.selected_socket = socket_id(socket).unwrap();
                let part = candidate(&editor, &catalog, &pick, None)
                    .unwrap_or_else(|e| panic!("{}: {e}", asset.key));
                assert_eq!(part.own_mount, editor.selected_socket);
                checked += 1;
            }
        }
        assert!(checked >= 80, "checked {checked} Base_F mounts");
        let find = |suffix: &str| {
            catalog
                .components
                .values()
                .find(|a| a.key.ends_with(suffix))
                .unwrap()
        };
        let board = catalog
            .components
            .values()
            .find(|a| a.definition.value("base") == "ControlBoard")
            .unwrap();
        let hollow = RobotAssembly {
            name: String::new(),
            parts: vec![AssemblyPart {
                component: 0,
                model: "@chassis".into(),
                transform: Mat4::IDENTITY,
                mass: 1.0,
                visible: true,
            }],
            mass: 1.0,
            bounds: (Vec3::ZERO, Vec3::ONE),
        };
        editor.pending = Some(board.key.clone());
        editor.selected_socket = default_socket(&board.model);
        let board_part = candidate(&editor, &catalog, &pick, None).unwrap();
        let pose = board_part.mounting.matrix()
            * selected_socket(&board.model, board_part.own_mount)
                .unwrap()
                .transform
                .inverse();
        assert!(
            validate(&editor, &catalog, &board_part, pose, Some(&hollow)).is_ok(),
            "the inside of a chassis must remain hollow"
        );
        let through_roof =
            Mat4::from_translation(Vec3::Y * (editor.document.robot.chassis.height - 0.07)) * pose;
        assert!(
            validate(&editor, &catalog, &board_part, through_roof, Some(&hollow)).is_err(),
            "a component crossing the roof must be rejected"
        );
        let tank = catalog
            .components
            .values()
            .find(|a| a.definition.value("dir") == "airtank_big")
            .unwrap();
        let motor = find("styles/ztek2.txt");
        for (asset, id) in [
            (board, default_socket(&board.model)),
            (tank, default_socket(&tank.model)),
            (motor, 1),
            (motor, 2),
        ] {
            editor.pending = Some(asset.key.clone());
            editor.selected_socket = id;
            editor
                .document
                .robot
                .components
                .push(candidate(&editor, &catalog, &pick, None).unwrap());
        }
        let resolved = |robot: &RobotDefinition| {
            robot
                .components
                .iter()
                .enumerate()
                .map(|(i, c)| {
                    if i == 0 {
                        ResolvedComponent {
                            key: "@chassis".into(),
                            model: Arc::new(robot.chassis.model.clone().unwrap()),
                            hidden: false,
                        }
                    } else {
                        let a = &catalog.components[&normalize(&c.definition)];
                        ResolvedComponent {
                            key: a.key.clone(),
                            model: a.model.clone(),
                            hidden: false,
                        }
                    }
                })
                .collect::<Vec<_>>()
        };
        let assembly = assemble(&editor.document.robot, &resolved(&editor.document.robot)).unwrap();
        let wheel = find("miniwheel.txt");
        editor.pending = Some(wheel.key.clone());
        editor.selected_socket = default_socket(&wheel.model);
        let wheel_pick = Pick {
            attachment: Some(3),
            component: 3,
            position: Vec3::ZERO,
            ..pick
        };
        let wheel_part = candidate(&editor, &catalog, &wheel_pick, Some(&assembly)).unwrap();
        assert_eq!(wheel_part.parent_mount, 3); // ztek's axle_M -> wheel's Axle_F
        editor.document.robot.components.push(wheel_part);
        assert!(
            candidate(&editor, &catalog, &wheel_pick, Some(&assembly)).is_err(),
            "occupied axle must be rejected"
        );
        let bytes = ra2_formats::bot::encode(&editor.document.robot).unwrap();
        let robot = ra2_formats::bot::decode(&bytes).unwrap();
        assert_eq!(robot.components[3].own_mount, 1);
        assert_eq!(robot.components[4].own_mount, 2);
        let assembly = assemble(&robot, &resolved(&robot)).unwrap();
        for (i, asset, id) in [(3, motor, 1), (4, motor, 2)] {
            let socket = selected_socket(&asset.model, id).unwrap();
            let frame = assembly.parts[i].transform * socket.transform;
            assert!(frame.transform_point3(Vec3::ZERO).length() < 1e-5);
            assert!(frame.transform_vector3(Vec3::Z).dot(Vec3::Y) > 0.999);
        }
        println!(
            "Checked {checked} original base mounts, both Z-Tek mounts, large air tank, mixed-case wheel coupling, occupancy and BOT roundtrip"
        );
    }
}
