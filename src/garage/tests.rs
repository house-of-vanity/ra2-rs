use super::*;
use arena_core::robot::RobotDefinition;
use bevy::time::TimeUpdateStrategy;
use std::{path::Path, time::Duration};
pub(crate) fn native_garage(path: &str) -> (App, RobotDefinition) {
    let root = std::env::var_os("RA2_GAME_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../RobotArena2"));
    let catalog = crate::editor::catalog::Catalog::load(
        &root,
        &mut legacy_ui::assets::LegacyAssets::default(),
    )
    .unwrap();
    let robot = ra2_formats::bot::decode(&std::fs::read(root.join(path)).unwrap()).unwrap();
    let mut editor = Editor {
        mode: Mode::Test,
        ..Default::default()
    };
    editor.document.load(robot.clone());
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, TransformPlugin));
    app.insert_resource(Time::<Fixed>::from_seconds(crate::STEP))
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
            crate::STEP,
        )))
        .insert_resource(TimestepMode::Fixed {
            dt: crate::STEP as f32,
            substeps: 1,
        })
        .add_plugins(RapierPhysicsPlugin::<NoUserData>::default().in_fixed_schedule());
    app.insert_resource(editor)
        .insert_resource(Navigation {
            page: Page::Workshop,
            back: Page::Main,
        })
        .init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<StandardMaterial>>()
        .init_resource::<Assets<Image>>()
        .insert_resource(catalog);
    let context = app
        .world_mut()
        .spawn((
            GarageContext,
            RapierContextSimulation::default(),
            RapierConfiguration::new(1.0),
        ))
        .id();
    let camera = app.world_mut().spawn_empty().id();
    app.insert_resource(Garage {
        instance: RobotInstance {
            active: true,
            running: true,
            ..RobotInstance::new(0, context, crate::config::GarageConfig::default())
        },
        camera,
        target: default(),
        obstacles: vec![],
        environment: vec![],
        reset: false,
        obstacle: 0,
        loaded_obstacle: 0,
        yaw: 0.0,
        pitch: 0.0,
        distance: 4.0,
        hud_time: 0.0,
    });
    app.add_systems(
        Startup,
        |mut commands: Commands,
         mut garage: ResMut<Garage>,
         editor: Res<Editor>,
         catalog: Res<crate::editor::catalog::Catalog>,
         mut meshes: ResMut<Assets<Mesh>>,
         mut materials: ResMut<Assets<StandardMaterial>>,
         mut images: ResMut<Assets<Image>>| {
            garage.placement = Some(environment::start(&catalog).unwrap());
            spawn::robot(
                &mut commands,
                &mut garage,
                &editor.document.robot,
                &catalog,
                &mut meshes,
                &mut materials,
                &mut images,
            )
            .unwrap();
            garage.environment = environment::environment(
                &mut commands,
                &garage,
                &catalog,
                &mut meshes,
                &mut materials,
                &mut images,
            )
            .unwrap();
        },
    )
    .add_systems(
        FixedUpdate,
        (drive::motors, damage::capture, tyres::contacts)
            .chain()
            .before(PhysicsSet::SyncBackend),
    )
    .add_systems(FixedUpdate, damage::contacts.after(PhysicsSet::Writeback));
    (app, robot)
}

#[test]
#[ignore = "requires original wheel definitions and models"]
fn native_wheel_contacts_are_smooth_for_every_wheel_variant() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../RobotArena2");
    let catalog = crate::editor::catalog::Catalog::load(
        &root,
        &mut legacy_ui::assets::LegacyAssets::default(),
    )
    .unwrap();
    let mut count = 0;
    for asset in catalog
        .components
        .values()
        .filter(|a| a.definition.value("base") == "Wheel")
    {
        let (_, centre, rotation, shape) = arena_runtime::robot::wheel_collider(&asset.model)
            .unwrap_or_else(|| panic!("No smooth tyre for {}", asset.key));
        let cylinder = shape.raw.as_round_cylinder().unwrap();
        let radius = cylinder.inner_shape.radius + cylinder.border_radius;
        let half_width = cylinder.inner_shape.half_height + cylinder.border_radius;
        let visible: Vec<_> = asset
            .model
            .objects
            .iter()
            .filter(|o| o.visible && !o.faces.is_empty())
            .flat_map(|o| o.positions())
            .map(|p| rotation.inverse() * (p - centre))
            .collect();
        let radial = visible
            .iter()
            .map(|p| Vec2::new(p.x, p.z).length())
            .reduce(f32::max)
            .unwrap();
        let axial = visible.iter().map(|p| p.y.abs()).reduce(f32::max).unwrap();
        assert!(
            (radius - radial).abs() < 1e-5,
            "{}: tyre radius inflated",
            asset.key
        );
        assert!(
            (half_width - axial).abs() < 1e-5,
            "{}: tyre width inflated",
            asset.key
        );
        let sensor = asset
            .model
            .object(asset.definition.value("contact"))
            .unwrap();
        let sensor_radius = sensor
            .positions()
            .map(|p| rotation.inverse() * (p - centre))
            .map(|p| Vec2::new(p.x, p.z).length())
            .reduce(f32::max)
            .unwrap();
        println!(
            "Smooth wheel {} radius={radius:.4} width={:.4} old_sensor_radius={sensor_radius:.4} grip={} resistance={}",
            asset.name(),
            half_width * 2.,
            asset.definition.value("grip"),
            asset.definition.value("resistance")
        );
        count += 1;
    }
    assert!(count >= 9);
}

#[test]
#[ignore = "requires native C.R.U.S.H. and Scout wheel geometry"]
fn native_wheels_rest_at_visible_tyre_height_without_polygon_colliders() {
    for path in ["Teams/Team0/Bot0.bot", "AI/Team0/Bot0.bot"] {
        let (mut app, robot) = native_garage(path);
        for _ in 0..360 {
            app.update();
        }
        let world = app.world();
        let garage = world.resource::<Garage>();
        let rig = garage.rig.as_ref().unwrap();
        let catalog = world.resource::<crate::editor::catalog::Catalog>();
        let frozen = catalog.freeze(&robot).unwrap();
        let floor = garage
            .environment
            .iter()
            .find_map(|e| world.get::<Collider>(*e))
            .unwrap()
            .raw
            .compute_local_aabb()
            .mins
            .y;
        let mut cylinders = 0;
        for entity in &garage.bodies {
            if let Some(compound) = world
                .get::<Collider>(*entity)
                .and_then(|c| c.raw.as_compound())
            {
                cylinders += compound
                    .shapes()
                    .iter()
                    .filter(|(_, s)| s.as_round_cylinder().is_some())
                    .count();
            }
        }
        let mut wheels = 0;
        for (i, c) in robot
            .components
            .iter()
            .enumerate()
            .filter(|(_, c)| c.class == "Wheel")
        {
            let asset = &catalog.components[&ra2_formats::components::normalize(&c.definition)];
            let mut bottom = f32::INFINITY;
            for (n, object) in asset
                .model
                .objects
                .iter()
                .enumerate()
                .filter(|(_, o)| o.visible)
            {
                let body = rig.objects[i][n];
                let transform = world
                    .get::<Transform>(garage.bodies[body])
                    .unwrap()
                    .to_matrix()
                    * rig.bodies[body].frame.inverse()
                    * frozen.assembly.parts[i].transform;
                for p in object.positions() {
                    bottom = bottom.min(transform.transform_point3(p).y);
                }
            }
            let gap = bottom - floor;
            println!("VISIBLE TYRE {} part {i} floor gap={gap:.5}m", robot.name);
            // Small gaps between a smooth circle and the original polygon edges are
            // unavoidable; the oversized native contact sensor must not lift it.
            assert!(
                (-0.005..0.02).contains(&gap),
                "wheel floats above its visible tyre: {gap}"
            );
            wheels += 1;
        }
        assert_eq!(cylinders, wheels, "expected exactly one cylinder per wheel");
    }
}

fn press_native_keys(app: &mut App, robot: &RobotDefinition, keys: &[i32]) {
    let samples: Vec<_> = robot
        .controllers
        .iter()
        .map(|c| {
            c.keys
                .iter()
                .map(|k| {
                    f32::from(
                        k.get(k.len().saturating_sub(2))
                            .is_some_and(|k| keys.contains(k)),
                    )
                })
                .collect()
        })
        .collect();
    let mut g = app.world_mut().resource_mut::<Garage>();
    g.commands = g.controls.evaluate(robot, &samples);
}

