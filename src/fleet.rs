//! Application-level catalog, asset sharing and the all-robots benchmark scene.
use crate::{Options, scene::Scene, simulation::Home};
use arena_core::{
    Model,
    robot::{RobotAssembly, RobotDefinition},
};
use arena_runtime::{
    content::{FileTextures, TextureBytes, TextureSource},
    render::{self, PreparedModel, RenderSettings, TextureCache},
    robot,
};
use bevy::prelude::*;
use bevy_rapier3d::prelude::*;
use ra2_formats::{
    ModelImporters,
    assembly::ResolvedComponent,
    bot,
    components::{self, ComponentCatalog, ComponentDefinition},
};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};

pub struct RobotAsset {
    pub model: Arc<Model>,
    pub source: PathBuf,
    pub paint: Option<Vec<u8>>,
}
pub struct LoadedRobot {
    pub source: PathBuf,
    pub definition: RobotDefinition,
    pub assembly: RobotAssembly,
}

#[derive(Resource, Default)]
pub struct Fleet {
    pub robots: Vec<LoadedRobot>,
    pub models: BTreeMap<String, RobotAsset>,
    pub spacing: f32,
    pub copies: usize,
    pub load_seconds: f32,
}
#[derive(Resource, Default)]
pub struct FleetStats {
    pub robots: usize,
    pub components: usize,
    pub unique_models: usize,
    pub meshes: usize,
    pub triangles: usize,
    pub textures: usize,
    pub missing: usize,
}
#[derive(Component)]
pub struct ImportedRobot {
    pub index: usize,
    pub copy: usize,
}

