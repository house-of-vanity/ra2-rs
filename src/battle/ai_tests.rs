use super::{Phase, Session, ai};
use crate::{
    config::GarageConfig,
    frontend::{Navigation, Page},
    gameplay::{RobotBody, RobotInstance, damage::BodyParts, drive},
    garage::{Garage, tests::native_garage},
};
use bevy::ecs::system::RunSystemOnce;
use bevy::prelude::*;
use bevy_rapier3d::prelude::*;

#[test]
#[ignore = "requires original Scout and garage assets (RA2_GAME_DIR)"]
fn scout_ai_drives_toward_opponent_headlessly() {
    check_approach("AI/Team0/Bot0.bot", "front", Vec2::new(0., 1.));
}

#[test]
#[ignore = "requires original assets; known failure: Scout stalls on bridge approach"]
fn scout_navigates_from_lower_floor_towards_bridge() {
    let (mut app, _) = native_garage("AI/Team0/Bot0.bot");
    app.update();
    let root = std::path::PathBuf::from(std::env::var_os("RA2_GAME_DIR").unwrap());
    let descriptor = crate::content::arenas::load(&root)
        .unwrap()
        .into_iter()
        .find(|a| a.key == "bridge")
        .unwrap();
    let (context, bodies) = {
        let garage = app.world().resource::<Garage>();
        (garage.context, garage.bodies.clone())
    };
    let old_environment: Vec<Entity> = app
        .world_mut()
        .query::<(Entity, &Name)>()
        .iter(app.world())
        .filter(|(_, name)| name.as_str() == "Garage walls and floor")
        .map(|(entity, _)| entity)
        .collect();
    for entity in old_environment {
        if let Ok(e) = app.world_mut().get_entity_mut(entity) {
            e.despawn();
        }
    }
    let loaded = app
        .world_mut()
        .run_system_once(
            move |mut commands: Commands,
                  catalog: Res<crate::editor::catalog::Catalog>,
                  mut meshes: ResMut<Assets<Mesh>>,
                  mut materials: ResMut<Assets<StandardMaterial>>,
                  mut images: ResMut<Assets<Image>>| {
                super::arena::spawn(
                    &mut commands,
                    context,
                    false,
                    &descriptor,
                    &catalog,
                    &GarageConfig::default(),
                    &mut meshes,
                    &mut materials,
                    &mut images,
                )
            },
        )
        .unwrap()
        .unwrap();
    let initial = app.world().get::<Transform>(bodies[0]).unwrap().translation;
    let start = Vec3::new(0., -1.95, 6.5);
    for &body in &bodies {
        app.world_mut()
            .get_mut::<Transform>(body)
            .unwrap()
            .translation += start - initial;
        *app.world_mut().get_mut::<Velocity>(body).unwrap() = Velocity::zero();
    }
    let goal = Vec3::new(0., -0.2, 0.);
    let target_body = app
        .world_mut()
        .spawn((RobotBody, Transform::from_translation(goal)))
        .id();
    let mut scout = std::mem::replace(
        &mut app.world_mut().resource_mut::<Garage>().instance,
        RobotInstance::new(0, context, GarageConfig::default()),
    );
    scout.id = 2;
    scout.running = true;
    let mut target = RobotInstance::new(1, context, GarageConfig::default());
    target.running = true;
    target.bodies.push(target_body);
    target.damage.bodies.push(BodyParts::default());
    app.insert_resource(Session {
        context,
        phase: Phase::Playing,
        robots: vec![target, scout],
        arena_state: loaded.state,
        environment: loaded.entities,
        ..default()
    });
    app.world_mut().resource_mut::<Navigation>().page = Page::Match;
    app.add_systems(FixedUpdate, ai::update.before(drive::motors));
    let mut best = start.distance(goal);
    let mut highest = start.y;
    for _ in 0..(crate::STEP.recip() as usize * 120) {
        app.update();
        let position = app.world().get::<Transform>(bodies[0]).unwrap().translation;
        best = best.min(position.distance(goal));
        highest = highest.max(position.y);
    }
    assert!(
        best < 2.5 && highest > start.y + 0.5,
        "Scout did not climb from lower bridge floor: closest={best:.2}, highest={highest:.2}"
    );
}

