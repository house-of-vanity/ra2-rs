use bevy::{
    core_pipeline::tonemapping::{DebandDither, Tonemapping},
    diagnostic::{DiagnosticPath, DiagnosticsStore, FrameTimeDiagnosticsPlugin},
    prelude::*,
    render::view::screenshot::{Screenshot, save_to_disk},
};
use bevy_rapier3d::prelude::{RapierDiagnosticsPlugin, Velocity};

use crate::{
    MAX_OBJECTS, Options,
    fleet::{Fleet, FleetStats, ImportedRobot},
    frontend::Navigation,
    scene::{self, ImportStats, Scene},
    settings::EngineSettings,
    simulation::{self, Chassis, DriveInput, LoadState, PhysicsTicks, SceneActions, Visual},
};

pub struct ViewPlugin;

impl Plugin for ViewPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ViewState>()
            .add_systems(Startup, setup.after(simulation::setup))
            .add_systems(Startup, scene::spawn_visuals)
            .add_systems(
                Update,
                (controls, smoke_test)
                    .chain()
                    .before(simulation::apply_actions),
            )
            .add_systems(
                Update,
                (attach_visuals, follow_camera).after(simulation::apply_actions),
            )
            .add_systems(Update, update_hud);
    }
}

#[derive(Resource)]
struct ViewState {
    follow: bool,
    selected_bot: Option<usize>,
}

impl Default for ViewState {
    fn default() -> Self {
        Self {
            follow: true,
            selected_bot: None,
        }
    }
}

#[derive(Resource)]
struct VisualAssets {
    floor: Handle<Mesh>,
    wall: Handle<Mesh>,
    chassis: Handle<Mesh>,
    wheel: Handle<Mesh>,
    cube: Handle<Mesh>,
    spoke: Handle<Mesh>,
    nose: Handle<Mesh>,
    floor_material: Handle<StandardMaterial>,
    wall_material: Handle<StandardMaterial>,
    chassis_material: Handle<StandardMaterial>,
    wheel_material: Handle<StandardMaterial>,
    accent_material: Handle<StandardMaterial>,
    cubes: Vec<Handle<StandardMaterial>>,
}

#[derive(Component)]
struct StatsText;
#[derive(Component)]
pub(crate) struct HudOverlay;
#[derive(Component)]
pub(crate) struct DemoCamera;

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    scene: Res<Scene>,
    options: Res<Options>,
    mut view_state: ResMut<ViewState>,
) {
    if options.all_bots {
        view_state.follow = false;
    }
    let robot = &scene.config.robot;
    let mut material = |r, g, b| {
        materials.add(StandardMaterial {
            base_color: Color::srgb(r, g, b),
            perceptual_roughness: 0.8,
            ..default()
        })
    };
    commands.insert_resource(VisualAssets {
        floor: meshes.add(Cuboid::new(
            scene.lab_half_size * 2.0,
            0.5,
            scene.lab_half_size * 2.0,
        )),
        wall: meshes.add(Cuboid::new(scene.lab_half_size * 2.0, 2.0, 0.5)),
        chassis: meshes.add(Cuboid::new(
            robot.chassis_size[0],
            robot.chassis_size[1],
            robot.chassis_size[2],
        )),
        wheel: meshes.add(
            Mesh::from(Cylinder::new(robot.wheel_radius, robot.wheel_width))
                .rotated_by(Quat::from_rotation_z(std::f32::consts::FRAC_PI_2)),
        ),
        cube: meshes.add(Cuboid::new(0.9, 0.9, 0.9)),
        spoke: meshes.add(Cuboid::new(0.53, 0.1, 1.05)),
        nose: meshes.add(Cuboid::new(0.9, 0.08, 0.4)),
        floor_material: material(0.19, 0.24, 0.29),
        wall_material: material(0.32, 0.39, 0.46),
        chassis_material: material(0.13, 0.65, 0.70),
        wheel_material: material(0.055, 0.06, 0.07),
        accent_material: material(1.0, 0.72, 0.14),
        cubes: vec![
            material(0.94, 0.35, 0.21),
            material(0.23, 0.62, 0.93),
            material(0.95, 0.74, 0.21),
            material(0.34, 0.81, 0.48),
        ],
    });
    commands.spawn((
        DirectionalLight {
            illuminance: 14_000.0,
            shadow_maps_enabled: true,
            ..default()
        },
        Transform::from_rotation(Quat::from_euler(EulerRot::XYZ, -0.9, -0.6, 0.0)),
    ));
    commands.insert_resource(GlobalAmbientLight {
        color: Color::WHITE,
        brightness: 400.0,
        ..default()
    });
    let camera = commands
        .spawn((
            Camera3d::default(),
            DemoCamera,
            Tonemapping::None,
            DebandDither::Disabled,
            Transform::from_translation(scene.origin + Vec3::new(7.0, 8.0, 10.0))
                .looking_at(scene.origin, Vec3::Y),
        ))
        .id();
    commands
        .spawn((
            HudOverlay,
            UiTargetCamera(camera),
            Node {
                position_type: PositionType::Absolute,
                left: px(16),
                top: px(16),
                padding: UiRect::all(px(14)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.025, 0.04, 0.065, 0.92)),
        ))
        .with_children(|parent| {
            parent.spawn((
                StatsText,
                Text::new("Waiting for simulation metrics..."),
                TextFont {
                    font_size: FontSize::Px(16.0),
                    ..default()
                },
                TextColor(Color::srgb(0.87, 0.94, 1.0)),
            ));
        });
    commands.spawn((
        HudOverlay,
        UiTargetCamera(camera),
        Text::new("WASD / arrows: test robot    F: overview    Tab: inspect bot\n+ / -: add / remove 100 cubes    Shift: 1000    C: clear\nSpace: shake bodies    R: reset    V: VSync    T: shadows    H: HUD    Esc: menu"),
        TextFont { font_size: FontSize::Px(15.0), ..default() },
        TextColor(Color::srgb(0.95, 0.96, 1.0)),
        Node { position_type: PositionType::Absolute, left: px(16), bottom: px(16), padding: UiRect::all(px(12)), ..default() },
        BackgroundColor(Color::srgba(0.025, 0.04, 0.065, 0.92)),
    ));
}

