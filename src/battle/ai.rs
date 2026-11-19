//! Computer players use the same controller and actuator path as human players.
use super::{Phase, Session, ai_controls, ai_tactics};
use crate::{
    frontend::{Navigation, Page},
    gameplay::RobotBody,
};
use bevy::prelude::*;
use std::collections::BTreeMap;

pub(super) struct Brain {
    body: Entity,
    tactics: ai_tactics::Tactics,
    controls: ai_controls::AiControls,
    combat_sign: f32,
    obstacles: Vec<ai_tactics::Rect>,
    scan_timer: f32,
    waypoint: Option<Vec2>,
}

pub(crate) fn update(
    mut session: ResMut<Session>,
    nav: Res<Navigation>,
    catalog: Res<crate::editor::catalog::Catalog>,
    poses: Query<&Transform, With<RobotBody>>,
    arena_poses: Query<&Transform, Without<RobotBody>>,
    mut brains: Local<BTreeMap<u32, Brain>>,
    mut facing_cache: Local<BTreeMap<Entity, f32>>,
) {
    if session.network || nav.page != Page::Match || session.phase != Phase::Playing {
        if session.robots.is_empty() || session.phase == Phase::Setup {
            brains.clear();
            facing_cache.clear();
        }
        return;
    }
    let mode = session.mode;
    let robots: Vec<_> = session
        .robots
        .iter()
        .filter_map(|robot| {
            let body = *robot.bodies.first()?;
            let pose = poses.get(body).ok()?;
            let sign = *facing_cache.entry(body).or_insert_with(|| {
                robot
                    .document
                    .as_ref()
                    .map_or(1.0, |document| combat_sign(document, &catalog))
            });
            let forward = pose.rotation * Vec3::NEG_Z * sign;
            let health = robot
                .damage
                .health
                .parts
                .first()
                .map_or(0.0, |part| part.fraction());
            Some(ai_tactics::Robot {
                id: robot.id,
                side: mode.side(robot.id as usize - 1),
                position: [pose.translation.x, pose.translation.z],
                forward: [forward.x, forward.z],
                health,
                alive: robot.running,
            })
        })
        .collect();
    let bounds = session
        .arena_state
        .bounds
        .map(|(min, max)| ai_tactics::Rect {
            min: [min.x, min.z],
            max: [max.x, max.z],
        });
    let objective = if mode == arena_core::match_rules::Mode::KingOfHill {
        session
            .arena_state
            .king
            .as_ref()
            .map(|zone| ai_tactics::Rect {
                min: [zone.min.x, zone.min.z],
                max: [zone.max.x, zone.max.z],
            })
    } else {
        None
    };
    let hazards: Vec<_> = session
        .hazards_runtime
        .iter()
        .flat_map(|trap| {
            trap.zones.iter().map(|zone| ai_tactics::Hazard {
                area: ai_tactics::Rect {
                    min: [zone.min.x, zone.min.z],
                    max: [zone.max.x, zone.max.z],
                },
                dangerous: true,
            })
        })
        .collect();
    let session = &mut *session;
    let geometry = &session.arena_state.ai_obstacles;
    for robot in &mut session.robots {
        if robot.id == 1 || !robot.running {
            continue;
        }
        let Some(me) = robots.iter().find(|other| other.id == robot.id).copied() else {
            continue;
        };
        let (Some(document), Some(rig)) = (&robot.document, &robot.rig) else {
            continue;
        };
        let body = robot.bodies[0];
        if brains
            .get(&robot.id)
            .is_some_and(|brain| brain.body != body)
        {
            brains.remove(&robot.id);
        }
        let brain = brains.entry(robot.id).or_insert_with(|| Brain {
            body,
            tactics: ai_tactics::Tactics::new(robot.id as u64),
            controls: ai_controls::AiControls::new(ai_controls::WiringAnalysis::new(document, rig)),
            combat_sign: combat_sign(document, &catalog),
            obstacles: Vec::new(),
            scan_timer: 0.0,
            waypoint: None,
        });
        brain.scan_timer -= crate::STEP as f32;
        let rescan = brain.scan_timer <= 0.0;
        if rescan {
            brain.scan_timer = 0.25;
            if let Ok(pose) = poses.get(body) {
                brain.obstacles = geometry.nearby(
                    pose.translation,
                    document.chassis.height.max(0.3),
                    8.0,
                    |entity| arena_poses.get(entity).ok().copied(),
                );
            }
        }
        let decision = brain.tactics.decide(
            &ai_tactics::World {
                me,
                robots: &robots,
                bounds,
                objective,
                obstacles: &brain.obstacles,
                hazards: &hazards,
                radius: 0.6,
            },
            crate::STEP as f32,
        );
        let forward = Vec2::from_array(me.forward).normalize_or_zero();
        let mut direction = Vec2::from_array(decision.direction).normalize_or_zero();
        let location = Vec2::from_array(me.position);
        if rescan {
            brain.waypoint = if matches!(
                decision.tactic,
                ai_tactics::Tactic::Attack | ai_tactics::Tactic::Flank
            ) && !brain.obstacles.is_empty()
                && direction != Vec2::ZERO
            {
                let destination = location + direction * 5.0;
                super::ai_obstacles::waypoint(location, destination, &brain.obstacles, 0.6)
                    .filter(|p| p.distance(destination) > 0.1)
            } else {
                None
            };
        }
        if !matches!(
            decision.tactic,
            ai_tactics::Tactic::Attack | ai_tactics::Tactic::Flank
        ) {
            brain.waypoint = None;
        }
        if let Some(waypoint) = brain.waypoint {
            let to_waypoint = waypoint - location;
            if to_waypoint.length() > 0.2 {
                direction = to_waypoint.normalize_or_zero();
            } else {
                brain.waypoint = None;
            }
        }
        let dot = forward.dot(direction).clamp(-1.0, 1.0);
        let mut turn = (forward.y * direction.x - forward.x * direction.y).clamp(-1.0, 1.0);
        let reverse_allowed = matches!(
            decision.tactic,
            ai_tactics::Tactic::Recover | ai_tactics::Tactic::Flee
        );
        if !reverse_allowed && dot < -0.8 && turn.abs() < 0.25 {
            turn = if robot.id % 2 == 0 { 1.0 } else { -1.0 };
        }
        let intent = ai_controls::Intent {
            forward: if direction == Vec2::ZERO {
                0.0
            } else if dot < -0.5 && reverse_allowed {
                -0.5
            } else if dot < -0.5 {
                0.0
            } else {
                dot.max(0.25)
            } * brain.combat_sign,
            turn,
            fire: decision.target.is_some(),
        };
        let samples = brain.controls.samples(intent);
        let mut commands = robot.controls.evaluate(document, &samples);
        for (command, old) in commands.iter_mut().zip(&robot.commands) {
            command.fire |= old.fire;
        }
        robot.commands = commands;
    }
}

