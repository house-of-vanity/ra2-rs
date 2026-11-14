use bevy::prelude::*;
use bevy_rapier3d::prelude::*;

use crate::{
    MAX_OBJECTS, Options, STEP,
    scene::{self, Scene},
};

pub struct SimulationPlugin;

impl Plugin for SimulationPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Time::<Fixed>::from_seconds(STEP))
            .insert_resource(TimestepMode::Fixed {
                dt: STEP as f32,
                substeps: 1,
            })
            .init_resource::<DriveInput>()
            .init_resource::<SceneActions>()
            .init_resource::<LoadState>()
            .init_resource::<PhysicsTicks>()
            .add_plugins(RapierPhysicsPlugin::<NoUserData>::default().in_fixed_schedule())
            .add_systems(Startup, setup)
            .add_systems(Update, apply_actions)
            .add_systems(FixedUpdate, drive_motors.before(PhysicsSet::SyncBackend))
            .add_systems(FixedUpdate, count_tick.after(PhysicsSet::Writeback));
    }
}

#[derive(Resource, Default)]
pub struct DriveInput {
    pub throttle: f32,
    pub turn: f32,
}

#[derive(Resource, Default)]
pub struct SceneActions {
    pub add: usize,
    pub remove: usize,
    pub clear: bool,
    pub reset: bool,
    pub shake: bool,
}

#[derive(Resource, Default)]
pub struct LoadState {
    pub objects: Vec<Entity>,
    next_slot: usize,
}

#[derive(Resource, Default)]
pub struct PhysicsTicks(pub u64);

#[derive(Component)]
pub struct Chassis;

#[derive(Component)]
pub struct Wheel {
    pub side: f32,
}

#[derive(Component)]
pub struct LoadCube;

#[derive(Component, Clone, Copy)]
pub struct Home(pub Transform);

#[derive(Component, Clone, Copy)]
pub enum Visual {
    Floor,
    Wall,
    Chassis,
    Wheel,
    Cube(usize),
}

pub fn setup(
    mut commands: Commands,
    options: Res<Options>,
    scene: Res<Scene>,
    mut load: ResMut<LoadState>,
) {
    if scene.model.is_some() {
        scene::spawn_collision(&mut commands, &scene);
    } else {
        commands.spawn((
            RigidBody::Fixed,
            Collider::cuboid(scene.lab_half_size, 0.25, scene.lab_half_size),
            Transform::from_xyz(0.0, -0.25, 0.0),
            Friction::coefficient(1.0),
            Visual::Floor,
        ));
        for (position, angle) in [
            (Vec3::new(0.0, 1.0, -scene.lab_half_size), 0.0),
            (Vec3::new(0.0, 1.0, scene.lab_half_size), 0.0),
            (
                Vec3::new(-scene.lab_half_size, 1.0, 0.0),
                std::f32::consts::FRAC_PI_2,
            ),
            (
                Vec3::new(scene.lab_half_size, 1.0, 0.0),
                std::f32::consts::FRAC_PI_2,
            ),
        ] {
            commands.spawn((
                RigidBody::Fixed,
                Collider::cuboid(scene.lab_half_size, 1.0, 0.25),
                Transform::from_translation(position).with_rotation(Quat::from_rotation_y(angle)),
                Visual::Wall,
            ));
        }
    }

    let robot = &scene.config.robot;
    let origin = scene.origin;
    let transform = Transform::from_translation(origin);
    let chassis = commands
        .spawn((
            Chassis,
            RigidBody::Dynamic,
            Collider::cuboid(
                robot.chassis_size[0] / 2.0,
                robot.chassis_size[1] / 2.0,
                robot.chassis_size[2] / 2.0,
            ),
            ColliderMassProperties::Mass(robot.chassis_mass),
            transform,
            Home(transform),
            Velocity::zero(),
            Sleeping::disabled(),
            AdditionalSolverIterations(8),
            Ccd::enabled(),
            Damping {
                linear_damping: 0.05,
                angular_damping: 0.2,
            },
            Visual::Chassis,
        ))
        .id();

    for side in [-1.0, 1.0] {
        for axle in [-robot.wheel_base / 2.0, robot.wheel_base / 2.0] {
            let anchor = Vec3::new(side * robot.wheel_track / 2.0, robot.axle_height, axle);
            let transform = Transform::from_translation(origin + anchor);
            let joint = RevoluteJointBuilder::new(Vec3::X)
                .local_anchor1(anchor)
                .motor_velocity(0.0, robot.motor_damping)
                .motor_max_force(robot.motor_force)
                .contacts_enabled(false);
            commands.spawn((
                Wheel { side },
                RigidBody::Dynamic,
                // The body's frame stays aligned with the chassis; rotate only its shape.
                Collider::compound(vec![(
                    Vec3::ZERO,
                    Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
                    Collider::cylinder(robot.wheel_width / 2.0, robot.wheel_radius),
                )]),
                ColliderMassProperties::Mass(robot.wheel_mass),
                transform,
                Home(transform),
                Velocity::zero(),
                Sleeping::disabled(),
                Ccd::enabled(),
                Friction::coefficient(1.6),
                ImpulseJoint::new(chassis, joint),
                Visual::Wheel,
            ));
        }
    }
    add_cubes(&mut commands, &mut load, options.objects, &scene);
}

