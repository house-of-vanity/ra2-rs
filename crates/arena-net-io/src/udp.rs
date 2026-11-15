//! Non-blocking direct-IP IO and separate IPv4 LAN discovery. No game rules here.
use arena_net::{transport::*, *};
use std::{
    collections::BTreeMap,
    io,
    net::{Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket},
};
pub struct UdpIo {
    socket: UdpSocket,
    local: PeerId,
    routes: BTreeMap<PeerId, SocketAddr>,
    unadmitted: Vec<(SocketAddr, Vec<u8>)>,
    pub rejected: u64,
}
impl UdpIo {
    pub fn bind(local: PeerId, address: SocketAddr) -> io::Result<Self> {
        let socket = UdpSocket::bind(address)?;
        socket.set_nonblocking(true)?;
        Ok(Self {
            socket,
            local,
            routes: BTreeMap::new(),
            unadmitted: vec![],
            rejected: 0,
        })
    }
    pub fn address(&self) -> io::Result<SocketAddr> {
        self.socket.local_addr()
    }
    pub fn set_local(&mut self, local: PeerId) {
        self.local = local;
    }
    pub fn take_unadmitted(&mut self) -> Vec<(SocketAddr, Vec<u8>)> {
        std::mem::take(&mut self.unadmitted)
    }
    pub fn raw_send(&self, address: SocketAddr, bytes: &[u8]) -> io::Result<usize> {
        self.socket.send_to(bytes, address)
    }
    pub fn remove_route(&mut self, peer: PeerId) {
        self.routes.remove(&peer);
    }
    pub fn route(&self, peer: PeerId) -> Option<SocketAddr> {
        self.routes.get(&peer).copied()
    }
    /// Admission/identity belongs to the session handshake, never to untrusted packet bytes.
    pub fn add_route(&mut self, peer: PeerId, address: SocketAddr) -> Result<()> {
        if peer == self.local
            || self.routes.len() >= MAX_PLAYERS
            || self.routes.values().any(|a| *a == address)
        {
            return Err(Error::Conflict);
        }
        self.routes.insert(peer, address);
        Ok(())
    }
}
impl DatagramIo for UdpIo {
    fn send(&mut self, _now: u64, packet: Datagram) -> Result<()> {
        if packet.from != self.local {
            return Err(Error::Unauthorized);
        }
        if packet.bytes.len() > MTU {
            return Err(Error::Limit);
        }
        let address = self.routes.get(&packet.to).ok_or(Error::Unauthorized)?;
        match self.socket.send_to(&packet.bytes, address) {
            Ok(n) if n == packet.bytes.len() => Ok(()),
            Ok(_) => Err(Error::Io("partial UDP datagram".into())),
            Err(e)
                if e.kind() == io::ErrorKind::WouldBlock
                    || e.kind() == io::ErrorKind::Interrupted =>
            {
                self.rejected += 1;
                Ok(())
            }
            Err(e) => Err(Error::Io(e.to_string())),
        }
    }
    fn receive(&mut self, _now: u64, peer: PeerId) -> Vec<Datagram> {
        if peer != self.local {
            return vec![];
        }
        let mut out = vec![];
        let mut buf = [0u8; MTU + 1];
        for _ in 0..256 {
            match self.socket.recv_from(&mut buf) {
                Ok((n, address)) => {
                    if n > MTU {
                        self.rejected += 1;
                        continue;
                    }
                    if buf[..n].starts_with(b"OAH2") || !self.routes.values().any(|a| *a == address)
                    {
                        if self.unadmitted.len() < 32 {
                            self.unadmitted.push((address, buf[..n].to_vec()));
                        }
                    } else if let Some((&from, _)) =
                        self.routes.iter().find(|(_, a)| **a == address)
                    {
                        out.push(Datagram {
                            from,
                            to: self.local,
                            bytes: buf[..n].to_vec(),
                        })
                    } else {
                        self.rejected += 1;
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(_) => {
                    self.rejected += 1;
                    break;
                }
            }
        }
        out
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Advertisement {
    pub name: String,
    pub port: u16,
    pub occupied: u8,
    pub capacity: u8,
    pub nonce: u64,
}
impl Advertisement {
    pub fn encode(&self) -> Result<Vec<u8>> {
        if self.name.len() > 96
            || self.capacity > 4
            || self.capacity < 2
            || self.occupied > self.capacity
            || self.port == 0
        {
            return Err(Error::Limit);
        }
        let mut b = b"OAL1".to_vec();
        b.extend(PROTOCOL.to_le_bytes());
        b.extend(self.nonce.to_le_bytes());
        b.extend(self.port.to_le_bytes());
        b.extend([self.occupied, self.capacity, self.name.len() as u8]);
        b.extend(self.name.as_bytes());
        Ok(b)
    }
    pub fn decode(b: &[u8], nonce: u64) -> Result<Self> {
        if b.len() < 19 || &b[..4] != b"OAL1" {
            return Err(Error::Malformed);
        }
        if u16::from_le_bytes(b[4..6].try_into().unwrap()) != PROTOCOL {
            return Err(Error::Version);
        }
        if u64::from_le_bytes(b[6..14].try_into().unwrap()) != nonce {
            return Err(Error::Conflict);
        }
        if b.len() != 19 + b[18] as usize {
            return Err(Error::Malformed);
        }
        let a = Self {
            name: std::str::from_utf8(&b[19..])
                .map_err(|_| Error::Malformed)?
                .into(),
            port: u16::from_le_bytes(b[14..16].try_into().unwrap()),
            occupied: b[16],
            capacity: b[17],
            nonce,
        };
        a.encode()?;
        Ok(a)
    }
}
pub struct Discovery {
    socket: UdpSocket,
}
impl Discovery {
    pub fn bind(port: u16) -> io::Result<Self> {
        let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, port))?;
        socket.set_broadcast(true)?;
        socket.set_nonblocking(true)?;
        Ok(Self { socket })
    }
    pub fn address(&self) -> io::Result<SocketAddr> {
        self.socket.local_addr()
    }
    pub fn search(&self, port: u16, nonce: u64) -> io::Result<usize> {
        self.query(SocketAddrV4::new(Ipv4Addr::BROADCAST, port).into(), nonce)
    }
    pub fn query(&self, address: SocketAddr, nonce: u64) -> io::Result<usize> {
        let mut b = b"OAQ1".to_vec();
        b.extend(PROTOCOL.to_le_bytes());
        b.extend(nonce.to_le_bytes());
        self.socket.send_to(&b, address)
    }
    pub fn answer(&self, ad: &Advertisement, in_lobby: bool) -> Result<usize> {
        let mut count = 0;
        let mut buf = [0u8; 128];
        for _ in 0..16 {
            match self.socket.recv_from(&mut buf) {
                Ok((n, address)) => {
                    if !in_lobby
                        || n != 14
                        || &buf[..4] != b"OAQ1"
                        || u16::from_le_bytes(buf[4..6].try_into().unwrap()) != PROTOCOL
                    {
                        continue;
                    }
                    let mut response = ad.clone();
                    response.nonce = u64::from_le_bytes(buf[6..14].try_into().unwrap());
                    self.socket
                        .send_to(&response.encode()?, address)
                        .map_err(|e| Error::Io(e.to_string()))?;
                    count += 1;
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) => return Err(Error::Io(e.to_string())),
            }
        }
        Ok(count)
    }
    pub fn results(&self, nonce: u64) -> Vec<(SocketAddr, Advertisement)> {
        let mut out = vec![];
        let mut b = [0u8; 128];
        for _ in 0..64 {
            match self.socket.recv_from(&mut b) {
                Ok((n, address)) => {
                    if let Ok(ad) = Advertisement::decode(&b[..n], nonce) {
                        out.push((SocketAddr::new(address.ip(), ad.port), ad))
                    }
                }
                Err(_) => break,
            }
        }
        out
    }
}
