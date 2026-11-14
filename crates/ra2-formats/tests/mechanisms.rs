use arena_core::Model;
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
fn scan(p: &Path, out: &mut Vec<PathBuf>) {
    if p.is_dir() {
        for e in std::fs::read_dir(p).unwrap() {
            scan(&e.unwrap().path(), out)
        }
    } else {
        out.push(p.into())
    }
}
#[test]
#[ignore = "requires a local RA2 installation"]
fn all_original_robots_build_articulated_rigs() {
    let root = std::env::var_os("RA2_GAME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../RobotArena2"));
    let mut files = vec![];
    scan(&root, &mut files);
    let index = files
        .iter()
        .map(|p| {
            (
                normalize(&p.strip_prefix(&root).unwrap().to_string_lossy()),
                p,
            )
        })
        .collect::<BTreeMap<_, _>>();
    let defs =
        ComponentCatalog::from_cfz(&std::fs::read(root.join("Components.cfz")).unwrap()).unwrap();
    let decoder = ModelImporters::with_ra2();
    let mut cache = BTreeMap::<String, Arc<Model>>::new();
    let mut bots = 0;
    let mut joints = 0;
    for file in files
        .iter()
        .filter(|p| p.extension().is_some_and(|s| s.eq_ignore_ascii_case("bot")))
    {
        let robot = ra2_formats::bot::decode(&std::fs::read(file).unwrap()).unwrap();
        let mut models = vec![ResolvedComponent {
            key: "chassis".into(),
            model: Arc::new(robot.chassis.model.clone().unwrap()),
            hidden: false,
        }];
        let mut definitions = vec![ComponentDefinition::parse("")];
        for c in robot.components.iter().skip(1) {
            let d = defs.get(&c.definition).unwrap().clone();
            let k = normalize(&d.model_path().unwrap());
            let m = cache
                .entry(k.clone())
                .or_insert_with(|| {
                    Arc::new(
                        decoder
                            .decode(&std::fs::read(index[&k]).unwrap())
                            .unwrap()
                            .model,
                    )
                })
                .clone();
            models.push(ResolvedComponent {
                key: k,
                model: m,
                hidden: c.class == "SmartZone",
            });
            definitions.push(d);
        }
        let a = assemble(&robot, &models).unwrap();
        let rig = ra2_formats::mechanisms::build(&robot, &a, &models, &definitions)
            .unwrap_or_else(|e| panic!("{}: {e}", file.display()));
        assert!(
            rig.bodies
                .iter()
                .all(|b| b.frame.is_finite() && b.mass >= 0.0)
        );
        for j in &rig.joints {
            assert!(j.parent < rig.bodies.len() && j.child < rig.bodies.len());
            assert!(
                j.axis.length() > 0.9,
                "{} {} {:?}",
                file.display(),
                j.component,
                j.axis
            );
        }
        for (i, m) in models.iter().enumerate() {
            assert_eq!(rig.objects[i].len(), m.model.objects.len());
        }
        let frozen = ra2_formats::pose::freeze(&robot, &models, &definitions).unwrap();
        assert!((frozen.assembly.mass - a.mass).abs() < 0.001);
        let socket = |model: &Model, id: u32| {
            model
                .attachments
                .iter()
                .find(|a| a.properties.get("id").and_then(|s| s.parse::<u32>().ok()) == Some(id))
                .or_else(|| {
                    if model.attachments.len() == 1 {
                        model.attachments.first()
                    } else {
                        None
                    }
                })
                .unwrap()
                .transform
        };
        for (i, c) in robot.components.iter().enumerate().skip(1) {
            // Binding changes the rigid coordinate frame, preserving scale, vertices and UVs.
            let raw = &models[i].model;
            let bound = ra2_formats::pose::bind_model(raw, &definitions[i]);
            let bound = bound.as_ref().unwrap_or(raw);
            let posed = &frozen.models[i].model;
            for (source, target) in bound.objects.iter().zip(&posed.objects) {
                assert_eq!(source.vertices, target.vertices);
                assert_eq!(source.faces.len(), target.faces.len());
                assert_eq!(source.uv_channels.len(), target.uv_channels.len());
                for (channel, uv) in &source.uv_channels {
                    assert_eq!(uv.vertices, target.uv_channels[channel].vertices);
                    assert_eq!(uv.faces, target.uv_channels[channel].faces);
                }
                if rig.objects[i]
                    .iter()
                    .all(|&body| body == rig.component_bodies[i])
                {
                    assert!(
                        source.transform.abs_diff_eq(target.transform, 0.00001),
                        "{} part {i}: display-node TM was changed",
                        file.display()
                    );
                }
            }
            let parent = c.parent.unwrap();
            let expected = frozen.assembly.parts[parent].transform
                * if parent == 0 {
                    arena_core::Mat4::IDENTITY
                } else {
                    socket(&frozen.models[parent].model, c.parent_mount)
                        * arena_core::Mat4::from_rotation_x(std::f32::consts::PI)
                }
                * c.mounting.matrix();
            let actual =
                frozen.assembly.parts[i].transform * socket(&frozen.models[i].model, c.own_mount);
            assert!(
                expected.abs_diff_eq(actual, 0.0005),
                "{} part {i}: posed socket frames drifted",
                file.display()
            );
            if robot.name.eq_ignore_ascii_case("EMERGENCY")
                && c.definition.to_lowercase().contains("flipper")
            {
                let part = &frozen.assembly.parts[i];
                let points = frozen.models[i]
                    .model
                    .objects
                    .iter()
                    .filter(|o| o.visible)
                    .flat_map(|o| o.positions())
                    .map(|p| part.transform.transform_point3(p));
                let (min, max) = points.fold(
                    (
                        arena_core::Vec3::splat(f32::INFINITY),
                        arena_core::Vec3::splat(f32::NEG_INFINITY),
                    ),
                    |(min, max), p| (min.min(p), max.max(p)),
                );
                println!("EMERGENCY flipper {i}: {min:?}..{max:?}");
                assert!(
                    (max.y - min.y) < (max.x - min.x).max(max.z - min.z) * 0.5,
                    "Emergency flipper stands through roof"
                );
                let object = &frozen.models[i].model.objects[0];
                let world = part.transform * object.transform;
                // Native mesh triangles 2/3 carry the broad red/white ramp UVs;
                // triangles 4/5 are its flat underside. Bounds alone cannot
                // distinguish the upright wedge from an upside-down wedge.
                // Saved 10-degree rest tilt plus the mesh's ramp slope gives
                // a working normal.y of about 0.923, rather than a level ramp.
                for face in &object.faces[2..4] {
                    let [a, b, c] = face
                        .vertices
                        .map(|v| world.transform_point3(object.vertices[v]));
                    let normal = (b - a).cross(c - a).normalize();
                    assert!(
                        normal.y > 0.9,
                        "Emergency flipper {i}: working face is inverted: {normal:?}"
                    );
                }
                let tip = world.transform_point3(object.vertices[2]);
                let rear = world.transform_point3(object.vertices[1]);
                assert!(
                    tip.z < rear.z - 0.6 && tip.y < rear.y,
                    "Emergency flipper {i}: wedge does not descend towards the front"
                );
            }
        }
        // Regressions reported in the damage demo: raised strikers must not
        // become ground supports. Dementia has a deliberately low front tool.
        let weapon = match robot.name.to_ascii_lowercase().as_str() {
            "bear" => Some(("bearclaw", 0.6)),
            "bushido" => Some(("samurai", 0.6)),
            "deadbeat" => Some(("sledgehammer", 0.9)),
            "c.r.u.s.h." => Some(("/hammer/", 1.0)),
            "dementia" => Some(("darkspike", -0.1)),
            _ => None,
        };
        if let Some((tool, minimum_height)) = weapon {
            let mut found = 0;
            let wheel_floor = robot
                .components
                .iter()
                .enumerate()
                .filter(|(_, c)| c.class == "Wheel")
                .flat_map(|(i, _)| {
                    let t = frozen.assembly.parts[i].transform;
                    frozen.models[i]
                        .model
                        .objects
                        .iter()
                        .filter(|o| o.visible)
                        .flat_map(move |o| o.positions().map(move |p| t.transform_point3(p).y))
                })
                .reduce(f32::min)
                .unwrap();
            for (i, m) in frozen
                .models
                .iter()
                .enumerate()
                .filter(|(_, m)| m.key.contains(tool))
            {
                let t = frozen.assembly.parts[i].transform;
                let min_y = m
                    .model
                    .objects
                    .iter()
                    .filter(|o| o.visible)
                    .flat_map(|o| o.positions())
                    .map(|p| t.transform_point3(p).y)
                    .reduce(f32::min)
                    .unwrap();
                println!(
                    "{} tool {i}: min y={min_y:.6}, wheel floor={wheel_floor:.6}",
                    robot.name
                );
                assert!(
                    min_y > minimum_height && min_y > wheel_floor + 0.03,
                    "{} tool {i} points into floor: {min_y} (wheels {wheel_floor})",
                    robot.name
                );
                found += 1;
            }
            assert_eq!(found, if tool == "bearclaw" { 2 } else { 1 });
        }
        joints += rig.joints.len();
        bots += 1;
    }
    assert!(bots >= 57);
    assert!(joints > 150);
    println!("RIG CORPUS PASSED: {bots} native robots, {joints} joints");
}
