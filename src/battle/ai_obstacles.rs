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
    /// Build a local multi-level floor grid and find a connected route over
    /// ramps. Each cell retains multiple heights, so bridges are not flattened.
    pub(crate) fn height_waypoint(
        &self,
        start: Vec3,
        goal: Vec3,
        mut transform: impl FnMut(Entity) -> Option<Transform>,
    ) -> Option<Vec2> {
        use std::cmp::Reverse;
        use std::collections::BinaryHeap;
        const CELL: f32 = 0.75;
        const LAYERS: usize = 4;
        let a = Vec2::new(start.x, start.z);
        let b = Vec2::new(goal.x, goal.z);
        if !start.is_finite() || !goal.is_finite() {
            return None;
        }
        let origin = ((a.min(b) - Vec2::splat(6.0)) / CELL).floor() * CELL;
        let extent = a.max(b) + Vec2::splat(6.0) - origin;
        let width = (extent.x / CELL).ceil() as usize + 1;
        let height = (extent.y / CELL).ceil() as usize + 1;
        if width > 96 || height > 96 {
            return None;
        }
        let mut floors = vec![Vec::<f32>::new(); width * height];
        let mut walls = Vec::<(Vec2, Vec2, f32, f32)>::new();
        for (entity, triangles) in &self.meshes {
            let Some(pose) = transform(*entity) else {
                continue;
            };
            for triangle in triangles {
                let p = triangle.map(|v| pose.transform_point(v));
                let normal = (p[1] - p[0]).cross(p[2] - p[0]);
                if !normal.is_finite() || normal.length_squared() < 1e-8 {
                    continue;
                }
                if normal.y.abs() / normal.length() < 0.65 {
                    let min = p
                        .iter()
                        .copied()
                        .fold(Vec3::splat(f32::INFINITY), Vec3::min);
                    let max = p
                        .iter()
                        .copied()
                        .fold(Vec3::splat(f32::NEG_INFINITY), Vec3::max);
                    if max.x >= origin.x
                        && min.x <= origin.x + extent.x
                        && max.z >= origin.y
                        && min.z <= origin.y + extent.y
                    {
                        walls.push((
                            Vec2::new(min.x, min.z) - Vec2::splat(1.1),
                            Vec2::new(max.x, max.z) + Vec2::splat(1.1),
                            min.y,
                            max.y,
                        ));
                    }
                    continue;
                }
                let p0 = Vec2::new(p[0].x, p[0].z);
                let p1 = Vec2::new(p[1].x, p[1].z);
                let p2 = Vec2::new(p[2].x, p[2].z);
                let cross = |v: Vec2, w: Vec2| v.x * w.y - v.y * w.x;
                let det = cross(p1 - p0, p2 - p0);
                if det.abs() < 1e-6 {
                    continue;
                }
                let low = ((p0.min(p1).min(p2) - origin) / CELL)
                    .floor()
                    .max(Vec2::ZERO);
                let high = ((p0.max(p1).max(p2) - origin) / CELL)
                    .ceil()
                    .min(Vec2::new((width - 1) as f32, (height - 1) as f32));
                if low.x > high.x || low.y > high.y {
                    continue;
                }
                for z in low.y as usize..=high.y as usize {
                    for x in low.x as usize..=high.x as usize {
                        let point = origin + Vec2::new(x as f32, z as f32) * CELL;
                        let u = cross(point - p0, p2 - p0) / det;
                        let v = cross(p1 - p0, point - p0) / det;
                        if u < -0.001 || v < -0.001 || u + v > 1.001 {
                            continue;
                        }
                        let y = p[0].y + u * (p[1].y - p[0].y) + v * (p[2].y - p[0].y);
                        let cell = &mut floors[z * width + x];
                        if y.is_finite()
                            && !cell.iter().any(|h| (h - y).abs() < 0.15)
                            && cell.len() < LAYERS
                        {
                            cell.push(y);
                        }
                    }
                }
            }
        }
        let locate = |point: Vec3| -> Option<usize> {
            let grid = (Vec2::new(point.x, point.z) - origin) / CELL;
            let cx = grid.x.round() as isize;
            let cz = grid.y.round() as isize;
            (-2..=2)
                .flat_map(|dz| (-2..=2).map(move |dx| (cx + dx, cz + dz)))
                .filter(|&(x, z)| x >= 0 && z >= 0 && (x as usize) < width && (z as usize) < height)
                .flat_map(|(x, z)| {
                    floors[z as usize * width + x as usize]
                        .iter()
                        .enumerate()
                        .map(move |(layer, y)| {
                            let dist = Vec2::new(x as f32 - grid.x, z as f32 - grid.y).length()
                                * CELL
                                + (y - point.y).abs() * 2.0;
                            ((z as usize * width + x as usize) * LAYERS + layer, dist)
                        })
                })
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .filter(|(_, cost)| *cost < 2.5)
                .map(|(id, _)| id)
        };
        let source = locate(start)?;
        let destination = locate(goal)?;
        let count = floors.len() * LAYERS;
        let mut cost = vec![u32::MAX; count];
        let mut previous = vec![usize::MAX; count];
        let mut queue = BinaryHeap::new();
        cost[source] = 0;
        queue.push((Reverse(0_u32), source));
        while let Some((Reverse(current), node)) = queue.pop() {
            if node == destination {
                break;
            }
            if current != cost[node] {
                continue;
            }
            let cell = node / LAYERS;
            let x = cell % width;
            let z = cell / width;
            let y = floors[cell][node % LAYERS];
            for (dx, dz) in [(1_isize, 0_isize), (-1, 0), (0, 1), (0, -1)] {
                let nx = x as isize + dx;
                let nz = z as isize + dz;
                if nx < 0 || nz < 0 || nx as usize >= width || nz as usize >= height {
                    continue;
                }
                let next_cell = nz as usize * width + nx as usize;
                for (layer, next_y) in floors[next_cell].iter().enumerate() {
                    let rise = (next_y - y).abs();
                    if rise > 0.48 {
                        continue;
                    }
                    let from = origin + Vec2::new(x as f32, z as f32) * CELL;
                    let to = origin + Vec2::new(nx as f32, nz as f32) * CELL;
                    if walls.iter().any(|&(min, max, low, high)| {
                        high > y.min(*next_y) + 0.15
                            && low < y.max(*next_y) + 0.8
                            && intersects(from, to, min, max)
                    }) {
                        continue;
                    }
                    let next = next_cell * LAYERS + layer;
                    let next_cost = current + 100 + (rise * 60.0) as u32;
                    if next_cost < cost[next] {
                        cost[next] = next_cost;
                        previous[next] = node;
                        queue.push((Reverse(next_cost), next));
                    }
                }
            }
        }
        if cost[destination] == u32::MAX {
            return None;
        }
        let mut path = vec![destination];
        while *path.last()? != source {
            path.push(*previous.get(*path.last()?)?);
            if *path.last()? == usize::MAX {
                return None;
            }
        }
        path.reverse();
        let node = *path.get(3.min(path.len() - 1))? / LAYERS;
        Some(origin + Vec2::new((node % width) as f32, (node / width) as f32) * CELL)
    }
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