pub fn scan(path: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
    for entry in std::fs::read_dir(path).map_err(|e| format!("{}: {e}", path.display()))? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.is_dir() {
            scan(&path, files)?;
        } else {
            files.push(path);
        }
    }
    Ok(())
}
impl Fleet {
    pub fn load(options: &Options) -> Result<Self, String> {
        if !options.all_bots {
            return Ok(Self::default());
        }
        let started = Instant::now();
        let mut files = vec![];
        scan(&options.game_dir, &mut files)?;
        files.sort();
        let index: BTreeMap<_, _> = files
            .iter()
            .map(|p| {
                (
                    components::normalize(
                        &p.strip_prefix(&options.game_dir).unwrap().to_string_lossy(),
                    ),
                    p.clone(),
                )
            })
            .collect();
        let mut catalog = ComponentCatalog::from_cfz(
            &std::fs::read(options.game_dir.join("Components.cfz")).map_err(|e| e.to_string())?,
        )?;
        for (key, path) in &index {
            if key.starts_with("components/") && key.ends_with(".txt") {
                catalog.definitions.insert(
                    key.clone(),
                    ComponentDefinition::parse(
                        &std::fs::read_to_string(path).map_err(|e| e.to_string())?,
                    ),
                );
            }
        }
        let importers = ModelImporters::with_ra2();
        let mut models = BTreeMap::new();
        let mut robots = vec![];
        for path in files
            .iter()
            .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("bot")))
        {
            let definition = bot::decode(&std::fs::read(path).map_err(|e| e.to_string())?)
                .map_err(|e| format!("{}: {e}", path.display()))?;
            let chassis_key = format!("{}#chassis", path.display());
            let chassis = definition.chassis.model.clone().ok_or_else(|| {
                format!(
                    "{}: chassis without embedded mesh is not yet supported",
                    path.display()
                )
            })?;
            models.insert(
                chassis_key.clone(),
                RobotAsset {
                    model: Arc::new(chassis),
                    source: path.clone(),
                    paint: definition.chassis.texture.clone(),
                },
            );
            let mut resolved = vec![ResolvedComponent {
                key: chassis_key.clone(),
                model: models[&chassis_key].model.clone(),
                hidden: false,
            }];
            let mut definitions = vec![ComponentDefinition::parse("")];
            for c in definition.components.iter().skip(1) {
                definitions.push(catalog.get(&c.definition)?.clone());
                let def = catalog.get(&c.definition)?;
                let key = components::normalize(&def.model_path()?);
                if !models.contains_key(&key) {
                    let source = index
                        .get(&key)
                        .ok_or_else(|| format!("Missing model {key} for {}", definition.name))?
                        .clone();
                    let model = importers
                        .decode(&std::fs::read(&source).map_err(|e| e.to_string())?)?
                        .model;
                    models.insert(
                        key.clone(),
                        RobotAsset {
                            model: Arc::new(model),
                            source,
                            paint: None,
                        },
                    );
                }
                resolved.push(ResolvedComponent {
                    model: models[&key].model.clone(),
                    key,
                    hidden: c.class == "SmartZone",
                });
            }
            let posed = ra2_formats::pose::freeze(&definition, &resolved, &definitions)?;
            for (authored, component) in resolved.iter().zip(&posed.models) {
                if component.key != authored.key {
                    let source = models[&authored.key].source.clone();
                    models.insert(
                        component.key.clone(),
                        RobotAsset {
                            model: component.model.clone(),
                            source,
                            paint: None,
                        },
                    );
                }
            }
            let assembly = posed.assembly;
            println!(
                "BOT {}: {} components, {:.1} kg, bounds {:?}..{:?}",
                definition.name,
                definition.components.len(),
                assembly.mass,
                assembly.bounds.0,
                assembly.bounds.1
            );
            robots.push(LoadedRobot {
                source: path.clone(),
                definition,
                assembly,
            });
        }
        if robots.is_empty() {
            return Err("No .bot files found".into());
        }
        let used = robots
            .iter()
            .flat_map(|r| r.assembly.parts.iter().map(|p| p.model.clone()))
            .collect::<std::collections::BTreeSet<_>>();
        models.retain(|key, _| used.contains(key));
        let spacing = robots
            .iter()
            .map(|r| {
                let size = r.assembly.bounds.1 - r.assembly.bounds.0;
                size.x.max(size.z) + 2.0
            })
            .fold(6.5, f32::max);
        let fleet = Self {
            robots,
            models,
            spacing,
            copies: options.bot_copies,
            load_seconds: started.elapsed().as_secs_f32(),
        };
        println!(
            "Fleet: {} source robots, {} shared models, {} copies, spacing {:.1} m, loaded in {:.3}s",
            fleet.robots.len(),
            fleet.models.len(),
            fleet.copies,
            fleet.spacing,
            fleet.load_seconds
        );
        Ok(fleet)
    }
    pub fn arena_half_size(&self) -> f32 {
        32.0_f32.max(
            ((self.robots.len() * self.copies) as f32).sqrt().ceil() * self.spacing / 2.0 + 6.0,
        )
    }
    pub fn placement(&self, index: usize) -> Vec3 {
        let count = self.robots.len() * self.copies;
        let cols = (count as f32).sqrt().ceil() as usize;
        let rows = count.div_ceil(cols);
        let robot = &self.robots[index % self.robots.len()];
        Vec3::new(
            (index % cols) as f32 - (cols - 1) as f32 / 2.0,
            0.0,
            (index / cols) as f32 - (rows - 1) as f32 / 2.0,
        ) * self.spacing
            + Vec3::new(0.0, 0.35 - robot.assembly.bounds.0.y, 0.0)
    }
}

