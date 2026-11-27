//! Original Multiplayer / IP Connect / Create / Prematch GIBs and native robot picker.
use super::*;
use crate::{
    editor::panels::{patch, text},
    legacy_ui::{Cell, State, UiEvent, assets::LegacyAssets, widgets::*},
};
use arena_core::match_rules::Mode;
use bevy::clipboard::Clipboard;
use ra2_formats::gib::Widget;
const MODES: [(&str, Mode); 5] = [
    ("deathmatch checkbox", Mode::Deathmatch),
    ("battle royal checkbox", Mode::BattleRoyal),
    ("teams checkbox", Mode::Teams),
    ("tabletop checkbox", Mode::TableTop),
    ("kingofhill checkbox", Mode::KingOfHill),
];
pub(super) fn mode(n: u8) -> Mode {
    MODES.get(n as usize).map_or(Mode::Deathmatch, |(_, m)| *m)
}
fn show_error(n: &mut Multiplayer, r: Result<(), String>) {
    if let Err(e) = r {
        n.error = Some(e);
    }
}
#[allow(clippy::too_many_arguments)]
pub(super) fn input(
    mut net: ResMut<Multiplayer>,
    mut battle: ResMut<battle::Session>,
    mut events: ResMut<legacy_ui::Events>,
    mut state: ResMut<State>,
    mut nav: ResMut<Navigation>,
    mut sounds: ResMut<crate::ui_sound::Sounds>,
    mut draft: ResMut<crate::frontend::Draft>,
    settings: Res<crate::settings::EngineSettings>,
    mut clipboard: ResMut<Clipboard>,
    time: Res<Time<Real>>,
) {
    if nav.page != Page::Multiplayer {
        return;
    }
    for event in std::mem::take(&mut events.0) {
        let event = match event {
            UiEvent::Submit(field) if field == "chat textbox" && net.screen == Screen::Lobby => {
                let message = std::mem::take(&mut net.chat_input);
                if !message.is_empty() {
                    net.send(Request::Chat(message));
                }
                state.focused = field;
                net.revision += 1;
                continue;
            }
            UiEvent::Submit(field) if field == "IP address" && net.ip_dialog => UiEvent::Activate {
                name: "connect button".into(),
                owner: String::new(),
                point: Vec2::ZERO,
            },
            other => other,
        };
        match event {
            UiEvent::Activate { name, owner, .. } => {
                sounds.play("Sounds/button_click.wav");
                if let Some(choice) = name.strip_prefix("choice:")
                    && let Some((dropdown, index)) = choice.rsplit_once(':')
                    && let Some(choice_value) = index
                        .parse::<usize>()
                        .ok()
                        .and_then(|i| state.choices.get(dropdown).and_then(|v| v.get(i)))
                        .cloned()
                {
                    value(&mut net, &battle, &mut state, dropdown, &choice_value);
                    state.popup = None;
                    net.revision += 1;
                    continue;
                }
                if state.choices.contains_key(&name) {
                    state.popup = if state.popup.as_deref() == Some(&name) {
                        None
                    } else {
                        Some(name)
                    };
                    state.revision += 1;
                    continue;
                }
                if net.error.is_some() {
                    if ["ok button", "cancel button"].contains(&name.as_str()) {
                        net.error = None;
                    }
                    net.revision += 1;
                    continue;
                }
                if net.help {
                    if name == "close button" {
                        net.help = false;
                        net.revision += 1;
                    }
                    continue;
                }
                if net.ip_dialog {
                    match name.as_str() {
                        "cancel button" => net.ip_dialog = false,
                        "connect button" => {
                            let result = (|| {
                                let m = net
                                    .manifest
                                    .clone()
                                    .ok_or("Compatibility manifest unavailable")?;
                                let cfg = net.settings.clone();
                                let (command, status) = if net.join_iroh {
                                    let target: JoinTarget = net.join_input.parse()?;
                                    let status = match target {
                                        JoinTarget::Code(code) => {
                                            format!("Finding room {code} on Mainline...")
                                        }
                                        JoinTarget::Endpoint(_) => {
                                            "Connecting to EndpointID...".into()
                                        }
                                    };
                                    (service::Command::JoinIroh(cfg, m, target), status)
                                } else {
                                    let addr = service::address(&net.ip)?;
                                    (
                                        service::Command::Connect(cfg, m, addr),
                                        format!("Connecting to {addr}..."),
                                    )
                                };
                                net.service
                                    .as_ref()
                                    .ok_or("Network worker unavailable")?
                                    .send(command)?;
                                net.host = false;
                                net.ip_dialog = false;
                                net.screen = Screen::Lobby;
                                net.status = status;
                                net.selected = net.selected.or(Some(0));
                                Ok(())
                            })();
                            show_error(&mut net, result);
                        }
                        _ => {}
                    }
                } else if net.picker {
                    match name.as_str() {
                        "cancel button" => net.picker = false,
                        "ok button" => {
                            if let Some(i) = net.picker_bot {
                                let result = net.choose(&battle, i);
                                show_error(&mut net, result);
                                net.picker = false;
                            }
                        }
                        _ => {
                            if let Some(i) = name
                                .strip_prefix("robot:")
                                .and_then(|n| n.parse::<usize>().ok())
                            {
                                net.picker_bot = Some(i);
                            }
                        }
                    }
                } else {
                    if let Some(addr) = name
                        .strip_prefix("server:")
                        .and_then(|s| s.parse::<SocketAddr>().ok())
                        && net.server_list.contains_key(&addr)
                    {
                        net.selected_server = Some(addr);
                    }
                    if let Some(index) = name
                        .strip_prefix("arena:")
                        .and_then(|n| n.parse::<usize>().ok())
                        && let Some(a) = battle.arenas.get(index)
                        && a.modes.contains(&mode(net.settings.mode))
                    {
                        net.settings.arena = a.key.clone();
                    }
                    if let Some(i) = MODES.iter().position(|(key, _)| *key == name) {
                        net.settings.mode = i as u8;
                        if battle
                            .arenas
                            .iter()
                            .find(|a| a.key == net.settings.arena)
                            .is_none_or(|a| !a.modes.contains(&mode(i as u8)))
                            && let Some(a) = battle
                                .arenas
                                .iter()
                                .find(|a| a.modes.contains(&mode(i as u8)))
                        {
                            net.settings.arena = a.key.clone();
                        }
                    }
                    match name.as_str() {
                        "IROH button" | "Internet radio but" | "Internet Label"
                            if net.screen == Screen::Browse =>
                        {
                            net.join_iroh = true;
                            net.ip_dialog = true;
                            state.focused = "IP address".into();
                        }
                        "internet radio but" | "internet label"
                            if net.screen == Screen::Create && net.players.is_empty() =>
                        {
                            net.invite.get_or_insert_with(HostInvite::generate);
                        }
                        "LAN radio but" | "lan label"
                            if net.screen == Screen::Create && net.players.is_empty() =>
                        {
                            net.invite = None;
                            net.copied_endpoint = false;
                        }
                        "Help button" => net.help = true,
                        "copy endpoint" => {
                            if let Some(invite) = &net.invite {
                                match clipboard.set_text(invite.endpoint_id().to_string()) {
                                    Ok(()) => net.copied_endpoint = true,
                                    Err(e) => {
                                        net.error = Some(format!("Could not copy EndpointID: {e}"))
                                    }
                                }
                            }
                        }
                        "Refresh button" | "LAN radio but" if net.screen == Screen::Browse => {
                            net.refresh_servers(time.elapsed());
                        }
                        "Create Game button" => {
                            net.screen = Screen::Create;
                            net.tab = 0;
                            net.host = true;
                            net.invite = None;
                            net.publication = None;
                            net.copied_endpoint = false;
                        }
                        "IP connect button" => {
                            net.join_iroh = false;
                            net.ip_dialog = true;
                            state.focused = "IP address".into();
                        }
                        "Join game button" => {
                            if let Some(addr) = net.selected_server {
                                net.ip = addr.to_string();
                                net.join_iroh = false;
                                net.ip_dialog = true;
                            } else {
                                net.error = Some(
                                    "Select a LAN game, use IP Connect, or join by code".into(),
                                );
                            }
                        }
                        "mp settings button" => net.tab = 0,
                        "gametype button" => net.tab = 1,
                        "arena button" => net.tab = 2,
                        "hazard checkbox" => net.settings.hazards = !net.settings.hazards,
                        "all classes cb" => net.settings.weight = None,
                        "lightweight cb" => net.settings.weight = Some(0),
                        "mediumweight cb" => net.settings.weight = Some(1),
                        "heavyweight cb" => net.settings.weight = Some(2),
                        "start game button" => {
                            let result = net.start_host(&battle);
                            show_error(&mut net, result);
                        }
                        "cancel game button" => {
                            net.screen = Screen::Browse;
                            net.invite = None;
                            net.copied_endpoint = false;
                        }
                        "disconnect button" => {
                            net.disconnect();
                            battle.network = false;
                        }
                        "begin match button" => net.send(Request::Start),
                        "accept button" => {
                            let settings = net.settings.clone();
                            net.pending_ready = None;
                            net.send(Request::Settings(settings));
                            net.screen = Screen::Lobby;
                        }
                        "cancel button" => net.screen = Screen::Lobby,
                        "settings button" if net.host => {
                            net.screen = Screen::Create;
                            net.tab = 1;
                        }
                        "MainMenu button" | "main menu button" | "Quit Session button" => {
                            nav.page = Page::Main
                        }
                        "Options button" | "options button" => {
                            nav.back = nav.page;
                            nav.page = Page::Settings;
                            draft.0 = settings.clone();
                        }
                        _ => {
                            let key = if name.starts_with('p') { &name } else { &owner };
                            if let Some(slot) = key.get(1..2).and_then(|n| n.parse::<usize>().ok())
                                && slot == net.local
                            {
                                if key.ends_with("ready checkbox") {
                                    net.toggle_ready();
                                } else if key.ends_with("change robot button")
                                    || key.ends_with("bot img")
                                {
                                    net.picker = true;
                                    net.picker_bot = net.selected;
                                    net.picker_team = net
                                        .selected
                                        .and_then(|i| battle.library.get(i))
                                        .map_or_else(String::new, |e| e.team_key.clone());
                                }
                            }
                        }
                    }
                }
                net.revision += 1;
            }
            UiEvent::Value { name, value: v } => {
                value(&mut net, &battle, &mut state, &name, &v);
                net.revision += 1;
            }
            UiEvent::Wheel { name, delta } => {
                let scroll = state.scroll.entry(name).or_default();
                *scroll = (*scroll as i32 - delta.signum() as i32).max(0) as usize;
                net.revision += 1;
            }
            _ => {}
        }
    }
}
fn value(n: &mut Multiplayer, b: &battle::Session, state: &mut State, key: &str, v: &str) {
    match key {
        "IP address" if n.join_iroh => n.join_input = v.chars().take(128).collect(),
        "IP address" => n.ip = v.chars().take(128).collect(),
        "server name textbox" => {
            n.settings.name = v[..v.floor_char_boundary(96.min(v.len()))].into()
        }
        "screen name" => n.name = v[..v.floor_char_boundary(64.min(v.len()))].into(),
        "chat textbox" => n.chat_input = v[..v.floor_char_boundary(256.min(v.len()))].into(),
        "team dropdown" => {
            if let Some((team, _)) = crate::content::robots::teams(&b.library)
                .into_iter()
                .find(|(_, label)| label == v)
            {
                n.picker_team = team.team_key.clone();
                n.picker_bot = b.library.iter().position(|e| {
                    e.team_key == n.picker_team
                        && n.settings
                            .weight
                            .is_none_or(|w| e.robot.weight_class == w as u32)
                });
                state.scroll.remove("robots grid");
            }
        }
        "time dropdown" => {
            if let Some(m) = v
                .split_whitespace()
                .next()
                .and_then(|v| v.parse::<u16>().ok())
                && (1..=20).contains(&m)
            {
                n.settings.minutes = m;
            }
        }
        _ => {}
    }
}
#[allow(clippy::too_many_arguments)]
pub(super) fn bind(
    net: Res<Multiplayer>,
    battle: Res<battle::Session>,
    nav: Res<Navigation>,
    assets: Res<LegacyAssets>,
    mut state: ResMut<State>,
    time: Res<Time<Real>>,
    mut last: Local<Option<(u64, usize)>>,
) {
    if nav.page != Page::Multiplayer
        && !(nav.page == Page::Settings && nav.back == Page::Multiplayer)
    {
        *last = None;
        return;
    }
    let dots = if net.screen == Screen::Browse {
        net.search.dots(time.elapsed())
    } else {
        0
    };
    let version = (net.revision, dots);
    if *last == Some(version) {
        return;
    }
    *last = Some(version);
    state.rebuild();
    match net.screen {
        Screen::Browse => {
            if let Some(mut root) = widget(&assets, "multiplayer/multiplayer.gib") {
                if let Some(button) = action_button(
                    &root,
                    "IP connect button",
                    "IROH button",
                    "Join by code",
                    0,
                    0,
                    128,
                ) {
                    if let Some(group) = find_mut(&mut root, "Gamespy grp") {
                        group.children = vec![button];
                    }
                }
                text(&mut root, "Internet Label", "Iroh");
                patch(&mut root, "server listbox", "Columns", "5");
                state.root = Some(Arc::new(root));
            }
            state.checked.insert("LAN radio but".into(), true);
            state.interactive.insert("Internet Label".into());
            state.cells.insert(
                "heading grid".into(),
                ["Game name", "Players", "Address", "Port", "Connection"]
                    .iter()
                    .map(|v| Cell {
                        label: (*v).into(),
                        ..default()
                    })
                    .collect(),
            );
            let mut rows: Vec<Cell> = net
                .server_list
                .iter()
                .flat_map(|(addr, ad)| {
                    let selected = net.selected_server == Some(*addr);
                    [
                        ad.name.clone(),
                        format!("{}/{}", ad.occupied, ad.capacity),
                        addr.ip().to_string(),
                        addr.port().to_string(),
                        "LAN".into(),
                    ]
                    .into_iter()
                    .map(move |label| Cell {
                        key: format!("server:{addr}"),
                        label,
                        selected,
                        ..default()
                    })
                })
                .collect();
            let line = match &net.search {
                SearchState::Searching { .. } => Some(format!("Searching{}", ".".repeat(dots))),
                SearchState::Finished if rows.is_empty() => Some("No LAN games found".into()),
                SearchState::Failed(_) => Some("Search unavailable".into()),
                _ => None,
            };
            if let Some(label) = line {
                rows.push(Cell { label, ..default() });
            }
            state.cells.insert("server listbox".into(), rows);
            let status = match &net.search {
                SearchState::Failed(reason) => format!("LAN search failed: {reason}"),
                _ => net.status.clone(),
            };
            state.values.insert("status".into(), status);
        }
        Screen::Create => {
            state.root = assets
                .widget(if net.players.is_empty() {
                    "multiplayer/mp_create.gib"
                } else {
                    "multiplayer/changeoptions.gib"
                })
                .ok();
            let file = [
                "multiplayer/mpc_settings.gib",
                "exhibition/gametype.gib",
                "exhibition/arena.gib",
            ][net.tab];
            if let Some(mut panel) = widget(&assets, file) {
                if net.tab == 0 {
                    state
                        .values
                        .insert("server name textbox".into(), net.settings.name.clone());
                    visible(&mut panel, "password label", false);
                    visible(&mut panel, "password textbox", false);
                    visible(&mut panel, "connection speed label", false);
                    visible(&mut panel, "connection dropdown", false);
                    text(&mut panel, "internet label", "Iroh native networking");
                    patch(&mut panel, "LAN internet optgrp", "Size", "270 0 55 0");
                    patch(&mut panel, "internet label", "Size", "190 0 16 0");
                    state
                        .checked
                        .insert("internet radio but".into(), net.invite.is_some());
                    state
                        .checked
                        .insert("LAN radio but".into(), net.invite.is_none());
                    if net.players.is_empty() {
                        state
                            .interactive
                            .extend(["internet label".into(), "lan label".into()]);
                    } else {
                        state
                            .disabled
                            .extend(["internet radio but".into(), "LAN radio but".into()]);
                    }
                    if net.invite.is_some() {
                        state.visibility.insert("password textbox".into(), false);
                        state.visibility.insert("connection dropdown".into(), false);
                        invite_controls(&mut panel, &assets, &net, 50, 149);
                    }
                }
                if net.tab == 2
                    && let Some(a) = battle.arenas.iter().find(|a| a.key == net.settings.arena)
                {
                    image(&mut panel, "arena preview img", &a.preview);
                    state.values.insert(
                        "description".into(),
                        format!("<text font fg_mcn14>{}</text>", a.description),
                    );
                }
                state.slots.insert("content group".into(), Arc::new(panel));
            }
            for (i, (key, _)) in MODES.iter().enumerate() {
                state
                    .checked
                    .insert((*key).into(), net.settings.mode == i as u8);
            }
            for (key, w) in [
                ("all classes cb", None),
                ("lightweight cb", Some(0)),
                ("mediumweight cb", Some(1)),
                ("heavyweight cb", Some(2)),
            ] {
                state.checked.insert(key.into(), net.settings.weight == w);
            }
            for (i, key) in ["mp settings button", "gametype button", "arena button"]
                .iter()
                .enumerate()
            {
                state.checked.insert((*key).into(), i == net.tab);
            }
            state
                .checked
                .insert("hazard checkbox".into(), net.settings.hazards);
            state.values.insert(
                "time dropdown".into(),
                format!("{} minutes", net.settings.minutes),
            );
            state.choices.insert(
                "time dropdown".into(),
                [1, 2, 3, 4, 5, 10, 15, 20]
                    .iter()
                    .map(|m| format!("{m} minutes"))
                    .collect(),
            );
            state.cells.insert(
                "arena listbox".into(),
                battle
                    .arenas
                    .iter()
                    .enumerate()
                    .filter(|(_, a)| a.modes.contains(&mode(net.settings.mode)))
                    .map(|(i, a)| Cell {
                        key: format!("arena:{i}"),
                        label: a.name.clone(),
                        selected: net.settings.arena == a.key,
                        ..default()
                    })
                    .collect(),
            );
        }
        Screen::Lobby | Screen::Loading | Screen::Playing => {
            if let Some(mut root) = widget(&assets, "multiplayer/prematch.gib") {
                // The stock prematch sidebar reserves a full panel for spectators.
                if (net.invite.is_some() || (!net.host && net.players.is_empty()))
                    && let Some(sidebar) = find_parent_of(&mut root, "spectators label")
                {
                    sidebar.children.clear();
                    if net.invite.is_some() {
                        invite_controls(sidebar, &assets, &net, 5, 18);
                    } else {
                        let lines = if net.join_iroh
                            && net
                                .join_input
                                .parse::<arena_net_io::native::RoomCode>()
                                .is_ok()
                        {
                            vec!["Finding room on Mainline", net.join_input.as_str()]
                        } else {
                            vec!["Connecting to host..."]
                        };
                        for (i, line) in lines.into_iter().enumerate() {
                            sidebar.children.push(invite_text(
                                line,
                                4,
                                18 + i as u16 * 20,
                                "fg_mcn14",
                            ));
                        }
                    }
                }
                state.root = Some(Arc::new(root));
            }
            state.values.insert("screen name".into(), net.name.clone());
            state.values.insert(
                "arena name".into(),
                battle
                    .arenas
                    .iter()
                    .find(|a| a.key == net.settings.arena)
                    .map_or_else(|| net.settings.arena.clone(), |a| a.name.clone()),
            );
            state
                .values
                .insert("game type".into(), mode(net.settings.mode).tag().into());
            state.values.insert(
                "chat messages".into(),
                format!(
                    "<text font fg_mcn14>{}</text>",
                    net.chat.iter().cloned().collect::<Vec<_>>().join("\n")
                ),
            );
            state
                .values
                .insert("chat textbox".into(), net.chat_input.clone());
            state
                .visibility
                .insert("team2 group".into(), net.settings.capacity() > 2);
            for slot in 0..net.settings.capacity() {
                let p = net.players.iter().find(|p| p.slot as usize == slot);
                if let Some(mut card) = widget(
                    &assets,
                    if p.is_some() {
                        "multiplayer/activeplayer.gib"
                    } else {
                        "multiplayer/noplayer.gib"
                    },
                ) {
                    if let Some(p) = p {
                        // Both text rows sit inside the two-pixel frame in activeplayer.gib.
                        patch(&mut card, "team name", "Position", "2 0 2 0");
                        patch(&mut card, "team name", "Size", "146 0 14 0");
                        patch(&mut card, "bot name", "Position", "2 0 16 0");
                        patch(&mut card, "bot name", "Size", "146 0 16 0");
                        text(&mut card, "team name", &p.team);
                        text(&mut card, "bot name", &p.bot_name);
                        visible(&mut card, "spectator [TextBox]", false);
                        visible(&mut card, "eject button", false);
                        visible(&mut card, "change robot button", slot == net.local);
                        image(
                            &mut card,
                            "player icon img",
                            if slot == net.local {
                                "multiplayer/local_player_icon.bmp"
                            } else {
                                "multiplayer/remote_player_icon.bmp"
                            },
                        );
                        if let Some(bot) = net.bots.get(&p.digest)
                            && let Some(bytes) = &bot.snapshot
                        {
                            let key = thumbnail(p);
                            image(&mut card, "bot img", &key);
                            let _ = bytes;
                        }
                    }
                    rename(&mut card, &format!("p{slot}/"));
                    state
                        .slots
                        .insert(format!("player{} group", slot + 1), Arc::new(card));
                    state.checked.insert(
                        format!("p{slot}/ready checkbox"),
                        if slot == net.local {
                            net.local_ready()
                        } else {
                            p.is_some_and(|p| p.ready)
                        },
                    );
                    if slot != net.local {
                        state.read_only.insert(format!("p{slot}/ready checkbox"));
                    } else if p.is_none_or(|p| {
                        net.bots.get(&p.digest).is_none_or(|b| {
                            net.settings
                                .weight
                                .is_some_and(|w| b.weight_class != w as u32)
                        })
                    }) {
                        state.disabled.insert(format!("p{slot}/ready checkbox"));
                    }
                }
            }
            if !net.host
                || !net.start_available
                || net.pending_ready.is_some()
                || net.screen != Screen::Lobby
            {
                state.disabled.insert("begin match button".into());
            }
            if !net.host {
                state.disabled.insert("settings button".into());
            }
        }
    }
    if net.ip_dialog
        && let Some(mut dialog) = widget(&assets, "multiplayer/ip_connect.gib")
    {
        if net.join_iroh {
            text(&mut dialog, "caption bar", "Join Iroh game");
            fn join_labels(w: &mut Widget) {
                if w.kind == "Text" {
                    let label = w.value("_Text").trim();
                    let replacement = if label.starts_with("Enter the server IP") {
                        Some("Enter the room code or paste the host's EndpointID.")
                    } else if label.starts_with("example:") {
                        Some("Example: 0427-9136")
                    } else {
                        None
                    };
                    if let Some(value) = replacement {
                        w.fields.insert("_Text".into(), vec![value.into()]);
                    }
                }
                if let Some(inner) = w.wrapped.as_deref_mut() {
                    join_labels(inner);
                }
                for child in &mut w.children {
                    join_labels(child);
                }
            }
            join_labels(&mut dialog);
            patch(&mut dialog, "IP address", "Position", "28 0 97 0");
            patch(&mut dialog, "IP address", "Size", "284 0 25 0");
        }
        state.values.insert(
            "IP address".into(),
            if net.join_iroh {
                net.join_input.clone()
            } else {
                net.ip.clone()
            },
        );
        state.dialogs.push(Arc::new(dialog));
    }
    if net.picker
        && let Some(mut dialog) = widget(&assets, "exhibition/selectbot_player.gib")
    {
        let teams = crate::content::robots::teams(&battle.library);
        state.choices.insert(
            "team dropdown".into(),
            teams.iter().map(|(_, s)| s.clone()).collect(),
        );
        if let Some((e, label)) = teams.iter().find(|(e, _)| e.team_key == net.picker_team) {
            state.values.insert("team dropdown".into(), label.clone());
            state.values.insert("team motto".into(), e.motto.clone());
            image(&mut dialog, "team logo img", &e.logo);
        }
        state.cells.insert(
            "robots grid".into(),
            battle
                .library
                .iter()
                .enumerate()
                .filter(|(_, e)| {
                    e.team_key == net.picker_team
                        && net
                            .settings
                            .weight
                            .is_none_or(|w| e.robot.weight_class == w as u32)
                })
                .filter_map(|(i, e)| {
                    let mut cell = widget(&assets, "exhibition/choosebot_cell.gib")?;
                    image(&mut cell, "bot preview", &format!("exhibition/bot-{i}.tga"));
                    image(
                        &mut cell,
                        "weight icon",
                        match e.robot.weight_class {
                            0 => "exhibition/light_icon.bmp",
                            1 => "exhibition/medium_icon.bmp",
                            _ => "exhibition/heavy_icon.bmp",
                        },
                    );
                    Some(Cell {
                        key: format!("robot:{i}"),
                        selected: net.picker_bot == Some(i),
                        template: Some(Arc::new(cell)),
                        ..default()
                    })
                })
                .collect(),
        );
        text(
            &mut dialog,
            "robot name",
            net.picker_bot
                .and_then(|i| battle.library.get(i))
                .map_or("", |e| e.robot.name.as_str()),
        );
        if net.picker_bot.is_none() {
            state.disabled.insert("ok button".into());
        }
        state.dialogs.push(Arc::new(dialog));
    }
    if net.help {
        let instructions = match net.screen {
            Screen::Browse => {
                "Use Refresh List to find LAN games, IP Address for a direct connection, or Join by code to enter an eight-digit room code or EndpointID."
            }
            Screen::Create => {
                "Choose Iroh native networking to get a room code. Start Game opens the lobby. Share the eight digits or click EndpointID to copy the full host ID."
            }
            _ => {
                "Choose your robot and click Ready. When all players are ready, the host can begin the match."
            }
        };
        help(
            &mut state,
            &assets,
            "Multiplayer Help",
            "help/help_blank.gib",
        );
        state.values.insert("help text".into(), instructions.into());
    }
    if let Some(error) = &net.error
        && let Some(mut dialog) = widget(&assets, "common/messagebox.gib")
    {
        text(&mut dialog, "caption bar", "Multiplayer");
        state.values.insert("text box".into(), error.clone());
        for key in ["yes button", "no button", "cancel button"] {
            visible(&mut dialog, key, false);
        }
        visible(&mut dialog, "ok button", true);
        state.dialogs.push(Arc::new(dialog));
    }
}

