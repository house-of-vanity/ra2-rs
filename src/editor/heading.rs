//! Original forward-heading indicator. Native slider uses 20 steps per turn
//! (43bdf0); the arrow is a game asset, shown while adjusting and for 2 s after.
use super::{Editor, Mode};
use arena_core::robot::RobotAssembly;
use arena_runtime::{
    content::FileTextures,
    render::{PreparedModel, RenderSettings, TextureCache, prepare_model},
};
use bevy::prelude::*;

pub(super) fn from_slider(value: f32) -> f32 {
    (value.clamp(0., 1.) * 20.).round_ties_even() * 0.05 * std::f32::consts::TAU
}
pub(super) fn slider_value(heading: f32) -> f32 {
    // 360 degrees is the right endpoint, not a jump back to the left thumb.
    if (-0.00001..=std::f32::consts::TAU + 0.00001).contains(&heading) {
        (heading / std::f32::consts::TAU).clamp(0., 1.)
    } else {
        heading.rem_euclid(std::f32::consts::TAU) / std::f32::consts::TAU
    }
}
#[derive(Default)]
pub struct Indicator {
    prepared: Option<PreparedModel>,
    entity: Option<Entity>,
    previous: Option<f32>,
    until: f64,
}
impl Indicator {
    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &mut self,
        commands: &mut Commands,
        editor: &Editor,
        assembly: Option<&RobotAssembly>,
        ui: &crate::legacy_ui::State,
        now: f64,
        game_dir: &std::path::Path,
        meshes: &mut Assets<Mesh>,
        materials: &mut Assets<StandardMaterial>,
        images: &mut Assets<Image>,
    ) {
        let angle = editor.document.robot.heading;
        let overview = editor.mode == Mode::Overview && editor.dialog.is_none();
        if overview
            && (self.previous != Some(angle)
                || ui.pressed == "heading slider"
                || ui.hover == "heading slider")
        {
            self.until = now + 2.;
        }
        self.previous = Some(angle);
        if !overview || now >= self.until || assembly.is_none() {
            self.hide(commands);
            return;
        }
        if self.prepared.is_none() {
            let path = game_dir.join("UI/botlab/heading_arrow.gmf");
            self.prepared = std::fs::read(&path)
                .ok()
                .and_then(|b| ra2_formats::ModelImporters::with_ra2().decode(&b).ok())
                .and_then(|import| {
                    prepare_model(
                        &import.model,
                        &FileTextures::new(path.parent().unwrap()),
                        &RenderSettings::default(),
                        &mut TextureCache::default(),
                        meshes,
                        materials,
                        images,
                    )
                    .ok()
                });
            if let Some(model) = &self.prepared {
                for handle in model.material_handles() {
                    if let Some(mut m) = materials.get_mut(handle) {
                        m.unlit = true;
                    }
                }
            }
        }
        let Some(model) = &self.prepared else {
            return;
        };
        let entity = *self.entity.get_or_insert_with(|| {
            model.spawn_on_layer(commands, "Forward heading arrow", Transform::default(), 11)
        });
        // Adapt native radius 3 to our closer workshop camera. Keep the original
        // asset and direction; this indicator never changes picking or physics.
        let chassis = editor
            .document
            .robot
            .chassis
            .model
            .as_ref()
            .and_then(|m| m.bounds());
        let radius = chassis
            .map_or(1., |(a, b)| {
                a.xz().abs().max(b.xz().abs()).max_element() + 0.25
            })
            .clamp(0.6, 1.3);
        let rotation = Quat::from_rotation_y(-angle);
        commands.entity(entity).insert((
            Visibility::Visible,
            Transform::from_translation(rotation * Vec3::NEG_Z * radius + Vec3::Y * 0.05)
                .with_rotation(rotation),
        ));
    }
    pub fn hide(&self, commands: &mut Commands) {
        if let Some(entity) = self.entity {
            commands.entity(entity).insert(Visibility::Hidden);
        }
    }
}
