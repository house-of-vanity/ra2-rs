//! Declarative extraction of the shipped Introduction generators. Never runs Python.
use super::arenas::{arguments, calls, number, quoted, vector};
use arena_core::{Quat, Vec3};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug)]
enum Gate {
    Always,
    Hazards,
    Player(usize),
}
#[derive(Clone, Debug)]
enum Action {
    Wait(String),
    Camera(Vec<String>),
    Sound(Vec<String>),
}
#[derive(Clone, Debug, Default)]
pub(crate) struct Introduction {
    pub music: Option<String>,
    fade_out: f32,
    actions: Vec<(Gate, Action)>,
}
#[derive(Clone, Debug)]
pub(crate) struct Shot {
    pub start: f32,
    pub end: f32,
    pub position: [Vec3; 2],
    pub rotation: [Quat; 2],
    pub fov: [f32; 2],
}
#[derive(Default)]
pub(crate) struct Timeline {
    pub duration: f32,
    pub shots: Vec<Shot>,
    pub cues: Vec<(f32, String)>,
    pub music: Option<String>,
    pub fade_out: f32,
}
impl Introduction {
    pub fn parse(source: &str) -> Result<Self, String> {
        let Some(start) = source.find("    def Introduction(self):") else {
            return Ok(Self::default());
        };
        let source = &source[start..];
        let source = source
            .find("\n    def ")
            .map_or(source, |end| &source[..end]);
        let mut groups = BTreeMap::new();
        let mut intro = Self::default();
        let mut gate = Gate::Always;
        let mut gate_indent = usize::MAX;
        for raw in source.lines() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let indent = raw.len() - raw.trim_start().len();
            if indent <= gate_indent {
                gate = Gate::Always;
            }
            if line == "if self.bHazardsOn:" {
                gate = Gate::Hazards;
                gate_indent = indent;
                continue;
            }
            if let Some(slot) = line
                .strip_prefix("if ")
                .and_then(|s| s.strip_suffix(" in players:"))
            {
                gate = Gate::Player(
                    slot.parse()
                        .map_err(|_| format!("Invalid intro slot {slot}"))?,
                );
                gate_indent = indent;
                continue;
            }
            if let Some((name, options)) = line.split_once('=') {
                let key = match name.trim() {
                    "arenaOpt" => Some("arena"),
                    "genericOpt" => Some("generic"),
                    "hazardOpt" => Some("hazards"),
                    "botOpt" => Some("bots"),
                    _ => None,
                };
                if let Some(key) = key {
                    let paths =
                        arguments(options.trim().trim_start_matches('(').trim_end_matches(')'))
                            .iter()
                            .filter_map(|s| quoted(s))
                            .map(|s| {
                                if s.contains('/') {
                                    s
                                } else {
                                    format!("Sounds/announcers/{s}")
                                }
                            })
                            .collect::<Vec<_>>();
                    groups.insert(key, paths);
                }
            }
            if line.starts_with("self.intro_music =") {
                intro.music = calls(line, "plus.createSound")
                    .first()
                    .and_then(|c| c.args.first())
                    .and_then(|s| quoted(s));
            }
            for key in ["arena", "generic", "hazards", "bots"] {
                if line.starts_with(&format!("sounds['{key}'] =")) {
                    if let Some(path) = calls(line, "plus.createSound")
                        .first()
                        .and_then(|c| c.args.first())
                        .and_then(|s| quoted(s))
                    {
                        groups.insert(key, vec![path]);
                    }
                }
            }
            for call in calls(line, "plus.fadeOutLoop") {
                if call.args.first().is_some_and(|s| s == "self.intro_music") {
                    intro.fade_out = number(&call.args[1])? / 1000.;
                }
            }
            if let Some(wait) = line.strip_prefix("yield ") {
                if wait != "delaytime" {
                    number(wait)?;
                }
                intro.actions.push((gate, Action::Wait(wait.into())));
            }
            for call in calls(line, "plus.animateCamera") {
                if call.args.len() != 8 {
                    return Err("Unsupported native intro camera".into());
                }
                // Validate every numeric field at import, including inactive shots.
                for i in [0, 3] {
                    vector(&call.args[i])?;
                }
                for i in [1, 4] {
                    rotation(&call.args[i])?;
                }
                for i in [2, 5, 6, 7] {
                    if call.args[i] != "delaytime" {
                        number(&call.args[i])?;
                    }
                }
                intro.actions.push((gate, Action::Camera(call.args)));
            }
            for key in ["arena", "generic", "hazards", "bots"] {
                if line.contains(&format!("plus.playSound(sounds['{key}'])")) {
                    if let Some(paths) = groups.get(key) {
                        intro.actions.push((gate, Action::Sound(paths.clone())));
                    }
                }
            }
        }
        Ok(intro)
    }
    pub fn timeline(&self, hazards: bool, players: [bool; 4], mut seed: u64) -> Timeline {
        let mut result = Timeline {
            music: self.music.clone(),
            fade_out: self.fade_out.max(0.01),
            ..Default::default()
        };
        let delay = 6. - players.iter().filter(|p| **p).count() as f32;
        let value = |s: &str| {
            if s == "delaytime" {
                delay
            } else {
                number(s).unwrap()
            }
        };
        for (gate, action) in &self.actions {
            if !match *gate {
                Gate::Always => true,
                Gate::Hazards => hazards,
                Gate::Player(i) => players.get(i).copied().unwrap_or(false),
            } {
                continue;
            }
            match action {
                Action::Wait(wait) => result.duration += value(wait),
                Action::Camera(args) => result.shots.push(Shot {
                    start: result.duration + value(&args[6]),
                    end: result.duration + value(&args[7]),
                    position: [vector(&args[0]).unwrap(), vector(&args[3]).unwrap()],
                    rotation: [rotation(&args[1]).unwrap(), rotation(&args[4]).unwrap()],
                    fov: [value(&args[2]), value(&args[5])].map(|v| {
                        if v > std::f32::consts::PI {
                            v.to_radians()
                        } else {
                            v
                        }
                    }),
                }),
                Action::Sound(paths) if !paths.is_empty() => {
                    seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                    result.cues.push((
                        result.duration,
                        paths[(seed >> 32) as usize % paths.len()].clone(),
                    ));
                }
                _ => (),
            }
        }
        result
    }
}
fn rotation(text: &str) -> Result<Quat, String> {
    let v = arguments(text.trim().trim_start_matches('(').trim_end_matches(')'));
    if v.len() != 2 {
        return Err("Invalid native camera rotation".into());
    }
    Ok(Quat::from_rotation_y(-number(&v[1])?) * Quat::from_rotation_x(-number(&v[0])?))
}
impl Timeline {
    pub fn camera(&self, age: f32) -> Option<(Vec3, Quat, f32)> {
        let shot = self
            .shots
            .iter()
            .rev()
            .find(|s| age >= s.start)
            .or(self.shots.first())?;
        let t = ((age - shot.start) / (shot.end - shot.start).max(0.001)).clamp(0., 1.);
        Some((
            shot.position[0].lerp(shot.position[1], t),
            shot.rotation[0].slerp(shot.rotation[1], t),
            shot.fov[0] + (shot.fov[1] - shot.fov[0]) * t,
        ))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn all_native_intros_keep_conditional_shots_cues_and_timing() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../RobotArena2");
        if !root.exists() {
            return;
        }
        for arena in super::super::arenas::load(&root).unwrap() {
            let full = arena.presentation.timeline(true, [true; 4], 7);
            let pair = arena
                .presentation
                .timeline(false, [true, true, false, false], 7);
            assert_eq!(
                full.shots.len() - pair.shots.len(),
                arena
                    .presentation
                    .timeline(true, [true, true, false, false], 7)
                    .shots
                    .len()
                    - pair.shots.len()
                    + 2,
                "{}",
                arena.name
            );
            assert_eq!(full.cues.len(), 4, "{}", arena.name);
            assert_eq!(pair.cues.len(), 3, "{}", arena.name);
            assert!(
                full.duration > 18. && full.duration < 40.,
                "{}: {}",
                arena.name,
                full.duration
            );
            for (_, path) in &full.cues {
                assert!(root.join(path).is_file(), "{path}");
            }
            let (position, rotation, fov) = full.camera(0.).unwrap();
            assert!(position.is_finite() && rotation.is_finite() && (0.1..1.6).contains(&fov));
        }
    }
}