fn attach_visuals(
    mut commands: Commands,
    assets: Res<VisualAssets>,
    objects: Query<(Entity, &Visual), Without<Mesh3d>>,
) {
    for (entity, visual) in &objects {
        let (mesh, material) = match *visual {
            Visual::Floor => (&assets.floor, &assets.floor_material),
            Visual::Wall => (&assets.wall, &assets.wall_material),
            Visual::Chassis => (&assets.chassis, &assets.chassis_material),
            Visual::Wheel => (&assets.wheel, &assets.wheel_material),
            Visual::Cube(color) => (&assets.cube, &assets.cubes[color]),
        };
        commands
            .entity(entity)
            .insert((Mesh3d(mesh.clone()), MeshMaterial3d(material.clone())));
        if matches!(visual, Visual::Wheel | Visual::Chassis) {
            let wheel = matches!(visual, Visual::Wheel);
            commands.entity(entity).with_children(|parent| {
                parent.spawn((
                    Mesh3d(if wheel {
                        assets.spoke.clone()
                    } else {
                        assets.nose.clone()
                    }),
                    MeshMaterial3d(assets.accent_material.clone()),
                    Transform::from_translation(if wheel {
                        Vec3::ZERO
                    } else {
                        Vec3::new(0.0, 0.39, -1.4)
                    }),
                ));
            });
        }
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "Bevy injects independent ECS resources"
)]
fn controls(
    keys: Res<ButtonInput<KeyCode>>,
    options: Res<Options>,
    mut drive: ResMut<DriveInput>,
    mut actions: ResMut<SceneActions>,
    mut state: ResMut<ViewState>,
    fleet: Res<Fleet>,
    nav: Res<Navigation>,
    mut preferences: ResMut<EngineSettings>,
    mut exit: MessageWriter<crate::shutdown::ExitRequest>,
) {
    // Keep the benchmark camera, HUD and workload reproducible even if its window
    // receives keyboard input while the automated capture is running.
    if options.smoke_test {
        if keys.just_pressed(KeyCode::Escape) {
            exit.write(crate::shutdown::ExitRequest);
        }
        return;
    }
    if nav.visible() || options.menu_smoke_test {
        return;
    }
    if keys.just_pressed(KeyCode::KeyH) {
        preferences.diagnostics = !preferences.diagnostics;
    }
    let pressed = |a, b| {
        if keys.pressed(a) || keys.pressed(b) {
            1.0
        } else {
            0.0
        }
    };
    drive.throttle =
        pressed(KeyCode::KeyW, KeyCode::ArrowUp) - pressed(KeyCode::KeyS, KeyCode::ArrowDown);
    drive.turn =
        pressed(KeyCode::KeyA, KeyCode::ArrowLeft) - pressed(KeyCode::KeyD, KeyCode::ArrowRight);
    let amount = if keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight) {
        1000
    } else {
        100
    };
    if keys.just_pressed(KeyCode::Equal) || keys.just_pressed(KeyCode::NumpadAdd) {
        actions.add = amount;
    }
    if keys.just_pressed(KeyCode::Minus) || keys.just_pressed(KeyCode::NumpadSubtract) {
        actions.remove = amount;
    }
    if keys.just_pressed(KeyCode::Tab) && !fleet.robots.is_empty() {
        let total = fleet.robots.len() * fleet.copies;
        state.selected_bot = Some(state.selected_bot.map_or(0, |i| (i + 1) % total));
        state.follow = true;
    }
    actions.clear |= keys.just_pressed(KeyCode::KeyC);
    actions.reset |= keys.just_pressed(KeyCode::KeyR);
    actions.shake |= keys.just_pressed(KeyCode::Space);
    if keys.just_pressed(KeyCode::KeyF) {
        state.follow = !state.follow;
    }
    if keys.just_pressed(KeyCode::KeyV) {
        preferences.vsync = !preferences.vsync;
    }
    if keys.just_pressed(KeyCode::KeyT) {
        preferences.shadows = !preferences.shadows;
    }
}

