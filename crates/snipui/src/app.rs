//! Main iced application: page routing, sidebar navigation, state management.

use crate::{ipc_client, pages, theme};
use crate::widgets::hotkey_input;
use iced::widget::{button, column, container, row, text, Space};
use iced::{Background, Border, Element, Length, Subscription, Task, Theme};
use ns_common::config::NsConfig;
use ns_common::history::HistoryEntry;
use ns_common::hotkey::{HotkeyCombination, Modifier};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Page {
    Settings,
    History,
    About,
}

#[derive(Debug, Clone)]
pub enum Message {
    NavigateTo(Page),
    // Settings messages
    Settings(pages::settings::Message),
    // History messages
    History(pages::history::Message),
    // About messages
    About(pages::about::Message),
    // IPC responses
    ConfigLoaded(NsConfig),
    ConfigSaved,
    HistoryLoaded(Vec<HistoryEntry>, u32, Vec<Option<Vec<u8>>>),
    EntryDeleted(uuid::Uuid),
    IpcError(String),
    HotkeyCaptured(HotkeyCombination),
    OpenSponsorLink,
    RefreshHistory,
}

pub struct NsApp {
    page: Page,
    config: NsConfig,
    settings_state: pages::settings::State,
    history_state: pages::history::State,
}

impl NsApp {
    pub fn new() -> (Self, Task<Message>) {
        let config = NsConfig::load();
        let initial_page = crate::INITIAL_PAGE
            .get()
            .map(|s| match s.as_str() {
                "history" => Page::History,
                "about" => Page::About,
                _ => Page::Settings,
            })
            .unwrap_or(Page::Settings);

        let app = Self {
            page: initial_page.clone(),
            config: config.clone(),
            settings_state: pages::settings::State::new(config),
            history_state: pages::history::State::new(),
        };

        // Try to load config from snipd via IPC on startup
        let load_config = Task::perform(
            async { ipc_client::get_config().await },
            |result| match result {
                Ok(config) => Message::ConfigLoaded(config),
                Err(e) => Message::IpcError(format!("Load config: {e}")),
            },
        );

        // Show the window after iced has rendered the first frame.
        // Window starts hidden (visible: false) to avoid the white flash from
        // the default Win32 class background brush.
        let show_window = iced::window::get_oldest().then(|maybe_id| {
            if let Some(id) = maybe_id {
                iced::window::run_with_handle(id, |handle| {
                    use iced::window::raw_window_handle::RawWindowHandle;
                    if let RawWindowHandle::Win32(win32) = handle.as_raw() {
                        let hwnd = win32.hwnd.get();
                        // BG_PRIMARY (0x1E, 0x1E, 0x23) in Win32 BGR COLORREF
                        const BG_DARK: u32 = 0x00231E1E;
                        const GCL_HBRBACKGROUND: i32 = -10;
                        const SW_SHOW: i32 = 5;
                        extern "system" {
                            fn CreateSolidBrush(color: u32) -> isize;
                            fn SetClassLongPtrW(hwnd: isize, index: i32, new: isize) -> isize;
                            fn ShowWindow(hwnd: isize, cmd: i32) -> i32;
                        }
                        unsafe {
                            let brush = CreateSolidBrush(BG_DARK);
                            SetClassLongPtrW(hwnd, GCL_HBRBACKGROUND, brush);
                            ShowWindow(hwnd, SW_SHOW);
                        }
                    }
                })
                .then(|_| Task::none())
            } else {
                Task::none()
            }
        });

        let mut tasks = vec![load_config, show_window];
        if initial_page == Page::History {
            let load_hist = Task::perform(
                async move { ipc_client::get_history(0, 20, None).await },
                |result| match result {
                    Ok((entries, total, thumbnails)) => Message::HistoryLoaded(entries, total, thumbnails),
                    Err(e) => Message::IpcError(format!("Load history: {e}")),
                },
            );
            tasks.push(load_hist);
        }
        (app, Task::batch(tasks))
    }

    pub fn title(&self) -> String {
        match self.page {
            Page::Settings => "Nano Snipper — Settings".into(),
            Page::History => "Nano Snipper — History".into(),
            Page::About => "Nano Snipper — About".into(),
        }
    }

    pub fn theme(&self) -> Theme {
        theme::nano_snipper_theme()
    }

