//! Original Multiplayer / IP Connect / Create / Prematch GIBs and native robot picker.
use super::*;
use crate::{
    editor::panels::{patch, text},
    legacy_ui::{Cell, State, UiEvent, assets::LegacyAssets, widgets::*},
};
use arena_core::match_rules::Mode;
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
) {
    if nav.page != Page::Multiplayer {
        return;
    }
    for event in std::mem::take(&mut events.0) {
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
                if net.ip_dialog {
                    match name.as_str() {
                        "cancel button" => net.ip_dialog = false,
                        "connect button" => {
                            let result = (|| {
                                let addr = service::address(&net.ip)?;
                                let m = net
                                    .manifest
                                    .clone()
                                    .ok_or("Compatibility manifest unavailable")?;
                                let cfg = net.settings.clone();
                                net.service
                                    .as_ref()
                                    .ok_or("Network worker unavailable")?
                                    .send(service::Command::Connect(cfg, m, addr))?;
                                net.host = false;
                                net.ip_dialog = false;
                                net.screen = Screen::Lobby;
                                net.status = format!("Connecting to {addr}...");
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
                    if let Some(index) = name
                        .strip_prefix("server:")
                        .and_then(|n| n.parse::<usize>().ok())
                    {
                        net.selected_server = net.server_list.keys().nth(index).copied();
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
                    match name.as_str(){
    "IROH button"|"Internet radio but"=>net.error=Some("IROH invite connections are not implemented yet. Use LAN or IP Connect.".into()),
    "Refresh button"|"LAN radio but"=>{net.server_list.clear();if let Some(s)=&net.service{let _=s.send(service::Command::Search);}net.status="Searching the local network...".into();}
    "Create Game button"=>{net.screen=Screen::Create;net.tab=0;net.host=true;}
    "IP connect button"=>net.ip_dialog=true,
    "Join game button"=>{if let Some(addr)=net.selected_server{net.ip=addr.to_string();net.ip_dialog=true;}else{net.error=Some("Select a LAN game or use IP Connect".into());}}
    "mp settings button"=>net.tab=0,"gametype button"=>net.tab=1,"arena button"=>net.tab=2,
    "hazard checkbox"=>net.settings.hazards=!net.settings.hazards,
    "all classes cb"=>net.settings.weight=None,"lightweight cb"=>net.settings.weight=Some(0),"mediumweight cb"=>net.settings.weight=Some(1),"heavyweight cb"=>net.settings.weight=Some(2),
    "start game button"=>{
     let result=(||{let m=net.manifest.clone().ok_or("Compatibility manifest unavailable")?;let cfg=net.settings.clone();net.service.as_ref().ok_or("Network worker unavailable")?.send(service::Command::Host(cfg,m,GAME_PORT))?;net.host=true;net.local=0;net.status=format!("Hosting on UDP port {GAME_PORT}");let index=net.selected.unwrap_or(0);net.choose(&battle,index)?;Ok(())})();show_error(&mut net,result);
    }
    "cancel game button"=>net.screen=Screen::Browse,
    "disconnect button"=>{net.disconnect();battle.network=false;net.service=Some(service::Service::spawn());}
    "begin match button"=>net.send(Request::Start),
    "accept button"=>{let settings=net.settings.clone();net.send(Request::Settings(settings));net.screen=Screen::Lobby;},
    "cancel button"=>net.screen=Screen::Lobby,
    "settings button" if net.host=>{net.screen=Screen::Create;net.tab=1;}
    "main menu button"|"Quit Session button"=>nav.page=Page::Main,
    "options button"=>{nav.back=nav.page;nav.page=Page::Settings;draft.0=settings.clone();}
    _=>{
     let key=if name.starts_with('p'){&name}else{&owner};
     if let Some(slot)=key.get(1..2).and_then(|n|n.parse::<usize>().ok()) && slot==net.local {
      if key.ends_with("ready checkbox"){let ready=!net.players.iter().find(|p|p.slot as usize==slot).is_some_and(|p|p.ready);net.send(Request::Ready(ready));}
      else if key.ends_with("change robot button") || key.ends_with("bot img") {net.picker=true;net.picker_bot=net.selected;net.picker_team=net.selected.and_then(|i|battle.library.get(i)).map_or_else(String::new,|e|e.team_key.clone());}
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
        "IP address" => n.ip = v.chars().take(128).collect(),
        "server name textbox" => n.settings.name = v.chars().take(96).collect(),
        "screen name" => n.name = v.chars().take(64).collect(),
        "chat textbox" => n.chat_input = v.chars().take(256).collect(),
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
    mut net: ResMut<Multiplayer>,
    battle: Res<battle::Session>,
    nav: Res<Navigation>,
    assets: Res<LegacyAssets>,
    mut state: ResMut<State>,
    keys: Res<ButtonInput<KeyCode>>,
    mut last: Local<Option<u64>>,
) {
    if nav.page != Page::Multiplayer
        && !(nav.page == Page::Settings && nav.back == Page::Multiplayer)
    {
        *last = None;
        return;
    }
    if keys.just_pressed(KeyCode::Enter)
        && !net.chat_input.is_empty()
        && net.screen == Screen::Lobby
    {
        let msg = std::mem::take(&mut net.chat_input);
        net.send(Request::Chat(msg));
        net.revision += 1;
    }
    if *last == Some(net.revision) {
        return;
    }
    *last = Some(net.revision);
    *state = State {
        revision: state.revision + 1,
        focused: state.focused.clone(),
        scroll: state.scroll.clone(),
        popup: state.popup.clone(),
        ..default()
    };
    if let Ok(footer) = assets.widget("common/bottomnav_main.gib") {
        state.slots.insert("bottom slot".into(), footer);
    }
    match net.screen {
        Screen::Browse => {
            if let Some(mut root) = widget(&assets, "multiplayer/multiplayer.gib") {
                // Replace the original GameSpy image in its original group with an ordinary button.
                fn original_button(
                    w: &ra2_formats::gib::Widget,
                ) -> Option<ra2_formats::gib::Widget> {
                    if w.kind == "Border" && w.find("IP connect button").is_some() {
                        return Some(w.clone());
                    }
                    w.wrapped
                        .as_deref()
                        .and_then(original_button)
                        .or_else(|| w.children.iter().find_map(original_button))
                }
                let button = original_button(&root);
                fn replace(
                    w: &mut ra2_formats::gib::Widget,
                    button: &Option<ra2_formats::gib::Widget>,
                ) {
                    if w.name() == "Gamespy grp" {
                        w.children.clear();
                        if let Some(mut b) = button.clone() {
                            fn caption(w: &mut ra2_formats::gib::Widget) {
                                if matches!(w.kind.as_str(), "Border" | "Caption" | "Button") {
                                    w.fields.insert("Position".into(), vec!["0 0 0 0".into()]);
                                    w.fields.insert("Size".into(), vec!["128 0 28 0".into()]);
                                }
                                if w.kind == "Button" {
                                    w.fields.insert("Name".into(), vec!["IROH button".into()]);
                                }
                                if w.kind == "Text" {
                                    w.fields.insert("_Text".into(), vec!["IROH".into()]);
                                }
                                if let Some(x) = &mut w.wrapped {
                                    caption(x);
                                }
                                for x in &mut w.children {
                                    caption(x);
                                }
                            }
                            caption(&mut b);
                            w.children.push(b);
                        }
                        return;
                    }
                    if let Some(x) = &mut w.wrapped {
                        replace(x, button);
                    }
                    for x in &mut w.children {
                        replace(x, button);
                    }
                }
                replace(&mut root, &button);
                patch(&mut root, "server listbox", "Columns", "5");
                state.root = Some(Arc::new(root));
            }
            state.checked.insert("LAN radio but".into(), true);
            state.disabled.insert("Internet radio but".into());
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
            state.cells.insert(
                "server listbox".into(),
                net.server_list
                    .iter()
                    .enumerate()
                    .flat_map(|(i, (a, ad))| {
                        let selected = net.selected_server == Some(*a);
                        [
                            ad.name.clone(),
                            format!("{}/{}", ad.occupied, ad.capacity),
                            a.ip().to_string(),
                            a.port().to_string(),
                            "LAN".into(),
                        ]
                        .into_iter()
                        .map(move |label| Cell {
                            key: format!("server:{i}"),
                            label,
                            selected,
                            ..default()
                        })
                    })
                    .collect(),
            );
            state.values.insert("status".into(), net.status.clone());
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
                    state.disabled.insert("internet radio but".into());
                    state.checked.insert("LAN radio but".into(), true);
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
            state.root = assets.widget("multiplayer/prematch.gib").ok();
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
                        p.is_some_and(|p| p.ready),
                    );
                    if slot != net.local
                        || p.is_none_or(|p| {
                            net.bots.get(&p.digest).is_none_or(|b| {
                                net.settings
                                    .weight
                                    .is_some_and(|w| b.weight_class != w as u32)
                            })
                        })
                    {
                        state.disabled.insert(format!("p{slot}/ready checkbox"));
                    }
                }
            }
            if !net.host
                || net.players.len() < net.settings.minimum()
                || !net.players.iter().all(|p| p.ready)
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
        && let Some(dialog) = widget(&assets, "multiplayer/ip_connect.gib")
    {
        state.values.insert("IP address".into(), net.ip.clone());
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
    if let Some(error) = &net.error
        && let Some(mut dialog) = widget(&assets, "common/messagebox.gib")
    {
        text(&mut dialog, "caption bar", "Multiplayer");
        text(&mut dialog, "text box", error);
        for key in ["yes button", "no button", "cancel button"] {
            visible(&mut dialog, key, false);
        }
        visible(&mut dialog, "ok button", true);
        state.dialogs.push(Arc::new(dialog));
    }
}