#[test]
#[ignore = "native wheel collision and tank steering calibration"]
fn native_tank_turning_profile() {
    for path in [
        "AI/Team0/Bot0.bot",
        "Teams/Team0/Bot0.bot",
        "AI/Team0/Bot2.bot",
        "Teams/Team0/Bot5.bot",
    ] {
        for smooth in [false, true] {
            let (mut app, robot) = native_garage(path);
            app.world_mut()
                .resource_mut::<Garage>()
                .tuning
                .smooth_wheel_contacts = smooth;
            app.update();
            // A steering axle needs forward motion. Use an open plane so that
            // these measurements cannot include a collision with the garage wall.
            let environment = app.world().resource::<Garage>().environment.clone();
            let floor = environment
                .iter()
                .find_map(|e| app.world().get::<Collider>(*e))
                .unwrap()
                .raw
                .compute_local_aabb()
                .mins
                .y;
            let context = app.world().resource::<Garage>().context;
            for entity in environment {
                app.world_mut().despawn(entity);
            }
            app.world_mut().spawn((
                RigidBody::Fixed,
                Collider::cuboid(100., 0.5, 100.),
                Transform::from_xyz(0., floor - 0.5, 0.),
                Friction::coefficient(0.3),
                CollisionGroups::new(Group::GROUP_1, Group::ALL),
                RapierContextEntityLink(context),
            ));
            for _ in 0..240 {
                app.update();
            }
            let body = app.world().resource::<Garage>().bodies[0];
            for key in [30, 32] {
                let steering = robot.components.iter().any(|c| c.class == "Steering");
                let keys = if steering { vec![17, key] } else { vec![key] };
                press_native_keys(&mut app, &robot, &keys);
                let mut previous =
                    app.world().get::<Transform>(body).unwrap().rotation * Vec3::NEG_Z;
                let mut yaw = 0_f32;
                let mut min_y = f32::INFINITY;
                let mut max_y = f32::NEG_INFINITY;
                let mut tilt = 1_f32;
                for _ in 0..240 {
                    app.update();
                    let t = app.world().get::<Transform>(body).unwrap();
                    let forward = t.rotation * Vec3::NEG_Z;
                    yaw += previous
                        .cross(forward)
                        .y
                        .atan2(previous.x * forward.x + previous.z * forward.z);
                    previous = forward;
                    min_y = min_y.min(t.translation.y);
                    max_y = max_y.max(t.translation.y);
                    tilt = tilt.min((t.rotation * Vec3::Y).dot(Vec3::Y));
                }
                println!(
                    "TURN {} smooth={smooth} key={key} yaw={yaw:.3}rad bounce={:.4}m up={tilt:.3}",
                    robot.name,
                    max_y - min_y
                );
                assert!(yaw.is_finite() && min_y > -1.);
                if smooth && steering {
                    let world = app.world();
                    let g = world.resource::<Garage>();
                    let rig = g.rig.as_ref().unwrap();
                    for j in rig
                        .joints
                        .iter()
                        .filter(|j| j.kind == ActuatorKind::Steering)
                    {
                        let joint = world.get::<ImpulseJoint>(g.bodies[j.child]).unwrap();
                        let position = drive::joint_position(
                            world.get::<Transform>(g.bodies[j.parent]).unwrap(),
                            world.get::<Transform>(g.bodies[j.child]).unwrap(),
                            joint.data.as_ref(),
                            false,
                        );
                        assert!(
                            position.abs() < 0.68,
                            "steering knuckle escaped its limits: {position}"
                        );
                    }
                }
                if smooth {
                    assert!(yaw.abs() > 0.5, "{}: turn stalled", robot.name);
                    assert!(
                        if key == 30 { yaw > 0. } else { yaw < 0. },
                        "{}: reversed steering",
                        robot.name
                    );
                    assert!(tilt > 0.85, "{}: turn tips chassis", robot.name);
                }
            }
        }
    }
}