fn add_cubes(commands: &mut Commands, load: &mut LoadState, requested: usize, scene: &Scene) {
    let count = requested.min(MAX_OBJECTS.saturating_sub(load.objects.len()));
    for _ in 0..count {
        let slot = load.next_slot % MAX_OBJECTS;
        load.next_slot += 1;
        // Keep even the maximum load inside the imported arena's 25 x 25 x 16 m box.
        let imported = scene.model.is_some();
        let columns = if imported { 26 } else { 24 };
        let spacing = if imported { 0.95 } else { 1.3 };
        let transform = Transform::from_xyz(
            ((slot % columns) as f32 - (columns - 1) as f32 / 2.0) * spacing,
            if imported { 2.0 } else { 3.0 } + (slot / (columns * columns)) as f32 * spacing,
            if imported {
                -12.177705 + ((slot / columns) % columns) as f32 * spacing
            } else {
                12.0 - ((slot / columns) % columns) as f32 * 1.3
            },
        )
        .with_rotation(Quat::from_rotation_y((slot % 7) as f32 * 0.08));
        let entity = commands
            .spawn((
                LoadCube,
                RigidBody::Dynamic,
                Collider::cuboid(0.45, 0.45, 0.45),
                transform,
                Home(transform),
                Velocity::zero(),
                Sleeping::default(),
                Restitution::coefficient(0.15),
                Friction::coefficient(0.65),
                Damping {
                    linear_damping: 0.1,
                    angular_damping: 0.2,
                },
                Visual::Cube(slot % 4),
            ))
            .id();
        load.objects.push(entity);
    }
}

#[allow(
    clippy::type_complexity,
    reason = "ECS query explicitly lists the components affected by reset"
)]
pub fn apply_actions(
    mut commands: Commands,
    mut actions: ResMut<SceneActions>,
    mut load: ResMut<LoadState>,
    mut drive: ResMut<DriveInput>,
    scene: Res<Scene>,
    mut bodies: Query<(
        Entity,
        &Home,
        &mut Transform,
        &mut Velocity,
        &mut Sleeping,
        Has<LoadCube>,
        Has<crate::fleet::ImportedRobot>,
    )>,
) {
    if actions.clear {
        for entity in load.objects.drain(..) {
            commands.entity(entity).despawn();
        }
        load.next_slot = 0;
    } else {
        for _ in 0..actions.remove.min(load.objects.len()) {
            if let Some(entity) = load.objects.pop() {
                commands.entity(entity).despawn();
            }
        }
    }
    add_cubes(&mut commands, &mut load, actions.add, &scene);
    if actions.reset {
        *drive = DriveInput::default();
    }
    if actions.reset || actions.shake {
        for (entity, home, mut transform, mut velocity, mut sleeping, cube, robot) in &mut bodies {
            if actions.reset {
                *transform = home.0;
                *velocity = Velocity::zero();
            } else if cube || robot {
                let phase = (entity.index().index() % 17) as f32;
                velocity.linear += Vec3::new(phase.sin() * 2.0, 4.0, phase.cos() * 2.0);
                velocity.angular += Vec3::new(1.0, 0.7, -0.5);
            }
            sleeping.sleeping = false;
        }
    }
    *actions = SceneActions::default();
}

fn drive_motors(
    drive: Res<DriveInput>,
    scene: Res<Scene>,
    mut wheels: Query<(&Wheel, &mut ImpulseJoint)>,
) {
    let robot = &scene.config.robot;
    for (wheel, mut joint) in &mut wheels {
        let target = -(robot.motor_speed * drive.throttle
            + robot.turn_speed * drive.turn * wheel.side)
            .clamp(-robot.motor_speed, robot.motor_speed);
        let motor = joint
            .data
            .as_ref()
            .as_revolute()
            .expect("Wheel has a revolute joint")
            .motor();
        if motor.is_none_or(|motor| (motor.target_vel - target).abs() > 0.001) {
            joint
                .data
                .as_mut()
                .as_revolute_mut()
                .unwrap()
                .set_motor_velocity(target, robot.motor_damping);
        }
    }
}

fn count_tick(mut ticks: ResMut<PhysicsTicks>) {
    ticks.0 += 1;
}
