//! Reset via Handler methods: DECSTR is ignored upstream and RIS erases history.
use crate::terminal::EventProxy;
use alacritty_terminal::vte::ansi::{
    Attr, CharsetIndex, Handler, NamedMode, NamedPrivateMode as Private, ScpCharPath,
    ScpUpdateMode, StandardCharset,
};
use alacritty_terminal::{
    grid::Dimensions,
    index::{Column, Line, Point},
    term::{Term, TermMode},
};

pub(crate) fn prepare(term: &mut Term<EventProxy>, separator: &str) {
    let alternate = if term.mode().contains(TermMode::ALT_SCREEN) {
        Some(term.bounds_to_string(
            Point::new(Line(0), Column(0)),
            Point::new(term.bottommost_line(), term.last_column()),
        ))
    } else {
        None
    };
    // Empty both keyboard stacks, since leaving alt swaps the stacks too.
    term.pop_keyboard_modes(u16::MAX);
    term.unset_private_mode(Private::SwapScreenAndSetRestoreCursor.into());
    term.pop_keyboard_modes(u16::MAX);
    term.set_private_mode(Private::SwapScreenAndSetRestoreCursor.into());
    term.pop_keyboard_modes(u16::MAX);
    term.unset_private_mode(Private::SwapScreenAndSetRestoreCursor.into());
    term.pop_keyboard_modes(u16::MAX);
    for mode in [
        Private::CursorKeys,
        Private::Origin,
        Private::ReportMouseClicks,
        Private::ReportCellMouseMotion,
        Private::ReportAllMouseMotion,
        Private::ReportFocusInOut,
        Private::Utf8Mouse,
        Private::SgrMouse,
        Private::BracketedPaste,
    ] {
        term.unset_private_mode(mode.into());
    }
    for mode in [
        Private::ShowCursor,
        Private::LineWrap,
        Private::AlternateScroll,
        Private::UrgencyHints,
    ] {
        term.set_private_mode(mode.into());
    }
    term.unset_mode(NamedMode::Insert.into());
    term.unset_mode(NamedMode::LineFeedNewLine.into());
    term.unset_keypad_application_mode();
    term.set_scrolling_region(1, None);
    term.set_cursor_style(None);
    term.set_scp(ScpCharPath::LTR, ScpUpdateMode::ImplementationDependant);
    term.terminal_attribute(Attr::Reset);
    for charset in [
        CharsetIndex::G0,
        CharsetIndex::G1,
        CharsetIndex::G2,
        CharsetIndex::G3,
    ] {
        term.configure_charset(charset, StandardCharset::Ascii);
    }
    term.set_active_charset(CharsetIndex::G0);
    for color in 0..alacritty_terminal::term::color::COUNT {
        term.reset_color(color);
    }
    term.set_hyperlink(None);
    term.selection = None;
    if term.mode().contains(TermMode::VI) {
        term.toggle_vi_mode();
    }
    term.scroll_display(alacritty_terminal::grid::Scroll::Bottom);
    // Preserve the crashed alternate-screen text in primary history as well.
    // Use Handler::input for printable text, so a label cannot become an escape.
    append(term, "\n");
    if let Some(text) = alternate {
        append(term, text.trim_end());
        append(term, "\n");
    }
    append(term, separator);
    append(term, "\n");
}
fn append(term: &mut Term<EventProxy>, text: &str) {
    for ch in text.chars() {
        if ch == '\n' {
            term.carriage_return();
            term.linefeed();
        } else if !ch.is_control() {
            term.input(ch);
        }
    }
}
