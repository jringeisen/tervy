//! DevDesk terminal engine.
//!
//! Owns PTY spawning, the `alacritty_terminal` grid, key encoding, render
//! snapshots and process-group termination. It knows nothing about projects,
//! processes or agents, and never depends on GPUI (architecture.md §3).
//!
//! Populated in M1.
