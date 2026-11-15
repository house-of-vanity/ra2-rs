//! Lobby rules and bounded content transfer. No address, socket, ECS or GUI dependencies.
use crate::*;
use std::collections::BTreeMap;
pub type Digest = [u8; 32];
pub const MAX_BOT_BYTES: usize = 4 * 1024 * 1024;
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Manifest {
    pub protocol: u16,
    pub build: Digest,
    pub assets: Digest,
    pub physics: Digest,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Lobby,
    Playing,
    Paused,
    Finished,
}
#[derive(Clone, Debug)]
pub struct Slot {
    pub peer: PeerId,
    pub bot: Digest,
    pub ready: bool,
}
pub struct Lobby {
    pub phase: Phase,
    pub manifest: Manifest,
    pub slots: BTreeMap<PeerId, Slot>,
    bots: BTreeMap<Digest, Vec<u8>>,
}
impl Lobby {
    pub fn new(manifest: Manifest) -> Self {
        Self {
            phase: Phase::Lobby,
            manifest,
            slots: BTreeMap::new(),
            bots: BTreeMap::new(),
        }
    }
    pub fn join(&mut self, peer: PeerId, manifest: &Manifest, bot: Digest) -> Result<()> {
        if self.phase != Phase::Lobby {
            return Err(Error::Phase);
        }
        if manifest != &self.manifest || manifest.protocol != PROTOCOL {
            return Err(Error::Incompatible);
        }
        if self.slots.contains_key(&peer) {
            return Err(Error::Conflict);
        }
        if self.slots.len() == MAX_PLAYERS {
            return Err(Error::Limit);
        }
        self.slots.insert(
            peer,
            Slot {
                peer,
                bot,
                ready: false,
            },
        );
        Ok(())
    }
    pub fn install_bot(&mut self, peer: PeerId, upload: BotUpload) -> Result<()> {
        if self.phase != Phase::Lobby {
            return Err(Error::Phase);
        }
        if self.slots.get(&peer).ok_or(Error::Unauthorized)?.bot != upload.digest {
            return Err(Error::Unauthorized);
        }
        let (digest, bytes) = upload.finish()?;
        self.bots.insert(digest, bytes);
        Ok(())
    }
    pub fn ready(&mut self, peer: PeerId) -> Result<()> {
        if self.phase != Phase::Lobby {
            return Err(Error::Phase);
        }
        let slot = self.slots.get_mut(&peer).ok_or(Error::Unauthorized)?;
        if !self.bots.contains_key(&slot.bot) {
            return Err(Error::Incompatible);
        }
        slot.ready = true;
        Ok(())
    }
    pub fn start(&mut self, peer: PeerId) -> Result<()> {
        if peer != HOST {
            return Err(Error::Unauthorized);
        }
        if self.phase != Phase::Lobby
            || self.slots.len() < 2
            || !self.slots.contains_key(&HOST)
            || !self.slots.values().all(|s| s.ready)
        {
            return Err(Error::Phase);
        }
        self.phase = Phase::Playing;
        Ok(())
    }
    pub fn pause(&mut self, peer: PeerId, paused: bool) -> Result<()> {
        if peer != HOST {
            return Err(Error::Unauthorized);
        }
        if !matches!(self.phase, Phase::Playing | Phase::Paused) {
            return Err(Error::Phase);
        }
        self.phase = if paused {
            Phase::Paused
        } else {
            Phase::Playing
        };
        Ok(())
    }
    pub fn leave(&mut self, peer: PeerId) {
        self.slots.remove(&peer);
        self.bots
            .retain(|digest, _| self.slots.values().any(|slot| slot.bot == *digest));
        if peer == HOST {
            self.phase = Phase::Finished;
        }
    }
}
pub struct BotUpload {
    digest: Digest,
    bytes: Vec<u8>,
    expected: usize,
}
impl BotUpload {
    pub fn new(digest: Digest, size: usize) -> Result<Self> {
        if size == 0 || size > MAX_BOT_BYTES {
            return Err(Error::Limit);
        }
        Ok(Self {
            digest,
            bytes: Vec::new(),
            expected: size,
        })
    }
    /// The reliable bulk lane supplies ordered chunks. Only the next offset is accepted.
    pub fn chunk(&mut self, offset: usize, bytes: &[u8]) -> Result<()> {
        if offset != self.bytes.len() {
            return Err(Error::Conflict);
        }
        if bytes.len() > 1024 || bytes.is_empty() || bytes.len() > self.expected - self.bytes.len()
        {
            return Err(Error::Limit);
        }
        self.bytes.extend_from_slice(bytes);
        Ok(())
    }
    pub fn complete(&self) -> bool {
        self.bytes.len() == self.expected
    }
    pub fn finish(self) -> Result<(Digest, Vec<u8>)> {
        if self.bytes.len() != self.expected || blake3::hash(&self.bytes).as_bytes() != &self.digest
        {
            return Err(Error::Hash);
        }
        Ok((self.digest, self.bytes))
    }
}
