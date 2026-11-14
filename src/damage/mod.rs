//! Cheap, permanent visual dents over ordinary rigid-body physics.
mod content;
mod effects;
mod paint;
mod physics;
mod render;
mod ui;
mod validation;

use crate::Options;
use arena_core::damage::{SurfaceHit, ray_triangle};
use bevy::{diagnostic::FrameTimeDiagnosticsPlugin, prelude::*};
use std::{thread::JoinHandle, time::Instant};

// Multipliers are linear: the highest setting has 64x the reference triangles.
const MESH_DENSITIES: [f32; 5] = [0.5, 1.0, 2.0, 4.0, 8.0];
const DEFAULT_DENSITY: usize = 1;

#[derive(Clone, Copy)]
enum Target {
    Shell(SurfaceHit),
    Component(usize),
}
#[derive(Clone, Copy)]
struct Aim {
    target: Target,
    point: Vec3,
    normal: Vec3,
    distance: f32,
}

struct Active {
    loaded: content::Loaded,
    physics: physics::RigidWorld,
    index: usize,
}

impl Active {
    fn pick(&self, origin: Vec3, direction: Vec3) -> Option<Aim> {
        let transform = self.physics.transform();
        let inverse = transform.to_matrix().inverse();
        let world_origin = origin;
        let world_direction = direction;
        let mut origin = inverse.transform_point3(origin);
        let mut direction = inverse.transform_vector3(direction).normalize();
        let mut hit = self.loaded.shell.raycast(origin, direction).map(|h| Aim {
            target: Target::Shell(h),
            point: h.point,
            normal: h.normal,
            distance: h.distance,
        });
        let mut previous = None;
        for &(component, points) in &self.loaded.picks {
            if previous != Some(component) {
                let inverse = self.physics.frame(component).to_matrix().inverse();
                origin = inverse.transform_point3(world_origin);
                direction = inverse.transform_vector3(world_direction).normalize();
                previous = Some(component);
            }
            if let Some((distance, _)) = ray_triangle(origin, direction, points)
                && hit.is_none_or(|h| distance < h.distance)
            {
                let n = (points[1] - points[0])
                    .cross(points[2] - points[0])
                    .normalize_or_zero();
                hit = Some(Aim {
                    target: Target::Component(component),
                    point: origin + direction * distance,
                    normal: if n.dot(direction) > 0.0 { -n } else { n },
                    distance,
                });
            }
        }
        hit
    }
}

struct Loading {
    index: usize,
    density: usize,
    task: JoinHandle<Result<content::Loaded, String>>,
}

#[derive(Resource)]
struct Lab {
    active: Active,
    selection: usize,
    density: usize,
    ui_tab: usize,
    density_preview: f32,
    loading: Option<Loading>,
    filter: String,
    search_focus: bool,
    page: usize,
    wire: bool,
    smooth: bool,
    boxes: bool,
    show_impact: bool,
    paused: bool,
    power: f32,
    radius: f32,
    depth: f32,
    max_depth: f32,
    aim: Option<Aim>,
    last_impact: Option<Aim>,
    revision: u64,
    strikes: usize,
    changed: usize,
    damage_ms: f64,
    effects_hit_ms: f64,
    effect_settings: effects::Settings,
    effects: effects::Effects,
    accumulator: f32,
    cooldown: f32,
    reset: bool,
    status: String,
}

impl Lab {
    fn new(loaded: content::Loaded, index: usize) -> Result<Self, String> {
        let physics = physics::RigidWorld::new(&loaded)?;
        let density = loaded.density;
        Ok(Self {
            active: Active {
                loaded,
                physics,
                index,
            },
            selection: index,
            density,
            ui_tab: 0,
            density_preview: density as f32,
            loading: None,
            filter: String::new(),
            search_focus: false,
            page: 0,
            wire: false,
            smooth: false,
            boxes: false,
            show_impact: true,
            paused: false,
            power: 1.0,
            radius: 0.55,
            depth: 0.055,
            max_depth: 0.45,
            aim: None,
            last_impact: None,
            revision: 0,
            strikes: 0,
            changed: 0,
            damage_ms: 0.0,
            effects_hit_ms: 0.0,
            effect_settings: effects::Settings::default(),
            effects: effects::Effects::default(),
            accumulator: 0.0,
            cooldown: 0.0,
            reset: false,
            status: "Ready".into(),
        })
    }

