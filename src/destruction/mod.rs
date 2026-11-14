//! Interactive destruction laboratory; input, simulation, legacy content and GPU updates are separate.
pub(crate) mod content;
mod render;
#[cfg(test)]
mod tests;
mod tools;
mod ui;

use crate::Options;
use arena_runtime::destruction::{ArmourShell, QUALITIES, world::DestructionWorld};
use bevy::{diagnostic::FrameTimeDiagnosticsPlugin, prelude::*};
use bevy_rapier3d::rapier::prelude::{
    ColliderBuilder, ColliderHandle, RigidBodyBuilder, RigidBodyHandle, SoftBody, SoftBodyHandle,
};
use content::Content;
use std::collections::VecDeque;

#[derive(Resource)]
struct Lab {
    physics: DestructionWorld,
    quality: usize,
    direct: bool,
    iterations: usize,
    hz: u32,
    paused: bool,
    slow: bool,
    wire: bool,
    reset: bool,
    tool: tools::Tool,
    power: f32,
    cut_angle: f32,
    target: Option<(ColliderHandle, Vec3, Vec3)>,
    cooldown: f32,
    saw_active: bool,
    samples: VecDeque<f64>,
    backlog: f64,
    dropped: f64,
    render_ms: f64,
    strikes: usize,
    status: String,
    generation: u64,
}
#[derive(Component)]
struct LabEntity;
#[derive(Component)]
struct LabCamera;
#[derive(Resource)]
struct Orbit {
    yaw: f32,
    pitch: f32,
    distance: f32,
    focus: Vec3,
}

fn build_physics(
    content: &Content,
    quality: usize,
    direct: bool,
    iterations: usize,
) -> Result<DestructionWorld, String> {
    let shell = ArmourShell::from_model(&content.chassis, QUALITIES[quality], 0.035)?;
    let mut physics = DestructionWorld::new(
        shell,
        content.origin,
        content.assembly.parts[0].mass,
        &content.parts()?,
        true,
        iterations,
    )?;
    physics
        .world
        .integration_parameters
        .soft_bodies
        .fem
        .max_dense_dofs = if direct { 2048 } else { 0 };
    let floor = content
        .arena
        .object("h_box")
        .ok_or("Missing garage collision mesh")?;
    let vertices = floor
        .positions()
        .map(|p| p + content.arena_offset)
        .collect();
    let indices = floor
        .faces
        .iter()
        .map(|f| f.vertices.map(|i| i as u32))
        .collect();
    let collider = ColliderBuilder::trimesh(vertices, indices)
        .map_err(|e| e.to_string())?
        .friction(0.8);
    physics.world.insert(RigidBodyBuilder::fixed(), collider);
    physics.world.integration_parameters.dt = 1.0 / 60.0;
    Ok(physics)
}

pub fn run(options: Options) -> Result<(), String> {
    let content = Content::load(&options.game_dir)?;
    if options.destruction_check {
        return check(&content);
    }
    let quality = 2;
    let direct = true;
    let iterations = QUALITIES[quality].iterations;
    let physics = build_physics(&content, quality, direct, iterations)?;
    println!(
        "DESTRUCTION LAB: {} / {} components / {} shell particles / {} tetrahedra",
        content.robot.name,
        content.part_indices.len(),
        physics.shell.positions.len(),
        physics.shell.cells.len()
    );
    App::new()
        .insert_resource(options)
        .insert_resource(content)
        .insert_resource(Lab {
            physics,
            quality,
            direct,
            iterations,
            hz: 60,
            paused: false,
            slow: false,
            wire: false,
            reset: false,
            tool: tools::Tool::Hammer,
            power: 1.0,
            cut_angle: 0.0,
            target: None,
            cooldown: 0.0,
            saw_active: false,
            samples: VecDeque::with_capacity(240),
            backlog: 0.0,
            dropped: 0.0,
            render_ms: 0.0,
            strikes: 0,
            status: String::new(),
            generation: 0,
        })
        .insert_resource(Orbit {
            yaw: 0.65,
            pitch: 0.65,
            distance: 4.0,
            focus: Vec3::new(0.0, 0.35, 0.0),
        })
        .insert_resource(ClearColor(Color::srgb(0.025, 0.035, 0.055)))
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Robot Arena 2 / Destruction Lab".into(),
                resolution: (1440, 960).into(),
                present_mode: bevy::window::PresentMode::AutoNoVsync,
                ..default()
            }),
            close_when_requested: false,
            ..default()
        }))
        .add_plugins((
            crate::shutdown::ShutdownPlugin,
            FrameTimeDiagnosticsPlugin::new(120),
        ))
        .add_observer(ui::activate)
        .add_systems(Startup, (render::setup, ui::setup).chain())
        .add_systems(
            Update,
            (
                ui::keyboard,
                tools::input,
                reset,
                simulate,
                render::synchronize,
                render::effects,
                ui::refresh,
                smoke,
            )
                .chain(),
        )
        .run();
    Ok(())
}

