//! Event-driven dents and swept-surface damage. No solver, springs or FEM.
use crate::{Mat4, Model, Vec2, Vec3};
use std::collections::{HashMap, VecDeque};

#[derive(Clone, Debug)]
pub struct Triangle {
    pub nodes: [usize; 3],
    pub uv: [Vec2; 3],
    pub normal: Vec3,
}

#[derive(Clone, Debug)]
pub struct DentMesh {
    pub rest: Vec<Vec3>,
    pub positions: Vec<Vec3>,
    pub triangles: Vec<Triangle>,
    neighbours: Vec<Vec<usize>>,
    normals: Vec<Vec3>,
    pub bounds: (Vec3, Vec3),
}

#[derive(Clone, Copy, Debug)]
pub struct SurfaceHit {
    pub triangle: usize,
    pub point: Vec3,
    pub rest_point: Vec3,
    pub normal: Vec3,
    pub distance: f32,
}

#[derive(Clone, Debug)]
pub struct DamageBox {
    pub component: usize,
    pub name: String,
    /// Component-local box, positioned relative to the chassis frame.
    pub transform: Mat4,
    pub min: Vec3,
    pub max: Vec3,
    pub damage: f32,
    pub intrusion: f32,
}

#[derive(Default, Debug)]
pub struct DentResult {
    pub changed_nodes: usize,
    pub affected_components: Vec<usize>,
    pub max_intrusion: f32,
}

impl DentMesh {
    pub fn from_model(model: &Model, spacing: f32) -> Result<Self, String> {
        Self::from_model_with_density(model, spacing, 1.0)
    }

    /// Linear subdivision multiplier; twice the detail produces four times the
    /// triangles. The reference spacing/count stays identical at density 1.
    pub fn from_model_with_density(
        model: &Model,
        spacing: f32,
        density: f32,
    ) -> Result<Self, String> {
        if !density.is_finite() || density <= 0.0 {
            return Err("Mesh density must be finite and positive".into());
        }
        let faces: Vec<_> = model
            .objects
            .iter()
            .filter(|o| o.visible)
            .flat_map(|o| o.faces.iter().map(move |f| (o, f)))
            .collect();
        if faces.is_empty() {
            return Err("Chassis has no visible triangles".into());
        }
        let longest = faces
            .iter()
            .flat_map(|(o, f)| {
                let p = f
                    .vertices
                    .map(|i| o.transform.transform_point3(o.vertices[i]));
                [
                    p[0].distance(p[1]),
                    p[1].distance(p[2]),
                    p[2].distance(p[0]),
                ]
            })
            .fold(0.0_f32, f32::max);
        // One subdivision count for every face: shared edges have identical nodes.
        let reference = (longest / spacing.max(0.1)).ceil().clamp(2.0, 8.0);
        let subdivisions = (reference * density).round().clamp(1.0, 64.0) as usize;
        let mut rest = Vec::new();
        let mut welded = HashMap::new();
        let mut triangles = Vec::new();
        for (object, face) in faces {
            let p = face
                .vertices
                .map(|i| object.transform.transform_point3(object.vertices[i]));
            let uv = face.uv.map(Vec2::from_array);
            let normal = (p[1] - p[0]).cross(p[2] - p[0]).normalize_or_zero();
            if normal == Vec3::ZERO {
                continue;
            }
            let mut grid = HashMap::new();
            for i in 0..=subdivisions {
                for j in 0..=subdivisions - i {
                    let a = i as f32 / subdivisions as f32;
                    let b = j as f32 / subdivisions as f32;
                    let position = p[0] * (1.0 - a - b) + p[1] * a + p[2] * b;
                    let key = position.to_array().map(|v| (v * 100_000.0).round() as i32);
                    let node = *welded.entry(key).or_insert_with(|| {
                        let id = rest.len();
                        rest.push(position);
                        id
                    });
                    grid.insert(
                        (i, j),
                        (node, uv[0] * (1.0 - a - b) + uv[1] * a + uv[2] * b),
                    );
                }
            }
            for i in 0..subdivisions {
                for j in 0..subdivisions - i {
                    let mut add = |ids: [(usize, usize); 3]| {
                        let vertices = ids.map(|id| grid[&id]);
                        triangles.push(Triangle {
                            nodes: vertices.map(|v| v.0),
                            uv: vertices.map(|v| v.1),
                            normal,
                        });
                    };
                    add([(i, j), (i + 1, j), (i, j + 1)]);
                    if i + j + 1 < subdivisions {
                        add([(i + 1, j), (i + 1, j + 1), (i, j + 1)]);
                    }
                }
            }
        }
        if rest.is_empty() {
            return Err("Chassis has zero-area geometry".into());
        }
        let mut neighbours = vec![Vec::new(); rest.len()];
        let mut normals = vec![Vec3::ZERO; rest.len()];
        for triangle in &triangles {
            for i in 0..3 {
                let node = triangle.nodes[i];
                normals[node] += triangle.normal;
                for other in triangle.nodes {
                    if node != other && !neighbours[node].contains(&other) {
                        neighbours[node].push(other);
                    }
                }
            }
        }
        for n in &mut normals {
            *n = n.normalize_or_zero();
        }
        let min = rest
            .iter()
            .copied()
            .fold(Vec3::splat(f32::INFINITY), Vec3::min);
        let max = rest
            .iter()
            .copied()
            .fold(Vec3::splat(f32::NEG_INFINITY), Vec3::max);
        Ok(Self {
            positions: rest.clone(),
            rest,
            triangles,
            neighbours,
            normals,
            bounds: (min, max),
        })
    }