fn find_mut<'a>(w: &'a mut Widget, name: &str) -> Option<&'a mut Widget> {
    if w.name() == name {
        return Some(w);
    }
    w.wrapped
        .as_deref_mut()
        .and_then(|inner| find_mut(inner, name))
        .or_else(|| {
            w.children
                .iter_mut()
                .find_map(|child| find_mut(child, name))
        })
}

fn find_parent_of<'a>(w: &'a mut Widget, name: &str) -> Option<&'a mut Widget> {
    if w.children.iter().any(|child| child.name() == name) {
        return Some(w);
    }
    w.wrapped
        .as_deref_mut()
        .and_then(|inner| find_parent_of(inner, name))
        .or_else(|| {
            w.children
                .iter_mut()
                .find_map(|child| find_parent_of(child, name))
        })
}

fn action_button(
    root: &Widget,
    original: &str,
    name: &str,
    caption: &str,
    x: u16,
    y: u16,
    width: u16,
) -> Option<Widget> {
    fn template<'a>(w: &'a Widget, name: &str) -> Option<&'a Widget> {
        if w.kind == "Border"
            && w.wrapped
                .as_deref()
                .is_some_and(|caption| caption.kind == "Caption" && caption.find(name).is_some())
        {
            return Some(w);
        }
        w.wrapped
            .as_deref()
            .and_then(|inner| template(inner, name))
            .or_else(|| w.children.iter().find_map(|child| template(child, name)))
    }
    let mut button = template(root, original)?.clone();
    let wrapper = button.wrapped.as_mut()?;
    let control = wrapper.wrapped.as_mut()?;
    control.fields.insert("Name".into(), vec![name.into()]);
    control
        .fields
        .insert("Position".into(), vec![format!("{x} 0 {y} 0")]);
    control
        .fields
        .insert("Size".into(), vec![format!("{width} 0 22 0")]);
    let label = wrapper.children.iter_mut().find(|w| w.kind == "Text")?;
    label.fields.insert("_Text".into(), vec![caption.into()]);
    label
        .fields
        .insert("Position".into(), vec!["6 0 14 0".into()]);
    Some(button)
}