    fn impact(&mut self, aim: Aim, power: f32) {
        let start = Instant::now();
        let transform = self.active.physics.aim_frame(aim);
        match aim.target {
            Target::Shell(hit) => {
                // Only components still inside the robot receive shell intrusion.
                let mut attached: Vec<_> = self
                    .active
                    .loaded
                    .boxes
                    .iter()
                    .filter(|b| self.active.physics.attached(b.component))
                    .cloned()
                    .collect();
                let result = self.active.loaded.shell.dent(
                    hit,
                    self.radius,
                    self.depth * power,
                    self.max_depth,
                    25.0 * power,
                    &mut attached,
                );
                self.active.loaded.content.health.apply(
                    0,
                    25.0 * power,
                    transform.transform_point(aim.point),
                );
                for updated in attached {
                    if let Some(b) = self
                        .active
                        .loaded
                        .boxes
                        .iter_mut()
                        .find(|b| b.component == updated.component)
                    {
                        let health = &mut self.active.loaded.content.health;
                        let delta = (updated.damage - b.damage).max(0.);
                        let maximum = health.parts[updated.component].maximum;
                        health.apply(
                            updated.component,
                            delta * maximum / 100.,
                            transform.transform_point(aim.point),
                        );
                        b.damage = (1. - health.parts[updated.component].fraction()) * 100.;
                        b.intrusion = updated.intrusion;
                    }
                }
                self.changed = result.changed_nodes;
                self.status = format!(
                    "Dent: {} vertices / {} components / intrusion {:.1} mm",
                    result.changed_nodes,
                    result.affected_components.len(),
                    result.max_intrusion * 1000.0
                );
            }
            Target::Component(index) => {
                if self.active.physics.attached(index) {
                    if let Some(component) = self
                        .active
                        .loaded
                        .boxes
                        .iter_mut()
                        .find(|c| c.component == index)
                    {
                        let health = &mut self.active.loaded.content.health;
                        let amount = health
                            .apply(index, 60.0 * power, transform.transform_point(aim.point))
                            .map_or(0., |e| e.amount);
                        component.damage = (1. - health.parts[index].fraction()) * 100.;
                        self.status = format!("Direct hit: {} / {amount:.0} HP", component.name);
                    }
                } else {
                    self.status = "Hit detached assembly".into();
                }
                self.changed = 0;
            }
        }
        let detached = self
            .active
            .physics
            .detach_failed(&self.active.loaded.boxes, aim, power);
        if detached > 0 {
            let robot = &self.active.loaded.content.robot;
            let health = &mut self.active.loaded.content.health;
            for b in &self.active.loaded.boxes {
                if !self.active.physics.attached(b.component) && health.parts[b.component].attached
                {
                    health.disconnect(robot, b.component);
                }
            }
            self.status
                .push_str(&format!(" / detached {detached} assembly(s)"));
        }
        self.damage_ms = start.elapsed().as_secs_f64() * 1000.0;
        let effects_start = Instant::now();
        let color = match aim.target {
            Target::Shell(hit) => {
                let uv = paint::hit_uv(&self.active.loaded.shell, hit);
                let color = self.active.loaded.paint.sample(uv);
                if self.effect_settings.wear && self.effect_settings.wear_strength > 0.0 {
                    self.active.loaded.paint.strike(
                        &self.active.loaded.content.chassis,
                        hit.rest_point,
                        self.active.loaded.shell.triangles[hit.triangle].normal,
                        self.effect_settings.wear_radius,
                        power,
                        self.strikes as u32 + 1,
                    );
                }
                color
            }
            Target::Component(_) => [0.35, 0.38, 0.4, 1.0],
        };
        self.effects.emit(
            transform.transform_point(aim.point),
            transform.rotation * aim.normal,
            color,
            power,
            &self.effect_settings,
        );
        self.effects_hit_ms = effects_start.elapsed().as_secs_f64() * 1000.0;
        self.last_impact = Some(aim);
        self.strikes += 1;
        self.revision += 1;
    }
}

