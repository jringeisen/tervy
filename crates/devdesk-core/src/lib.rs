//! DevDesk core: domain model, config, persistence, the supervisor that owns
//! every running session, and the agent providers.
//!
//! Plain synchronous Rust (threads + channels). It never depends on GPUI
//! (architecture.md §2–§3).
//!
pub mod config;
pub mod env;
pub mod instance;
pub mod process;
pub mod store;
pub mod supervisor;

pub mod agent;
pub mod hook;
pub mod ipc;
pub mod layout;
pub mod mcp;

pub mod codex;

pub mod meetings;
pub mod perf;
pub mod prompts;

pub mod customization;
