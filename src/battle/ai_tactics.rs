//! Standalone, deterministic tactical steering in the arena's XZ plane.
//!
//! Feed positions and bounds in world units, health as a fraction in 0..=1,
//! and `side` from `Mode::side(slot)`. Rectangles are conservative XZ AABBs;
//! use arena `State::bounds` for `bounds` and individual runtime trap `zones`
//! and `buttons` for hazards. `Trap::min/max` only describe its first zone.
//! Include trigger zones even when a trap is inactive if entering activates it;
//! mark active or otherwise dangerous regions with `dangerous = true`.
//! The caller supplies collision geometry and applies the returned direction to
//! its drive controls. This is local steering, not global pathfinding.

use std::f32::consts::PI;

/// Horizontal world coordinates: `[x, z]`.
pub type Point = [f32; 2];

#[derive(Clone, Copy, Debug)]
pub struct Robot {
    pub id: u32,
    pub side: usize,
    pub position: Point,
    /// Horizontal forward direction; need not be normalized.
    pub forward: Point,
    pub health: f32,
    pub alive: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct Rect {
    pub min: Point,
    pub max: Point,
}

#[derive(Clone, Copy, Debug)]
pub struct Hazard {
    pub area: Rect,
    pub dangerous: bool,
}

pub struct World<'a> {
    pub me: Robot,
    pub robots: &'a [Robot],
    /// Playable XZ bounds, if known. The Y/ringout threshold is not a 2D edge.
    pub bounds: Option<Rect>,
    /// King of the Hill scoring zone in world XZ coordinates, if applicable.
    pub objective: Option<Rect>,
    pub obstacles: &'a [Rect],
    pub hazards: &'a [Hazard],
    /// Approximate robot radius, in world units.
    pub radius: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tactic {
    Idle,
    Attack,
    Flank,
    Flee,
    Avoid,
    Recover,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Decision {
    pub target: Option<u32>,
    pub tactic: Tactic,
    /// Desired world-space travel direction, normalized or zero.
    pub direction: Point,
}

/// One instance per robot. Replaying the same inputs, time steps and seed
/// produces the same decisions. `dt` is in seconds.
pub struct Tactics {
    rng: u64,
    previous: Option<Point>,
    stuck_time: f32,
    recovery_time: f32,
    recovery_side: f32,
    last_direction: Point,
    health_sample: Option<(u32, f32, f32)>,
    own_damage: f32,
    enemy_damage: f32,
}

impl Tactics {
    pub fn new(seed: u64) -> Self {
        Self {
            rng: seed,
            previous: None,
            stuck_time: 0.0,
            recovery_time: 0.0,
            recovery_side: 1.0,
            last_direction: [0.0, 0.0],
            health_sample: None,
            own_damage: 0.0,
            enemy_damage: 0.0,
        }
    }

    fn random(&mut self) -> f32 {
        // SplitMix64 also has a valid, non-degenerate zero seed.
        self.rng = self.rng.wrapping_add(0x9e3779b97f4a7c15);
        let mut z = self.rng;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
        z ^= z >> 31;
        (z >> 40) as f32 / (1u32 << 24) as f32
    }

    pub fn decide(&mut self, world: &World<'_>, dt: f32) -> Decision {
        let me = world.me;
        let dt = if dt.is_finite() {
            dt.clamp(0.0, 0.25)
        } else {
            0.0
        };
        if !me.alive || !finite(me.position) {
            self.previous = None;
            self.stuck_time = 0.0;
            self.recovery_time = 0.0;
            self.last_direction = [0.0, 0.0];
            self.health_sample = None;
            self.own_damage = 0.0;
            self.enemy_damage = 0.0;
            return Decision {
                target: None,
                tactic: Tactic::Idle,
                direction: [0.0, 0.0],
            };
        }
        // Measure progress across a time window, not per frame: tiny movements at
        // high frame rates should not be mistaken for a stationary robot.
        if let Some(previous) = self.previous {
            if length(sub(me.position, previous)) >= 0.2 {
                self.previous = Some(me.position);
                self.stuck_time = 0.0;
            } else if length(self.last_direction) > 0.5 && self.recovery_time <= 0.0 {
                self.stuck_time += dt;
            } else if length(self.last_direction) <= 0.5 {
                self.stuck_time = 0.0;
                self.previous = Some(me.position);
            }
        } else {
            self.previous = Some(me.position);
        }
        if self.stuck_time >= 1.5 {
            self.recovery_time = 0.8;
            self.recovery_side = if self.random() < 0.5 { -1.0 } else { 1.0 };
            self.stuck_time = 0.0;
            self.previous = Some(me.position);
        }
        self.recovery_time = (self.recovery_time - dt).max(0.0);

        let objective = world
            .objective
            .filter(|r| finite(r.min) && finite(r.max) && (0..2).all(|i| r.min[i] < r.max[i]));
        let on_hill = objective.is_some_and(|r| contains(r, me.position));
        let target = world
            .robots
            .iter()
            .filter(|r| {
                r.id != me.id
                    && r.side != me.side
                    && r.alive
                    && finite(r.position)
                    && (!on_hill || length(sub(r.position, me.position)) <= 4.0)
            })
            .min_by(|a, b| {
                // Prefer nearby and damaged enemies; tie-break by ID, independent of input order.
                let score =
                    |r: &Robot| length(sub(r.position, me.position)) + 2.0 * health(r.health);
                score(a).total_cmp(&score(b)).then_with(|| a.id.cmp(&b.id))
            });
        let target_id = target.map(|r| r.id);
        // Exponentially decaying damage totals approximate a recent 3-second
        // window. Never carry an old opponent's trend over to a new target.
        if let Some(enemy) = target {
            let current = (enemy.id, health(me.health), health(enemy.health));
            if let Some((id, own, other)) = self.health_sample.filter(|s| s.0 == enemy.id) {
                if dt > 0.0 {
                    let decay = (-dt / 3.0).exp();
                    self.own_damage = self.own_damage * decay + (own - current.1).max(0.0);
                    self.enemy_damage = self.enemy_damage * decay + (other - current.2).max(0.0);
                    self.health_sample = Some((id, current.1, current.2));
                }
            } else {
                self.own_damage = 0.0;
                self.enemy_damage = 0.0;
                self.health_sample = Some(current);
            }
        } else {
            self.health_sample = None;
            self.own_damage = 0.0;
            self.enemy_damage = 0.0;
        }
        let danger_here = world
            .hazards
            .iter()
            .any(|h| h.dangerous && contains(expand(h.area, world.radius.max(0.0)), me.position));
        let (mut tactic, desired) = if self.recovery_time > 0.0 {
            let forward = unit(me.forward);
            let forward = if length(forward) > 0.0 {
                forward
            } else {
                self.last_direction
            };
            (
                Tactic::Recover,
                unit(add(
                    scale(forward, -1.0),
                    scale(perp(forward), self.recovery_side * 0.7),
                )),
            )
        } else if let Some(enemy) = target {
            let to_enemy = sub(enemy.position, me.position);
            let distance = length(to_enemy);
            let toward = unit(to_enemy);
            let losing = self.own_damage > 0.08 && self.own_damage > self.enemy_damage * 1.5 + 0.03;
            if (health(me.health) < 0.25 || (health(me.health) < 0.65 && losing)) && distance < 12.0
            {
                (Tactic::Flee, scale(toward, -1.0))
            } else {
                let radius = if world.radius.is_finite() {
                    world.radius.max(0.0)
                } else {
                    0.0
                };
                let offset = (radius * 2.0 + 0.8).max(1.2);
                // A nearby dangerous region is useful only if the enemy can be
                // pushed towards it from a safe staging point on the far side.
                let push = world
                    .hazards
                    .iter()
                    .filter(|h| h.dangerous)
                    .filter_map(|h| {
                        let r = h.area;
                        if !finite(r.min) || !finite(r.max) || (0..2).any(|i| r.min[i] > r.max[i]) {
                            return None;
                        }
                        let nearest = [
                            enemy.position[0].clamp(r.min[0], r.max[0]),
                            enemy.position[1].clamp(r.min[1], r.max[1]),
                        ];
                        let delta = sub(nearest, enemy.position);
                        let gap = length(delta);
                        if !(0.5..=6.0).contains(&gap) {
                            return None;
                        }
                        let axis = unit(delta);
                        let staging = sub(enemy.position, scale(axis, offset));
                        safe_point(world, staging, radius).then_some((gap, axis, staging))
                    })
                    .min_by(|a, b| a.0.total_cmp(&b.0));
                let candidate = if let Some((_, axis, staging)) = push {
                    // Once aligned, drive straight into the enemy rather than
                    // orbiting the staging point.
                    if dot(unit(sub(me.position, enemy.position)), axis) < -0.75 {
                        None
                    } else {
                        Some(staging)
                    }
                } else if distance < 5.0 && distance > offset && health(me.health) > 0.4 && !losing
                {
                    let forward = unit(enemy.forward);
                    let behind = sub(enemy.position, scale(forward, offset));
                    let already_behind =
                        dot(unit(sub(me.position, enemy.position)), forward) < -0.5;
                    if length(forward) > 0.0 && !already_behind && safe_point(world, behind, radius)
                    {
                        Some(behind)
                    } else {
                        None
                    }
                } else {
                    None
                };
                if let Some(aim) = candidate {
                    let direction = unit(sub(aim, me.position));
                    // Do not sacrifice closing distance to circle endlessly.
                    if dot(direction, toward) >= 0.5 {
                        (Tactic::Flank, direction)
                    } else {
                        (Tactic::Attack, toward)
                    }
                } else {
                    (Tactic::Attack, toward)
                }
            }
        } else {
            (Tactic::Idle, [0.0, 0.0])
        };

        let mut desired = desired;
        if let Some(hill) = objective {
            if !on_hill && self.recovery_time <= 0.0 {
                let center = scale(add(hill.min, hill.max), 0.5);
                desired = unit(sub(center, me.position));
                tactic = Tactic::Attack;
            } else if on_hill && target.is_none() && self.recovery_time <= 0.0 {
                // Stay put rather than chasing opponents beyond the hill.
                desired = [0.0, 0.0];
                tactic = Tactic::Idle;
            }
        }
        if danger_here {
            // Escape even with no opponent; combine outward vectors from all zones.
            let mut escape = [0.0, 0.0];
            for h in world.hazards.iter().filter(|h| {
                h.dangerous && contains(expand(h.area, world.radius.max(0.0)), me.position)
            }) {
                escape = add(
                    escape,
                    outward(expand(h.area, world.radius.max(0.0)), me.position),
                );
            }
            if length(escape) > 0.0 {
                desired = unit(escape);
            }
            tactic = Tactic::Avoid;
        }
        let direction = steer(
            world,
            desired,
            if on_hill && !danger_here {
                objective
            } else {
                None
            },
        );
        if length(desired) > 0.0
            && dot(direction, desired) < 0.98
            && tactic != Tactic::Recover
            && tactic != Tactic::Flee
        {
            tactic = Tactic::Avoid;
        }
        self.last_direction = direction;
        Decision {
            target: target_id,
            tactic,
            direction,
        }
    }
}

fn health(h: f32) -> f32 {
    if h.is_finite() {
        h.clamp(0.0, 1.0)
    } else {
        0.0
    }
}
fn finite(p: Point) -> bool {
    p[0].is_finite() && p[1].is_finite()
}
fn add(a: Point, b: Point) -> Point {
    [a[0] + b[0], a[1] + b[1]]
}
fn sub(a: Point, b: Point) -> Point {
    [a[0] - b[0], a[1] - b[1]]
}
fn scale(a: Point, s: f32) -> Point {
    [a[0] * s, a[1] * s]
}
fn dot(a: Point, b: Point) -> f32 {
    a[0] * b[0] + a[1] * b[1]
}
fn length(a: Point) -> f32 {
    dot(a, a).sqrt()
}
fn unit(a: Point) -> Point {
    let n = length(a);
    if n.is_finite() && n > 1e-6 {
        scale(a, 1.0 / n)
    } else {
        [0.0, 0.0]
    }
}
fn perp(a: Point) -> Point {
    [-a[1], a[0]]
}
fn expand(r: Rect, n: f32) -> Rect {
    Rect {
        min: [r.min[0] - n, r.min[1] - n],
        max: [r.max[0] + n, r.max[1] + n],
    }
}
fn contains(r: Rect, p: Point) -> bool {
    (0..2).all(|i| p[i] >= r.min[i] && p[i] <= r.max[i])
}
fn safe_point(world: &World<'_>, p: Point, radius: f32) -> bool {
    if let Some(bounds) = world.bounds {
        let inner = expand(bounds, -radius);
        if !finite(inner.min) || !finite(inner.max) || !contains(inner, p) {
            return false;
        }
    }
    !world
        .obstacles
        .iter()
        .any(|r| contains(expand(*r, radius), p))
        && !world
            .hazards
            .iter()
            .any(|h| h.dangerous && contains(expand(h.area, radius), p))
}
fn outward(r: Rect, p: Point) -> Point {
    let choices = [
        (p[0] - r.min[0], [-1.0, 0.0]),
        (r.max[0] - p[0], [1.0, 0.0]),
        (p[1] - r.min[1], [0.0, -1.0]),
        (r.max[1] - p[1], [0.0, 1.0]),
    ];
    choices
        .into_iter()
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .unwrap()
        .1
}

/// Segment against AABB. A robot already inside may move toward the nearest exit.
fn blocked(r: Rect, start: Point, end: Point) -> bool {
    if contains(r, start) {
        return dot(sub(end, start), outward(r, start)) <= 0.0;
    }
    let delta = sub(end, start);
    let (mut lo, mut hi) = (0.0_f32, 1.0_f32);
    for i in 0..2 {
        if delta[i].abs() < 1e-6 {
            if start[i] < r.min[i] || start[i] > r.max[i] {
                return false;
            }
        } else {
            let a = (r.min[i] - start[i]) / delta[i];
            let b = (r.max[i] - start[i]) / delta[i];
            lo = lo.max(a.min(b));
            hi = hi.min(a.max(b));
            if lo > hi {
                return false;
            }
        }
    }
    true
}

fn steer(world: &World<'_>, desired: Point, hold: Option<Rect>) -> Point {
    if length(desired) < 1e-6 {
        return [0.0, 0.0];
    }
    let radius = if world.radius.is_finite() {
        world.radius.max(0.0)
    } else {
        0.0
    };
    let start = world.me.position;
    let base = desired[1].atan2(desired[0]);
    // Test progressively wider deviations, retaining a fixed left/right tie break.
    for step in 0..=12 {
        for sign in if step == 0 {
            &[1.0][..]
        } else {
            &[1.0, -1.0][..]
        } {
            let angle = base + *sign * step as f32 * PI / 12.0;
            let direction = [angle.cos(), angle.sin()];
            // Only constrain a short step: the obstacle lookahead can be wider
            // than the hill itself. Hazard escape deliberately bypasses this.
            if hold.is_some_and(|hill| !contains(hill, add(start, scale(direction, 0.1)))) {
                continue;
            }
            let end = add(start, scale(direction, (radius * 2.0 + 1.0).max(2.0)));
            if let Some(b) = world.bounds {
                let inner = expand(b, -radius);
                if !finite(inner.min)
                    || !finite(inner.max)
                    || (0..2).any(|i| inner.min[i] > inner.max[i])
                {
                    return [0.0, 0.0];
                }
                // If already outside, allow movement back toward the playable area.
                if contains(inner, start) {
                    if !contains(inner, end) {
                        continue;
                    }
                } else {
                    let closest = [
                        start[0].clamp(inner.min[0], inner.max[0]),
                        start[1].clamp(inner.min[1], inner.max[1]),
                    ];
                    if dot(direction, sub(closest, start)) <= 0.0 {
                        continue;
                    }
                }
            }
            if world
                .obstacles
                .iter()
                .any(|r| blocked(expand(*r, radius), start, end))
            {
                continue;
            }
            if world
                .hazards
                .iter()
                .any(|h| h.dangerous && blocked(expand(h.area, radius), start, end))
            {
                continue;
            }
            return direction;
        }
    }
    [0.0, 0.0]
}

#[cfg(test)]
mod tests {
    use super::*;
    fn robot(id: u32, side: usize, x: f32, z: f32) -> Robot {
        Robot {
            id,
            side,
            position: [x, z],
            forward: [1.0, 0.0],
            health: 1.0,
            alive: true,
        }
    }
    fn world<'a>(
        me: Robot,
        robots: &'a [Robot],
        obstacles: &'a [Rect],
        hazards: &'a [Hazard],
    ) -> World<'a> {
        World {
            me,
            robots,
            bounds: None,
            objective: None,
            obstacles,
            hazards,
            radius: 0.25,
        }
    }
    #[test]
    fn selects_enemy_excludes_allies_and_dead_and_breaks_ties_by_id() {
        let me = robot(0, 0, 0.0, 0.0);
        let mut dead = robot(1, 1, 1.0, 0.0);
        dead.alive = false;
        let robots = [
            robot(4, 0, 0.1, 0.0),
            dead,
            robot(3, 1, 8.0, 0.0),
            robot(2, 2, -8.0, 0.0),
        ];
        let d = Tactics::new(1).decide(&world(me, &robots, &[], &[]), 0.1);
        assert_eq!(d.target, Some(2));
        assert_eq!(d.tactic, Tactic::Attack);
    }
    #[test]
    fn fleeing_and_flanking() {
        let mut me = robot(0, 0, 0.0, 0.0);
        let mut enemies = [robot(1, 1, 3.0, 0.0)];
        enemies[0].forward = [-1.0, 0.0];
        me.health = 0.1;
        let d = Tactics::new(0).decide(&world(me, &enemies, &[], &[]), 0.1);
        assert_eq!(d.tactic, Tactic::Flee);
        assert!(d.direction[0] < 0.0);
        me.health = 1.0;
        assert_eq!(
            Tactics::new(0)
                .decide(&world(me, &enemies, &[], &[]), 0.1)
                .tactic,
            Tactic::Flank
        );
    }
    #[test]
    fn avoids_obstacles_and_active_hazards_and_escapes_zone() {
        let me = robot(0, 0, 0.0, 0.0);
        let enemies = [robot(1, 1, 10.0, 0.0)];
        let rect = Rect {
            min: [1.0, -0.4],
            max: [2.0, 0.4],
        };
        let d = Tactics::new(0).decide(&world(me, &enemies, &[rect], &[]), 0.1);
        assert_eq!(d.tactic, Tactic::Avoid);
        assert!(d.direction[1].abs() > 0.1);
        let hazard = [Hazard {
            area: rect,
            dangerous: true,
        }];
        let d = Tactics::new(0).decide(&world(me, &enemies, &[], &hazard), 0.1);
        assert_eq!(d.tactic, Tactic::Avoid);
        let inside = robot(0, 0, 1.5, 0.0);
        let d = Tactics::new(0).decide(&world(inside, &[], &[], &hazard), 0.1);
        assert_eq!(d.tactic, Tactic::Avoid);
        assert!(length(d.direction) > 0.9);
    }
    #[test]
    fn respects_bounds_and_no_target_is_idle() {
        let me = robot(0, 0, 0.0, 0.0);
        let enemies = [robot(1, 1, 10.0, 0.0)];
        let mut w = world(me, &enemies, &[], &[]);
        w.bounds = Some(Rect {
            min: [-5.0, -5.0],
            max: [1.0, 5.0],
        });
        let d = Tactics::new(0).decide(&w, 0.1);
        assert!(d.direction[0] <= 0.375 + 0.001);
        assert_eq!(
            Tactics::new(0)
                .decide(&world(me, &[], &[], &[]), 0.1)
                .tactic,
            Tactic::Idle
        );
    }
    #[test]
    fn stuck_recovers_deterministically_and_motion_prevents_recovery() {
        let me = robot(0, 0, 0.0, 0.0);
        let enemies = [robot(1, 1, 10.0, 0.0)];
        let w = world(me, &enemies, &[], &[]);
        let (mut a, mut b) = (Tactics::new(7), Tactics::new(7));
        for _ in 0..16 {
            assert_eq!(a.decide(&w, 0.1), b.decide(&w, 0.1));
        }
        assert_eq!(a.decide(&w, 0.1).tactic, Tactic::Recover);
        let mut moving = Tactics::new(7);
        for i in 0..30 {
            let mut m = me;
            m.position[0] = i as f32 * 0.03;
            assert_ne!(
                moving.decide(&world(m, &enemies, &[], &[]), 0.1).tactic,
                Tactic::Recover
            );
        }
    }
    #[test]
    fn rear_approach_closes_and_stops_flanking_when_behind() {
        let enemy = robot(1, 1, 3.0, 0.0);
        let enemies = [enemy];
        let me = robot(0, 0, 3.0, 3.0);
        let d = Tactics::new(0).decide(&world(me, &enemies, &[], &[]), 0.1);
        assert_eq!(d.tactic, Tactic::Flank);
        assert!(dot(d.direction, unit(sub(enemy.position, me.position))) >= 0.5);
        assert!(d.direction[1] < 0.0);
        let behind = robot(0, 0, 1.0, 0.0);
        let d = Tactics::new(0).decide(&world(behind, &enemies, &[], &[]), 0.1);
        assert_eq!(d.tactic, Tactic::Attack);
        assert!(d.direction[0] > 0.99);
    }
    #[test]
    fn relative_damage_changes_tactic_and_expires() {
        let mut me = robot(0, 0, 0.0, 0.0);
        me.health = 0.6;
        let mut enemies = [robot(1, 1, 7.0, 0.0)];
        let mut ai = Tactics::new(4);
        assert_eq!(
            ai.decide(&world(me, &enemies, &[], &[]), 0.1).tactic,
            Tactic::Attack
        );
        me.health = 0.45;
        assert_eq!(
            ai.decide(&world(me, &enemies, &[], &[]), 0.1).tactic,
            Tactic::Flee
        );
        // Causing comparable damage turns the tide; no need to wait for expiry.
        enemies[0].health = 0.7;
        assert_eq!(
            ai.decide(&world(me, &enemies, &[], &[]), 0.1).tactic,
            Tactic::Attack
        );
        let mut ai = Tactics::new(4);
        ai.decide(&world(me, &enemies, &[], &[]), 0.1);
        me.health = 0.30;
        assert_eq!(
            ai.decide(&world(me, &enemies, &[], &[]), 0.1).tactic,
            Tactic::Flee
        );
        // Moving avoids the independent stuck recovery timer.
        for _ in 0..100 {
            me.position[1] += 0.21;
            ai.decide(&world(me, &enemies, &[], &[]), 0.25);
        }
        me.position = [0.0, 0.0];
        assert_eq!(
            ai.decide(&world(me, &enemies, &[], &[]), 0.1).tactic,
            Tactic::Attack
        );
    }
    #[test]
    fn new_target_does_not_inherit_damage_history() {
        let mut me = robot(0, 0, 0.0, 0.0);
        me.health = 0.6;
        let mut enemies = [robot(1, 1, 7.0, 0.0)];
        let mut ai = Tactics::new(0);
        ai.decide(&world(me, &enemies, &[], &[]), 0.1);
        me.health = 0.45;
        assert_eq!(
            ai.decide(&world(me, &enemies, &[], &[]), 0.1).tactic,
            Tactic::Flee
        );
        enemies[0].id = 2;
        assert_eq!(
            ai.decide(&world(me, &enemies, &[], &[]), 0.1).tactic,
            Tactic::Attack
        );
    }
    #[test]
    fn stages_on_safe_side_to_push_into_hazard_then_attacks() {
        let enemy = robot(1, 1, 4.0, 0.0);
        let enemies = [enemy];
        let hazards = [Hazard {
            area: Rect {
                min: [6.0, -1.0],
                max: [8.0, 1.0],
            },
            dangerous: true,
        }];
        let me = robot(0, 0, 4.0, 3.0);
        let d = Tactics::new(0).decide(&world(me, &enemies, &[], &hazards), 0.1);
        assert_eq!(d.tactic, Tactic::Flank);
        assert!(d.direction[1] < 0.0);
        let aligned = robot(0, 0, 1.0, 0.0);
        let d = Tactics::new(0).decide(&world(aligned, &enemies, &[], &hazards), 0.1);
        assert_eq!(d.tactic, Tactic::Attack);
        assert!(d.direction[0] > 0.99);
        let inactive = [Hazard {
            dangerous: false,
            ..hazards[0]
        }];
        // Facing away means ordinary rear tactics do not apply from behind.
        let d = Tactics::new(0).decide(&world(aligned, &enemies, &[], &inactive), 0.1);
        assert_eq!(d.tactic, Tactic::Attack);
    }
    #[test]
    fn hill_capture_overrides_chase_and_works_without_enemies() {
        let me = robot(0, 0, 0.0, 0.0);
        let enemies = [robot(1, 1, -3.0, 0.0)];
        let mut w = world(me, &enemies, &[], &[]);
        w.objective = Some(Rect {
            min: [5.0, -1.0],
            max: [7.0, 1.0],
        });
        let d = Tactics::new(0).decide(&w, 0.1);
        assert!(d.direction[0] > 0.99);
        w.robots = &[];
        let d = Tactics::new(0).decide(&w, 0.1);
        assert!(d.direction[0] > 0.99);
    }
    #[test]
    fn hill_holds_against_distant_enemies_and_fights_nearby_without_leaving() {
        let me = robot(0, 0, 0.95, 0.0);
        let enemies = [robot(1, 1, 10.0, 0.0)];
        let hill = Rect {
            min: [-1.0, -1.0],
            max: [1.0, 1.0],
        };
        let mut w = world(me, &enemies, &[], &[]);
        w.objective = Some(hill);
        let d = Tactics::new(0).decide(&w, 0.1);
        assert_eq!(d.tactic, Tactic::Idle);
        assert_eq!(d.target, None);
        assert_eq!(d.direction, [0.0, 0.0]);
        let nearby = [robot(1, 1, 3.0, 0.0)];
        w.robots = &nearby;
        let d = Tactics::new(0).decide(&w, 0.1);
        assert_eq!(d.target, Some(1));
        assert!(contains(hill, add(me.position, scale(d.direction, 0.1))));
    }
    #[test]
    fn hill_does_not_prevent_hazard_escape() {
        let me = robot(0, 0, 0.95, 0.0);
        let hazards = [Hazard {
            area: Rect {
                min: [0.0, -3.0],
                max: [1.0, 3.0],
            },
            dangerous: true,
        }];
        let mut w = world(me, &[], &[], &hazards);
        w.objective = Some(Rect {
            min: [-1.0, -1.0],
            max: [1.0, 1.0],
        });
        let d = Tactics::new(0).decide(&w, 0.1);
        assert_eq!(d.tactic, Tactic::Avoid);
        assert!(d.direction[0] > 0.9);
    }
}
