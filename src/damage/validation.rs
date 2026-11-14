use super::*;

pub(super) fn check(catalogue: &content::Catalogue) -> Result<(), String> {
    let mut failures = Vec::new();
    let start = Instant::now();
    for (entry, density) in catalogue.entries.iter().flat_map(|entry| {
        [DEFAULT_DENSITY, MESH_DENSITIES.len() - 1].map(|density| (entry, density))
    }) {
        let result = (|| {
            let mut loaded =
                content::Loaded::load_with_density(&catalogue.root, &entry.path, density)?;
            let mut physics = physics::RigidWorld::new(&loaded)?;
            assert!(physics.world.soft_bodies.is_empty());
            for _ in 0..60 {
                physics.step(1.0 / 120.0);
            }
            if !physics.world.quarantine().is_empty() {
                return Err("Rigid body became unstable".into());
            }
            let (min, max) = loaded.shell.bounds;
            let hit = loaded
                .shell
                .raycast(
                    Vec3::new((min.x + max.x) * 0.5, max.y + 2.0, (min.z + max.z) * 0.5),
                    -Vec3::Y,
                )
                .ok_or("Cannot pick roof")?;
            physics.strike(
                Aim {
                    target: Target::Shell(hit),
                    point: hit.point,
                    normal: hit.normal,
                    distance: hit.distance,
                },
                1.0,
            );
            let contacts = (0..120).filter_map(|_| physics.step(1.0 / 120.0)).count();
            if contacts != 1 {
                return Err(format!(
                    "Hammer produced {contacts} damage events, expected exactly one"
                ));
            }
            let result = loaded
                .shell
                .dent(hit, 0.65, 0.08, 0.4, 30.0, &mut loaded.boxes);
            if result.changed_nodes == 0 {
                return Err("Roof dent changed no vertices".into());
            }
            if loaded.shell.positions.iter().any(|p| !p.is_finite()) {
                return Err("Nonfinite dent position".into());
            }
            if density == DEFAULT_DENSITY {
                physics::check_detachment(&loaded)?;
            }
            println!(
                "DAMAGE CHECK: {} / {}x / {} nodes / {} triangles / {} boxes / {} moved",
                entry.name,
                MESH_DENSITIES[density],
                loaded.shell.positions.len(),
                loaded.shell.triangles.len(),
                loaded.boxes.len(),
                result.changed_nodes
            );
            Ok::<_, String>(())
        })();
        if let Err(error) = result {
            failures.push(format!(
                "{} / {}x: {error}",
                entry.path.display(),
                MESH_DENSITIES[density]
            ));
        }
    }
    if !failures.is_empty() {
        return Err(failures.join("\n"));
    }
    check_lifecycle(catalogue)?;
    println!(
        "PASS: {} native robots load, rigid-step, ray-pick and dent at 1x and 8x mesh density without soft bodies ({:.2}s)",
        catalogue.entries.len(),
        start.elapsed().as_secs_f64()
    );
    Ok(())
}

/// Runs the actual spawn/update/replace systems without a window or GPU. This
/// catches ECS query conflicts and leaked instance entities in restricted CI.
fn break_component(lab: &mut Lab) -> usize {
    let b = lab
        .active
        .loaded
        .boxes
        .iter()
        .find(|b| {
            lab.active.physics.attached(b.component)
                && lab.active.loaded.content.health.parts[b.component].damageable
        })
        .expect("Native robot has no attached components")
        .clone();
    lab.active.loaded.content.health.parts[b.component].current = 60.;
    let before = lab.active.physics.detached.len();
    lab.impact(
        Aim {
            target: Target::Component(b.component),
            point: b.transform.transform_point3((b.min + b.max) * 0.5),
            normal: Vec3::Y,
            distance: 1.,
        },
        1.,
    );
    assert_eq!(
        lab.active.physics.detached.len(),
        before + 1,
        "A real damage event reaching 100% did not detach the component"
    );
    b.component
}

