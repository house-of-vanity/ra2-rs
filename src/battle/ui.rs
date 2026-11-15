//! Shared native in-match HUD and results. Entry menus remain in their own modules.
use super::*;
use crate::{
    editor::panels::{patch, text},
    legacy_ui::{State, assets::LegacyAssets, widgets::*},
};
use std::sync::Arc;

#[allow(clippy::too_many_arguments)]
pub(crate) fn input(
    mut s: ResMut<Session>,
    mut events: ResMut<legacy_ui::Events>,
    mut nav: ResMut<Navigation>,
    mut sounds: ResMut<crate::ui_sound::Sounds>,
    mut draft: ResMut<crate::frontend::Draft>,
    settings: Res<crate::settings::EngineSettings>,
    mut network: ResMut<crate::multiplayer::Multiplayer>,
) {
    if !matches!(nav.page, Page::Match | Page::Results) {
        return;
    }
    for event in std::mem::take(&mut events.0) {
        if let crate::legacy_ui::UiEvent::Activate { name, .. } = event {
            sounds.play("Sounds/button_click.wav");
            match name.as_str() {
                "continue button" => {
                    if s.network {
                        network.return_lobby();
                    } else {
                        nav.page = Page::Exhibition;
                    }
                }
                "Quit Session button" | "main menu button" => nav.page = Page::Main,
                "options button" => {
                    nav.back = nav.page;
                    nav.page = Page::Settings;
                    draft.0 = settings.clone();
                }
                "skip button" if !s.network && matches!(s.phase, Phase::Introduction(_)) => {
                    s.begin_countdown()
                }
                "ok button" | "cancel button" => {
                    s.error = None;
                    network.error = None;
                }
                _ => {}
            }
        }
    }
}
#[allow(clippy::too_many_arguments)]
pub(crate) fn bind(
    s: Res<Session>,
    nav: Res<Navigation>,
    mut state: ResMut<State>,
    assets: Res<LegacyAssets>,
    poses: Query<&Transform, With<RobotBody>>,
    mut last: Local<Option<(Page, u64)>>,
    network: Res<crate::multiplayer::Multiplayer>,
) {
    let page = if nav.page == Page::Settings {
        nav.back
    } else {
        nav.page
    };
    if !matches!(page, Page::Match | Page::Results) {
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
    *state = State {
        revision: state.revision + 1,
        scroll: state.scroll.clone(),
        focused: state.focused.clone(),
        popup: state.popup.clone(),
        ..default()
    };
    state.root = assets.widget("common/gamewindow.gib").ok();
    for key in ["chat group", "chat displaybox"] {
        state.visibility.insert(key.into(), false);
    }
    state.visibility.insert(
        "skip group".into(),
        matches!(s.phase, Phase::Introduction(_)),
    );
    state.visibility.insert(
        "countdown group".into(),
        matches!(s.phase, Phase::Countdown(_)),
    );
    state.visibility.insert(
        "time group".into(),
        matches!(s.phase, Phase::Playing | Phase::Finished(_)),
    );
    state.values.insert(
        "count number".into(),
        if let Phase::Countdown(t) = s.phase {
            format!("{}", t.ceil() as u32)
        } else {
            String::new()
        },
    );
    let seconds = s.remaining.ceil() as u32;
    state.values.insert(
        "time".into(),
        format!("{}:{:02}", seconds / 60, seconds % 60),
    );
    state.values.insert(
        "camera textbox".into(),
        if matches!(s.phase, Phase::Introduction(_)) {
            "Arena Introduction"
        } else {
            s.follow.mode.label()
        }
        .into(),
    );
    for slot in 0..4 {
        state.visibility.insert(
            format!("player{} status", slot + 1),
            s.robots.iter().any(|r| r.id as usize == slot + 1),
        );
    }
    for robot in &s.robots {
        let i = robot.id as usize - 1;
        if let Some(mut card) = widget(&assets, "common/statusmeters.gib") {
            text(
                &mut card,
                "name",
                robot.document.as_ref().map_or("", |r| r.name.as_str()),
            );
            text(&mut card, "points", &format!("{:.0}", s.score[i]));
            for (key, fraction, width) in [
                (
                    "battery bar",
                    robot.rig.as_ref().map_or(0., |r| r.electrical.fraction()),
                    52.,
                ),
                (
                    "air bar",
                    robot.rig.as_ref().map_or(0., |r| r.pneumatic.fraction()),
                    52.,
                ),
                (
                    "armor bar",
                    robot
                        .damage
                        .health
                        .parts
                        .first()
                        .map_or(0., |p| p.fraction()),
                    26.,
                ),
            ] {
                patch(
                    &mut card,
                    key,
                    "Size",
                    &format!("{} 0 4 0", width * fraction),
                );
            }
            rename(&mut card, &format!("status{i}/"));
            state
                .slots
                .insert(format!("player{} status", i + 1), Arc::new(card));
        }
    }
    if s.network && !s.authority {
        state.disabled.insert("continue button".into());
    }
    if page == Page::Results
        && let Some(mut results) = widget(&assets, "common/matchresults.gib")
    {
        for i in 0..4 {
            if !s.robots.iter().any(|r| r.id as usize == i + 1) {
                visible(&mut results, &format!("result slot {}", i + 1), false);
                continue;
            }
            if let Some(mut cell) = widget(&assets, "common/result_cell.gib") {
                if let Some(entry) = s.selected[i].and_then(|n| s.library.get(n)) {
                    text(&mut cell, "bot name", &entry.robot.name);
                    text(&mut cell, "team name", &entry.team);
                    image(&mut cell, "team logo", &entry.logo);
                    image(
                        &mut cell,
                        "bot preview",
                        &format!("exhibition/bot-{}.tga", s.selected[i].unwrap()),
                    );
                }
                text(&mut cell, "points", &format!("{:.0}", s.score[i]));
                visible(&mut cell, "winner text", s.winners[i]);
                let dead = s
                    .robots
                    .iter()
                    .find(|r| r.id as usize == i + 1)
                    .and_then(|r| {
                        r.bodies
                            .first()
                            .and_then(|e| poses.get(*e).ok())
                            .map(|p| defeated(r, p.translation))
                    })
                    .unwrap_or(true);
                visible(&mut cell, "immobilized text", dead || s.eliminated[i]);
                visible(&mut cell, "Spectator [TextBox]", false);
                rename(&mut cell, &format!("result{i}/"));
                state
                    .slots
                    .insert(format!("result slot {}", i + 1), Arc::new(cell));
            }
        }
        state.dialogs.push(Arc::new(results));
    }
    if let Some(error) = s.error.as_ref().or(if s.network {
        network.error.as_ref()
    } else {
        None
    }) && let Some(mut dialog) = widget(&assets, "common/messagebox.gib")
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
