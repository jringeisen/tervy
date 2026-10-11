//! Coordinated workspace and terminal palettes that follow macOS appearance.
use devdesk_term::snapshot::Palette;
use gpui_kit::Global;
use gpui_kit::component::{Theme, ThemeMode};

#[derive(Default)]
struct Preference(String);
impl Global for Preference {}

pub(crate) fn configure(value: &str, window: &mut Window, cx: &mut App) {
    cx.set_global(Preference(value.into()));
    sync_appearance(window, cx);
}
fn warm(cx: &App) -> bool {
    cx.try_global::<Preference>()
        .is_some_and(|p| p.0 == "warm-dark")
}
use gpui_kit::{App, Window, px, rgb};

/// Appearance changes reload the component theme, so apply our workspace colors
/// after each reload. Theme::update also updates semantic tokens and the base
/// scrollbar/resize theme; changing only raw colors leaves components stale.
pub(crate) fn sync_appearance(window: &mut Window, cx: &mut App) {
    match cx.try_global::<Preference>().map(|p| p.0.as_str()) {
        Some("dark" | "warm-dark") => Theme::change(ThemeMode::Dark, Some(window), cx),
        Some("light") => Theme::change(ThemeMode::Light, Some(window), cx),
        _ => Theme::sync_system_appearance(Some(window), cx),
    }
    apply_palette(cx);
}

pub(crate) fn terminal_palette(cx: &App) -> Palette {
    if warm(cx) {
        let mut palette = Palette::default();
        palette.background.r = 0x28;
        palette.background.g = 0x24;
        palette.background.b = 0x20;
        palette.foreground.r = 0xeb;
        palette.foreground.g = 0xdb;
        palette.foreground.b = 0xb2;
        palette.bold_foreground = palette.foreground;
        palette
    } else if Theme::global(cx).is_dark() {
        Palette::default()
    } else {
        Palette::light()
    }
}

pub(crate) fn apply_palette(cx: &mut App) {
    let warm = warm(cx);
    Theme::update(cx, |theme| {
        let (surface, raised, hover, selected, border, foreground, muted) = if warm {
            (
                0x282420, 0x302c28, 0x39342f, 0x303f50, 0x49433d, 0xeee8de, 0xb8aea1,
            )
        } else if theme.is_dark() {
            (
                0x212734, 0x282e39, 0x303743, 0x2c4059, 0x3b4350, 0xe9edf3, 0xa9b2c0,
            )
        } else {
            (
                0xf8f9fb, 0xf0f2f5, 0xe8ebf0, 0xe2ecfa, 0xd9dee6, 0x252c38, 0x5f6b7b,
            )
        };
        let surface = rgb(surface).into();
        let raised = rgb(raised).into();
        let hover = rgb(hover).into();
        let selected = rgb(selected).into();
        let border = rgb(border).into();
        let foreground = rgb(foreground).into();
        let muted = rgb(muted).into();

        let primary = rgb(if theme.is_dark() { 0x366dcc } else { 0x245db5 }).into();
        let primary_hover = rgb(if theme.is_dark() { 0x4179dc } else { 0x1e52a2 }).into();
        let primary_active = rgb(if theme.is_dark() { 0x2c5fba } else { 0x19488e }).into();
        theme.radius = px(6.);
        theme.radius_lg = px(8.);
        theme.shadow = false;
        theme.focus_ring = true;
        theme.primary = primary;
        theme.primary_hover = primary_hover;
        theme.primary_active = primary_active;
        theme.primary_foreground = rgb(0xffffff).into();
        theme.button_primary = primary;
        theme.button_primary_hover = primary_hover;
        theme.button_primary_active = primary_active;
        theme.button_primary_foreground = rgb(0xffffff).into();
        theme.ring = rgb(if theme.is_dark() { 0x8ab4ff } else { 0x245db5 }).into();
        theme.background = surface;
        theme.foreground = foreground;
        theme.border = border;
        theme.accent = hover;
        theme.accent_foreground = foreground;
        theme.muted = raised;
        theme.muted_foreground = muted;
        theme.sidebar = raised;
        theme.sidebar_foreground = foreground;
        theme.sidebar_border = border;
        theme.sidebar_accent = selected;
        theme.sidebar_accent_foreground = foreground;
        theme.tab_bar = surface;
        theme.tab = surface;
        theme.tab_foreground = muted;
        theme.tab_active = raised;
        theme.tab_active_foreground = foreground;
        theme.popover = surface;
        theme.popover_foreground = foreground;
        theme.colors.list = raised;
        theme.list_hover = hover;
        theme.list_active = selected;
        theme.list_head = raised;
        theme.input = border;
        theme.button = raised;
        theme.button_hover = hover;
        theme.button_active = selected;
        theme.button_foreground = foreground;
        theme.secondary = raised;
        theme.secondary_hover = hover;
        theme.secondary_active = selected;
        theme.secondary_foreground = foreground;
        theme.selection = rgb(if theme.is_dark() { 0x334e5e } else { 0xd5e4f0 }).into();
        theme.scrollbar = surface;
        theme.scrollbar_thumb = border;
        theme.scrollbar_thumb_hover = muted;
    });
}
