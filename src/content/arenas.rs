//! Read-only declarations from original arena scripts. No Python runtime/eval.
use arena_core::{Vec3, match_rules::Mode};
use std::{collections::BTreeMap, path::Path};
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum HazardKind {
    Spikes,
    Saw,
    PitSlider,
    Hammer,
    HellRaiser,
    Smasher,
    TrapDoor,
    Flame,
    Electricity,
    Rotator,
}
#[derive(Clone, Debug)]
pub(crate) struct Slider {
    pub name: String,
    pub parent: String,
    pub child: String,
    pub axis: Vec3,
    pub limits: [f32; 2],
}
#[derive(Clone, Debug)]
pub(crate) struct Hazard {
    pub key: String,
    pub kind: HazardKind,
    pub joints: Vec<String>,
    pub zones: Vec<String>,
    pub buttons: Vec<String>,
    pub power: f32,
    pub delay: f32,
    pub speed: f32,
    pub location: Vec3,
    pub velocity: Vec3,
    pub ray_offset: f32,
}
#[derive(Clone, Debug)]
pub(crate) struct Arena {
    pub key: String,
    pub name: String,
    pub model: String,
    pub preview: String,
    pub description: String,
    pub modes: Vec<Mode>,
    pub sliders: Vec<Slider>,
    pub hazards: Vec<Hazard>,
    pub sounds: BTreeMap<String, String>,
    pub ringout_y: Option<f32>,
    pub king_zone: Option<String>,
    pub presentation: super::introduction::Introduction,
    pub ambience: Vec<String>,
}
#[derive(Clone, Debug)]
pub(crate) struct Call {
    pub name: String,
    pub args: Vec<String>,
    pub line: usize,
}
/// Commas inside tuples and quoted names are retained. Comments are discarded.
pub(crate) fn arguments(text: &str) -> Vec<String> {
    let mut quote = None;
    let mut depth = 0i32;
    let mut start = 0;
    let mut out = vec![];
    for (i, c) in text.char_indices() {
        if let Some(q) = quote {
            if c == q {
                quote = None;
            }
            continue;
        }
        match c {
            '\'' | '"' => quote = Some(c),
            '(' | '[' => depth += 1,
            ')' | ']' => depth -= 1,
            ',' if depth == 0 => {
                out.push(text[start..i].trim().to_string());
                start = i + 1;
            }
            _ => (),
        }
    }
    if !text[start..].trim().is_empty() {
        out.push(text[start..].trim().into());
    }
    out
}
pub(crate) fn quoted(s: &str) -> Option<String> {
    let s = s.trim();
    let q = s.chars().next()?;
    (['\'', '"'].contains(&q) && s.ends_with(q))
        .then(|| s[1..s.len() - 1].replace("\\\\", "/").replace('\\', "/"))
}
pub(crate) fn number(s: &str) -> Result<f32, String> {
    s.trim()
        .parse::<f32>()
        .ok()
        .filter(|v| v.is_finite())
        .ok_or_else(|| format!("Unsupported arena number {s}"))
}
pub(crate) fn vector(s: &str) -> Result<Vec3, String> {
    let fields = arguments(s.trim().trim_start_matches('(').trim_end_matches(')'));
    if fields.len() != 3 {
        return Err(format!("Invalid arena vector {s}"));
    }
    Ok(Vec3::new(
        number(&fields[0])?,
        number(&fields[1])?,
        -number(&fields[2])?,
    ))
}
pub(crate) fn calls(source: &str, needle: &str) -> Vec<Call> {
    let mut out = vec![];
    for (line, raw) in source.lines().enumerate() {
        let text = raw.trim();
        if text.starts_with('#') {
            continue;
        }
        let mut remaining = text;
        let mut base = 0;
        while let Some(i) = remaining.find(&format!("{needle}(")) {
            let start = base + i + needle.len() + 1;
            let mut depth = 1;
            let mut quote = None;
            let mut end = None;
            for (offset, c) in text[start..].char_indices() {
                if let Some(q) = quote {
                    if c == q {
                        quote = None;
                    }
                    continue;
                }
                match c {
                    '\'' | '"' => quote = Some(c),
                    '(' => depth += 1,
                    ')' => {
                        depth -= 1;
                        if depth == 0 {
                            end = Some(start + offset);
                            break;
                        }
                    }
                    _ => (),
                }
            }
            let Some(end) = end else {
                break;
            };
            out.push(Call {
                name: needle.into(),
                args: arguments(&text[start..end]),
                line,
            });
            base = end + 1;
            remaining = &text[base..];
        }
    }
    out
}
fn value(source: &str, name: &str) -> Option<String> {
    source
        .lines()
        .filter_map(|line| {
            let (a, b) = line.trim().split_once('=')?;
            (a.trim() == name).then(|| b.trim().to_string())
        })
        .last()
}
pub(crate) fn load(root: &Path) -> Result<Vec<Arena>, String> {
    let mut paths = std::fs::read_dir(root.join("Arenas"))
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "py"))
        .collect::<Vec<_>>();
    paths.sort();
    let mut result = vec![];
    for path in paths {
        let source = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
        if !source.contains("Arenas.register(")
            || value(&source, "in_list").is_some_and(|v| v == "False")
        {
            continue;
        }
        let arena = parse(
            path.file_stem().unwrap().to_string_lossy().as_ref(),
            &source,
        )?;
        if !root.join(&arena.model).is_file() {
            return Err(format!("Missing native arena {}", arena.model));
        }
        result.push(arena);
    }
    result.sort_by_key(|a| (a.key != "box", a.name.clone()));
    if result.is_empty() {
        return Err("No original arena declarations found".into());
    }
    Ok(result)
}
fn parse(key: &str, source: &str) -> Result<Arena, String> {
    let name = value(source, "name")
        .and_then(|v| quoted(&v))
        .ok_or("Arena has no name")?;
    let modes_text = value(source, "game_types").ok_or("Arena has no game types")?;
    let modes = Mode::ALL
        .into_iter()
        .filter(|m| modes_text.contains(m.tag()))
        .collect::<Vec<_>>();
    let mut construction = calls(source, "SuperArena.__init__");
    construction.extend(calls(source, "KingOfHill.__init__"));
    let model = construction
        .iter()
        .find_map(|c| c.args.get(1).and_then(|s| quoted(s)))
        .ok_or("Arena has no model")?;
    let king_zone = construction
        .iter()
        .find_map(|c| c.args.get(2).and_then(|s| quoted(s)));
    let preview = format!(
        "Arenas/{}",
        value(source, "preview")
            .and_then(|v| quoted(&v))
            .ok_or("Arena has no preview")?
    );
    let lines = source.lines().map(str::trim).collect::<Vec<_>>();
    let description = lines
        .windows(2)
        .filter(|p| p[0].starts_with("class "))
        .filter_map(|p| quoted(p[1]))
        .last()
        .unwrap_or_default();
    let ringout_y = lines.iter().find_map(|line| {
        line.split("plus.getLocation(each)[1] <")
            .nth(1)
            .and_then(|s| s.trim().trim_end_matches(':').split_whitespace().next())
            .and_then(|s| number(s).ok())
    });
    let mut sliders = vec![];
    let mut vars = BTreeMap::<String, String>::new();
    let mut hazards = vec![];
    let mut declarations = calls(source, "self.AddPrismatic");
    declarations.extend(calls(source, "self.GetHinge"));
    for kind in [
        "Spikes",
        "Saws",
        "PitSlider",
        "Hammer",
        "HellRaiser",
        "Smasher",
        "TrapDoor",
        "Flame",
        "Electricity",
    ] {
        declarations.extend(calls(source, &format!("Hazards.{kind}")));
    }
    declarations.sort_by_key(|c| c.line);
    for call in declarations {
        let line = lines[call.line];
        let variable = line.split('=').next().unwrap_or("").trim().to_string();
        if call.name == "self.AddPrismatic" {
            if call.args.len() != 8 {
                return Err(format!("Unsupported native slider in {key}: {line}"));
            }
            let name = format!("slider{}", sliders.len());
            sliders.push(Slider {
                name: name.clone(),
                parent: quoted(&call.args[0]).ok_or("Slider parent")?,
                child: quoted(&call.args[1]).ok_or("Slider child")?,
                axis: Vec3::new(
                    number(&call.args[2])?,
                    number(&call.args[3])?,
                    -number(&call.args[4])?,
                )
                .normalize(),
                limits: [number(&call.args[5])?, number(&call.args[6])?],
            });
            vars.insert(variable, name);
        } else if call.name == "self.GetHinge" {
            vars.insert(variable, quoted(&call.args[0]).ok_or("Hinge name")?);
        } else {
            let kind = match call.name.as_str() {
                "Hazards.Spikes" => HazardKind::Spikes,
                "Hazards.Saws" => HazardKind::Saw,
                "Hazards.PitSlider" => HazardKind::PitSlider,
                "Hazards.Hammer" => HazardKind::Hammer,
                "Hazards.HellRaiser" => HazardKind::HellRaiser,
                "Hazards.Smasher" => HazardKind::Smasher,
                "Hazards.TrapDoor" => HazardKind::TrapDoor,
                "Hazards.Flame" => HazardKind::Flame,
                _ => HazardKind::Electricity,
            };
            let mut variable = variable;
            if let Some((base, _)) = line.split_once(".append(") {
                variable = format!(
                    "{}[{}]",
                    base.trim(),
                    hazards
                        .iter()
                        .filter(|h: &&Hazard| h.key.starts_with(base.trim()))
                        .count()
                );
            }
            let joint_count = if kind == HazardKind::Smasher {
                2
            } else if matches!(kind, HazardKind::Flame | HazardKind::Electricity) {
                0
            } else {
                1
            };
            let joints = call
                .args
                .iter()
                .take(joint_count)
                .map(|a| {
                    vars.get(a)
                        .cloned()
                        .ok_or_else(|| format!("Unknown joint variable {a} in {key}"))
                })
                .collect::<Result<Vec<_>, _>>()?;
            let location = call
                .args
                .iter()
                .skip(joint_count)
                .find(|s| s.starts_with('('))
                .map_or(Ok(Vec3::ZERO), |s| vector(s))?;
            let velocity = if kind == HazardKind::Flame {
                call.args.get(1).map_or(Ok(Vec3::Y * 7.), |s| vector(s))?
            } else {
                Vec3::ZERO
            };
            hazards.push(Hazard {
                key: variable,
                kind,
                joints,
                zones: vec![],
                buttons: vec![],
                power: if kind == HazardKind::Spikes {
                    number(&call.args[1])?
                } else {
                    2000.
                },
                delay: if kind == HazardKind::PitSlider {
                    number(&call.args[1])?
                } else {
                    0.
                },
                speed: 0.2,
                ray_offset: if kind == HazardKind::Flame {
                    call.args.get(3).map_or(Ok(0.), |s| number(s))?
                } else {
                    0.
                },
                location,
                velocity,
            });
        }
    }
    // Rotating claws call the hinge API directly instead of constructing a Hazard.
    for (var, joint) in &vars {
        if source.contains(&format!("{var}.SetDirection(-100)")) {
            hazards.push(Hazard {
                key: var.clone(),
                kind: HazardKind::Rotator,
                joints: vec![joint.clone()],
                zones: vec![],
                buttons: vec![],
                power: 1000.,
                delay: 0.,
                speed: 2.,
                location: Vec3::ZERO,
                velocity: Vec3::ZERO,
                ray_offset: 0.,
            });
        }
    }
    // Honor per-arena motor overrides (e.g. the Skull's slowly moving grate).
    for hazard in &mut hazards {
        if hazard.kind == HazardKind::PitSlider {
            for call in calls(source, &format!("{}.slider.SetPowerSettings", hazard.key)) {
                if let [speed, force] = call.args.as_slice() {
                    hazard.speed = number(speed)?.abs();
                    hazard.power = number(force)?.abs();
                }
            }
        }
    }
    let ambience = calls(source, "plus.createSound")
        .into_iter()
        .filter_map(|c| {
            let path = quoted(c.args.first()?)?;
            let var = lines[c.line].split('=').next()?.trim();
            (var.starts_with("self.")
                && source.contains(&format!("plus.loopSound({var})"))
                && path.to_lowercase().contains("ambience"))
            .then_some(path)
        })
        .collect();
    let zones = calls(source, "self.RegisterZone")
        .into_iter()
        .filter_map(|c| {
            Some((
                c.args.get(1)?.parse::<u32>().ok()?,
                quoted(c.args.first()?)?,
            ))
        })
        .collect::<BTreeMap<_, _>>();
    let mut id = None;
    let mut in_event = false;
    for line in &lines {
        if line.starts_with("def ") {
            in_event = line.starts_with("def ZoneEvent");
            id = None;
        }
        if !in_event || line.starts_with('#') {
            continue;
        }
        if let Some(s) = line
            .split("id")
            .nth(1)
            .and_then(|s| s.trim_start().strip_prefix("=="))
        {
            id = s
                .trim_start()
                .split(|c: char| !c.is_ascii_digit())
                .next()
                .and_then(|s| s.parse::<u32>().ok());
        }
        for h in &mut hazards {
            if line.contains(&format!("{}.", h.key))
                && let Some(zone) = id.and_then(|id| zones.get(&id))
            {
                let target = if line.contains(".Button()") {
                    &mut h.buttons
                } else {
                    &mut h.zones
                };
                if !target.contains(zone) {
                    target.push(zone.clone());
                }
            }
        }
    }
    for h in &mut hazards {
        if h.key.starts_with("self.teeth[") {
            if let Some(i) = h
                .key
                .split('[')
                .nth(1)
                .and_then(|s| s.trim_end_matches(']').parse::<u32>().ok())
                && let Some(zone) = zones.get(&(i + 1))
            {
                h.zones.push(zone.clone());
            }
        }
    }
    let sounds = calls(source, "self.SetSubMaterialSound")
        .into_iter()
        .filter_map(|c| Some((quoted(c.args.first()?)?, quoted(c.args.get(3)?)?)))
        .collect();
    Ok(Arena {
        key: key.to_lowercase(),
        name,
        model,
        preview,
        description,
        modes,
        sliders,
        hazards,
        sounds,
        ringout_y,
        king_zone,
        presentation: super::introduction::Introduction::parse(source)?,
        ambience,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tuples_and_quoted_names_remain_single_arguments() {
        assert_eq!(
            calls("# x(ignored)\nx(\"a,b\", (1,2,3), -0.8)", "x")[0].args,
            ["\"a,b\"", "(1,2,3)", "-0.8"]
        );
    }
    #[test]
    #[ignore = "requires installed original game"]
    fn native_arena_catalog_has_all_modes_and_hazard_bindings() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../RobotArena2");
        let arenas = load(&root).unwrap();
        assert_eq!(arenas.len(), 12);
        for mode in Mode::ALL {
            assert!(arenas.iter().any(|a| a.modes.contains(&mode)));
        }
        for a in arenas {
            for h in a.hazards {
                assert!(
                    !h.zones.is_empty()
                        || matches!(h.kind, HazardKind::PitSlider | HazardKind::Rotator),
                    "{} {} missing zone",
                    a.name,
                    h.key
                );
            }
        }
    }
}
