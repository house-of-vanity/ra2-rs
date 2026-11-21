//! Native-match session DTOs. No GUI, socket addresses, BOT parser or Bevy entities.
use crate::{
    lobby::{Digest, MAX_BOT_BYTES, Manifest},
    *,
};
use serde::{Deserialize, Serialize};
pub const GAME_PORT: u16 = 27666;
pub const DISCOVERY_PORT: u16 = 27667;
pub const TIMEOUT_US: u64 = 15_000_000;
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    pub name: String,
    pub arena: String,
    pub mode: u8,
    pub minutes: u16,
    pub hazards: bool,
    pub weight: Option<u8>,
}
impl Settings {
    pub fn validate(&self) -> Result<()> {
        if self.name.is_empty()
            || self.name.len() > 96
            || self.arena.len() > 128
            || self.arena.is_empty()
            || self.mode > 4
            || !(1..=20).contains(&self.minutes)
            || self.weight.is_some_and(|n| n > 2)
        {
            return Err(Error::Malformed);
        }
        Ok(())
    }
    pub fn minimum(&self) -> usize {
        match self.mode {
            1 => 3,
            2 => 4,
            _ => 2,
        }
    }
    pub fn capacity(&self) -> usize {
        if self.mode == 0 { 2 } else { 4 }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Player {
    pub slot: u8,
    pub alias: String,
    pub team: String,
    pub bot_name: String,
    pub digest: Digest,
    pub size: u32,
    pub ready: bool,
}
impl Player {
    pub fn validate(&self) -> Result<()> {
        if self.slot >= 4
            || self.alias.len() > 64
            || self.team.len() > 128
            || self.bot_name.len() > 128
            || self.size as usize > MAX_BOT_BYTES
        {
            return Err(Error::Limit);
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Control {
    Hello {
        nonce: u64,
        manifest: Manifest,
    },
    Welcome {
        nonce: u64,
        slot: u8,
        session: u128,
    },
    Reject {
        nonce: u64,
        reason: String,
    },
    Connected,
    Select(Player),
    Need(Digest),
    Offer {
        digest: Digest,
        size: u32,
    },
    Installed(Digest),
    TransferReady(Digest),
    Skin(Skin),
    Sounds {
        epoch: u64,
        events: Vec<(String, [f32; 3])>,
    },
    Ready(bool),
    Roster {
        settings: Settings,
        players: Vec<Player>,
        epoch: u64,
    },
    Prepare {
        epoch: u64,
    },
    Loaded {
        epoch: u64,
    },
    Begin {
        epoch: u64,
    },
    ReturnLobby {
        epoch: u64,
    },
    Pause {
        epoch: u64,
        tick: u64,
        paused: bool,
    },
    Chat {
        alias: String,
        text: String,
    },
    Ping(u64),
    Pong(u64),
    Leave(String),
}
pub fn encode<T: Serialize>(message: &T) -> Result<Vec<u8>> {
    let mut data = vec![0; crate::codec::MAX_MESSAGE - 11];
    let len = postcard::to_slice(message, &mut data)
        .map_err(|_| Error::Limit)?
        .len();
    data.truncate(len);
    if data.len() > crate::codec::MAX_MESSAGE - 11 {
        return Err(Error::Limit);
    }
    Ok(data)
}
pub fn decode<'a, T: Deserialize<'a>>(bytes: &'a [u8]) -> Result<T> {
    if bytes.len() > crate::codec::MAX_MESSAGE - 11 {
        return Err(Error::Limit);
    }
    let (value, rest) = postcard::take_from_bytes(bytes).map_err(|_| Error::Malformed)?;
    if !rest.is_empty() {
        return Err(Error::Malformed);
    }
    Ok(value)
}
impl Control {
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::Sounds { events, .. }
                if events.len() > 64
                    || events
                        .iter()
                        .any(|(p, v)| p.len() > 256 || v.iter().any(|n| !n.is_finite())) =>
            {
                return Err(Error::Limit);
            }
            Self::Select(p) => p.validate()?,
            Self::Roster {
                settings, players, ..
            } => {
                settings.validate()?;
                if players.len() > settings.capacity() {
                    return Err(Error::Limit);
                }
                let mut slots = std::collections::BTreeSet::new();
                for p in players {
                    p.validate()?;
                    if !slots.insert(p.slot) {
                        return Err(Error::Conflict);
                    }
                }
            }
            Self::Offer { size, .. } if *size == 0 || *size as usize > MAX_BOT_BYTES => {
                return Err(Error::Limit);
            }
            Self::Welcome { slot, .. } if !(1..=3).contains(slot) => {
                return Err(Error::Unauthorized);
            }
            Self::Chat { alias, text } if alias.len() > 64 || text.len() > 256 => {
                return Err(Error::Limit);
            }
            Self::Reject { reason, .. } | Self::Leave(reason) if reason.len() > 256 => {
                return Err(Error::Limit);
            }
            _ => {}
        }
        Ok(())
    }
}
/// All native controller slots, not an assumed WASD layout. Each rising edge has
/// its own monotonic counter, so packet duplication cannot fire a burst twice.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct InputCommand {
    pub sequence: u64,
    pub levels: Vec<u8>,
    pub edges: Vec<u32>,
}
impl InputCommand {
    pub fn valid(&self) -> bool {
        self.sequence > 0 && self.levels.len() <= 128 && self.edges.len() == self.levels.len()
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Wiring {
    pub epoch: u64,
    pub generation: u64,
    /// Ordered keyframes covering up to 32 preceding steps. Between keyframes
    /// levels/edge counters are constant; this avoids resending held keys 32 times.
    pub redundant: Vec<InputCommand>,
    pub sequence: u64,
    pub levels: Vec<u8>,
    pub edges: Vec<u32>,
}
impl Wiring {
    pub fn validate(&self) -> Result<()> {
        if self.sequence == 0
            || self.sequence > (1 << 48)
            || self.levels.len() > 128
            || self.edges.len() != self.levels.len()
            || self.redundant.len() > 32
            || self.redundant.iter().any(|c| {
                !c.valid()
                    || c.sequence >= self.sequence
                    || self.sequence - c.sequence > 32
                    || c.levels.len() != self.levels.len()
            })
        {
            return Err(Error::Limit);
        }
        if self
            .redundant
            .windows(2)
            .any(|p| p[0].sequence >= p[1].sequence)
        {
            return Err(Error::Conflict);
        }
        Ok(())
    }
    /// Expand bounded input runs back into one command per fixed physics step.
    pub fn commands(&self) -> Result<Vec<InputCommand>> {
        self.validate()?;
        let latest = InputCommand {
            sequence: self.sequence,
            levels: self.levels.clone(),
            edges: self.edges.clone(),
        };
        let keys = self
            .redundant
            .iter()
            .chain(std::iter::once(&latest))
            .collect::<Vec<_>>();
        let mut result = vec![];
        for (i, key) in keys.iter().enumerate() {
            let end = keys
                .get(i + 1)
                .map_or(self.sequence + 1, |next| next.sequence);
            for sequence in key.sequence..end {
                let mut c = (*key).clone();
                c.sequence = sequence;
                result.push(c);
            }
        }
        Ok(result)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Pose {
    pub id: u32,
    pub position: [f32; 3],
    pub rotation: [f32; 4],
    pub linear: [f32; 3],
    pub angular: [f32; 3],
}
impl Pose {
    pub fn validate(&self) -> Result<()> {
        if self
            .position
            .iter()
            .chain(&self.rotation)
            .chain(&self.linear)
            .chain(&self.angular)
            .any(|n| !n.is_finite())
            || self.position.iter().any(|n| n.abs() > 1_000_000.)
            || (self.rotation.iter().map(|n| n * n).sum::<f32>() - 1.).abs() > 0.01
        {
            return Err(Error::Malformed);
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Robot {
    pub slot: u8,
    pub hp: Vec<f32>,
    pub energy: f32,
    pub air: f32,
    pub motors: Vec<[f32; 4]>,
    pub commands: Vec<[f32; 3]>,
    pub detached: Vec<u16>,
    /// Damage numbers retain authoritative age/source; snapshots do not reapply HP.
    pub numbers: Vec<Number>,
    pub shell_revision: u64,
    pub controls: arena_core::mechanism::ControlsSnapshot,
    pub acknowledged: u64,
    pub edges: Vec<u32>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Number {
    pub hp: f32,
    pub position: [f32; 3],
    pub age: f32,
    pub source: Option<u32>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Frame {
    pub epoch: u64,
    pub input_generation: u64,
    pub tick: u64,
    pub phase: u8,
    pub phase_time: f32,
    pub paused: bool,
    pub remaining: f32,
    pub scores: [f32; 4],
    pub eliminated: [bool; 4],
    pub immobility_countdown: [u8; 4],
    pub counted_out: [bool; 4],
    pub winners: [bool; 4],
    pub bodies: Vec<Pose>,
    pub robots: Vec<Robot>,
    pub traps: Vec<Trap>,
}
impl Frame {
    pub fn validate(&self) -> Result<()> {
        if self.robots.len() > 4
            || self.bodies.len() > 512
            || self.traps.len() > 128
            || self
                .immobility_countdown
                .iter()
                .any(|&seconds| seconds > 10)
            || self.phase > 3
            || self.tick > (1 << 48)
            || !self.phase_time.is_finite()
            || !self.remaining.is_finite()
            || self.scores.iter().any(|n| !n.is_finite() || *n < 0.)
        {
            return Err(Error::Limit);
        }
        let mut ids = std::collections::BTreeSet::new();
        for b in &self.bodies {
            b.validate()?;
            if !ids.insert(b.id) {
                return Err(Error::Conflict);
            }
        }
        let mut slots = std::collections::BTreeSet::new();
        for r in &self.robots {
            if !r.controls.valid()
                || r.edges.len() > 128
                || r.slot >= 4
                || !slots.insert(r.slot)
                || r.hp.len() > 256
                || r.motors.len() > 128
                || r.commands.len() > 256
                || r.commands.iter().flatten().any(|n| !n.is_finite())
                || r.detached.len() > 256
                || r.numbers.len() > 32
                || r.hp
                    .iter()
                    .chain([&r.energy, &r.air])
                    .any(|n| !n.is_finite() || *n < 0.)
                || r.motors.iter().flatten().any(|n| !n.is_finite())
            {
                return Err(Error::Limit);
            }
            for n in &r.numbers {
                if !n.hp.is_finite()
                    || n.hp < 0.
                    || !n.age.is_finite()
                    || n.age < 0.
                    || n.position.iter().any(|p| !p.is_finite())
                {
                    return Err(Error::Malformed);
                }
            }
        }
        for trap in &self.traps {
            if !trap.clock.is_finite() || trap.zapping.len() > 4 {
                return Err(Error::Malformed);
            }
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Trap {
    pub index: u16,
    pub clock: f32,
    pub active: bool,
    pub occupied: bool,
    pub zapping: Vec<u32>,
}
/// A reliable, complete mesh checkpoint is sent only after a new dent revision.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Skin {
    pub epoch: u64,
    pub slot: u8,
    pub revision: u64,
    pub positions: Vec<[f32; 3]>,
}