    pub fn raycast(&self, origin: Vec3, direction: Vec3) -> Option<SurfaceHit> {
        self.triangles
            .iter()
            .enumerate()
            .filter_map(|(index, triangle)| {
                let p = triangle.nodes.map(|i| self.positions[i]);
                let (distance, bary) = ray_triangle(origin, direction, p)?;
                let rest = triangle.nodes.map(|i| self.rest[i]);
                let normal = (p[1] - p[0]).cross(p[2] - p[0]).normalize_or_zero();
                Some(SurfaceHit {
                    triangle: index,
                    point: origin + direction * distance,
                    rest_point: rest[0] * bary.x + rest[1] * bary.y + rest[2] * bary.z,
                    normal: if normal.dot(direction) > 0.0 {
                        -normal
                    } else {
                        normal
                    },
                    distance,
                })
            })
            .min_by(|a, b| a.distance.total_cmp(&b.distance))
    }

    /// Only this operation changes geometry or component damage. Resting contacts
    /// and subsequent render/physics frames never accumulate damage on their own.
    pub fn dent(
        &mut self,
        hit: SurfaceHit,
        radius: f32,
        depth: f32,
        max_depth: f32,
        budget: f32,
        components: &mut [DamageBox],
    ) -> DentResult {
        if hit.triangle >= self.triangles.len() || radius <= 0.0 || depth <= 0.0 {
            return DentResult::default();
        }
        let outward = self.triangles[hit.triangle].normal;
        let limit = max_depth
            .max(0.0)
            .min((self.bounds.1 - self.bounds.0).dot(outward.abs()) * 0.72);
        let mut next = self.positions.clone();
        let mut seen = vec![false; self.rest.len()];
        let mut queue: VecDeque<_> = self.triangles[hit.triangle].nodes.into_iter().collect();
        while let Some(i) = queue.pop_front() {
            if seen[i] {
                continue;
            }
            seen[i] = true;
            let r2 = self.rest[i].distance_squared(hit.rest_point) / (radius * radius);
            if r2 >= 1.0 || self.normals[i].dot(outward) < -0.15 {
                continue;
            }
            let weight = (1.0 - r2) * (1.0 - r2);
            let old_depth = (self.rest[i] - self.positions[i]).dot(outward).max(0.0);
            let amount = (depth * weight).min((limit - old_depth).max(0.0));
            next[i] -= outward * amount;
            next[i] = self.rest[i] + (next[i] - self.rest[i]).clamp_length_max(max_depth.max(0.0));
            queue.extend(self.neighbours[i].iter().copied());
        }
        // Back off the whole edit if it would invert or collapse a triangle.
        let valid = |positions: &[Vec3]| {
            self.triangles.iter().all(|t| {
                let p = t.nodes.map(|i| positions[i]);
                let r = t.nodes.map(|i| self.rest[i]);
                (p[1] - p[0]).cross(p[2] - p[0]).dot(t.normal)
                    > (r[1] - r[0]).cross(r[2] - r[0]).length() * 0.04
            })
        };
        for _ in 0..8 {
            if valid(&next) {
                break;
            }
            for (p, old) in next.iter_mut().zip(&self.positions) {
                *p = (*p + *old) * 0.5;
            }
        }
        if !valid(&next) {
            return DentResult::default();
        }
        let changed_nodes = next
            .iter()
            .zip(&self.positions)
            .filter(|(a, b)| a.distance_squared(**b) > 1e-12)
            .count();
        if changed_nodes == 0 {
            return DentResult::default();
        }
        let mut intrusion = vec![0.0_f32; components.len()];
        for triangle in &self.triangles {
            let old = triangle.nodes.map(|i| self.positions[i]);
            let new = triangle.nodes.map(|i| next[i]);
            if old
                .iter()
                .zip(new)
                .all(|(a, b)| a.distance_squared(b) < 1e-12)
            {
                continue;
            }
            for (i, component) in components.iter().enumerate() {
                intrusion[i] = intrusion[i].max(swept_intrusion(old, new, component));
            }
        }
        // Max per component, not sum over triangles: damage is independent of tessellation.
        let scores: Vec<_> = intrusion
            .iter()
            .map(|d| (d - 0.001).max(0.0) * 500.0)
            .collect();
        let scale = (budget.max(0.0) / scores.iter().sum::<f32>().max(0.001)).min(1.0);
        let mut result = DentResult {
            changed_nodes,
            ..Default::default()
        };
        for (i, component) in components.iter_mut().enumerate() {
            if scores[i] > 0.0 {
                component.damage = (component.damage + scores[i] * scale).min(100.0);
                component.intrusion += intrusion[i];
                result.max_intrusion = result.max_intrusion.max(intrusion[i]);
                result.affected_components.push(component.component);
            }
        }
        self.positions = next;
        result
    }
}

