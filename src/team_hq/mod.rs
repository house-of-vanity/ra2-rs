//! Original team-selection/HQ layouts bound to native Teams.txt and team BOT slots.
mod data;
use crate::{
    Options,
    editor::{self, Editor},
    frontend::{Draft, Navigation, Page},
    legacy_ui::{
        self, Cell, UiEvent,
        assets::{LegacyAssets, decode_image},
    },
    settings::EngineSettings,
};
use bevy::prelude::*;
use ra2_formats::{
    components::normalize,
    gib::Widget,
    teams::{Team, Teams},
};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Clone, Debug)]
enum Dialog {
    Team(Option<usize>),
    DeleteTeam,
    Scrap,
    Import,
    Export,
    Message(String),
    Help,
    Event(usize),
}
#[derive(Resource)]
pub struct Headquarters {
    pub root: PathBuf,
    pub teams: Teams,
    pub selected: Option<usize>,
    pub slot: usize,
    slot_selected: bool,
    tab: usize,
    dialog: Option<Dialog>,
    logo: String,
    revision: u64,
}
pub struct TeamHqPlugin;
#[derive(SystemSet, Debug, Clone, Hash, PartialEq, Eq)]
struct Input;
impl Plugin for TeamHqPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup.after(legacy_ui::assets::load))
            .configure_sets(Update, Input.after(legacy_ui::Input).before(editor::Edit))
            .add_systems(Update, input.in_set(Input))
            .add_systems(Update, sync.after(editor::Edit).before(legacy_ui::Paint));
    }
}
pub fn robot_path(root: &Path, team: usize, slot: usize) -> PathBuf {
    root.join("Teams")
        .join(format!("Team{team}"))
        .join(format!("Bot{slot}.bot"))
}
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let temp = path.with_extension("tmp");
    std::fs::write(&temp, bytes).map_err(|e| e.to_string())?;
    std::fs::rename(&temp, path).map_err(|e| e.to_string())
}
pub fn register_robot(root: &Path, team: usize, slot: usize) -> Result<(), String> {
    let path = root.join("Teams/Teams.txt");
    let mut db = Teams::decode(&std::fs::read(&path).map_err(|e| e.to_string())?)?;
    if !db.active.contains(&team) {
        return Err("The robot's team no longer exists".into());
    }
    db.records[team].set_robot(slot, true)?;
    write_atomic(&path, &db.encode())
}
fn setup(mut commands: Commands, options: Res<Options>, mut assets: ResMut<LegacyAssets>) {
    let teams = std::fs::read(options.game_dir.join("Teams/Teams.txt"))
        .map_err(|e| e.to_string())
        .and_then(|b| Teams::decode(&b));
    let (teams, dialog) = match teams {
        Ok(db) => (db, None),
        Err(e) => (
            Teams::default(),
            Some(Dialog::Message(format!("Cannot read Teams/Teams.txt: {e}"))),
        ),
    };
    let mut logos = vec![];
    let _ = crate::content::scan(&options.game_dir.join("Teams"), &mut logos);
    for path in logos
        .into_iter()
        .filter(|p| p.extension().is_some_and(|s| s.eq_ignore_ascii_case("bmp")))
    {
        if let Ok(bytes) = std::fs::read(&path)
            && let Ok(image) = decode_image(&bytes, "bmp")
        {
            assets.images.insert(
                normalize(
                    &path
                        .strip_prefix(&options.game_dir)
                        .unwrap()
                        .to_string_lossy(),
                ),
                image,
            );
        }
    }
    let data = data::NativeData::load(&options.game_dir);
    for team in &data.opponents {
        let path = options.game_dir.join(team.logo());
        if let Ok(bytes) = std::fs::read(path)
            && let Ok(image) = decode_image(&bytes, "bmp")
        {
            assets.images.insert(normalize(&team.logo()), image);
        }
    }
    commands.insert_resource(data);
    let selected = teams.active.first().copied();
    commands.insert_resource(Headquarters {
        root: options.game_dir.clone(),
        teams,
        selected,
        slot: 0,
        slot_selected: false,
        tab: 0,
        dialog,
        logo: String::new(),
        revision: 0,
    });
}
impl Headquarters {
    fn persist(&mut self, draft: Teams) -> Result<(), String> {
        write_atomic(&self.root.join("Teams/Teams.txt"), &draft.encode())?;
        self.teams = draft;
        self.revision += 1;
        Ok(())
    }
    fn import_robot(&mut self, bytes: &[u8]) -> Result<(), String> {
        ra2_formats::bot::decode(bytes)?;
        let id = self.team()?;
        let path = robot_path(&self.root, id, self.slot);
        let previous = match std::fs::read(&path) {
            Ok(bytes) => Some(bytes),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.to_string()),
        };
        let mut db = self.teams.clone();
        db.records[id].set_robot(self.slot, true)?;
        write_atomic(&path, bytes)?;
        if let Err(error) = self.persist(db) {
            let rollback = if let Some(bytes) = previous {
                write_atomic(&path, &bytes)
            } else {
                std::fs::remove_file(&path).map_err(|e| e.to_string())
            };
            return Err(match rollback {
                Ok(()) => error,
                Err(rollback) => format!("{error}; could not restore robot slot: {rollback}"),
            });
        }
        Ok(())
    }
    fn team(&self) -> Result<usize, String> {
        self.selected
            .filter(|i| self.teams.active.contains(i))
            .ok_or("Select a team".into())
    }
    fn design(&self, name: &str) -> Result<PathBuf, String> {
        let name = name.trim().trim_end_matches(".bot");
        if name.is_empty() || name.contains(['/', '\\', ':', '\0']) || name == ".." {
            return Err("Enter a robot filename".into());
        }
        Ok(self.root.join("Robot Designs").join(format!("{name}.bot")))
    }
    #[allow(
        clippy::too_many_arguments,
        reason = "Original command, UI, document and navigation context"
    )]
    fn activate(
        &mut self,
        name: &str,
        owner: &str,
        ui: &mut legacy_ui::State,
        editor: &mut Editor,
        nav: &mut Navigation,
        draft: &mut Draft,
        settings: &EngineSettings,
    ) -> Result<(), String> {
        if name == "Options button" {
            nav.back = nav.page;
            nav.page = Page::Settings;
            draft.0 = settings.clone();
            return Ok(());
        }
        if name.eq_ignore_ascii_case("MainMenu button")
            || name.eq_ignore_ascii_case("main menu button")
        {
            self.dialog = None;
            nav.page = Page::Main;
            return Ok(());
        }
        if name == "Help button" {
            self.dialog = Some(Dialog::Help);
            return Ok(());
        }
        if name == "enter event button" {
            self.dialog = Some(Dialog::Message(
                "Competition events are not available in this build yet.".into(),
            ));
            return Ok(());
        }
        if name == "cancel button" || name == "no button" || name == "close button" {
            self.dialog = None;
            return Ok(());
        }
        if name == "ok button" || name == "yes button" {
            match self.dialog.clone() {
                Some(Dialog::Team(edit)) => {
                    let mut db = self.teams.clone();
                    let id = match edit {
                        Some(id) => id,
                        None => (0..db.records.len())
                            .find(|i| !db.active.contains(i))
                            .ok_or("All sixteen team slots are occupied")?,
                    };
                    if edit.is_none() {
                        db.records[id] = Team::empty();
                        db.active.push(id);
                    }
                    db.records[id].set_identity(
                        ui.values
                            .get("name input textbox")
                            .map(String::as_str)
                            .unwrap_or(""),
                        ui.values
                            .get("team motto input textbox")
                            .map(String::as_str)
                            .unwrap_or(""),
                        &self.logo,
                    )?;
                    self.persist(db)?;
                    self.selected = Some(id);
                }
                Some(Dialog::DeleteTeam) => {
                    let id = self.team()?;
                    let mut db = self.teams.clone();
                    db.active.retain(|&i| i != id);
                    self.persist(db)?;
                    self.selected = self.teams.active.first().copied();
                }
                Some(Dialog::Scrap) => {
                    let id = self.team()?;
                    let mut db = self.teams.clone();
                    db.records[id].set_robot(self.slot, false)?;
                    self.persist(db)?;
                }
                Some(Dialog::Import) => {
                    let path = self.design(
                        ui.values
                            .get("import path")
                            .ok_or("Select a robot design")?,
                    )?;
                    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
                    self.import_robot(&bytes)?;
                }
                Some(Dialog::Export) => {
                    let path = self.design(
                        ui.values
                            .get("file name")
                            .ok_or("Enter an export filename")?,
                    )?;
                    let bytes = std::fs::read(robot_path(&self.root, self.team()?, self.slot))
                        .map_err(|e| e.to_string())?;
                    write_atomic(&path, &bytes)?;
                }
                _ => (),
            }
            self.dialog = None;
            return Ok(());
        }
        if matches!(name, "Less" | "Down") && ui.cells.contains_key(owner) {
            let n = ui.scroll.entry(owner.into()).or_default();
            *n = if name == "Less" {
                n.saturating_sub(1)
            } else {
                n.saturating_add(1)
            };
            return Ok(());
        }
        if self.dialog.is_some() {
            if matches!(self.dialog, Some(Dialog::Event(_)))
                && let Some(slot) = name
                    .strip_prefix("robot:")
                    .and_then(|s| s.parse::<usize>().ok())
            {
                self.slot = slot.min(5);
                self.slot_selected = true;
            }
            if let Some(logo) = name.strip_prefix("logo:") {
                self.logo = logo.into();
            }
            if let Some(design) = name.strip_prefix("design:") {
                ui.values.insert("import path".into(), design.into());
            }
            return Ok(());
        }
        if let Some(id) = name
            .strip_prefix("event:")
            .and_then(|s| s.parse::<usize>().ok())
        {
            self.slot_selected = false;
            self.dialog = Some(Dialog::Event(id));
            return Ok(());
        }
        if let Some(id) = name
            .strip_prefix("team:")
            .and_then(|s| s.parse::<usize>().ok())
        {
            if self.teams.active.contains(&id) {
                self.selected = Some(id);
            }
            return Ok(());
        }
        if let Some(slot) = name
            .strip_prefix("robot:")
            .and_then(|s| s.parse::<usize>().ok())
        {
            self.slot = slot.min(5);
            self.slot_selected = true;
            return Ok(());
        }
        match name {
            "new team button" | "edit team button" => {
                let edit = if name == "edit team button" {
                    Some(self.team()?)
                } else {
                    None
                };
                ui.values.insert(
                    "name input textbox".into(),
                    edit.map(|i| self.teams.records[i].name())
                        .unwrap_or_default(),
                );
                ui.values.insert(
                    "team motto input textbox".into(),
                    edit.map(|i| self.teams.records[i].motto())
                        .unwrap_or_default(),
                );
                self.logo = edit
                    .map(|i| self.teams.records[i].logo())
                    .unwrap_or_else(|| "Teams/orange_skull.bmp".into());
                self.dialog = Some(Dialog::Team(edit));
            }
            "delete team button" => {
                self.team()?;
                self.dialog = Some(Dialog::DeleteTeam);
            }
            "continue button" => {
                self.team()?;
                self.tab = 0;
                self.slot = 0;
                self.slot_selected = false;
                nav.page = Page::TeamHQ;
            }
            "select team button" => nav.page = Page::TeamSelect,
            "robots button" => self.tab = 0,
            "league button" => self.tab = 1,
            "events button" => self.tab = 2,
            "history button" => self.tab = 3,
            "workshop button" => {
                let id = self.team()?;
                let path = robot_path(&self.root, id, self.slot);
                if self.teams.records[id].robots().contains(&self.slot) {
                    editor::controls::load(editor, &path)?;
                } else {
                    let revision = editor.document.revision + 1;
                    *editor = Editor::default();
                    editor.document.revision = revision;
                }
                editor.team_slot = Some((id, self.slot));
                editor.team_root = Some(self.root.clone());
                editor.return_page = Page::TeamHQ;
                editor.mode = editor::Mode::Overview;
                editor.dialog = None;
                editor.pending = None;
                editor.ui_revision += 1;
                nav.page = Page::Workshop;
            }
            "import button" => {
                ui.values.insert("import path".into(), String::new());
                self.dialog = Some(Dialog::Import);
            }
            "export button" => {
                let robot = ra2_formats::bot::decode(
                    &std::fs::read(robot_path(&self.root, self.team()?, self.slot))
                        .map_err(|e| e.to_string())?,
                )?;
                ui.values.insert("file name".into(), robot.name);
                self.dialog = Some(Dialog::Export);
            }
            "scrap button" => self.dialog = Some(Dialog::Scrap),
            _ => (),
        }
        Ok(())
    }
}
#[allow(clippy::too_many_arguments)]
fn input(
    mut hq: ResMut<Headquarters>,
    mut events: ResMut<legacy_ui::Events>,
    mut ui: ResMut<legacy_ui::State>,
    mut editor: ResMut<Editor>,
    mut nav: ResMut<Navigation>,
    mut draft: ResMut<Draft>,
    settings: Res<EngineSettings>,
    mut sounds: ResMut<crate::ui_sound::Sounds>,
) {
    if !matches!(nav.page, Page::TeamSelect | Page::TeamHQ) {
        return;
    }
    for event in std::mem::take(&mut events.0) {
        let result = match event {
            UiEvent::Activate { name, owner, .. } => {
                sounds.play("Sounds/button_click.wav");
                hq.activate(
                    &name,
                    &owner,
                    &mut ui,
                    &mut editor,
                    &mut nav,
                    &mut draft,
                    &settings,
                )
            }
            UiEvent::Value { .. } => Ok(()),
            UiEvent::Wheel { name, delta } => {
                let n = ui.scroll.entry(name).or_default();
                *n = (*n as i32 - delta.signum() as i32).max(0) as usize;
                Ok(())
            }
            _ => continue,
        };
        if let Err(error) = result {
            hq.dialog = Some(Dialog::Message(error));
        }
        hq.revision += 1;
    }
}
fn patch_image(w: &mut Widget, name: &str, path: &str) {
    if w.name() == name {
        w.fields.insert("_Filename".into(), vec![path.into()]);
    }
    if let Some(inner) = &mut w.wrapped {
        patch_image(inner, name, path);
    }
    for child in &mut w.children {
        patch_image(child, name, path);
    }
}
fn team_cell(assets: &LegacyAssets, team: &Team) -> Option<Arc<Widget>> {
    let mut cell = (*assets.widget("teamhq/team_cell.gib").ok()?).clone();
    editor::panels::text(&mut cell, "team name textbox", &team.name());
    editor::panels::text(&mut cell, "team motto textbox", &team.motto());
    patch_image(&mut cell, "logo img", &team.logo());
    Some(Arc::new(cell))
}
fn robot_cells(hq: &Headquarters, id: usize, assets: &mut LegacyAssets) -> Vec<Cell> {
    let team = &hq.teams.records[id];
    let mut cells = Vec::new();
    for slot in [0, 3, 1, 4, 2, 5] {
        let robot = team
            .robots()
            .contains(&slot)
            .then(|| {
                std::fs::read(robot_path(&hq.root, id, slot))
                    .ok()
                    .and_then(|b| ra2_formats::bot::decode(&b).ok())
            })
            .flatten();
        let file = if robot.is_some() {
            "bot_cell"
        } else {
            "nobot_cell"
        };
        let mut cell = assets
            .widget(&format!("teamhq/{file}.gib"))
            .ok()
            .map(|w| (*w).clone());
        if let Some(w) = &mut cell {
            let mut image = "teamhq/nobot.bmp".to_string();
            if let Some(robot) = &robot {
                editor::panels::text(w, "bot name", &robot.name);
                editor::panels::text(
                    w,
                    "weight class",
                    match robot.weight_class {
                        0 => "Lightweight",
                        1 => "Middleweight",
                        _ => "Heavyweight",
                    },
                );
                if let Some(snapshot) = &robot.snapshot
                    && let Ok(pixels) = decode_image(snapshot, "tga")
                {
                    image = format!("hq/bot-{id}-{slot}.tga");
                    assets.images.insert(image.clone(), pixels);
                }
            }
            patch_image(w, "bot snapshot img", &image);
        }
        cells.push(Cell {
            key: format!("robot:{slot}"),
            selected: hq.slot_selected && hq.slot == slot,
            template: cell.map(Arc::new),
            ..default()
        });
    }
    cells
}
#[allow(clippy::too_many_arguments)]
fn sync(
    mut hq: ResMut<Headquarters>,
    nav: Res<Navigation>,
    mut ui: ResMut<legacy_ui::State>,
    mut assets: ResMut<LegacyAssets>,
    mut last: Local<Option<(Page, u64)>>,
    data: Option<Res<data::NativeData>>,
) {
    let page = if nav.page == Page::Settings {
        nav.back
    } else {
        nav.page
    };
    if !matches!(page, Page::TeamSelect | Page::TeamHQ) {
        *last = None;
        return;
    }
    if *last == Some((page, hq.revision)) {
        return;
    }
    if last.is_none()
        && let Ok(bytes) = std::fs::read(hq.root.join("Teams/Teams.txt"))
        && let Ok(db) = Teams::decode(&bytes)
    {
        hq.teams = db;
    }
    *last = Some((page, hq.revision));
    ui.root = assets
        .widget(if page == Page::TeamSelect {
            "teamhq/select_team.gib"
        } else {
            "teamhq/teamhq.gib"
        })
        .ok();
    ui.slots.clear();
    ui.dialogs.clear();
    ui.cells.clear();
    ui.lines.clear();
    ui.viewports.clear();
    ui.attachments.clear();
    ui.checked.clear();
    ui.disabled.clear();
    ui.read_only.clear();
    ui.interactive.clear();
    ui.images.clear();
    ui.colors.clear();
    ui.choices.clear();
    ui.visibility.clear();
    ui.values.retain(|k, _| {
        [
            "name input textbox",
            "team motto input textbox",
            "file name",
            "import path",
        ]
        .contains(&k.as_str())
    });
    if page == Page::TeamSelect {
        ui.cells.insert(
            "select team grid".into(),
            hq.teams
                .active
                .iter()
                .map(|&id| Cell {
                    key: format!("team:{id}"),
                    selected: hq.selected == Some(id),
                    template: team_cell(&assets, &hq.teams.records[id]),
                    ..default()
                })
                .collect(),
        );
        if hq.selected.is_none() {
            ui.disabled
                .extend(["continue button".into(), "delete team button".into()]);
        }
        if hq.teams.active.len() >= 16 {
            ui.disabled.insert("new team button".into());
        }
    } else if let Ok(id) = hq.team() {
        let team = &hq.teams.records[id];
        ui.values.insert("hq team name textbox".into(), team.name());
        ui.values.insert("hq motto textbox".into(), team.motto());
        ui.images
            .insert("hq logo img".into(), normalize(&team.logo()));
        let panel = ["hq_robots", "hq_league", "hq_events", "hq_history"][hq.tab];
        if let Ok(w) = assets.widget(&format!("teamhq/{panel}.gib")) {
            ui.slots.insert("hq content group".into(), w);
        }
        for (i, name) in [
            "robots button",
            "league button",
            "events button",
            "history button",
        ]
        .iter()
        .enumerate()
        {
            ui.checked.insert((*name).into(), i == hq.tab);
        }
        if hq.tab == 0 {
            ui.cells
                .insert("robot grid".into(), robot_cells(&hq, id, &mut assets));
            if !hq.slot_selected {
                ui.disabled
                    .extend(["workshop button".into(), "import button".into()]);
            }
            if !hq.slot_selected || !team.robots().contains(&hq.slot) {
                ui.disabled
                    .extend(["export button".into(), "scrap button".into()]);
            }
        }
        data::bind(data.as_deref(), &hq, team, &mut assets, &mut ui);
    }
    if let Some(dialog) = &hq.dialog {
        let file = match dialog {
            Dialog::Team(_) => "teamhq/editteam.gib",
            Dialog::Import => "botlab/importbot.gib",
            Dialog::Export => "botlab/exportbot.gib",
            Dialog::Help => "help/helpframe.gib",
            Dialog::Event(_) => "teamhq/joinevent.gib",
            _ => "common/messagebox.gib",
        };
        if let Ok(mut w) = assets.widget(file) {
            if matches!(dialog, Dialog::Team(_)) {
                // Teams.txt stores each motto on one physical line. Keep visual wrapping.
                editor::panels::patch(
                    std::sync::Arc::make_mut(&mut w),
                    "team motto input textbox",
                    "AllowNewlines",
                    "false",
                );
            } else if matches!(dialog, Dialog::Help) {
                ui.read_only.insert("New [TextBox]".into());
            }
            ui.dialogs.push(w);
        }
        match dialog {
            Dialog::Team(edit) => {
                ui.values.insert(
                    "window caption".into(),
                    if edit.is_some() {
                        "Edit Robotics Team"
                    } else {
                        "New Robotics Team"
                    }
                    .into(),
                );
                ui.images
                    .insert("logo image preview".into(), normalize(&hq.logo));
                ui.cells.insert(
                    "logo listbox".into(),
                    assets
                        .images
                        .keys()
                        .filter(|k| k.starts_with("teams/"))
                        .map(|key| Cell {
                            key: format!("logo:{key}"),
                            label: Path::new(key).file_stem().unwrap().to_string_lossy().into(),
                            selected: normalize(&hq.logo) == *key,
                            ..default()
                        })
                        .collect(),
                );
            }
            Dialog::Import => {
                let mut files = vec![];
                let _ = crate::content::scan(&hq.root.join("Robot Designs"), &mut files);
                let cells: Vec<_> = files
                    .iter()
                    .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("bot")))
                    .map(|p| {
                        let name = p.file_stem().unwrap().to_string_lossy();
                        Cell {
                            key: format!("design:{name}"),
                            label: name.into(),
                            ..default()
                        }
                    })
                    .collect();
                ui.cells.insert("file list".into(), cells.clone());
                ui.cells.insert("robot listbox".into(), cells);
            }
            Dialog::Export => (),
            Dialog::Help => {
                ui.values
                    .insert("caption bar".into(), "Team HQ Help".into());
                let path = if page == Page::TeamSelect {
                    "help/help_selectteam.gib"
                } else {
                    "help/help_teamhq.gib"
                };
                if let Ok(w) = assets.widget(path) {
                    ui.slots.insert("help content group".into(), w);
                }
            }
            Dialog::Event(event) => {
                if let Some(event) = data.as_ref().and_then(|d| d.events.get(*event)) {
                    ui.values.insert("event name".into(), event.name.clone());
                    ui.values
                        .insert("description".into(), event.description.clone());
                    ui.images
                        .insert("preview img".into(), event.preview.clone());
                }
                if let Ok(id) = hq.team() {
                    let name =
                        if hq.slot_selected && hq.teams.records[id].robots().contains(&hq.slot) {
                            std::fs::read(robot_path(&hq.root, id, hq.slot))
                                .ok()
                                .and_then(|b| ra2_formats::bot::decode(&b).ok())
                                .map(|r| r.name)
                                .unwrap_or_default()
                        } else {
                            String::new()
                        };
                    ui.values.insert("robot name".into(), name);
                    ui.cells
                        .insert("robot grid".into(), robot_cells(&hq, id, &mut assets));
                    if !hq.slot_selected || !hq.teams.records[id].robots().contains(&hq.slot) {
                        ui.disabled.insert("enter event button".into());
                    }
                }
            }
            _ => {
                let (caption, text, confirm) = match dialog {
                    Dialog::DeleteTeam => {
                        ("Delete Team?", "Delete the selected robotics team?", true)
                    }
                    Dialog::Scrap => (
                        "Scrap Robot?",
                        "Scrap the robot in the selected inventory slot?",
                        true,
                    ),
                    Dialog::Message(message) => ("Team HQ", message.as_str(), false),
                    _ => unreachable!(),
                };
                ui.values.insert("caption bar".into(), caption.into());
                ui.values.insert("text box".into(), text.into());
                for (name, show) in [
                    ("yes button", confirm),
                    ("no button", confirm),
                    ("ok button", !confirm),
                    ("cancel button", false),
                ] {
                    ui.visibility.insert(name.into(), show);
                }
            }
        }
    }
    if page == Page::TeamHQ && hq.tab == 0 {
        let frame = legacy_ui::layout::compose(&assets, &ui, 800, 600);
        if let Some(rect) = frame.regions.get("robot grid")
            && let Some(panel) = ui.slots.get_mut("hq content group")
        {
            let panel = Arc::make_mut(panel);
            editor::panels::patch(
                panel,
                "robot grid",
                "RowHeight",
                &(rect.height() / 3.).to_string(),
            );
            let width = rect.width() / 2.;
            editor::panels::patch(
                panel,
                "robot grid",
                "ColWidths",
                &format!("{width} 17 {width} 17"),
            );
        }
    }
    ui.revision += 1;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires the original UI and BOT assets"]
    fn original_hq_edits_native_teams_and_robot_slots_without_losing_saved_state() {
        let original = Path::new(env!("CARGO_MANIFEST_DIR")).join("../RobotArena2");
        let root = std::env::temp_dir().join(format!("ra2-hq-crud-{}", std::process::id()));
        std::fs::create_dir_all(root.join("Teams")).unwrap();
        let source_bytes = std::fs::read(original.join("Teams/Teams.txt")).unwrap();
        let mut db = Teams::decode(&source_bytes).unwrap();
        assert_eq!(
            db.encode(),
            source_bytes,
            "Loading a native save changed its bytes"
        );
        let opaque = db.records[0].lines[4..].to_vec();
        db.records[0].set_robot(0, true).unwrap();
        db.records[0].set_robot(1, false).unwrap();
        db.records[0].set_robot(2, false).unwrap();
        write_atomic(&root.join("Teams/Teams.txt"), &db.encode()).unwrap();
        let bot = std::fs::read(original.join("Teams/Team0/Bot0.bot")).unwrap();
        write_atomic(&robot_path(&root, 0, 0), &bot).unwrap();
        let mut assets = LegacyAssets::load(&original).unwrap();
        let mut ui = legacy_ui::State::default();
        let mut editor = Editor::default();
        let settings = EngineSettings::default();
        let mut draft = Draft(settings.clone());
        let mut nav = Navigation {
            page: Page::TeamSelect,
            back: Page::Main,
        };
        let mut hq = Headquarters {
            root: root.clone(),
            teams: db,
            selected: Some(0),
            slot: 0,
            slot_selected: false,
            tab: 0,
            dialog: None,
            logo: String::new(),
            revision: 0,
        };
        let activate = |hq: &mut Headquarters,
                        name,
                        ui: &mut legacy_ui::State,
                        editor: &mut Editor,
                        nav: &mut Navigation,
                        draft: &mut Draft| {
            hq.activate(name, "", ui, editor, nav, draft, &settings)
                .unwrap()
        };
        activate(
            &mut hq,
            "continue button",
            &mut ui,
            &mut editor,
            &mut nav,
            &mut draft,
        );
        assert_eq!(nav.page, Page::TeamHQ);
        activate(
            &mut hq,
            "robot:1",
            &mut ui,
            &mut editor,
            &mut nav,
            &mut draft,
        );
        activate(
            &mut hq,
            "workshop button",
            &mut ui,
            &mut editor,
            &mut nav,
            &mut draft,
        );
        assert_eq!(editor.team_slot, Some((0, 1)));
        assert!(
            editor.document.robot.chassis.model.is_none(),
            "An empty slot retained the previous robot"
        );
        editor
            .document
            .load(ra2_formats::bot::decode(&bot).unwrap());
        editor
            .document
            .edit("Rename test robot", |r| {
                r.name = "HQ native slot test".into();
                Ok(())
            })
            .unwrap();
        editor::controls::save_team_slot(&mut editor).unwrap();
        hq.teams = Teams::decode(&std::fs::read(root.join("Teams/Teams.txt")).unwrap()).unwrap();
        nav.page = Page::TeamHQ;
        assert!(hq.teams.records[0].robots().contains(&1));
        assert_eq!(
            ra2_formats::bot::decode(&std::fs::read(robot_path(&root, 0, 1)).unwrap())
                .unwrap()
                .name,
            "HQ native slot test"
        );
        assert_eq!(std::fs::read(robot_path(&root, 0, 0)).unwrap(), bot);
        activate(
            &mut hq,
            "edit team button",
            &mut ui,
            &mut editor,
            &mut nav,
            &mut draft,
        );
        ui.values
            .insert("name input textbox".into(), "Renamed team".into());
        ui.values.insert(
            "team motto input textbox".into(),
            "Saved without losing the season".into(),
        );
        activate(
            &mut hq,
            "ok button",
            &mut ui,
            &mut editor,
            &mut nav,
            &mut draft,
        );
        assert_eq!(hq.teams.records[0].name(), "Renamed team");
        assert_eq!(hq.teams.records[0].lines[4..], opaque);
        activate(
            &mut hq,
            "export button",
            &mut ui,
            &mut editor,
            &mut nav,
            &mut draft,
        );
        ui.values.insert("file name".into(), "exported-test".into());
        activate(
            &mut hq,
            "ok button",
            &mut ui,
            &mut editor,
            &mut nav,
            &mut draft,
        );
        assert!(root.join("Robot Designs/exported-test.bot").is_file());
        activate(
            &mut hq,
            "robot:2",
            &mut ui,
            &mut editor,
            &mut nav,
            &mut draft,
        );
        activate(
            &mut hq,
            "import button",
            &mut ui,
            &mut editor,
            &mut nav,
            &mut draft,
        );
        ui.values
            .insert("import path".into(), "exported-test".into());
        activate(
            &mut hq,
            "ok button",
            &mut ui,
            &mut editor,
            &mut nav,
            &mut draft,
        );
        assert!(hq.teams.records[0].robots().contains(&2));
        activate(
            &mut hq,
            "scrap button",
            &mut ui,
            &mut editor,
            &mut nav,
            &mut draft,
        );
        activate(
            &mut hq,
            "no button",
            &mut ui,
            &mut editor,
            &mut nav,
            &mut draft,
        );
        assert!(hq.teams.records[0].robots().contains(&2));
        activate(
            &mut hq,
            "scrap button",
            &mut ui,
            &mut editor,
            &mut nav,
            &mut draft,
        );
        activate(
            &mut hq,
            "yes button",
            &mut ui,
            &mut editor,
            &mut nav,
            &mut draft,
        );
        assert!(!hq.teams.records[0].robots().contains(&2));
        let saved = std::fs::read(robot_path(&root, 0, 1)).unwrap();
        hq.slot = 1;
        // Force a Teams.txt write failure after importing the replacement BOT.
        std::fs::create_dir(root.join("Teams/Teams.tmp")).unwrap();
        assert!(hq.import_robot(&bot).is_err());
        assert_eq!(std::fs::read(robot_path(&root, 0, 1)).unwrap(), saved);
        std::fs::remove_dir(root.join("Teams/Teams.tmp")).unwrap();
        std::fs::write(root.join("Teams/Teams.txt"), b"invalid state").unwrap();
        editor.document.robot.name = "Must not replace the saved slot".into();
        assert!(editor::controls::save_team_slot(&mut editor).is_err());
        assert_eq!(
            std::fs::read(robot_path(&root, 0, 1)).unwrap(),
            saved,
            "Failed team write did not restore the robot"
        );
        write_atomic(&root.join("Teams/Teams.txt"), &hq.teams.encode()).unwrap();
        activate(
            &mut hq,
            "robot:0",
            &mut ui,
            &mut editor,
            &mut nav,
            &mut draft,
        );
        // Source GIBs can be composed with every original HQ command and six slots.
        let mut app = App::new();
        app.insert_resource(hq)
            .insert_resource(nav)
            .insert_resource(ui)
            .insert_resource(std::mem::take(&mut assets))
            .add_systems(Update, sync);
        app.update();
        let state = app.world().resource::<legacy_ui::State>();
        let frame =
            legacy_ui::layout::compose(app.world().resource::<LegacyAssets>(), state, 800, 600);
        for name in [
            "workshop button",
            "import button",
            "export button",
            "scrap button",
            "select team button",
            "edit team button",
        ] {
            assert!(
                frame.hits.iter().any(|h| h.name == name),
                "Native HQ command missing: {name}"
            );
        }
        assert_eq!(
            state.cells["robot grid"]
                .iter()
                .map(|c| c.key.as_str())
                .collect::<Vec<_>>(),
            [
                "robot:0", "robot:3", "robot:1", "robot:4", "robot:2", "robot:5"
            ]
        );
        assert_eq!(
            std::fs::read(original.join("Teams/Teams.txt")).unwrap(),
            source_bytes
        );
        std::fs::remove_dir_all(&root).unwrap();
    }
}