fn invite_text(value: &str, x: u16, baseline: u16, font: &str) -> Widget {
    Widget {
        kind: "Text".into(),
        wrapped: None,
        children: vec![],
        fields: [
            ("_Font".to_string(), vec![font.into()]),
            ("_Text".into(), vec![value.into()]),
            ("Position".into(), vec![format!("{x} 0 {baseline} 0")]),
            ("Visible".into(), vec!["true".into()]),
        ]
        .into(),
    }
}

fn invite_controls(panel: &mut Widget, assets: &LegacyAssets, net: &Multiplayer, x: u16, y: u16) {
    let Some(invite) = &net.invite else {
        return;
    };
    panel
        .children
        .push(invite_text("Room code", x, y, "fg_mcn14"));
    panel
        .children
        .push(invite_text(&invite.code.to_string(), x, y + 22, "fg_d18"));
    let caption = if net.copied_endpoint {
        "Copied!"
    } else {
        "EndpointID"
    };
    if let Ok(dialog) = assets.widget("multiplayer/ip_connect.gib")
        && let Some(button) = action_button(
            &dialog,
            "connect button",
            "copy endpoint",
            caption,
            x,
            y + 29,
            80,
        )
    {
        panel.children.push(button);
    }
    let status = match net.publication {
        None => "Opens on Start Game",
        Some(Publication::Publishing(_)) => "Publishing code...",
        Some(Publication::Ready(_)) => "Code ready",
        Some(Publication::Retrying(_)) => "DHT retrying. Use EndpointID.",
    };
    panel
        .children
        .push(invite_text(status, x, y + 69, "fg_mcn14"));
}
