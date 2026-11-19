//! Collision geometry for AI navigation. Positions and heights are world-space metres.
//!
//! Call `Geometry::nearby` for each robot, supplying the current world transform of
//! each arena body. Feed the returned rectangles to tactical steering and, when
//! chasing a destination, use `waypoint` to route around longer walls. Never cache
//! the result across physics ticks: articulated and loose bodies can move.

use super::ai_tactics::Rect;
use bevy::prelude::*;

#[derive(Default)]
pub(crate) struct Geometry {
    meshes: Vec<(Entity, Vec<[Vec3; 3]>)>,
}

impl Geometry {
    pub(super) fn add_mesh(
        &mut self,
        entity: Entity,
        vertices: &[Vec3],
        faces: impl Iterator<Item = [usize; 3]>,
    ) {
        let triangles = faces
            .filter_map(|[a, b, c]| Some([*vertices.get(a)?, *vertices.get(b)?, *vertices.get(c)?]))
            .collect::<Vec<_>>();
        if !triangles.is_empty() {
            self.meshes.push((entity, triangles));
        }
    }

    /// Extract wall footprints within `range` of `position`, at this robot's
    /// height. `position.y` is the floor height; `height` is the robot's height.
    /// Missing/despawned bodies are ignored. Horizontal floors and ceilings are
    /// discarded; a shell is never represented by one arena-sized rectangle.
    pub(crate) fn nearby(
        &self,
        position: Vec3,
        height: f32,
        range: f32,
        mut transform: impl FnMut(Entity) -> Option<Transform>,
    ) -> Vec<Rect> {
        if !position.is_finite()
            || !height.is_finite()
            || height <= 0.1
            || !range.is_finite()
            || range <= 0.0
        {
            return Vec::new();
        }
        let low = position.y + 0.05;
        let high = position.y + height;
        let mut result = Vec::new();
        for (entity, triangles) in &self.meshes {
            let Some(pose) = transform(*entity) else {
                continue;
            };
            for triangle in triangles {
                let p = triangle.map(|v| pose.transform_point(v));
                if p.iter().any(|v| !v.is_finite()) {
                    continue;
                }
                let normal = (p[1] - p[0]).cross(p[2] - p[0]);
                let length = normal.length();
                // Exclude floors, ceilings and shallow ramps, including a floor
                // incorporated in the same triangle mesh as internal walls.
                if length < 1e-6 || normal.y.abs() / length > 0.7 {
                    continue;
                }
                let mut polygon = p.to_vec();
                polygon = clip_height(&polygon, low, true);
                polygon = clip_height(&polygon, high, false);
                if polygon.len() < 3 {
                    continue;
                }
                let min = polygon
                    .iter()
                    .copied()
                    .fold(Vec3::splat(f32::INFINITY), Vec3::min);
                let max = polygon
                    .iter()
                    .copied()
                    .fold(Vec3::splat(f32::NEG_INFINITY), Vec3::max);
                if max.x < position.x - range
                    || min.x > position.x + range
                    || max.z < position.z - range
                    || min.z > position.z + range
                {
                    continue;
                }
                result.push(Rect {
                    min: [min.x, min.z],
                    max: [max.x, max.z],
                });
            }
        }
        result
    }
}

fn clip_height(poly: &[Vec3], y: f32, above: bool) -> Vec<Vec3> {
    if poly.is_empty() {
        return Vec::new();
    }
    let inside = |p: Vec3| if above { p.y >= y } else { p.y <= y };
    let mut out = Vec::new();
    let mut a = *poly.last().unwrap();
    for &b in poly {
        if inside(a) != inside(b) {
            out.push(a.lerp(b, (y - a.y) / (b.y - a.y)));
        }
        if inside(b) {
            out.push(b);
        }
        a = b;
    }
    out
}