    pub fn subscription(&self) -> Subscription<Message> {
        let mut subs = Vec::new();

        if self.settings_state.listening_action().is_some() {
            subs.push(iced::keyboard::on_key_press(|key, mods| {
                convert_key_to_hotkey(key, mods).map(Message::HotkeyCaptured)
            }));
        }

        if self.page == Page::History {
            subs.push(iced::time::every(std::time::Duration::from_secs(2)).map(|_| Message::RefreshHistory));
        }

        Subscription::batch(subs)
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::NavigateTo(page) => {
                self.page = page.clone();
                match page {
                    Page::History => self.load_history(),
                    Page::Settings | Page::About => Task::none(),
                }
            }

            Message::Settings(msg) => {
                let changed = self.settings_state.update(msg, &mut self.config);
                if changed {
                    let config = self.config.clone();
                    Task::perform(
                        async move { ipc_client::set_config(config).await },
                        |result| match result {
                            Ok(()) => Message::ConfigSaved,
                            Err(e) => Message::IpcError(format!("Save config: {e}")),
                        },
                    )
                } else {
                    Task::none()
                }
            }

            Message::History(msg) => {
                match &msg {
                    pages::history::Message::SearchChanged(_) => {
                        self.history_state.update(msg);
                        self.load_history()
                    }
                    pages::history::Message::LoadPage(_) => {
                        self.history_state.update(msg);
                        self.load_history()
                    }
                    pages::history::Message::DeleteEntry(id) => {
                        let id = *id;
                        self.history_state.update(msg);
                        Task::perform(
                            async move { ipc_client::delete_entry(id).await },
                            move |result| match result {
                                Ok(()) => Message::EntryDeleted(id),
                                Err(e) => Message::IpcError(format!("Delete: {e}")),
                            },
                        )
                    }
                    pages::history::Message::OpenEntry(id) => {
                        let id = *id;
                        self.history_state.update(msg);
                        // Open the capture file in the default viewer
                        if let Some(entry) = self.history_state.find_entry(&id) {
                            let path = ns_common::paths::captures_dir().join(&entry.file_path);
                            if path.exists() {
                                let _ = std::process::Command::new("cmd")
                                    .args(["/C", "start", "", &path.to_string_lossy()])
                                    .spawn();
                            }
                        }
                        Task::none()
                    }
                }
            }

            Message::ConfigLoaded(config) => {
                self.config = config.clone();
                self.settings_state = pages::settings::State::new(config);
                Task::none()
            }

            Message::ConfigSaved => {
                tracing::info!("Config saved to snipd");
                Task::none()
            }

            Message::HistoryLoaded(entries, total, thumbnails) => {
                self.history_state.set_entries(entries, total, thumbnails);
                Task::none()
            }

            Message::EntryDeleted(_id) => {
                // Reload history after delete
                self.load_history()
            }

            Message::HotkeyCaptured(combo) => {
                if let Some(action) = self.settings_state.listening_action() {
                    let changed = self.settings_state.update(
                        pages::settings::Message::HotkeyMsg(
                            action,
                            hotkey_input::Message::KeyCaptured(combo),
                        ),
                        &mut self.config,
                    );
                    if changed {
                        let config = self.config.clone();
                        return Task::perform(
                            async move { ipc_client::set_config(config).await },
                            |result| match result {
                                Ok(()) => Message::ConfigSaved,
                                Err(e) => Message::IpcError(format!("Save config: {e}")),
                            },
                        );
                    }
                }
                Task::none()
            }

            Message::RefreshHistory => self.load_history(),

            Message::About(pages::about::Message::OpenBitcoinCom) | Message::OpenSponsorLink => {
                let _ = std::process::Command::new("cmd")
                    .args(["/C", "start", "", "https://www.bitcoin.com"])
                    .spawn();
                Task::none()
            }

            Message::IpcError(err) => {
                tracing::error!("IPC error: {err}");
                Task::none()
            }
        }
    }

    fn load_history(&self) -> Task<Message> {
        let offset = self.history_state.offset();
        let limit = self.history_state.page_size();
        let search = self.history_state.search_query();
        Task::perform(
            async move { ipc_client::get_history(offset, limit, search).await },
            |result| match result {
                Ok((entries, total, thumbnails)) => {
                    Message::HistoryLoaded(entries, total, thumbnails)
                }
                Err(e) => Message::IpcError(format!("Load history: {e}")),
            },
        )
    }

    pub fn view(&self) -> Element<'_, Message> {
        let is_settings = self.page == Page::Settings;
        let is_history = self.page == Page::History;
        let is_about = self.page == Page::About;

        // ── Sidebar ──────────────────────────────────────────────────

        let title = text("Nano Snipper")
            .size(18)
            .font(theme::FONT_BOLD)
            .color(theme::TEXT_PRIMARY);

        let subtitle = column![
            text("Sponsored by")
                .size(11)
                .color(theme::TEXT_PRIMARY),
            button(
                iced::widget::image(iced::widget::image::Handle::from_bytes(
                    include_bytes!("../../../resources/bitcoincom_logo_light.png").as_slice(),
                ))
                .width(120),
            )
            .on_press(Message::OpenSponsorLink)
            .style(theme::invisible_button)
            .padding(0),
        ]
        .spacing(4);

        let settings_nav = nav_item("Settings", is_settings, Message::NavigateTo(Page::Settings));
        let history_nav = nav_item("History", is_history, Message::NavigateTo(Page::History));
        let about_nav = nav_item("About", is_about, Message::NavigateTo(Page::About));

        let sidebar = container(
            column![
                // Brand area
                container(column![title, subtitle].spacing(8))
                    .padding(iced::Padding { top: 24.0, right: 20.0, bottom: 16.0, left: 20.0 }),
                // Divider
                container(Space::new(Length::Fill, 1))
                    .style(|_: &Theme| container::Style {
                        background: Some(Background::Color(theme::BORDER_SUBTLE)),
                        ..container::Style::default()
                    })
                    .width(Length::Fill)
                    .padding([0, 12]),
                // Nav items
                column![settings_nav, history_nav, about_nav]
                    .spacing(2)
                    .padding([12, 8]),
            ]
            .spacing(0),
        )
        .style(theme::sidebar)
        .width(Length::Fixed(200.0))
        .height(Length::Fill);

        // ── Content ──────────────────────────────────────────────────

        let content: Element<Message> = match self.page {
            Page::Settings => self
                .settings_state
                .view(&self.config)
                .map(Message::Settings),
            Page::History => self.history_state.view().map(Message::History),
            Page::About => pages::about::view().map(Message::About),
        };

        let content_area = container(content)
            .style(theme::content_area)
            .width(Length::Fill)
            .height(Length::Fill);

        // ── Layout ───────────────────────────────────────────────────

        row![sidebar, content_area]
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }
}

