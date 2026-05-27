// WinUI3-inspired theme tokens, fonts, and widget style helpers used by the
// settings window. Both Dark and Light palettes are provided; the active
// palette is selected per `Mode`.

use iced::font::{Family, Stretch, Style as FontStyle, Weight};
use iced::theme::Palette;
use iced::widget::{button, container};
use iced::{Background, Border, Color, Font, Shadow, Theme};

// Fonts embedded in the binary. The settings window registers all three at
// startup so iced/cosmic-text can match by family + weight.
pub const FLUENT_ICONS_BYTES: &[u8] =
    include_bytes!("../../assets/fonts/FluentSystemIcons-Regular.ttf");
pub const PRETENDARD_MEDIUM_BYTES: &[u8] =
    include_bytes!("../../assets/fonts/PretendardJP-Medium.otf");
pub const PRETENDARD_SEMIBOLD_BYTES: &[u8] =
    include_bytes!("../../assets/fonts/PretendardJP-SemiBold.otf");

const PRETENDARD_FAMILY: &str = "Pretendard JP";

pub const ICON_FONT: Font = Font::with_name("FluentSystemIcons-Regular");

pub const BODY_FONT: Font = Font {
    family: Family::Name(PRETENDARD_FAMILY),
    weight: Weight::Medium,
    stretch: Stretch::Normal,
    style: FontStyle::Normal,
};

pub const HEADING_FONT: Font = Font {
    family: Family::Name(PRETENDARD_FAMILY),
    weight: Weight::Semibold,
    stretch: Stretch::Normal,
    style: FontStyle::Normal,
};

pub const BODY_SIZE: f32 = 14.0;
pub const HEADING_SIZE: f32 = 16.0;
pub const SECTION_TITLE_SIZE: f32 = 14.0;
pub const CAPTION_SIZE: f32 = 12.0;
pub const ICON_SIZE: f32 = 14.0;
pub const STATUS_DOT_SIZE: f32 = 10.0;

// Fluent UI System Icons (Regular, 16px design size). Code points verified
// against
// https://raw.githubusercontent.com/microsoft/fluentui-system-icons/main/fonts/FluentSystemIcons-Regular.json
// — earlier values were off; many didn't even point at glyphs that exist.
pub const ICON_SHIELD: &str = "\u{EAC3}"; // ic_fluent_shield_16_regular
pub const ICON_SETTINGS: &str = "\u{F6A8}"; // ic_fluent_settings_16_regular
pub const ICON_INFO: &str = "\u{F4A2}"; // ic_fluent_info_16_regular
pub const ICON_EYE_OFF: &str = "\u{E5F4}"; // ic_fluent_eye_off_16_regular
pub const ICON_ROCKET: &str = "\u{F676}"; // ic_fluent_rocket_16_regular
pub const ICON_ARROW_SYNC: &str = "\u{E110}"; // ic_fluent_arrow_sync_16_regular
pub const ICON_CIRCLE: &str = "\u{F2BA}"; // ic_fluent_circle_16_regular
pub const ICON_OPEN: &str = "\u{F581}"; // ic_fluent_open_16_regular

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Dark,
    Light,
}

/// Flat palette of color tokens for one mode. Captured by widget style
/// closures so the active palette flows through the whole view tree.
#[derive(Debug, Clone, Copy)]
pub struct Tokens {
    pub background: Color,
    pub card: Color,
    pub card_hover: Color,
    pub text_primary: Color,
    pub text_secondary: Color,
    pub accent: Color,
    pub accent_hover: Color,
    pub accent_pressed: Color,
    pub toggle_off: Color,
    pub divider: Color,
    pub danger: Color,
    pub success: Color,
}

impl Tokens {
    pub const fn dark() -> Self {
        Self {
            background: rgb(0x20, 0x20, 0x20),
            card: rgb(0x2D, 0x2D, 0x2D),
            card_hover: rgb(0x38, 0x38, 0x38),
            text_primary: rgb(0xFF, 0xFF, 0xFF),
            text_secondary: rgb(0x9E, 0x9E, 0x9E),
            accent: rgb(0x00, 0x78, 0xD4),
            accent_hover: rgb(0x1A, 0x8A, 0xD4),
            accent_pressed: rgb(0x00, 0x60, 0xAA),
            toggle_off: rgb(0x78, 0x78, 0x78),
            divider: rgb(0x3D, 0x3D, 0x3D),
            danger: rgb(0xFF, 0x44, 0x44),
            success: rgb(0x6C, 0xCB, 0x5F),
        }
    }

    pub const fn light() -> Self {
        Self {
            background: rgb(0xF3, 0xF3, 0xF3),
            card: rgb(0xFF, 0xFF, 0xFF),
            card_hover: rgb(0xF5, 0xF5, 0xF5),
            text_primary: rgb(0x1F, 0x1F, 0x1F),
            text_secondary: rgb(0x60, 0x60, 0x60),
            accent: rgb(0x00, 0x67, 0xC0),
            accent_hover: rgb(0x00, 0x55, 0xA8),
            accent_pressed: rgb(0x00, 0x44, 0x88),
            toggle_off: rgb(0xC8, 0xC8, 0xC8),
            divider: rgb(0xE0, 0xE0, 0xE0),
            danger: rgb(0xCC, 0x33, 0x33),
            success: rgb(0x10, 0x82, 0x40),
        }
    }

