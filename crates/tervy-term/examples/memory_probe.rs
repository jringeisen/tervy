//! Headless A10 probe. Peak RSS includes both grids and allocator overhead.
//! Run each scenario in a fresh release process to compare plateau phases.
use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::Dimensions as _;
use alacritty_terminal::term::cell::Cell;
use alacritty_terminal::term::{Config, Term};
use alacritty_terminal::vte::ansi::Processor;
use tervy_term::Dimensions;
#[derive(Clone)]
struct Ignore;
impl EventListener for Ignore {
    fn send_event(&self, _: Event) {}
}
fn main() {
    let scenario = std::env::args().nth(1).unwrap_or_else(|| "plain".into());
    assert!(matches!(
        scenario.as_str(),
        "plain" | "styled" | "combining" | "links"
    ));
    let dimensions = Dimensions {
        columns: 120,
        rows: 40,
        ..Dimensions::default()
    };
    let mut term = Term::new(
        Config {
            scrolling_history: 5000,
            ..Config::default()
        },
        &dimensions,
        Ignore,
    );
    let mut parser: Processor = Processor::new();
    let started = std::time::Instant::now();
    for i in 1..=20_000 {
        let line = match scenario.as_str() {
            "styled" => format!("\x1b[1;3;4;31;44m{}\x1b[0m\r\n", "x".repeat(119)),
            "combining" => format!(
                "{}\r\n",
                "e\u{301}\u{301}\u{301}\u{301}\u{301}\u{301}\u{301}\u{301}".repeat(119)
            ),
            "links" => {
                let mut line = String::new();
                for column in 0..119 {
                    line.push_str(&format!(
                        "\x1b]8;;https://example.test/{i}/{column}/{}\x1b\\X\x1b]8;;\x1b\\",
                        "a".repeat(512)
                    ));
                }
                line.push_str("\r\n");
                line
            }
            _ => format!("{}\r\n", "x".repeat(119)),
        };
        parser.advance(&mut term, line.as_bytes());
        if matches!(i, 5_000 | 10_000 | 20_000) {
            // SAFETY: getrusage writes a caller-owned structure. On macOS,
            // ru_maxrss is bytes; this tool targets the Tervy macOS host.
            let mut usage = unsafe { std::mem::zeroed::<libc::rusage>() };
            unsafe {
                libc::getrusage(libc::RUSAGE_SELF, &mut usage);
            }
            println!(
                "{{\"scenario\":\"{scenario}\",\"lines\":{i},\"history_lines\":{},\"cell_bytes\":{},\"peak_resident_bytes\":{},\"elapsed_ms\":{:.3}}}",
                term.grid().history_size(),
                std::mem::size_of::<Cell>(),
                usage.ru_maxrss,
                started.elapsed().as_secs_f64() * 1000.
            );
        }
    }
    assert_eq!(term.grid().history_size(), 5000);
}