fn check_approach(path: &str, direction: &str, offset: Vec2) {
    let (mut app, robot) = native_garage(path);
    // Let the native assembly spawn and settle before measuring its movement.
    for _ in 0..240 {
        app.update();
    }
    let (context, body) = {
        let garage = app.world().resource::<Garage>();
        (garage.context, garage.bodies[0])
    };
    let start = *app.world().get::<Transform>(body).unwrap();
    let forward = (start.rotation * Vec3::NEG_Z).with_y(0.).normalize();
    let left = Vec3::Y.cross(forward);
    let target_position = start.translation + (left * offset.x + forward * offset.y) * 4.0;
    let target_body = app
        .world_mut()
        .spawn((RobotBody, Transform::from_translation(target_position)))
        .id();

    // Move the real assembly into the battle session; leave an inactive garage
    // owner so the existing shared motor and contact systems still have it.
    let mut scout = std::mem::replace(
        &mut app.world_mut().resource_mut::<Garage>().instance,
        RobotInstance::new(0, context, GarageConfig::default()),
    );
    scout.id = 2;
    scout.running = true;
    let mut opponent = RobotInstance::new(1, context, GarageConfig::default());
    opponent.running = true;
    opponent.bodies.push(target_body);
    opponent.damage.bodies.push(BodyParts::default());
    app.insert_resource(Session {
        context,
        phase: Phase::Playing,
        robots: vec![opponent, scout],
        ..default()
    });
    app.world_mut().resource_mut::<Navigation>().page = Page::Match;
    app.add_systems(FixedUpdate, ai::update.before(drive::motors));

    let initial_distance = start.translation.distance(target_position);
    let mut closest = initial_distance;
    let mut furthest = 0.0_f32;
    let mut final_position = start.translation;
    for _ in 0..(crate::STEP.recip() as usize * 5) {
        app.update();
        let position = app.world().get::<Transform>(body).unwrap().translation;
        assert!(position.is_finite(), "Scout position became non-finite");
        closest = closest.min(position.distance(target_position));
        furthest = furthest.max(position.distance(start.translation));
        final_position = position;
    }
    println!(
        "{} ({path}), {direction}: initial={initial_distance:.3} m closest={closest:.3} m max_displacement={furthest:.3} m final={final_position:?}",
        robot.name,
    );
    assert!(
        furthest > 0.25,
        "{} ({direction}) did not move: {furthest:.3} m",
        robot.name
    );
    assert!(
        closest < initial_distance - 0.25,
        "{} ({direction}) did not approach its opponent: {initial_distance:.3} -> {closest:.3} m; start {:?}, target {target_position:?}, final {final_position:?}, max displacement {furthest:.3} m",
        robot.name,
        start.translation,
    );
}

macro_rules! approach_test {
    ($name:ident, $path:literal, $direction:literal, $x:expr, $y:expr) => {
        #[test]
        #[ignore = "requires original robot and garage assets (RA2_GAME_DIR)"]
        fn $name() {
            check_approach($path, $direction, Vec2::new($x, $y));
        }
    };
}

approach_test!(scout_ai_left, "AI/Team0/Bot0.bot", "left", 1., 0.);
approach_test!(scout_ai_right, "AI/Team0/Bot0.bot", "right", -1., 0.);
approach_test!(scout_ai_behind, "AI/Team0/Bot0.bot", "behind", 0., -1.);
approach_test!(crush_ai_front, "Teams/Team0/Bot0.bot", "front", 0., 1.);
approach_test!(crush_ai_left, "Teams/Team0/Bot0.bot", "left", 1., 0.);
approach_test!(crush_ai_right, "Teams/Team0/Bot0.bot", "right", -1., 0.);
approach_test!(crush_ai_behind, "Teams/Team0/Bot0.bot", "behind", 0., -1.);
approach_test!(sting_ray_ai_front, "Teams/Team0/Bot1.bot", "front", 0., 1.);
approach_test!(sting_ray_ai_left, "Teams/Team0/Bot1.bot", "left", 1., 0.);
approach_test!(sting_ray_ai_right, "Teams/Team0/Bot1.bot", "right", -1., 0.);
approach_test!(
    sting_ray_ai_behind,
    "Teams/Team0/Bot1.bot",
    "behind",
    0.,
    -1.
);