fn reset(
    mut commands: Commands,
    content: Res<Content>,
    mut lab: ResMut<Lab>,
    entities: Query<Entity, With<LabEntity>>,
) {
    if !lab.reset {
        return;
    }
    match build_physics(&content, lab.quality, lab.direct, lab.iterations) {
        Ok(physics) => {
            for e in &entities {
                commands.entity(e).despawn();
            }
            lab.physics = physics;
            lab.generation += 1;
            lab.samples.clear();
            lab.backlog = 0.0;
            lab.dropped = 0.0;
            lab.target = None;
            lab.cooldown = 0.0;
            lab.strikes = 0;
            lab.status = "Robot restored. Mesh / solver changes rebuild the specimen.".into();
        }
        Err(e) => {
            lab.status = e;
        }
    }
    lab.reset = false;
}
fn simulate(time: Res<Time>, mut lab: ResMut<Lab>) {
    if lab.paused {
        lab.backlog = 0.0;
        return;
    }
    let scale = if lab.slow { 0.2 } else { 1.0 };
    lab.backlog += (time.delta_secs_f64() * scale).min(0.1);
    let step = 1.0 / lab.hz as f64;
    let iterations = lab.iterations;
    // A bounded catch-up avoids a spiral of death when experimenting with expensive settings.
    let max_steps = if lab.hz == 120 { 4 } else { 2 };
    for _ in 0..max_steps {
        if lab.backlog < step {
            break;
        }
        lab.physics.step(step as f32, iterations);
        if lab.physics.unstable {
            lab.paused = true;
            lab.status = "Solver diverged. Restore [R], increase iterations or use FEM.".into();
            lab.backlog = 0.0;
            break;
        }
        lab.backlog -= step;
        let ms = lab.physics.last_step_ms;
        lab.samples.push_back(ms);
        if lab.samples.len() > 240 {
            lab.samples.pop_front();
        }
    }
    if lab.backlog >= step {
        let dropped = lab.backlog - step;
        lab.dropped += dropped;
        lab.backlog = step;
    }
}

fn check_presets(content: &Content) -> Result<(), String> {
    for (q, quality) in QUALITIES.iter().enumerate() {
        for direct in [true, false] {
            let iterations = quality.iterations;
            let mut p = build_physics(content, q, direct, iterations)?;
            for tick in 0..120 {
                p.step(1.0 / 60.0, iterations);
                if p.unstable {
                    return Err(format!("Unstable preset {q} direct={direct} tick={tick}"));
                }
            }
            let com = p.world.soft_bodies[p.root].center_of_mass();
            if Vec2::new(com.x, com.z).length() > 0.4 {
                return Err(format!("Drifting preset {q} direct={direct}: {com:?}"));
            }
            println!(
                "Preset {q} direct={direct}: COM {com:?} nodes {} broken {}",
                p.shell.positions.len(),
                p.mounts.iter().filter(|m| m.broken).count()
            );
        }
    }
    Ok(())
}

fn check(content: &Content) -> Result<(), String> {
    if std::env::var_os("RA2_PRESETS").is_some() {
        check_presets(content)?;
    }

    let mut physics = build_physics(content, 1, true, 3)?;
    println!(
        "Spawn {:?}, mass {}, shell bounds {:?}..{:?}",
        content.origin,
        content.assembly.parts[0].mass,
        physics.shell.positions.iter().copied().reduce(Vec3::min),
        physics.shell.positions.iter().copied().reduce(Vec3::max)
    );
    for tick in 0..90 {
        physics.step(1.0 / 60.0, 3);
        if tick % 15 == 0 {
            println!(
                "tick {tick} COM {:?} fragments {}",
                physics.world.soft_bodies[physics.root].center_of_mass(),
                physics.world.soft_bodies.len()
            );
        }
    }
    let p = physics.world.soft_bodies[physics.root].center_of_mass();
    println!(
        "Resting Scout COM {p:?}, {} nodes, {} cells",
        physics.shell.positions.len(),
        physics.shell.cells.len()
    );
    if !p.is_finite()
        || p.y.abs() > 1.5
        || Vec2::new(p.x, p.z).length() > 0.4
        || physics.world.soft_bodies.len() != 1
    {
        return Err("Robot did not settle in garage".into());
    }
    let hit = physics
        .hit(p + Vec3::Y * 3.0, -Vec3::Y)
        .ok_or("Cannot aim at Scout")?;
    physics.strike(hit.1, Vec3::Y, 2.0);
    let before = physics.world.soft_bodies[physics.root]
        .particles()
        .iter()
        .map(|p| p.rest_position())
        .collect::<Vec<_>>();
    for _ in 0..90 {
        physics.step(1.0 / 60.0, 3);
    }
    let dent = physics.world.soft_bodies[physics.root]
        .particles()
        .iter()
        .zip(before)
        .map(|(p, b)| p.rest_position().distance(b))
        .fold(0.0, f32::max);
    println!(
        "Hammer permanent deformation {dent:.5} m; fractures {}",
        physics.fractures
    );
    if dent < 0.005 {
        return Err("Hammer produced no permanent damage".into());
    }
    let p = physics.world.soft_bodies[physics.root].center_of_mass();
    let before = physics.world.soft_bodies.len();
    // A full cross section verifies actual topological separation, independently of mouse input.
    let blade = [
        p + Vec3::new(0.0, -3.0, -3.0),
        p + Vec3::new(0.0, 3.0, -3.0),
        p + Vec3::new(0.0, 0.0, 4.0),
    ];
    let handles: Vec<_> = physics.world.soft_bodies.iter().map(|(h, _)| h).collect();
    let mut cut = false;
    for h in handles {
        cut |= physics.world.cut_soft_body(h, &blade).is_some();
    }
    for _ in 0..30 {
        physics.step(1.0 / 60.0, 3);
    }
    let after = physics.world.soft_bodies.len();
    println!("Cut: {before} -> {after} pieces");
    if !cut || after <= before {
        return Err("Cut did not separate real soft bodies".into());
    }
    for (_, b) in physics.world.soft_bodies.iter() {
        if b.particle_positions().any(|p| !p.is_finite()) {
            return Err("Nonfinite destruction state".into());
        }
        if physics.shell.surface(b).is_empty() {
            return Err("Fragment has no renderable surface".into());
        }
    }
    println!(
        "DESTRUCTION CHECK PASSED: original Scout, hollow armour, physical hammer, plasticity, cutting and fragment rendering"
    );
    Ok(())
}