/// Build a sidebar navigation item with optional active accent bar.
fn nav_item(label: &str, is_active: bool, msg: Message) -> Element<'_, Message> {
    let accent_bar = container(Space::new(3, 24)).style(move |_: &Theme| container::Style {
        background: if is_active {
            Some(Background::Color(theme::ACCENT))
        } else {
            None
        },
        border: Border {
            radius: 1.5.into(),
            ..Border::default()
        },
        ..container::Style::default()
    });

    let style_fn: fn(&Theme, button::Status) -> button::Style = if is_active {
        theme::nav_button_active
    } else {
        theme::nav_button_inactive
    };

    let btn = button(
        text(label)
            .size(14)
            .color(if is_active {
                theme::TEXT_PRIMARY
            } else {
                theme::TEXT_SECONDARY
            }),
    )
    .style(style_fn)
    .on_press(msg)
    .padding([8, 12])
    .width(Length::Fill);

    row![accent_bar, btn]
        .spacing(4)
        .align_y(iced::Alignment::Center)
        .into()
}

/// Convert an iced keyboard event into a `HotkeyCombination`.
/// Returns `None` for bare modifier keys (wait for an actual key).
fn convert_key_to_hotkey(
    key: iced::keyboard::Key,
    mods: iced::keyboard::Modifiers,
) -> Option<HotkeyCombination> {
    let vk = match key {
        iced::keyboard::Key::Character(ref c) => {
            let ch = c.to_uppercase().chars().next()?;
            match ch {
                'A'..='Z' => ch as u32,
                '0'..='9' => ch as u32,
                // Shift+digit produces these symbols on US keyboards;
                // map them back to their VK digit codes.
                '!' => 0x31, // Shift+1
                '@' => 0x32, // Shift+2
                '#' => 0x33, // Shift+3
                '$' => 0x34, // Shift+4
                '%' => 0x35, // Shift+5
                '^' => 0x36, // Shift+6
                '&' => 0x37, // Shift+7
                '*' => 0x38, // Shift+8
                '(' => 0x39, // Shift+9
                ')' => 0x30, // Shift+0
                _ => return None,
            }
        }
        iced::keyboard::Key::Named(named) => {
            use iced::keyboard::key::Named::*;
            match named {
                F1 => 0x70,
                F2 => 0x71,
                F3 => 0x72,
                F4 => 0x73,
                F5 => 0x74,
                F6 => 0x75,
                F7 => 0x76,
                F8 => 0x77,
                F9 => 0x78,
                F10 => 0x79,
                F11 => 0x7A,
                F12 => 0x7B,
                Space => 0x20,
                Enter => 0x0D,
                Tab => 0x09,
                ArrowUp => 0x26,
                ArrowDown => 0x28,
                ArrowLeft => 0x25,
                ArrowRight => 0x27,
                Home => 0x24,
                End => 0x23,
                PageUp => 0x21,
                PageDown => 0x22,
                Insert => 0x2D,
                Delete => 0x2E,
                Backspace => 0x08,
                // Ignore bare modifier presses
                Shift | Control | Alt | Super => return None,
                _ => return None,
            }
        }
        _ => return None,
    };

    let mut modifiers = Vec::new();
    if mods.control() {
        modifiers.push(Modifier::Ctrl);
    }
    if mods.shift() {
        modifiers.push(Modifier::Shift);
    }
    if mods.alt() {
        modifiers.push(Modifier::Alt);
    }
    if mods.logo() {
        modifiers.push(Modifier::Win);
    }

    Some(HotkeyCombination { modifiers, key: vk })
}
