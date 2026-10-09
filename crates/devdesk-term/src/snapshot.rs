//! Copy a visible grid while briefly holding Term, then shape/paint elsewhere.
//! ASCII neighbors batch into runs. Wide/fallback glyphs get their own cell
//! anchor so a shaping engine's advances cannot move later terminal columns.

use crate::terminal::EventProxy;
use alacritty_terminal::grid::Dimensions as _;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Term, TermMode, point_to_viewport};
use alacritty_terminal::vte::ansi::{Color, CursorShape, Rgb};

#[derive(Clone)]
pub struct Palette {
    pub foreground: Rgb,
    pub background: Rgb,
    pub selection: Rgb,
}

impl Default for Palette {
    fn default() -> Self {
        Self {
            foreground: rgb(0xd8dee9),
            background: rgb(0x20242c),
            selection: rgb(0x405674),
        }
    }
}

pub const fn rgb(value: u32) -> Rgb {
    Rgb {
        r: (value >> 16) as u8,
        g: (value >> 8) as u8,
        b: value as u8,
    }
}

impl Palette {
    pub fn indexed(&self, index: usize) -> Rgb {
        const ANSI: [u32; 16] = [
            0x1d2027, 0xd95965, 0x8cbf73, 0xe5c07b, 0x63a6e9, 0xc678dd, 0x56b6c2, 0xc8ccd4,
            0x5c6370, 0xef7681, 0xace08d, 0xf5d99a, 0x85c4ff, 0xe8a1f1, 0x7dd8e3, 0xf5f5f5,
        ];
        match index {
            0..=15 => rgb(ANSI[index]),
            16..=231 => {
                let value = index - 16;
                let component = |n: usize| if n == 0 { 0 } else { (55 + 40 * n) as u8 };
                Rgb {
                    r: component(value / 36),
                    g: component(value / 6 % 6),
                    b: component(value % 6),
                }
            }
            232..=255 => {
                let gray = (8 + 10 * (index - 232)) as u8;
                Rgb {
                    r: gray,
                    g: gray,
                    b: gray,
                }
            }
            257 => self.background,
            259..=266 => rgb(ANSI[index - 259]),
            _ => self.foreground,
        }
    }

