//! Windows 11 Fluent Design-inspired custom theme for snipui.
//! Defines the color palette, font constants, and style functions used
//! by all pages and widgets.

use iced::widget::{button, container, text_input};
use iced::{Background, Border, Color, Font, Shadow, Theme};

// ── Color palette ────────────────────────────────────────────────────

// Backgrounds (dark layers, lightest = most elevated)
pub const BG_PRIMARY: Color = color(0x1E, 0x1E, 0x23);
pub const BG_SIDEBAR: Color = color(0x16, 0x16, 0x1A);
pub const BG_SURFACE: Color = color(0x2B, 0x2B, 0x30);
pub const BG_ELEVATED: Color = color(0x35, 0x35, 0x3A);

// Borders
pub const BORDER_SUBTLE: Color = color(0x3E, 0x3E, 0x45);

// Text
pub const TEXT_PRIMARY: Color = color(0xF2, 0xF2, 0xF7);
pub const TEXT_SECONDARY: Color = color(0x99, 0x99, 0xA6);
pub const TEXT_TERTIARY: Color = color(0x66, 0x66, 0x70);

// Accent
pub const ACCENT: Color = color(0x33, 0x66, 0xCC);

// Semantic
pub const DANGER: Color = color(0xCC, 0x44, 0x44);
pub const DANGER_HOVER: Color = color(0xDD, 0x55, 0x55);
pub const SUCCESS: Color = color(0x44, 0xAA, 0x44);

// ── Fonts ────────────────────────────────────────────────────────────

pub const FONT_SEGOE: Font = Font {
    family: iced::font::Family::Name("Segoe UI"),
    weight: iced::font::Weight::Normal,
    stretch: iced::font::Stretch::Normal,
    style: iced::font::Style::Normal,
};

pub const FONT_SEMIBOLD: Font = Font {
    family: iced::font::Family::Name("Segoe UI"),
    weight: iced::font::Weight::Semibold,
    stretch: iced::font::Stretch::Normal,
    style: iced::font::Style::Normal,
};

pub const FONT_BOLD: Font = Font {
    family: iced::font::Family::Name("Segoe UI"),
    weight: iced::font::Weight::Bold,
    stretch: iced::font::Stretch::Normal,
    style: iced::font::Style::Normal,
};

pub const FONT_MONO: Font = Font {
    family: iced::font::Family::Name("Cascadia Mono"),
    weight: iced::font::Weight::Normal,
    stretch: iced::font::Stretch::Normal,
    style: iced::font::Style::Normal,
};

// ── Theme constructor ────────────────────────────────────────────────

pub fn nano_snipper_theme() -> Theme {
    Theme::custom(
        "Nano Snipper".to_string(),
        iced::theme::Palette {
            background: BG_PRIMARY,
            text: TEXT_PRIMARY,
            primary: ACCENT,
            success: SUCCESS,
            danger: DANGER,
        },
    )
}

// ── Container styles ─────────────────────────────────────────────────

/// Card container — used for grouped settings sections and history entries.
pub fn card(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(BG_SURFACE)),
        border: Border {
            color: BORDER_SUBTLE,
            width: 1.0,
            radius: 8.0.into(),
        },
        shadow: Shadow::default(),
        text_color: None,
    }
}

/// Sidebar background.
pub fn sidebar(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(BG_SIDEBAR)),
        border: Border::default(),
        shadow: Shadow::default(),
        text_color: None,
    }
}

/// Main content area background.
pub fn content_area(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(BG_PRIMARY)),
        border: Border::default(),
        shadow: Shadow::default(),
        text_color: None,
    }
}

/// Thumbnail container with rounded corners.
pub fn thumbnail(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(color(0x20, 0x20, 0x25))),
        border: Border {
            color: BORDER_SUBTLE,
            width: 1.0,
            radius: 4.0.into(),
        },
        shadow: Shadow::default(),
        text_color: None,
    }
}

// ── Button styles ────────────────────────────────────────────────────

/// Sidebar nav button — active (selected) page.
pub fn nav_button_active(_theme: &Theme, status: button::Status) -> button::Style {
    let bg = match status {
        button::Status::Hovered | button::Status::Pressed => BG_ELEVATED,
        _ => BG_SURFACE,
    };
    button::Style {
        background: Some(Background::Color(bg)),
        text_color: TEXT_PRIMARY,
        border: Border {
            radius: 6.0.into(),
            ..Border::default()
        },
        shadow: Shadow::default(),
    }
}

/// Sidebar nav button — inactive page.
pub fn nav_button_inactive(_theme: &Theme, status: button::Status) -> button::Style {
    let bg = match status {
        button::Status::Hovered | button::Status::Pressed => BG_ELEVATED,
        _ => Color::TRANSPARENT,
    };
    button::Style {
        background: Some(Background::Color(bg)),
        text_color: TEXT_SECONDARY,
        border: Border {
            radius: 6.0.into(),
            ..Border::default()
        },
        shadow: Shadow::default(),
    }
}


