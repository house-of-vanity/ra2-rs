//! Read-only native season data. Field order verified against 1.12 reader 0x4fabf0.
use super::*;
#[derive(Default)]
pub struct Statistics {
    pub seasons: u32,
    pub current_rank: u32,
    pub best_rank: u32,
    pub total_events: u32,
    pub wins: u32,
    pub losses: u32,
    pub events_won: u32,
    pub highest_score: u32,
    pub fastest_win: f32,
    pub points: u32,
    pub opponents: Vec<u32>,
    pub league: usize,
    pub kills: u32,
    pub schedule: Vec<usize>,
    pub prizes: Vec<String>,
}
impl Statistics {
    pub fn read(team: &Team) -> Option<Self> {
        let line = |at: usize| std::str::from_utf8(team.lines.get(at)?).ok();
        let number = |at| line(at)?.trim().parse::<u32>().ok();
        let prizes = number(15)? as usize;
        let flag = 16usize.checked_add(prizes)?;
        let schedule = if line(flag)? == "true" {
            line(flag + 1)?
                .split_whitespace()
                .map(str::parse)
                .collect::<Result<Vec<_>, _>>()
                .ok()?
        } else {
            vec![]
        };
        let tail = team.lines.len().checked_sub(4)?;
        let opponents: Vec<u32> = line(tail)?
            .split_whitespace()
            .map(str::parse)
            .collect::<Result<_, _>>()
            .ok()?;
        if opponents.len() != 15 {
            return None;
        }
        Some(Self {
            seasons: number(5)?.saturating_add(1),
            current_rank: number(6)?,
            best_rank: number(7)?,
            total_events: number(8)?,
            wins: number(9)?,
            losses: number(10)?,
            events_won: number(11)?,
            highest_score: number(12)?,
            fastest_win: line(13)?.parse().ok()?,
            points: number(tail + 2)?,
            opponents,
            kills: number(tail + 1)?,
            league: line(tail + 3)?.split_whitespace().next()?.parse().ok()?,
            schedule,
            prizes: (16..flag)
                .filter_map(|i| line(i).map(String::from))
                .collect(),
        })
    }
}
pub struct Event {
    pub name: String,
    pub description: String,
    pub preview: String,
}
fn events(bytes: &[u8]) -> Result<Vec<Event>, String> {
    let text = String::from_utf8_lossy(bytes);
    let lines: Vec<_> = text.lines().collect();
    let mut at = 0;
    let mut result = vec![];
    while at < lines.len() {
        if lines[at].is_empty() {
            at += 1;
            continue;
        }
        let id = lines[at]
            .parse::<usize>()
            .map_err(|_| "Invalid native event ID")?;
        if id != result.len() {
            return Err("Native event IDs must be consecutive".into());
        }
        at += 1;
        let rounds = lines
            .get(at + 2)
            .and_then(|l| l.parse::<usize>().ok())
            .filter(|&n| n <= 16)
            .ok_or("Invalid native event rounds")?;
        let preview = at + 3 + 2 * (rounds + 1) + 1;
        let end = at + 3 + 2 * (rounds + 1) + 7;
        if end > lines.len() {
            return Err("Truncated Events.txt record".into());
        }
        result.push(Event {
            name: lines[at].into(),
            description: lines[at + 1].into(),
            preview: ui_path(lines[preview]),
        });
        at = end;
    }
    Ok(result)
}
pub fn ui_path(path: &str) -> String {
    let path = normalize(path);
    path.strip_prefix("ui/").unwrap_or(&path).to_string()
}
#[derive(Resource, Default)]
pub struct NativeData {
    pub opponents: Vec<Team>,
    pub events: Vec<Event>,
}
impl NativeData {
    pub fn load(root: &Path) -> Self {
        let opponents = std::fs::read(root.join("AI/Teams.txt"))
            .ok()
            .map(|b| {
                ra2_formats::teams::decode_opponents(&b)
                    .into_iter()
                    .map(|(_, t)| t)
                    .collect()
            })
            .unwrap_or_default();
        let events = std::fs::read(root.join("Events.txt"))
            .map_err(|e| e.to_string())
            .and_then(|b| events(&b))
            .unwrap_or_else(|e| {
                warn!("Native event catalogue: {e}");
                vec![]
            });
        Self { opponents, events }
    }
}
pub fn bind(
    data: Option<&NativeData>,
    hq: &Headquarters,
    team: &Team,
    assets: &mut LegacyAssets,
    ui: &mut legacy_ui::State,
) {
    let Some(stats) = Statistics::read(team) else {
        return;
    };
    let rank = |n: u32| {
        if (1..=16).contains(&n) {
            n.to_string()
        } else {
            "n/a".into()
        }
    };
    let league = ["AMATEUR", "SEMI-PROFESSIONAL", "PROFESSIONAL"][stats.league.min(2)];
    ui.values
        .insert("season".into(), format!("Season {}", stats.seasons));
    ui.values.insert(
        "competition level".into(),
        format!("{league} LEAGUE STANDINGS"),
    );
    let rank_now = stats
        .opponents
        .iter()
        .filter(|&&score| score > stats.points)
        .count() as u32
        + 1;
    for (key, value) in [
        ("total seasons", stats.seasons.to_string()),
        (
            "current rank",
            rank(
                if !stats.schedule.is_empty() && (1..=16).contains(&stats.current_rank) {
                    stats.current_rank
                } else {
                    rank_now
                },
            ),
        ),
        (
            "best rank",
            if stats.seasons > 1 {
                rank(stats.best_rank)
            } else {
                "n/a".into()
            },
        ),
        ("total events", stats.total_events.to_string()),
        ("wins", stats.wins.to_string()),
        ("losses", stats.losses.to_string()),
        ("events won", stats.events_won.to_string()),
        ("highest score", stats.highest_score.to_string()),
        (
            "fastest win",
            if stats.fastest_win < 10000. && stats.fastest_win.is_finite() {
                let n = stats.fastest_win as u32;
                format!("{}:{:02}", n / 60, n % 60)
            } else {
                "n/a".into()
            },
        ),
    ] {
        ui.values.insert(key.into(), value);
    }
    ui.values.insert(
        "events remaining".into(),
        format!("Events remaining: {}", stats.schedule.len()),
    );
    ui.values.insert(
        "schedule title".into(),
        format!("Event Schedule - Season {}", stats.seasons),
    );
    let mut kills = stats.kills;
    let mut x = 0;
    for value in [100, 20, 5, 1] {
        let count = (kills / value).min(64 - x);
        kills %= value;
        for _ in 0..count {
            let fields = [
                ("Name", format!("kill skull {x}")),
                ("Position", format!("{} 0 0 0", x * 24)),
                ("Size", "24 0 24 0".into()),
                ("Visible", "true".into()),
                ("_Filename", format!("teamhq/skull{value}.tga")),
                ("_ForeColor", "1 1 1 1".into()),
                ("_Alpha", "true".into()),
            ]
            .into_iter()
            .map(|(k, v)| (k.to_string(), vec![v]))
            .collect();
            ui.attachments.push((
                "kills group".into(),
                Arc::new(Widget {
                    kind: "Image".into(),
                    fields,
                    wrapped: None,
                    children: vec![],
                }),
            ));
            x += 1;
        }
    }
    if hq.tab == 1
        && let Some(data) = data
    {
        let mut standings: Vec<_> = std::iter::once((team, stats.points, true))
            .chain(
                data.opponents
                    .iter()
                    .enumerate()
                    .map(|(i, t)| (t, stats.opponents.get(i).copied().unwrap_or(0), false)),
            )
            .collect();
        standings.sort_by_key(|a| std::cmp::Reverse(a.1));
        ui.cells.insert(
            "league grid".into(),
            standings
                .into_iter()
                .enumerate()
                .filter_map(|(rank, (team, points, player))| {
                    let mut cell = (*assets.widget("teamhq/league_cell.gib").ok()?).clone();
                    for (name, value) in [
                        ("league team name", team.name()),
                        ("motto", team.motto()),
                        ("points", points.to_string()),
                        ("rank", (rank + 1).to_string()),
                    ] {
                        editor::panels::text(&mut cell, name, &value);
                    }
                    patch_image(&mut cell, "league team logo", &team.logo());
                    Some(Cell {
                        key: format!("standings:{rank}"),
                        selected: player,
                        template: Some(Arc::new(cell)),
                        ..default()
                    })
                })
                .collect(),
        );
    }
    if hq.tab == 2
        && let Some(data) = data
    {
        // Until a new season is started, display actual available events from
        // the catalogue; an existing saved nine-event schedule is used verbatim.
        let schedule: Vec<_> = if stats.schedule.is_empty() {
            (0..data.events.len().min(9)).collect()
        } else {
            stats.schedule
        };
        ui.cells.insert(
            "event grid".into(),
            schedule
                .into_iter()
                .filter_map(|id| {
                    let event = data.events.get(id)?;
                    let mut cell = (*assets.widget("teamhq/eventcell.gib").ok()?).clone();
                    patch_image(&mut cell, "preview img", &event.preview);
                    Some(Cell {
                        key: format!("event:{id}"),
                        template: Some(Arc::new(cell)),
                        ..default()
                    })
                })
                .collect(),
        );
    }
    if hq.tab == 3 {
        ui.cells.insert(
            "prizes grid".into(),
            stats
                .prizes
                .iter()
                .enumerate()
                .map(|(i, path)| Cell {
                    key: format!("prize:{i}"),
                    image: if assets.image(&ui_path(path)).is_some() {
                        ui_path(path)
                    } else {
                        "teamhq/questionmark.bmp".into()
                    },
                    ..default()
                })
                .collect(),
        );
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires original native season assets"]
    fn original_season_catalogue_loads_all_events_and_opponents() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../RobotArena2");
        let raw = std::fs::read(root.join("Events.txt")).unwrap();
        let catalogue = events(&raw).unwrap();
        assert_eq!(catalogue.len(), 25);
        assert_eq!(catalogue[0].name, "F.O.R.C.E. Tournament");
        assert_eq!(
            catalogue[24].name,
            "Father-Daughter Brunch and Robotic Combat"
        );
        assert_eq!(catalogue[0].preview, "teamhq/events/ev_force.bmp");
        let data = NativeData::load(&root);
        assert_eq!(data.opponents.len(), 15);
        for team in &data.opponents {
            assert!(Statistics::read(team).is_some(), "{}", team.name());
        }
    }
    #[test]
    fn native_statistics_and_scheduled_event_indices_follow_variable_prize_list() {
        let mut team = Team::empty();
        team.lines[5] = b"3".to_vec();
        team.lines[9] = b"17".to_vec();
        team.lines[15] = b"1".to_vec();
        team.lines.insert(16, b"UI/teamhq/trophy1.tga".to_vec());
        team.lines[17] = b"true".to_vec();
        team.lines.insert(18, b"8 7 6 5 4 3 2 1 0".to_vec());
        let stats = Statistics::read(&team).unwrap();
        assert_eq!(stats.seasons, 4);
        assert_eq!(stats.wins, 17);
        assert_eq!(stats.schedule, [8, 7, 6, 5, 4, 3, 2, 1, 0]);
        assert_eq!(stats.prizes, ["UI/teamhq/trophy1.tga"]);
        assert_eq!(stats.opponents.len(), 15);
    }
}
