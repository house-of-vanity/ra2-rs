//! Native chassis geometry: 1000x1000 design coordinates, paired contours and panel UVs.
use crate::{robot::*, *};
use std::collections::BTreeMap;
pub const DESIGN_SCALE: f32 = 0.0022;
pub const MAX_POINTS: usize = 16;

fn cross(a: Vec2, b: Vec2, c: Vec2) -> f32 {
    (b - a).perp_dot(c - a)
}
fn area(points: &[Vec2]) -> f32 {
    points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .take(points.len())
        .map(|(a, b)| a.perp_dot(*b))
        .sum::<f32>()
        * 0.5
}
pub fn validate_outline(points: &[Vec2]) -> Result<(), String> {
    if !(3..=MAX_POINTS).contains(&points.len()) {
        return Err("A closed outline needs 3 to 16 points".into());
    }
    for (i, p) in points.iter().enumerate() {
        if !p.is_finite() || p.min_element() < 0.0 || p.max_element() > 1000.0 {
            return Err("Outline must stay inside the blueprint".into());
        }
        let next = points[(i + 1) % points.len()];
        if p.distance(next) < 1.0 {
            return Err("Outline points are too close".into());
        }
        for j in i + 1..points.len() {
            if j == i || (i + 1) % points.len() == j || (j + 1) % points.len() == i {
                continue;
            }
            let (a, b, c, d) = (*p, next, points[j], points[(j + 1) % points.len()]);
            if cross(a, b, c) * cross(a, b, d) < 0.0 && cross(c, d, a) * cross(c, d, b) < 0.0 {
                return Err("Outline edges intersect".into());
            }
        }
    }
    if area(points).abs() < 1.0 {
        return Err("Outline has zero area".into());
    }
    Ok(())
}
fn triangulate(points: &[Vec2]) -> Result<Vec<[usize; 3]>, String> {
    let sign = area(points).signum();
    let mut ids: Vec<_> = (0..points.len()).collect();
    let mut result = vec![];
    while ids.len() > 3 {
        let ear = (0..ids.len())
            .find(|&i| {
                let a = ids[(i + ids.len() - 1) % ids.len()];
                let b = ids[i];
                let c = ids[(i + 1) % ids.len()];
                if cross(points[a], points[b], points[c]) * sign <= 0.0001 {
                    return false;
                }
                !ids.iter()
                    .filter(|&&j| j != a && j != b && j != c)
                    .any(|&j| {
                        cross(points[a], points[b], points[j]) * sign >= 0.0
                            && cross(points[b], points[c], points[j]) * sign >= 0.0
                            && cross(points[c], points[a], points[j]) * sign >= 0.0
                    })
            })
            .ok_or("Cannot triangulate outline")?;
        result.push([
            ids[(ear + ids.len() - 1) % ids.len()],
            ids[ear],
            ids[(ear + 1) % ids.len()],
        ]);
        ids.remove(ear);
    }
    result.push([ids[0], ids[1], ids[2]]);
    Ok(result)
}
pub fn rectangle(length: f32, width: f32) -> Vec<Vec2> {
    let size = Vec2::new(width, length).clamp(Vec2::splat(50.0), Vec2::splat(1000.0)) * 0.5;
    vec![
        Vec2::new(500.0 - size.x, 500.0 - size.y),
        Vec2::new(500.0 + size.x, 500.0 - size.y),
        Vec2::new(500.0 + size.x, 500.0 + size.y),
        Vec2::new(500.0 - size.x, 500.0 + size.y),
    ]
}
pub fn circle(radius: f32) -> Vec<Vec2> {
    (0..16)
        .map(|i| {
            Vec2::splat(500.0)
                + Vec2::from_angle(i as f32 * std::f32::consts::TAU / 16.0)
                    * radius.clamp(25.0, 500.0)
        })
        .map(|p| p.round())
        .collect()
}
pub fn rebuild(chassis: &mut Chassis) -> Result<(), String> {
    validate_outline(&chassis.bottom)?;
    validate_outline(&chassis.top)?;
    let n = chassis.bottom.len();
    if chassis.top.len() != n {
        return Err("Top and baseplate need matching corners".into());
    }
    if !chassis.height.is_finite() || !(0.05..=1.5).contains(&chassis.height) {
        return Err("Invalid chassis height".into());
    }
    let mut vertices = vec![];
    let mut normals = vec![];
    let mut groups: Vec<Vec<usize>> = vec![];
    let point = |p: Vec2, y| {
        Vec3::new(
            (p.x - 500.0) * DESIGN_SCALE,
            y,
            (p.y - 500.0) * DESIGN_SCALE,
        )
    };
    vertices.extend(chassis.bottom.iter().map(|&p| point(p, 0.0)));
    normals.extend(vec![-Vec3::Y; n]);
    groups.push((0..n).collect());
    vertices.extend(chassis.top.iter().map(|&p| point(p, chassis.height)));
    normals.extend(vec![Vec3::Y; n]);
    groups.push((n..2 * n).collect());
    for i in 0..n {
        let next = (i + 1) % n;
        let start = vertices.len();
        let p = [
            point(chassis.bottom[i], 0.0),
            point(chassis.bottom[next], 0.0),
            point(chassis.top[next], chassis.height),
            point(chassis.top[i], chassis.height),
        ];
        let edge = p[1] - p[0];
        let outward = Vec3::new(edge.z, 0.0, -edge.x) * area(&chassis.bottom).signum();
        let mut normal = (p[1] - p[0]).cross(p[2] - p[0]).normalize_or_zero();
        if normal.dot(outward) < 0.0 {
            normal = -normal;
        }
        vertices.extend(p);
        normals.extend([normal; 4]);
        groups.push((start..start + 4).collect());
    }
    let columns = ((groups.len() as f32).sqrt().ceil() as usize).max(1);
    let rows = groups.len().div_ceil(columns);
    let cell = Vec2::new(256.0 / columns as f32, 256.0 / rows as f32);
    let mut uv = vec![[0.0; 2]; vertices.len()];
    let mut layout = ChassisPaintLayout::default();
    for (g, ids) in groups.iter().enumerate() {
        let min = Vec2::new((g % columns) as f32, (g / columns) as f32) * cell + Vec2::splat(4.0);
        let max = min + cell - Vec2::splat(8.0);
        let points: Vec<_> = if g < 2 {
            ids.iter()
                .map(|&id| Vec2::new(vertices[id].x, vertices[id].z))
                .collect()
        } else {
            let origin = vertices[ids[0]];
            let along = (vertices[ids[1]] - origin).normalize_or_zero();
            ids.iter()
                .map(|&id| Vec2::new((vertices[id] - origin).dot(along), -vertices[id].y))
                .collect()
        };
        let lo = points
            .iter()
            .copied()
            .fold(Vec2::splat(f32::INFINITY), Vec2::min);
        let hi = points
            .iter()
            .copied()
            .fold(Vec2::splat(f32::NEG_INFINITY), Vec2::max);
        let span = (hi - lo).max(Vec2::splat(0.0001));
        let pixels: Vec<_> = points
            .iter()
            .map(|p| min + (*p - lo) / span * (max - min))
            .collect();
        for (&id, p) in ids.iter().zip(&pixels) {
            uv[id] = (*p / 256.0).to_array();
        }
        layout.group_uvs.push(pixels);
        layout.rectangles.push([
            g as i32,
            min.x.floor() as i32 - 2,
            min.y.floor() as i32 - 2,
            max.x.ceil() as i32 + 2,
            max.y.ceil() as i32 + 2,
        ]);
    }
    let mut faces = vec![];
    let mut add = |mut ids: [usize; 3], group: usize| {
        let normal = normals[ids[0]];
        if (vertices[ids[1]] - vertices[ids[0]])
            .cross(vertices[ids[2]] - vertices[ids[0]])
            .dot(normal)
            < 0.0
        {
            ids.swap(1, 2);
        }
        faces.push(Face {
            vertices: ids,
            material: 0,
            uv: ids.map(|i| uv[i]),
            normals: ids.map(|i| normals[i]),
        });
        layout
            .face_groups
            .push([group as i32, ids[0] as i32, ids[2] as i32, ids[1] as i32]);
    };
    for ids in triangulate(&chassis.bottom)? {
        add(ids, 0);
    }
    for ids in triangulate(&chassis.top)? {
        add(ids.map(|i| i + n), 1);
    }
    for i in 0..n {
        let s = 2 * n + 4 * i;
        add([s, s + 1, s + 2], i + 2);
        add([s, s + 2, s + 3], i + 2);
    }
    for i in 0..n {
        layout.top_corners.push([
            i as i32,
            3,
            (n + i) as i32,
            (2 * n + 4 * i + 3) as i32,
            (2 * n + 4 * ((i + n - 1) % n) + 2) as i32,
        ]);
    }
    let mut buckle: BTreeMap<[i32; 3], (Vec3, Vec<usize>)> = BTreeMap::new();
    for (i, &p) in vertices.iter().enumerate() {
        let key = p.to_array().map(|v| (v * 1_000_000.0).round() as i32);
        buckle.entry(key).or_insert((p, vec![])).1.push(i);
    }
    chassis.buckling_points = buckle.into_values().collect();
    chassis.face_count = faces.len();
    chassis.paint_layout = layout;
    let texture = Texture {
        file: "@chassis.tga".into(),
        scale: Vec2::ONE,
        offset: Vec2::ZERO,
        rotation: 0.0,
        channel: 1,
    };
    let material = Material {
        name: "Chassis paint".into(),
        color: [0.65; 3],
        alpha: 1.0,
        two_sided: false,
        texture: Some(texture),
        opacity_texture: None,
        submaterials: BTreeMap::new(),
    };
    let model = Model {
        materials: BTreeMap::from([(0, material)]),
        objects: vec![Object {
            name: "chassis".into(),
            visible: true,
            parent: None,
            transform: Mat4::IDENTITY,
            vertices,
            faces,
            material: 0,
            uv_channels: BTreeMap::new(),
        }],
        ..Default::default()
    };
    model.validate()?;
    chassis.model = Some(model);
    if chassis.texture.is_none() {
        chassis.texture = Some(super::paint::solid_tga(256, 256, [185, 185, 185, 0]));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn generated_native_mesh_has_paint_panels_seams_and_matching_top_corners() {
        let mut c = crate::editor::Workshop::new().robot.chassis;
        c.bottom = rectangle(600.0, 700.0);
        c.top = c.bottom.clone();
        rebuild(&mut c).unwrap();
        let mesh = &c.model.as_ref().unwrap().objects[0];
        assert_eq!(mesh.vertices.len(), 24);
        assert_eq!(mesh.faces.len(), 12);
        assert_eq!(c.paint_layout.group_uvs.len(), 6);
        assert_eq!(c.paint_layout.top_corners.len(), 4);
        assert_eq!(c.paint_layout.face_groups.len(), 12);
        for face in &mesh.faces {
            let [a, b, d] = face.vertices.map(|i| mesh.vertices[i]);
            assert!((b - a).cross(d - a).dot(face.normals[0]) > 0.0);
        }
    }
    #[test]
    fn concave_contours_work_and_crossing_contours_fail() {
        let p = vec![
            Vec2::new(100.0, 100.0),
            Vec2::new(900.0, 100.0),
            Vec2::new(500.0, 500.0),
            Vec2::new(900.0, 900.0),
            Vec2::new(100.0, 900.0),
        ];
        assert_eq!(triangulate(&p).unwrap().len(), 3);
        assert!(
            validate_outline(&[
                Vec2::ZERO,
                Vec2::splat(100.0),
                Vec2::new(0.0, 100.0),
                Vec2::new(100.0, 0.0)
            ])
            .is_err()
        );
    }
}