#[derive(Default)]
struct SmokeState {
    stage: usize,
    age: f32,
}
#[allow(clippy::too_many_arguments)]
fn smoke(
    mut commands: Commands,
    time: Res<Time<Real>>,
    options: Res<Options>,
    mut lab: ResMut<Lab>,
    mut state: Local<SmokeState>,
    mut probe_timer: Local<f32>,
    camera: Single<(&Camera, &GlobalTransform), With<LabCamera>>,
    mut exit: MessageWriter<crate::shutdown::ExitRequest>,
) {
    *probe_timer += time.delta_secs();
    if std::env::var_os("RA2_UI_PROBE").is_some() && *probe_timer > 0.15 {
        *probe_timer = 0.0;
        if let Some(body) = lab.physics.world.soft_bodies.get(lab.physics.root) {
            let com = body.center_of_mass();
            let roof = lab
                .physics
                .hit(com + Vec3::Y * 3.0, -Vec3::Y)
                .map(|(_, p, _)| p)
                .unwrap_or(com);
            let pixel = camera
                .0
                .world_to_viewport(camera.1, roof)
                .ok()
                .unwrap_or(Vec2::ZERO);
            let nodes: usize = lab
                .physics
                .world
                .soft_bodies
                .iter()
                .map(|(_, b)| b.num_particles())
                .sum();
            let json = format!(
                "{{\"com\":[{},{},{}],\"roof\":[{},{}],\"nodes\":{nodes},\"fragments\":{},\"cuts\":{},\"strikes\":{},\"broken\":{},\"quality\":{},\"direct\":{},\"paused\":{}}}",
                com.x,
                com.y,
                com.z,
                pixel.x,
                pixel.y,
                lab.physics.world.soft_bodies.len(),
                lab.physics.cuts,
                lab.strikes,
                lab.physics.mounts.iter().filter(|m| m.broken).count(),
                lab.quality,
                lab.direct,
                lab.paused
            );
            let _ = std::fs::write("logs/destruction-state.json", json);
        }
    }
    if !options.smoke_test {
        return;
    }
    state.age += time.delta_secs();
    if state.age < 1.3 {
        return;
    }
    state.age = 0.0;
    let screenshot = |commands: &mut Commands, name: &str| {
        commands
            .spawn(bevy::render::view::screenshot::Screenshot::primary_window())
            .observe(bevy::render::view::screenshot::save_to_disk(format!(
                "logs/destruction-{name}.png"
            )));
    };
    match state.stage {
        0 => screenshot(&mut commands, "original"),
        1 => {
            let p = lab.physics.world.soft_bodies[lab.physics.root].center_of_mass();
            if let Some((_, p, n)) = lab.physics.hit(p + Vec3::Y * 3.0, -Vec3::Y) {
                lab.physics.strike(p, n, 2.0);
                lab.strikes += 1;
            }
        }
        2 => screenshot(&mut commands, "dented"),
        3 => {
            let p = lab.physics.world.soft_bodies[lab.physics.root].center_of_mass();
            let blade = [
                p + Vec3::new(0.0, -3.0, -3.0),
                p + Vec3::new(0.0, 3.0, -3.0),
                p + Vec3::new(0.0, 0.0, 4.0),
            ];
            let h = lab.physics.root;
            lab.physics.world.cut_soft_body(h, &blade);
        }
        4 => screenshot(&mut commands, "cut"),
        5 => {
            lab.quality = 2;
            lab.reset = true;
        }
        6 => {
            ui::dispatch(ui::Action::Solver, &mut lab);
            lab.wire = true;
        }
        7 => screenshot(&mut commands, "high-wire"),
        _ => {
            println!(
                "DESTRUCTION GUI CHECK PASSED: source textures, hammer, cut, quality rebuild, solver switch, wireframe"
            );
            exit.write(crate::shutdown::ExitRequest);
        }
    }
    state.stage += 1;
}
