use crate::{CaptureMode, Rect};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A single history entry representing a completed capture.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    /// Time-sortable UUIDv7.
    pub id: Uuid,
    /// When the capture was taken (Unix timestamp millis).
    pub timestamp_ms: i64,
    /// Capture mode used.
    pub mode: CaptureMode,
    /// Screen region captured (physical pixels).
    pub region: Rect,
    /// Relative path to the PNG file (from captures dir).
    pub file_path: String,
    /// File size in bytes.
    pub file_size: u64,
    /// Image dimensions.
    pub width: u32,
    pub height: u32,
    /// OCR text, if extracted.
    pub ocr_text: Option<String>,
    /// Whether the entry has annotation.
    pub annotated: bool,
}

impl HistoryEntry {
    pub fn new(mode: CaptureMode, region: Rect, file_path: String, file_size: u64, width: u32, height: u32) -> Self {
        Self {
            id: Uuid::now_v7(),
            timestamp_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as i64,
            mode,
            region,
            file_path,
            file_size,
            width,
            height,
            ocr_text: None,
            annotated: false,
        }
    }
}
