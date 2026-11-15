//! Same Connection::poll admission/packet implementation as the UDP service, on an in-memory socket.
use arena_net::{lobby::Manifest, session::*, transport::*, *};
use arena_net_io::{
    ip::{Connection, SocketIo},
    session::{Notice, Request, Stage},
};
use std::{
    cell::RefCell,
    collections::BTreeMap,
    net::{Ipv4Addr, SocketAddr},
    rc::Rc,
};
struct Bus {
    wire: VirtualWire,
    now: u64,
}
struct MemorySocket {
    bus: Rc<RefCell<Bus>>,
    socket: u16,
    local: PeerId,
    routes: BTreeMap<PeerId, SocketAddr>,
    unknown: Vec<(SocketAddr, Vec<u8>)>,
}
fn addr(n: u16) -> SocketAddr {
    (Ipv4Addr::LOCALHOST, 20000 + n).into()
}
impl MemorySocket {
    fn new(bus: Rc<RefCell<Bus>>, socket: u16) -> Self {
        Self {
            bus,
            socket,
            local: PeerId(socket as u64),
            routes: BTreeMap::new(),
            unknown: vec![],
        }
    }
}
impl DatagramIo for MemorySocket {
    fn send(&mut self, now: u64, p: Datagram) -> Result<()> {
        assert_eq!(p.from, self.local);
        let to = self.routes.get(&p.to).ok_or(Error::Unauthorized)?.port() - 20000;
        self.bus.borrow_mut().wire.send(
            now,
            Datagram {
                from: PeerId(self.socket as u64),
                to: PeerId(to as u64),
                bytes: p.bytes,
            },
        )
    }
    fn receive(&mut self, now: u64, _: PeerId) -> Vec<Datagram> {
        let ps = self
            .bus
            .borrow_mut()
            .wire
            .receive(now, PeerId(self.socket as u64));
        let mut out = vec![];
        for p in ps {
            let a = addr(p.from.0 as u16);
            if p.bytes.starts_with(b"OAH2") || !self.routes.values().any(|v| *v == a) {
                self.unknown.push((a, p.bytes));
            } else {
                let from = *self.routes.iter().find(|(_, v)| **v == a).unwrap().0;
                out.push(Datagram {
                    from,
                    to: self.local,
                    bytes: p.bytes,
                });
            }
        }
        out
    }
}
impl SocketIo for MemorySocket {
    fn address(&self) -> std::io::Result<SocketAddr> {
        Ok(addr(self.socket))
    }
    fn raw_send(&self, a: SocketAddr, b: &[u8]) -> std::io::Result<usize> {
        let mut bus = self.bus.borrow_mut();
        let now = bus.now;
        bus.wire
            .send(
                now,
                Datagram {
                    from: PeerId(self.socket as u64),
                    to: PeerId((a.port() - 20000) as u64),
                    bytes: b.to_vec(),
                },
            )
            .unwrap();
        Ok(b.len())
    }
    fn take_unadmitted(&mut self) -> Vec<(SocketAddr, Vec<u8>)> {
        std::mem::take(&mut self.unknown)
    }
    fn set_local(&mut self, p: PeerId) {
        self.local = p;
    }
    fn add_route(&mut self, p: PeerId, a: SocketAddr) -> Result<()> {
        self.routes.insert(p, a);
        Ok(())
    }
    fn remove_route(&mut self, p: PeerId) {
        self.routes.remove(&p);
    }
    fn route(&self, p: PeerId) -> Option<SocketAddr> {
        self.routes.get(&p).copied()
    }
}
fn manifest() -> Manifest {
    Manifest {
        protocol: PROTOCOL,
        build: [1; 32],
        assets: [2; 32],
        physics: [3; 32],
    }
}
fn settings() -> Settings {
    Settings {
        name: "IP test".into(),
        arena: "box".into(),
        mode: 0,
        minutes: 5,
        hazards: false,
        weight: None,
    }
}
#[test]
fn ip_handshake_retries_loss_checks_assets_and_releases_a_disconnected_lobby_slot() {
    let mut p = NetworkProfile::ideal();
    p.upstream_ms = 100;
    p.downstream_ms = 100;
    p.jitter_ms = 20;
    p.loss_per_10000 = 1000;
    p.duplicate_per_10000 = 500;
    let bus = Rc::new(RefCell::new(Bus {
        wire: VirtualWire::new(p, 41),
        now: 0,
    }));
    let mut host =
        Connection::host_with_io(settings(), manifest(), MemorySocket::new(bus.clone(), 0))
            .unwrap();
    let mut client = Connection::client_with_io(
        settings(),
        manifest(),
        MemorySocket::new(bus.clone(), 1),
        addr(0),
    )
    .unwrap();
    for t in 1..=1200 {
        let now = t * 1_000_000 / HZ;
        bus.borrow_mut().now = now;
        host.poll(now).unwrap();
        client.poll(now).unwrap();
    }
    assert_eq!(host.node.links.len(), 1);
    assert_eq!(client.node.local, PeerId(1));
    assert!(
        client
            .node
            .notices
            .iter()
            .any(|n| matches!(n, Notice::Connected(1)))
    );
    assert_eq!(client.node.players.len(), 2);
    // A third user is rejected as full, without mutating either admitted endpoint.
    let mut extra = Connection::client_with_io(
        settings(),
        manifest(),
        MemorySocket::new(bus.clone(), 2),
        addr(0),
    )
    .unwrap();
    for t in 1201..=1440 {
        let now = t * 1_000_000 / HZ;
        bus.borrow_mut().now = now;
        host.poll(now).unwrap();
        client.poll(now).unwrap();
        extra.poll(now).unwrap();
    }
    assert_eq!(extra.node.stage, Stage::Closed);
    assert_eq!(host.node.players.len(), 2);
    client.node.request(Request::Leave).unwrap();
    for t in 1441..=1560 {
        let now = t * 1_000_000 / HZ;
        bus.borrow_mut().now = now;
        host.poll(now).unwrap();
        client.poll(now).unwrap();
    }
    assert_eq!(host.node.players.len(), 1);
    // A new socket address reuses canonical player slot 1, while old session packets are rejected.
    let mut replacement = Connection::client_with_io(
        settings(),
        manifest(),
        MemorySocket::new(bus.clone(), 3),
        addr(0),
    )
    .unwrap();
    for t in 1561..=1800 {
        let now = t * 1_000_000 / HZ;
        bus.borrow_mut().now = now;
        host.poll(now).unwrap();
        replacement.poll(now).unwrap();
    }
    assert_eq!(replacement.node.local, PeerId(1));
    assert_eq!(replacement.node.players.len(), 2);
    host.node.drop_peer(PeerId(1), "Test leave").unwrap();
    let mut wrong = manifest();
    wrong.assets[0] ^= 1;
    let mut incompatible = Connection::client_with_io(
        settings(),
        wrong,
        MemorySocket::new(bus.clone(), 4),
        addr(0),
    )
    .unwrap();
    for t in 1801..=2040 {
        let now = t * 1_000_000 / HZ;
        bus.borrow_mut().now = now;
        host.poll(now).unwrap();
        incompatible.poll(now).unwrap();
    }
    assert_eq!(incompatible.node.stage, Stage::Closed);
    assert!(
        incompatible
            .node
            .notices
            .iter()
            .any(|n| matches!(n,Notice::Disconnected(reason)if reason.contains("Incompatible")))
    );
}
