//! Networking runs in real time on its own thread; loading arenas or pausing gameplay
//! cannot stop packet acknowledgements, discovery, heartbeat or connection timeouts.
use arena_net::{lobby::Manifest, session::*, *};
use arena_net_io::{
    ip::Connection,
    native::{HostInvite, JoinTarget, NativeGame, Publication},
    session::{Node, Notice, Request},
    udp::{Advertisement, Discovery},
};
use std::{
    net::{Ipv4Addr, SocketAddr},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};
pub enum Command {
    Host(Settings, Manifest, u16),
    Connect(Settings, Manifest, SocketAddr),
    HostIroh(Settings, Manifest, HostInvite),
    JoinIroh(Settings, Manifest, JoinTarget),
    Search(u64),
    Game(Request),
    Stop,
}
pub enum Event {
    Game(Notice),
    Found(SocketAddr, Advertisement),
    SearchFinished(u64, Option<String>),
    Input(u8, Wiring),
    Stats(u64, u64, u64),
    Error(String),
    Publication(Publication),
}
struct LanSearch {
    socket: Discovery,
    nonce: u64,
    started: Instant,
    next_probe: Duration,
}
impl LanSearch {
    fn start(nonce: u64) -> std::io::Result<Self> {
        let socket = Discovery::bind(0)?;
        socket.search(DISCOVERY_PORT, nonce)?;
        Ok(Self {
            socket,
            nonce,
            started: Instant::now(),
            next_probe: Duration::from_secs(1),
        })
    }
    fn poll(&mut self, out: &mpsc::SyncSender<Event>) -> bool {
        for (addr, ad) in self.socket.results(self.nonce) {
            let _ = out.try_send(Event::Found(addr, ad));
        }
        let age = self.started.elapsed();
        if age >= Duration::from_secs(3) {
            let _ = out.send(Event::SearchFinished(self.nonce, None));
            return true;
        }
        if age >= self.next_probe {
            // Repeat the broadcast so one lost UDP packet does not hide a lobby.
            let _ = self.socket.search(DISCOVERY_PORT, self.nonce);
            self.next_probe = age + Duration::from_secs(1);
        }
        false
    }
}
enum Game {
    Ip(Connection),
    Iroh(NativeGame),
}
impl Game {
    fn node(&mut self) -> &mut Node {
        match self {
            Self::Ip(c) => &mut c.node,
            Self::Iroh(c) => c.node(),
        }
    }
    fn poll(&mut self, now: u64) -> Result<()> {
        match self {
            Self::Ip(c) => c.poll(now),
            Self::Iroh(c) => c.poll(now),
        }
    }
}
pub struct Service {
    pub tx: mpsc::SyncSender<Command>,
    pub rx: std::sync::Mutex<mpsc::Receiver<Event>>,
}
impl Service {
    pub fn spawn() -> Self {
        let (tx, rx) = mpsc::sync_channel(256);
        let (out, events) = mpsc::sync_channel(512);
        thread::Builder::new()
            .name("arena-network".into())
            .spawn(move || {
                let origin = Instant::now();
                let mut connection: Option<Game> = None;
                let mut browser: Option<LanSearch> = None;
                let mut last_stats = 0;
                let mut publication = None;
                loop {
                    let now = origin.elapsed().as_micros() as u64;
                    let mut stop = false;
                    for _ in 0..64 {
                        let command = match rx.try_recv() {
                            Ok(c) => c,
                            Err(mpsc::TryRecvError::Empty) => break,
                            Err(mpsc::TryRecvError::Disconnected) => {
                                stop = true;
                                break;
                            }
                        };
                        let opening = matches!(
                            &command,
                            Command::Host(..)
                                | Command::Connect(..)
                                | Command::HostIroh(..)
                                | Command::JoinIroh(..)
                        );
                        let result: Result<()> = match command {
                            Command::Host(s, m, port) => {
                                browser = None;
                                Connection::host(s, m, port).map(|c| connection = Some(Game::Ip(c)))
                            }
                            Command::Connect(s, m, a) => {
                                browser = None;
                                Connection::connect(s, m, a).map(|c| connection = Some(Game::Ip(c)))
                            }
                            Command::HostIroh(s, m, invite) => {
                                browser = None;
                                NativeGame::host(s, m, invite)
                                    .map(|c| connection = Some(Game::Iroh(c)))
                            }
                            Command::JoinIroh(s, m, target) => {
                                browser = None;
                                NativeGame::join(s, m, target)
                                    .map(|c| connection = Some(Game::Iroh(c)))
                            }
                            Command::Search(nonce) => {
                                browser = match LanSearch::start(nonce) {
                                    Ok(search) => Some(search),
                                    Err(e) => {
                                        let _ = out.send(Event::SearchFinished(
                                            nonce,
                                            Some(e.to_string()),
                                        ));
                                        None
                                    }
                                };
                                Ok(())
                            }
                            Command::Game(r) => connection
                                .as_mut()
                                .ok_or(Error::Phase)
                                .and_then(|c| c.node().request(r)),
                            Command::Stop => {
                                if let Some(c) = &mut connection {
                                    let _ = c.node().request(Request::Leave);
                                    let _ = c.poll(now);
                                }
                                stop = true;
                                Ok(())
                            }
                        };
                        if let Err(e) = result {
                            let reason = match e {
                                Error::Io(message) => message,
                                other => format!("{other:?}"),
                            };
                            let event = if opening {
                                Event::Game(Notice::Disconnected(reason))
                            } else {
                                Event::Error(reason)
                            };
                            let _ = out.try_send(event);
                        }
                    }
                    if stop {
                        break;
                    }
                    if browser.as_mut().is_some_and(|search| search.poll(&out)) {
                        browser = None;
                    }
                    if let Some(c) = &mut connection {
                        if let Err(e) = c.poll(now) {
                            let reason = match e {
                                Error::Io(message) => message,
                                other => format!("{other:?}"),
                            };
                            let _ = out.try_send(Event::Game(Notice::Disconnected(reason)));
                            connection = None;
                            continue;
                        }
                        if let Game::Iroh(game) = c
                            && let Some(p) = game.take_publication()
                        {
                            publication = Some(p);
                        }
                        if let Some(p) = publication
                            && out.try_send(Event::Publication(p)).is_ok()
                        {
                            publication = None;
                        }
                        let node = c.node();
                        for (slot, input) in std::mem::take(&mut node.inputs) {
                            let _ = out.try_send(Event::Input(slot, input));
                        }
                        for notice in node.notices.drain(..) {
                            if let Err(mpsc::TrySendError::Full(_)) =
                                out.try_send(Event::Game(notice))
                            {
                                let _ = out.try_send(Event::Error(
                                    "Game stopped consuming network messages".into(),
                                ));
                                stop = true;
                                break;
                            }
                        }
                        if now - last_stats >= 500_000 {
                            last_stats = now;
                            let _ = out.try_send(Event::Stats(
                                node.rtt_us,
                                node.links.values().map(|l| l.sent_bytes).sum(),
                                node.links.values().map(|l| l.received_bytes).sum(),
                            ));
                        }
                    }
                    if stop {
                        break;
                    }
                    // The worker never touches Bevy's world or GPU.
                    thread::sleep(Duration::from_millis(2));
                }
            })
            .expect("network worker thread");
        Self {
            tx,
            rx: std::sync::Mutex::new(events),
        }
    }
    pub fn send(&self, c: Command) -> std::result::Result<(), String> {
        self.tx.try_send(c).map_err(|e| e.to_string())
    }
}
impl Drop for Service {
    fn drop(&mut self) {
        let _ = self.tx.try_send(Command::Stop);
    }
}
pub fn address(text: &str) -> std::result::Result<SocketAddr, String> {
    if let Ok(a) = text.trim().parse::<SocketAddr>() {
        return Ok(a);
    }
    if let Ok(ip) = text.trim().parse::<std::net::IpAddr>() {
        return Ok(SocketAddr::new(ip, GAME_PORT));
    }
    if text.trim() == "localhost" {
        return Ok((Ipv4Addr::LOCALHOST, GAME_PORT).into());
    }
    Err("Enter an IP address, optionally followed by :port".into())
}