/// Danger button (Delete) — text-only until hovered.
pub fn danger_button(_theme: &Theme, status: button::Status) -> button::Style {
    let (bg, txt) = match status {
        button::Status::Active => (Color::TRANSPARENT, DANGER),
        button::Status::Hovered => (DANGER, Color::WHITE),
        button::Status::Pressed => (DANGER_HOVER, Color::WHITE),
        button::Status::Disabled => (
            Color::TRANSPARENT,
            Color { a: 0.5, ..DANGER },
        ),
    };
    button::Style {
        background: Some(Background::Color(bg)),
        text_color: txt,
        border: Border {
            radius: 4.0.into(),
            ..Border::default()
        },
        shadow: Shadow::default(),
    }
}

/// Ghost/text button — transparent idle, elevated on hover.
pub fn ghost_button(_theme: &Theme, status: button::Status) -> button::Style {
    let (bg, txt) = match status {
        button::Status::Active => (Color::TRANSPARENT, TEXT_PRIMARY),
        button::Status::Hovered => (BG_ELEVATED, TEXT_PRIMARY),
        button::Status::Pressed => (BG_SURFACE, TEXT_PRIMARY),
        button::Status::Disabled => (Color::TRANSPARENT, TEXT_TERTIARY),
    };
    button::Style {
        background: Some(Background::Color(bg)),
        text_color: txt,
        border: Border {
            radius: 4.0.into(),
            ..Border::default()
        },
        shadow: Shadow::default(),
    }
}

/// Invisible button — no background change on any state.
pub fn invisible_button(_theme: &Theme, _status: button::Status) -> button::Style {
    button::Style {
        background: Some(Background::Color(Color::TRANSPARENT)),
        text_color: TEXT_PRIMARY,
        border: Border::default(),
        shadow: Shadow::default(),
    }
}

/// Hotkey input button — normal state.
pub fn hotkey_button(_theme: &Theme, status: button::Status) -> button::Style {
    let bg = match status {
        button::Status::Hovered | button::Status::Pressed => BG_ELEVATED,
        _ => BG_SURFACE,
    };
    button::Style {
        background: Some(Background::Color(bg)),
        text_color: TEXT_PRIMARY,
        border: Border {
            color: BORDER_SUBTLE,
            width: 1.0,
            radius: 4.0.into(),
        },
        shadow: Shadow::default(),
    }
}

/// Hotkey input button — listening/recording state.
pub fn hotkey_button_listening(_theme: &Theme, _status: button::Status) -> button::Style {
    button::Style {
        background: Some(Background::Color(BG_SURFACE)),
        text_color: ACCENT,
        border: Border {
            color: ACCENT,
            width: 2.0,
            radius: 4.0.into(),
        },
        shadow: Shadow::default(),
    }
}

// ── Text input styles ────────────────────────────────────────────────

/// Search bar input.
pub fn search_input(_theme: &Theme, status: text_input::Status) -> text_input::Style {
    let border_color = match status {
        text_input::Status::Focused => ACCENT,
        text_input::Status::Hovered => TEXT_TERTIARY,
        _ => BORDER_SUBTLE,
    };
    text_input::Style {
        background: Background::Color(BG_SURFACE),
        border: Border {
            color: border_color,
            width: 1.0,
            radius: 6.0.into(),
        },
        icon: TEXT_SECONDARY,
        placeholder: TEXT_TERTIARY,
        value: TEXT_PRIMARY,
        selection: ACCENT,
    }
}

// ── Scrollable styles ────────────────────────────────────────────────

use iced::widget::scrollable;

/// Thin sleek scrollbar.
pub fn thin_scrollbar(_theme: &Theme, status: scrollable::Status) -> scrollable::Style {
    let scrollbar_color = match status {
        scrollable::Status::Hovered { .. } | scrollable::Status::Dragged { .. } => BG_ELEVATED,
        _ => Color::TRANSPARENT,
    };
    let scroller_color = match status {
        scrollable::Status::Hovered { is_vertical_scrollbar_hovered, .. } => {
            if is_vertical_scrollbar_hovered { TEXT_SECONDARY } else { TEXT_TERTIARY }
        }
        scrollable::Status::Dragged { .. } => TEXT_SECONDARY,
        scrollable::Status::Active => TEXT_TERTIARY,
    };
    scrollable::Style {
        container: container::Style::default(),
        vertical_rail: scrollable::Rail {
            background: Some(Background::Color(scrollbar_color)),
            border: Border::default(),
            scroller: scrollable::Scroller {
                color: scroller_color,
                border: Border {
                    radius: 2.0.into(),
                    ..Border::default()
                },
            },
        },
        horizontal_rail: scrollable::Rail {
            background: None,
            border: Border::default(),
            scroller: scrollable::Scroller {
                color: Color::TRANSPARENT,
                border: Border::default(),
            },
        },
        gap: None,
    }
}

// ── Helper ───────────────────────────────────────────────────────────

/// Build a `Color` from 8-bit RGB values (const-compatible).
const fn color(r: u8, g: u8, b: u8) -> Color {
    Color {
        r: r as f32 / 255.0,
        g: g as f32 / 255.0,
        b: b as f32 / 255.0,
        a: 1.0,
    }
}
