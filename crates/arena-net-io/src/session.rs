//! Stateful lobby, BOT transfer, load barrier and native match delivery over admitted links.
//! This module has no socket or GUI dependency; the IP service only performs admission.
use crate::LinkEndpoint;
use arena_net::{
    codec::Message,
    lobby::{BotUpload, Digest, MAX_BOT_BYTES},
    session::*,
    transport::DatagramIo,
    *,
};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
#[derive(Clone, Debug)]
pub enum Notice {
    Connected(u8),
    Roster(Settings, Vec<Player>, u64),
    Bot(Digest, Vec<u8>),
    Prepare(u64),
    Begin(u64),
    Lobby,
    Frame(Frame),
    Skin(Skin),
    Sounds(Vec<(String, [f32; 3])>),
    Pause(u64, bool),
    Chat(String, String),
    Disconnected(String),
}
#[derive(Clone, Debug)]
pub enum Request {
    Select(Player, Vec<u8>),
    Ready(bool),
    Settings(Settings),
    Start,
    Loaded(u64),
    Frame(Frame),
    Skin(Skin),
    Wiring(Wiring),
    ReturnLobby,
    Sounds(u64, Vec<(String, [f32; 3])>),
    Pause(u64, bool),
    Chat(String),
    Leave,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Lobby,
    Loading,
    Playing,
    Closed,
}
pub struct Node {
    pub local: PeerId,
    pub stage: Stage,
    pub settings: Settings,
    pub players: BTreeMap<u8, Player>,
    pub epoch: u64,
    pub links: BTreeMap<PeerId, LinkEndpoint>,
    pub bots: BTreeMap<Digest, Vec<u8>>,
    pub notices: VecDeque<Notice>,
    pub inputs: BTreeMap<u8, Wiring>,
    pub rtt_us: u64,
    uploads: BTreeMap<Digest, BotUpload>,
    outbound: VecDeque<(PeerId, Digest, usize)>,
    input_sequences: BTreeMap<u8, (u64, u64)>,
    input_generation: u64,
    installed: BTreeMap<PeerId, BTreeSet<Digest>>,
    loaded: BTreeSet<PeerId>,
    last_seen: BTreeMap<PeerId, u64>,
    last_ping: u64,
    loading_since: Option<u64>,
}
fn empty(slot: u8) -> Player {
    Player {
        slot,
        alias: format!("Player {}", slot + 1),
        team: String::new(),
        bot_name: String::new(),
        digest: [0; 32],
        size: 0,
        ready: false,
    }
}
impl Node {
    pub fn new(local: PeerId, settings: Settings) -> Result<Self> {
        settings.validate()?;
        Ok(Self {
            local,
            stage: Stage::Lobby,
            settings,
            players: if local == HOST {
                BTreeMap::from([(0, empty(0))])
            } else {
                BTreeMap::new()
            },
            epoch: 0,
            links: BTreeMap::new(),
            bots: BTreeMap::new(),
            notices: VecDeque::new(),
            inputs: BTreeMap::new(),
            rtt_us: 0,
            uploads: BTreeMap::new(),
            input_sequences: BTreeMap::new(),
            input_generation: 0,
            outbound: VecDeque::new(),
            installed: BTreeMap::new(),
            loaded: BTreeSet::new(),
            last_seen: BTreeMap::new(),
            last_ping: 0,
            loading_since: None,
        })
    }
    pub fn add_peer(&mut self, peer: PeerId, session: SessionId, now: u64) -> Result<()> {
        if self.stage != Stage::Lobby || peer == self.local || self.links.contains_key(&peer) {
            return Err(Error::Phase);
        }
        if self.local == HOST
            && (peer.0 >= self.settings.capacity() as u64
                || self.players.len() >= self.settings.capacity())
        {
            return Err(Error::Limit);
        }
        self.links
            .insert(peer, LinkEndpoint::new(self.local, peer, session));
        self.last_seen.insert(peer, now);
        self.installed.insert(peer, BTreeSet::new());
        if self.local == HOST {
            self.players.insert(peer.0 as u8, empty(peer.0 as u8));
            self.roster()?;
        } else {
            self.send(HOST, Control::Connected)?;
            self.notices
                .push_back(Notice::Connected(self.local.0 as u8));
        }
        Ok(())
    }
    fn send(&mut self, to: PeerId, c: Control) -> Result<()> {
        c.validate()?;
        self.links
            .get_mut(&to)
            .ok_or(Error::Unauthorized)?
            .send(&Message::Control(encode(&c)?))
    }
    fn broadcast(&mut self, c: Control) -> Result<()> {
        let peers = self.links.keys().copied().collect::<Vec<_>>();
        for p in peers {
            self.send(p, c.clone())?;
        }
        Ok(())
    }
    fn roster(&mut self) -> Result<()> {
        let ps = self.players.values().cloned().collect::<Vec<_>>();
        self.broadcast(Control::Roster {
            settings: self.settings.clone(),
            players: ps.clone(),
            epoch: self.epoch,
        })?;
        self.notices
            .push_back(Notice::Roster(self.settings.clone(), ps, self.epoch));
        Ok(())
    }
    fn offer(&mut self, peer: PeerId, digest: Digest) -> Result<()> {
        if self
            .installed
            .get(&peer)
            .is_some_and(|set| set.contains(&digest))
        {
            return Ok(());
        }
        if let Some(bytes) = self.bots.get(&digest) {
            self.send(
                peer,
                Control::Offer {
                    digest,
                    size: bytes.len() as u32,
                },
            )?;
        }
        Ok(())
    }
    fn selected(&mut self, peer: PeerId, mut p: Player) -> Result<()> {
        if self.stage != Stage::Lobby || !self.players.contains_key(&(peer.0 as u8)) {
            return Err(Error::Phase);
        }
        p.slot = peer.0 as u8;
        p.ready = false;
        p.validate()?;
        self.players.insert(p.slot, p.clone());
        if self.bots.contains_key(&p.digest) {
            let peers = self.links.keys().copied().collect::<Vec<_>>();
            for to in peers {
                self.offer(to, p.digest)?;
            }
        } else {
            self.send(peer, Control::Need(p.digest))?;
        }
        self.roster()
    }
    pub fn can_start(&self) -> bool {
        self.local == HOST
            && self.stage == Stage::Lobby
            && self.players.len() >= self.settings.minimum()
            && self
                .players
                .values()
                .all(|p| p.ready && p.size > 0 && self.bots.contains_key(&p.digest))
            && self.links.keys().all(|peer| {
                self.players.values().all(|p| {
                    self.installed
                        .get(peer)
                        .is_some_and(|set| set.contains(&p.digest))
                })
            })
    }
    pub fn request(&mut self, r: Request) -> Result<()> {
        match r {
            Request::Select(mut p, bytes) => {
                if self.stage != Stage::Lobby || bytes.is_empty() || bytes.len() > MAX_BOT_BYTES {
                    return Err(Error::Phase);
                }
                p.digest = *blake3::hash(&bytes).as_bytes();
                p.size = bytes.len() as u32;
                p.slot = self.local.0 as u8;
                p.validate()?;
                self.bots.insert(p.digest, bytes);
                if self.local == HOST {
                    self.selected(HOST, p)?;
                } else {
                    self.send(HOST, Control::Select(p))?;
                }
            }
            Request::Ready(value) => {
                if self.local == HOST {
                    self.set_ready(HOST, value)?;
                } else {
                    self.send(HOST, Control::Ready(value))?;
                }
            }
            Request::Settings(settings) => {
                if self.local != HOST || self.stage != Stage::Lobby {
                    return Err(Error::Unauthorized);
                }
                settings.validate()?;
                if self
                    .players
                    .keys()
                    .any(|s| *s as usize >= settings.capacity())
                {
                    return Err(Error::Limit);
                }
                self.settings = settings;
                for p in self.players.values_mut() {
                    p.ready = false;
                }
                self.roster()?;
            }
            Request::Start => {
                if !self.can_start() {
                    return Err(Error::Phase);
                }
                self.stage = Stage::Loading;
                self.epoch += 1;
                self.loaded.clear();
                self.input_sequences.clear();
                self.input_generation = 0;
                self.inputs.clear();
                self.roster()?;
                self.broadcast(Control::Prepare { epoch: self.epoch })?;
                self.notices.push_back(Notice::Prepare(self.epoch));
            }
            Request::Loaded(epoch) => {
                if self.local == HOST {
                    self.mark_loaded(HOST, epoch)?;
                } else {
                    self.send(HOST, Control::Loaded { epoch })?;
                }
            }
            Request::Wiring(w) => {
                w.validate()?;
                if self.local == HOST {
                    self.inputs.insert(0, w);
                } else {
                    self.links
                        .get_mut(&HOST)
                        .ok_or(Error::Phase)?
                        .send(&Message::Wiring(encode(&w)?))?;
                }
            }
            Request::Frame(f) => {
                if self.local != HOST {
                    return Err(Error::Unauthorized);
                }
                f.validate()?;
                if f.input_generation != self.input_generation {
                    self.input_sequences.clear();
                    self.inputs.clear();
                    self.input_generation = f.input_generation;
                }
                let bytes = encode(&f)?;
                for link in self.links.values_mut() {
                    link.send(&Message::GameState(bytes.clone()))?;
                }
            }
            Request::Skin(s) => {
                if self.local != HOST {
                    return Err(Error::Unauthorized);
                }
                self.broadcast(Control::Skin(s))?;
            }
            Request::Pause(tick, paused) => {
                if self.local != HOST || self.stage != Stage::Playing {
                    return Err(Error::Unauthorized);
                }
                self.broadcast(Control::Pause {
                    epoch: self.epoch,
                    tick,
                    paused,
                })?;
                self.notices.push_back(Notice::Pause(tick, paused));
            }
            Request::Chat(text) => {
                let text = text.chars().take(256).collect::<String>();
                if self.local == HOST {
                    let alias = self
                        .players
                        .get(&0)
                        .map_or("Host", |p| &p.alias)
                        .to_string();
                    self.broadcast(Control::Chat {
                        alias: alias.clone(),
                        text: text.clone(),
                    })?;
                    self.notices.push_back(Notice::Chat(alias, text));
                } else {
                    self.send(
                        HOST,
                        Control::Chat {
                            alias: String::new(),
                            text,
                        },
                    )?;
                }
            }
            Request::Sounds(epoch, events) => {
                if self.local != HOST {
                    return Err(Error::Unauthorized);
                }
                self.broadcast(Control::Sounds { epoch, events })?;
            }
            Request::ReturnLobby => {
                if self.local != HOST || self.stage != Stage::Playing {
                    return Err(Error::Unauthorized);
                }
                self.stage = Stage::Lobby;
                for p in self.players.values_mut() {
                    p.ready = false;
                }
                self.inputs.clear();
                self.input_sequences.clear();
                self.broadcast(Control::ReturnLobby { epoch: self.epoch })?;
                self.roster()?;
                self.notices.push_back(Notice::Lobby);
            }
            Request::Leave => {
                self.broadcast(Control::Leave("Disconnected".into()))?;
                self.stage = Stage::Closed;
            }
        }
        Ok(())
    }
    fn set_ready(&mut self, peer: PeerId, value: bool) -> Result<()> {
        if self.stage != Stage::Lobby {
            return Err(Error::Phase);
        }
        let p = self
            .players
            .get_mut(&(peer.0 as u8))
            .ok_or(Error::Unauthorized)?;
        if value && (p.size == 0 || !self.bots.contains_key(&p.digest)) {
            return Err(Error::Incompatible);
        }
        p.ready = value;
        self.roster()
    }
    fn mark_loaded(&mut self, peer: PeerId, epoch: u64) -> Result<()> {
        if self.stage == Stage::Playing && epoch == self.epoch && self.loaded.contains(&peer) {
            return Ok(());
        }
        if self.stage != Stage::Loading || epoch != self.epoch {
            return Err(Error::Phase);
        }
        self.loaded.insert(peer);
        if self.loaded.len() == self.players.len() {
            self.stage = Stage::Playing;
            self.broadcast(Control::Begin { epoch })?;
            self.notices.push_back(Notice::Begin(epoch));
        }
        Ok(())
    }
    fn control(&mut self, from: PeerId, c: Control) -> Result<()> {
        c.validate()?;
        if self.local == HOST {
            match c {
                Control::Connected => {
                    self.roster()?;
                    let digests = self
                        .players
                        .values()
                        .filter(|p| p.size > 0)
                        .map(|p| p.digest)
                        .collect::<Vec<_>>();
                    for d in digests {
                        self.offer(from, d)?;
                    }
                }
                Control::Select(p) => {
                    if self.stage == Stage::Lobby {
                        self.selected(from, p)?;
                    }
                }
                Control::Ready(v) => {
                    if self.stage == Stage::Lobby {
                        self.set_ready(from, v)?;
                    }
                }
                Control::Loaded { epoch } => self.mark_loaded(from, epoch)?,
                Control::Chat { text, .. } => {
                    let alias = self
                        .players
                        .get(&(from.0 as u8))
                        .ok_or(Error::Unauthorized)?
                        .alias
                        .clone();
                    self.broadcast(Control::Chat {
                        alias: alias.clone(),
                        text: text.clone(),
                    })?;
                    self.notices.push_back(Notice::Chat(alias, text));
                }
                Control::Installed(d) => {
                    self.installed.entry(from).or_default().insert(d);
                }
                Control::Need(d) => {
                    self.offer(from, d)?;
                }
                Control::TransferReady(d) => self.enqueue_bot(from, d)?,
                Control::Offer { digest, size } => {
                    if self
                        .players
                        .get(&(from.0 as u8))
                        .is_none_or(|p| p.digest != digest || p.size != size)
                    {
                        return Err(Error::Unauthorized);
                    }
                    self.start_upload(digest, size)?;
                }
                Control::Ping(t) => self.send(from, Control::Pong(t))?,
                Control::Pong(_) => {}
                Control::Leave(_) => {
                    self.drop_peer(from, "Player disconnected")?;
                }
                _ => return Err(Error::Unauthorized),
            }
        } else {
            if from != HOST {
                return Err(Error::Unauthorized);
            }
            match c {
                Control::Roster {
                    settings,
                    players,
                    epoch,
                } => {
                    self.settings = settings.clone();
                    self.epoch = epoch;
                    self.players = players.iter().cloned().map(|p| (p.slot, p)).collect();
                    self.notices
                        .push_back(Notice::Roster(settings, players.clone(), epoch));
                    for p in players {
                        if p.size > 0 && !self.bots.contains_key(&p.digest) {
                            self.send(HOST, Control::Need(p.digest))?;
                        }
                    }
                }
                Control::Need(d) => {
                    self.send(
                        HOST,
                        Control::Offer {
                            digest: d,
                            size: self.bots.get(&d).ok_or(Error::Incompatible)?.len() as u32,
                        },
                    )?;
                }
                Control::TransferReady(d) => self.enqueue_bot(HOST, d)?,
                Control::Sounds { epoch, events } => {
                    if epoch == self.epoch {
                        self.notices.push_back(Notice::Sounds(events));
                    }
                }
                Control::Skin(s) => {
                    if s.epoch == self.epoch
                        && s.slot < 4
                        && s.positions.len() <= 16384
                        && s.positions.iter().flatten().all(|n| n.is_finite())
                    {
                        self.notices.push_back(Notice::Skin(s));
                    } else {
                        return Err(Error::Malformed);
                    }
                }
                Control::Offer { digest, size } => self.start_upload(digest, size)?,
                Control::Installed(_) => {}
                Control::Prepare { epoch } => {
                    if epoch != self.epoch || self.stage != Stage::Lobby {
                        return Err(Error::Phase);
                    }
                    self.stage = Stage::Loading;
                    self.notices.push_back(Notice::Prepare(epoch));
                }
                Control::ReturnLobby { epoch } => {
                    if epoch != self.epoch {
                        return Err(Error::Phase);
                    }
                    self.stage = Stage::Lobby;
                    self.notices.push_back(Notice::Lobby);
                }
                Control::Begin { epoch } => {
                    if epoch != self.epoch || self.stage != Stage::Loading {
                        return Err(Error::Phase);
                    }
                    self.stage = Stage::Playing;
                    self.notices.push_back(Notice::Begin(epoch));
                }
                Control::Pause {
                    epoch,
                    tick,
                    paused,
                } => {
                    if epoch == self.epoch {
                        self.notices.push_back(Notice::Pause(tick, paused));
                    }
                }
                Control::Chat { alias, text } => self.notices.push_back(Notice::Chat(alias, text)),
                Control::Ping(t) => self.send(HOST, Control::Pong(t))?,
                Control::Pong(_) => {}
                Control::Leave(reason) => {
                    self.stage = Stage::Closed;
                    self.notices.push_back(Notice::Disconnected(reason));
                }
                _ => return Err(Error::Unauthorized),
            }
        }
        Ok(())
    }
    fn start_upload(&mut self, d: Digest, size: u32) -> Result<()> {
        if self.bots.contains_key(&d) {
            if self.local != HOST {
                self.send(HOST, Control::Installed(d))?;
            }
            return Ok(());
        }
        if self.uploads.len() >= 8 {
            return Err(Error::Limit);
        }
        self.uploads
            .entry(d)
            .or_insert(BotUpload::new(d, size as usize)?);
        if self.local == HOST {
            let peer = self
                .players
                .values()
                .find(|p| p.digest == d)
                .ok_or(Error::Unauthorized)?
                .slot;
            self.send(PeerId(peer as u64), Control::TransferReady(d))?;
        } else {
            self.send(HOST, Control::TransferReady(d))?;
        }
        Ok(())
    }
    fn enqueue_bot(&mut self, to: PeerId, d: Digest) -> Result<()> {
        if !self.bots.contains_key(&d) {
            return Err(Error::Incompatible);
        }
        if !self.outbound.iter().any(|(p, h, _)| *p == to && *h == d) {
            if self.outbound.len() >= 32 {
                return Err(Error::Limit);
            }
            self.outbound.push_back((to, d, 0));
        }
        Ok(())
    }
    pub fn drop_peer(&mut self, peer: PeerId, reason: &str) -> Result<()> {
        self.links.remove(&peer);
        self.last_seen.remove(&peer);
        self.installed.remove(&peer);
        self.players.remove(&(peer.0 as u8));
        self.outbound.retain(|(p, _, _)| *p != peer);
        if peer == HOST || self.stage != Stage::Lobby {
            self.stage = Stage::Closed;
            self.broadcast(Control::Leave(reason.into()))?;
            self.notices.push_back(Notice::Disconnected(reason.into()));
        } else {
            self.roster()?;
        }
        Ok(())
    }
    pub fn pump(&mut self, now: u64, wire: &mut impl DatagramIo) -> Result<()> {
        if self.stage == Stage::Loading {
            let started = *self.loading_since.get_or_insert(now);
            if now.saturating_sub(started) > 60_000_000 {
                self.stage = Stage::Closed;
                self.broadcast(Control::Leave("Arena loading timed out".into()))?;
                self.notices
                    .push_back(Notice::Disconnected("Arena loading timed out".into()));
            }
        } else {
            self.loading_since = None;
        }
        let packets = wire.receive(now, self.local);
        let peers = self.links.keys().copied().collect::<Vec<_>>();
        for p in peers {
            let incoming = packets
                .iter()
                .filter(|d| d.from == p)
                .cloned()
                .collect::<Vec<_>>();
            if !incoming.is_empty() {
                self.last_seen.insert(p, now);
            }
            let messages = self
                .links
                .get_mut(&p)
                .unwrap()
                .update(now, incoming, wire)?;
            for m in messages {
                match m {
                    Message::Control(bytes) => {
                        let c: Control = decode(&bytes)?;
                        if let Control::Pong(t) = c {
                            self.rtt_us = now.saturating_sub(t);
                        } else {
                            self.control(p, c)?;
                        }
                    }
                    Message::Wiring(bytes) if self.local == HOST => {
                        let w: Wiring = decode(&bytes)?;
                        w.validate()?;
                        if w.epoch == self.epoch
                            && w.generation == self.input_generation
                            && self
                                .input_sequences
                                .get(&(p.0 as u8))
                                .is_none_or(|old| (w.generation, w.sequence) > *old)
                        {
                            self.input_sequences
                                .insert(p.0 as u8, (w.generation, w.sequence));
                            self.inputs.insert(p.0 as u8, w);
                        }
                    }
                    Message::GameState(bytes) if p == HOST => {
                        let f: Frame = decode(&bytes)?;
                        f.validate()?;
                        if f.epoch == self.epoch {
                            self.notices.push_back(Notice::Frame(f));
                        }
                    }
                    Message::Goodbye => self.drop_peer(p, "Player left")?,
                    _ => return Err(Error::Unauthorized),
                }
            }
            let chunks = self
                .links
                .get_mut(&p)
                .map(|l| std::mem::take(&mut l.bulk_received))
                .unwrap_or_default();
            for bytes in chunks {
                if bytes.len() < 36 {
                    return Err(Error::Malformed);
                }
                let digest: Digest = bytes[..32].try_into().unwrap();
                let offset = u32::from_le_bytes(bytes[32..36].try_into().unwrap()) as usize;
                if self.bots.contains_key(&digest) {
                    continue;
                }
                let upload = self.uploads.get_mut(&digest).ok_or(Error::Unauthorized)?;
                upload.chunk(offset, &bytes[36..])?;
                // Transfer length is verified by BotUpload, exposed without allocating another buffer.
                if upload.complete() {
                    let (_, data) = self.uploads.remove(&digest).unwrap().finish()?;
                    self.notices.push_back(Notice::Bot(digest, data.clone()));
                    self.bots.insert(digest, data);
                    if self.local == HOST {
                        let peers = self.links.keys().copied().collect::<Vec<_>>();
                        for to in peers {
                            self.offer(to, digest)?;
                        }
                    } else {
                        self.send(HOST, Control::Installed(digest))?;
                    }
                }
            }
        }
        // At most four chunks per pump; bulk cannot monopolise control/snapshot packets.
        for _ in 0..4 {
            let Some((peer, digest, offset)) = self.outbound.pop_front() else {
                break;
            };
            if let Some(data) = self.bots.get(&digest) {
                let end = (offset + 988).min(data.len());
                let mut chunk = digest.to_vec();
                chunk.extend((offset as u32).to_le_bytes());
                chunk.extend(&data[offset..end]);
                if let Some(link) = self.links.get_mut(&peer) {
                    link.send_bulk(chunk)?;
                    if end < data.len() {
                        self.outbound.push_back((peer, digest, end));
                    }
                }
            }
        }
        if now.saturating_sub(self.last_ping) >= 500_000 {
            self.last_ping = now;
            let peers = self.links.keys().copied().collect::<Vec<_>>();
            for p in peers {
                self.send(p, Control::Ping(now))?;
            }
        }
        for peer in self
            .last_seen
            .iter()
            .filter(|(_, at)| now.saturating_sub(**at) > TIMEOUT_US)
            .map(|(p, _)| *p)
            .collect::<Vec<_>>()
        {
            self.drop_peer(peer, "Connection timed out")?;
        }
        if self.notices.len() > 1024 {
            return Err(Error::Limit);
        }
        Ok(())
    }
}
