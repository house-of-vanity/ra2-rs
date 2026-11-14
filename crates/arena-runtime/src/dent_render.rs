//! Shared event-driven dent mesh upload; panel smoothing preserves authored edges.
use bevy::prelude::*;
#[derive(Default)]
pub struct SmoothingGroups {
    corners: Vec<[usize; 3]>,
    count: usize,
}
impl SmoothingGroups {
    pub fn new(shell: &arena_core::damage::DentMesh) -> Self {
        let mut nodes: Vec<Vec<(Vec3, usize)>> = vec![Vec::new(); shell.rest.len()];
        let mut count = 0;
        let corners = shell
            .triangles
            .iter()
            .map(|t| {
                t.nodes.map(|node| {
                    if let Some(&(_, group)) =
                        nodes[node].iter().find(|(n, _)| n.dot(t.normal) > 0.99999)
                    {
                        group
                    } else {
                        let group = count;
                        count += 1;
                        nodes[node].push((t.normal, group));
                        group
                    }
                })
            })
            .collect();
        Self { corners, count }
    }
}

pub fn update(
    mesh: &mut Mesh,
    shell: &arena_core::damage::DentMesh,
    smooth: bool,
    groups: &SmoothingGroups,
) {
    let mut positions = Vec::with_capacity(shell.triangles.len() * 3);
    let mut normals = Vec::with_capacity(positions.capacity());
    let mut uv = Vec::with_capacity(positions.capacity());
    let mut averaged = if smooth {
        vec![Vec3::ZERO; groups.count]
    } else {
        Vec::new()
    };
    if smooth {
        for (triangle, corners) in shell.triangles.iter().zip(&groups.corners) {
            let p = triangle.nodes.map(|i| shell.positions[i]);
            // Area weighting uses the deformed surface, not the original normals.
            let area = (p[1] - p[0]).cross(p[2] - p[0]);
            for &group in corners {
                averaged[group] += area;
            }
        }
        for normal in &mut averaged {
            *normal = normal.normalize_or_zero();
        }
    }
    for (i, triangle) in shell.triangles.iter().enumerate() {
        let p = triangle.nodes.map(|i| shell.positions[i]);
        let n = (p[1] - p[0]).cross(p[2] - p[0]).normalize_or_zero();
        positions.extend(p.map(|p| p.to_array()));
        if smooth {
            normals.extend(groups.corners[i].map(|group| {
                let normal = averaged[group];
                if normal == Vec3::ZERO {
                    n.to_array()
                } else {
                    normal.to_array()
                }
            }));
        } else {
            normals.extend([n.to_array(); 3]);
        }
        uv.extend(triangle.uv.map(|p| p.to_array()));
    }
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uv);
}
