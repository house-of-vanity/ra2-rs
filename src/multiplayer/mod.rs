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
use arena_net_io::{
    native::{HostInvite, JoinTarget, Publication},
    session::{Notice, Request},
};
use bevy::prelude::*;
use std::{
    collections::{BTreeMap, VecDeque},
    net::SocketAddr,
    sync::Arc,
    time::Duration,
};
#[derive(Clone, Copy, PartialEq, Eq)]
enum Screen {
    Browse,
    Create,
    Lobby,
    Loading,
    Playing,
}
#[derive(Default)]
enum SearchState {
    #[default]
    Idle,
    Searching {
        nonce: u64,
        started: Duration,
    },
    Finished,
    Failed(String),
}
impl SearchState {
    fn accepts(&self, id: u64) -> bool {
        matches!(self, Self::Searching { nonce, .. } if *nonce == id)
    }
    fn dots(&self, now: Duration) -> usize {
        match self {
            Self::Searching { started, .. } => {
                1 + (now.saturating_sub(*started).as_millis() / 350 % 3) as usize
            }
            _ => 0,
        }
    }
}
#[derive(Clone, Copy)]
struct PendingReady {
    sent: bool,
    wanted: bool,
}

#[derive(Resource)]
pub(crate) struct Multiplayer {
    service: Option<service::Service>,
    screen: Screen,
    tab: usize,
    ip_dialog: bool,
    help: bool,
    join_iroh: bool,
    join_input: String,
    invite: Option<HostInvite>,
    publication: Option<Publication>,
    copied_endpoint: bool,
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
    start_available: bool,
    pending_ready: Option<PendingReady>,
    manifest: Option<Manifest>,
    selected: Option<usize>,
    picker: bool,
    picker_team: String,
    picker_bot: Option<usize>,
    bots: BTreeMap<Digest, Arc<arena_core::robot::RobotDefinition>>,
    server_list: BTreeMap<SocketAddr, arena_net_io::udp::Advertisement>,
    selected_server: Option<SocketAddr>,
    search: SearchState,
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
            help: false,
            join_iroh: false,
            join_input: String::new(),
            invite: None,
            publication: None,
            copied_endpoint: false,
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
            start_available: false,
            pending_ready: None,
            manifest: None,
            selected: None,
            picker: false,
            picker_team: String::new(),
            picker_bot: None,
            bots: BTreeMap::new(),
            server_list: BTreeMap::new(),
            selected_server: None,
            search: SearchState::Idle,
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
    fn local_ready(&self) -> bool {
        self.pending_ready.map_or_else(
            || {
                self.players
                    .iter()
                    .any(|p| p.slot as usize == self.local && p.ready)
            },
            |pending| pending.wanted,
        )
    }
    fn toggle_ready(&mut self) {
        let wanted = !self.local_ready();
        if let Some(pending) = &mut self.pending_ready {
            pending.wanted = wanted;
        } else {
            self.submit_ready(wanted);
        }
    }
    fn submit_ready(&mut self, wanted: bool) {
        let Some(service) = &self.service else { return };
        match service.send(service::Command::Game(Request::Ready(wanted))) {
            Ok(()) => {
                self.pending_ready = Some(PendingReady {
                    sent: wanted,
                    wanted,
                })
            }
            Err(error) => {
                self.pending_ready = None;
                self.error = Some(error);
            }
        }
    }
    fn update_roster(&mut self, settings: Settings, players: Vec<Player>, epoch: u64) {
        let previous = self.players.iter().find(|p| p.slot as usize == self.local);
        let local = players.iter().find(|p| p.slot as usize == self.local);
        if settings != self.settings
            || self.epoch != epoch
            || previous.map(|p| p.digest) != local.map(|p| p.digest)
        {
            self.pending_ready = None;
        } else if let Some(pending) = self.pending_ready
            && local.is_some_and(|p| p.ready == pending.sent)
        {
            self.pending_ready = None;
            if pending.wanted != pending.sent {
                self.submit_ready(pending.wanted);
            }
        }
        self.settings = settings;
        self.players = players;
        self.epoch = epoch;
    }
    fn send(&mut self, r: Request) {
        if let Some(s) = &self.service {
            if let Err(e) = s.send(service::Command::Game(r)) {
                self.error = Some(e);
            }
        }
    }
    fn choose(&mut self, session: &battle::Session, index: usize) -> Result<(), String> {
        let request = self.robot_request(session, index)?;
        self.pending_ready = None;
        self.send(request);
        Ok(())
    }
    fn robot_request(
        &mut self,
        session: &battle::Session,
        index: usize,
    ) -> Result<Request, String> {
        let e = session.library.get(index).ok_or("Choose a robot")?;
        let bytes = ra2_formats::bot::encode(&e.robot)?;
        let digest = *blake3::hash(&bytes).as_bytes();
        validate_bot(&e.robot)?;
        self.bots.insert(digest, e.robot.clone());
        self.selected = Some(index);
        Ok(Request::Select(
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
        ))
    }
    fn refresh_servers(&mut self, started: Duration) {
        self.server_list.clear();
        self.selected_server = None;
        self.status.clear();
        let nonce = arena_net_io::ip::nonce();
        self.search = match self
            .service
            .as_ref()
            .map(|s| s.send(service::Command::Search(nonce)))
        {
            Some(Ok(())) => SearchState::Searching { nonce, started },
            Some(Err(e)) => SearchState::Failed(e),
            None => SearchState::Failed("Network worker unavailable".into()),
        };
        self.revision += 1;
    }
    fn start_host(&mut self, battle: &battle::Session) -> Result<(), String> {
        let manifest = self
            .manifest
            .clone()
            .ok_or("Compatibility manifest unavailable")?;
        self.local = 0;
        let robot = self.robot_request(battle, self.selected.unwrap_or(0))?;
        let settings = self.settings.clone();
        let command = if let Some(invite) = &self.invite {
            service::Command::HostIroh(settings, manifest, invite.clone())
        } else {
            service::Command::Host(settings, manifest, GAME_PORT)
        };
        let service = self.service.as_ref().ok_or("Network worker unavailable")?;
        service.send(command)?;
        if let Err(e) = service.send(service::Command::Game(robot)) {
            self.disconnect();
            return Err(e);
        }
        self.host = true;
        self.screen = Screen::Lobby;
        self.status = if let Some(invite) = &self.invite {
            self.publication = Some(Publication::Publishing(invite.code));
            "Hosting with iroh".into()
        } else {
            format!("Hosting on UDP port {GAME_PORT}")
        };
        Ok(())
    }
    fn disconnect(&mut self) {
        self.service = None;
        self.host = false;
        self.search = SearchState::Idle;
        self.status.clear();
        self.ip_dialog = false;
        self.help = false;
        self.join_iroh = false;
        self.invite = None;
        self.publication = None;
        self.copied_endpoint = false;
        self.players.clear();
        self.start_available = false;
        self.pending_ready = None;
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
        net.refresh_servers(time.elapsed());
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
                if net.search.accepts(ad.nonce) && net.server_list.get(&a) != Some(&ad) {
                    net.server_list.insert(a, ad);
                    net.revision += 1;
                }
            }
            service::Event::SearchFinished(nonce, error) => {
                if net.search.accepts(nonce) {
                    net.search = error.map_or(SearchState::Finished, SearchState::Failed);
                    net.revision += 1;
                }
            }
            service::Event::Error(e) => {
                if net.error.is_none() {
                    net.error = Some(e);
                }
                net.revision += 1;
            }
            service::Event::Publication(p) => {
                let code = match p {
                    Publication::Ready(code)
                    | Publication::Retrying(code)
                    | Publication::Publishing(code) => code,
                };
                if let Some(invite) = &mut net.invite
                    && invite.code != code
                {
                    let previous = invite.code;
                    invite.code = code;
                    net.error = Some(format!(
                        "Room code {previous} was already in use. Your new code is {code}."
                    ));
                }
                net.publication = Some(p);
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
                        net.update_roster(settings, players, epoch);
                        if net.screen != Screen::Playing && net.screen != Screen::Loading {
                            net.screen = Screen::Lobby;
                        }
                    }
                    Notice::StartAvailable(available) => net.start_available = available,
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

#[cfg(test)]
mod readiness_tests {
    use super::*;
    use std::sync::{Mutex, mpsc};

