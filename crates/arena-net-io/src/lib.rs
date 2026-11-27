//! Replaceable packet and socket adapters. No game physics or renderer.
mod packet;
pub mod udp;
pub use packet::LinkEndpoint;

pub mod ip;
pub mod native;
pub mod session;
