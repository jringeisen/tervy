//! A terminal is a fixed cell grid, not flowing text. Snapshot copying is the
//! only operation here that locks Term. Font lookup, shaping and GPU painting
//! happen after that lock has been released. CoreText supplies missing glyphs;
//! each non-ASCII cluster retains its own grid anchor regardless of its advance.

use crate::terminal_view::TerminalView;
use devdesk_term::{CursorShape, Dimensions, Rgb};
use gpui_kit::*;

#[derive(Clone, Copy)]
pub struct CellMetrics {
    pub width: Pixels,
    pub height: Pixels,
}
impl Default for CellMetrics {
    fn default() -> Self {
        Self {
            width: px(8.),
            height: px(18.),
        }
    }
}

pub struct TerminalElement {
    view: Entity<TerminalView>,
}
impl TerminalElement {
    pub fn new(view: Entity<TerminalView>) -> Self {
        Self { view }
    }
}
impl IntoElement for TerminalElement {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

pub struct PaintedRun {
    line: ShapedLine,
    bounds: Bounds<Pixels>,
}
pub struct PaintState {
    runs: Vec<PaintedRun>,
    symbols: Vec<(char, Bounds<Pixels>, Hsla)>,
}

impl Element for TerminalElement {
    type RequestLayoutState = ();
    type PrepaintState = PaintState;
    fn id(&self) -> Option<ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }
    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = relative(1.).into();
        (window.request_layout(style, [], cx), ())
    }
    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> PaintState {
        let _phase = crate::perf::phase("terminal prepaint");
        let font_size = px(self.view.read(cx).font_size);
        let mut regular = font(self.view.read(cx).font_family.clone());
        regular.features = FontFeatures::disable_ligatures();
        let font_id = window.text_system().resolve_font(&regular);
        let metrics = CellMetrics {
            width: window
                .text_system()
                .ch_advance(font_id, font_size)
                .unwrap_or(px(8.)),
            height: (window.text_system().ascent(font_id, font_size)
                + window.text_system().descent(font_id, font_size)
                + px(3.))
            .ceil(),
        };
        self.view.update(cx, |view, _| {
            view.bounds = bounds;
            view.metrics = metrics;
            let dimensions = Dimensions {
                columns: (bounds.size.width / metrics.width)
                    .floor()
                    .clamp(2., u16::MAX as f32) as u16,
                rows: (bounds.size.height / metrics.height)
                    .floor()
                    .clamp(2., u16::MAX as f32) as u16,
                cell_width: f32::from(metrics.width).round() as u16,
                cell_height: f32::from(metrics.height).round() as u16,
            };
            if let Some(session) = &view.session {
                // prepaint runs once per frame; repeated OS resize events are
                // reduced to the final cell dimensions for that frame.
                if view.size != dimensions {
                    session.handle.resize(dimensions);
                }
                view.snapshot = Some(session.handle.snapshot(&view.palette));
            }
            view.size = dimensions;
        });
        let view = self.view.read(cx);
        let mut runs = Vec::new();
        let mut symbols = Vec::new();
        if let Some(snapshot) = &view.snapshot {
            for run in &snapshot.runs {
                if run.text.chars().count() == 1
                    && let Some(character) = run.text.chars().next()
                    && grid_symbol(character)
                {
                    symbols.push((
                        character,
                        cell_bounds(bounds.origin, metrics, run.column, run.row, run.cells),
                        color(run.style.foreground),
                    ));
                    continue;
                }
                if run.text.trim().is_empty() && !run.style.underline && !run.style.strikeout {
                    continue;
                }
                let mut font = regular.clone();
                if run.style.bold {
                    font = font.bold();
                }
                if run.style.italic {
                    font = font.italic();
                }
                let color = color(run.style.foreground);
                let text_run = TextRun {
                    len: run.text.len(),
                    font,
                    color,
                    background_color: None,
                    underline: run.style.underline.then_some(UnderlineStyle {
                        thickness: px(1.),
                        color: Some(color),
                        wavy: false,
                    }),
                    strikethrough: run.style.strikeout.then_some(StrikethroughStyle {
                        thickness: px(1.),
                        color: Some(color),
                    }),
                };
                runs.push(PaintedRun {
                    line: window.text_system().shape_line(
                        run.text.clone().into(),
                        font_size,
                        &[text_run],
                        None,
                    ),
                    bounds: cell_bounds(bounds.origin, metrics, run.column, run.row, run.cells),
                });
            }
            if !view.preedit.is_empty()
                && let Some(cursor) = &snapshot.cursor
            {
                let color = color(view.palette.foreground);
                let text_run = TextRun {
                    len: view.preedit.len(),
                    font: regular,
                    color,
                    background_color: None,
                    underline: Some(UnderlineStyle {
                        thickness: px(1.),
                        color: Some(color),
                        wavy: false,
                    }),
                    strikethrough: None,
                };
                runs.push(PaintedRun {
                    line: window.text_system().shape_line(
                        view.preedit.clone().into(),
                        font_size,
                        &[text_run],
                        None,
                    ),
                    bounds: cell_bounds(
                        bounds.origin,
                        metrics,
                        cursor.column,
                        cursor.row,
                        snapshot.columns - cursor.column,
                    ),
                });
            }
        }
        PaintState { runs, symbols }
    }
    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        state: &mut PaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let _phase = crate::perf::phase("terminal paint");
        self.view.update(cx, |view, cx| {
            let focus = view.focus.clone();
            let metrics = view.metrics;
            window.handle_input(
                &focus,
                ElementInputHandler::new(bounds, self.view.clone()),
                cx,
            );
            window.with_content_mask(Some(ContentMask { bounds }), |window| {
                window.paint_quad(fill(bounds, color(view.palette.background)));
                if let Some(snapshot) = &view.snapshot {
                    for bg in &snapshot.backgrounds {
                        window.paint_quad(fill(
                            cell_bounds(bounds.origin, metrics, bg.column, bg.row, bg.cells),
                            color(bg.color),
                        ));
                    }
                    for (character, bounds, color) in &state.symbols {
                        paint_symbol(*character, *bounds, *color, window);
                    }
                    for run in &state.runs {
                        window.with_content_mask(
                            Some(ContentMask { bounds: run.bounds }),
                            |window| {
                                if let Err(error) = run.line.paint(
                                    run.bounds.origin,
                                    metrics.height,
                                    TextAlign::Left,
                                    None,
                                    window,
                                    cx,
                                ) {
                                    tracing::warn!(%error, "terminal glyph paint failed");
                                }
                            },
                        );
                    }
                    if let Some(cursor) = &snapshot.cursor
                        && cursor.shape != CursorShape::Hidden
                    {
                        let mut rect =
                            cell_bounds(bounds.origin, metrics, cursor.column, cursor.row, 1);
                        let mut ink = color(view.palette.foreground);
                        if focus.is_focused(window) && window.is_window_active() {
                            match cursor.shape {
                                CursorShape::Beam => rect.size.width = px(2.),
                                CursorShape::Underline => {
                                    rect.origin.y += metrics.height - px(2.);
                                    rect.size.height = px(2.);
                                }
                                _ => ink.a = 0.35,
                            }
                            window.paint_quad(fill(rect, ink));
                        } else {
                            // An unfocused cursor remains visible without a timer.
                            window.paint_quad(fill(
                                Bounds::new(rect.origin, size(rect.size.width, px(1.))),
                                ink,
                            ));
                        }
                    }
                }
            });
            if let Some(probe) = view.probe.as_mut()
                && let Some(snapshot) = &view.snapshot
                && probe.on_paint(snapshot, &self.view, window)
            {
                cx.quit();
            }
        });
    }
}
fn cell_bounds(
    origin: Point<Pixels>,
    metrics: CellMetrics,
    col: usize,
    row: usize,
    cells: usize,
) -> Bounds<Pixels> {
    Bounds::new(
        origin + point(metrics.width * col as f32, metrics.height * row as f32),
        size(metrics.width * cells as f32, metrics.height),
    )
}
fn color(value: Rgb) -> Hsla {
    rgb((value.r as u32) << 16 | (value.g as u32) << 8 | value.b as u32).into()
}

