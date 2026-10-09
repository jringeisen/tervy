//! Copy a visible grid while briefly holding Term, then shape/paint elsewhere.
//! ASCII neighbors batch into runs. Wide/fallback glyphs get their own cell
//! anchor so a shaping engine's advances cannot move later terminal columns.

use crate::terminal::EventProxy;
use alacritty_terminal::grid::Dimensions as _;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Term, TermMode, point_to_viewport};
use alacritty_terminal::vte::ansi::{Color, CursorShape, Rgb};

#[derive(Clone, PartialEq, Eq)]
pub struct Palette {
    pub foreground: Rgb,
    pub bold_foreground: Rgb,
    pub background: Rgb,
    pub selection: Rgb,
    pub ansi: [Rgb; 16],
}

impl Default for Palette {
    fn default() -> Self {
        Self {
            // Terminal.app's bundled Clear Dark profile, converted to sRGB
            // through AppKit on this Mac. The renderer uses an opaque surface.
            foreground: rgb(0xe6e6e6),
            bold_foreground: rgb(0xf0f0f0),
            background: rgb(0x212734),
            selection: rgb(0x334e5e),
            ansi: [
                0x35424c, 0xb45648, 0x6caa71, 0xc4ac62, 0x6d96b4, 0xbd7bcd, 0x7ccbcd, 0xdee5eb,
                0x465c6d, 0xdf6c5a, 0x79be7e, 0xe5c872, 0x67b5ed, 0xd389e5, 0x84dde0, 0xe5eff5,
            ]
            .map(rgb),
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
    /// TUIs often query the terminal background once, then emit nearby RGB
    /// shades for prompts and message strips. Those cells outlive an appearance
    /// change. Retone only neutral RGB surfaces near the opposite theme's
    /// background; colored cells, indexed colors and inverse video stay exact.
    fn surface_background(&self, color: Rgb) -> Rgb {
        let (other_background, other_foreground) = if self.background == rgb(0xf8f9fb) {
            (rgb(0x212734), rgb(0xe6e6e6))
        } else if self.background == rgb(0x212734) {
            (rgb(0xf8f9fb), rgb(0x29313d))
        } else {
            return color;
        };
        let channels = [color.r, color.g, color.b];
        let neutral = channels.iter().max().unwrap() - channels.iter().min().unwrap() <= 24;
        let close = color.r.abs_diff(other_background.r) <= 48
            && color.g.abs_diff(other_background.g) <= 48
            && color.b.abs_diff(other_background.b) <= 48;
        if !neutral || !close {
            return color;
        }
        let brightness =
            |value: Rgb| (f32::from(value.r) + f32::from(value.g) + f32::from(value.b)) / 3.;
        let contrast = brightness(other_foreground) - brightness(other_background);
        let shade = ((brightness(color) - brightness(other_background)) / contrast).clamp(0., 0.25);
        let blend = |paper: u8, ink: u8| {
            (f32::from(paper) + (f32::from(ink) - f32::from(paper)) * shade).round() as u8
        };
        Rgb {
            r: blend(self.background.r, self.foreground.r),
            g: blend(self.background.g, self.foreground.g),
            b: blend(self.background.b, self.foreground.b),
        }
    }

    /// A light companion to Clear Dark. ANSI ink is darker so colored shell
    /// output remains readable on the warm white surface. Explicit RGB and
    /// the standard 256-color cube still belong to the terminal application.
    pub fn light() -> Self {
        Self {
            foreground: rgb(0x29313d),
            bold_foreground: rgb(0x161e29),
            background: rgb(0xf8f9fb),
            selection: rgb(0xd5e4f0),
            ansi: [
                0x29313d, 0xa83232, 0x327343, 0x856119, 0x285f9a, 0x874495, 0x237477, 0xdce1e8,
                0x586473, 0xb83d3d, 0x357e46, 0x916a16, 0x316ba6, 0x964da5, 0x287c80, 0xffffff,
            ]
            .map(rgb),
        }
    }

    pub fn indexed(&self, index: usize) -> Rgb {
        match index {
            0..=15 => self.ansi[index],
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
            259..=266 => self.ansi[index - 259],
            267 => self.bold_foreground,
            268 => Rgb {
                r: ((u16::from(self.foreground.r) + u16::from(self.background.r)) / 2) as u8,
                g: ((u16::from(self.foreground.g) + u16::from(self.background.g)) / 2) as u8,
                b: ((u16::from(self.foreground.b) + u16::from(self.background.b)) / 2) as u8,
            },
            _ => self.foreground,
        }
    }

    fn resolve(&self, color: Color, term: &Term<EventProxy>, bold: bool) -> Rgb {
        let index = match color {
            Color::Spec(rgb) => return rgb,
            Color::Indexed(index) => index as usize,
            Color::Named(index) => index as usize,
        };
        let index = if bold && index < 8 {
            index + 8
        } else if bold && index == 256 {
            267
        } else {
            index
        };
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
            } else if matches!(cell.bg, Color::Spec(_)) {
                background = palette.surface_background(background);
            }
            if cell.flags.contains(Flags::DIM) {
                // Faint means less contrast against this cell's background.
                // Scaling RGB toward black leaves light-theme black ink
                // unchanged and makes dim colored text darker instead.
                let fade = |ink: u8, paper: u8| ((u16::from(ink) + u16::from(paper)) / 2) as u8;
                foreground = Rgb {
                    r: fade(foreground.r, background.r),
                    g: fade(foreground.g, background.g),
                    b: fade(foreground.b, background.b),
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

    #[test]
    fn ansi_faint_differs_from_typed_text_in_light_and_dark_palettes() {
        for (foreground, background) in [(0x000000, 0xffffff), (0xffffff, 0x000000)] {
            let (proxy, _events) = EventProxy::new();
            let mut term = Term::new(Default::default(), &Dimensions::default(), proxy);
            let mut parser: Processor = Processor::new();
            parser.advance(&mut term, b"A\x1b[2mB\x1b[22mC");
            let palette = Palette {
                foreground: rgb(foreground),
                background: rgb(background),
                ..Default::default()
            };
            let snapshot = RenderSnapshot::from_term(&term, &palette);
            assert_eq!(snapshot.runs[0].style.foreground, rgb(foreground));
            assert_eq!(snapshot.runs[1].text, "B");
            assert_eq!(snapshot.runs[1].style.foreground, rgb(0x7f7f7f));
            assert_eq!(snapshot.runs[2].style.foreground, rgb(foreground));
        }
    }

    #[test]
    fn ansi_faint_uses_the_cells_explicit_background() {
        let (proxy, _events) = EventProxy::new();
        let mut term = Term::new(Default::default(), &Dimensions::default(), proxy);
        let mut parser: Processor = Processor::new();
        parser.advance(&mut term, b"\x1b[38;2;240;120;60;48;2;20;40;80;2mX");
        let snapshot = RenderSnapshot::from_term(&term, &Palette::default());
        assert_eq!(snapshot.runs[0].text, "X");
        assert_eq!(
            snapshot.runs[0].style.foreground,
            Rgb {
                r: 130,
                g: 80,
                b: 70
            }
        );
        assert_eq!(
            snapshot.backgrounds[0].color,
            Rgb {
                r: 20,
                g: 40,
                b: 80
            }
        );
    }
    #[test]
    fn neutral_tui_strips_follow_appearance_without_mutating_terminal_cells() {
        for (bytes, original, from, to) in [
            (
                b"\x1b[48;2;59;62;73mX".as_slice(),
                rgb(0x3b3e49),
                Palette::default(),
                Palette::light(),
            ),
            (
                b"\x1b[48;2;232;236;242mX".as_slice(),
                rgb(0xe8ecf2),
                Palette::light(),
                Palette::default(),
            ),
        ] {
            let (proxy, _events) = EventProxy::new();
            let mut term = Term::new(Default::default(), &Dimensions::default(), proxy);
            let mut parser: Processor = Processor::new();
            parser.advance(&mut term, bytes);
            assert_eq!(
                RenderSnapshot::from_term(&term, &from).backgrounds[0].color,
                original
            );
            let snapshot = RenderSnapshot::from_term(&term, &to);
            let surface = snapshot.backgrounds[0].color;
            assert_ne!(surface, original);
            assert_ne!(surface, to.background, "retain a distinguishable strip");
            assert!(surface.r.abs_diff(to.background.r) < 40);
            assert!(surface.g.abs_diff(to.background.g) < 40);
            assert!(surface.b.abs_diff(to.background.b) < 40);
            assert_eq!(snapshot.runs[0].style.foreground, to.foreground);
            assert_eq!(
                RenderSnapshot::from_term(&term, &from).backgrounds[0].color,
                original
            );
        }
    }

    #[test]
    fn surface_adaptation_preserves_colored_and_indexed_backgrounds() {
        let (proxy, _events) = EventProxy::new();
        let mut term = Term::new(Default::default(), &Dimensions::default(), proxy);
        let mut parser: Processor = Processor::new();
        parser.advance(
            &mut term,
            b"\x1b[48;2;20;40;80mA\x1b[48;5;236mB\x1b[48;2;59;62;73;7mC",
        );
        let snapshot = RenderSnapshot::from_term(&term, &Palette::light());
        assert_eq!(snapshot.backgrounds[0].color, rgb(0x142850));
        assert_eq!(snapshot.backgrounds[1].color, rgb(0x303030));
        let inverse = snapshot.runs.iter().find(|run| run.text == "C").unwrap();
        assert_eq!(inverse.style.foreground, rgb(0x3b3e49));
    }

    #[test]
    fn switching_palettes_recolors_existing_cells_and_preserves_explicit_rgb() {
        let (proxy, _events) = EventProxy::new();
        let mut term = Term::new(Default::default(), &Dimensions::default(), proxy);
        let mut parser: Processor = Processor::new();
        parser.advance(&mut term, b"A\x1b[31mB\x1b[1mC\x1b[0;38;2;20;40;60mD");
        for palette in [Palette::default(), Palette::light(), Palette::default()] {
            let snapshot = RenderSnapshot::from_term(&term, &palette);
            assert_eq!(snapshot.runs[0].style.foreground, palette.foreground);
            assert_eq!(snapshot.runs[1].style.foreground, palette.ansi[1]);
            assert_eq!(snapshot.runs[2].style.foreground, palette.ansi[9]);
            assert_eq!(snapshot.runs[3].style.foreground, rgb(0x14283c));
            assert_eq!(palette.indexed(196), rgb(0xff0000));
            assert_eq!(palette.indexed(244), rgb(0x808080));
        }
    }

    #[test]
    fn clear_dark_bold_reset_and_explicit_colors_keep_their_intended_styles() {
        let (proxy, _events) = EventProxy::new();
        let mut term = Term::new(Default::default(), &Dimensions::default(), proxy);
        let mut parser: Processor = Processor::new();
        parser.advance(&mut term, b"A\x1b[1mB\x1b[22mC\x1b[38;2;20;40;60mD");
        let palette = Palette::default();
        let snapshot = RenderSnapshot::from_term(&term, &palette);
        assert_eq!(snapshot.runs[0].style.foreground, palette.foreground);
        assert_eq!(snapshot.runs[1].style.foreground, palette.bold_foreground);
        assert!(snapshot.runs[1].style.bold);
        assert_eq!(snapshot.runs[2].style.foreground, palette.foreground);
        assert!(!snapshot.runs[2].style.bold);
        assert_eq!(
            snapshot.runs[3].style.foreground,
            Rgb {
                r: 20,
                g: 40,
                b: 60
            }
        );
    }
}