fn check_lifecycle(catalogue: &content::Catalogue) -> Result<(), String> {
    let catalogue = content::Catalogue::scan(&catalogue.root)?;
    let loaded = content::Loaded::load(&catalogue.root, &catalogue.entries[0].path)?;
    // Debris uses a cheap floor plane. Verify it agrees with both the original
    // visible arena and its collision shell after the arena translation.
    let has_floor = |visible: bool| {
        loaded
            .content
            .arena
            .objects
            .iter()
            .filter(|o| {
                if visible {
                    o.visible
                } else {
                    o.name == "h_box"
                }
            })
            .any(|o| {
                let positions: Vec<_> = o
                    .positions()
                    .map(|p| p + loaded.content.arena_offset)
                    .collect();
                o.faces.iter().any(|f| {
                    let p = f.vertices.map(|i| positions[i]);
                    p.iter().all(|v| v.y.abs() < 0.002)
                        && (p[1] - p[0]).cross(p[2] - p[0]).length() > 1.
                })
            })
    };
    assert!(
        has_floor(false) && has_floor(true),
        "Debris floor plane does not match native arena geometry"
    );
    let lab = Lab::new(loaded, 0)?;
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default()))
        .init_asset::<Mesh>()
        .init_asset::<StandardMaterial>()
        .init_asset::<Image>()
        .insert_resource(lab)
        .insert_resource(catalogue)
        .add_message::<crate::shutdown::ExitRequest>()
        .add_observer(ui::activate)
        .add_observer(ui::range_change)
        .insert_resource(Orbit {
            yaw: 0.65,
            pitch: 0.65,
            distance: 4.5,
            focus: Vec3::ZERO,
        })
        .add_systems(Startup, (render::setup, effects::setup, ui::setup).chain())
        .add_systems(
            Update,
            (
                ui::range_visuals,
                load_robot,
                effects::animate,
                render::synchronize,
                effects::draw,
            )
                .chain(),
        );
    app.update();
    let density_slider = {
        let world = app.world_mut();
        let mut controls = world.query::<(Entity, &ui::Control)>();
        controls
            .iter(world)
            .find_map(|(entity, control)| (*control == ui::Control::Density).then_some(entity))
            .unwrap()
    };
    // Range controls clamp input and preview density without repeated reloads.
    app.world_mut()
        .trigger(bevy::ui_widgets::ValueChange::<f32> {
            source: density_slider,
            value: 99.0,
            is_final: false,
        });
    assert_eq!(app.world().resource::<Lab>().density, DEFAULT_DENSITY);
    assert_eq!(app.world().resource::<Lab>().density_preview, 4.0);
    app.update();
    assert!(
        app.world().resource::<Lab>().loading.is_none(),
        "Density preview rebuilt the robot before release"
    );
    let shown = {
        let world = app.world_mut();
        let mut labels = world.query::<(&ui::Label, &Text)>();
        labels
            .iter(world)
            .find_map(|(label, text)| {
                matches!(label, ui::Label::Value(ui::Control::Density)).then_some(text.0.clone())
            })
            .unwrap()
    };
    assert_eq!(
        shown, "8x",
        "Range value must be visible next to the control immediately"
    );
    for (value, expected) in [
        (99.0, 4),
        (-5.0, 0),
        (DEFAULT_DENSITY as f32, DEFAULT_DENSITY),
    ] {
        app.world_mut()
            .trigger(bevy::ui_widgets::ValueChange::<f32> {
                source: density_slider,
                value,
                is_final: true,
            });
        assert_eq!(app.world().resource::<Lab>().density, expected);
    }
    let shading_button = {
        let world = app.world_mut();
        let mut actions = world.query::<(Entity, &ui::Action)>();
        actions
            .iter(world)
            .find_map(|(entity, action)| matches!(action, ui::Action::Shading).then_some(entity))
            .expect("Shading toggle is missing")
    };
    {
        let mut lab = app.world_mut().resource_mut::<Lab>();
        let (min, max) = lab.active.loaded.shell.bounds;
        let hit = lab
            .active
            .loaded
            .shell
            .raycast(
                Vec3::new((min.x + max.x) * 0.5, max.y + 1.0, (min.z + max.z) * 0.5),
                -Vec3::Y,
            )
            .unwrap();
        lab.impact(
            Aim {
                target: Target::Shell(hit),
                point: hit.point,
                normal: hit.normal,
                distance: hit.distance,
            },
            1.0,
        );
        assert!(lab.changed > 0);
        assert!(lab.effects.count() > 0);
        assert!(lab.active.loaded.paint.revision > 0);
    }
    let positions = app
        .world()
        .resource::<Lab>()
        .active
        .loaded
        .shell
        .positions
        .clone();
    for smooth in [true, false] {
        app.world_mut().trigger(bevy::ui_widgets::Activate {
            entity: shading_button,
        });
        app.update();
        let lab = app.world().resource::<Lab>();
        assert_eq!(lab.smooth, smooth);
        assert_eq!(lab.strikes, 1, "Shading toggle reset the dent history");
        assert_eq!(lab.active.loaded.shell.positions, positions);
        assert!(
            lab.loading.is_none(),
            "Shading toggle must not reload the robot"
        );
    }
    let body_count = app
        .world()
        .resource::<Lab>()
        .active
        .physics
        .world
        .bodies
        .len();
    {
        let mut lab = app.world_mut().resource_mut::<Lab>();
        for _ in 0..960 {
            lab.effects.step(1. / 120.);
        }
        assert!(lab.effects.debris_count() > 0);
        assert_eq!(lab.effects.count(), 0);
        assert_eq!(lab.active.physics.world.bodies.len(), body_count);
    }
    app.update();
    let visible_chunks = |world: &mut World| {
        world
            .query_filtered::<&Visibility, With<effects::GroundVisual>>()
            .iter(world)
            .filter(|v| **v != Visibility::Hidden)
            .count()
    };
    assert!(
        visible_chunks(app.world_mut()) > 0,
        "Landed debris has no visible mesh"
    );
    app.update();
    assert_eq!(
        app.world().resource::<effects::Graphics>().draw_ms,
        0.,
        "Resting debris must not regenerate meshes every frame"
    );
    let (particles_button, clear_debris_button) = {
        let world = app.world_mut();
        let mut actions = world.query::<(Entity, &ui::Action)>();
        let mut find = |action: ui::Action| {
            actions
                .iter(world)
                .find_map(|(entity, a)| {
                    (std::mem::discriminant(a) == std::mem::discriminant(&action)).then_some(entity)
                })
                .unwrap()
        };
        (find(ui::Action::Particles), find(ui::Action::ClearDebris))
    };
    app.world_mut().trigger(bevy::ui_widgets::Activate {
        entity: particles_button,
    });
    assert!(
        app.world().resource::<Lab>().effects.debris_count() > 0,
        "Disabling emission must preserve existing debris"
    );
    app.world_mut().trigger(bevy::ui_widgets::Activate {
        entity: clear_debris_button,
    });
    app.update();
    assert_eq!(
        visible_chunks(app.world_mut()),
        0,
        "Clear ground left stale geometry visible"
    );
    app.world_mut().trigger(bevy::ui_widgets::Activate {
        entity: particles_button,
    });
    let broken = {
        let mut lab = app.world_mut().resource_mut::<Lab>();
        let broken = break_component(&mut lab);
        let body = lab.active.physics.detached[0].body;
        lab.active.physics.world.bodies[body].set_position(
            bevy_rapier3d::rapier::math::Pose::from_parts(
                Vec3::new(4., 2., -2.),
                Quat::from_rotation_z(0.4),
            ),
            true,
        );
        lab.wire = true;
        lab.boxes = true;
        lab.revision += 1;
        broken
    };
    app.update();
    let (expected_frame, expected_box, picked) = {
        let lab = app.world().resource::<Lab>();
        let delta = lab.active.physics.transform().to_matrix().inverse()
            * lab.active.physics.frame(broken).to_matrix();
        let b = lab
            .active
            .loaded
            .boxes
            .iter()
            .find(|b| b.component == broken)
            .unwrap();
        let expected_box = delta
            * b.transform
            * Mat4::from_translation((b.min + b.max) * 0.5)
            * Mat4::from_scale(b.max - b.min);
        let world = lab.active.physics.frame(broken);
        let picked = lab.active.loaded.picks.iter().filter(|(i, _)| *i == broken).any(|(_, p)| {
            let center = world.transform_point((p[0] + p[1] + p[2]) / 3.);
            let normal = world.rotation * (p[1] - p[0]).cross(p[2] - p[0]).normalize_or_zero();
            lab.active.pick(center + normal * 0.03, -normal).is_some_and(|aim|
                matches!(aim.target, Target::Component(i) if lab.active.physics.detached[0].parts.contains(&i)))
        });
        (
            delta * lab.active.loaded.content.assembly.parts[broken].transform,
            expected_box,
            picked,
        )
    };
    assert!(
        picked,
        "Ray picking did not follow the detached body's new pose"
    );
    {
        let world = app.world_mut();
        let mut frames = world.query::<(Entity, &render::PartFrame, &Transform)>();
        let (entity, _, transform) = frames
            .iter(world)
            .find(|(_, part, _)| part.0 == broken)
            .unwrap();
        assert!(
            transform.to_matrix().abs_diff_eq(expected_frame, 1e-4),
            "Textured component did not follow detached physics"
        );
        let mut wires = world.query_filtered::<(&ChildOf, &Visibility), With<render::Wire>>();
        assert!(
            wires
                .iter(world)
                .any(|(parent, v)| parent.parent() == entity && *v == Visibility::Visible),
            "Wire view disappeared after detachment"
        );
        let mut boxes = world.query::<(&render::BoxVisual, &Transform, &Visibility)>();
        let (_, transform, visibility) = boxes.iter(world).find(|(b, _, _)| b.0 == broken).unwrap();
        assert!(transform.to_matrix().abs_diff_eq(expected_box, 1e-4));
        assert_eq!(*visibility, Visibility::Visible);
    }
    let roots = |world: &mut World| -> Vec<Entity> {
        world
            .query::<(Entity, &Name)>()
            .iter(world)
            .filter(|(_, name)| name.as_str().starts_with("Damage robot / "))
            .map(|(e, _)| e)
            .collect()
    };
    let baseline_meshes = app.world().resource::<Assets<Mesh>>().len();
    let baseline_materials = app.world().resource::<Assets<StandardMaterial>>().len();
    let baseline_images = app.world().resource::<Assets<Image>>().len();
    let reference_nodes = app
        .world()
        .resource::<Lab>()
        .active
        .loaded
        .shell
        .positions
        .len();
    for (index, density) in [
        (1, DEFAULT_DENSITY),
        (0, DEFAULT_DENSITY),
        (0, 4),
        (0, 0),
        (1, 3),
        (0, DEFAULT_DENSITY),
    ] {
        let count = app.world().resource::<content::Catalogue>().entries.len();
        let index = index % count;
        let same_robot = app.world().resource::<Lab>().active.index == index;
        let orbit_before = {
            let mut orbit = app.world_mut().resource_mut::<Orbit>();
            orbit.focus = Vec3::new(0.1, 0.2, 0.3);
            orbit.distance = 6.7;
            (orbit.focus, orbit.distance)
        };
        let previous = roots(app.world_mut());
        assert_eq!(previous.len(), 1);
        {
            let mut lab = app.world_mut().resource_mut::<Lab>();
            let (min, max) = lab.active.loaded.shell.bounds;
            let hit = lab
                .active
                .loaded
                .shell
                .raycast(
                    Vec3::new((min.x + max.x) * 0.5, max.y + 1.0, (min.z + max.z) * 0.5),
                    -Vec3::Y,
                )
                .unwrap();
            lab.impact(
                Aim {
                    target: Target::Shell(hit),
                    point: hit.point,
                    normal: hit.normal,
                    distance: hit.distance,
                },
                1.0,
            );
            for _ in 0..960 {
                lab.effects.step(1. / 120.);
            }
            assert!(lab.effects.debris_count() > 0);
            break_component(&mut lab);
            lab.wire = true;
            lab.boxes = true;
            lab.smooth = true;
            lab.revision += 1;
            lab.selection = index;
            lab.density = density;
            lab.reset = true;
        }
        for _ in 0..1000 {
            app.update();
            if app.world().resource::<Lab>().loading.is_none() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        let lab = app.world().resource::<Lab>();
        if lab.loading.is_some() {
            return Err("Robot replacement timed out".into());
        }
        assert_eq!(lab.active.index, index);
        assert_eq!(lab.active.loaded.density, density);
        assert!(
            lab.active.physics.detached.is_empty(),
            "Detached rigid bodies survived robot replacement"
        );
        assert_eq!(lab.active.physics.world.bodies.len(), 2);
        assert_eq!(
            lab.effects.count(),
            0,
            "Particles survived robot replacement"
        );
        assert_eq!(
            lab.effects.debris_count(),
            0,
            "Ground debris survived robot replacement"
        );
        assert_eq!(
            lab.active.loaded.paint.revision, 0,
            "Paint wear survived replacement"
        );
        assert!(lab.smooth, "Shading choice was reset on replacement");
        if index == 0 && density == 4 {
            assert!(
                lab.active.loaded.shell.positions.len() > reference_nodes * 16,
                "Density increase did not refine the mesh beyond the old subdivision cap"
            );
        }
        if same_robot {
            let orbit = app.world().resource::<Orbit>();
            assert_eq!((orbit.focus, orbit.distance), orbit_before);
        }
        assert_eq!(lab.strikes, 0);
        assert!(lab.active.loaded.boxes.iter().all(|b| b.damage == 0.0));
        assert_eq!(roots(app.world_mut()).len(), 1);
        assert!(
            app.world().get_entity(previous[0]).is_err(),
            "Old robot root survived replacement"
        );
    }
    // A late result from a superseded density must not overwrite the latest choice.
    app.world_mut().resource_mut::<Lab>().density = 4;
    app.update();
    assert!(app.world().resource::<Lab>().loading.is_some());
    app.world_mut().resource_mut::<Lab>().density = 0;
    for _ in 0..1000 {
        app.update();
        if app.world().resource::<Lab>().loading.is_none() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    {
        let lab = app.world().resource::<Lab>();
        assert!(lab.loading.is_none(), "Density replacement timed out");
        assert_eq!(lab.active.loaded.density, 0);
        assert!(lab.active.loaded.shell.positions.len() < reference_nodes);
    }
    for _ in 0..8 {
        app.update();
    }
    assert!(
        app.world().resource::<Assets<Mesh>>().len() <= baseline_meshes + 8,
        "Instance meshes leaked after replacement"
    );
    assert!(
        app.world().resource::<Assets<StandardMaterial>>().len() <= baseline_materials + 2,
        "Instance materials leaked after replacement"
    );
    assert!(
        app.world().resource::<Assets<Image>>().len() <= baseline_images + 2,
        "Instance images leaked after replacement"
    );
    println!(
        "PASS: headless scene systems, Flat/Smooth toggle, wire/boxes, robot/density replacement, latest requested density, range values, bounded particles/paint cleanup, camera preservation and mesh/material/image cleanup"
    );
    Ok(())
}

#[derive(Default)]
pub(super) struct Smoke {
    stage: usize,
    age: f32,
    first: Option<usize>,
    baseline: usize,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn smoke(
    mut commands: Commands,
    time: Res<Time<Real>>,
    options: Res<Options>,
    mut lab: ResMut<Lab>,
    catalogue: Res<content::Catalogue>,
    mut state: Local<Smoke>,
    meshes: Res<Assets<Mesh>>,
    mut exit: MessageWriter<crate::shutdown::ExitRequest>,
) {
    if !options.smoke_test || lab.loading.is_some() {
        return;
    }
    state.age += time.delta_secs();
    if state.age < 1.2 {
        return;
    }
    state.age = 0.0;
    let capture = |commands: &mut Commands, name: &str| {
        commands
            .spawn(bevy::render::view::screenshot::Screenshot::primary_window())
            .observe(bevy::render::view::screenshot::save_to_disk(format!(
                "logs/damage-{name}.png"
            )));
    };
    match state.stage {
        0 => {
            capture(&mut commands, "original");
            state.first = Some(lab.active.index);
            state.baseline = meshes.len();
        }
        1..=3 => {
            let (min, max) = lab.active.loaded.shell.bounds;
            // Prefer the chassis roof even when equipment protrudes above it.
            let hit = lab
                .active
                .loaded
                .shell
                .raycast(
                    Vec3::new((min.x + max.x) * 0.5, max.y + 1.0, (min.z + max.z) * 0.5),
                    -Vec3::Y,
                )
                .unwrap();
            let aim = Aim {
                target: Target::Shell(hit),
                point: hit.point,
                normal: hit.normal,
                distance: hit.distance,
            };
            lab.active.physics.strike(aim, 2.0);
        }
        4 => {
            assert!(lab.strikes > 0, "Hammer must contact robot");
            assert!(lab.changed > 0);
            lab.wire = true;
            lab.boxes = true;
            lab.revision += 1;
        }
        5 => capture(&mut commands, "wire-damage"),
        6 => {
            lab.selection = (lab.active.index + 1) % catalogue.entries.len();
        }
        7 => {
            assert_eq!(lab.strikes, 0);
            assert!(lab.active.loaded.boxes.iter().all(|b| b.damage == 0.0));
            lab.selection = state.first.unwrap();
        }
        8 => {
            lab.wire = false;
            lab.boxes = false;
            lab.revision += 1;
        }
        9 => {
            capture(&mut commands, "reloaded");
            assert!(
                meshes.len() <= state.baseline + 8,
                "Mesh assets retained after round-trip switching"
            );
        }
        _ => {
            info!("PASS: rigid hammer, permanent dents, debug wire, boxes and robot replacement");
            exit.write(crate::shutdown::ExitRequest);
        }
    }
    state.stage += 1;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires a local RA2 installation"]
    fn native_square_extender_styles_use_real_alpha_cutouts() {
        let root = std::env::var_os("RA2_GAME_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../RobotArena2")
            });
        for style in 1..=7 {
            let path = root.join(format!("Components/extender_square/sq_extender{style}.gmf"));
            let model = ra2_formats::ModelImporters::with_ra2()
                .decode(&std::fs::read(&path).unwrap())
                .unwrap()
                .model;
            let mut meshes = Assets::default();
            let mut materials = Assets::default();
            let mut images = Assets::default();
            arena_runtime::render::prepare_model(
                &model,
                &arena_runtime::content::FileTextures::new(path.parent().unwrap()),
                &arena_runtime::render::RenderSettings::default(),
                &mut arena_runtime::render::TextureCache::default(),
                &mut meshes,
                &mut materials,
                &mut images,
            )
            .unwrap();
            let mut checked = 0;
            for (_, material) in materials.iter() {
                assert!(
                    matches!(material.alpha_mode, AlphaMode::Mask(_)),
                    "Square extender style {style} must discard holes, not blend metal"
                );
                assert_eq!(material.base_color.alpha(), 1.0);
                let image = images
                    .get(material.base_color_texture.as_ref().unwrap())
                    .unwrap();
                let data = image.data.as_ref().unwrap();
                assert!(data.as_chunks::<4>().0.iter().any(|p| p[3] == 0));
                assert!(data.as_chunks::<4>().0.iter().any(|p| p[3] == 255));
                checked += 1;
            }
            assert!(checked > 0);
        }
    }
}
