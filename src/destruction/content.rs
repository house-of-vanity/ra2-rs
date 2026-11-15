//! Original BOT/CFZ/GMF loading, separate from the destruction simulation.
use arena_core::{
    Model,
    robot::{RobotAssembly, RobotDefinition},
};
use arena_runtime::{
    content::FileTextures,
    destruction::world::AttachedPart,
    render::{self, PreparedModel, RenderSettings, TextureCache},
};
use bevy::prelude::*;
use bevy_rapier3d::rapier::{math::Pose, prelude::*};
use ra2_formats::{
    ModelImporters,
    assembly::ResolvedComponent,
    components::{self, ComponentCatalog, ComponentDefinition},
};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Resource)]
pub struct Content {
    pub robot: RobotDefinition,
    pub health: arena_core::health::Health,
    pub assembly: RobotAssembly,
    pub chassis: Model,
    pub models: Vec<Model>,
    pub sources: Vec<PathBuf>,
    pub part_indices: Vec<usize>,
    pub arena: Model,
    pub arena_path: PathBuf,
    pub arena_offset: Vec3,
    pub origin: Vec3,
}
impl Content {
    pub fn load(root: &Path) -> Result<Self, String> {
        Self::load_file(root, None)
    }
    /// Shared native asset loading for both damage laboratories.
    pub fn load_file(root: &Path, selected: Option<&Path>) -> Result<Self, String> {
        let mut files = Vec::new();
        crate::content::scan(root, &mut files)?;
        let index: BTreeMap<_, _> = files
            .iter()
            .map(|p| {
                (
                    components::normalize(&p.strip_prefix(root).unwrap().to_string_lossy()),
                    p.clone(),
                )
            })
            .collect();
        let default_path = index
            .get("ai/team0/bot0.bot")
            .ok_or("Original Scout BOT not found")?;
        let robot_path = selected.unwrap_or(default_path);
        let robot =
            ra2_formats::bot::decode(&std::fs::read(robot_path).map_err(|e| e.to_string())?)?;
        let chassis = robot
            .chassis
            .model
            .clone()
            .ok_or("Scout chassis mesh missing")?;
        let catalog = ComponentCatalog::from_cfz(
            &std::fs::read(root.join("Components.cfz")).map_err(|e| e.to_string())?,
        )?;
        let importer = ModelImporters::with_ra2();
        let mut models = vec![chassis.clone()];
        let mut sources = vec![robot_path.to_path_buf()];
        let mut resolved = vec![ResolvedComponent {
            key: "@chassis".into(),
            model: Arc::new(chassis.clone()),
            hidden: false,
        }];
        let mut part_indices = Vec::new();
        let mut definitions = vec![ComponentDefinition::parse("")];
        for (i, component) in robot.components.iter().enumerate().skip(1) {
            let definition = catalog.get(&component.definition)?;
            definitions.push(definition.clone());
            let key = components::normalize(&definition.model_path()?);
            let path = index
                .get(&key)
                .ok_or_else(|| format!("Missing model {key}"))?;
            let model = importer
                .decode(&std::fs::read(path).map_err(|e| e.to_string())?)?
                .model;
            let hidden = component.class == "SmartZone";
            resolved.push(ResolvedComponent {
                key,
                model: Arc::new(model.clone()),
                hidden,
            });
            models.push(model);
            sources.push(path.clone());
            if !hidden {
                part_indices.push(i);
            }
        }
        let posed = ra2_formats::pose::freeze(&robot, &resolved, &definitions)?;
        let assembly = posed.assembly;
        models = posed
            .models
            .into_iter()
            .map(|m| (*m.model).clone())
            .collect();
        let arena_path = index
            .get("arenas/practice arena/practice_arena.gmf")
            .ok_or("Original practice arena missing")?
            .clone();
        let arena = importer
            .decode(&std::fs::read(&arena_path).map_err(|e| e.to_string())?)?
            .model;
        let arena = ra2_formats::scene::display_groups(&arena)?;
        let floor = arena
            .object("h_box")
            .and_then(|o| o.positions().map(|p| p.y).reduce(f32::min))
            .ok_or("Practice arena has no collision shell")?;
        let start = arena
            .attachments
            .iter()
            .find(|a| {
                a.properties
                    .get("type")
                    .is_some_and(|t| t.eq_ignore_ascii_case("startpoint"))
            })
            .map(|a| a.transform.w_axis.truncate())
            .unwrap_or(Vec3::ZERO);
        let arena_offset = Vec3::new(-start.x, -floor, -start.z);
        let origin = Vec3::new(0.0, -assembly.bounds.0.y + 0.06, 0.0);
        let health = ra2_formats::health::build(&robot, &definitions)?;
        Ok(Self {
            robot,
            health,
            assembly,
            chassis,
            models,
            sources,
            part_indices,
            arena,
            arena_path,
            arena_offset,
            origin,
        })
    }
    pub fn parts(&self) -> Result<Vec<AttachedPart>, String> {
        self.part_indices
            .iter()
            .map(|&i| {
                let shapes = arena_runtime::robot::collision_parts(&self.models[i]);
                let shape = if shapes.len() == 1 {
                    shapes[0].1.raw.clone()
                } else {
                    SharedShape::compound(
                        shapes
                            .into_iter()
                            .map(|(_, c)| (Pose::IDENTITY, c.raw))
                            .collect(),
                    )
                };
                Ok(AttachedPart {
                    transform: self.assembly.parts[i].transform,
                    shape,
                    mass: self.assembly.parts[i].mass,
                    parent: self.robot.components[i]
                        .parent
                        .and_then(|p| self.part_indices.iter().position(|&j| j == p)),
                })
            })
            .collect()
    }
    pub fn prepare(
        &self,
        meshes: &mut Assets<Mesh>,
        materials: &mut Assets<StandardMaterial>,
        images: &mut Assets<Image>,
    ) -> Result<PreparedContent, String> {
        let mut cache = TextureCache::default();
        let settings = RenderSettings {
            roughness: 0.55,
            ..default()
        };
        let arena = render::prepare_model(
            &self.arena,
            &FileTextures::new(self.arena_path.parent().unwrap()),
            &RenderSettings::default(),
            &mut cache,
            meshes,
            materials,
            images,
        )?;
        let mut parts = Vec::new();
        for &i in &self.part_indices {
            parts.push(render::prepare_model(
                &self.models[i],
                &FileTextures::new(self.sources[i].parent().unwrap()),
                &settings,
                &mut cache,
                meshes,
                materials,
                images,
            )?);
        }
        Ok(PreparedContent { arena, parts })
    }
}
#[derive(Resource)]
pub struct PreparedContent {
    pub arena: PreparedModel,
    pub parts: Vec<PreparedModel>,
}
