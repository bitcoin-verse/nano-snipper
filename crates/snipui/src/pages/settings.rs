//! Settings page: hotkey remapping, retention config, behavior settings.
//! Grouped into card sections following Windows 11 Fluent Design.

use crate::theme;
use crate::widgets::hotkey_input::{self, HotkeyInput};
use iced::widget::{column, container, row, scrollable, slider, text, toggler, Space};
use iced::{Element, Length};
use ns_common::config::NsConfig;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyAction {
    Region,
    Fullscreen,
}

#[derive(Debug, Clone)]
pub enum Message {
    AutoCopyTimeoutChanged(u32),
    MaxAgeDaysChanged(u32),
    MaxSizeMbChanged(u32),
    StartOnLoginToggled(bool),
    HotkeyMsg(HotkeyAction, hotkey_input::Message),
}

pub struct State {
    auto_copy_timeout: u32,
    max_age_days: u32,
    max_size_mb: u64,
    start_on_login: bool,
    hk_region: HotkeyInput,
    hk_fullscreen: HotkeyInput,
}

impl State {
    pub fn new(config: NsConfig) -> Self {
        Self {
            auto_copy_timeout: config.behavior.auto_copy_timeout_secs,
            max_age_days: config.retention.max_age_days,
            max_size_mb: config.retention.max_size_mb,
            start_on_login: config.behavior.start_on_login,
            hk_region: HotkeyInput::new("Region capture", config.hotkeys.region.clone()),
            hk_fullscreen: HotkeyInput::new("Fullscreen capture", config.hotkeys.fullscreen.clone()),
        }
    }

    /// Update state from a message. Returns `true` if the config was modified
    /// and should be persisted to nanosnipper via IPC.
    pub fn update(&mut self, msg: Message, config: &mut NsConfig) -> bool {
        match msg {
            Message::AutoCopyTimeoutChanged(v) => {
                self.auto_copy_timeout = v;
                config.behavior.auto_copy_timeout_secs = v;
                true
            }
            Message::MaxAgeDaysChanged(v) => {
                self.max_age_days = v;
                config.retention.max_age_days = v;
                true
            }
            Message::MaxSizeMbChanged(v) => {
                self.max_size_mb = v as u64;
                config.retention.max_size_mb = v as u64;
                true
            }
            Message::StartOnLoginToggled(v) => {
                self.start_on_login = v;
                config.behavior.start_on_login = v;
                true
            }
            Message::HotkeyMsg(action, inner) => {
                let was_capture = matches!(inner, hotkey_input::Message::KeyCaptured(_));
                let widget = match action {
                    HotkeyAction::Region => &mut self.hk_region,
                    HotkeyAction::Fullscreen => &mut self.hk_fullscreen,
                };
                widget.update(inner);
                if was_capture {
                    match action {
                        HotkeyAction::Region => config.hotkeys.region = widget.current.clone(),
                        HotkeyAction::Fullscreen => {
                            config.hotkeys.fullscreen = widget.current.clone()
                        }
                    }
                    true
                } else {
                    false
                }
            }
        }
    }

    /// Returns which hotkey action is currently listening, if any.
    pub fn listening_action(&self) -> Option<HotkeyAction> {
        if self.hk_region.listening {
            Some(HotkeyAction::Region)
        } else if self.hk_fullscreen.listening {
            Some(HotkeyAction::Fullscreen)
        } else {
            None
        }
    }

    pub fn view(&self, _config: &NsConfig) -> Element<'_, Message> {
        // ── Hotkeys card ─────────────────────────────────────────────

        let hotkeys_card = container(
            column![
                text("Hotkeys")
                    .size(16)
                    .font(theme::FONT_SEMIBOLD)
                    .color(theme::TEXT_PRIMARY),
                text("Configure global keyboard shortcuts for capture modes")
                    .size(12)
                    .color(theme::TEXT_SECONDARY),
                column![
                    self.hk_region
                        .view()
                        .map(|m| Message::HotkeyMsg(HotkeyAction::Region, m)),
                    self.hk_fullscreen
                        .view()
                        .map(|m| Message::HotkeyMsg(HotkeyAction::Fullscreen, m)),
                ]
                .spacing(6),
            ]
            .spacing(10),
        )
        .style(theme::card)
        .padding(16)
        .width(Length::Fill);

        // ── Behavior card ────────────────────────────────────────────

        let timeout_value = if self.auto_copy_timeout == 0 {
            "Off".to_string()
        } else {
            format!("{} sec", self.auto_copy_timeout)
        };

        let timeout_control = column![
            row![
                text("Auto-copy timeout").color(theme::TEXT_PRIMARY),
                Space::with_width(Length::Fill),
                text(timeout_value)
                    .size(13)
                    .color(theme::TEXT_SECONDARY),
            ]
            .align_y(iced::Alignment::Center),
            text("Seconds before the capture is automatically copied to clipboard")
                .size(12)
                .color(theme::TEXT_TERTIARY),
            slider(0..=30, self.auto_copy_timeout, Message::AutoCopyTimeoutChanged),
        ]
        .spacing(4);

        let behavior_card = container(
            column![
                text("Behavior")
                    .size(16)
                    .font(theme::FONT_SEMIBOLD)
                    .color(theme::TEXT_PRIMARY),
                timeout_control,
                toggler(self.start_on_login)
                    .label("Start on login")
                    .on_toggle(Message::StartOnLoginToggled),
            ]
            .spacing(12),
        )
        .style(theme::card)
        .padding(16)
        .width(Length::Fill);

        // ── Retention card ───────────────────────────────────────────

        let age_value = if self.max_age_days == 0 {
            "Unlimited".to_string()
        } else {
            format!("{} days", self.max_age_days)
        };

        let size_value = if self.max_size_mb == 0 {
            "Unlimited".to_string()
        } else if self.max_size_mb >= 1024 {
            format!("{:.1} GB", self.max_size_mb as f64 / 1024.0)
        } else {
            format!("{} MB", self.max_size_mb)
        };

        let retention_card = container(
            column![
                text("Retention")
                    .size(16)
                    .font(theme::FONT_SEMIBOLD)
                    .color(theme::TEXT_PRIMARY),
                text("Manage storage used by capture history")
                    .size(12)
                    .color(theme::TEXT_SECONDARY),
                column![
                    row![
                        text("Max age").color(theme::TEXT_PRIMARY),
                        Space::with_width(Length::Fill),
                        text(age_value)
                            .size(13)
                            .color(theme::TEXT_SECONDARY),
                    ]
                    .align_y(iced::Alignment::Center),
                    slider(0..=90, self.max_age_days, Message::MaxAgeDaysChanged),
                ]
                .spacing(4),
                column![
                    row![
                        text("Max size").color(theme::TEXT_PRIMARY),
                        Space::with_width(Length::Fill),
                        text(size_value)
                            .size(13)
                            .color(theme::TEXT_SECONDARY),
                    ]
                    .align_y(iced::Alignment::Center),
                    slider(
                        0..=8192u32,
                        self.max_size_mb as u32,
                        Message::MaxSizeMbChanged
                    ),
                ]
                .spacing(4),
            ]
            .spacing(12),
        )
        .style(theme::card)
        .padding(16)
        .width(Length::Fill);

        // ── Scrollable layout ────────────────────────────────────────

        let content = scrollable(
            column![hotkeys_card, behavior_card, retention_card]
                .spacing(16)
                .padding(24)
                .width(Length::Fill),
        )
        .height(Length::Fill);

        container(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }
}