/// Native bindings explicitly specify the combat-facing side for stock robots.
/// For custom designs, infer front/back only from a clearly offset weapon; a
/// central or sideways weapon leaves the editor's forward direction unchanged.
fn combat_sign(
    robot: &arena_core::robot::RobotDefinition,
    catalog: &crate::editor::catalog::Catalog,
) -> f32 {
    if let Ok(bindings) = std::fs::read_to_string(catalog.root.join("AI/Bindings.py"))
        && let Some(sign) = native_combat_sign(&bindings, &robot.name)
    {
        return sign;
    }
    let Ok(frozen) = catalog.freeze(robot) else {
        return 1.0;
    };
    let mut sum = 0.0;
    let mut count = 0;
    for (index, component) in robot.components.iter().enumerate() {
        if !component.class.eq_ignore_ascii_case("weapon") {
            continue;
        }
        if let Some(part) = frozen
            .assembly
            .parts
            .iter()
            .find(|part| part.component == index)
        {
            sum += part.transform.w_axis.z;
            count += 1;
        }
    }
    if count > 0 && sum / count as f32 > 0.12 {
        -1.0
    } else {
        1.0
    }
}

fn native_combat_sign(bindings: &str, robot_name: &str) -> Option<f32> {
    for line in bindings
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("list.append("))
    {
        let Some((_, tail)) = line.split_once('"') else {
            continue;
        };
        let Some((name, rest)) = tail.split_once('"') else {
            continue;
        };
        if !name.eq_ignore_ascii_case(robot_name) {
            continue;
        }
        if let Some((_, nose)) = rest.split_once("'nose'")
            && let Some((_, value)) = nose.split_once(':')
        {
            let value = value
                .split([',', '}'])
                .next()
                .unwrap_or("")
                .trim()
                .replace(' ', "");
            if value == "math.pi" || value == "-math.pi" {
                return Some(-1.0);
            }
            if value == "0" || value == "0.0" {
                return Some(1.0);
            }
            // Side-facing or other angles cannot be represented by reversing
            // throttle; retain ordinary drive orientation instead.
            return Some(1.0);
        }
        return Some(1.0);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::native_combat_sign;
    #[test]
    fn reads_original_combat_facing_without_running_python() {
        let bindings = "# list.append( (\"Dummy\", \"Rammer\", {'nose': math.pi}) )\nlist.append( (\"BackSlash\", \"Flipper\", { 'nose' : math.pi, 'weapons': (16, 17) }) )\nlist.append( (\"Scout\", \"Flipper\", {}) )";
        assert_eq!(native_combat_sign(bindings, "backslash"), Some(-1.0));
        assert_eq!(native_combat_sign(bindings, "Scout"), Some(1.0));
        assert_eq!(native_combat_sign(bindings, "Dummy"), None);
    }
}
