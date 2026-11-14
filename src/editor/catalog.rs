use crate::legacy_ui::assets::{LegacyAssets, decode_image};
use arena_core::Model;
use bevy::prelude::*;
use ra2_formats::{
    ModelImporters,
    components::{ComponentCatalog, ComponentDefinition, normalize},
};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Clone)]
pub struct ComponentAsset {
    pub key: String,
    pub reference: String,
    pub definition: ComponentDefinition,
    pub model: Arc<Model>,
    pub source: PathBuf,
    pub preview: String,
}
impl ComponentAsset {
    pub fn name(&self) -> &str {
        self.definition.value("name")
    }
    pub fn styles(&self) -> Vec<(String, String)> {
        let fields: Vec<_> = self
            .definition
            .value("styles")
            .split(',')
            .map(str::trim)
            .collect();
        let mut styles = vec![(
            fields
                .first()
                .copied()
                .filter(|s| !s.is_empty())
                .unwrap_or("Default")
                .into(),
            self.key.clone(),
        )];
        for pair in fields.get(1..).unwrap_or_default().as_chunks::<2>().0 {
            styles.push((
                pair[1].into(),
                normalize(&format!("components/styles/{}", pair[0])),
            ));
        }
        styles
    }
}
#[derive(Resource, Default)]
pub struct Catalog {
    pub root: PathBuf,
    pub components: BTreeMap<String, ComponentAsset>,
    pub index: BTreeMap<String, PathBuf>,
    pub collisions: BTreeMap<String, Vec<(String, bevy_rapier3d::prelude::Collider)>>,
}
impl Catalog {
    pub fn load(root: &Path, ui: &mut LegacyAssets) -> Result<Self, String> {
        let mut files = vec![];
        crate::fleet::scan(root, &mut files)?;
        let index: BTreeMap<_, _> = files
            .iter()
            .map(|p| {
                (
                    normalize(&p.strip_prefix(root).unwrap().to_string_lossy()),
                    p.clone(),
                )
            })
            .collect();
        let archive = ra2_formats::archive::CfzArchive::decode(
            &std::fs::read(root.join("Components.cfz")).map_err(|e| e.to_string())?,
        )?;
        let native_names = archive.names;
        let mut definitions = ComponentCatalog::from_cfz(
            &std::fs::read(root.join("Components.cfz")).map_err(|e| e.to_string())?,
        )?;
        for (key, path) in &index {
            if key.starts_with("components/") && key.ends_with(".txt") {
                definitions.definitions.insert(
                    key.clone(),
                    ComponentDefinition::parse(
                        &std::fs::read_to_string(path).map_err(|e| e.to_string())?,
                    ),
                );
            }
        }
        let mut models = BTreeMap::<String, Arc<Model>>::new();
        let mut components = BTreeMap::new();
        let decoder = ModelImporters::with_ra2();
        for (key, definition) in definitions.definitions {
            if definition.value("model").is_empty() {
                continue;
            }
            let model_key = normalize(&definition.model_path()?);
            let source = index
                .get(&model_key)
                .ok_or_else(|| format!("Missing component model {model_key}"))?
                .clone();
            let model = if let Some(model) = models.get(&model_key) {
                model.clone()
            } else {
                let model = Arc::new(
                    decoder
                        .decode(&std::fs::read(&source).map_err(|e| e.to_string())?)?
                        .model,
                );
                models.insert(model_key, model.clone());
                model
            };
            // Editing, placement proxies and the live garage use the same native
            // master-body frame as the static damage/benchmark scenes.
            let model = ra2_formats::pose::bind_model(&model, &definition)
                .map(Arc::new)
                .unwrap_or(model);
            let preview = normalize(&format!(
                "components/{}/{}",
                definition.value("dir"),
                definition.value("preview")
            ));
            if let Some(path) = index.get(&preview)
                && !ui.images.contains_key(&preview)
            {
                let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
                ui.images
                    .insert(preview.clone(), decode_image(&bytes, "bmp")?);
            }
            components.insert(
                key.clone(),
                ComponentAsset {
                    reference: native_names
                        .get(&key)
                        .cloned()
                        .or_else(|| {
                            index
                                .get(&key)
                                .and_then(|p| p.strip_prefix(root).ok())
                                .map(|p| p.to_string_lossy().into())
                        })
                        .unwrap_or_else(|| key.clone()),
                    key,
                    definition,
                    model,
                    source,
                    preview,
                },
            );
        }
        info!(
            "Workshop catalog: {} component definitions, {} shared models",
            components.len(),
            models.len()
        );
        let collisions = components
            .iter()
            .map(|(key, asset)| {
                (
                    key.clone(),
                    arena_runtime::robot::collision_parts(&asset.model),
                )
            })
            .collect();
        Ok(Self {
            collisions,
            root: root.into(),
            components,
            index,
        })
    }
    /// The same native initial-pose solver used by the demo. Saved BOT mounting
    /// records stay in their original local connector frames.
    pub fn freeze(
        &self,
        robot: &arena_core::robot::RobotDefinition,
    ) -> Result<ra2_formats::pose::FrozenRobot, String> {
        use ra2_formats::{assembly::ResolvedComponent, components::ComponentDefinition};
        let mut models = vec![ResolvedComponent {
            key: "@chassis".into(),
            model: Arc::new(robot.chassis.model.clone().ok_or("Build a chassis first")?),
            hidden: false,
        }];
        let mut definitions = vec![ComponentDefinition::parse("")];
        for component in robot.components.iter().skip(1) {
            let asset = self
                .components
                .get(&normalize(&component.definition))
                .ok_or_else(|| format!("Missing component {}", component.definition))?;
            models.push(ResolvedComponent {
                key: asset.key.clone(),
                model: asset.model.clone(),
                hidden: component.class == "SmartZone",
            });
            definitions.push(asset.definition.clone());
        }
        ra2_formats::pose::freeze(robot, &models, &definitions)
    }
    /// Freeze an uninstalled template in its default pose for the browser.
    pub fn preview(&self, key: &str) -> Result<ra2_formats::assembly::ResolvedComponent, String> {
        use arena_core::robot::*;
        let asset = self
            .components
            .get(key)
            .ok_or("Missing component template")?;
        if !asset.model.objects.iter().any(|o| o.visible) || asset.model.attachments.is_empty() {
            return Ok(ra2_formats::assembly::ResolvedComponent {
                key: asset.key.clone(),
                model: asset.model.clone(),
                hidden: false,
            });
        }
        let mut robot = arena_core::editor::Workshop::new().robot;
        robot.chassis.model = Some(Model::default());
        let mut settings = BTreeMap::new();
        if asset.definition.value("base") == "BurstMotor" {
            settings.insert("start_angle".into(), "0".into());
            settings.insert("stop_angle".into(), "2.4".into());
        }
        robot.components.push(RobotComponent {
            parent: Some(0),
            parent_mount: 0,
            own_mount: super::placement::default_socket(&asset.model),
            mounting: MountPlacement::default(),
            class: asset.definition.value("base").into(),
            definition: asset.reference.clone(),
            settings,
        });
        self.freeze(&robot)?
            .models
            .pop()
            .ok_or("Missing component preview".into())
    }
    pub fn styles_for(&self, key: &str) -> Vec<(String, String)> {
        self.components
            .values()
            .find_map(|a| {
                let styles = a.styles();
                (styles.len() > 1 && styles.iter().any(|(_, k)| k == key)).then_some(styles)
            })
            .unwrap_or_else(|| vec![("Default".into(), key.into())])
    }
    pub fn browser(&self, category: &str) -> Vec<&ComponentAsset> {
        let mut items: Vec<_> = self
            .components
            .values()
            .filter(|a| {
                a.definition.value("type") == category
                    && a.definition.value("hidden").parse::<u32>().unwrap_or(0) == 0
                    && !a.key.contains("/styles/")
            })
            .collect();
        items.sort_by(|a, b| {
            let sort =
                |a: &ComponentAsset| a.definition.value("sort").parse::<i32>().unwrap_or(100);
            sort(a).cmp(&sort(b)).then_with(|| a.name().cmp(b.name()))
        });
        items
    }
}