/// Two-sided Moller-Trumbore ray test, also used for picking native components.
pub fn ray_triangle(origin: Vec3, direction: Vec3, p: [Vec3; 3]) -> Option<(f32, Vec3)> {
    let e1 = p[1] - p[0];
    let e2 = p[2] - p[0];
    let h = direction.cross(e2);
    let det = e1.dot(h);
    if det.abs() < 1e-8 {
        return None;
    }
    let s = origin - p[0];
    let u = s.dot(h) / det;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = s.cross(e1);
    let v = direction.dot(q) / det;
    if v < 0.0 || u + v > 1.00001 {
        return None;
    }
    let t = e2.dot(q) / det;
    (t > 0.0001).then_some((t, Vec3::new(1.0 - u - v, u, v)))
}

fn segment_inside(a: Vec3, b: Vec3, min: Vec3, max: Vec3) -> f32 {
    let delta = b - a;
    let mut enter = 0.0_f32;
    let mut leave = 1.0_f32;
    for axis in 0..3 {
        if delta[axis].abs() < 1e-8 {
            if a[axis] < min[axis] || a[axis] > max[axis] {
                return 0.0;
            }
        } else {
            let t1 = (min[axis] - a[axis]) / delta[axis];
            let t2 = (max[axis] - a[axis]) / delta[axis];
            enter = enter.max(t1.min(t2));
            leave = leave.min(t1.max(t2));
        }
    }
    (leave - enter).max(0.0) * delta.length()
}