    #[test]
    fn two_clicks_before_the_host_replies_keep_the_last_choice() {
        let (tx, commands) = mpsc::sync_channel(4);
        let (_, rx) = mpsc::sync_channel(4);
        let mut net = Multiplayer {
            service: Some(service::Service {
                tx,
                rx: Mutex::new(rx),
            }),
            players: vec![Player {
                slot: 0,
                alias: "Host".into(),
                team: "Team".into(),
                bot_name: "Bot".into(),
                digest: [1; 32],
                size: 100,
                ready: false,
            }],
            ..Default::default()
        };
        net.toggle_ready();
        net.toggle_ready();
        assert!(!net.local_ready());
        assert!(matches!(
            commands.try_recv(),
            Ok(service::Command::Game(Request::Ready(true)))
        ));
        assert!(commands.try_recv().is_err());
        let mut players = net.players.clone();
        players[0].ready = true;
        net.update_roster(net.settings.clone(), players.clone(), 0);
        assert!(!net.local_ready());
        assert!(matches!(
            commands.try_recv(),
            Ok(service::Command::Game(Request::Ready(false)))
        ));
        players[0].ready = false;
        net.update_roster(net.settings.clone(), players, 0);
        assert!(!net.local_ready() && net.pending_ready.is_none());
        assert!(commands.try_recv().is_err());
    }
}
