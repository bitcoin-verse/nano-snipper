use crate::config::NsConfig;
use crate::history::HistoryEntry;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Messages exchanged between nanosnipper and snipui over named pipes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload")]
pub enum IpcMessage {
    // --- snipui → nanosnipper ---
    /// Request the current config.
    GetConfig,
    /// Update config (nanosnipper should reload).
    SetConfig(NsConfig),
    /// Request history page.
    GetHistory {
        offset: u32,
        limit: u32,
        search: Option<String>,
    },
    /// Delete a history entry.
    DeleteEntry(Uuid),
    /// Delete all history entries.
    DeleteAllEntries,
    /// Request nanosnipper to trigger a capture.
    TriggerCapture(crate::CaptureMode),
    /// Pause/resume hotkeys.
    SetPaused(bool),

    // --- nanosnipper → snipui ---
    /// Config data response.
    ConfigData(NsConfig),
    /// History page response.
    HistoryData {
        entries: Vec<HistoryEntry>,
        total: u32,
        thumbnails: Vec<Option<Vec<u8>>>,
    },
    /// A new capture was completed.
    CaptureCompleted(HistoryEntry),
    /// An entry was deleted.
    EntryDeleted(Uuid),
    /// All entries were deleted.
    AllEntriesDeleted,
    /// Error response.
    Error(String),
    /// Acknowledge with no payload.
    Ack,
}

/// The named pipe path for IPC.
pub const PIPE_NAME: &str = r"\\.\pipe\NanoSnipper";
