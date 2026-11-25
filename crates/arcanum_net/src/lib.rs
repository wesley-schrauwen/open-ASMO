//! Transports between the host ([`arcanum_sim`]) and clients.
//!
//! A transport has two ends. The [`HostTransport`] receives every player's intents and sends
//! deltas to one player; a [`ClientTransport`] sends its player's intents and receives deltas.
//! [`loopback`] connects both ends in one process, and [`LocalHost`] runs a host behind it
//! (single player, and the host of a LAN game). [`lan`] carries both ends over the LAN
//! (TCP for messages that must arrive, UDP for those that may be lost); Steam comes later.

pub mod local;
pub mod loopback;
pub mod transport;
pub mod lan;

pub use local::LocalHost;
pub use loopback::{LoopbackClient, LoopbackHost};
pub use transport::{ClientTransport, HostTransport, TransportError};
pub use lan::{LanClient, LanHost};
