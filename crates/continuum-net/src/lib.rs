//! Discovery, transport and crypto between Continuum peers.

pub mod error;
pub mod identity;
pub mod noise;
pub mod peer;

pub use error::NetError;
pub use identity::{default_identity_path, device_id_from_public, Identity};
pub use noise::Session;
pub use peer::Peer;
