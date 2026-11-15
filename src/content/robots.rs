use arena_core::robot::RobotDefinition;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
#[derive(Clone)]
pub(crate) struct Entry {
    pub path: PathBuf,
    pub team: String,
    pub team_key: String,
    pub logo: String,
    pub motto: String,
    pub robot: Arc<RobotDefinition>,
}
pub(crate) fn load(root: &Path) -> Result<Vec<Entry>, String> {
    let mut result = Vec::new();
    for section in ["Teams", "AI"] {
        let path = root.join(section).join("Teams.txt");
        let Ok(bytes) = std::fs::read(path) else {
            continue;
        };
        let records = if section == "AI" {
            ra2_formats::teams::decode_opponents(&bytes)
        } else {
            let db = ra2_formats::teams::Teams::decode(&bytes)?;
            db.active
                .into_iter()
                .filter_map(|i| db.records.get(i).cloned().map(|t| (i, t)))
                .collect()
        };
        for (id, team) in records {
            for slot in team.robots() {
                let path = root.join(section).join(format!("Team{id}/Bot{slot}.bot"));
                let Some(robot) = std::fs::read(&path)
                    .ok()
                    .and_then(|b| ra2_formats::bot::decode(&b).ok())
                else {
                    continue;
                };
                result.push(Entry {
                    path,
                    team: team.name(),
                    team_key: format!("{section}/Team{id}"),
                    logo: team.logo(),
                    motto: team.motto(),
                    robot: Arc::new(robot),
                });
            }
        }
    }
    if result.is_empty() {
        return Err("No playable native BOT files found in Teams or AI".into());
    }
    Ok(result)
}

/// Stable identities keep separately saved teams selectable even when names match.
pub(crate) fn teams(library: &[Entry]) -> Vec<(&Entry, String)> {
    let mut unique = std::collections::BTreeMap::new();
    for entry in library {
        unique.entry(&entry.team_key).or_insert(entry);
    }
    let mut result = unique
        .values()
        .map(|&entry| {
            let duplicates = unique
                .values()
                .filter(|other| other.team == entry.team)
                .count()
                > 1;
            let label = if duplicates {
                format!("{} ({})", entry.team, entry.team_key)
            } else {
                entry.team.clone()
            };
            (entry, label)
        })
        .collect::<Vec<_>>();
    result.sort_by(|a, b| a.1.to_lowercase().cmp(&b.1.to_lowercase()));
    result
}
