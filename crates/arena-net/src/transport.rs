//! IO boundary below packet reliability. Implementations can be UDP, iroh or a virtual wire.
use crate::*;
pub const MTU: usize = 1200;
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Datagram {
    pub from: PeerId,
    pub to: PeerId,
    pub bytes: Vec<u8>,
}
pub trait DatagramIo {
    fn send(&mut self, now_us: u64, packet: Datagram) -> Result<()>;
    fn receive(&mut self, now_us: u64, peer: PeerId) -> Vec<Datagram>;
}

#[derive(Clone, Debug)]
pub struct NetworkProfile {
    pub name: String,
    pub upstream_ms: u32,
    pub downstream_ms: u32,
    pub jitter_ms: u32,
    pub loss_per_10000: u32,
    pub duplicate_per_10000: u32,
    /// Per direction, per peer, including packet headers. Zero means unlimited.
    pub bytes_per_second: u64,
    pub blackout: Option<(u64, u64)>,
}
impl NetworkProfile {
    pub fn ideal() -> Self {
        Self {
            name: "ideal".into(),
            upstream_ms: 0,
            downstream_ms: 0,
            jitter_ms: 0,
            loss_per_10000: 0,
            duplicate_per_10000: 0,
            bytes_per_second: 0,
            blackout: None,
        }
    }
    pub fn rtt_ms(&self) -> u32 {
        self.upstream_ms + self.downstream_ms
    }
    /// Prediction runs ahead of host by a bounded lead; this is an explicit negotiated parameter.
    pub fn lead_ticks(&self) -> u64 {
        ((self.upstream_ms + self.jitter_ms) as u64 * HZ).div_ceil(1000) + 4
    }
    pub fn render_delay_ticks(&self) -> u64 {
        ((self.downstream_ms + self.jitter_ms) as u64 * HZ).div_ceil(1000) + 8
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WireMetrics {
    pub packets: u64,
    pub bytes: u64,
    pub dropped: u64,
    pub duplicated: u64,
    pub delivered: u64,
    pub peak_queue: usize,
}
pub struct VirtualWire {
    pub profile: NetworkProfile,
    pub metrics: WireMetrics,
    rng: u64,
    sequence: u64,
    queue: std::collections::BTreeMap<(u64, u64), Datagram>,
    next: std::collections::BTreeMap<(PeerId, PeerId), u64>,
}
impl VirtualWire {
    pub fn new(profile: NetworkProfile, seed: u64) -> Self {
        Self {
            profile,
            metrics: Default::default(),
            rng: seed.max(1),
            sequence: 0,
            queue: Default::default(),
            next: Default::default(),
        }
    }
    fn random(&mut self) -> u64 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        self.rng
    }
    fn schedule(&mut self, now: u64, packet: Datagram) {
        let jitter = self.profile.jitter_ms as i64;
        let j = if jitter == 0 {
            0
        } else {
            (self.random() % (2 * jitter + 1) as u64) as i64 - jitter
        };
        let latency = if packet.to == HOST {
            self.profile.upstream_ms
        } else {
            self.profile.downstream_ms
        } as i64;
        let mut at = now + ((latency + j).max(0) as u64) * 1000;
        if self.profile.bytes_per_second > 0 {
            let next = self.next.entry((packet.from, packet.to)).or_default();
            *next = (*next).max(now)
                + (packet.bytes.len() as u64 * 1_000_000).div_ceil(self.profile.bytes_per_second);
            at = at.max(*next + (latency.max(0) as u64) * 1000);
        }
        if self.queue.len() >= 4096 {
            self.metrics.dropped += 1;
            return;
        }
        self.sequence += 1;
        self.queue.insert((at, self.sequence), packet);
        self.metrics.peak_queue = self.metrics.peak_queue.max(self.queue.len());
    }
}
impl DatagramIo for VirtualWire {
    fn send(&mut self, now: u64, packet: Datagram) -> Result<()> {
        if packet.bytes.len() > MTU {
            return Err(Error::Limit);
        }
        self.metrics.packets += 1;
        self.metrics.bytes += packet.bytes.len() as u64;
        if self
            .profile
            .blackout
            .is_some_and(|(a, b)| now >= a && now < b)
            || self.random() % 10000 < self.profile.loss_per_10000 as u64
        {
            self.metrics.dropped += 1;
            return Ok(());
        }
        if self.random() % 10000 < self.profile.duplicate_per_10000 as u64 {
            self.metrics.duplicated += 1;
            self.schedule(now, packet.clone());
        }
        self.schedule(now, packet);
        Ok(())
    }
    fn receive(&mut self, now: u64, peer: PeerId) -> Vec<Datagram> {
        let keys = self
            .queue
            .range(..=(now, u64::MAX))
            .filter(|(_, p)| p.to == peer)
            .map(|(k, _)| *k)
            .collect::<Vec<_>>();
        let packets = keys
            .iter()
            .filter_map(|k| self.queue.remove(k))
            .collect::<Vec<_>>();
        self.metrics.delivered += packets.len() as u64;
        packets
    }
}
