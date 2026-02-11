//! Thumbnail widget for the history grid.

use iced::widget::{column, container, image, text};
use iced::Element;
use ns_common::history::HistoryEntry;

#[derive(Debug, Clone)]
pub enum Message {
    Clicked(uuid::Uuid),
}

/// Render a single capture thumbnail with metadata.
pub fn view(entry: &HistoryEntry, thumbnail_data: Option<&[u8]>) -> Element<'static, Message> {
    let _id = entry.id;
    let info = format!("{}x{}", entry.width, entry.height);
    let time = format_time(entry.timestamp_ms);

    let mut content = column![].spacing(2).align_x(iced::Alignment::Center);

    // Render thumbnail image if available
    if let Some(data) = thumbnail_data {
        let handle = image::Handle::from_bytes(data.to_vec());
        content = content.push(
            image(handle)
                .width(iced::Length::Fixed(180.0))
                .height(iced::Length::Shrink),
        );
    }

    content = content.push(text(time).size(11));
    content = content.push(text(info).size(10));

    container(content)
        .width(iced::Length::Fixed(200.0))
        .padding(4)
        .into()
}

fn format_time(ts_ms: i64) -> String {
    let secs = ts_ms / 1000;
    chrono::DateTime::from_timestamp(secs, 0)
        .map(|dt| dt.format("%H:%M:%S").to_string())
        .unwrap_or_else(|| "??:??:??".to_string())
}
