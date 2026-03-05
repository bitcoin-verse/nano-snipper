//! History page: capture list with thumbnails, search, and pagination.

use crate::theme;
use iced::widget::image::Handle;
use iced::widget::{button, column, container, row, scrollable, text, text_input, Space};
use iced::{Element, Length};
use ns_common::history::HistoryEntry;

#[derive(Debug, Clone)]
pub enum Message {
    SearchChanged(String),
    LoadPage(u32),
    DeleteEntry(uuid::Uuid),
    OpenEntry(uuid::Uuid),
    DeleteAll,
    OpenFolder,
}

pub struct State {
    entries: Vec<HistoryEntry>,
    thumbnails: Vec<Option<Vec<u8>>>,
    /// Cached image handles so we don't re-create them on every view() call.
    thumb_handles: Vec<Option<Handle>>,
    total: u32,
    current_page: u32,
    search_query: String,
    page_size: u32,
}

impl State {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            thumbnails: Vec::new(),
            thumb_handles: Vec::new(),
            total: 0,
            current_page: 0,
            search_query: String::new(),
            page_size: 20,
        }
    }

    pub fn set_entries(
        &mut self,
        entries: Vec<HistoryEntry>,
        total: u32,
        thumbnails: Vec<Option<Vec<u8>>>,
    ) {
        self.entries = entries;
        // Build cached handles once, reused across view() calls
        self.thumb_handles = thumbnails
            .iter()
            .map(|t| t.as_ref().map(|data| Handle::from_bytes(data.clone())))
            .collect();
        self.thumbnails = thumbnails;
        self.total = total;
    }

    pub fn find_entry(&self, id: &uuid::Uuid) -> Option<&HistoryEntry> {
        self.entries.iter().find(|e| e.id == *id)
    }

    pub fn offset(&self) -> u32 {
        self.current_page * self.page_size
    }

    pub fn page_size(&self) -> u32 {
        self.page_size
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty() && self.total == 0
    }

    pub fn search_query(&self) -> Option<String> {
        if self.search_query.is_empty() {
            None
        } else {
            Some(self.search_query.clone())
        }
    }

    pub fn update(&mut self, msg: Message) {
        match msg {
            Message::SearchChanged(query) => {
                self.search_query = query;
                self.current_page = 0;
            }
            Message::LoadPage(page) => {
                self.current_page = page;
            }
            Message::DeleteEntry(_id) => {}
            Message::OpenEntry(_id) => {}
            Message::DeleteAll => {}
            Message::OpenFolder => {}
        }
    }

    pub fn view(&self) -> Element<'_, Message> {
        // ── Search bar ───────────────────────────────────────────────

        let search_bar = text_input("Search captures...", &self.search_query)
            .on_input(Message::SearchChanged)
            .padding(10)
            .size(14)
            .style(theme::search_input)
            .width(Length::Fill);

        let open_folder_btn = button(text("Open Folder").size(12))
            .style(theme::ghost_button)
            .on_press(Message::OpenFolder)
            .padding([6, 10]);

        let mut toolbar = row![search_bar, open_folder_btn].spacing(8).align_y(iced::Alignment::Center);

        if self.total > 0 {
            let delete_all_btn = button(text("Delete All").size(12))
                .style(theme::danger_button)
                .on_press(Message::DeleteAll)
                .padding([6, 10]);
            toolbar = toolbar.push(delete_all_btn);
        }

        // ── Entry list ───────────────────────────────────────────────

        let entries_list = if self.entries.is_empty() {
            column![container(
                column![
                    text("No captures yet")
                        .size(16)
                        .font(theme::FONT_SEMIBOLD)
                        .color(theme::TEXT_SECONDARY),
                    text("Use Ctrl+Shift+1 to take a screenshot")
                        .size(13)
                        .color(theme::TEXT_TERTIARY),
                ]
                .spacing(4)
                .align_x(iced::Alignment::Center),
            )
            .width(Length::Fill)
            .padding([40, 0])
            .align_x(iced::alignment::Horizontal::Center)]
        } else {
            let mut col = column![].spacing(6);
            for (i, entry) in self.entries.iter().enumerate() {
                let entry_id = entry.id;
                let cached_handle = self.thumb_handles.get(i).and_then(|h| h.as_ref());

                // Thumbnail
                let mut entry_content = row![].spacing(12).align_y(iced::Alignment::Center);

                if let Some(handle) = cached_handle {
                    let thumb = container(
                        iced::widget::image(handle.clone())
                            .width(Length::Fixed(80.0))
                            .height(Length::Fixed(56.0)),
                    )
                    .style(theme::thumbnail)
                    .clip(true);
                    entry_content = entry_content.push(thumb);
                }

                // Info column: date + dimensions/mode
                let date = format_timestamp(entry.timestamp_ms);
                let details = format!("{}x{}  {:?}", entry.width, entry.height, entry.mode);

                let info = column![
                    text(date).size(13).color(theme::TEXT_PRIMARY),
                    text(details).size(12).color(theme::TEXT_SECONDARY),
                ]
                .spacing(2);

                let open_btn = button(info)
                    .style(theme::ghost_button)
                    .on_press(Message::OpenEntry(entry_id))
                    .padding([8, 12])
                    .width(Length::Fill);

                entry_content = entry_content.push(open_btn);

                // Delete button
                let delete_btn = button(text("Delete").size(12))
                    .style(theme::danger_button)
                    .on_press(Message::DeleteEntry(entry_id))
                    .padding([6, 10]);

                entry_content = entry_content.push(delete_btn);

                // Wrap in card
                let entry_card = container(entry_content)
                    .style(theme::card)
                    .padding(8)
                    .width(Length::Fill);

                col = col.push(entry_card);
            }
            col
        };

        // ── Pagination ───────────────────────────────────────────────

        let total_pages = (self.total + self.page_size - 1) / self.page_size;

        let mut pagination = row![].spacing(8).align_y(iced::Alignment::Center);

        if self.current_page > 0 {
            pagination = pagination.push(
                button(text("< Prev").size(13))
                    .style(theme::ghost_button)
                    .on_press(Message::LoadPage(self.current_page - 1))
                    .padding([6, 12]),
            );
        }

        pagination = pagination.push(Space::with_width(Length::Fill));

        pagination = pagination.push(
            text(format!(
                "Page {} of {}",
                self.current_page + 1,
                total_pages.max(1)
            ))
            .size(13)
            .color(theme::TEXT_SECONDARY),
        );

        pagination = pagination.push(Space::with_width(Length::Fill));

        if self.current_page + 1 < total_pages {
            pagination = pagination.push(
                button(text("Next >").size(13))
                    .style(theme::ghost_button)
                    .on_press(Message::LoadPage(self.current_page + 1))
                    .padding([6, 12]),
            );
        }

        // ── Status ───────────────────────────────────────────────────

        let status = text(format!(
            "Showing {} of {} captures",
            self.entries.len(),
            self.total
        ))
        .size(12)
        .color(theme::TEXT_TERTIARY);

        // ── Layout ───────────────────────────────────────────────────

        let content = column![
            toolbar,
            scrollable(entries_list).height(Length::Fill),
            pagination,
            status,
        ]
        .spacing(12)
        .padding(24)
        .width(Length::Fill);

        container(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }
}

fn format_timestamp(ts_ms: i64) -> String {
    let secs = ts_ms / 1000;
    let dt = chrono::DateTime::from_timestamp(secs, 0);
    match dt {
        Some(dt) => dt.format("%Y-%m-%d %H:%M:%S").to_string(),
        None => "Unknown".to_string(),
    }
}
