//! Backend-independent robot blueprints and resolved assemblies.
use crate::{Mat4, Model, Quat, Vec2, Vec3};
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub struct RobotComponent {
    pub parent: Option<usize>,
    pub parent_mount: u32,
    pub own_mount: u32,
    pub mounting: MountPlacement,
    pub class: String,
    pub definition: String,
    pub settings: BTreeMap<String, String>,
}

/// Editable placement between attachment frames, in engine coordinates.
#[derive(Clone, Copy, Debug)]
pub struct MountPlacement {
    pub position: Vec3,
    pub rotation: Quat,
    pub twist: f32,
    pub offset: f32,
}
impl Default for MountPlacement {
    fn default() -> Self {
        Self {
            position: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            twist: 0.0,
            offset: 0.0,
        }
    }
}
impl MountPlacement {
    pub fn matrix(&self) -> Mat4 {
        Mat4::from_rotation_translation(self.rotation, self.position)
            * Mat4::from_translation(Vec3::Z * self.offset)
            * Mat4::from_rotation_z(self.twist)
    }
}

#[derive(Clone, Debug, Default)]
pub struct ChassisPaintLayout {
    pub group_uvs: Vec<Vec<Vec2>>,
    pub rectangles: Vec<[i32; 5]>,
    pub face_groups: Vec<[i32; 4]>,
    pub top_corners: Vec<[i32; 5]>,
}

#[derive(Clone, Debug)]
pub struct Chassis {
    pub bottom: Vec<Vec2>,
    pub top: Vec<Vec2>,
    pub height: f32,
    pub model: Option<Model>,
    pub texture: Option<Vec<u8>>,
    pub armour: u32,
    pub surface_density: f32,
    pub hardness: f32,
    pub buckling_points: Vec<(Vec3, Vec<usize>)>,
    pub face_count: usize,
    pub paint_layout: ChassisPaintLayout,
    pub material_parameters: [f32; 5],
    pub armour_flags: [f32; 2],
}

#[derive(Clone, Debug)]
pub struct Controller {
    pub name: String,
    pub position: [i32; 2],
    pub kind: u32,
    pub keys: Vec<Vec<i32>>,
}

#[derive(Clone, Debug)]
pub struct ControlBinding {
    pub component: usize,
    pub action: String,
    pub controller: [i32; 2],
    pub key: usize,
}

#[derive(Clone, Debug)]
pub struct RobotDefinition {
    pub name: String,
    pub source_version: String,
    pub weight_class: u32,
    pub components: Vec<RobotComponent>,
    pub chassis: Chassis,
    pub snapshot: Option<Vec<u8>>,
    pub damaged_model: Option<Model>,
    pub heading: f32,
    pub controllers: Vec<Controller>,
    pub bindings: Vec<ControlBinding>,
    pub default_paint: i32,
}

#[derive(Clone, Debug)]
pub struct AssemblyPart {
    pub component: usize,
    pub model: String,
    pub transform: Mat4,
    pub mass: f32,
    pub visible: bool,
}

#[derive(Clone, Debug)]
pub struct RobotAssembly {
    pub name: String,
    pub parts: Vec<AssemblyPart>,
    pub mass: f32,
    pub bounds: (Vec3, Vec3),
}