fn closest_bary(p: Vec3, t: [Vec3; 3]) -> Vec3 {
    // Closest point on a triangle (including edges), used to probe small boxes
    // between mesh nodes. These tests cannot miss solely due to sparse vertices.
    let ab = t[1] - t[0];
    let ac = t[2] - t[0];
    let ap = p - t[0];
    let d1 = ab.dot(ap);
    let d2 = ac.dot(ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        return Vec3::X;
    }
    let bp = p - t[1];
    let d3 = ab.dot(bp);
    let d4 = ac.dot(bp);
    if d3 >= 0.0 && d4 <= d3 {
        return Vec3::Y;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let v = d1 / (d1 - d3);
        return Vec3::new(1.0 - v, v, 0.0);
    }
    let cp = p - t[2];
    let d5 = ab.dot(cp);
    let d6 = ac.dot(cp);
    if d6 >= 0.0 && d5 <= d6 {
        return Vec3::Z;
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        let w = d2 / (d2 - d6);
        return Vec3::new(1.0 - w, 0.0, w);
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && d4 - d3 >= 0.0 && d5 - d6 >= 0.0 {
        let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        return Vec3::new(0.0, 1.0 - w, w);
    }
    let denom = 1.0 / (va + vb + vc);
    let v = vb * denom;
    let w = vc * denom;
    Vec3::new(1.0 - v - w, v, w)
}

fn swept_intrusion(old: [Vec3; 3], new: [Vec3; 3], component: &DamageBox) -> f32 {
    let inverse = component.transform.inverse();
    let old = old.map(|p| inverse.transform_point3(p));
    let new = new.map(|p| inverse.transform_point3(p));
    let min = old
        .into_iter()
        .chain(new)
        .fold(Vec3::splat(f32::INFINITY), Vec3::min);
    let max = old
        .into_iter()
        .chain(new)
        .fold(Vec3::splat(f32::NEG_INFINITY), Vec3::max);
    if min.cmpgt(component.max).any() || max.cmplt(component.min).any() {
        return 0.0;
    }
    let probe = |bary: Vec3| {
        let a = old[0] * bary.x + old[1] * bary.y + old[2] * bary.z;
        let b = new[0] * bary.x + new[1] * bary.y + new[2] * bary.z;
        segment_inside(a, b, component.min, component.max)
    };
    let mut amount = probe(Vec3::splat(1.0 / 3.0));
    for corner in 0..8 {
        let p = Vec3::new(
            if corner & 1 == 0 {
                component.min.x
            } else {
                component.max.x
            },
            if corner & 2 == 0 {
                component.min.y
            } else {
                component.max.y
            },
            if corner & 4 == 0 {
                component.min.z
            } else {
                component.max.z
            },
        );
        amount = amount.max(probe(closest_bary(p, old)));
    }
    amount.max(probe(closest_bary(
        (component.min + component.max) * 0.5,
        old,
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn roof() -> DentMesh {
        let mut model = Model::default();
        model.objects.push(crate::Object {
            name: "roof".into(),
            visible: true,
            parent: None,
            transform: Mat4::IDENTITY,
            vertices: vec![
                Vec3::new(-1.0, 0.4, -1.0),
                Vec3::new(-1.0, 0.4, 1.0),
                Vec3::new(1.0, 0.4, 1.0),
                Vec3::new(1.0, 0.4, -1.0),
                Vec3::new(0.0, 0.0, 0.0),
            ],
            faces: vec![
                crate::Face {
                    vertices: [0, 1, 2],
                    material: 0,
                    uv: [[0.0; 2]; 3],
                    normals: [Vec3::Y; 3],
                },
                crate::Face {
                    vertices: [0, 2, 3],
                    material: 0,
                    uv: [[0.0; 2]; 3],
                    normals: [Vec3::Y; 3],
                },
            ],
            material: 0,
            uv_channels: Default::default(),
        });
        let mut mesh = DentMesh::from_model(&model, 0.35).unwrap();
        mesh.bounds.0.y = 0.0;
        mesh
    }
    fn battery() -> DamageBox {
        DamageBox {
            component: 1,
            name: "Battery".into(),
            transform: Mat4::IDENTITY,
            min: Vec3::new(-0.1, 0.1, -0.1),
            max: Vec3::new(0.1, 0.30, 0.1),
            damage: 0.0,
            intrusion: 0.0,
        }
    }
    #[test]
    fn dent_preserves_seams_limits_depth_and_only_damages_when_reaching_component() {
        let mut mesh = roof();
        let rest = mesh.positions.clone();
        let mut boxes = vec![battery()];
        let hit = mesh.raycast(Vec3::Y * 2.0, -Vec3::Y).unwrap();
        let r = mesh.dent(hit, 0.7, 0.05, 0.25, 20.0, &mut boxes);
        assert!(r.changed_nodes > 0);
        assert_eq!(boxes[0].damage, 0.0);
        for _ in 0..6 {
            let hit = mesh.raycast(Vec3::Y * 2.0, -Vec3::Y).unwrap();
            mesh.dent(hit, 0.7, 0.05, 0.25, 20.0, &mut boxes);
        }
        assert!(boxes[0].damage > 0.0);
        assert!(boxes[0].intrusion > 0.0);
        assert!(
            mesh.positions
                .iter()
                .zip(rest)
                .all(|(p, r)| r.y - p.y <= 0.25001)
        );
        let damage = boxes[0].damage;
        let hit = mesh.raycast(Vec3::Y * 2.0, -Vec3::Y).unwrap();
        mesh.dent(hit, 0.7, 0.0, 0.25, 20.0, &mut boxes);
        assert_eq!(boxes[0].damage, damage);
        for t in &mesh.triangles {
            let p = t.nodes.map(|i| mesh.positions[i]);
            assert!((p[1] - p[0]).cross(p[2] - p[0]).dot(t.normal) > 0.0);
        }
    }
    #[test]
    fn swept_region_detects_small_rotated_box_between_triangle_vertices() {
        let component = DamageBox {
            transform: Mat4::from_rotation_y(0.5),
            min: Vec3::new(-0.02, 0.15, -0.02),
            max: Vec3::new(0.02, 0.25, 0.02),
            ..battery()
        };
        let old = [
            Vec3::new(-1.0, 0.4, -1.0),
            Vec3::new(0.0, 0.4, 1.0),
            Vec3::new(1.0, 0.4, -1.0),
        ];
        let new = old.map(|p| p - Vec3::Y * 0.3);
        assert!(swept_intrusion(old, new, &component) > 0.09);
        let far = DamageBox {
            transform: Mat4::from_translation(Vec3::X * 3.0),
            ..component
        };
        assert_eq!(swept_intrusion(old, new, &far), 0.0);
    }

    #[test]
    fn nearby_disconnected_surface_is_not_pulled_into_the_dent() {
        let mut mesh = roof();
        let count = mesh.rest.len();
        let other: Vec<_> = mesh.rest.iter().map(|p| *p - Vec3::Y * 0.04).collect();
        let triangles: Vec<_> = mesh
            .triangles
            .iter()
            .map(|t| Triangle {
                nodes: t.nodes.map(|i| i + count),
                ..t.clone()
            })
            .collect();
        let neighbours: Vec<_> = mesh
            .neighbours
            .iter()
            .map(|n| n.iter().map(|i| i + count).collect())
            .collect();
        mesh.rest.extend(&other);
        mesh.positions.extend(&other);
        mesh.triangles.extend(triangles);
        mesh.neighbours.extend(neighbours);
        mesh.normals.extend(vec![Vec3::Y; count]);
        let hit = mesh.raycast(Vec3::Y * 2.0, -Vec3::Y).unwrap();
        mesh.dent(hit, 0.7, 0.03, 0.25, 20.0, &mut []);
        assert_eq!(&mesh.positions[count..], &other);
        assert!(
            mesh.positions[..count]
                .iter()
                .zip(&mesh.rest[..count])
                .any(|(a, b)| a != b)
        );
    }

    #[test]
    fn overlapping_components_share_the_hit_damage_budget() {
        let mut mesh = roof();
        let mut boxes = vec![battery(), battery()];
        boxes[1].component = 2;
        let hit = mesh.raycast(Vec3::Y * 2.0, -Vec3::Y).unwrap();
        mesh.dent(hit, 0.7, 0.22, 0.25, 7.0, &mut boxes);
        assert!(boxes.iter().all(|b| b.damage > 0.0));
        assert!(boxes.iter().map(|b| b.damage).sum::<f32>() <= 7.0001);
    }
}