pub(crate) fn line_clear(start: Vec2, end: Vec2, obstacles: &[Rect]) -> bool {
    !obstacles.iter().any(|r| {
        let min = Vec2::from_array(r.min);
        let max = Vec2::from_array(r.max);
        min.is_finite() && max.is_finite() && intersects(start, end, min, max)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_between_heights_only_when_ramp_connects_them() {
        let entity = Entity::PLACEHOLDER;
        let mut geometry = Geometry::default();
        let quad = |x0: f32, x1: f32, y0: f32, y1: f32| {
            [
                Vec3::new(x0, y0, -3.),
                Vec3::new(x1, y1, -3.),
                Vec3::new(x1, y1, 3.),
                Vec3::new(x0, y0, 3.),
            ]
        };
        geometry.add_mesh(
            entity,
            &quad(-5., -2., 0., 0.),
            [[0, 1, 2], [0, 2, 3]].into_iter(),
        );
        geometry.add_mesh(
            entity,
            &quad(2., 5., 1.5, 1.5),
            [[0, 1, 2], [0, 2, 3]].into_iter(),
        );
        let start = Vec3::new(-4., 0.1, 0.);
        let goal = Vec3::new(4., 1.6, 0.);
        assert!(
            geometry
                .height_waypoint(start, goal, |_| Some(Transform::IDENTITY))
                .is_none()
        );
        geometry.add_mesh(
            entity,
            &quad(-2., 2., 0., 1.5),
            [[0, 1, 2], [0, 2, 3]].into_iter(),
        );
        let next = geometry
            .height_waypoint(start, goal, |_| Some(Transform::IDENTITY))
            .expect("ramp connects the two floors");
        assert!(next.x > start.x && next.x < goal.x, "{next:?}");
    }

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