#[derive(Resource)]
struct Orbit {
    yaw: f32,
    pitch: f32,
    distance: f32,
    focus: Vec3,
}
#[derive(Component)]
struct LabCamera;

pub fn run(options: Options) -> Result<(), String> {
    let catalogue = content::Catalogue::scan(&options.game_dir)?;
    if options.damage_check {
        return validation::check(&catalogue);
    }
    let index = catalogue
        .entries
        .iter()
        .position(|e| {
            e.path
                .to_string_lossy()
                .replace('\\', "/")
                .to_lowercase()
                .ends_with("ai/team0/bot0.bot")
        })
        .unwrap_or(0);
    let loaded = content::Loaded::load(&catalogue.root, &catalogue.entries[index].path)?;
    let lab = Lab::new(loaded, index)?;
    std::fs::create_dir_all("logs").map_err(|e| e.to_string())?;
    App::new()
        .insert_resource(options)
        .insert_resource(catalogue)
        .insert_resource(lab)
        .insert_resource(Orbit {
            yaw: 0.65,
            pitch: 0.65,
            distance: 4.5,
            focus: Vec3::new(0.0, 0.4, 0.0),
        })
        .insert_resource(ClearColor(Color::srgb(0.025, 0.035, 0.055)))
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Robot Arena 2 / Rigid Damage Lab".into(),
                resolution: (1600, 1000).into(),
                present_mode: bevy::window::PresentMode::AutoVsync,
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
        .add_observer(ui::range_change)
        .add_systems(Startup, (render::setup, effects::setup, ui::setup).chain())
        .add_systems(
            Update,
            (
                ui::keyboard,
                ui::range_visuals,
                ui::scroll,
                load_robot,
                input,
                simulate,
                effects::animate,
                render::synchronize,
                effects::draw,
                ui::refresh,
                validation::smoke,
            )
                .chain(),
        )
        .run();
    Ok(())
}

