//! Deformable armour adapter. Native asset data stays independent of Rapier and the demo UI.
//! The hollow shell uses conforming tetrahedral prisms, not a solid fill of the robot's interior.
use arena_core::{Model, Vec2, Vec3};
use bevy_rapier3d::rapier::prelude::*;
use std::collections::BTreeMap;
pub mod world;

#[derive(Clone, Copy, Debug)]
pub struct Quality {
    pub name: &'static str,
    pub subdivisions: usize,
    pub iterations: usize,
}
pub const QUALITIES: [Quality; 4] = [
    Quality {
        name: "LOW",
        subdivisions: 1,
        iterations: 2,
    },
    Quality {
        name: "MEDIUM",
        subdivisions: 2,
        iterations: 3,
    },
    Quality {
        name: "HIGH",
        subdivisions: 3,
        iterations: 4,
    },
    Quality {
        name: "ULTRA",
        subdivisions: 5,
        iterations: 6,
    },
];

#[derive(Clone)]
pub struct PaintTriangle {
    pub points: [Vec3; 3],
    pub uv: [Vec2; 3],
    pub normal: Vec3,
}
#[derive(Clone)]
pub struct ArmourShell {
    pub positions: Vec<Vec3>,
    pub cells: Vec<[u32; 4]>,
    pub paint: Vec<PaintTriangle>,
    /// The unweighted centre at construction; `.mass()` gives every particle equal mass.
    pub rest_center: Vec3,
}
fn key(p: Vec3) -> [i32; 3] {
    p.to_array().map(|v| (v * 100_000.0).round() as i32)
}
impl ArmourShell {
    pub fn from_model(model: &Model, quality: Quality, thickness: f32) -> Result<Self, String> {
        if !thickness.is_finite() || thickness <= 0.0 || !(1..=8).contains(&quality.subdivisions) {
            return Err("Invalid armour meshing parameters".into());
        }
        let mut paint = Vec::new();
        let mut normal_sums = BTreeMap::<[i32; 3], Vec3>::new();
        for object in model.objects.iter().filter(|o| o.visible) {
            for face in &object.faces {
                let points = face
                    .vertices
                    .map(|i| object.transform.transform_point3(object.vertices[i]));
                let normal = (points[1] - points[0])
                    .cross(points[2] - points[0])
                    .normalize_or_zero();
                if normal == Vec3::ZERO {
                    continue;
                }
                for point in points {
                    *normal_sums.entry(key(point)).or_default() += normal;
                }
                paint.push(PaintTriangle {
                    points,
                    uv: face.uv.map(Vec2::from_array),
                    normal,
                });
            }
        }
        if paint.is_empty() {
            return Err("Chassis has no visible triangles".into());
        }
        for normal in normal_sums.values_mut() {
            *normal = normal.normalize_or_zero();
        }
        let n = quality.subdivisions;
        let mut positions = Vec::new();
        let mut index = BTreeMap::new();
        let mut triangles = Vec::new();
        for face in &paint {
            let normals = face.points.map(|p| normal_sums[&key(p)]);
            let mut row = BTreeMap::new();
            for i in 0..=n {
                for j in 0..=n - i {
                    let w = Vec3::new(
                        1.0 - (i + j) as f32 / n as f32,
                        i as f32 / n as f32,
                        j as f32 / n as f32,
                    );
                    let point = face.points[0] * w.x + face.points[1] * w.y + face.points[2] * w.z;
                    let normal = (normals[0] * w.x + normals[1] * w.y + normals[2] * w.z)
                        .normalize_or_zero();
                    let next = (positions.len() / 2) as u32;
                    let id = *index.entry(key(point)).or_insert_with(|| {
                        positions.push(point);
                        positions.push(point - normal * thickness);
                        next
                    });
                    row.insert((i, j), id);
                }
            }
            for i in 0..n {
                for j in 0..n - i {
                    triangles.push([row[&(i, j)], row[&(i + 1, j)], row[&(i, j + 1)]]);
                    if i + j + 1 < n {
                        triangles.push([row[&(i + 1, j)], row[&(i + 1, j + 1)], row[&(i, j + 1)]]);
                    }
                }
            }
        }
        let mut cells = Vec::new();
        for mut t in triangles {
            // A global ordering gives both neighbouring prisms the same diagonal on their seam.
            t.sort_unstable();
            let [a, b, c] = t.map(|i| i * 2);
            for cell in [
                [a, b, c, c + 1],
                [a, b, b + 1, c + 1],
                [a, a + 1, b + 1, c + 1],
            ] {
                let p = cell.map(|i| positions[i as usize]);
                let volume = (p[1] - p[0]).cross(p[2] - p[0]).dot(p[3] - p[0]).abs() / 6.0;
                if volume > 1.0e-9 {
                    cells.push(cell);
                }
            }
        }
        if cells.is_empty() {
            return Err("Chassis shell has zero volume".into());
        }
        let rest_center = positions.iter().sum::<Vec3>() / positions.len() as f32;
        Ok(Self {
            positions,
            cells,
            paint,
            rest_center,
        })
    }

