//! Native BOT catalogue and CPU-only preparation for the rigid damage lab.
use super::*;
use crate::destruction::content::Content;
use arena_core::damage::{DamageBox, DentMesh};
use std::path::{Path, PathBuf};

pub struct Entry {
    pub name: String,
    pub path: PathBuf,
}

#[derive(Resource)]
pub struct Catalogue {
    pub root: PathBuf,
    pub entries: Vec<Entry>,
}

impl Catalogue {
    pub fn scan(root: &Path) -> Result<Self, String> {
        let mut files = Vec::new();
        crate::content::scan(root, &mut files)?;
        let mut entries = Vec::new();
        for path in files
            .into_iter()
            .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("bot")))
        {
            match std::fs::read(&path)
                .map_err(|e| e.to_string())
                .and_then(|b| ra2_formats::bot::decode(&b))
            {
                Ok(robot) => entries.push(Entry {
                    name: robot.name,
                    path,
                }),
                Err(e) => warn!("Cannot index {}: {e}", path.display()),
            }
        }
        entries.sort_by(|a, b| {
            a.name
                .to_lowercase()
                .cmp(&b.name.to_lowercase())
                .then(a.path.cmp(&b.path))
        });
        if entries.is_empty() {
            return Err("No readable BOT files in game directory".into());
        }
        Ok(Self {
            root: root.into(),
            entries,
        })
    }
    pub fn matching(&self, filter: &str) -> Vec<usize> {
        let filter = filter.to_lowercase();
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                e.name.to_lowercase().contains(&filter)
                    || e.path.to_string_lossy().to_lowercase().contains(&filter)
            })
            .map(|(i, _)| i)
            .collect()
    }
}

pub struct Loaded {
    pub content: Content,
    pub shell: DentMesh,
    pub paint: paint::Paint,
    pub density: usize,
    pub boxes: Vec<DamageBox>,
    pub picks: Vec<(usize, [Vec3; 3])>,
}

impl Loaded {
    pub fn load(root: &Path, path: &Path) -> Result<Self, String> {
        Self::load_with_density(root, path, DEFAULT_DENSITY)
    }

    pub fn load_with_density(root: &Path, path: &Path, density: usize) -> Result<Self, String> {
        let factor = MESH_DENSITIES.get(density).ok_or("Unknown mesh density")?;
        let content = Content::load_file(root, Some(path))?;
        let shell = DentMesh::from_model_with_density(&content.chassis, 0.35, *factor)?;
        let paint = paint::Paint::new(content.robot.chassis.texture.as_deref())?;
        let mut boxes = Vec::new();
        let mut picks = Vec::new();
        for &i in &content.part_indices {
            let model = &content.models[i];
            let points: Vec<_> = model
                .objects
                .iter()
                .filter(|o| o.visible)
                .flat_map(|o| o.positions())
                .collect();
            if points.is_empty() {
                continue;
            }
            let min = points
                .iter()
                .copied()
                .fold(Vec3::splat(f32::INFINITY), Vec3::min);
            let max = points
                .iter()
                .copied()
                .fold(Vec3::splat(f32::NEG_INFINITY), Vec3::max);
            let extent = (max - min).max(Vec3::splat(0.02));
            let center = (min + max) * 0.5;
            boxes.push(DamageBox {
                component: i,
                name: format!("{}: {}", i, content.robot.components[i].definition),
                transform: content.assembly.parts[i].transform,
                min: center - extent * 0.5,
                max: center + extent * 0.5,
                damage: 0.0,
                intrusion: 0.0,
            });
            for object in model.objects.iter().filter(|o| o.visible) {
                let transform = content.assembly.parts[i].transform * object.transform;
                for face in &object.faces {
                    picks.push((
                        i,
                        face.vertices
                            .map(|v| transform.transform_point3(object.vertices[v])),
                    ));
                }
            }
        }
        Ok(Self {
            content,
            shell,
            paint,
            density,
            boxes,
            picks,
        })
    }
}
