//! Multiplayer entry point. Exhibition remains a separate single-player workflow.
pub(crate) mod check;
pub(crate) mod prediction;
mod service;
mod ui;
mod world;
use crate::{
    battle,
    frontend::{Navigation, Page},
    legacy_ui,
};
use arena_net::{
    lobby::{Digest, Manifest},
    session::*,
};
use arena_net_io::session::{Notice, Request};
use bevy::prelude::*;
use std::{
    collections::{BTreeMap, VecDeque},
    net::SocketAddr,
    sync::Arc,
};
#[derive(Clone, Copy, PartialEq, Eq)]
enum Screen {
    Browse,
    Create,
    Lobby,
    Loading,
    Playing,
}
#[derive(Resource)]
pub(crate) struct Multiplayer {
    service: Option<service::Service>,
    screen: Screen,
    tab: usize,
    ip_dialog: bool,
    pub host: bool,
    pub local: usize,
    pub epoch: u64,
    pub paused: bool,
    pub diagnostics: bool,
    pub tick: u64,
    pub input_generation: u64,
    pub pause_barrier_tick: u64,
    settings: Settings,
    players: Vec<Player>,
    manifest: Option<Manifest>,
    selected: Option<usize>,
    picker: bool,
    picker_team: String,
    picker_bot: Option<usize>,
    bots: BTreeMap<Digest, Arc<arena_core::robot::RobotDefinition>>,
    server_list: BTreeMap<SocketAddr, arena_net_io::udp::Advertisement>,
    selected_server: Option<SocketAddr>,
    name: String,
    ip: String,
    pub(crate) error: Option<String>,
    status: String,
    chat: VecDeque<String>,
    chat_input: String,
    revision: u64,
    pub rtt: u64,
    pub sent: u64,
    pub received: u64,
    pub inputs: BTreeMap<u8, (Wiring, f64)>,
    pub frames: VecDeque<(Frame, f64)>,
    skins: Vec<Skin>,
    pub loaded: bool,
    loaded_sent: bool,
    shell_sent: BTreeMap<u8, u64>,
    pub last_frame_tick: u64,
}
impl Default for Multiplayer {
    fn default() -> Self {
        Self {
            service: None,
            screen: Screen::Browse,
            tab: 0,
            ip_dialog: false,
            host: false,
            local: 0,
            epoch: 0,
            paused: false,
            diagnostics: false,
            tick: 0,
            input_generation: 0,
            pause_barrier_tick: 0,
            settings: Settings {
                name: "New Multiplayer Match".into(),
                arena: "combatarena".into(),
                mode: 0,
                minutes: 5,
                hazards: true,
                weight: None,
            },
            players: vec![],
            manifest: None,
            selected: None,
            picker: false,
            picker_team: String::new(),
            picker_bot: None,
            bots: BTreeMap::new(),
            server_list: BTreeMap::new(),
            selected_server: None,
            name: "Player".into(),
            ip: "127.0.0.1".into(),
            error: None,
            status: String::new(),
            chat: VecDeque::new(),
            chat_input: String::new(),
            revision: 0,
            rtt: 0,
            sent: 0,
            received: 0,
            inputs: BTreeMap::new(),
            frames: VecDeque::new(),
            skins: vec![],
            loaded: false,
            loaded_sent: false,
            shell_sent: BTreeMap::new(),
            last_frame_tick: 0,
        }
    }
}
impl Multiplayer {
    fn send(&mut self, r: Request) {
        if let Some(s) = &self.service {
            if let Err(e) = s.send(service::Command::Game(r)) {
                self.error = Some(e);
            }
        }
    }
    fn choose(&mut self, session: &battle::Session, index: usize) -> Result<(), String> {
        let e = session.library.get(index).ok_or("Choose a robot")?;
        let bytes = ra2_formats::bot::encode(&e.robot)?;
        let digest = *blake3::hash(&bytes).as_bytes();
        validate_bot(&e.robot)?;
        self.bots.insert(digest, e.robot.clone());
        self.selected = Some(index);
        self.send(Request::Select(
            Player {
                slot: self.local as u8,
                alias: self.name.clone(),
                team: e.team.clone(),
                bot_name: e.robot.name.clone(),
                digest,
                size: bytes.len() as u32,
                ready: false,
            },
            bytes,
        ));
        Ok(())
    }
    fn disconnect(&mut self) {
        self.service = None;
        self.players.clear();
        self.screen = Screen::Browse;
        self.loaded = false;
        self.loaded_sent = false;
        self.frames.clear();
        self.inputs.clear();
        self.skins.clear();
        self.shell_sent.clear();
        self.tick = 0;
        self.input_generation = 0;
        self.pause_barrier_tick = 0;
        self.last_frame_tick = 0;
        self.paused = false;
        self.revision += 1;
    }
}
fn thumbnail(p: &Player) -> String {
    format!(
        "multiplayer/bot-{}-{}.tga",
        p.slot,
        blake3::Hash::from_bytes(p.digest).to_hex()
    )
}
fn validate_bot(r: &arena_core::robot::RobotDefinition) -> Result<(), String> {
    if r.components.is_empty()
        || r.components.len() > 256
        || r.controllers.iter().map(|c| c.keys.len()).sum::<usize>() > 128
        || r.chassis.model.is_none()
    {
        return Err("Robot exceeds network component/input limits or has no chassis".into());
    }
    Ok(())
}
pub(crate) struct MultiplayerPlugin;
impl Plugin for MultiplayerPlugin {
    fn build(&self, app: &mut App) {
        app.world_mut()
            .insert_non_send(prediction::Prediction::default());
        app.init_resource::<world::AuthoritativeInputs>()
            .init_resource::<prediction::Stats>()
            .init_resource::<Multiplayer>()
            .add_systems(
                Update,
                (
                    poll,
                    ui::input,
                    world::prepare,
                    world::apply,
                    prediction::reconcile,
                    prediction::send_input,
                    ui::bind,
                )
                    .chain()
                    .after(legacy_ui::Input)
                    .after(battle::lifecycle)
                    .before(crate::battle::Presentation)
                    .before(crate::battle::ui::bind)
                    .before(legacy_ui::Paint),
            )
            .add_systems(
                FixedUpdate,
                (prediction::tick, world::controls)
                    .chain()
                    .before(crate::gameplay::drive::motors),
            )
            .add_systems(
                FixedUpdate,
                world::capture
                    .after(bevy_rapier3d::prelude::PhysicsSet::Writeback)
                    .after(battle::step),
            );
    }
}
fn manifest(options: &crate::Options, config: &crate::config::Config) -> Result<Manifest, String> {
    let mut assets = blake3::Hasher::new();
    // Native files affecting geometry, motors, arena rules and collision must agree.
    let mut files = vec![];
    for path in [
        options.game_dir.join("Components.cfz"),
        options.game_dir.join("RA2.cfz"),
    ] {
        let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        assets.update(&bytes);
    }
    for dir in ["Components", "Arenas"] {
        crate::content::scan(&options.game_dir.join(dir), &mut files)?;
    }
    files.sort();
    for path in files {
        if path.extension().and_then(|s| s.to_str()).is_some_and(|e| {
            ["txt", "gmf", "gmi", "gma", "py"].contains(&e.to_ascii_lowercase().as_str())
        }) {
            assets.update(
                path.strip_prefix(&options.game_dir)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .replace('\\', "/")
                    .as_bytes(),
            );
            assets.update(&std::fs::read(&path).map_err(|e| e.to_string())?);
        }
    }
    let mut physics = blake3::Hasher::new();
    physics.update(format!("{:?}", config.garage).as_bytes());
    for code in [
        include_str!("../gameplay/drive.rs"),
        include_str!("../gameplay/tyres.rs"),
        include_str!("../gameplay/damage.rs"),
        include_str!("../gameplay/spawn.rs"),
        include_str!("../../crates/arena-core/src/mechanism.rs"),
    ] {
        physics.update(code.as_bytes());
    }
    Ok(Manifest {
        protocol: arena_net::PROTOCOL,
        build: {
            let mut h = blake3::Hasher::new();
            for code in [
                concat!(env!("CARGO_PKG_VERSION"), "/native-network-v2"),
                include_str!("../../crates/arena-net/src/session.rs"),
                include_str!("../../crates/arena-net-io/src/session.rs"),
                include_str!("world.rs"),
            ] {
                h.update(code.as_bytes());
            }
            h.finalize().into()
        },
        assets: assets.finalize().into(),
        physics: physics.finalize().into(),
    })
}
#[allow(clippy::too_many_arguments)]
fn poll(
    mut net: ResMut<Multiplayer>,
    mut s: ResMut<battle::Session>,
    mut nav: ResMut<Navigation>,
    options: Res<crate::Options>,
    config: Res<crate::config::Config>,
    time: Res<Time<Real>>,
    catalog: Res<crate::editor::catalog::Catalog>,
    mut art: ResMut<legacy_ui::assets::LegacyAssets>,
    keys: Res<ButtonInput<KeyCode>>,
) {
    if s.network && keys.just_pressed(KeyCode::F4) {
        net.diagnostics = !net.diagnostics;
    }
    if !options.multiplayer_check && nav.page == Page::Multiplayer && net.service.is_none() {
        battle::refresh_library(&mut s, &options, &catalog, &mut art);
        if net.manifest.is_none() {
            match manifest(&options, &config) {
                Ok(m) => net.manifest = Some(m),
                Err(e) => net.error = Some(e),
            }
        }
        if let Some(arena) = s.arenas.first() {
            net.settings.arena = arena.key.clone();
        }
        net.service = Some(service::Service::spawn());
        if let Some(service) = &net.service {
            let _ = service.send(service::Command::Search);
        }
        net.revision += 1;
    }
    let staying = matches!(
        nav.page,
        Page::Multiplayer | Page::Match | Page::Pause | Page::Results
    ) || nav.page == Page::Settings
        && matches!(nav.back, Page::Multiplayer | Page::Match | Page::Pause);
    if !staying && net.service.is_some() {
        net.disconnect();
        s.network = false;
    }
    let events = net
        .service
        .as_ref()
        .map(|service| {
            service
                .rx
                .lock()
                .unwrap()
                .try_iter()
                .take(512)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    for event in events {
        match event {
            service::Event::Found(a, ad) => {
                net.server_list.insert(a, ad);
                net.revision += 1;
            }
            service::Event::Error(e) => {
                if e.starts_with("Network error:") {
                    net.disconnect();
                    s.network = false;
                    nav.page = Page::Multiplayer;
                }
                if net.error.is_none() {
                    net.error = Some(e);
                }
                net.revision += 1;
            }
            service::Event::Input(slot, w) => {
                net.inputs.insert(slot, (w, time.elapsed_secs_f64()));
            }
            service::Event::Stats(rtt, sent, received) => {
                net.rtt = rtt;
                net.sent = sent;
                net.received = received;
            }
            service::Event::Game(notice) => {
                match notice {
                    Notice::Lobby => {
                        net.screen = Screen::Lobby;
                        net.loaded = false;
                        net.loaded_sent = false;
                        net.inputs.clear();
                        net.frames.clear();
                        s.network = false;
                        nav.page = Page::Multiplayer;
                    }
                    Notice::Connected(slot) => {
                        net.local = slot as usize;
                        net.status = "Connected".into();
                        if let Some(index) = net.selected {
                            if let Err(e) = net.choose(&s, index) {
                                net.error = Some(e);
                            }
                        }
                    }
                    Notice::Roster(settings, players, epoch) => {
                        net.settings = settings;
                        net.players = players;
                        net.epoch = epoch;
                        if net.screen != Screen::Playing && net.screen != Screen::Loading {
                            net.screen = Screen::Lobby;
                        }
                    }
                    Notice::Bot(digest, bytes) => {
                        match ra2_formats::bot::decode(&bytes)
                            .and_then(|bot| validate_bot(&bot).map(|_| bot))
                        {
                            Ok(bot) => {
                                net.bots.insert(digest, Arc::new(bot));
                            }
                            Err(e) => {
                                net.error = Some(format!("Invalid received BOT: {e}"));
                                net.send(Request::Leave);
                            }
                        }
                    }
                    Notice::Prepare(epoch) => {
                        net.epoch = epoch;
                        net.screen = Screen::Loading;
                        net.loaded = false;
                        net.loaded_sent = false;
                        net.frames.clear();
                        net.skins.clear();
                        net.tick = 0;
                        net.input_generation = 0;
                        net.pause_barrier_tick = 0;
                        net.last_frame_tick = 0;
                        net.shell_sent.clear();
                        net.status = "Loading arena and robots...".into();
                    }
                    Notice::Begin(epoch) => {
                        if epoch == net.epoch {
                            net.screen = Screen::Playing;
                            s.net_waiting = false;
                            if net.host {
                                s.begin_countdown();
                            }
                            nav.page = Page::Match;
                        }
                    }
                    Notice::Frame(f) => {
                        if f.epoch == net.epoch
                            && net.frames.back().is_none_or(|(last, _)| f.tick > last.tick)
                        {
                            net.frames.push_back((f, time.elapsed_secs_f64()));
                            while net.frames.len() > 32 {
                                net.frames.pop_front();
                            }
                        }
                    }
                    Notice::Sounds(events) => {
                        s.sound_events
                            .extend(events.into_iter().map(|(p, v)| (p, Vec3::from_array(v))));
                    }
                    Notice::Skin(skin) => net.skins.push(skin),
                    Notice::Pause(tick, paused) => {
                        net.pause_barrier_tick = tick;
                        if net.host && net.paused != paused {
                            net.input_generation += 1;
                        }
                        net.paused = paused;
                        s.net_paused = paused;
                    }
                    Notice::Chat(alias, text) => {
                        net.chat.push_back(format!("{alias}: {text}"));
                        while net.chat.len() > 32 {
                            net.chat.pop_front();
                        }
                    }
                    Notice::Disconnected(reason) => {
                        net.disconnect();
                        s.network = false;
                        nav.page = Page::Multiplayer;
                        net.error = Some(reason);
                    }
                }
                net.revision += 1;
            }
        }
    }
}
