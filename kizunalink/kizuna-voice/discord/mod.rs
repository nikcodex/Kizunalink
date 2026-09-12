//! # Discord — Voice Gateway and Player
//!
//! Handles Discord voice connections, DAVE encryption, and player state management.
//!
//! - [`crypto`]: DAVE (Discord Audio/Video End-to-End) encryption.
//! - [`gateway`]: Voice gateway WebSocket and UDP transport.
//! - [`player`]: Player context, state, filters, and playback management.

pub mod crypto;
pub mod gateway;
pub mod player;