/// First waypoint on a shortest visibility-graph path around expanded rectangles.
/// `None` means no path was found. Include all obstacles in the relevant area;
/// call with a bounded local area and replan as the robot moves. This only routes
/// around supplied obstacles; arena bounds, hazards and other robots remain the
/// caller's responsibility. Starting inside an obstacle is left to local steering.
pub(crate) fn waypoint(start: Vec2, goal: Vec2, obstacles: &[Rect], radius: f32) -> Option<Vec2> {
    if !start.is_finite() || !goal.is_finite() || !radius.is_finite() || radius < 0.0 {
        return None;
    }
    let rects: Vec<_> = obstacles
        .iter()
        .filter_map(|r| {
            let min = Vec2::from_array(r.min) - Vec2::splat(radius);
            let max = Vec2::from_array(r.max) + Vec2::splat(radius);
            (min.is_finite() && max.is_finite() && min.cmple(max).all()).then_some((min, max))
        })
        .collect();
    let contains = |p: Vec2, (min, max): (Vec2, Vec2)| p.cmpge(min).all() && p.cmple(max).all();
    if rects
        .iter()
        .any(|&r| contains(start, r) || contains(goal, r))
    {
        return None;
    }
    if !rects
        .iter()
        .any(|&(min, max)| intersects(start, goal, min, max))
    {
        return Some(goal);
    }
    // This intentionally bounded planner is not a navmesh. Avoid cubic work
    // on highly tessellated scenes; callers can fall back to local steering.
    if rects.len() > 128 {
        return None;
    }
    let mut nodes = vec![start, goal];
    // Clearance avoids placing graph vertices directly on an obstacle boundary.
    for &(min, max) in &rects {
        let min = min - Vec2::splat(0.02);
        let max = max + Vec2::splat(0.02);
        for p in [
            Vec2::new(min.x, min.y),
            Vec2::new(min.x, max.y),
            Vec2::new(max.x, min.y),
            Vec2::new(max.x, max.y),
        ] {
            if !rects.iter().any(|&r| contains(p, r)) {
                nodes.push(p);
            }
        }
    }
    let mut distances = vec![f32::INFINITY; nodes.len()];
    let mut previous = vec![usize::MAX; nodes.len()];
    let mut visited = vec![false; nodes.len()];
    distances[0] = 0.0;
    loop {
        let Some(i) = (0..nodes.len())
            .filter(|&i| !visited[i] && distances[i].is_finite())
            .min_by(|&a, &b| distances[a].total_cmp(&distances[b]))
        else {
            return None;
        };
        if i == 1 {
            break;
        }
        visited[i] = true;
        for j in 1..nodes.len() {
            if visited[j]
                || rects
                    .iter()
                    .any(|&(min, max)| intersects(nodes[i], nodes[j], min, max))
            {
                continue;
            }
            let distance = distances[i] + nodes[i].distance(nodes[j]);
            if distance < distances[j] {
                distances[j] = distance;
                previous[j] = i;
            }
        }
    }
    let mut i = 1;
    while previous[i] != 0 {
        i = previous[i];
        if i == usize::MAX {
            return None;
        }
    }
    Some(nodes[i])
}

fn intersects(a: Vec2, b: Vec2, min: Vec2, max: Vec2) -> bool {
    let d = b - a;
    let (mut lo, mut hi) = (0.0_f32, 1.0_f32);
    for axis in 0..2 {
        if d[axis].abs() < 1e-6 {
            if a[axis] < min[axis] || a[axis] > max[axis] {
                return false;
            }
        } else {
            let x = (min[axis] - a[axis]) / d[axis];
            let y = (max[axis] - a[axis]) / d[axis];
            lo = lo.max(x.min(y));
            hi = hi.min(x.max(y));
            if lo > hi {
                return false;
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floor_is_ignored_wall_is_clipped_and_moving_body_is_followed() {
        let e = Entity::PLACEHOLDER;
        let mut geometry = Geometry::default();
        geometry.add_mesh(
            e,
            &[
                Vec3::new(-10., 0., -10.),
                Vec3::new(10., 0., -10.),
                Vec3::new(0., 0., 10.),
                Vec3::new(0., 0., -2.),
                Vec3::new(0., 2., -2.),
                Vec3::new(0., 2., 2.),
            ],
            [[0, 1, 2], [3, 4, 5]].into_iter(),
        );
        let scan = |x| {
            geometry.nearby(Vec3::ZERO, 1., 20., |_| {
                Some(Transform::from_xyz(x, 0., 0.))
            })
        };
        let first = scan(0.);
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].min[0], 0.);
        assert_eq!(scan(3.)[0].min[0], 3.);
        assert!(geometry.nearby(Vec3::ZERO, 1., 20., |_| None).is_empty());
        assert!(
            geometry
                .nearby(Vec3::new(0., 3., 0.), 1., 20., |_| Some(
                    Transform::default()
                ))
                .is_empty()
        );
    }

    #[test]
    fn routes_around_wall_and_returns_direct_goal_when_clear() {
        let wall = Rect {
            min: [-0.1, -2.],
            max: [0.1, 2.],
        };
        let start = Vec2::new(-3., 0.);
        let goal = Vec2::new(3., 0.);
        let next = waypoint(start, goal, &[wall], 0.5).unwrap();
        assert!(next.y.abs() > 2.5);
        assert!(!intersects(
            start,
            next,
            Vec2::new(-0.6, -2.5),
            Vec2::new(0.6, 2.5)
        ));
        assert_eq!(waypoint(start, goal, &[], 0.5), Some(goal));
        assert_eq!(waypoint(Vec2::ZERO, goal, &[wall], 0.5), None);
    }
}