// Common TUI rules and powerline separators reach cell edges exactly; using
// font bearings for these produces seams even in an otherwise monospaced font.
fn grid_symbol(c: char) -> bool {
    matches!(c, '─' | '━' | '│' | '┃' | '┌' | '┐' | '└' | '┘' | '├' | '┤' | '┬' | '┴' | '┼' | '\u{e0b0}'..='\u{e0b3}' | '\u{2580}'..='\u{2588}' | '▌' | '▐' | '▔' | '▕')
}
fn paint_symbol(c: char, bounds: Bounds<Pixels>, color: Hsla, window: &mut Window) {
    let x = bounds.left();
    let y = bounds.top();
    let w = bounds.size.width;
    let h = bounds.size.height;
    let center_x = x + (w / 2.).floor();
    let center_y = y + (h / 2.).floor();
    if matches!(c, '\u{e0b0}'..='\u{e0b3}') {
        let left = c == '\u{e0b2}' || c == '\u{e0b3}';
        let thin = c == '\u{e0b1}' || c == '\u{e0b3}';
        let mut path = if thin {
            PathBuilder::stroke(px(1.))
        } else {
            PathBuilder::fill()
        };
        let base = if left { x + w } else { x };
        let tip = if left { x } else { x + w };
        path.move_to(point(base, y));
        path.line_to(point(tip, y + h / 2.));
        path.line_to(point(base, y + h));
        if !thin {
            path.close();
        }
        if let Ok(path) = path.build() {
            window.paint_path(path, color);
        }
        return;
    }
    if matches!(c, '\u{2580}'..='\u{2588}' | '▌' | '▐' | '▔' | '▕') {
        let (origin, size) = match c {
            '▀' => (point(x, y), size(w, h / 2.)),
            '▌' => (point(x, y), size(w / 2., h)),
            '▐' => (point(x + w / 2., y), size(w / 2., h)),
            '▔' => (point(x, y), size(w, h / 8.)),
            '▕' => (point(x + w * 7. / 8., y), size(w / 8., h)),
            _ => {
                let height = h * (c as u32 - 0x2580) as f32 / 8.;
                (point(x, y + h - height), size(w, height))
            }
        };
        window.paint_quad(fill(Bounds::new(origin, size), color));
        return;
    }
    let left = matches!(c, '─' | '━' | '┐' | '┘' | '┤' | '┬' | '┴' | '┼');
    let right = matches!(c, '─' | '━' | '┌' | '└' | '├' | '┬' | '┴' | '┼');
    let up = matches!(c, '│' | '┃' | '└' | '┘' | '├' | '┤' | '┴' | '┼');
    let down = matches!(c, '│' | '┃' | '┌' | '┐' | '├' | '┤' | '┬' | '┼');
    let thickness = if c == '━' || c == '┃' {
        px(2.)
    } else {
        px(1.)
    };
    if left {
        window.paint_quad(fill(
            Bounds::new(
                point(x, center_y),
                size(center_x - x + thickness, thickness),
            ),
            color,
        ));
    }
    if right {
        window.paint_quad(fill(
            Bounds::new(point(center_x, center_y), size(x + w - center_x, thickness)),
            color,
        ));
    }
    if up {
        window.paint_quad(fill(
            Bounds::new(
                point(center_x, y),
                size(thickness, center_y - y + thickness),
            ),
            color,
        ));
    }
    if down {
        window.paint_quad(fill(
            Bounds::new(point(center_x, center_y), size(thickness, y + h - center_y)),
            color,
        ));
    }
}
