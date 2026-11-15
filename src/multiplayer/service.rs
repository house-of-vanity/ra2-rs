//! Networking runs in real time on its own thread; loading arenas or pausing gameplay
//! cannot stop packet acknowledgements, discovery, heartbeat or connection timeouts.
use arena_net::{lobby::Manifest, session::*, *};
use arena_net_io::{
    ip::Connection,
    session::{Notice, Request},
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
    Search,
    Game(Request),
    Stop,
}
pub enum Event {
    Game(Notice),
    Found(SocketAddr, Advertisement),
    Input(u8, Wiring),
    Stats(u64, u64, u64),
    Error(String),
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
                let mut connection: Option<Connection> = None;
                let mut browser: Option<Discovery> = None;
                let mut search_nonce = 0;
                let mut last_stats = 0;
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
                        let result: Result<()> = match command {
                            Command::Host(s, m, port) => {
                                browser = None;
                                Connection::host(s, m, port).map(|c| connection = Some(c))
                            }
                            Command::Connect(s, m, a) => {
                                browser = None;
                                Connection::connect(s, m, a).map(|c| connection = Some(c))
                            }
                            Command::Search => {
                                if browser.is_none() {
                                    browser = Discovery::bind(0).ok();
                                }
                                search_nonce = arena_net_io::ip::nonce();
                                browser
                                    .as_ref()
                                    .ok_or_else(|| {
                                        Error::Io("Cannot create LAN discovery socket".into())
                                    })
                                    .and_then(|b| {
                                        b.search(DISCOVERY_PORT, search_nonce)
                                            .map(|_| ())
                                            .map_err(|e| Error::Io(e.to_string()))
                                    })
                            }
                            Command::Game(r) => connection
                                .as_mut()
                                .ok_or(Error::Phase)
                                .and_then(|c| c.node.request(r)),
                            Command::Stop => {
                                if let Some(c) = &mut connection {
                                    let _ = c.node.request(Request::Leave);
                                    let _ = c.poll(now);
                                }
                                stop = true;
                                Ok(())
                            }
                        };
                        if let Err(e) = result {
                            let _ = out.try_send(Event::Error(format!("{e:?}")));
                        }
                    }
                    if stop {
                        break;
                    }
                    if let Some(b) = &browser {
                        for (a, ad) in b.results(search_nonce) {
                            let _ = out.try_send(Event::Found(a, ad));
                        }
                    }
                    if let Some(c) = &mut connection {
                        if let Err(e) = c.poll(now) {
                            let _ = out.try_send(Event::Error(format!("Network error: {e:?}")));
                            connection = None;
                            continue;
                        }
                        for (slot, input) in std::mem::take(&mut c.node.inputs) {
                            let _ = out.try_send(Event::Input(slot, input));
                        }
                        for notice in c.node.notices.drain(..) {
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
                                c.node.rtt_us,
                                c.node.links.values().map(|l| l.sent_bytes).sum(),
                                c.node.links.values().map(|l| l.received_bytes).sum(),
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
