# Pinned registry source patches

✅ These sources were copied from this machine's Cargo registry, not GitHub HEAD.
Original versions, copyright notices, licenses and `.cargo_vcs_info.json` are
retained. DevDesk patches are marked `DevDesk` in the source. Published test
fixture datasets and integration-target manifest entries are omitted.

- `alacritty_terminal` **0.26.0**, Apache-2.0, Alacritty contributors:
  [upstream](https://github.com/alacritty/alacritty).
  `EventLoop` retains a **64 KiB** PTY read buffer instead of 1 MiB, matching
  the existing approximately 64 KiB locked-read batch and reducing per-reader
  committed stack memory. 🧪 M7 noisy/flood and idle-memory probes verify this.
  `Cell::push_zerowidth` retains at most **8 combining characters** per cell.
  `Term::set_hyperlink` rejects links with URI > **8192 bytes** or ID > **256
  bytes**, leaving their text visible. A rotating pool permits at most **256
  live hyperlink allocations** per terminal, bounding retained URI bytes to
  **2 MiB**. Rejected links keep their text; slots recover as old links leave
  scrollback. Slot checks take constant work and use weak references.
  `Term::set_title` retains at most **200 Unicode characters**, including saved
  titles, rather than up to a full OSC payload per title-stack entry.
  `tty::from_fd` clears inherited variables before installing the explicit
  resolved environment (ADR-026), so shell rc unsets stay unset.
  `Pty::drop` skips signalling an already-reaped child, preventing a later
  drop of the drained loop from signalling a reused PID.
  `Term::replace_event_proxy` is a small DevDesk API addition: after joining the
  old reader, the worker replaces the event sink before attaching a fresh PTY.
  This keeps query replies and lifecycle events bound to the new instance while
  preserving the process's grid and history (ADR-027).
- `vte` **0.15.0**, MIT OR Apache-2.0, VTE contributors:
  [upstream](https://github.com/alacritty/vte).
  OSC payload length under `std` is capped at **160 KiB** (the Vec allocation
  may round up to 256 KiB). An overflowing OSC is
  discarded in its entirety through its terminator, and parsing then resumes.
  The limit accommodates the 100 KB clipboard-write policy after base64
  expansion. This also bounds unterminated OSC growth.

🧪 Reason: M1/A10's registry-source probe retained 100,000 combining marks in one
cell and a 100 KB hyperlink. Fixed scrollback length alone did not bound memory.
The dependency versions are unchanged; the event-sink setter is the only public API addition. No GPL code is used.
DevDesk regression tests cover oversized OSC, recovery, normal clipboard/title
sequences, links and combining clusters. Reapply or remove these small patches
deliberately on a dependency upgrade; inspect the registry diff first.

The exact changes against the registry are recorded in [devdesk.patch](devdesk.patch).