#[allow(
    clippy::too_many_arguments,
    reason = "Independent Bevy resources for atomic scene replacement"
)]
fn load_robot(
    mut commands: Commands,
    mut lab: ResMut<Lab>,
    catalogue: Res<content::Catalogue>,
    mut graphics: ResMut<render::Graphics>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut orbit: ResMut<Orbit>,
) {
    if lab.loading.as_ref().is_some_and(|l| l.task.is_finished()) {
        let loading = lab.loading.take().unwrap();
        let result = loading
            .task
            .join()
            .unwrap_or_else(|_| Err("Robot loader failed".into()));
        if loading.index == lab.selection && loading.density == lab.density {
            match result.and_then(|loaded| {
                physics::RigidWorld::new(&loaded).map(|physics| Active {
                    loaded,
                    physics,
                    index: loading.index,
                })
            }) {
                Ok(active) => {
                    // All instance-owned visuals and physics are replaced together.
                    // Dropping the old handles also releases unique meshes/textures.
                    let robot_changed = lab.active.index != active.index;
                    lab.active = active;
                    lab.effects.clear();
                    lab.aim = None;
                    lab.last_impact = None;
                    lab.accumulator = 0.0;
                    lab.cooldown = 0.0;
                    lab.strikes = 0;
                    lab.changed = 0;
                    lab.damage_ms = 0.0;
                    lab.effects_hit_ms = 0.0;
                    lab.revision += 1;
                    lab.status = format!("Loaded {}", catalogue.entries[loading.index].name);
                    if robot_changed {
                        orbit.focus = Vec3::new(0.0, 0.4, 0.0);
                        orbit.distance = (lab.active.loaded.content.assembly.bounds.1
                            - lab.active.loaded.content.assembly.bounds.0)
                            .length()
                            .mul_add(1.1, 1.8)
                            .clamp(2.5, 11.0);
                    }
                    render::replace_robot(
                        &mut commands,
                        &lab,
                        &mut graphics,
                        &mut meshes,
                        &mut materials,
                        &mut images,
                    );
                }
                Err(e) => {
                    lab.status = format!("Load failed: {e}");
                    lab.selection = lab.active.index;
                    lab.density = lab.active.loaded.density;
                    lab.density_preview = lab.density as f32;
                }
            }
        }
    }
    if lab.loading.is_none()
        && (lab.selection != lab.active.index
            || lab.density != lab.active.loaded.density
            || lab.reset)
    {
        let index = lab.selection;
        let density = lab.density;
        let root = catalogue.root.clone();
        let path = catalogue.entries[index].path.clone();
        lab.loading = Some(Loading {
            index,
            density,
            task: std::thread::spawn(move || {
                content::Loaded::load_with_density(&root, &path, density)
            }),
        });
        lab.reset = false;
        lab.status = format!(
            "Loading {} / mesh {}x...",
            catalogue.entries[index].name, MESH_DENSITIES[density]
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn input(
    time: Res<Time<Real>>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    motion: Res<bevy::input::mouse::AccumulatedMouseMotion>,
    scroll: Res<bevy::input::mouse::AccumulatedMouseScroll>,
    sliders: Query<&bevy::ui_widgets::SliderDragState>,
    window: Single<&Window>,
    mut camera: Single<(&Camera, &GlobalTransform, &mut Transform), With<LabCamera>>,
    mut orbit: ResMut<Orbit>,
    mut lab: ResMut<Lab>,
) {
    lab.cooldown = (lab.cooldown - time.delta_secs()).max(0.0);
    let over_ui = sliders.iter().any(|slider| slider.dragging)
        || window
            .cursor_position()
            .is_some_and(|p| ui::over_ui(p, window.size()));
    if !over_ui && mouse.pressed(MouseButton::Right) {
        if keys.pressed(KeyCode::ShiftLeft) {
            let distance = orbit.distance;
            orbit.focus += (-camera.1.right().as_vec3() * motion.delta.x
                + Vec3::Y * motion.delta.y)
                * distance
                * 0.0015;
        } else {
            orbit.yaw -= motion.delta.x * 0.006;
            orbit.pitch = (orbit.pitch + motion.delta.y * 0.005).clamp(-1.3, 1.5);
        }
    }
    if !over_ui {
        orbit.distance = (orbit.distance * (-scroll.delta.y * 0.1).exp()).clamp(1.3, 15.0);
    }
    let (sy, cy) = orbit.yaw.sin_cos();
    let (sp, cp) = orbit.pitch.sin_cos();
    camera.2.translation = orbit.focus + Vec3::new(sy * cp, sp, cy * cp) * orbit.distance;
    camera.2.look_at(orbit.focus, Vec3::Y);
    // Use the just-updated camera pose, rather than the previous propagated one.
    let global = GlobalTransform::from(*camera.2);
    lab.aim = window
        .cursor_position()
        .filter(|_| !over_ui)
        .and_then(|p| camera.0.viewport_to_world(&global, p).ok())
        .and_then(|ray| lab.active.pick(ray.origin, *ray.direction));
    if !lab.paused
        && !lab.search_focus
        && lab.loading.is_none()
        && lab.cooldown == 0.0
        && mouse.pressed(MouseButton::Left)
        && !mouse.pressed(MouseButton::Right)
        && let Some(aim) = lab.aim
    {
        let power = lab.power;
        lab.active.physics.strike(aim, power);
        lab.cooldown = 0.75;
    }
}

fn simulate(time: Res<Time<Real>>, mut lab: ResMut<Lab>) {
    if lab.paused || lab.loading.is_some() {
        return;
    }
    lab.active.loaded.content.health.advance(time.delta_secs());
    lab.accumulator = (lab.accumulator + time.delta_secs().min(0.05)).min(4.0 / 120.0);
    while lab.accumulator >= 1.0 / 120.0 {
        if let Some((aim, power)) = lab.active.physics.step(1.0 / 120.0) {
            lab.impact(aim, power);
        }
        lab.accumulator -= 1.0 / 120.0;
    }
}
