//! Transport-independent authoritative multiplayer contracts. No Bevy, sockets or renderer.
pub mod clock;
pub mod codec;
pub mod input;
pub mod lobby;
pub mod native_input;
pub mod prediction;
pub mod session;
pub mod snapshot;
pub mod transport;

pub type Tick = u64;
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PeerId(pub u64);
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BodyId(pub u32);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SessionId(pub u128);
pub const HOST: PeerId = PeerId(0);
pub const HZ: u64 = 120;
pub const DT: f32 = 1. / HZ as f32;
pub const MAX_PLAYERS: usize = 4;
pub const MAX_BODIES: usize = 128;
pub const HISTORY: usize = 360;
pub const PROTOCOL: u16 = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    Malformed,
    Limit,
    Version,
    Unauthorized,
    Phase,
    Conflict,
    MissingHistory,
    Incompatible,
    Hash,
    Io(String),
}
pub type Result<T> = std::result::Result<T, Error>;