    fn resolve(&self, color: Color, term: &Term<EventProxy>, bold: bool) -> Rgb {
        let index = match color {
            Color::Spec(rgb) => return rgb,
            Color::Indexed(index) => index as usize,
            Color::Named(index) => index as usize,
        };
        let index = if bold && index < 8 { index + 8 } else { index };
        term.colors()[index].unwrap_or_else(|| self.indexed(index))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunStyle {
    pub foreground: Rgb,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strikeout: bool,
}

#[derive(Clone, Debug)]
pub struct TextRun {
    pub row: usize,
    pub column: usize,
    pub cells: usize,
    pub text: String,
    pub style: RunStyle,
}

#[derive(Clone, Debug)]
pub struct Background {
    pub row: usize,
    pub column: usize,
    pub cells: usize,
    pub color: Rgb,
}

#[derive(Clone, Debug)]
pub struct Cursor {
    pub row: usize,
    pub column: usize,
    pub shape: CursorShape,
}

pub struct Hyperlink {
    pub row: usize,
    pub column: usize,
    pub cells: usize,
    pub uri: String,
}

pub struct RenderSnapshot {
    pub runs: Vec<TextRun>,
    pub backgrounds: Vec<Background>,
    pub hyperlinks: Vec<Hyperlink>,
    pub cursor: Option<Cursor>,
    pub mode: TermMode,
    pub display_offset: usize,
    pub columns: usize,
    pub rows: usize,
}

impl RenderSnapshot {
    pub fn from_term(term: &Term<EventProxy>, palette: &Palette) -> Self {
        let content = term.renderable_content();
        let mut result = Self {
            runs: Vec::new(),
            backgrounds: Vec::new(),
            hyperlinks: Vec::new(),
            cursor: point_to_viewport(content.display_offset, content.cursor.point).map(|point| {
                Cursor {
                    row: point.line,
                    column: point.column.0,
                    shape: content.cursor.shape,
                }
            }),
            mode: content.mode,
            display_offset: content.display_offset,
            columns: term.columns(),
            rows: term.screen_lines(),
        };
        let mut batch_ascii = false;
        for indexed in content.display_iter {
            let Some(point) = point_to_viewport(content.display_offset, indexed.point) else {
                continue;
            };
            let row = point.line;
            let column = point.column.0;
            let cell = indexed.cell;
            let mut foreground = palette.resolve(cell.fg, term, cell.flags.contains(Flags::BOLD));
            let mut background = palette.resolve(cell.bg, term, false);
            if cell.flags.contains(Flags::INVERSE) {
                std::mem::swap(&mut foreground, &mut background);
            }
            if cell.flags.contains(Flags::DIM) {
                foreground = Rgb {
                    r: foreground.r / 2,
                    g: foreground.g / 2,
                    b: foreground.b / 2,
                };
            }
            if content
                .selection
                .as_ref()
                .is_some_and(|selection| selection.contains(indexed.point))
            {
                background = palette.selection;
            }
            if let Some(previous) = result.backgrounds.last_mut()
                && previous.row == row
                && previous.column + previous.cells == column
                && previous.color == background
            {
                previous.cells += 1;
            } else {
                result.backgrounds.push(Background {
                    row,
                    column,
                    cells: 1,
                    color: background,
                });
            }
            if cell
                .flags
                .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
            {
                continue;
            }
            let cells = if cell.flags.contains(Flags::WIDE_CHAR) {
                2
            } else {
                1
            };
            let style = RunStyle {
                foreground,
                bold: cell.flags.contains(Flags::BOLD),
                italic: cell.flags.contains(Flags::ITALIC),
                underline: cell.flags.intersects(Flags::ALL_UNDERLINES),
                strikeout: cell.flags.contains(Flags::STRIKEOUT),
            };
            let mut text = if cell.flags.contains(Flags::HIDDEN) {
                " ".into()
            } else {
                cell.c.to_string()
            };
            if let Some(zero) = cell.zerowidth() {
                text.extend(zero.iter());
            }
            let ascii = text.is_ascii() && cells == 1;
            if batch_ascii
                && ascii
                && let Some(previous) = result.runs.last_mut()
                && previous.row == row
                && previous.column + previous.cells == column
                && previous.style == style
            {
                previous.text.push_str(&text);
                previous.cells += cells;
            } else {
                result.runs.push(TextRun {
                    row,
                    column,
                    cells,
                    text,
                    style,
                });
            }
            batch_ascii = ascii;
            if let Some(link) = cell.hyperlink()
                && link.uri().len() <= 8192
            {
                result.hyperlinks.push(Hyperlink {
                    row,
                    column,
                    cells,
                    uri: link.uri().to_owned(),
                });
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::Dimensions;
    use alacritty_terminal::vte::ansi::Processor;

    #[test]
    fn style_batches_split_and_wide_cells_keep_column_anchors() {
        let (proxy, _) = EventProxy::new();
        let mut term = Term::new(Default::default(), &Dimensions::default(), proxy);
        let mut parser: Processor = Processor::new();
        parser.advance(&mut term, "abc\x1b[31mde\x1b[0m界x".as_bytes());
        let snap = RenderSnapshot::from_term(&term, &Palette::default());
        assert_eq!(snap.runs[0].text, "abc");
        assert_eq!(snap.runs[1].text, "de");
        assert_eq!(snap.runs[2].text, "界");
        assert_eq!(snap.runs[2].cells, 2);
        assert_eq!(snap.runs[3].column, 7);
        assert_eq!(snap.backgrounds.len(), Dimensions::default().rows as usize);
    }
}