#[test]
#[ignore = "requires original Scout, Sting Ray and garage assets (RA2_GAME_DIR)"]
fn native_ai_robots_approach_and_contact_each_other() {
    let (mut app, _) = native_garage("AI/Team0/Bot0.bot");
    // The owner is embedded in the spawned colliders, so set it before Startup.
    app.world_mut().resource_mut::<Garage>().instance.id = 2;
    app.update();
    let (context, mut placement) = {
        let garage = app.world().resource::<Garage>();
        (garage.context, garage.placement.unwrap())
    };
    let root = std::env::var_os("RA2_GAME_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../RobotArena2"));
    let mut opponent =
        ra2_formats::bot::decode(&std::fs::read(root.join("Teams/Team0/Bot1.bot")).unwrap())
            .unwrap();
    assert_eq!(opponent.name.to_ascii_lowercase(), "sting ray");
    opponent.heading += std::f32::consts::PI;
    placement.translation += Vec3::NEG_Z * 5.0;
    let opponent = app
        .world_mut()
        .run_system_once(
            move |mut commands: Commands,
                  catalog: Res<crate::editor::catalog::Catalog>,
                  mut meshes: ResMut<Assets<Mesh>>,
                  mut materials: ResMut<Assets<StandardMaterial>>,
                  mut images: ResMut<Assets<Image>>| {
                let mut robot = RobotInstance::new(3, context, GarageConfig::default());
                robot.group = Group::GROUP_3;
                robot.placement = Some(placement);
                crate::gameplay::spawn::robot(
                    &mut commands,
                    &mut robot,
                    &opponent,
                    &catalog,
                    &mut meshes,
                    &mut materials,
                    &mut images,
                )
                .unwrap();
                robot.active = true;
                robot.running = true;
                robot
            },
        )
        .unwrap();
    let scout = std::mem::replace(
        &mut app.world_mut().resource_mut::<Garage>().instance,
        RobotInstance::new(0, context, GarageConfig::default()),
    );
    app.insert_resource(Session {
        context,
        robots: vec![scout, opponent],
        ..default()
    });
    for _ in 0..240 {
        app.update();
    }
    let bodies = {
        let session = app.world().resource::<Session>();
        [
            session.robots[0].bodies.clone(),
            session.robots[1].bodies.clone(),
        ]
    };
    let positions = |world: &World| {
        [
            world.get::<Transform>(bodies[0][0]).unwrap().translation,
            world.get::<Transform>(bodies[1][0]).unwrap().translation,
        ]
    };
    let start = positions(app.world());
    let baseline_damage = {
        let robots = &app.world().resource::<Session>().robots;
        [
            robots[0].damage.health.received,
            robots[1].damage.health.received,
        ]
    };
    let initial_distance = start[0].distance(start[1]);
    let mut closest = initial_distance;
    let mut movement = [0.0_f32; 2];
    let mut contact_ticks = 0;
    let mut attributed_damage = [false; 2];
    app.world_mut().resource_mut::<Navigation>().page = Page::Match;
    app.world_mut().resource_mut::<Session>().phase = Phase::Playing;
    app.add_systems(FixedUpdate, ai::update.before(drive::motors));
    for _ in 0..(crate::STEP.recip() as usize * 10) {
        app.update();
        let current = positions(app.world());
        assert!(
            current.iter().all(|p| p.is_finite()),
            "non-finite robot position"
        );
        closest = closest.min(current[0].distance(current[1]));
        for i in 0..2 {
            movement[i] = movement[i].max(current[i].distance(start[i]));
            attributed_damage[i] |= app.world().resource::<Session>().robots[i]
                .damage
                .health
                .numbers
                .iter()
                .any(|n| n.source == Some(if i == 0 { 3 } else { 2 }));
        }
        let world = app.world();
        let simulation = world.get::<RapierContextSimulation>(context).unwrap();
        let colliders = world.get::<RapierContextColliders>(context).unwrap();
        let rigid_bodies = world.get::<RapierRigidBodySet>(context).unwrap();
        if simulation
            .contact_pairs(colliders, rigid_bodies)
            .any(|pair| {
                pair.has_any_active_contact()
                    && pair
                        .collider1()
                        .zip(pair.collider2())
                        .is_some_and(|(a, b)| {
                            (bodies[0].contains(&a) && bodies[1].contains(&b))
                                || (bodies[0].contains(&b) && bodies[1].contains(&a))
                        })
            })
        {
            contact_ticks += 1;
        }
    }
    let robots = &app.world().resource::<Session>().robots;
    let damage = [
        robots[0].damage.health.received - baseline_damage[0],
        robots[1].damage.health.received - baseline_damage[1],
    ];
    println!(
        "Scout vs Sting Ray: initial={initial_distance:.3} m closest={closest:.3} m movement={movement:?} contact_ticks={contact_ticks} HP_loss={damage:?} opponent_attributed={attributed_damage:?}"
    );
    assert!(
        movement.iter().all(|&m| m > 0.25),
        "both AI robots must move: {movement:?}"
    );
    assert!(
        closest < initial_distance - 0.25,
        "robots did not approach: {initial_distance:.3} -> {closest:.3} m"
    );
    assert!(contact_ticks > 0, "robots did not physically contact");
    assert!(
        attributed_damage.iter().any(|&hit| hit),
        "no damage attributed to the opposing robot: HP loss {damage:?}"
    );
}
