use crate::config::NsConfig;
use crate::history::HistoryEntry;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Messages exchanged between snipd and snipui over named pipes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload")]
pub enum IpcMessage {
    // --- snipui → snipd ---
    /// Request the current config.
    GetConfig,
    /// Update config (snipd should reload).
    SetConfig(NsConfig),
    /// Request history page.
    GetHistory {
        offset: u32,
        limit: u32,
        search: Option<String>,
    },
    /// Delete a history entry.
    DeleteEntry(Uuid),
    /// Request snipd to trigger a capture.
    TriggerCapture(crate::CaptureMode),
    /// Pause/resume hotkeys.
    SetPaused(bool),

    // --- snipd → snipui ---
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
    /// Error response.
    Error(String),
    /// Acknowledge with no payload.
    Ack,
}

/// The named pipe path for IPC.
pub const PIPE_NAME: &str = r"\\.\pipe\NanoSnipper";
