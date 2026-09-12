//! # Lavalink — Protocol and Route Planning
//!
//! Implements the Lavalink v4 protocol for REST API and WebSocket communication,
//! plus IP route planning for source API rate limit avoidance.
//!
//! - [`protocol`]: Wire format, opcodes, events, tracks, stats, and session management.
//! - [`routeplanner`]: IP rotation and balancing for source APIs.

pub mod protocol;
pub mod routeplanner;