    pub fn builder(
        &self,
        origin: Vec3,
        mass: f32,
        fem: bool,
        iterations: usize,
    ) -> SoftBodyBuilder {
        SoftBodyBuilder::new(self.positions.iter().map(|p| *p + origin).collect())
            .cells(self.cells.clone())
            .cell_model(if fem {
                SoftBodyCellModel::Corotational
            } else {
                SoftBodyCellModel::Volume
            })
            .solver(if fem {
                SoftBodySolver::Fem
            } else {
                SoftBodySolver::Constraints
            })
            .mass(mass.max(5.0))
            .can_sleep(false)
            .linear_damping(0.7)
            .material(SoftBodyMaterial {
                young_modulus: 2.5e6,
                edge_softness: SpringCoefficients::new(120.0, 1.0),
                volume_softness: SpringCoefficients::new(100.0, 1.0),
                edge_plastic_yield: 0.06,
                edge_plastic_creep: 25.0,
                edge_plastic_max: 0.7,
                poisson_ratio: 0.28,
                elastic_damping_ratio: 0.9,
                deformation_damping: 8.0,
                plastic_yield: 0.035,
                plastic_creep: 35.0,
                plastic_max: 0.8,
                // Ductile armour yields without spontaneously shredding under equipment loads.
                // Saw cuts open cracks; the mount graph breaks on physical impulses.
                tear_strain: None,
                tear_smoothing: 0.04,
                min_piece: Some(4),
                max_tears_per_step: 8,
                ..Default::default()
            })
            .particle_radius(0.012)
            .additional_pgs_iterations(iterations)
            .surface_collider(
                ColliderBuilder::ball(0.01)
                    .friction(0.65)
                    .restitution(0.02)
                    .collision_groups(InteractionGroups::new(
                        Group::GROUP_2,
                        Group::GROUP_1,
                        InteractionTestMode::And,
                    )),
            )
    }

    /// Cache this correspondence on topology changes. Per-frame rendering only gathers positions.
    pub fn surface(&self, body: &SoftBody) -> Vec<SurfaceTriangle> {
        body.boundary()
            .iter()
            .map(|&ids| {
                let rest = ids.map(|i| {
                    body.particles()[i as usize].initial_rest_position() + self.rest_center
                });
                let normal = (rest[1] - rest[0])
                    .cross(rest[2] - rest[0])
                    .normalize_or_zero();
                let center = rest.iter().sum::<Vec3>() / 3.0;
                let closest = self
                    .paint
                    .iter()
                    .filter(|f| normal.dot(f.normal) > 0.7)
                    .filter_map(|f| {
                        let distance = (center - f.points[0]).dot(f.normal).abs();
                        (distance < 0.005).then_some((distance, f))
                    })
                    .min_by(|a, b| a.0.total_cmp(&b.0));
                let uv = closest.map(|(_, f)| {
                    rest.map(|p| {
                        let w = barycentric(f.points, p);
                        f.uv[0] * w.x + f.uv[1] * w.y + f.uv[2] * w.z
                    })
                });
                SurfaceTriangle { ids, uv }
            })
            .collect()
    }
}

pub struct SurfaceTriangle {
    pub ids: [u32; 3],
    /// None marks the interior metal or a newly exposed fracture face.
    pub uv: Option<[Vec2; 3]>,
}
fn barycentric(t: [Vec3; 3], p: Vec3) -> Vec3 {
    let (v0, v1, v2) = (t[1] - t[0], t[2] - t[0], p - t[0]);
    let (a, b, c, d, e) = (v0.dot(v0), v0.dot(v1), v1.dot(v1), v2.dot(v0), v2.dot(v1));
    let denominator = a * c - b * b;
    if denominator.abs() < 1.0e-12 {
        return Vec3::X;
    }
    let y = (c * d - b * e) / denominator;
    let z = (a * e - b * d) / denominator;
    Vec3::new(1.0 - y - z, y, z)
}