struct RobotTextures<'a> {
    asset: &'a RobotAsset,
    key: &'a str,
}
impl TextureSource for RobotTextures<'_> {
    fn read(&self, name: &str) -> Result<TextureBytes, String> {
        if name == "@chassis.tga" {
            let mut bytes = self.asset.paint.clone().ok_or("Missing embedded paint")?;
            // Opaque in the old renderer, even though saved TGA pixels have alpha=0.
            // Correct only the GPU copy; BOT data must remain unchanged for saving.
            for pixel in bytes[18..].as_chunks_mut::<4>().0 {
                pixel[3] = 255;
            }
            return Ok(TextureBytes {
                key: format!("{}#paint", self.key),
                extension: "tga".into(),
                bytes,
            });
        }
        FileTextures::new(self.asset.source.parent().unwrap()).read(name)
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "Bevy injects independent ECS resources"
)]
pub fn spawn(
    mut commands: Commands,
    fleet: Res<Fleet>,
    options: Res<Options>,
    scene: Res<Scene>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    let mut stats = FleetStats::default();
    if fleet.robots.is_empty() {
        commands.insert_resource(stats);
        return;
    }
    let settings = RenderSettings {
        roughness: scene.config.rendering.roughness,
        opacity_map_alpha: scene.config.rendering.opacity_map_alpha,
    };
    let mut texture_cache = TextureCache::default();
    let mut prepared = BTreeMap::<String, PreparedModel>::new();
    let mut hulls = BTreeMap::new();
    for (key, asset) in &fleet.models {
        let source = RobotTextures { asset, key };
        let model = render::prepare_model(
            &asset.model,
            &source,
            &settings,
            &mut texture_cache,
            &mut meshes,
            &mut materials,
            &mut images,
        )
        .expect("Validated robot model");
        stats.textures += model.stats.textures;
        stats.missing += model.stats.missing;
        prepared.insert(key.clone(), model);
        if options.bot_physics
            && let Some(hull) = robot::component_hull(&asset.model)
        {
            hulls.insert(key.clone(), hull);
        }
    }
    let collision: Vec<_> = fleet
        .robots
        .iter()
        .map(|r| {
            if options.bot_physics {
                Some(robot::compound(&r.assembly, &hulls).expect("Validated robot collider"))
            } else {
                None
            }
        })
        .collect();
    for slot in 0..fleet.robots.len() * fleet.copies {
        let index = slot % fleet.robots.len();
        let copy = slot / fleet.robots.len();
        let robot = &fleet.robots[index];
        let pose = Transform::from_translation(fleet.placement(slot))
            .with_rotation(Quat::from_rotation_y(robot.definition.heading));
        let root = commands
            .spawn((
                ImportedRobot { index, copy },
                Name::new(robot.definition.name.clone()),
                pose,
                Visibility::default(),
            ))
            .id();
        if let Some(collider) = &collision[index] {
            commands.entity(root).insert((
                RigidBody::Dynamic,
                collider.clone(),
                ColliderMassProperties::Mass(robot.assembly.mass),
                Home(pose),
                Velocity::zero(),
                Sleeping::default(),
                Friction::coefficient(0.8),
                Restitution::coefficient(0.05),
                Damping {
                    linear_damping: 0.15,
                    angular_damping: 0.4,
                },
                Ccd::enabled(),
            ));
        }
        for part in robot.assembly.parts.iter().filter(|p| p.visible) {
            let model = &prepared[&part.model];
            let entity = model.spawn(
                &mut commands,
                &format!("{} part {}", robot.definition.name, part.component),
                Transform::from_matrix(part.transform),
            );
            commands.entity(entity).insert(ChildOf(root));
            stats.meshes += model.stats.meshes;
            stats.triangles += model.stats.triangles;
        }
        stats.components += robot.definition.components.len();
        stats.robots += 1;
    }
    stats.unique_models = prepared.len();
    stats.textures = texture_cache.len();
    println!(
        "Fleet rendering: {} robots, {} components, {} meshes, {} triangles, {} textures, {} missing; physics {}",
        stats.robots,
        stats.components,
        stats.meshes,
        stats.triangles,
        stats.textures,
        stats.missing,
        if options.bot_physics {
            "compound rigid bodies"
        } else {
            "off"
        }
    );
    commands.insert_resource(stats);
}
