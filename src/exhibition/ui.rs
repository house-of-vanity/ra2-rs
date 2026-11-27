//! Bind original Exhibition/selection/HUD/result GIBs without rebuilding their art.
use crate::battle::*;
use crate::editor::panels::text;
use crate::legacy_ui::widgets::{image, rename, visible, widget};
use crate::legacy_ui::{Cell, State, UiEvent, assets::LegacyAssets};
use crate::{
    frontend::{Navigation, Page},
    legacy_ui,
};
use arena_core::match_rules::Mode;
use bevy::prelude::*;
use std::sync::Arc;
const MODES: [(&str, Mode); 5] = [
    ("deathmatch checkbox", Mode::Deathmatch),
    ("battle royal checkbox", Mode::BattleRoyal),
    ("teams checkbox", Mode::Teams),
    ("tabletop checkbox", Mode::TableTop),
    ("kingofhill checkbox", Mode::KingOfHill),
];
fn open(s: &mut Session, slot: usize) {
    s.picker = Some(slot);
    s.picker_bot = s.selected[slot];
    s.picker_team = s
        .picker_bot
        .and_then(|i| s.library.get(i))
        .map_or_else(String::new, |e| e.team_key.clone());
}
fn set_value(s: &mut Session, state: &mut State, name: &str, value: &str) {
    if name == "team dropdown" {
        if let Some((team, _)) = crate::content::robots::teams(&s.library)
            .into_iter()
            .find(|(_, label)| label == value)
        {
            s.picker_team = team.team_key.clone();
            s.picker_bot = s.library.iter().position(|e| {
                e.team_key == s.picker_team && s.weight.is_none_or(|w| e.robot.weight_class == w)
            });
            state.scroll.remove("robots grid");
        }
    } else if name == "time dropdown"
        && let Some(minutes) = value
            .split_whitespace()
            .next()
            .and_then(|v| v.parse::<u32>().ok())
        && [1, 2, 3, 4, 5, 10, 15, 20].contains(&minutes)
    {
        s.minutes = minutes;
    }
}
#[allow(clippy::too_many_arguments)]
pub(crate) fn input(
    mut s: ResMut<Session>,
    mut events: ResMut<legacy_ui::Events>,
    mut nav: ResMut<Navigation>,
    mut state: ResMut<State>,
    mut sounds: ResMut<crate::ui_sound::Sounds>,
    mut draft: ResMut<crate::frontend::Draft>,
    settings: Res<crate::settings::EngineSettings>,
) {
    if nav.page != Page::Exhibition {
        return;
    }
    let pending = std::mem::take(&mut events.0);
    for event in pending {
        match event {
            UiEvent::Activate { name, owner, .. } => {
                sounds.play("Sounds/button_click.wav");
                if let Some(choice) = name.strip_prefix("choice:")
                    && let Some((dropdown, index)) = choice.rsplit_once(':')
                    && let Some(value) = index
                        .parse::<usize>()
                        .ok()
                        .and_then(|i| state.choices.get(dropdown).and_then(|c| c.get(i)))
                        .cloned()
                {
                    set_value(&mut s, &mut state, dropdown, &value);
                    state.popup = None;
                    s.revision += 1;
                    continue;
                }
                let dropdown = if name == "Down Button" {
                    owner.as_str()
                } else {
                    name.as_str()
                };
                if state.choices.contains_key(dropdown) {
                    state.popup = if state.popup.as_deref() == Some(dropdown) {
                        None
                    } else {
                        Some(dropdown.into())
                    };
                    state.revision += 1;
                    continue;
                }
                state.popup = None;
                if s.exhibition_help {
                    if name == "close button" {
                        s.exhibition_help = false;
                        s.revision += 1;
                    }
                    continue;
                }
                if s.error.is_some() && (name == "ok button" || name == "cancel button") {
                    s.error = None;
                } else if s.picker.is_some() {
                    if name == "cancel button" {
                        s.picker = None;
                    } else if name == "ok button" {
                        if let (Some(slot), Some(bot)) = (s.picker, s.picker_bot) {
                            s.selected[slot] = Some(bot);
                            s.picker = None;
                        }
                    } else if let Some(i) = name
                        .strip_prefix("robot:")
                        .and_then(|p| p.parse::<usize>().ok())
                    {
                        s.picker_bot = Some(i);
                    }
                } else {
                    if let Some((_, mode)) = MODES.iter().find(|(key, _)| *key == name) {
                        s.choose_mode(*mode);
                    } else if let Some(index) = name
                        .strip_prefix("arena:")
                        .and_then(|n| n.parse::<usize>().ok())
                    {
                        if s.arenas
                            .get(index)
                            .is_some_and(|a| a.modes.contains(&s.mode))
                        {
                            s.arena = index;
                        }
                    }
                    match name.as_str() {
                        "gametype button" => s.tab = 0,
                        "arena button" => s.tab = 1,
                        "robots button" => s.tab = 2,
                        "hazard checkbox" => s.hazards = !s.hazards,
                        "all classes cb" => s.weight = None,
                        "lightweight cb" => s.weight = Some(0),
                        "mediumweight cb" => s.weight = Some(1),
                        "heavyweight cb" => s.weight = Some(2),
                        "start game button" => s.start = true,
                        "continue button" => nav.page = Page::Exhibition,
                        "MainMenu button" | "Quit Session button" | "main menu button" => {
                            nav.page = Page::Main
                        }
                        "Help button" => s.exhibition_help = true,
                        "Options button" | "options button" => {
                            nav.back = nav.page;
                            nav.page = Page::Settings;
                            draft.0 = settings.clone();
                        }
                        "skip button" if matches!(s.phase, Phase::Introduction(_)) => {
                            s.begin_countdown()
                        }
                        _ => {
                            let key = if name.starts_with("p") { &name } else { &owner };
                            if let Some(slot) = key.get(1..2).and_then(|n| n.parse::<usize>().ok())
                                && slot < s.mode.slots()
                                && (key.ends_with("change robot button")
                                    || key.ends_with("human player button")
                                    || key.ends_with("computer player button")
                                    || key.ends_with("bot img"))
                            {
                                open(&mut s, slot);
                            } else if let Some(slot) =
                                key.get(1..2).and_then(|n| n.parse::<usize>().ok())
                                && slot > 0
                                && slot < s.mode.slots()
                                && key.ends_with("eject button")
                            {
                                s.selected[slot] = None;
                            }
                        }
                    }
                }
                s.revision += 1;
            }
            UiEvent::Value { name, value } => {
                set_value(&mut s, &mut state, &name, &value);
                s.revision += 1;
            }
            UiEvent::Wheel { name, delta } => {
                let scroll = state.scroll.entry(name).or_default();
                *scroll = (*scroll as i32 - delta.signum() as i32).max(0) as usize;
                s.revision += 1;
            }
            _ => {}
        }
    }
}
#[allow(clippy::too_many_arguments)]
pub(crate) fn bind(
    s: Res<Session>,
    nav: Res<Navigation>,
    mut state: ResMut<State>,
    assets: Res<LegacyAssets>,
    mut last: Local<Option<(Page, u64)>>,
) {
    let page = if nav.page == Page::Settings {
        nav.back
    } else {
        nav.page
    };
    if page != Page::Exhibition {
        *last = None;
        return;
    }
    if *last == Some((page, s.revision)) {
        return;
    }
    if last.as_ref().is_none_or(|(p, _)| *p != page) {
        state.focused.clear();
        state.popup = None;
    }
    *last = Some((page, s.revision));
    state.rebuild();
    if page == Page::Exhibition {
        state.root = assets.widget("exhibition/exhib_setup.gib").ok();
        let file = ["gametype", "arena", "exhib_robots"][s.tab];
        if let Some(mut panel) = widget(&assets, &format!("exhibition/{file}.gib")) {
            if s.tab == 1
                && let Some(arena) = s.arenas.get(s.arena)
            {
                image(&mut panel, "arena preview img", &arena.preview);
                state.values.insert(
                    "description".into(),
                    format!("<text font fg_mcn14>{}</text>", arena.description),
                );
            }
            state.slots.insert("content group".into(), Arc::new(panel));
        }
        for (key, mode) in MODES {
            state.checked.insert(key.into(), s.mode == mode);
        }
        for (key, weight) in [
            ("all classes cb", None),
            ("lightweight cb", Some(0)),
            ("mediumweight cb", Some(1)),
            ("heavyweight cb", Some(2)),
        ] {
            state.checked.insert(key.into(), s.weight == weight);
        }
        state.checked.insert("hazard checkbox".into(), s.hazards);
        for (i, key) in ["gametype button", "arena button", "robots button"]
            .iter()
            .enumerate()
        {
            state.checked.insert((*key).into(), s.tab == i);
        }
        state.choices.insert(
            "time dropdown".into(),
            [1, 2, 3, 4, 5, 10, 15, 20]
                .iter()
                .map(|n| format!("{n} minutes"))
                .collect(),
        );
        state
            .values
            .insert("time dropdown".into(), format!("{} minutes", s.minutes));
        state.cells.insert(
            "arena listbox".into(),
            s.arenas
                .iter()
                .enumerate()
                .filter(|(_, a)| a.modes.contains(&s.mode))
                .map(|(i, a)| Cell {
                    key: format!("arena:{i}"),
                    label: a.name.clone(),
                    selected: s.arena == i,
                    ..default()
                })
                .collect(),
        );
        for slot in 0..s.mode.slots() {
            let selected = s.selected[slot].and_then(|i| s.library.get(i).map(|e| (i, e)));
            if let Some(mut card) = widget(
                &assets,
                if selected.is_some() {
                    "multiplayer/activeplayer.gib"
                } else {
                    "multiplayer/noplayer.gib"
                },
            ) {
                if let Some((i, entry)) = selected {
                    text(&mut card, "team name", &entry.team);
                    text(&mut card, "bot name", &entry.robot.name);
                    image(&mut card, "bot img", &format!("exhibition/bot-{i}.tga"));
                    image(
                        &mut card,
                        "player icon img",
                        if slot == 0 {
                            "multiplayer/local_player_icon.bmp"
                        } else {
                            "multiplayer/computer_player_icon.bmp"
                        },
                    );
                    visible(&mut card, "spectator [TextBox]", false);
                    visible(&mut card, "ready checkbox", false);
                    visible(&mut card, "ready label", false);
                    visible(&mut card, "eject button", slot > 0);
                }
                rename(&mut card, &format!("p{slot}/"));
                state
                    .slots
                    .insert(format!("player{} group", slot + 1), Arc::new(card));
            }
        }
        state
            .visibility
            .insert("team 2 group".into(), s.mode.slots() > 2);
        if !s.ready() {
            state.disabled.insert("start game button".into());
        }
        if let Some(slot) = s.picker
            && let Some(mut dialog) = widget(
                &assets,
                if slot == 0 {
                    "exhibition/selectbot_player.gib"
                } else {
                    "exhibition/selectbot_computer.gib"
                },
            )
        {
            let teams = crate::content::robots::teams(&s.library);
            state.choices.insert(
                "team dropdown".into(),
                teams.iter().map(|(_, label)| label.clone()).collect(),
            );
            if let Some((entry, label)) = teams.iter().find(|(e, _)| e.team_key == s.picker_team) {
                state.values.insert("team dropdown".into(), label.clone());
                state
                    .values
                    .insert("team motto".into(), entry.motto.clone());
                image(&mut dialog, "team logo img", &entry.logo);
            }
            text(&mut dialog, "robot name", "");
            let mut cells = vec![];
            for (i, entry) in s.library.iter().enumerate().filter(|(_, e)| {
                e.team_key == s.picker_team && s.weight.is_none_or(|w| e.robot.weight_class == w)
            }) {
                if let Some(mut cell) = widget(&assets, "exhibition/choosebot_cell.gib") {
                    image(&mut cell, "bot preview", &format!("exhibition/bot-{i}.tga"));
                    image(
                        &mut cell,
                        "weight icon",
                        match entry.robot.weight_class {
                            0 => "exhibition/light_icon.bmp",
                            1 => "exhibition/medium_icon.bmp",
                            _ => "exhibition/heavy_icon.bmp",
                        },
                    );
                    cells.push(Cell {
                        key: format!("robot:{i}"),
                        template: Some(Arc::new(cell)),
                        selected: s.picker_bot == Some(i),
                        ..default()
                    });
                }
            }
            state.cells.insert("robots grid".into(), cells);
            if let Some(entry) = s.picker_bot.and_then(|i| s.library.get(i)) {
                text(&mut dialog, "robot name", &entry.robot.name);
                state
                    .values
                    .insert("team motto".into(), entry.motto.clone());
                image(&mut dialog, "team logo img", &entry.logo);
            }
            if s.picker_bot.is_none() {
                state.disabled.insert("ok button".into());
            }
            state.dialogs.push(Arc::new(dialog));
        }
    }
    if s.exhibition_help {
        crate::legacy_ui::widgets::help(
            &mut state,
            &assets,
            "Exhibition Help",
            "help/help_exhibition.gib",
        );
    }
    if let Some(error) = &s.error
        && let Some(mut dialog) = widget(&assets, "common/messagebox.gib")
    {
        text(&mut dialog, "text box", error);
        text(&mut dialog, "caption bar", "Exhibition");
        for name in ["yes button", "no button", "cancel button"] {
            visible(&mut dialog, name, false);
        }
        visible(&mut dialog, "ok button", true);
        state.dialogs.push(Arc::new(dialog));
    }
}
