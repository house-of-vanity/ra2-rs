//! RA2 import boundary: bytes -> validated engine-independent assets + diagnostics.
//! No Bevy/Rapier dependency and no renderer-specific handles.
pub mod archive;
mod ascii;
pub mod assembly;
mod binary;
pub mod bot;
mod bot_write;
pub mod components;
pub mod gib;
pub mod giff;
pub mod mechanisms;
pub mod pose;
pub mod teams;

use arena_core::*;

#[derive(Debug, Default)]
pub struct ImportReport {
    pub format: &'static str,
    pub warnings: Vec<String>,
    /// Recognized source nodes not yet mapped to engine data (lights, old constraints, etc.).
    /// Exact bytes are retained so adding support later does not require guessing their layout.
    pub extensions: Vec<SourceExtension>,
}

#[derive(Debug)]
pub struct SourceExtension {
    pub node_type: u32,
    pub offset: usize,
    pub bytes: Vec<u8>,
}

#[derive(Debug)]
pub struct ImportedModel {
    pub model: Model,
    pub report: ImportReport,
}

/// New formats can implement this boundary without modifying the renderer or physics adapter.
pub trait ModelDecoder: Send + Sync {
    fn recognizes(&self, bytes: &[u8]) -> bool;
    fn decode(&self, bytes: &[u8]) -> Result<ImportedModel, String>;
}

pub struct GmfDecoder;
impl ModelDecoder for GmfDecoder {
    fn recognizes(&self, bytes: &[u8]) -> bool {
        bytes.starts_with(b"GMI") || bytes.starts_with(b"GMA")
    }
    fn decode(&self, bytes: &[u8]) -> Result<ImportedModel, String> {
        let mut imported = if bytes.starts_with(b"GMI") {
            binary::decode(bytes)?
        } else if bytes.starts_with(b"GMA") {
            ImportedModel {
                model: ascii::parse(
                    std::str::from_utf8(bytes).map_err(|_| "Invalid GMA text encoding")?,
                )?,
                report: ImportReport {
                    format: "GMA v3",
                    ..Default::default()
                },
            }
        } else {
            return Err("Unknown model signature (expected GMA or GMI)".into());
        };
        let mut hidden = std::collections::HashSet::new();
        for body in imported.model.all_bodies() {
            if body.disabled || body.use_display_proxy {
                hidden.insert(body.name.clone());
            }
            if !body.proxy.is_empty() && body.proxy != "(null)" {
                hidden.insert(body.proxy.clone());
            }
        }
        for object in &mut imported.model.objects {
            object.visible = !hidden.contains(&object.name);
            if let Some(mat) = imported.model.materials.get(&object.material)
                && object.faces.iter().any(|f| {
                    !mat.submaterials.is_empty() && !mat.submaterials.contains_key(&f.material)
                })
            {
                imported.report.warnings.push(format!("{} uses an undefined submaterial; runtime may use the first submaterial as fallback", object.name));
            }
        }
        imported.model.validate()?;
        Ok(imported)
    }
}

#[derive(Default)]
pub struct ModelImporters {
    decoders: Vec<Box<dyn ModelDecoder>>,
}
impl ModelImporters {
    pub fn with_ra2() -> Self {
        let mut registry = Self::default();
        registry.register(GmfDecoder);
        registry
    }
    pub fn register(&mut self, decoder: impl ModelDecoder + 'static) {
        self.decoders.push(Box::new(decoder));
    }
    pub fn decode(&self, bytes: &[u8]) -> Result<ImportedModel, String> {
        self.decoders
            .iter()
            .find(|d| d.recognizes(bytes))
            .ok_or("No decoder registered for this model")?
            .decode(bytes)
    }
}

fn reflect(v: Vec3) -> Vec3 {
    Vec3::new(v.x, v.y, -v.z)
}
fn right_handed(m: Mat4) -> Mat4 {
    let flip = Mat4::from_scale(Vec3::new(1.0, 1.0, -1.0));
    flip * m * flip
}
fn geometry(value: u32) -> CollisionGeometry {
    match value {
        1 => CollisionGeometry::Convex,
        2 => CollisionGeometry::TriangleMesh,
        3 => CollisionGeometry::TriangleMeshProxy,
        4 => CollisionGeometry::ConvexProxy,
        5 => CollisionGeometry::Box,
        v => CollisionGeometry::Other(v),
    }
}

fn properties(text: &str) -> std::collections::BTreeMap<String, String> {
    text.lines()
        .flat_map(|l| l.split(','))
        .filter_map(|line| {
            let (key, value) = line.split_once('=')?;
            Some((key.trim().to_ascii_lowercase(), value.trim().to_string()))
        })
        .collect()
}

pub mod armor;

pub mod health;

pub mod scene;