    pub const fn for_mode(mode: Mode) -> Self {
        match mode {
            Mode::Dark => Self::dark(),
            Mode::Light => Self::light(),
        }
    }
}

pub fn theme_for(mode: Mode) -> Theme {
    let t = Tokens::for_mode(mode);
    let palette = Palette {
        background: t.background,
        text: t.text_primary,
        primary: t.accent,
        success: t.success,
        danger: t.danger,
    };
    let name = match mode {
        Mode::Dark => "WinUI3 Dark",
        Mode::Light => "WinUI3 Light",
    };
    Theme::custom(name.to_string(), palette)
}

/// Semi-transparent root tint so the DWM Mica backdrop applied to the window
/// shows through with the same layered feel as WinUI surfaces. We still set
/// `text_color` so child widgets inherit the right foreground color.
pub fn root_container(tokens: Tokens) -> impl Fn(&Theme) -> container::Style + 'static {
    move |_| container::Style {
        background: Some(Background::Color(mica_root_fill(tokens))),
        text_color: Some(tokens.text_primary),
        border: Border::default(),
        shadow: Shadow::default(),
    }
}

pub fn card_container(tokens: Tokens) -> impl Fn(&Theme) -> container::Style + 'static {
    move |_| container::Style {
        background: Some(Background::Color(mica_card_fill(tokens))),
        text_color: Some(tokens.text_primary),
        border: Border {
            color: with_alpha(tokens.divider, mica_border_alpha(tokens)),
            width: 1.0,
            radius: 8.0.into(),
        },
        shadow: Shadow::default(),
    }
}

pub fn primary_button(
    tokens: Tokens,
) -> impl Fn(&Theme, button::Status) -> button::Style + 'static {
    move |_, status| {
        let bg = match status {
            button::Status::Active => tokens.accent,
            button::Status::Hovered => tokens.accent_hover,
            button::Status::Pressed => tokens.accent_pressed,
            button::Status::Disabled => with_alpha(tokens.accent, 0.4),
        };
        button::Style {
            background: Some(Background::Color(bg)),
            text_color: Color::WHITE,
            border: Border {
                color: Color::TRANSPARENT,
                width: 0.0,
                radius: 4.0.into(),
            },
            shadow: Shadow::default(),
        }
    }
}

pub fn secondary_button(
    tokens: Tokens,
) -> impl Fn(&Theme, button::Status) -> button::Style + 'static {
    move |_, status| {
        let bg = match status {
            button::Status::Active => mica_card_fill(tokens),
            button::Status::Hovered => with_alpha(tokens.card_hover, mica_hover_alpha(tokens)),
            button::Status::Pressed => with_alpha(tokens.divider, mica_pressed_alpha(tokens)),
            button::Status::Disabled => with_alpha(tokens.card, 0.35),
        };
        button::Style {
            background: Some(Background::Color(bg)),
            text_color: tokens.text_primary,
            border: Border {
                color: with_alpha(tokens.divider, mica_border_alpha(tokens)),
                width: 1.0,
                radius: 4.0.into(),
            },
            shadow: Shadow::default(),
        }
    }
}

/// Pill track for the custom toggle (iced 0.13's `toggler` hard-codes
/// `radius = height / (32/13)`, which only renders as a soft-cornered
/// rectangle — never a real pill — so we draw our own).
pub fn pill_track(tokens: Tokens, on: bool, height: f32) -> impl Fn(&Theme) -> container::Style {
    let bg = if on { tokens.accent } else { tokens.toggle_off };
    move |_| container::Style {
        background: Some(Background::Color(bg)),
        text_color: Some(Color::WHITE),
        border: Border {
            color: Color::TRANSPARENT,
            width: 0.0,
            radius: (height / 2.0).into(),
        },
        shadow: Shadow::default(),
    }
}

pub fn pill_thumb(diameter: f32) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(Background::Color(Color::WHITE)),
        text_color: None,
        border: Border {
            color: Color::TRANSPARENT,
            width: 0.0,
            radius: (diameter / 2.0).into(),
        },
        shadow: Shadow::default(),
    }
}

const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color {
        r: r as f32 / 255.0,
        g: g as f32 / 255.0,
        b: b as f32 / 255.0,
        a: 1.0,
    }
}

const fn is_light(tokens: Tokens) -> bool {
    tokens.background.r > 0.5
}

const fn mica_root_fill(tokens: Tokens) -> Color {
    if is_light(tokens) {
        with_alpha(tokens.background, 0.42)
    } else {
        with_alpha(tokens.background, 0.36)
    }
}

const fn mica_card_fill(tokens: Tokens) -> Color {
    if is_light(tokens) {
        with_alpha(tokens.card, 0.68)
    } else {
        with_alpha(tokens.card, 0.58)
    }
}

const fn mica_hover_alpha(tokens: Tokens) -> f32 {
    if is_light(tokens) { 0.76 } else { 0.66 }
}

const fn mica_pressed_alpha(tokens: Tokens) -> f32 {
    if is_light(tokens) { 0.70 } else { 0.62 }
}

const fn mica_border_alpha(tokens: Tokens) -> f32 {
    if is_light(tokens) { 0.70 } else { 0.85 }
}

const fn with_alpha(color: Color, alpha: f32) -> Color {
    Color {
        r: color.r,
        g: color.g,
        b: color.b,
        a: alpha,
    }
}
