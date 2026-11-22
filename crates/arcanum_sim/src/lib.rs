//! The authoritative game simulation (the host).
//!
//! `arcanum_sim` owns the canonical game state: characters, the loaded map, entity positions,
//! quest flags and combat. It runs in its own headless world with no rendering or audio.
//! In single player the host lives in the client process; in multiplayer it may be a separate
//! process or the same one.
//!
//! Communication with clients goes through the `arcanum_net` transports: the host receives
//! [`PlayerIntent`]s and sends [`LedgerDelta`]s. The client never changes shared state
//! directly.

pub mod activity;
pub mod blocking;
pub mod clock;
pub mod container;
pub mod creation;
pub mod delta;
pub mod doors;
pub mod gait;
pub mod ground;
pub mod inventory;
pub mod keys;
pub mod intent;
pub mod movement;
pub mod net_id;
pub mod pathfinding;
pub mod player;
pub mod plugin;
pub mod save;
pub mod session;
pub mod sheet;
pub mod wander;
pub mod world;

pub use activity::*;
pub use blocking::*;
pub use clock::*;
pub use container::*;
pub use creation::*;
pub use delta::*;
pub use doors::*;
pub use gait::*;
pub use ground::*;
pub use inventory::*;
pub use keys::*;
pub use intent::*;
pub use movement::*;
pub use net_id::*;
pub use pathfinding::*;
pub use player::*;
pub use plugin::*;
pub use save::*;
pub use session::*;
pub use sheet::*;
pub use wander::*;
pub use world::*;
