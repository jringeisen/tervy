//! DevDesk core: domain model, config, persistence, the supervisor that owns
//! every running session, and the agent providers.
//!
//! Plain synchronous Rust (threads + channels). It never depends on GPUI
//! (architecture.md §2–§3).
//!
pub mod config;
pub mod env;
pub mod instance;
pub mod supervisor;
