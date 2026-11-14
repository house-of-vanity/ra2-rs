use crate::{
    Options, STEP,
    fleet::{self, Fleet, FleetStats, ImportedRobot},
    scene::Scene,
    simulation::{SceneActions, SimulationPlugin},
};
use bevy::{diagnostic::DiagnosticsPlugin, prelude::*, time::TimeUpdateStrategy};
use bevy_rapier3d::prelude::*;
use std::time::Duration;

pub fn check(options: Options, scene: Scene, fleet: Fleet) {
    let expected = fleet.robots.len() * fleet.copies;
    let physical = options.bot_physics;
    let half = scene.lab_half_size;
    let mut app = App::new();
    app.insert_resource(options)
        .insert_resource(scene)
        .insert_resource(fleet)
        .add_plugins((MinimalPlugins, TransformPlugin, DiagnosticsPlugin))
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
            STEP,
        )))
        .init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<StandardMaterial>>()
        .init_resource::<Assets<Image>>()
        .add_plugins(SimulationPlugin)
        .add_systems(Startup, fleet::spawn.after(crate::simulation::setup));
    for _ in 0..360 {
        app.update();
    }
    {
        let world = app.world_mut();
        let mut robots = world.query::<(&ImportedRobot, &Transform, Option<&Velocity>)>();
        assert_eq!(robots.iter(world).count(), expected);
        for (_, pose, velocity) in robots.iter(world) {
            assert!(pose.translation.is_finite());
            assert!(
                pose.translation.x.abs() < half + 1.0
                    && pose.translation.z.abs() < half + 1.0
                    && pose.translation.y > -1.0
                    && pose.translation.y < 8.0,
                "Robot escaped arena: {pose:?}"
            );
            if physical {
                assert!(velocity.unwrap().linear.is_finite());
            }
        }
    }
    app.world_mut().resource_mut::<SceneActions>().shake = true;
    for _ in 0..120 {
        app.update();
    }
    app.world_mut().resource_mut::<SceneActions>().reset = true;
    app.update();
    let world = app.world_mut();
    let mut robots = world.query::<(&ImportedRobot, &Transform)>();
    let fleet = world.resource::<Fleet>();
    for (id, pose) in robots.iter(world) {
        let slot = id.copy * fleet.robots.len() + id.index;
        assert!(pose.translation.distance(fleet.placement(slot)) < 0.05);
    }
    let stats = world.resource::<FleetStats>();
    assert_eq!(stats.missing, 0, "Missing robot textures");
    println!(
        "PASS: {} robots, {} components, {} shared model assets, {} triangles; assembled bounds, collisions, shake/reset; physics {}",
        stats.robots, stats.components, stats.unique_models, stats.triangles, physical
    );
}
