//! Warm-light ("Claude paper") theme: palette, layout dimensions, and the
//! global theme application applied once at startup.

use gpui::{px, App, Hsla};
use gpui_component::{Theme, ThemeMode};

use crate::types::HttpMethod;

// ===== Palette (warm paper light), as 0xRRGGBB =====
const BACKGROUND: u32 = 0xFAF9F5; // main canvas
const SIDEBAR: u32 = 0xF0EEE6; // sidebar / muted surfaces
const SURFACE: u32 = 0xFFFFFF; // white surfaces (input / popover)
const FOREGROUND: u32 = 0x1A1915; // primary text
const MUTED_FG: u32 = 0x73706A; // secondary text
const BORDER: u32 = 0xE7E4DA; // hairline border
const HOVER: u32 = 0xEBE8DE; // subtle warm hover bg
const PRIMARY: u32 = 0xC15F3C; // coral / terracotta accent
const PRIMARY_HOVER: u32 = 0xAD5435;
const WASH: u32 = 0xF3E7E0; // light coral selection wash
const WASH_BORDER: u32 = 0xE8D3C8;
const SUCCESS: u32 = 0x4F8A5B; // 2xx / GET
const DANGER: u32 = 0xC0503F; // 4xx-5xx / DELETE
const WARNING: u32 = 0xC98A3C; // amber / POST,PUT
const SCROLLBAR: u32 = 0xD8D4C8;

// ===== Layout dimensions (px) =====
#[allow(dead_code)]
pub const SIDEBAR_WIDTH: f32 = 264.;
#[allow(dead_code)]
pub const SIDEBAR_MIN: f32 = 200.;
#[allow(dead_code)]
pub const SIDEBAR_MAX: f32 = 420.;
#[allow(dead_code)]
pub const REQUEST_INITIAL_HEIGHT: f32 = 350.;
#[allow(dead_code)]
pub const REQUEST_MIN: f32 = 150.;
#[allow(dead_code)]
pub const REQUEST_MAX: f32 = 700.;
#[allow(dead_code)]
pub const METHOD_SELECT_WIDTH: f32 = 92.;
#[allow(dead_code)]
pub const RAW_SUBTYPE_WIDTH: f32 = 120.;

/// Convert a 0xRRGGBB literal into an Hsla theme color.
fn c(hex: u32) -> Hsla {
    gpui::rgb(hex).into()
}

/// Semantic color for an HTTP method label (used by tab bar + history).
#[allow(dead_code)]
pub fn method_color(method: HttpMethod, theme: &Theme) -> Hsla {
    match method {
        HttpMethod::GET => theme.success,
        HttpMethod::POST | HttpMethod::PUT | HttpMethod::PATCH => theme.warning,
        HttpMethod::DELETE => theme.danger,
        HttpMethod::HEAD | HttpMethod::OPTIONS => theme.muted_foreground,
    }
}

/// Apply the warm-light theme to the global Theme. Call once after
/// `gpui_component::init(cx)`.
pub fn apply_theme(cx: &mut App) {
    let theme = Theme::global_mut(cx);
    theme.mode = ThemeMode::Light;
    theme.radius = px(6.);
    theme.radius_lg = px(8.);

    // Surfaces & text
    theme.background = c(BACKGROUND);
    theme.foreground = c(FOREGROUND);
    theme.muted = c(SIDEBAR);
    theme.muted_foreground = c(MUTED_FG);
    theme.border = c(BORDER);
    theme.input = c(SURFACE);
    theme.popover = c(SURFACE);
    theme.popover_foreground = c(FOREGROUND);
    theme.secondary = c(SIDEBAR);
    theme.secondary_foreground = c(FOREGROUND);
    theme.secondary_hover = c(HOVER);
    theme.secondary_active = c(HOVER);

    // Sidebar
    theme.sidebar = c(SIDEBAR);
    theme.sidebar_foreground = c(FOREGROUND);
    theme.sidebar_border = c(BORDER);

    // Accent / primary (coral)
    theme.primary = c(PRIMARY);
    theme.primary_hover = c(PRIMARY_HOVER);
    theme.primary_active = c(PRIMARY_HOVER);
    theme.primary_foreground = c(SURFACE);
    theme.ring = c(PRIMARY);
    theme.caret = c(PRIMARY);
    theme.accent = c(WASH);
    theme.accent_foreground = c(FOREGROUND);
    theme.selection = c(WASH);

    // Lists (history rows)
    theme.list = c(SIDEBAR);
    theme.list_hover = c(HOVER);
    theme.list_active = c(WASH);
    theme.list_active_border = c(WASH_BORDER);

    // Tabs
    theme.tab = c(BACKGROUND);
    theme.tab_active = c(SURFACE);

    // Status semantics
    theme.success = c(SUCCESS);
    theme.success_foreground = c(SURFACE);
    theme.danger = c(DANGER);
    theme.danger_foreground = c(SURFACE);
    theme.danger_hover = c(DANGER);
    theme.danger_active = c(DANGER);
    theme.warning = c(WARNING);
    theme.warning_foreground = c(SURFACE);

    // Scrollbar
    theme.scrollbar_thumb = c(SCROLLBAR);
    theme.scrollbar_thumb_hover = c(MUTED_FG);
}
