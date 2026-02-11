//! Custom widget for capturing hotkey combinations.
//! Displays current binding and enters "listening" mode on click.

use crate::theme;
use iced::widget::{button, row, text};
use iced::Element;
use ns_common::hotkey::HotkeyCombination;

#[derive(Debug, Clone)]
pub enum Message {
    StartListening,
    StopListening,
    KeyCaptured(HotkeyCombination),
}

pub struct HotkeyInput {
    pub label: String,
    pub current: HotkeyCombination,
    pub listening: bool,
}

impl HotkeyInput {
    pub fn new(label: impl Into<String>, current: HotkeyCombination) -> Self {
        Self {
            label: label.into(),
            current,
            listening: false,
        }
    }

    pub fn update(&mut self, msg: Message) {
        match msg {
            Message::StartListening => self.listening = true,
            Message::StopListening => self.listening = false,
            Message::KeyCaptured(combo) => {
                self.current = combo;
                self.listening = false;
            }
        }
    }

    pub fn view(&self) -> Element<'_, Message> {
        let display = if self.listening {
            "Press a key combination...".to_string()
        } else {
            format_hotkey(&self.current)
        };

        let style_fn = if self.listening {
            theme::hotkey_button_listening as fn(&iced::Theme, button::Status) -> button::Style
        } else {
            theme::hotkey_button
        };

        let display_text = text(display)
            .size(13)
            .font(theme::FONT_MONO)
            .color(if self.listening {
                theme::ACCENT
            } else {
                theme::TEXT_PRIMARY
            });

        let label = text(self.label.clone())
            .size(14)
            .color(theme::TEXT_PRIMARY);

        row![
            label,
            iced::widget::Space::with_width(iced::Length::Fill),
            button(display_text)
                .on_press(if self.listening {
                    Message::StopListening
                } else {
                    Message::StartListening
                })
                .style(style_fn)
                .padding([6, 12])
                .width(iced::Length::Shrink),
        ]
        .spacing(12)
        .align_y(iced::Alignment::Center)
        .into()
    }
}

fn format_hotkey(combo: &HotkeyCombination) -> String {
    let mut result = String::new();
    for (i, m) in combo.modifiers.iter().enumerate() {
        if i > 0 {
            result.push_str(" + ");
        }
        result.push_str(match m {
            ns_common::hotkey::Modifier::Win => "Win",
            ns_common::hotkey::Modifier::Ctrl => "Ctrl",
            ns_common::hotkey::Modifier::Shift => "Shift",
            ns_common::hotkey::Modifier::Alt => "Alt",
        });
    }
    if !combo.modifiers.is_empty() {
        result.push_str(" + ");
    }
    match combo.key {
        0x30..=0x39 => result.push((combo.key as u8) as char),
        0x41..=0x5A => result.push((combo.key as u8) as char),
        _ => result.push_str(&format!("0x{:X}", combo.key)),
    }
    result
}
