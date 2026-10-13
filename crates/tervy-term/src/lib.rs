//! Tervy terminal engine.
//!
//! Owns PTY spawning, the `alacritty_terminal` grid, key encoding, render
//! snapshots and process-group termination. It knows nothing about projects,
//! processes or agents, and never depends on GPUI (architecture.md §3).
//!
pub mod keys;
pub mod pty;
pub mod snapshot;
pub mod terminal;

pub use alacritty_terminal::event::Event;
pub use alacritty_terminal::grid::{Dimensions as GridDimensions, Scroll};
pub use alacritty_terminal::index::{Column, Line, Point, Side};
pub use alacritty_terminal::selection::{Selection, SelectionType};
pub use alacritty_terminal::term::TermMode;
pub use alacritty_terminal::vte::ansi::{CursorShape, Rgb};
pub use pty::{SpawnSpec, TerminalSession};
pub use terminal::{Dimensions, TerminalEvents, TerminalHandle, TerminalSignal};
pub mod kill;
mod restart;
