//! Engine-independent asset data. No file formats, ECS, renderer or physics backend.
//! Coordinates are right-handed, Y-up; transforms and local geometry stay separate.
pub use glam::{Mat3, Mat4, Quat, Vec2, Vec3, Vec4};
pub mod damage;
pub mod editor;
pub mod mechanism;
pub mod robot;
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub struct Texture {
    pub file: String,
    pub scale: Vec2,
    pub offset: Vec2,
    pub rotation: f32,
    /// UV channel number, starting at 1 as in the source assets.
    pub channel: u32,
}

#[derive(Clone, Debug)]
pub struct Material {
    pub name: String,
    pub color: [f32; 3],
    pub alpha: f32,
    pub two_sided: bool,
    pub texture: Option<Texture>,
    pub opacity_texture: Option<Texture>,
    pub submaterials: BTreeMap<usize, Material>,
}

#[derive(Clone, Debug)]
pub struct Face {
    pub vertices: [usize; 3],
    pub material: usize,
    pub uv: [[f32; 2]; 3],
    pub normals: [Vec3; 3],
}

#[derive(Clone, Debug)]
pub struct UvChannel {
    pub vertices: Vec<Vec2>,
    pub faces: Vec<[usize; 3]>,
}

#[derive(Clone, Debug)]
pub struct Object {
    pub name: String,
    pub visible: bool,
    pub parent: Option<String>,
    /// Authored node transform; scene adapters flatten parent-local empty groups.
    /// Geometry parents otherwise describe ownership of already authored frames.
    /// Keeping local vertices allows later motion without rebuilding meshes.
    pub transform: Mat4,
    pub vertices: Vec<Vec3>,
    pub faces: Vec<Face>,
    pub material: usize,
    pub uv_channels: BTreeMap<u32, UvChannel>,
}

impl Object {
    pub fn positions(&self) -> impl Iterator<Item = Vec3> + '_ {
        self.vertices
            .iter()
            .map(|v| self.transform.transform_point3(*v))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CollisionGeometry {
    Convex,
    TriangleMesh,
    ConvexProxy,
    TriangleMeshProxy,
    Box,
    Other(u32),
}

#[derive(Clone, Debug)]
pub struct Body {
    pub name: String,
    pub transform: Mat4,
    pub mass: f32,
    pub geometry: CollisionGeometry,
    pub proxy: String,
    pub display_proxy: String,
    pub use_display_proxy: bool,
    pub disabled: bool,
    pub initially_sleeping: bool,
    pub friction: f32,
    pub restitution: f32,
    pub children: Vec<Body>,
}

#[derive(Clone, Debug)]
pub struct Attachment {
    pub name: String,
    pub transform: Mat4,
    /// Original extensible key/value data: attachment type, id, class, etc.
    pub properties: BTreeMap<String, String>,
}

#[derive(Clone, Debug)]
pub enum JointKind {
    Hinge {
        limited: bool,
        friction: f32,
        limits: [f32; 2],
    },
    Ball,
    Ragdoll {
        parameters: [f32; 8],
    },
    Slider {
        limits: [f32; 2],
    },
}

#[derive(Clone, Debug)]
pub struct ModelJoint {
    pub name: String,
    pub bodies: [String; 2],
    pub pivot: Vec3,
    pub axis: Vec3,
    pub kind: JointKind,
}

#[derive(Clone, Debug, Default)]
pub struct Model {
    pub materials: BTreeMap<usize, Material>,
    pub objects: Vec<Object>,
    pub bodies: Vec<Body>,
    pub attachments: Vec<Attachment>,
    pub joints: Vec<ModelJoint>,
}

impl Model {
    pub fn object(&self, name: &str) -> Option<&Object> {
        self.objects.iter().find(|o| o.name == name)
    }

    pub fn all_bodies(&self) -> Vec<&Body> {
        fn visit<'a>(bodies: &'a [Body], out: &mut Vec<&'a Body>) {
            for body in bodies {
                out.push(body);
                visit(&body.children, out);
            }
        }
        let mut out = vec![];
        visit(&self.bodies, &mut out);
        out
    }

    pub fn bounds(&self) -> Option<(Vec3, Vec3)> {
        let mut positions = self.objects.iter().flat_map(Object::positions);
        let first = positions.next()?;
        Some(positions.fold((first, first), |(min, max), p| (min.min(p), max.max(p))))
    }

    /// Used at the import boundary, before data reaches a renderer or physics engine.
    pub fn validate(&self) -> Result<(), String> {
        fn material_valid(material: &Material) -> bool {
            material.color.iter().all(|v| v.is_finite())
                && material.alpha.is_finite()
                && [&material.texture, &material.opacity_texture]
                    .iter()
                    .all(|t| {
                        t.as_ref().is_none_or(|t| {
                            t.scale.is_finite() && t.offset.is_finite() && t.rotation.is_finite()
                        })
                    })
                && material.submaterials.values().all(material_valid)
        }
        if !self.materials.values().all(material_valid) {
            return Err("Invalid material values".into());
        }
        for object in &self.objects {
            if !object.transform.is_finite() || object.vertices.iter().any(|v| !v.is_finite()) {
                return Err(format!("Non-finite geometry in {}", object.name));
            }
            for channel in object.uv_channels.values() {
                if channel.vertices.iter().any(|v| !v.is_finite())
                    || channel
                        .faces
                        .iter()
                        .flatten()
                        .any(|i| *i >= channel.vertices.len())
                {
                    return Err(format!("Invalid UV channel in {}", object.name));
                }
            }
            for face in &object.faces {
                if face.vertices.iter().any(|i| *i >= object.vertices.len()) {
                    return Err(format!("Vertex index outside {}", object.name));
                }
                if face.normals.iter().any(|n| !n.is_finite())
                    || face.uv.iter().flatten().any(|v| !v.is_finite())
                {
                    return Err(format!("Invalid normal/UV in {}", object.name));
                }
                let material = self.materials.get(&object.material).ok_or_else(|| {
                    format!("Missing material {} in {}", object.material, object.name)
                })?;
                // Old exporters can leave out-of-range submaterial IDs; the importer
                // reports them and the renderer applies an explicit fallback policy.
                let _ = material;
            }
        }
        for body in self.all_bodies() {
            if !body.transform.is_finite()
                || !body.mass.is_finite()
                || body.mass < 0.0
                || !body.friction.is_finite()
                || !body.restitution.is_finite()
            {
                return Err(format!("Invalid physical properties in {}", body.name));
            }
        }
        if self.attachments.iter().any(|a| !a.transform.is_finite()) {
            return Err("Invalid attachment transform".into());
        }
        Ok(())
    }
}

pub mod health;
