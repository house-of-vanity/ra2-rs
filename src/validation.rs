use std::time::Duration;

use bevy::{diagnostic::DiagnosticsPlugin, prelude::*, time::TimeUpdateStrategy};
use bevy_rapier3d::prelude::*;

use crate::{
    MAX_OBJECTS, Options, STEP,
    scene::Scene,
    simulation::{
        Chassis, DriveInput, Home, LoadCube, LoadState, SceneActions, SimulationPlugin, Wheel,
    },
};

pub fn check_physics(options: Options, scene: Scene) {
    let initial = options.objects;
    let origin = scene.origin;
    let imported = scene.model.is_some();
    let mut app = App::new();
    app.insert_resource(options)
        .insert_resource(scene)
        .add_plugins((MinimalPlugins, TransformPlugin, DiagnosticsPlugin))
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
            STEP,
        )))
        .add_plugins((
            SimulationPlugin,
            RapierDiagnosticsPlugin::default().in_schedule(FixedUpdate),
        ));

    advance(&mut app, 180);
    let start = robot_transform(&mut app);
    assert!(
        (0.75..1.2).contains(&start.translation.y),
        "Robot did not settle on the arena floor: {:?}",
        start.translation
    );
    app.world_mut().resource_mut::<DriveInput>().throttle = 1.0;
    advance(&mut app, 120);
    let driven = robot_transform(&mut app);
    let speed = {
        let world = app.world_mut();
        world
            .query_filtered::<&Velocity, With<Chassis>>()
            .single(world)
            .unwrap()
            .linear
            .length()
    };
    assert!(
        start.translation.z - driven.translation.z > 0.5,
        "Wheel motors did not drive forwards: {:?} -> {:?}",
        start.translation,
        driven.translation
    );
    {
        let mut drive = app.world_mut().resource_mut::<DriveInput>();
        drive.throttle = 0.0;
        drive.turn = 1.0;
    }
    advance(&mut app, 180);
    let turned = robot_transform(&mut app);
    let yaw = (turned.rotation * Vec3::NEG_Z).angle_between(driven.rotation * Vec3::NEG_Z);
    assert!(yaw > 0.3, "Differential drive did not turn enough: {yaw}");

    let world = app.world_mut();
    let mut wheels = world.query_filtered::<(&Transform, &ImpulseJoint), With<Wheel>>();
    assert_eq!(wheels.iter(world).count(), 4);
    for (transform, joint) in wheels.iter(world) {
        let anchor = turned.transform_point(joint.data.as_ref().local_anchor1());
        assert!(
            transform.translation.distance(anchor) < 0.15,
            "Wheel detached from its axle"
        );
    }
    let mut cubes = world.query_filtered::<(&Transform, &Home), With<LoadCube>>();
    assert_eq!(cubes.iter(world).count(), initial);
    for (transform, home) in cubes.iter(world) {
        assert!(transform.translation.is_finite());
        assert!(
            transform.translation.y > 0.35,
            "Cube fell through the floor"
        );
        assert!(
            transform.translation.y < home.0.translation.y,
            "Gravity did not move a cube"
        );
    }

    app.world_mut().resource_mut::<SceneActions>().add = 100;
    advance(&mut app, 2);
    assert_eq!(
        app.world().resource::<LoadState>().objects.len(),
        (initial + 100).min(MAX_OBJECTS)
    );
    app.world_mut().resource_mut::<SceneActions>().remove = 100;
    advance(&mut app, 2);
    let expected = (initial + 100).min(MAX_OBJECTS).saturating_sub(100);
    assert_eq!(app.world().resource::<LoadState>().objects.len(), expected);
    app.world_mut().resource_mut::<SceneActions>().reset = true;
    advance(&mut app, 1);
    assert!(robot_transform(&mut app).translation.distance(origin) < 0.01);
    app.world_mut().resource_mut::<SceneActions>().clear = true;
    advance(&mut app, 2);
    assert!(app.world().resource::<LoadState>().objects.is_empty());
    let world = app.world_mut();
    assert_eq!(
        world
            .query_filtered::<Entity, With<LoadCube>>()
            .iter(world)
            .count(),
        0
    );
    if imported {
        let probe = app
            .world_mut()
            .spawn((
                RigidBody::Dynamic,
                Collider::ball(0.2),
                Transform::from_xyz(0.0, 3.0, 0.0),
                Velocity {
                    linear: Vec3::new(-80.0, 0.0, 0.0),
                    angular: Vec3::ZERO,
                },
                GravityScale(0.0),
                Ccd::enabled(),
                Restitution::coefficient(0.0),
            ))
            .id();
        let mut closest_x = 0.0_f32;
        for _ in 0..180 {
            advance(&mut app, 1);
            let position = app.world().get::<Transform>(probe).unwrap().translation;
            closest_x = closest_x.min(position.x);
            assert!(
                position.x > -12.72 && position.x < 12.72,
                "80 m/s probe crossed the original arena wall: {position:?}"
            );
        }
        assert!(
            closest_x < -12.2,
            "Probe did not reach the wall: {closest_x}"
        );
        println!(
            "PASS: original GMF floor and wall; 80 m/s CCD probe reached X={closest_x:.3} and stayed inside"
        );
    }
    println!(
        "PASS: {initial} cubes; gravity/collisions; 4 motorized wheel joints; forward {:.2} m, speed {speed:.2} m/s; turn {:.2} rad; add/remove/reset/clear",
        start.translation.z - driven.translation.z,
        yaw
    );
}

fn advance(app: &mut App, steps: usize) {
    for _ in 0..steps {
        app.update();
    }
}

fn robot_transform(app: &mut App) -> Transform {
    let world = app.world_mut();
    *world
        .query_filtered::<&Transform, With<Chassis>>()
        .single(world)
        .unwrap()
}
