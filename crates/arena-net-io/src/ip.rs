//! Shared session admission over addressed datagrams, plus direct-IP and LAN setup.
use crate::{
    session::{Node, Notice, Stage},
    udp::{Advertisement, Discovery, UdpIo},
};
use arena_net::{lobby::Manifest, session::*, *};
use std::{
    collections::BTreeMap,
    hash::{BuildHasher, Hasher},
    net::{IpAddr, Ipv4Addr, SocketAddr},
};
pub fn nonce() -> u64 {
    std::collections::hash_map::RandomState::new()
        .build_hasher()
        .finish()
}
/// The address is a UDP socket address or an authenticated iroh endpoint ID.
pub trait SocketIo: arena_net::transport::DatagramIo {
    type Address: Copy + Ord;
    const MTU: usize = arena_net::transport::MTU;
    fn raw_send(&self, address: Self::Address, bytes: &[u8]) -> std::io::Result<usize>;
    fn take_unadmitted(&mut self) -> Vec<(Self::Address, Vec<u8>)>;
    fn set_local(&mut self, local: PeerId);
    fn add_route(&mut self, peer: PeerId, address: Self::Address) -> Result<()>;
    fn remove_route(&mut self, peer: PeerId);
    fn route(&self, peer: PeerId) -> Option<Self::Address>;
}
impl SocketIo for UdpIo {
    type Address = SocketAddr;
    fn raw_send(&self, a: SocketAddr, b: &[u8]) -> std::io::Result<usize> {
        UdpIo::raw_send(self, a, b)
    }
    fn take_unadmitted(&mut self) -> Vec<(SocketAddr, Vec<u8>)> {
        UdpIo::take_unadmitted(self)
    }
    fn set_local(&mut self, p: PeerId) {
        UdpIo::set_local(self, p)
    }
    fn add_route(&mut self, p: PeerId, a: SocketAddr) -> Result<()> {
        UdpIo::add_route(self, p, a)
    }
    fn remove_route(&mut self, p: PeerId) {
        UdpIo::remove_route(self, p)
    }
    fn route(&self, p: PeerId) -> Option<SocketAddr> {
        UdpIo::route(self, p)
    }
}
pub struct Connection<T: SocketIo = UdpIo> {
    pub node: Node,
    pub io: T,
    manifest: Manifest,
    remote: Option<T::Address>,
    nonce: u64,
    started: u64,
    last_hello: u64,
    discovery: Option<(Discovery, u16)>,
    admissions: BTreeMap<T::Address, (u64, u8, SessionId)>,
}
fn raw(c: &Control) -> Result<Vec<u8>> {
    c.validate()?;
    let mut b = b"OAH2".to_vec();
    b.extend(PROTOCOL.to_le_bytes());
    b.extend(encode(c)?);
    if b.len() > 1200 {
        return Err(Error::Limit);
    }
    Ok(b)
}
fn parse(b: &[u8]) -> Result<Control> {
    if b.len() < 6 || b.len() > 1200 || &b[..4] != b"OAH2" {
        return Err(Error::Malformed);
    }
    if u16::from_le_bytes(b[4..6].try_into().unwrap()) != PROTOCOL {
        return Err(Error::Version);
    }
    let c: Control = decode(&b[6..])?;
    c.validate()?;
    Ok(c)
}
fn io_error(e: std::io::Error) -> Error {
    Error::Io(e.to_string())
}
impl Connection<UdpIo> {
    pub fn host(settings: Settings, manifest: Manifest, port: u16) -> Result<Self> {
        let io = UdpIo::bind(HOST, (Ipv4Addr::UNSPECIFIED, port).into()).map_err(io_error)?;
        let port = io.address().map_err(io_error)?.port();
        let mut connection = Self::host_with_io(settings, manifest, io)?;
        connection.discovery = Some((Discovery::bind(DISCOVERY_PORT).map_err(io_error)?, port));
        Ok(connection)
    }
    pub fn connect(settings: Settings, manifest: Manifest, remote: SocketAddr) -> Result<Self> {
        let bind = SocketAddr::new(
            if remote.is_ipv6() {
                IpAddr::V6(std::net::Ipv6Addr::UNSPECIFIED)
            } else {
                IpAddr::V4(Ipv4Addr::UNSPECIFIED)
            },
            0,
        );
        let io = UdpIo::bind(PeerId(1), bind).map_err(io_error)?;
        Self::client_with_io(settings, manifest, io, remote)
    }
}
impl<T: SocketIo> Connection<T> {
    pub fn host_with_io(settings: Settings, manifest: Manifest, io: T) -> Result<Self> {
        Self::new(settings, manifest, io, HOST, None)
    }
    pub fn client_with_io(
        settings: Settings,
        manifest: Manifest,
        io: T,
        remote: T::Address,
    ) -> Result<Self> {
        Self::new(settings, manifest, io, PeerId(1), Some(remote))
    }
    pub(crate) fn new(
        settings: Settings,
        manifest: Manifest,
        io: T,
        local: PeerId,
        remote: Option<T::Address>,
    ) -> Result<Self> {
        Ok(Self {
            node: Node::new(local, settings)?,
            io,
            manifest,
            remote,
            nonce: nonce(),
            started: 0,
            last_hello: 0,
            discovery: None,
            admissions: BTreeMap::new(),
        })
    }
    pub(crate) fn set_remote(&mut self, remote: T::Address) {
        self.remote = Some(remote);
    }
    pub fn poll(&mut self, now: u64) -> Result<()> {
        if self.remote.is_some() && self.started == 0 {
            self.started = now.max(1);
        }
        if let Some(address) = self.remote
            && self.node.links.is_empty()
            && self.node.stage != Stage::Closed
        {
            if now.saturating_sub(self.started) > TIMEOUT_US {
                self.node.stage = Stage::Closed;
                self.node
                    .notices
                    .push_back(Notice::Disconnected("Connection timed out".into()));
            } else if self.last_hello == 0 || now.saturating_sub(self.last_hello) >= 500_000 {
                self.io
                    .raw_send(
                        address,
                        &raw(&Control::Hello {
                            nonce: self.nonce,
                            manifest: self.manifest.clone(),
                        })?,
                    )
                    .map_err(io_error)?;
                self.last_hello = now;
            }
        }
        self.node.pump(now, &mut self.io)?;
        for (address, bytes) in self.io.take_unadmitted() {
            let Ok(c) = parse(&bytes) else {
                continue;
            };
            match c {
                Control::Hello { nonce, manifest } if self.node.local == HOST => {
                    if let Some((old, slot, session)) = self.admissions.get(&address)
                        && *old == nonce
                        && self.node.links.contains_key(&PeerId(*slot as u64))
                    {
                        self.io
                            .raw_send(
                                address,
                                &raw(&Control::Welcome {
                                    nonce,
                                    slot: *slot,
                                    session: session.0,
                                })?,
                            )
                            .map_err(io_error)?;
                        continue;
                    }
                    let reject = if manifest != self.manifest {
                        Some("Incompatible engine, game assets or physics settings")
                    } else if self.node.stage != Stage::Lobby {
                        Some("The match has already started")
                    } else if self.node.players.len() >= self.node.settings.capacity() {
                        Some("The lobby is full")
                    } else {
                        None
                    };
                    if let Some(reason) = reject {
                        self.io
                            .raw_send(
                                address,
                                &raw(&Control::Reject {
                                    nonce,
                                    reason: reason.into(),
                                })?,
                            )
                            .map_err(io_error)?;
                        continue;
                    }
                    if self.admissions.contains_key(&address) {
                        self.io
                            .raw_send(
                                address,
                                &raw(&Control::Reject {
                                    nonce,
                                    reason: "Already connected from this address".into(),
                                })?,
                            )
                            .map_err(io_error)?;
                        continue;
                    }
                    let slot = (1..self.node.settings.capacity() as u8)
                        .find(|s| !self.node.players.contains_key(s))
                        .ok_or(Error::Limit)?;
                    let session = SessionId(
                        ((crate::ip::nonce() as u128) << 64) | crate::ip::nonce() as u128,
                    );
                    self.io.add_route(PeerId(slot as u64), address)?;
                    self.node
                        .add_peer_with_mtu(PeerId(slot as u64), session, now, T::MTU)?;
                    self.admissions.insert(address, (nonce, slot, session));
                    self.io
                        .raw_send(
                            address,
                            &raw(&Control::Welcome {
                                nonce,
                                slot,
                                session: session.0,
                            })?,
                        )
                        .map_err(io_error)?;
                }
                Control::Welcome {
                    nonce,
                    slot,
                    session,
                } if self.remote == Some(address)
                    && nonce == self.nonce
                    && self.node.links.is_empty() =>
                {
                    self.node.local = PeerId(slot as u64);
                    self.io.set_local(self.node.local);
                    self.io.add_route(HOST, address)?;
                    self.node
                        .add_peer_with_mtu(HOST, SessionId(session), now, T::MTU)?;
                }
                Control::Reject { nonce, reason }
                    if self.remote == Some(address) && nonce == self.nonce =>
                {
                    self.node.stage = Stage::Closed;
                    self.node.notices.push_back(Notice::Disconnected(reason));
                }
                _ => {}
            }
        }
        // Remove routes promptly, so a vacated lobby slot can reconnect from a new address.
        for slot in 1..4 {
            if !self.node.links.contains_key(&PeerId(slot)) && self.node.local == HOST {
                self.io.remove_route(PeerId(slot));
            }
        }
        self.admissions.retain(|address, (_, slot, _)| {
            self.node.links.contains_key(&PeerId(*slot as u64))
                && self.io.route(PeerId(*slot as u64)) == Some(*address)
        });
        if let Some((d, port)) = &self.discovery {
            let _ = d.answer(
                &Advertisement {
                    name: self.node.settings.name.clone(),
                    port: *port,
                    occupied: self.node.players.len() as u8,
                    capacity: self.node.settings.capacity() as u8,
                    nonce: 0,
                },
                self.node.stage == Stage::Lobby,
            )?;
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn admission_is_bounded_versioned_and_nonce_checked_before_session() {
        let c = Control::Welcome {
            nonce: 7,
            slot: 1,
            session: 9,
        };
        let b = raw(&c).unwrap();
        assert_eq!(parse(&b).unwrap(), c);
        for n in 0..b.len() {
            assert!(parse(&b[..n]).is_err());
        }
        let mut b = b;
        b[4] = 99;
        assert_eq!(parse(&b), Err(Error::Version));
    }
}
