//! Test garage lifecycle. The editable document is copied into an isolated physics world.
use crate::gameplay::RobotInstance;
mod environment;
pub(crate) use crate::gameplay::{RobotBody as GarageBody, spawn};
#[cfg(test)]
use crate::gameplay::{damage, drive, tyres};
#[cfg(test)]
pub(crate) mod tests;
use crate::{
    editor::{Editor, Mode},
    frontend::{Navigation, Page},
    legacy_ui::{self, UiEvent},
};
#[cfg(test)]
use arena_core::mechanism::*;
use bevy::prelude::Real;
use bevy::{
    camera::{RenderTarget, visibility::RenderLayers},
    core_pipeline::tonemapping::Tonemapping,
    prelude::*,
    render::render_resource::TextureFormat,
};
use bevy_rapier3d::prelude::*;

#[derive(Component)]
pub(crate) struct GarageContext;
#[derive(Component)]
struct GarageCamera;
#[derive(Resource)]
pub(crate) struct Garage {
    pub(crate) instance: RobotInstance,
    pub(crate) camera: Entity,
    target: Handle<Image>,
    obstacles: Vec<Entity>,
    environment: Vec<Entity>,
    reset: bool,
    obstacle: usize,
    loaded_obstacle: usize,
    yaw: f32,
    pitch: f32,
    distance: f32,
    hud_time: f64,
}
impl Garage {
    /// Render-free owner of an actual robot assembly, used by prediction and headless tests.
    pub(crate) fn simulation(instance:RobotInstance)->Self {Self{instance,camera:Entity::PLACEHOLDER,target:default(),obstacles:vec![],environment:vec![],reset:false,obstacle:0,loaded_obstacle:0,yaw:0.,pitch:0.,distance:4.,hud_time:0.}}
}
impl std::ops::Deref for Garage {
    type Target = RobotInstance;
    fn deref(&self) -> &RobotInstance {
        &self.instance
    }
}
impl std::ops::DerefMut for Garage {
    fn deref_mut(&mut self) -> &mut RobotInstance {
        &mut self.instance
    }
}
pub struct GaragePlugin;
impl Plugin for GaragePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup).add_systems(
            Update,
            update.after(legacy_ui::Input).before(crate::editor::Edit),
        );
    }
}
fn setup(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    config: Res<crate::config::Config>,
) {
    let context = commands
        .spawn((
            GarageContext,
            RapierContextSimulation::default(),
            RapierConfiguration {
                physics_pipeline_active: false,
                ..RapierConfiguration::new(1.0)
            },
        ))
        .id();
    let target = images.add(Image::new_target_texture(
        1024,
        640,
        TextureFormat::Rgba8UnormSrgb,
        None,
    ));
    let camera = commands
        .spawn((
            GarageCamera,
            Camera3d::default(),
            Camera {
                is_active: false,
                order: -2,
                clear_color: ClearColorConfig::Custom(Color::srgb(0.07, 0.07, 0.07)),
                ..default()
            },
            RenderTarget::Image(target.clone().into()),
            RenderLayers::layer(9),
            Tonemapping::None,
            bevy::core_pipeline::tonemapping::DebandDither::Disabled,
            Msaa::Sample4,
            Transform::default(),
        ))
        .id();
    commands.spawn((
        DirectionalLight {
            illuminance: 3000.0,
            shadow_maps_enabled: false,
            ..default()
        },
        Transform::from_rotation(Quat::from_euler(EulerRot::XYZ, -1.0, -0.4, 0.0)),
        RenderLayers::layer(9),
    ));
    commands.insert_resource(Garage {
        instance: RobotInstance::new(0, context, config.garage.clone()),
        camera,
        target,
        obstacles: vec![],
        environment: vec![],
        reset: false,
        obstacle: 0,
        loaded_obstacle: usize::MAX,
        yaw: 0.0,
        pitch: 0.45,
        distance: 5.0,
        hud_time: 0.0,
    });
}
const OBSTACLES: [(&str, &str); 6] = [
    ("none cb", ""),
    ("crates cb", "Arenas/crates/crates.gmf"),
    ("blocks cb", "Arenas/cinderblocks/blocks.gmf"),
    ("barrels cb", "Arenas/barrels/barrels.gmf"),
    ("cones cb", "Arenas/cones/cones.gmf"),
    ("ramps cb", "Arenas/ramps/ramps.gmf"),
];
#[allow(
    clippy::too_many_arguments,
    clippy::type_complexity,
    reason = "Garage lifecycle uses independent scene, input, GUI and asset adapters"
)]
fn update(
    mut commands: Commands,
    mut garage: ResMut<Garage>,
    mut editor: ResMut<Editor>,
    catalog: Res<crate::editor::catalog::Catalog>,
    nav: Res<Navigation>,
    mut events: ResMut<legacy_ui::Events>,
    mut output: ResMut<legacy_ui::Output>,
    mut ui: ResMut<legacy_ui::State>,
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time<Real>>,
    mut cameras: Query<(&mut Camera, &mut Transform), With<GarageCamera>>,
    bodies: Query<(&Transform, &Velocity), (With<GarageBody>, Without<GarageCamera>)>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    let testing =
        editor.mode == Mode::Test && nav.page == Page::Workshop && editor.dialog.is_none();
    garage.running = testing;
    if let Ok((mut camera, _)) = cameras.get_mut(garage.camera) {
        camera.is_active = testing;
    }
    if editor.mode != Mode::Test {
        if garage.active
            || !garage.bodies.is_empty()
            || !garage.obstacles.is_empty()
            || !garage.environment.is_empty()
        {
            for e in garage.damage.fragments.drain(..) {
                commands.entity(e).despawn();
            }
            for e in garage.bodies.drain(..) {
                commands.entity(e).despawn();
            }
            for e in garage.obstacles.drain(..) {
                commands.entity(e).despawn();
            }
            for e in garage.environment.drain(..) {
                commands.entity(e).despawn();
            }
            garage.rig = None;
            garage.document = None;
            garage.active = false;
            garage.loaded_obstacle = usize::MAX;
        }
        return;
    }
    if !testing {
        return;
    }
    output
        .targets
        .insert("test arena placeholder".into(), garage.target.clone());
    events.0.retain(|event| {
        match event {
            UiEvent::Activate { name, .. } if name == "reset button" => {
                garage.reset = true;
                return false;
            }
            UiEvent::Activate { name, .. } if OBSTACLES.iter().any(|(n, _)| n == name) => {
                garage.obstacle = OBSTACLES.iter().position(|(n, _)| n == name).unwrap();
                editor.ui_revision += 1;
                return false;
            }
            UiEvent::Drag {
                name,
                delta,
                button: MouseButton::Right,
                ..
            } if name == "test arena placeholder" => {
                garage.yaw -= delta.x * 0.012;
                garage.pitch = (garage.pitch + delta.y * 0.012).clamp(0.05, 1.2);
                return false;
            }
            UiEvent::Wheel { name, delta } if name == "test arena placeholder" => {
                garage.distance = (garage.distance - delta * 0.3).clamp(2.0, 9.0);
                return false;
            }
            _ => (),
        }
        true
    });
    if keys.just_pressed(KeyCode::Home) {
        garage.reset = true;
    }
    if !garage.active || garage.reset {
        for e in garage.damage.fragments.drain(..) {
            commands.entity(e).despawn();
        }
        for e in garage.bodies.drain(..) {
            commands.entity(e).despawn();
        }
        for e in garage.obstacles.drain(..) {
            commands.entity(e).despawn();
        }
        garage.loaded_obstacle = usize::MAX;
        let result = environment::start(&catalog).and_then(|placement| {
            garage.placement = Some(placement);
            spawn::robot(
                &mut commands,
                &mut garage,
                &editor.document.robot,
                &catalog,
                &mut meshes,
                &mut materials,
                &mut images,
            )
        });
        if let Err(e) = result {
            editor.dialog = Some(crate::editor::Dialog::Message(e));
            editor.mode = Mode::Components;
            editor.ui_revision += 1;
            return;
        }
        if garage.environment.is_empty() {
            match environment::environment(
                &mut commands,
                &garage,
                &catalog,
                &mut meshes,
                &mut materials,
                &mut images,
            ) {
                Ok(entities) => garage.environment = entities,
                Err(e) => {
                    editor.dialog = Some(crate::editor::Dialog::Message(e));
                    editor.mode = Mode::Components;
                    editor.ui_revision += 1;
                    return;
                }
            }
        }
        garage.active = true;
        garage.reset = false;
        editor.ui_revision += 1;
    }
    if garage.loaded_obstacle != garage.obstacle {
        for e in garage.obstacles.drain(..) {
            commands.entity(e).despawn();
        }
        match environment::obstacles(
            &mut commands,
            &garage,
            &catalog,
            OBSTACLES[garage.obstacle].1,
            &mut meshes,
            &mut materials,
            &mut images,
        ) {
            Ok(e) => garage.obstacles = e,
            Err(e) => {
                warn!("Garage obstacles: {e}");
                editor.dialog = Some(crate::editor::Dialog::Message(e));
            }
        }
        garage.loaded_obstacle = garage.obstacle;
    }
    if let RobotInstance {
        document: Some(robot),
        controls,
        commands: previous,
        ..
    } = &mut garage.instance
    {
        let samples = crate::editor::controls::samples(robot, &keys, true);
        let mut next = controls.evaluate(robot, &samples);
        for (n, p) in next.iter_mut().zip(previous.iter()) {
            n.fire |= p.fire;
        }
        *previous = next;
    }
    if let Some(&body) = garage.bodies.first()
        && let Ok((pose, _)) = bodies.get(body)
        && let Ok((_, mut camera)) = cameras.get_mut(garage.camera)
    {
        let focus = pose.translation + Vec3::Y * 0.35;
        let eye = Vec3::new(
            garage.yaw.sin() * garage.pitch.cos(),
            garage.pitch.sin(),
            garage.yaw.cos() * garage.pitch.cos(),
        ) * garage.distance;
        *camera = Transform::from_translation(focus + eye).looking_at(focus, Vec3::Y);
        garage.damage.view = camera.to_matrix().inverse();
    }
    if let Some(r) = output.regions.get("test arena placeholder") {
        garage.damage.aspect = r.width() / r.height().max(1.);
        let size = (r.size() * output.scale)
            .as_uvec2()
            .max(UVec2::splat(64))
            .min(UVec2::new(1920, 1200));
        if let Some(mut image) = images.get_mut(&garage.target)
            && (image.width() != size.x || image.height() != size.y)
        {
            image.resize(bevy::render::render_resource::Extent3d {
                width: size.x,
                height: size.y,
                depth_or_array_layers: 1,
            });
        }
    }
    for (i, (name, _)) in OBSTACLES.iter().enumerate() {
        ui.checked.insert((*name).into(), garage.obstacle == i);
    }
    if time.elapsed_secs_f64() - garage.hud_time > 0.1 {
        if std::env::var_os("RA2_UI_PROBE").is_some()
            && let Some(&body) = garage.bodies.first()
            && let Ok((pose, velocity)) = bodies.get(body)
            && let Some(rig) = &garage.rig
        {
            let data = format!(
                "position {} {} {}\nvelocity {} {} {}\nbattery {}\nair {}\nbodies {}\nhull_hp {}\nreceived_damage {}\ndent_revision {}\n",
                pose.translation.x,
                pose.translation.y,
                pose.translation.z,
                velocity.linear.x,
                velocity.linear.y,
                velocity.linear.z,
                rig.electrical.remaining,
                rig.pneumatic.remaining,
                garage.bodies.len(),
                garage.damage.health.parts.first().map_or(0., |p| p.current),
                garage.damage.health.received,
                garage.damage.shell.as_ref().map_or(0, |s| s.revision)
            );
            let _ = std::fs::write("logs/garage-state.txt", data);
        }
        garage.hud_time = time.elapsed_secs_f64();
        editor.ui_revision += 1;
    }
}
pub fn bind_ui(
    garage: &Garage,
    ui: &mut legacy_ui::State,
    assets: &legacy_ui::assets::LegacyAssets,
) {
    if let Some(rig) = &garage.rig {
        ui.values.insert(
            "name".into(),
            garage
                .document
                .as_ref()
                .map(|r| r.name.clone())
                .unwrap_or_default(),
        );
        ui.values.insert("points".into(), "N/A".into());
        if let Ok(panel) = assets.widget("botlab/testarena.gib") {
            let mut panel = (*panel).clone();
            for (name, fraction, width) in [
                ("battery bar", rig.electrical.fraction(), 52.),
                ("air bar", rig.pneumatic.fraction(), 52.),
                (
                    "armor bar",
                    garage
                        .damage
                        .health
                        .parts
                        .first()
                        .map_or(1., |p| p.fraction()),
                    26.,
                ),
            ] {
                ui.visibility.insert(name.into(), fraction > 0.0);
                crate::editor::panels::patch(
                    &mut panel,
                    name,
                    "Size",
                    &format!("{} 0 4 0", width * fraction),
                );
            }
            ui.slots
                .insert("content slot".into(), std::sync::Arc::new(panel));
        }
    }
    for (i, (name, _)) in OBSTACLES.iter().enumerate() {
        ui.checked.insert((*name).into(), garage.obstacle == i);
    }
}