#[cfg(test)]
mod pose_tests {
    use super::*;
    #[test]
    #[ignore = "requires the original robot corpus"]
    fn original_editor_pose_matches_demo_for_entire_robot_corpus() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../RobotArena2");
        let catalog = Catalog::load(&root, &mut LegacyAssets::default()).unwrap();
        let mut files = vec![];
        crate::fleet::scan(&root, &mut files).unwrap();
        let mut count = 0;
        for path in files
            .iter()
            .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("bot")))
        {
            let demo = crate::destruction::content::Content::load_file(&root, Some(path)).unwrap();
            let before = ra2_formats::bot::encode(&demo.robot).unwrap();
            let frozen = catalog.freeze(&demo.robot).unwrap();
            for (i, (actual, expected)) in frozen
                .assembly
                .parts
                .iter()
                .zip(&demo.assembly.parts)
                .enumerate()
            {
                assert!(
                    actual.transform.abs_diff_eq(expected.transform, 0.0001),
                    "{} component {i}: editor/demo frame",
                    demo.robot.name
                );
                for (a, b) in frozen.models[i]
                    .model
                    .objects
                    .iter()
                    .zip(&demo.models[i].objects)
                {
                    assert!(
                        a.transform.abs_diff_eq(b.transform, 0.0001),
                        "{} component {i}: display/proxy frame",
                        demo.robot.name
                    );
                    assert_eq!(a.vertices, b.vertices);
                }
                for (a, b) in frozen.models[i]
                    .model
                    .attachments
                    .iter()
                    .zip(&demo.models[i].attachments)
                {
                    assert!(
                        a.transform.abs_diff_eq(b.transform, 0.0001),
                        "{} component {i}: socket frame",
                        demo.robot.name
                    );
                }
            }
            assert_eq!(ra2_formats::bot::encode(&demo.robot).unwrap(), before);
            count += 1;
        }
        for key in catalog.components.keys() {
            catalog
                .preview(key)
                .unwrap_or_else(|e| panic!("{key}: {e}"));
        }
        println!(
            "EDITOR/DEMO POSE CORPUS PASSED: {count} robots; {} template previews",
            catalog.components.len()
        );
        assert!(count >= 58);
    }
}
