//! Authenticated iroh datagrams underneath the same admission and game protocol as UDP.
mod room;
use crate::{
    ip::{Connection, SocketIo},
    session::{Node, Stage},
};
use arena_net::{
    Error, HOST, PeerId, Result,
    lobby::Manifest,
    session::Settings,
    transport::{Datagram, DatagramIo},
};
use bytes::Bytes;
use iroh::{
    Endpoint, EndpointAddr, EndpointId, SecretKey,
    endpoint::{Builder, Connection as Quic, presets},
};
use room::Directory;
pub use room::{Publication, RoomCode};
use std::{collections::BTreeMap, io, str::FromStr, sync::Arc, time::Duration};
use tokio::{
    runtime::Runtime,
    sync::{Semaphore, mpsc},
    time::timeout,
};

const ALPN: &[u8] = b"ra2-rs/game/1";
// Leave room for the QUIC DATAGRAM frame and encryption even on a 1200-byte path.
const GAME_MTU: usize = 1100;

#[derive(Clone)]
pub struct HostInvite {
    pub code: RoomCode,
    secret: SecretKey,
}
impl HostInvite {
    pub fn generate() -> Self {
        Self {
            code: RoomCode::generate(),
            secret: SecretKey::generate(),
        }
    }
    pub fn endpoint_id(&self) -> EndpointId {
        self.secret.public()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JoinTarget {
    Code(RoomCode),
    Endpoint(EndpointId),
}
impl FromStr for JoinTarget {
    type Err = String;
    fn from_str(input: &str) -> std::result::Result<Self, String> {
        let s = input.trim();
        if let Ok(code) = s.parse() {
            return Ok(Self::Code(code));
        }
        s.parse()
            .map(Self::Endpoint)
            .map_err(|_| "Enter an eight-digit room code or a full EndpointID".into())
    }
}

enum Role {
    Host(HostInvite),
    Guest(JoinTarget),
}
enum Signal {
    Open(Quic),
    Packet(EndpointId, usize, Bytes),
    Closed(EndpointId, usize),
    Publication(Publication),
    Failed(String),
}
type Signals = mpsc::Sender<Signal>;

/// The runtime and directory tasks live exactly as long as this game.
pub struct NativeGame {
    session: Connection<IrohIo>,
    events: mpsc::Receiver<Signal>,
    publication: Option<Publication>,
    runtime: Option<Runtime>,
}
impl NativeGame {
    pub fn host(settings: Settings, manifest: Manifest, invite: HostInvite) -> Result<Self> {
        Self::start(
            settings,
            manifest,
            Role::Host(invite),
            Endpoint::builder(presets::N0),
            Directory::Public,
        )
    }
    pub fn join(settings: Settings, manifest: Manifest, target: JoinTarget) -> Result<Self> {
        Self::start(
            settings,
            manifest,
            Role::Guest(target),
            Endpoint::builder(presets::N0),
            Directory::Public,
        )
    }
    fn start(
        settings: Settings,
        manifest: Manifest,
        role: Role,
        builder: Builder,
        directory: Directory,
    ) -> Result<Self> {
        let local = if matches!(role, Role::Host(_)) {
            HOST
        } else {
            PeerId(1)
        };
        let session = Connection::new(settings, manifest, IrohIo::new(local), local, None)?;
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .thread_name("ra2-iroh")
            .build()
            .map_err(|e| Error::Io(e.to_string()))?;
        let (tx, events) = mpsc::channel(1024);
        runtime.spawn(async move {
            if let Err(e) = run(builder, directory, role, tx.clone()).await {
                let _ = tx.send(Signal::Failed(e)).await;
            }
        });
        Ok(Self {
            session,
            events,
            publication: None,
            runtime: Some(runtime),
        })
    }
    pub fn node(&mut self) -> &mut Node {
        &mut self.session.node
    }
    pub fn take_publication(&mut self) -> Option<Publication> {
        self.publication.take()
    }
    pub fn poll(&mut self, now: u64) -> Result<()> {
        for _ in 0..512 {
            let Ok(event) = self.events.try_recv() else {
                break;
            };
            match event {
                Signal::Open(link) => {
                    let id = link.remote_id();
                    if self.session.io.links.contains_key(&id) {
                        link.close(0u32.into(), b"Already connected");
                        continue;
                    }
                    if self.session.node.local != HOST {
                        self.session.set_remote(id);
                    }
                    self.session.io.links.insert(id, link);
                }
                Signal::Packet(id, connection, bytes) => {
                    if self.session.io.current(id, connection) {
                        self.session.io.inbox.push((id, bytes));
                    }
                }
                Signal::Closed(id, connection) => {
                    if self.session.io.current(id, connection) {
                        self.session.io.links.remove(&id);
                        if self.session.node.stage != Stage::Closed {
                            if let Some(peer) = self.session.io.peer(id) {
                                self.session
                                    .node
                                    .drop_peer(peer, "Iroh connection closed")?;
                            } else if self.session.node.local != HOST {
                                return Err(Error::Io(
                                    "The host closed the iroh connection".into(),
                                ));
                            }
                        }
                    }
                }
                Signal::Failed(e) => return Err(Error::Io(e)),
                Signal::Publication(p) => self.publication = Some(p),
            }
        }
        self.session.poll(now)
    }
}
impl Drop for NativeGame {
    fn drop(&mut self) {
        for link in self.session.io.links.values() {
            link.close(0u32.into(), b"Left game");
        }
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_background();
        }
    }
}

async fn run(
    builder: Builder,
    directory: Directory,
    role: Role,
    tx: Signals,
) -> std::result::Result<(), String> {
    let mut builder = builder.alpns(vec![ALPN.to_vec()]);
    if let Role::Host(invite) = &role {
        builder = builder.secret_key(invite.secret.clone());
    }
    let endpoint = builder
        .bind()
        .await
        .map_err(|e| format!("Cannot start iroh: {e}"))?;
    match role {
        Role::Host(invite) => {
            tokio::spawn(room::publish(
                directory,
                endpoint.clone(),
                invite.code,
                tx.clone(),
            ));
            let permits = Arc::new(Semaphore::new(12));
            while let Some(incoming) = endpoint.accept().await {
                let Ok(permit) = permits.clone().try_acquire_owned() else {
                    incoming.refuse();
                    continue;
                };
                let events = tx.clone();
                tokio::spawn(async move {
                    let _permit = permit;
                    if let Ok(Ok(link)) =
                        timeout(Duration::from_secs(15), async { incoming.await }).await
                    {
                        let _ = read_peer(link, events).await;
                    }
                });
            }
        }
        Role::Guest(target) => {
            let link = dial(&endpoint, &directory, target).await?;
            read_peer(link, tx).await?;
        }
    }
    Ok(())
}

async fn dial(
    endpoint: &Endpoint,
    directory: &Directory,
    target: JoinTarget,
) -> std::result::Result<Quic, String> {
    let (mut addr, room) = match target {
        JoinTarget::Code(code) => {
            let dht = directory.open().await?;
            (room::resolve(&dht, code).await?, Some((dht, code)))
        }
        JoinTarget::Endpoint(id) => (EndpointAddr::new(id), None),
    };
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    let mut delay = Duration::from_secs(1);
    loop {
        let attempt = deadline.min(tokio::time::Instant::now() + Duration::from_secs(16));
        let reason =
            match tokio::time::timeout_at(attempt, endpoint.connect(addr.clone(), ALPN)).await {
                Ok(Ok(link)) => return Ok(link),
                Ok(Err(e)) => e.to_string(),
                Err(_) => "connection timed out".into(),
            };
        if tokio::time::Instant::now() + delay >= deadline {
            return Err(format!(
                "Cannot reach the iroh host: {reason}. Check that the lobby is still open."
            ));
        }
        // A friend can paste a freshly generated ID before its relay address has propagated.
        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(Duration::from_secs(5));
        if let Some((dht, code)) = &room
            && let Ok(Ok(updated)) =
                timeout(Duration::from_secs(4), room::resolve(dht, *code)).await
            && updated.id == addr.id
        {
            addr = updated;
        }
    }
}

async fn read_peer(link: Quic, tx: Signals) -> std::result::Result<(), String> {
    if link.max_datagram_size().is_none_or(|size| size < GAME_MTU) {
        link.close(0u32.into(), b"Game datagrams not supported");
        return Err("The peer does not support the required game datagram size".into());
    }
    let id = link.remote_id();
    let connection = link.stable_id();
    if tx.send(Signal::Open(link.clone())).await.is_err() {
        return Ok(());
    }
    while let Ok(Ok(bytes)) = timeout(Duration::from_secs(25), link.read_datagram()).await {
        if bytes.len() <= GAME_MTU {
            // Congestion drops unreliable packets. Lightyear retransmits reliable lanes.
            if let Err(mpsc::error::TrySendError::Closed(_)) =
                tx.try_send(Signal::Packet(id, connection, bytes))
            {
                return Ok(());
            }
        }
    }
    link.close(0u32.into(), b"Session ended");
    let _ = tx.send(Signal::Closed(id, connection)).await;
    Ok(())
}

struct IrohIo {
    local: PeerId,
    links: BTreeMap<EndpointId, Quic>,
    routes: BTreeMap<PeerId, EndpointId>,
    inbox: Vec<(EndpointId, Bytes)>,
    admission: Vec<(EndpointId, Vec<u8>)>,
}
impl IrohIo {
    fn new(local: PeerId) -> Self {
        Self {
            local,
            links: BTreeMap::new(),
            routes: BTreeMap::new(),
            inbox: vec![],
            admission: vec![],
        }
    }
    fn peer(&self, id: EndpointId) -> Option<PeerId> {
        self.routes
            .iter()
            .find_map(|(peer, route)| (*route == id).then_some(*peer))
    }
    fn current(&self, id: EndpointId, connection: usize) -> bool {
        self.links
            .get(&id)
            .is_some_and(|link| link.stable_id() == connection)
    }
}
impl SocketIo for IrohIo {
    type Address = EndpointId;
    const MTU: usize = GAME_MTU;
    fn raw_send(&self, address: EndpointId, bytes: &[u8]) -> io::Result<usize> {
        if let Some(link) = self.links.get(&address) {
            if bytes.len() > GAME_MTU {
                return Err(io::Error::other("Iroh packet exceeds game MTU"));
            }
            // A peer can close between polls. Its close event handles that peer independently.
            if link.send_datagram(Bytes::copy_from_slice(bytes)).is_err() {
                link.close(1u32.into(), b"Datagram route unavailable");
            }
        }
        Ok(bytes.len())
    }
    fn take_unadmitted(&mut self) -> Vec<(EndpointId, Vec<u8>)> {
        std::mem::take(&mut self.admission)
    }
    fn set_local(&mut self, local: PeerId) {
        self.local = local;
    }
    fn add_route(&mut self, peer: PeerId, address: EndpointId) -> Result<()> {
        if peer == self.local || self.peer(address).is_some() || !self.links.contains_key(&address)
        {
            return Err(Error::Conflict);
        }
        self.routes.insert(peer, address);
        Ok(())
    }
    fn remove_route(&mut self, peer: PeerId) {
        if let Some(id) = self.routes.remove(&peer)
            && let Some(link) = self.links.remove(&id)
        {
            link.close(0u32.into(), b"Left lobby");
        }
    }
    fn route(&self, peer: PeerId) -> Option<EndpointId> {
        self.routes.get(&peer).copied()
    }
}
impl DatagramIo for IrohIo {
    fn send(&mut self, _: u64, packet: Datagram) -> Result<()> {
        if packet.from != self.local {
            return Err(Error::Unauthorized);
        }
        let id = self.route(packet.to).ok_or(Error::Unauthorized)?;
        self.raw_send(id, &packet.bytes)
            .map_err(|e| Error::Io(e.to_string()))?;
        Ok(())
    }
    fn receive(&mut self, _: u64, local: PeerId) -> Vec<Datagram> {
        let mut delivered = Vec::new();
        for (id, bytes) in std::mem::take(&mut self.inbox) {
            if !self.links.contains_key(&id) {
                continue;
            }
            if bytes.starts_with(b"OAH2") {
                if self.admission.len() < 32 {
                    self.admission.push((id, bytes.to_vec()));
                }
            } else if let Some(from) = self.peer(id) {
                delivered.push(Datagram {
                    from,
                    to: local,
                    bytes: bytes.to_vec(),
                });
            }
        }
        delivered
    }
}

#[cfg(all(test, feature = "real-sockets"))]
mod tests;