#[allow(
    clippy::type_complexity,
    reason = "Disjoint ECS camera and body queries"
)]
fn follow_camera(
    time: Res<Time>,
    state: Res<ViewState>,
    scene: Res<Scene>,
    robot: Query<&Transform, With<Chassis>>,
    fleet: Res<Fleet>,
    bots: Query<(&ImportedRobot, &Transform), (Without<Camera3d>, Without<DemoCamera>)>,
    mut cameras: Query<&mut Transform, (With<DemoCamera>, Without<Chassis>)>,
) {
    let Ok(robot) = robot.single() else {
        return;
    };
    let inspected = state.selected_bot.and_then(|slot| {
        bots.iter()
            .find(|(id, _)| {
                id.index == slot % fleet.robots.len() && id.copy == slot / fleet.robots.len()
            })
            .map(|(_, pose)| *pose)
    });
    let focus = inspected.as_ref().unwrap_or(robot);
    let target = if state.follow && inspected.is_some() {
        focus.translation + Vec3::new(0.0, 0.4, 0.0) - Vec3::new(6.0, 0.0, -5.0).normalize() * 1.2
    } else if state.follow {
        focus.translation + focus.rotation * Vec3::new(0.0, 0.0, -2.0)
    } else {
        Vec3::ZERO
    };
    let offset = if inspected.is_some() && state.follow {
        Vec3::new(5.0, 4.0, 6.0)
    } else if state.follow {
        if scene.model.is_some() {
            Vec3::new(7.0, 8.0, 10.0)
        } else {
            Vec3::new(14.0, 13.0, 19.0)
        }
    } else {
        if scene.model.is_some() {
            Vec3::new(26.0, 30.0, 34.0)
        } else {
            Vec3::new(
                scene.lab_half_size * 1.22,
                scene.lab_half_size * 1.34,
                scene.lab_half_size * 1.47,
            )
        }
    };
    for mut camera in &mut cameras {
        let blend = 1.0 - (-5.0 * time.delta_secs()).exp();
        camera.translation = camera.translation.lerp(target + offset, blend);
        camera.look_at(target, Vec3::Y);
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "Bevy injects independent ECS resources into this system"
)]
fn update_hud(
    diagnostics: Res<DiagnosticsStore>,
    time: Res<Time<Real>>,
    ticks: Res<PhysicsTicks>,
    load: Res<LoadState>,
    state: Res<ViewState>,
    scene: Res<Scene>,
    imported: Res<ImportStats>,
    fleet: Res<Fleet>,
    fleet_stats: Res<FleetStats>,
    preferences: Res<EngineSettings>,
    options: Res<Options>,
    robot: Query<&Velocity, With<Chassis>>,
    mut texts: Query<&mut Text, With<StatsText>>,
    mut previous: Local<(f64, u64)>,
) {
    let elapsed = time.elapsed_secs_f64();
    let interval = elapsed - previous.0;
    if interval < 0.25 {
        return;
    }
    let measured = |path: &DiagnosticPath| {
        diagnostics
            .get(path)
            .and_then(|d| d.smoothed())
            .unwrap_or(0.0)
    };
    let count =
        |path: &DiagnosticPath| diagnostics.get(path).and_then(|d| d.value()).unwrap_or(0.0);
    let hz = (ticks.0 - previous.1) as f64 / interval;
    *previous = (elapsed, ticks.0);
    let speed = robot.single().map(|v| v.linear.length()).unwrap_or(0.0);
    let fleet_info = if !fleet.robots.is_empty() {
        let selected = state
            .selected_bot
            .map(|slot| {
                let r = &fleet.robots[slot % fleet.robots.len()];
                format!(
                    "Inspect: {} [{} / {}]\n{}",
                    r.definition.name,
                    slot + 1,
                    fleet_stats.robots,
                    r.source
                        .strip_prefix(&options.game_dir)
                        .unwrap_or(&r.source)
                        .display()
                )
            })
            .unwrap_or_else(|| "Tab: inspect robots | F: overview".into());
        format!(
            "Bots: {}  Components: {}\nShared models: {}  Load: {:.2}s\nRobot physics: {}\n{}\n\n",
            fleet_stats.robots,
            fleet_stats.components,
            fleet_stats.unique_models,
            fleet.load_seconds,
            if options.bot_physics {
                "welded compound bodies"
            } else {
                "render only"
            },
            selected
        )
    } else {
        String::new()
    };
    for mut text in &mut texts {
        text.0 = format!(
            "{}\n{}Meshes: {}  Triangles: {}\nTextures: {}  Missing: {}\n\nFPS             {:7.1}\nFrame avg       {:7.2} ms\nPhysics step    {:7.3} ms\n  Collision     {:7.3} ms\n  Solver        {:7.3} ms\nPhysics rate    {:7.1} / 120 Hz\n\nCubes           {:5} / {MAX_OBJECTS}\nBodies / awake  {:5.0} / {:.0}\nActive contacts {:5.0}\nJoints total    {:5.0}\nRobot speed     {:7.2} m/s\n\nVSync request: {}   Shadows: {}\nCamera: {}   {} build",
            scene.title,
            fleet_info,
            imported.meshes + fleet_stats.meshes,
            imported.triangles + fleet_stats.triangles,
            imported.textures + fleet_stats.textures,
            imported.missing + fleet_stats.missing,
            measured(&FrameTimeDiagnosticsPlugin::FPS),
            measured(&FrameTimeDiagnosticsPlugin::FRAME_TIME),
            measured(&RapierDiagnosticsPlugin::STEP_TIME),
            measured(&RapierDiagnosticsPlugin::COLLISION_DETECTION_TIME),
            measured(&RapierDiagnosticsPlugin::SOLVER_TIME),
            hz,
            load.objects.len(),
            count(&RapierDiagnosticsPlugin::RIGID_BODIES),
            count(&RapierDiagnosticsPlugin::ACTIVE_BODIES),
            count(&RapierDiagnosticsPlugin::ACTIVE_CONTACT_PAIRS),
            count(&RapierDiagnosticsPlugin::IMPULSE_JOINTS),
            speed,
            if preferences.vsync { "ON" } else { "OFF" },
            if preferences.shadows { "ON" } else { "OFF" },
            if state.follow { "follow" } else { "overview" },
            if cfg!(debug_assertions) {
                "Optimized dev"
            } else {
                "Release"
            },
        );
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "Bevy injects independent ECS resources into this system"
)]
fn smoke_test(
    options: Res<Options>,
    time: Res<Time<Real>>,
    mut stage: Local<u8>,
    mut actions: ResMut<SceneActions>,
    mut drive: ResMut<DriveInput>,
    mut state: ResMut<ViewState>,
    mut commands: Commands,
    mut exit: MessageWriter<crate::shutdown::ExitRequest>,
    overlays: Query<
        (&ComputedNode, &InheritedVisibility, &ComputedUiTargetCamera),
        With<HudOverlay>,
    >,
) {
    if !options.smoke_test {
        return;
    }
    let seconds = time.elapsed_secs();
    drive.throttle = if seconds < 2.8 { 0.6 } else { 0.0 };
    drive.turn = if seconds < 3.0 { 0.15 } else { 0.0 };
    if seconds > 2.0 && *stage == 0 {
        if options.all_bots {
            actions.shake = true;
        } else {
            actions.add = 100;
        }
        *stage = 1;
    }
    if seconds > 4.0 && *stage == 1 {
        if !options.all_bots {
            actions.remove = 100;
        }
        actions.shake = true;
        *stage = 2;
    }
    if seconds > 5.0 && *stage == 2 {
        actions.reset = true;
        *stage = 3;
    }
    if seconds > 6.0 && *stage == 3 {
        for (node, visibility, target) in &overlays {
            assert!(
                node.size().min_element() > 0.0 && visibility.get() && target.get().is_some(),
                "HUD must have a visible layout and a target camera"
            );
        }
        std::fs::create_dir_all("logs").expect("Create screenshot directory");
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(if options.all_bots {
                format!(
                    "logs/bots-{}-{}-overview.png",
                    options.bot_copies,
                    if options.bot_physics {
                        "rigid"
                    } else {
                        "render"
                    }
                )
            } else {
                "logs/stress-demo.png".into()
            }));
        *stage = 4;
    }
    if seconds > 6.3 && *stage == 4 {
        if options.all_bots {
            state.selected_bot = Some(0);
            state.follow = true;
        } else {
            state.follow = false;
        }
        *stage = 5;
    }
    if seconds > 7.8 && *stage == 5 {
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(if options.all_bots {
                format!(
                    "logs/bots-{}-{}-detail.png",
                    options.bot_copies,
                    if options.bot_physics {
                        "rigid"
                    } else {
                        "render"
                    }
                )
            } else {
                "logs/arena-overview.png".into()
            }));
        *stage = 6;
    }
    if seconds > 9.0 {
        exit.write(crate::shutdown::ExitRequest);
    }
}
