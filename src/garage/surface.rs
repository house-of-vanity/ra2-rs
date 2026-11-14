//! The garage uses the same cheap permanent dent geometry as the damage lab.
//! Physics remains rigid; mesh upload happens only on an actual damaging impact.
use super::*;
use arena_core::{
    Face, Model, Object,
    damage::{DentMesh, SurfaceHit},
};
use arena_runtime::dent_render::SmoothingGroups;
use bevy_rapier3d::parry::{query::PointQuery, shape::Triangle};

pub(super) struct Shell {
    pub geometry: DentMesh,
    groups: SmoothingGroups,
    pub meshes: Vec<Handle<Mesh>>,
    pub revision: u64,
}
impl Shell {
    pub fn new(model: &Model) -> Result<Self, String> {
        let geometry = DentMesh::from_model(model, 0.25)?;
        let groups = SmoothingGroups::new(&geometry);
        Ok(Self {
            geometry,
            groups,
            meshes: vec![],
            revision: 0,
        })
    }
    pub fn model(&self, source: &Model) -> Model {
        let mut result = source.clone();
        let material = source
            .objects
            .iter()
            .find(|o| o.visible)
            .map_or(0, |o| o.material);
        result.objects = vec![Object {
            name: "Chassis skin".into(),
            visible: true,
            parent: None,
            transform: Mat4::IDENTITY,
            vertices: self.geometry.positions.clone(),
            faces: self
                .geometry
                .triangles
                .iter()
                .map(|t| Face {
                    vertices: t.nodes,
                    material: 0,
                    uv: t.uv.map(|p| p.to_array()),
                    normals: [t.normal; 3],
                })
                .collect(),
            material,
            uv_channels: default(),
        }];
        result
    }
    pub fn impact(
        &mut self,
        point: Vec3,
        outward: Vec3,
        damage: f32,
        meshes: &mut Assets<Mesh>,
    ) -> bool {
        if damage < 1. {
            return false;
        }
        let g = &self.geometry;
        let hit = g
            .triangles
            .iter()
            .enumerate()
            .filter_map(|(i, t)| {
                if t.normal.dot(outward) < 0.2 {
                    return None;
                }
                let [a, b, c] = t.nodes.map(|n| g.positions[n]);
                let p = Triangle::new(a, b, c)
                    .project_local_point(point, true)
                    .point;
                let u = b - a;
                let v = c - a;
                let w = p - a;
                let denominator = u.dot(u) * v.dot(v) - u.dot(v).powi(2);
                if denominator.abs() < 1e-12 {
                    return None;
                }
                let y = (w.dot(u) * v.dot(v) - w.dot(v) * u.dot(v)) / denominator;
                let z = (w.dot(v) * u.dot(u) - w.dot(u) * u.dot(v)) / denominator;
                let rest = t.nodes.map(|n| g.rest[n]);
                Some((
                    p.distance_squared(point),
                    SurfaceHit {
                        triangle: i,
                        point: p,
                        rest_point: rest[0] * (1. - y - z) + rest[1] * y + rest[2] * z,
                        normal: t.normal,
                        distance: 0.,
                    },
                ))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, hit)| hit);
        let Some(hit) = hit else {
            return false;
        };
        let radius = (0.25 + damage.sqrt() * 0.035).clamp(0.25, 0.65);
        let depth = (damage * 0.0015).clamp(0.015, 0.12);
        let limit = ((g.bounds.1 - g.bounds.0).min_element() * 0.7).clamp(0.05, 0.35);
        let changed = self
            .geometry
            .dent(hit, radius, depth, limit, 0., &mut [])
            .changed_nodes;
        if changed == 0 {
            return false;
        }
        for handle in &self.meshes {
            if let Some(mut mesh) = meshes.get_mut(handle) {
                arena_runtime::dent_render::update(&mut mesh, &self.geometry, true, &self.groups);
            }
        }
        self.revision += 1;
        true
    }
}
