//! These are resource-boundary tests, not rendering limits: data must be
//! bounded in the parser/grid even when no terminal view ever takes a snapshot.
use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::term::Term;
use alacritty_terminal::vte::ansi::Processor;
use devdesk_term::{Column, Dimensions, Line};
#[derive(Clone)]
struct Ignore;
impl EventListener for Ignore {
    fn send_event(&self, _: Event) {}
}

#[test]
fn combining_flood_preserves_a_normal_cluster_without_unbounded_extra() {
    let mut term = Term::new(Default::default(), &Dimensions::default(), Ignore);
    let mut parser: Processor = Processor::new();
    parser.advance(&mut term, "e\u{301}".as_bytes());
    for _ in 0..100_000 {
        parser.advance(&mut term, "\u{301}".as_bytes());
    }
    let cell = &term.grid()[Line(0)][Column(0)];
    assert_eq!(cell.c, 'e');
    assert!(cell.zerowidth().unwrap().len() <= 8);
    assert_eq!(cell.zerowidth().unwrap()[0], '\u{301}');
}

#[test]
fn oversized_hyperlinks_drop_metadata_but_keep_visible_text() {
    let mut term = Term::new(Default::default(), &Dimensions::default(), Ignore);
    let mut parser: Processor = Processor::new();
    for (id, uri) in [
        ("i".repeat(257), "https://example.test".into()),
        ("ok".into(), "a".repeat(8193)),
    ] {
        parser.advance(
            &mut term,
            format!("\x1b]8;id={id};{uri}\x1b\\X\x1b]8;;\x1b\\").as_bytes(),
        );
    }
    parser.advance(
        &mut term,
        b"\x1b]8;;https://example.test\x1b\\Y\x1b]8;;\x1b\\",
    );
    let row = &term.grid()[Line(0)];
    assert_eq!(row[Column(0)].c, 'X');
    assert!(row[Column(0)].hyperlink().is_none());
    assert_eq!(row[Column(1)].c, 'X');
    assert!(row[Column(1)].hyperlink().is_none());
    assert_eq!(
        row[Column(2)].hyperlink().unwrap().uri(),
        "https://example.test"
    );
}

#[test]
fn oversized_and_unterminated_osc_are_bounded_and_parser_recovers() {
    #[derive(Default)]
    struct Capture(Vec<String>);
    impl alacritty_terminal::vte::Perform for Capture {
        fn osc_dispatch(&mut self, params: &[&[u8]], _: bool) {
            self.0.push(String::from_utf8_lossy(params[1]).into_owned());
        }
    }
    let mut parser = alacritty_terminal::vte::Parser::new();
    let mut capture = Capture::default();
    parser.advance(&mut capture, b"\x1b]0;");
    for _ in 0..500 {
        parser.advance(&mut capture, &[b'a'; 8192]);
    }
    parser.advance(&mut capture, b"\x07\x1b]0;normal title\x07");
    assert_eq!(capture.0, ["normal title"]);
    // ESC-ST and CAN cancellation must recover too.
    for terminator in [b"\x1b\\".as_slice(), b"\x18"] {
        parser.advance(&mut capture, b"\x1b]0;");
        for _ in 0..30 {
            parser.advance(&mut capture, &[b'b'; 8192]);
        }
        parser.advance(&mut capture, terminator);
        parser.advance(&mut capture, b"\x1b]0;recovered\x07");
    }
    assert_eq!(capture.0.len(), 3);
}

#[test]
fn distinct_links_have_a_terminal_budget_and_recover_after_history_clear() {
    use alacritty_terminal::grid::Dimensions as _;
    use alacritty_terminal::vte::ansi::Handler;
    let mut term = Term::new(Default::default(), &Dimensions::default(), Ignore);
    let mut parser: Processor = Processor::new();
    for i in 0..1000 {
        parser.advance(
            &mut term,
            format!("\x1b]8;;https://example.test/{i}\x1b\\X\x1b]8;;\x1b\\").as_bytes(),
        );
    }
    let linked = term
        .grid()
        .display_iter()
        .filter(|cell| cell.hyperlink().is_some())
        .count();
    assert!(linked <= 256);
    assert!(
        term.grid()
            .display_iter()
            .filter(|cell| cell.c == 'X')
            .count()
            >= 1000
    );
    term.clear_screen(alacritty_terminal::vte::ansi::ClearMode::All);
    term.grid_mut().clear_history();
    term.grid_mut().truncate();
    term.goto(0, 0);
    // At least one free slot is found within a full rotation.
    for i in 0..256 {
        parser.advance(
            &mut term,
            format!("\x1b]8;;https://recovered.test/{i}\x1b\\Y\x1b]8;;\x1b\\").as_bytes(),
        );
    }
    assert!(
        term.grid()
            .display_iter()
            .any(|cell| cell.hyperlink().is_some())
    );
    assert_eq!(term.columns(), 100);
}