#[test]
#[ignore = "requires a local RA2 installation"]
fn native_robot_drives_in_isolated_garage_without_mutating_editor() {
    let (mut app, robot) = native_garage("AI/Team0/Bot0.bot");
    let original = ra2_formats::bot::encode(&robot).unwrap();
    for _ in 0..240 {
        app.update();
    }
    let body = app.world().resource::<Garage>().bodies[0];
    let start = app.world().get::<Transform>(body).unwrap().translation;
    assert!(
        (-0.2..2.0).contains(&start.y),
        "robot escaped floor {start:?}"
    );
    let samples = robot
        .controllers
        .iter()
        .map(|c| {
            c.keys
                .iter()
                .map(|k| f32::from(k.get(k.len().saturating_sub(2)) == Some(&17)))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    {
        let mut g = app.world_mut().resource_mut::<Garage>();
        g.commands = g.controls.evaluate(&robot, &samples);
    }
    for _ in 0..240 {
        app.update();
    }
    let end = app.world().get::<Transform>(body).unwrap().translation;
    let remaining = app
        .world()
        .resource::<Garage>()
        .rig
        .as_ref()
        .unwrap()
        .electrical
        .remaining;
    let capacity = app
        .world()
        .resource::<Garage>()
        .rig
        .as_ref()
        .unwrap()
        .electrical
        .capacity;
    println!("Native drive: {start:?} -> {end:?}, battery {remaining}/{capacity}");
    assert!(
        Vec2::new(end.x - start.x, end.z - start.z).length() > 0.2,
        "native wiring did not drive the robot"
    );
    assert!(remaining < capacity && remaining > 0.0);
    let states = app
        .world()
        .resource::<Garage>()
        .bodies
        .iter()
        .map(|e| app.world().get::<Transform>(*e).unwrap().translation)
        .collect::<Vec<_>>();
    assert!(states.iter().all(|p| p.is_finite() && p.y > -1.0));
    assert_eq!(
        ra2_formats::bot::encode(&app.world().resource::<Editor>().document.robot).unwrap(),
        original
    );
    let before_turn = app.world().get::<Transform>(body).unwrap().rotation;
    let turn_samples = robot
        .controllers
        .iter()
        .map(|c| {
            c.keys
                .iter()
                .map(|k| f32::from(k.get(k.len().saturating_sub(2)) == Some(&30)))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    {
        let mut g = app.world_mut().resource_mut::<Garage>();
        g.commands = g.controls.evaluate(&robot, &turn_samples);
    }
    let mut turned = 0_f32;
    for _ in 0..180 {
        app.update();
        turned = turned
            .max(before_turn.angle_between(app.world().get::<Transform>(body).unwrap().rotation));
    }
    // A near-complete revolution can end close to the initial orientation.
    assert!(turned > 0.15, "differential steering did not turn");
    let actuator = app.world().resource::<Garage>().bodies[1];
    let before_fire = app.world().get::<Transform>(actuator).unwrap().rotation;
    let fire_samples = robot
        .controllers
        .iter()
        .map(|c| {
            c.keys
                .iter()
                .map(|k| f32::from(k.get(k.len().saturating_sub(2)) == Some(&57)))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    {
        let mut g = app.world_mut().resource_mut::<Garage>();
        g.commands = g.controls.evaluate(&robot, &fire_samples);
    }
    for _ in 0..36 {
        app.update();
    }
    let after_fire = app.world().get::<Transform>(actuator).unwrap().rotation;
    assert!(
        before_fire.angle_between(after_fire) > 0.1,
        "burst weapon did not fire"
    );
    // Without electrical supply the same wired inputs cannot power a motor.
    // Passive tyre torque is applied externally against the ground.
    {
        let mut g = app.world_mut().resource_mut::<Garage>();
        g.rig.as_mut().unwrap().electrical.remaining = 0.0;
    }
    for _ in 0..3 {
        app.update();
    }
    {
        let w = app.world_mut();
        let mut q = w.query::<(&drive::Motor, &ImpulseJoint)>();
        for (_, j) in q.iter(w) {
            assert!(
                j.data
                    .as_ref()
                    .motor(JointAxis::AngX)
                    .is_none_or(|m| m.max_force == 0.0)
            );
        }
    }
    println!(
        "GARAGE PHYSICS PASSED: native motors/wiring, wheel traction, supply depletion, isolated world, editor document unchanged"
    );
}

#[test]
#[ignore = "requires a local RA2 installation"]
fn native_strikers_hold_the_configured_pose_without_joint_frame_correction() {
    for path in [
        "AI/Team2/Bot2.bot",
        "AI/Team3/Bot1.bot",
        "AI/Team13/Bot2.bot",
        "AI/Team6/Bot1.bot",
        "Teams/Team0/Bot0.bot",
        "AI/Team5/Bot2.bot",
    ] {
        let (mut app, robot) = native_garage(path);
        let context = app.world().resource::<Garage>().context;
        app.world_mut()
            .get_mut::<RapierConfiguration>(context)
            .unwrap()
            .gravity = Vec3::ZERO;
        app.update();
        let bodies = app.world().resource::<Garage>().bodies.clone();
        app.world_mut()
            .entity_mut(bodies[0])
            .insert(RigidBody::Fixed);
        let initial: Vec<_> = bodies
            .iter()
            .map(|&e| *app.world().get::<Transform>(e).unwrap())
            .collect();
        let rig = app.world().resource::<Garage>().rig.as_ref().unwrap();
        for j in rig.joints.iter().filter(|j| j.kind == ActuatorKind::Burst) {
            let stroke: f32 = robot.components[j.component].settings["stop_angle"]
                .parse()
                .unwrap();
            assert_eq!(j.start, 0.0);
            assert!((j.stop - stroke).abs() < 0.00001);
        }
        for _ in 0..90 {
            app.update();
        }
        for (&entity, before) in bodies.iter().zip(initial) {
            let after = app.world().get::<Transform>(entity).unwrap();
            assert!(
                before.rotation.angle_between(after.rotation) < 0.015,
                "{}: solver reoriented body {:?}: {:?} -> {:?}",
                robot.name,
                entity,
                before.rotation,
                after.rotation
            );
            assert!(
                before.translation.distance(after.translation) < 0.01,
                "{}: solver moved body {:?}",
                robot.name,
                entity
            );
        }
        println!("{}: native rest pose held by solver", robot.name);
    }
}

#[test]
#[ignore = "native calibration measurements"]
fn native_actuation_profile() {
    for path in [
        "AI/Team0/Bot0.bot",
        "Teams/Team0/Bot0.bot",
        "AI/Team13/Bot2.bot",
        "Teams/Team0/Bot4.bot",
        "Teams/Team0/Bot5.bot",
    ] {
        let (mut app, robot) = native_garage(path);
        for _ in 0..120 {
            app.update();
        }
        let bodies = app.world().resource::<Garage>().bodies.clone();
        let start = app.world().get::<Transform>(bodies[0]).unwrap().translation;
        let samples: Vec<_> = robot
            .controllers
            .iter()
            .map(|c| {
                c.keys
                    .iter()
                    .map(|k| f32::from(k[k.len() - 2] == 17))
                    .collect()
            })
            .collect();
        {
            let mut g = app.world_mut().resource_mut::<Garage>();
            g.commands = g.controls.evaluate(&robot, &samples);
        }
        for _ in 0..240 {
            app.update();
        }
        let end = app.world().get::<Transform>(bodies[0]).unwrap().translation;
        assert!(
            end.z - start.z < -1.,
            "{}: W should drive forward with the native wiring: {:?}",
            robot.name,
            end - start
        );
        println!(
            "PROFILE {} drive delta={:?} distance={} velocity={:?}",
            robot.name,
            end - start,
            (end - start).length(),
            app.world().get::<Velocity>(bodies[0]).unwrap()
        );
        let rig = app.world().resource::<Garage>().rig.as_ref().unwrap();
        println!(
            "Mass sum={} supply={:?} air={:?}",
            rig.bodies.iter().map(|b| b.mass).sum::<f32>(),
            rig.electrical,
            rig.pneumatic
        );
        for (i, j) in rig.joints.iter().enumerate() {
            println!(
                "Joint {i} {j:?} saved={:?}",
                robot.components[j.component].settings
            );
        }
        // Remove the room as well: lifting a tall weapon into its ceiling
        // would measure an obstructed strike instead of the configured stroke.
        let environment = app.world().resource::<Garage>().environment.clone();
        for entity in environment {
            app.world_mut().despawn(entity);
        }
        // Fix the chassis and remove gravity to measure a complete unobstructed stroke.
        let context = app.world().resource::<Garage>().context;
        app.world_mut()
            .get_mut::<RapierConfiguration>(context)
            .unwrap()
            .gravity = Vec3::ZERO;
        app.world_mut()
            .entity_mut(bodies[0])
            .insert(RigidBody::Fixed);
        for &body in &bodies {
            app.world_mut()
                .get_mut::<Transform>(body)
                .unwrap()
                .translation
                .y += 5.;
            *app.world_mut().get_mut::<Velocity>(body).unwrap() = Velocity::zero();
        }
        for _ in 0..5 {
            app.update();
        }
        let samples: Vec<_> = robot
            .controllers
            .iter()
            .map(|c| {
                c.keys
                    .iter()
                    .map(|k| f32::from(k[k.len() - 2] == 57))
                    .collect()
            })
            .collect();
        {
            let mut g = app.world_mut().resource_mut::<Garage>();
            g.commands = g.controls.evaluate(&robot, &samples);
        }
        let mut peaks = std::collections::BTreeMap::<usize, f32>::new();
        let mut endpoint = std::collections::BTreeMap::<usize, f32>::new();
        let mut completed = std::collections::BTreeMap::<usize, f32>::new();
        for step in 0..180 {
            app.update();
            let world = app.world();
            let g = world.resource::<Garage>();
            let rig = g.rig.as_ref().unwrap();
            for (i, j) in
                rig.joints.iter().enumerate().filter(|(_, j)| {
                    matches!(j.kind, ActuatorKind::Burst | ActuatorKind::BurstPiston)
                })
            {
                let joint = world.get::<ImpulseJoint>(bodies[j.child]).unwrap();
                let angle = drive::joint_position(
                    world.get::<Transform>(bodies[j.parent]).unwrap(),
                    world.get::<Transform>(bodies[j.child]).unwrap(),
                    joint.data.as_ref(),
                    j.kind == ActuatorKind::BurstPiston,
                );
                let progress = (angle - j.start) / (j.stop - j.start);
                assert!(
                    (-0.03..=1.04).contains(&progress),
                    "{} joint {i}: escaped signed stroke: {progress}",
                    robot.name
                );
                let peak = peaks.entry(i).or_default();
                *peak = peak.max(progress);
                let time = step as f32 * crate::STEP as f32;
                if progress >= 0.95 {
                    endpoint.entry(i).or_insert(time);
                }
                if step > 2 && g.states[i].burst_time <= 0. {
                    completed.entry(i).or_insert(time);
                }
            }
        }
        let rig = app.world().resource::<Garage>().rig.as_ref().unwrap();
        for (i, j) in rig
            .joints
            .iter()
            .enumerate()
            .filter(|(_, j)| matches!(j.kind, ActuatorKind::Burst | ActuatorKind::BurstPiston))
        {
            assert!(
                peaks[&i] > 0.95,
                "{} joint {i}: incomplete strike {}",
                robot.name,
                peaks[&i]
            );
            let out = endpoint[&i];
            let cycle = completed[&i];
            assert!(
                out < 0.6 && cycle < 1.5,
                "{} joint {i}: slow stroke {out}, full cycle {cycle}",
                robot.name
            );
            if j.kind == ActuatorKind::Burst {
                assert!(
                    cycle - out > out,
                    "{} joint {i}: return should be slower than strike",
                    robot.name
                );
            }
            println!(
                "CYCLE {} joint {i}: strike {out:.3}s, return {:.3}s, peak {:.3}",
                robot.name,
                cycle - out,
                peaks[&i]
            );
        }
    }
}

#[test]
#[ignore = "requires original Walrus assets"]
fn native_walrus_completes_150_piston_cycles_without_exhausting_air() {
    let (mut app, robot) = native_garage("Teams/Team0/Bot5.bot");
    let original = ra2_formats::bot::encode(&robot).unwrap();
    app.update();
    let context = app.world().resource::<Garage>().context;
    app.world_mut()
        .get_mut::<RapierConfiguration>(context)
        .unwrap()
        .gravity = Vec3::ZERO;
    let bodies = app.world().resource::<Garage>().bodies.clone();
    app.world_mut()
        .entity_mut(bodies[0])
        .insert(RigidBody::Fixed);
    for &body in &bodies {
        app.world_mut()
            .get_mut::<Transform>(body)
            .unwrap()
            .translation
            .y += 5.;
    }
    for _ in 0..5 {
        app.update();
    }
    let (motor, spec) = app
        .world()
        .resource::<Garage>()
        .rig
        .as_ref()
        .unwrap()
        .joints
        .iter()
        .enumerate()
        .find(|(_, j)| j.kind == ActuatorKind::BurstPiston)
        .map(|(i, j)| (i, j.clone()))
        .unwrap();
    let initial_air = app
        .world()
        .resource::<Garage>()
        .rig
        .as_ref()
        .unwrap()
        .pneumatic
        .remaining;
    let mut longest = 0.;
    for cycle in 0..150 {
        let samples: Vec<_> = robot
            .controllers
            .iter()
            .map(|c| {
                c.keys
                    .iter()
                    .map(|k| f32::from(k[k.len() - 2] == 57))
                    .collect()
            })
            .collect();
        {
            let mut g = app.world_mut().resource_mut::<Garage>();
            g.commands = g.controls.evaluate(&robot, &samples);
        }
        let mut reached: f32 = 0.;
        let mut complete = false;
        for step in 0..180 {
            app.update();
            let w = app.world();
            let joint = w.get::<ImpulseJoint>(bodies[spec.child]).unwrap();
            let position = drive::joint_position(
                w.get::<Transform>(bodies[spec.parent]).unwrap(),
                w.get::<Transform>(bodies[spec.child]).unwrap(),
                joint.data.as_ref(),
                true,
            );
            reached = reached.max(position);
            if step > 2 && w.resource::<Garage>().states[motor].burst_time <= 0. {
                assert!(
                    position.abs() < 0.05,
                    "Walrus did not retract on cycle {cycle}: {position}"
                );
                longest = f32::max(longest, step as f32 * crate::STEP as f32);
                complete = true;
                break;
            }
        }
        assert!(complete, "Walrus cycle {cycle} timed out");
        assert!(
            reached > spec.stop - 0.06,
            "Walrus cycle {cycle} only extended {reached} of {}",
            spec.stop
        );
        let release: Vec<_> = robot
            .controllers
            .iter()
            .map(|c| vec![0.; c.keys.len()])
            .collect();
        {
            let mut g = app.world_mut().resource_mut::<Garage>();
            g.commands = g.controls.evaluate(&robot, &release);
        }
        app.update();
    }
    let remaining = app
        .world()
        .resource::<Garage>()
        .rig
        .as_ref()
        .unwrap()
        .pneumatic
        .remaining;
    assert!(remaining > 0. && remaining < initial_air);
    assert!(
        longest < 0.6,
        "Burst piston was too slow: {longest}s per cycle"
    );
    assert_eq!(
        ra2_formats::bot::encode(&app.world().resource::<Editor>().document.robot).unwrap(),
        original
    );
    println!(
        "WALRUS 150 CYCLES PASSED: air {initial_air} -> {remaining}; longest complete cycle {longest:.3}s"
    );
}

#[test]
#[ignore = "requires native articulated robot models"]
fn native_burst_physics_goals_match_editor_end_poses() {
    for path in [
        "Teams/Team0/Bot0.bot",
        "Teams/Team0/Bot4.bot",
        "AI/Team2/Bot2.bot",
        "AI/Team3/Bot1.bot",
        "AI/Team13/Bot2.bot",
    ] {
        let (mut app, robot) = native_garage(path);
        let context = app.world().resource::<Garage>().context;
        app.world_mut()
            .get_mut::<RapierConfiguration>(context)
            .unwrap()
            .gravity = Vec3::ZERO;
        app.update();
        let world = app.world();
        let garage = world.resource::<Garage>();
        let rig = garage.rig.as_ref().unwrap();
        let catalog = world.resource::<crate::editor::catalog::Catalog>();
        let start = catalog.freeze(&robot).unwrap();
        for joint in rig.joints.iter().filter(|j| j.kind == ActuatorKind::Burst) {
            let mut end_robot = robot.clone();
            let part = &mut end_robot.components[joint.component];
            let angle: f32 = part.settings["start_angle"].parse().unwrap();
            part.settings
                .insert("start_angle".into(), (angle + joint.stop).to_string());
            let end = catalog.freeze(&end_robot).unwrap();
            let socket = rig.attachment_bodies[joint.component]
                .iter()
                .position(|&b| b == joint.child)
                .unwrap();
            let native_start = (start.assembly.parts[joint.component].transform
                * start.models[joint.component].model.attachments[socket].transform)
                .to_scale_rotation_translation()
                .1;
            let native_end = (end.assembly.parts[joint.component].transform
                * end.models[joint.component].model.attachments[socket].transform)
                .to_scale_rotation_translation()
                .1;
            let data = world
                .get::<ImpulseJoint>(garage.bodies[joint.child])
                .unwrap();
            let parent = world.get::<Transform>(garage.bodies[joint.parent]).unwrap();
            let axis = parent.rotation * data.data.as_ref().local_basis1() * Vec3::X;
            let actual = Quat::from_axis_angle(axis, joint.stop);
            let expected = native_end * native_start.inverse();
            assert!(
                actual.angle_between(expected) < 0.001,
                "{}: physical stroke has the wrong sign or axis",
                robot.name
            );
        }
    }
}

#[test]
#[ignore = "requires user's ANUSAI robot; read only"]
fn anusai_test_bot_wheels_respond_to_native_wiring() {
    let (mut app, robot) = native_garage("Teams/Team1/Bot0.bot");
    let original = ra2_formats::bot::encode(&robot).unwrap();
    for _ in 0..240 {
        app.update();
    }
    let body = app.world().resource::<Garage>().bodies[0];
    let start = app.world().get::<Transform>(body).unwrap().translation;
    press_native_keys(&mut app, &robot, &[17]);
    let mut spin = 0_f32;
    for _ in 0..240 {
        app.update();
        let g = app.world().resource::<Garage>();
        for j in g
            .rig
            .as_ref()
            .unwrap()
            .joints
            .iter()
            .filter(|j| j.kind == ActuatorKind::Spin)
        {
            let p = app.world().get::<Velocity>(g.bodies[j.parent]).unwrap();
            let c = app.world().get::<Velocity>(g.bodies[j.child]).unwrap();
            spin = spin.max((c.angular - p.angular).length());
        }
    }
    let end = app.world().get::<Transform>(body).unwrap().translation;
    println!("ANUSAI TEST_BOT {start:?} -> {end:?}, max wheel spin {spin}");
    assert!(spin > 1., "wired wheel is stalled");
    assert!((end - start).xz().length() > 0.2, "robot did not drive");
    assert_eq!(
        ra2_formats::bot::encode(&app.world().resource::<Editor>().document.robot).unwrap(),
        original
    );
}

#[test]
#[ignore = "requires native robot and garage assets"]
fn garage_wall_impacts_damage_hp_but_resting_contacts_do_not() {
    let (mut app, _) = native_garage("Teams/Team0/Bot0.bot");
    for _ in 0..360 {
        app.update();
    }
    assert_eq!(
        app.world().resource::<Garage>().damage.health.received,
        0.,
        "initial drop/rest caused damage"
    );
    let g = app.world().resource::<Garage>();
    let entities = g.bodies.clone();
    let context = g.context;
    let position = app
        .world()
        .get::<Transform>(entities[0])
        .unwrap()
        .translation;
    app.world_mut().spawn((
        RigidBody::Fixed,
        Collider::cuboid(10., 2., 0.2),
        Transform::from_translation(position - Vec3::Z * 3.),
        CollisionGroups::new(Group::GROUP_1, Group::ALL),
        RapierContextEntityLink(context),
    ));
    for e in entities {
        app.world_mut().get_mut::<Velocity>(e).unwrap().linear = Vec3::NEG_Z * 8.;
    }
    let mut shown = false;
    for _ in 0..360 {
        app.update();
        shown |= app
            .world()
            .resource::<Garage>()
            .damage
            .health
            .numbers
            .iter()
            .any(|n| n.age >= 0.15 && n.amount.round() > 0.);
    }
    let damage = app.world().resource::<Garage>().damage.health.received;
    println!("Wall impact actual HP lost {damage:.2}; number displayed={shown}");
    assert!(damage >= 5., "collision did not damage HP");
    assert!(shown);
    for _ in 0..360 {
        app.update();
    }
    let after = app.world().resource::<Garage>().damage.health.received;
    assert!(
        (after - damage).abs() < 0.01,
        "persistent/resting contact repeats damage"
    );
}

#[test]
#[ignore = "requires native robot parts"]
fn garage_destroyed_wheel_detaches_once_without_mutating_blueprint() {
    for path in ["Teams/Team0/Bot0.bot", "AI/Team0/Bot2.bot"] {
        let (mut app, robot) = native_garage(path);
        for _ in 0..240 {
            app.update();
        }
        let wheel = robot
            .components
            .iter()
            .position(|c| c.class == "Wheel")
            .unwrap();
        let saved = ra2_formats::bot::encode(&robot).unwrap();
        let mass_before = app
            .world()
            .resource::<Garage>()
            .rig
            .as_ref()
            .unwrap()
            .bodies
            .iter()
            .map(|b| b.mass)
            .sum::<f32>();
        app.world_mut()
            .resource_mut::<Garage>()
            .damage
            .pending
            .push((wheel, 100000., Vec3::ZERO));
        for _ in 0..10 {
            app.update();
        }
        let g = app.world().resource::<Garage>();
        assert_eq!(g.damage.health.parts[wheel].current, 0.);
        assert!(!g.damage.health.operational(wheel));
        assert_eq!(g.damage.fragments.len(), 1);
        let fragment = g.damage.fragments[0];
        assert_eq!(
            app.world().get::<RigidBody>(fragment),
            Some(&RigidBody::Dynamic)
        );
        assert!(app.world().get::<Collider>(fragment).is_some());
        let mass_after = g
            .bodies
            .iter()
            .chain(&g.damage.fragments)
            .filter(|e| app.world().get::<Collider>(**e).is_some())
            .filter_map(|e| app.world().get::<ColliderMassProperties>(*e))
            .map(|m| match m {
                ColliderMassProperties::Mass(m) => *m,
                ColliderMassProperties::MassProperties(m) => m.mass,
                _ => 0.,
            })
            .sum::<f32>();
        println!("Detached wheel {wheel}; mass before {mass_before:.3} after {mass_after:.3}");
        assert!((mass_before - mass_after).abs() < 0.1);
        assert_eq!(
            ra2_formats::bot::encode(&app.world().resource::<Editor>().document.robot).unwrap(),
            saved
        );
    }
}

#[test]
#[ignore = "requires native articulated C.R.U.S.H. assembly"]
fn garage_broken_motor_drops_its_whole_branch_and_preserves_other_joints() {
    let (mut app, robot) = native_garage("Teams/Team0/Bot0.bot");
    for _ in 0..240 {
        app.update();
    }
    let root = robot
        .components
        .iter()
        .position(|c| c.class == "BurstMotor")
        .unwrap();
    let children: Vec<_> = robot
        .components
        .iter()
        .enumerate()
        .filter(|(_, c)| c.parent == Some(root))
        .map(|(i, _)| i)
        .collect();
    assert!(!children.is_empty());
    press_native_keys(&mut app, &robot, &[57]);
    for _ in 0..6 {
        app.update();
    }
    let g = app.world().resource::<Garage>();
    let before: Vec<_> = children
        .iter()
        .map(|&i| g.damage.health.parts[i].current)
        .collect();
    let before_joints = g
        .rig
        .as_ref()
        .unwrap()
        .joints
        .iter()
        .filter(|j| j.component != root)
        .map(|j| g.bodies[j.child])
        .collect::<Vec<_>>();
    app.world_mut()
        .resource_mut::<Garage>()
        .damage
        .pending
        .push((root, 10000., Vec3::ZERO));
    app.update();
    {
        let fragment = app.world().resource::<Garage>().damage.fragments[0];
        assert!(
            app.world()
                .get::<Velocity>(fragment)
                .unwrap()
                .angular
                .length()
                > 0.1,
            "detachment lost the moving hammer's rotation"
        );
    }
    for _ in 0..11 {
        app.update();
    }
    let g = app.world().resource::<Garage>();
    assert_eq!(
        g.damage.fragments.len(),
        1,
        "a single broken mount created multiple fragments"
    );
    for (&child, hp) in children.iter().zip(before) {
        assert_eq!(g.damage.health.parts[child].current, hp);
        assert!(!g.damage.health.parts[child].attached);
        assert!(
            !g.damage
                .bodies
                .iter()
                .flat_map(|b| &b.solids)
                .any(|s| s.component == child)
        );
    }
    for e in before_joints {
        assert!(
            app.world().get::<ImpulseJoint>(e).is_some(),
            "unrelated joint was broken"
        );
    }
    println!(
        "Broken Burst {root}: one rigid fragment, {} intact children; unrelated joints preserved",
        children.len()
    );
}

#[test]
#[ignore = "requires ANUSAI New Robot; read only"]
fn anusai_new_robot_accelerates_without_flipping_in_place() {
    for key in [17, 31] {
        let (mut app, robot) = native_garage("Teams/Team1/Bot1.bot");
        let saved = ra2_formats::bot::encode(&robot).unwrap();
        app.update();
        let g = app.world().resource::<Garage>();
        let context = g.context;
        let environment = g.environment.clone();
        let floor = environment
            .iter()
            .find_map(|e| app.world().get::<Collider>(*e))
            .unwrap()
            .raw
            .compute_local_aabb()
            .mins
            .y;
        for e in environment {
            app.world_mut().despawn(e);
        }
        app.world_mut().spawn((
            RigidBody::Fixed,
            Collider::cuboid(1000., 0.5, 1000.),
            Transform::from_xyz(0., floor - 0.5, 0.),
            Friction::coefficient(0.3),
            CollisionGroups::new(Group::GROUP_1, Group::ALL),
            RapierContextEntityLink(context),
        ));
        for _ in 0..240 {
            app.update();
        }
        let g = app.world().resource::<Garage>();
        let body = g.bodies[0];
        let start = *app.world().get::<Transform>(body).unwrap();
        let props = app.world().get::<ReadMassProperties>(body).unwrap();
        println!(
            "NEW ROBOT {} key={key} mass {:.3}, hull COM {:?}, inertia {:?}",
            robot.name,
            g.rig
                .as_ref()
                .unwrap()
                .bodies
                .iter()
                .map(|b| b.mass)
                .sum::<f32>(),
            props.local_center_of_mass,
            props.principal_inertia
        );
        let mut minimum_up = 1_f32;
        let mut max_speed = 0_f32;
        let mut max_spin = 0_f32;
        for step in 0..960 {
            if step == 0 {
                press_native_keys(&mut app, &robot, &[key]);
            }
            if step == 360 {
                press_native_keys(&mut app, &robot, &[if key == 17 { 31 } else { 17 }]);
            }
            app.update();
            let pose = app.world().get::<Transform>(body).unwrap();
            let v = app.world().get::<Velocity>(body).unwrap();
            minimum_up = minimum_up.min((pose.rotation * Vec3::Y).y);
            max_speed = max_speed.max(v.linear.xz().length());
            max_spin = max_spin.max(v.angular.length());
            if step % 120 == 0 {
                println!(
                    "ACCEL {:.2}s position {:?} velocity {:?} up {:.3}",
                    step as f32 / 120.,
                    pose.translation,
                    v,
                    (pose.rotation * Vec3::Y).y
                );
            }
        }
        let end = app.world().get::<Transform>(body).unwrap().translation;
        println!(
            "ACCEL RESULT key={key} displacement {:.3} speed {max_speed:.3} spin {max_spin:.3} minimum_up {minimum_up:.3}",
            (end - start.translation).xz().length()
        );
        assert!(
            minimum_up > 0.8,
            "fast four-wheel robot flipped on launch/reversal"
        );
        assert!(max_speed > 8., "stability change made a fast robot slow");
        assert!(
            (end - start.translation).xz().length() > 15.,
            "robot failed to drive across arena-sized distance"
        );
        assert_eq!(
            ra2_formats::bot::encode(&app.world().resource::<Editor>().document.robot).unwrap(),
            saved
        );
    }
}

#[test]
#[ignore = "requires native garage walls/floor and ANUSAI robot"]
fn driven_robots_take_damage_against_original_shared_wall_floor_mesh() {
    for (file, key) in [("Teams/Team0/Bot0.bot", 31), ("Teams/Team1/Bot1.bot", 17)] {
        let (mut app, robot) = native_garage(file);
        for _ in 0..240 {
            app.update();
        }
        let body = app.world().resource::<Garage>().bodies[0];
        press_native_keys(&mut app, &robot, &[key]);
        let initial_positions = app
            .world()
            .resource::<Garage>()
            .damage
            .shell
            .as_ref()
            .unwrap()
            .geometry
            .positions
            .clone();
        let mut speed = 0_f32;
        let mut popup = false;
        for _ in 0..720 {
            app.update();
            speed = speed.max(
                app.world()
                    .get::<Velocity>(body)
                    .unwrap()
                    .linear
                    .xz()
                    .length(),
            );
            popup |= app
                .world()
                .resource::<Garage>()
                .damage
                .health
                .numbers
                .iter()
                .any(|n| n.age >= 0.15 && n.amount >= 0.5);
        }
        let g = app.world().resource::<Garage>();
        println!(
            "DRIVEN WALL {} speed {speed:.3} received {:.3} hull {:.3}/{:.3} popup {popup}",
            robot.name,
            g.damage.health.received,
            g.damage.health.parts[0].current,
            g.damage.health.parts[0].maximum
        );
        assert!(
            g.damage.health.received >= 1.,
            "real garage wall impact did not damage {}",
            robot.name
        );
        assert!(popup, "no damage number for real garage wall hit");
        assert!(
            g.damage.health.parts[0].current < g.damage.health.parts[0].maximum,
            "hull was not damaged"
        );
        let shell = g.damage.shell.as_ref().unwrap();
        assert!(shell.revision > 0, "wall impact did not dent hull");
        assert_ne!(shell.geometry.positions, initial_positions);
        assert!(!shell.meshes.is_empty());
        let assets = app.world().resource::<Assets<Mesh>>();
        for handle in &shell.meshes {
            let mesh = assets.get(handle).unwrap();
            let positions = mesh
                .attribute(Mesh::ATTRIBUTE_POSITION)
                .unwrap()
                .as_float3()
                .unwrap();
            assert_eq!(positions.len(), shell.geometry.triangles.len() * 3);
            assert!(positions.iter().all(|p| Vec3::from_array(*p).is_finite()));
            for (triangle, actual) in shell
                .geometry
                .triangles
                .iter()
                .zip(positions.as_chunks::<3>().0)
            {
                assert_eq!(
                    triangle
                        .nodes
                        .map(|i| shell.geometry.positions[i].to_array()),
                    *actual
                );
            }
        }
        press_native_keys(&mut app, &robot, &[]);
        for _ in 0..360 {
            app.update();
        }
        let received = app.world().resource::<Garage>().damage.health.received;
        for _ in 0..360 {
            app.update();
        }
        assert!(
            (app.world().resource::<Garage>().damage.health.received - received).abs() < 0.001,
            "resting against wall kept causing damage"
        );
        assert_eq!(
            ra2_formats::bot::encode(&app.world().resource::<Editor>().document.robot).unwrap(),
            ra2_formats::bot::encode(&robot).unwrap()
        );
    }
}

#[test]
#[ignore = "requires original garage and stock/user robot files"]
fn garage_wall_damage_speed_sweep() {
    for file in ["Teams/Team0/Bot0.bot", "Teams/Team1/Bot1.bot"] {
        for speed in [2., 5., 10., 20., 30.] {
            let (mut app, robot) = native_garage(file);
            for _ in 0..240 {
                app.update();
            }
            let g = app.world().resource::<Garage>();
            let bodies = g.bodies.clone();
            let wall_z = g
                .environment
                .iter()
                .filter_map(|e| app.world().get::<Collider>(*e))
                .map(|c| c.raw.compute_local_aabb().maxs.z)
                .fold(f32::NEG_INFINITY, f32::max);
            let front = bodies
                .iter()
                .filter_map(|e| {
                    let c = app.world().get::<Collider>(*e)?;
                    let t = app.world().get::<Transform>(*e)?;
                    Some(
                        c.raw
                            .compute_aabb(&bevy_rapier3d::rapier::math::Pose::from_parts(
                                t.translation,
                                t.rotation,
                            ))
                            .maxs
                            .z,
                    )
                })
                .fold(f32::NEG_INFINITY, f32::max);
            // Measure damage at a known short run-up. Passive tyre drag now
            // slows a released robot; a distant wall also measures stopping distance.
            for body in &bodies {
                app.world_mut()
                    .get_mut::<Transform>(*body)
                    .unwrap()
                    .translation
                    .z += wall_z - front - 0.08;
            }
            app.update();
            for body in bodies {
                app.world_mut().get_mut::<Velocity>(body).unwrap().linear = Vec3::Z * speed;
            }
            let mut peak = 0_f32;
            for _ in 0..420 {
                app.update();
                let h = &app.world().resource::<Garage>().damage.health;
                peak = h.numbers.iter().map(|n| n.amount).fold(peak, f32::max);
            }
            let h = &app.world().resource::<Garage>().damage.health;
            println!(
                "WALL SWEEP {} speed {speed:.1} peak {peak:.3} total {:.3}",
                robot.name, h.received
            );
            assert!(peak >= 1., "wall damage disappeared");
            assert!(
                peak <= 2. * speed,
                "{}: wall damage outside calibrated native order at {speed}: {peak}",
                robot.name
            );
            if speed == 20. {
                assert!(
                    (20. ..30.).contains(&peak),
                    "fast wall strike should be a few tens of HP"
                );
            }
            assert!(
                h.parts[0].current > 0.,
                "a single wall strike destroyed the hull"
            );
        }
    }
}

#[test]
#[ignore = "requires native practice arena display groups"]
fn garage_display_walls_doors_and_fence_align_with_collision_shell() {
    let (mut app, _) = native_garage("AI/Team0/Bot0.bot");
    app.update();
    let root = &app
        .world()
        .resource::<crate::editor::catalog::Catalog>()
        .root;
    let raw = ra2_formats::ModelImporters::with_ra2()
        .decode(&std::fs::read(root.join("Arenas/practice arena/practice_arena.gmf")).unwrap())
        .unwrap()
        .model;
    let model = ra2_formats::scene::display_groups(&raw).unwrap();
    let flatten_again = ra2_formats::scene::display_groups(&model).unwrap();
    for (a, b) in model.objects.iter().zip(&flatten_again.objects) {
        assert_eq!(a.transform, b.transform, "group transform applied twice");
    }
    let bounds = |name: &str| {
        let p = model.object(name).unwrap().positions().collect::<Vec<_>>();
        (
            p.iter().copied().reduce(Vec3::min).unwrap(),
            p.iter().copied().reduce(Vec3::max).unwrap(),
        )
    };
    let (shell_min, shell_max) = bounds("h_box");
    let (wall_min, wall_max) = bounds("building");
    let (door_min, door_max) = bounds("Plane05");
    let (_, fence_max) = bounds("fence");
    println!(
        "ALIGNED garage shell {shell_min:?}..{shell_max:?}; walls {wall_min:?}..{wall_max:?}; door {door_min:?}..{door_max:?}; fence {fence_max:?}"
    );
    assert!((wall_max.z - shell_max.z).abs() < 0.03);
    assert!((wall_min.z - shell_min.z).abs() < 0.03);
    assert!(
        (door_min.z - shell_min.z).abs() < 0.25,
        "door rendered metres in front of collision"
    );
    assert!((fence_max.x - shell_max.x).abs() < 0.03);
    let g = app.world().resource::<Garage>();
    let collider = g
        .environment
        .iter()
        .find_map(|e| app.world().get::<Collider>(*e))
        .unwrap();
    for (direction, expected) in [
        (Vec3::Z, shell_max.z),
        (Vec3::NEG_Z, -shell_min.z),
        (Vec3::X, shell_max.x),
        (Vec3::NEG_X, -shell_min.x),
    ] {
        let distance = collider
            .cast_ray(Vec3::ZERO, Quat::IDENTITY, Vec3::Y, direction, 100., false)
            .unwrap();
        assert!((distance - expected).abs() < 0.001);
    }
}

#[test]
#[ignore = "requires native Scout and practice arena"]
fn forward_heading_rotates_whole_garage_robot_and_preserves_wiring() {
    for heading in [
        0.,
        std::f32::consts::FRAC_PI_2,
        std::f32::consts::PI,
        std::f32::consts::TAU,
    ] {
        let (mut app, robot) = native_garage("AI/Team0/Bot0.bot");
        app.world_mut()
            .resource_mut::<Editor>()
            .document
            .robot
            .heading = heading;
        app.update();
        let body = app.world().resource::<Garage>().bodies[0];
        let initial = *app.world().get::<Transform>(body).unwrap();
        let expected = Quat::from_rotation_y(heading);
        assert!(
            initial.rotation.abs_diff_eq(expected, 0.001)
                || initial.rotation.abs_diff_eq(-expected, 0.001)
        );
        for _ in 0..240 {
            app.update();
        }
        let start = app.world().get::<Transform>(body).unwrap().translation;
        press_native_keys(&mut app, &robot, &[17]);
        for _ in 0..90 {
            app.update();
        }
        let delta = app.world().get::<Transform>(body).unwrap().translation - start;
        println!("HEADING {heading:.3}, driven Scout {delta:?}");
        assert!(
            delta.dot(expected * Vec3::NEG_Z) > 0.4,
            "heading does not agree with forward drive"
        );
        let mut result = app.world().resource::<Editor>().document.robot.clone();
        result.heading = robot.heading;
        assert_eq!(
            ra2_formats::bot::encode(&result).unwrap(),
            ra2_formats::bot::encode(&robot).unwrap()
        );
    }
}

#[test]
#[ignore = "requires original robots; measures moving and stationary steering"]
fn native_stationary_and_driving_turns() {
    for file in [
        "Teams/Team0/Bot0.bot",
        "AI/Team0/Bot0.bot",
        "Teams/Team0/Bot5.bot",
        "AI/Team0/Bot2.bot",
    ] {
        for turn in [30, 32] {
            for moving in [false, true] {
                let (mut app, robot) = native_garage(file);
                app.update();
                let g = app.world().resource::<Garage>();
                let context = g.context;
                let env = g.environment.clone();
                let floor = env
                    .iter()
                    .find_map(|e| app.world().get::<Collider>(*e))
                    .unwrap()
                    .raw
                    .compute_local_aabb()
                    .mins
                    .y;
                for e in env {
                    app.world_mut().despawn(e);
                }
                app.world_mut().spawn((
                    RigidBody::Fixed,
                    Collider::cuboid(200., 0.5, 200.),
                    Transform::from_xyz(0., floor - 0.5, 0.),
                    Friction::coefficient(0.5),
                    CollisionGroups::new(Group::GROUP_1, Group::ALL),
                    RapierContextEntityLink(context),
                ));
                for _ in 0..240 {
                    app.update();
                }
                let body = app.world().resource::<Garage>().bodies[0];
                if moving {
                    press_native_keys(&mut app, &robot, &[17]);
                    for _ in 0..240 {
                        app.update();
                    }
                }
                let start = *app.world().get::<Transform>(body).unwrap();
                let start_speed = app
                    .world()
                    .get::<Velocity>(body)
                    .unwrap()
                    .linear
                    .xz()
                    .length();
                let mut keys = vec![turn];
                if moving || robot.components.iter().any(|c| c.class == "Steering") {
                    keys.push(17);
                }
                press_native_keys(&mut app, &robot, &keys);
                let mut previous = start.rotation * Vec3::NEG_Z;
                let mut yaw = 0_f32;
                let mut up = 1_f32;
                for _ in 0..360 {
                    app.update();
                    let t = app.world().get::<Transform>(body).unwrap();
                    let f = t.rotation * Vec3::NEG_Z;
                    yaw += previous
                        .cross(f)
                        .y
                        .atan2(previous.x * f.x + previous.z * f.z);
                    previous = f;
                    up = up.min((t.rotation * Vec3::Y).y);
                }
                let v = app.world().get::<Velocity>(body).unwrap();
                let distance = (app.world().get::<Transform>(body).unwrap().translation
                    - start.translation)
                    .xz()
                    .length();
                println!(
                    "MOVING TURN {} moving={moving} key={turn} yaw={yaw:.3} initial_speed={start_speed:.2} end_speed={:.2} distance={distance:.2} up={up:.3}",
                    robot.name,
                    v.linear.xz().length()
                );
                assert!(
                    if turn == 30 { yaw > 0.8 } else { yaw < -0.8 },
                    "{}: steering stalled or reversed (moving={moving}): {yaw}",
                    robot.name
                );
                assert!(up > 0.9, "{}: steering tipped chassis", robot.name);
                if moving {
                    assert!(start_speed > 1., "moving test did not accelerate");
                    assert!(v.linear.xz().length() > 0.3, "turning stopped the robot");
                }
            }
        }
    }
}

#[test]
#[ignore = "requires native Scout; ground sensor and wheel material regression"]
fn powered_wheels_need_ground_and_grip_to_accelerate_the_assembly() {
    for airborne in [false, true] {
        let (mut app, robot) = native_garage("AI/Team0/Bot0.bot");
        for _ in 0..240 {
            app.update();
        }
        let g = app.world().resource::<Garage>();
        let bodies = g.bodies.clone();
        let context = g.context;
        if airborne {
            app.world_mut()
                .get_mut::<RapierConfiguration>(context)
                .unwrap()
                .gravity = Vec3::ZERO;
            for entity in &bodies {
                app.world_mut()
                    .get_mut::<Transform>(*entity)
                    .unwrap()
                    .translation
                    .y += 10.;
            }
        } else {
            for entity in &bodies {
                for tread in &mut app.world_mut().get_mut::<tyres::Tyres>(*entity).unwrap().0 {
                    tread.grip = 0.;
                }
            }
        }
        for _ in 0..30 {
            app.update();
        }
        for entity in &bodies {
            *app.world_mut().get_mut::<Velocity>(*entity).unwrap() = Velocity::zero();
        }
        press_native_keys(&mut app, &robot, &[17]);
        for _ in 0..240 {
            app.update();
        }
        let mut momentum = Vec3::ZERO;
        let mut mass = 0_f32;
        let mut spin = 0_f32;
        for entity in &bodies {
            let m = app.world().get::<ReadMassProperties>(*entity).unwrap().mass;
            let velocity = app.world().get::<Velocity>(*entity).unwrap();
            momentum += velocity.linear * m;
            mass += m;
            spin = spin.max(velocity.angular.length());
        }
        let speed = (momentum / mass).xz().length();
        println!(
            "TRACTION airborne={airborne} centre-of-mass speed={speed:.6}, wheel spin={spin:.3}"
        );
        assert!(spin > 1., "motor did not run in traction test");
        assert!(speed < 0.05, "ungrounded/slippery tyres applied propulsion");
    }
}

#[test]
#[ignore = "requires native C.R.U.S.H.; wheel rotation versus actual travel"]
fn grounded_wheels_stop_with_robot_after_release_and_against_wall() {
    for blocked in [false, true] {
        let (mut app, robot) = native_garage("Teams/Team0/Bot0.bot");
        app.update();
        let g = app.world().resource::<Garage>();
        let context = g.context;
        let bodies = g.bodies.clone();
        if !blocked {
            let env = g.environment.clone();
            let floor = env
                .iter()
                .find_map(|e| app.world().get::<Collider>(*e))
                .unwrap()
                .raw
                .compute_local_aabb()
                .mins
                .y;
            for e in env {
                app.world_mut().despawn(e);
            }
            app.world_mut().spawn((
                RigidBody::Fixed,
                Collider::cuboid(200., 0.5, 200.),
                Transform::from_xyz(0., floor - 0.5, 0.),
                Friction::coefficient(0.5),
                CollisionGroups::new(Group::GROUP_1, Group::ALL),
                RapierContextEntityLink(context),
            ));
        }
        for _ in 0..240 {
            app.update();
        }
        press_native_keys(&mut app, &robot, &[17]);
        for _ in 0..if blocked { 840 } else { 240 } {
            app.update();
        }
        if !blocked {
            press_native_keys(&mut app, &robot, &[]);
            for _ in 0..1200 {
                app.update();
            }
        }
        let root_velocity = app.world().get::<Velocity>(bodies[0]).unwrap();
        let mut spin = 0_f32;
        let g = app.world().resource::<Garage>();
        for j in
            g.rig.as_ref().unwrap().joints.iter().filter(|j| {
                j.kind == ActuatorKind::Spin && !g.damage.bodies[j.child].wheels.is_empty()
            })
        {
            let parent = app.world().get::<Velocity>(bodies[j.parent]).unwrap();
            let child = app.world().get::<Velocity>(bodies[j.child]).unwrap();
            spin = spin.max((child.angular - parent.angular).length());
        }
        println!(
            "WHEEL STOP blocked={blocked}, chassis speed {:.5}, wheel spin {spin:.5}",
            root_velocity.linear.xz().length()
        );
        assert!(
            root_velocity.linear.xz().length() < 0.1,
            "fixture did not stop"
        );
        assert!(spin < 0.3, "stopped robot's wheels kept rotating");
    }
}

#[test]
#[ignore = "requires native C.R.U.S.H.; visible axle angle versus wheel travel"]
fn wheel_rotation_matches_travel_in_reverse_and_tank_turns() {
    let (mut app, robot) = native_garage("Teams/Team0/Bot0.bot");
    app.update();
    let g = app.world().resource::<Garage>();
    let context = g.context;
    let env = g.environment.clone();
    let floor = env
        .iter()
        .find_map(|e| app.world().get::<Collider>(*e))
        .unwrap()
        .raw
        .compute_local_aabb()
        .mins
        .y;
    for e in env {
        app.world_mut().despawn(e);
    }
    app.world_mut().spawn((
        RigidBody::Fixed,
        Collider::cuboid(200., 0.5, 200.),
        Transform::from_xyz(0., floor - 0.5, 0.),
        Friction::coefficient(0.5),
        CollisionGroups::new(Group::GROUP_1, Group::ALL),
        RapierContextEntityLink(context),
    ));
    for _ in 0..240 {
        app.update();
    }
    let g = app.world().resource::<Garage>();
    let wheels = g
        .rig
        .as_ref()
        .unwrap()
        .joints
        .iter()
        .filter(|j| j.kind == ActuatorKind::Spin && !g.damage.bodies[j.child].wheels.is_empty())
        .map(|j| {
            (
                g.bodies[j.child],
                g.bodies[j.parent],
                g.damage.bodies[j.child].wheels[0].1,
            )
        })
        .collect::<Vec<_>>();
    let sample = |app: &App, child, parent| {
        let joint = app
            .world()
            .get::<ImpulseJoint>(child)
            .unwrap()
            .data
            .as_ref();
        let p = app.world().get::<Transform>(parent).unwrap();
        let c = app.world().get::<Transform>(child).unwrap();
        (
            p.transform_point(joint.local_anchor1()),
            p.rotation * joint.local_basis1() * Vec3::X,
            drive::joint_position(p, c, joint, false),
            p.rotation,
        )
    };
    for keys in [
        &[17][..],
        &[31][..],
        &[30][..],
        &[32][..],
        &[17, 30][..],
        &[17, 32][..],
    ] {
        press_native_keys(&mut app, &robot, keys);
        for _ in 0..120 {
            app.update();
        }
        let mut previous = wheels
            .iter()
            .map(|(c, p, _)| sample(&app, *c, *p))
            .collect::<Vec<_>>();
        let mut actual = vec![0_f32; wheels.len()];
        let mut expected = actual.clone();
        for _ in 0..120 {
            app.update();
            for (i, (child, parent, radius)) in wheels.iter().enumerate() {
                let now = sample(&app, *child, *parent);
                let before = previous[i];
                let axis = (now.1 + before.1).normalize();
                let forward = axis.cross(Vec3::Y).normalize();
                let parent_angle = (now.3 * before.3.inverse()).to_scaled_axis().dot(axis);
                actual[i] += (now.2 - before.2 + std::f32::consts::PI)
                    .rem_euclid(std::f32::consts::TAU)
                    - std::f32::consts::PI
                    + parent_angle;
                expected[i] += (now.0 - before.0).dot(forward) / radius;
                previous[i] = now;
            }
        }
        for (i, (a, e)) in actual.iter().zip(&expected).enumerate() {
            println!("ROLLING keys{keys:?} wheel{i}: measured angle {a:.3}, travel/radius {e:.3}");
            assert!(
                (a - e).abs() < 0.5 + e.abs() * 0.05,
                "wheel angle does not match ground travel"
            );
        }
    }
}

#[test]
#[ignore = "requires user's native TEST_BOT; read only"]
fn piston_spikes_damage_afk_hull_without_taxing_attacker_hull() {
    use bevy::ecs::system::RunSystemOnce;
    let (mut app, robot) = native_garage("Teams/Team1/Bot0.bot");
    let saved = ra2_formats::bot::encode(&robot).unwrap();
    assert!(robot.name.eq_ignore_ascii_case("TEST_BOT"));
    app.update();
    let (context, ground, environment) = {
        let g = app.world().resource::<Garage>();
        (g.context, g.placement.unwrap(), g.environment.clone())
    };
    for entity in environment {
        app.world_mut().despawn(entity);
    }
    app.world_mut().spawn((
        RigidBody::Fixed,
        Collider::cuboid(100., 0.5, 100.),
        Transform::from_xyz(0., ground.translation.y - 0.5, 0.),
        Friction::coefficient(0.5),
        CollisionGroups::new(Group::GROUP_1, Group::ALL),
        RapierContextEntityLink(context),
    ));
    for _ in 0..180 {
        app.update();
    }
    let (parent, child, stop) = {
        let g = app.world().resource::<Garage>();
        let rig = g.rig.as_ref().unwrap();
        let j = rig
            .joints
            .iter()
            .find(|j| {
                j.kind == ActuatorKind::BurstPiston
                    && g.damage.bodies[j.child]
                        .solids
                        .iter()
                        .any(|s| s.profile.piercing > 0.)
            })
            .expect("Native piston has no piercing output");
        (g.bodies[j.parent], g.bodies[j.child], j.stop)
    };
    let joint = app.world().get::<ImpulseJoint>(child).unwrap();
    let direction = app.world().get::<Transform>(parent).unwrap().rotation
        * joint.data.as_ref().local_basis1()
        * Vec3::X
        * stop.signum();
    let direction = Vec3::new(direction.x, 0., direction.z).normalize();
    let target = robot.clone();
    let mut placement = ground;
    placement.translation += direction * 8.;
    let opponent = app
        .world_mut()
        .run_system_once(
            move |mut commands: Commands,
                  catalog: Res<crate::editor::catalog::Catalog>,
                  mut meshes: ResMut<Assets<Mesh>>,
                  mut materials: ResMut<Assets<StandardMaterial>>,
                  mut images: ResMut<Assets<Image>>| {
                let mut r = RobotInstance::new(1, context, default());
                r.group = Group::GROUP_3;
                r.placement = Some(placement);
                spawn::robot(
                    &mut commands,
                    &mut r,
                    &target,
                    &catalog,
                    &mut meshes,
                    &mut materials,
                    &mut images,
                )
                .unwrap();
                r.active = true;
                r.running = true;
                r
            },
        )
        .unwrap();
    app.insert_resource(crate::battle::Session {
        robots: vec![opponent],
        ..default()
    });
    for _ in 0..180 {
        app.update();
    }
    let (tip, centre) = {
        let g = app.world().resource::<Garage>();
        let mut end = f32::NEG_INFINITY;
        let mut centre = Vec3::ZERO;
        let mut count = 0.;
        for (body, parts) in g.bodies.iter().zip(&g.damage.bodies) {
            let pose = app.world().get::<Transform>(*body).unwrap();
            for s in parts.solids.iter().filter(|s| s.profile.piercing > 0.) {
                let world =
                    pose.to_matrix() * Mat4::from_rotation_translation(s.rotation, s.position);
                let aabb =
                    s.collider
                        .raw
                        .compute_aabb(&bevy_rapier3d::rapier::math::Pose::from_parts(
                            world.w_axis.truncate(),
                            Quat::from_mat4(&world),
                        ));
                let c = (aabb.mins + aabb.maxs) * 0.5;
                let half = (aabb.maxs - aabb.mins) * 0.5;
                end = end.max(c.dot(direction) + half.dot(direction.abs()));
                centre += c;
                count += 1.;
            }
        }
        (end, centre / count)
    };
    let (bodies, shift) = {
        let r = &app.world().resource::<crate::battle::Session>().robots[0];
        let root = app.world().get::<Transform>(r.bodies[0]).unwrap();
        let hull = r.damage.bodies[0]
            .solids
            .iter()
            .find(|s| s.component == 0)
            .unwrap();
        let world =
            root.to_matrix() * Mat4::from_rotation_translation(hull.rotation, hull.position);
        let aabb = hull
            .collider
            .raw
            .compute_aabb(&bevy_rapier3d::rapier::math::Pose::from_parts(
                world.w_axis.truncate(),
                Quat::from_mat4(&world),
            ));
        let c = (aabb.mins + aabb.maxs) * 0.5;
        let half = (aabb.maxs - aabb.mins) * 0.5;
        let near = c.dot(direction) - half.dot(direction.abs());
        let delta = centre - c;
        let lateral = delta - direction * delta.dot(direction);
        (
            r.bodies.clone(),
            Vec3::new(lateral.x, 0., lateral.z) + direction * (tip + 0.18 - near),
        )
    };
    for body in bodies {
        app.world_mut()
            .get_mut::<Transform>(body)
            .unwrap()
            .translation += shift;
        *app.world_mut().get_mut::<Velocity>(body).unwrap() = Velocity::zero();
    }
    for _ in 0..30 {
        app.update();
    }
    let snapshots = app
        .world()
        .resource::<Garage>()
        .bodies
        .iter()
        .chain(&app.world().resource::<crate::battle::Session>().robots[0].bodies)
        .map(|e| (*e, *app.world().get::<Transform>(*e).unwrap()))
        .collect::<Vec<_>>();
    let before = [
        app.world().resource::<Garage>().damage.health.parts[0].current,
        app.world().resource::<crate::battle::Session>().robots[0]
            .damage
            .health
            .parts[0]
            .current,
    ];
    let mut tagged = false;
    for _ in 0..3 {
        press_native_keys(&mut app, &robot, &[]);
        for (e, t) in &snapshots {
            *app.world_mut().get_mut::<Transform>(*e).unwrap() = *t;
            *app.world_mut().get_mut::<Velocity>(*e).unwrap() = Velocity::zero();
        }
        for _ in 0..30 {
            app.update();
        }
        press_native_keys(&mut app, &robot, &[57]);
        for _ in 0..150 {
            app.update();
            tagged |= app.world().resource::<crate::battle::Session>().robots[0]
                .damage
                .health
                .numbers
                .iter()
                .any(|n| n.source == Some(0));
        }
        press_native_keys(&mut app, &robot, &[]);
        for _ in 0..120 {
            app.update();
        }
    }
    let own = &app.world().resource::<Garage>().damage.health;
    let other = &app.world().resource::<crate::battle::Session>().robots[0]
        .damage
        .health;
    let loss = [
        before[0] - own.parts[0].current,
        before[1] - other.parts[0].current,
    ];
    println!(
        "TEST_BOT 3 native space-button strokes: hull loss attacker={} AFK={}, all-part HP {} vs {}, source-tagged={tagged}",
        loss[0], loss[1], own.received, other.received
    );
    assert!(loss[1] > 5., "Piston spikes did not damage AFK hull");
    assert!(
        loss[0] < loss[1] * 0.1 + 0.5,
        "Attacker hull still taxed for weapon contact: {loss:?}"
    );
    assert!(tagged, "Hit lost its source for green damage feedback");
    assert!(
        own.received < other.received * 0.5,
        "Total weapon/attachment HP still mirrors target HP: {} vs {}",
        own.received,
        other.received
    );
    assert_eq!(
        ra2_formats::bot::encode(&app.world().resource::<Editor>().document.robot).unwrap(),
        saved
    );
}

#[test]
#[ignore = "requires native CRUSH and Stingray models; read only"]
fn crush_hammer_contact_does_not_mirror_hp_into_the_tool() {
    use bevy::ecs::system::RunSystemOnce;
    let (mut app, _) = native_garage("Teams/Team0/Bot0.bot");
    app.update();
    let (context, mut placement, root) = {
        let g = app.world().resource::<Garage>();
        (g.context, g.placement.unwrap(), g.bodies[0])
    };
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../RobotArena2/Teams/Team0/Bot1.bot");
    let target = ra2_formats::bot::decode(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(target.name.to_ascii_lowercase(), "sting ray");
    placement.translation.x += 8.;
    let opponent = app
        .world_mut()
        .run_system_once(
            move |mut commands: Commands,
                  catalog: Res<crate::editor::catalog::Catalog>,
                  mut meshes: ResMut<Assets<Mesh>>,
                  mut materials: ResMut<Assets<StandardMaterial>>,
                  mut images: ResMut<Assets<Image>>| {
                let mut r = RobotInstance::new(1, context, default());
                r.group = Group::GROUP_3;
                r.placement = Some(placement);
                spawn::robot(
                    &mut commands,
                    &mut r,
                    &target,
                    &catalog,
                    &mut meshes,
                    &mut materials,
                    &mut images,
                )
                .unwrap();
                r.active = true;
                r.running = true;
                r
            },
        )
        .unwrap();
    app.insert_resource(crate::battle::Session {
        robots: vec![opponent],
        ..default()
    });
    for _ in 0..180 {
        app.update();
    }
    // Real native compound shapes and HP, aligned for an isolated head strike.
    // Inject velocity, never HP: this exercises the full Rapier -> HP path.
    let bounds = |world: &World, body: Entity, s: &damage::Solid| {
        let pose = world.get::<Transform>(body).unwrap();
        let world = pose.to_matrix() * Mat4::from_rotation_translation(s.rotation, s.position);
        s.collider
            .raw
            .compute_aabb(&bevy_rapier3d::rapier::math::Pose::from_parts(
                world.w_axis.truncate(),
                Quat::from_mat4(&world),
            ))
    };
    let (head, head_bounds) = {
        let g = app.world().resource::<Garage>();
        g.bodies
            .iter()
            .zip(&g.damage.bodies)
            .find_map(|(b, p)| {
                p.solids
                    .iter()
                    .find(|s| s.tool)
                    .map(|s| (*b, bounds(app.world(), *b, s)))
            })
            .expect("CRUSH has no weapon collision")
    };
    assert_ne!(head, root);
    let (others, shift) = {
        let r = &app.world().resource::<crate::battle::Session>().robots[0];
        let hull = r.damage.bodies[0]
            .solids
            .iter()
            .find(|s| s.component == 0)
            .unwrap();
        let aabb = bounds(app.world(), r.bodies[0], hull);
        let hc = (head_bounds.mins + head_bounds.maxs) * 0.5;
        let tc = (aabb.mins + aabb.maxs) * 0.5;
        (
            r.bodies.clone(),
            Vec3::new(
                head_bounds.maxs.x + 0.002 - aabb.mins.x,
                hc.y - tc.y,
                hc.z - tc.z,
            ),
        )
    };
    for e in others {
        app.world_mut().get_mut::<Transform>(e).unwrap().translation += shift;
        *app.world_mut().get_mut::<Velocity>(e).unwrap() = Velocity::zero();
    }
    let own = app.world().resource::<Garage>().bodies.clone();
    for e in own {
        *app.world_mut().get_mut::<Velocity>(e).unwrap() = Velocity::linear(Vec3::X * 8.);
    }
    let before = [
        app.world().resource::<Garage>().damage.health.received,
        app.world().resource::<crate::battle::Session>().robots[0]
            .damage
            .health
            .received,
    ];
    for _ in 0..12 {
        app.update();
        if app.world().resource::<crate::battle::Session>().robots[0]
            .damage
            .health
            .received
            > before[1]
        {
            break;
        }
    }
    let g = app.world().resource::<Garage>();
    let target = &app.world().resource::<crate::battle::Session>().robots[0];
    let loss = [
        g.damage.health.received - before[0],
        target.damage.health.received - before[1],
    ];
    println!(
        "CRUSH hammer -> passive Stingray: total HP loss attacker={} target={}",
        loss[0], loss[1]
    );
    assert!(loss[1] > 1., "Hammer missed: {loss:?}");
    assert!(
        loss[0] < loss[1] * 0.5,
        "Hammer damage still mirrored: {loss:?}"
    );
    assert!(
        target
            .damage
            .health
            .numbers
            .iter()
            .any(|n| n.source == Some(0))
    );
}
