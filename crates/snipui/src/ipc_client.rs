//! IPC client wrapper for snipui → snipd communication.

use anyhow::Result;
use ns_common::config::NsConfig;
use ns_common::history::HistoryEntry;
use ns_common::ipc::IpcMessage;

/// Request config from snipd.
pub async fn get_config() -> Result<NsConfig> {
    let response = snip_ipc::IpcClient::send(&IpcMessage::GetConfig).await?;
    match response {
        IpcMessage::ConfigData(config) => Ok(config),
        IpcMessage::Error(e) => Err(anyhow::anyhow!("Server error: {e}")),
        _ => Err(anyhow::anyhow!("Unexpected response")),
    }
}

/// Send updated config to snipd.
pub async fn set_config(config: NsConfig) -> Result<()> {
    let response = snip_ipc::IpcClient::send(&IpcMessage::SetConfig(config)).await?;
    match response {
        IpcMessage::Ack => Ok(()),
        IpcMessage::Error(e) => Err(anyhow::anyhow!("Server error: {e}")),
        _ => Err(anyhow::anyhow!("Unexpected response")),
    }
}

/// Request a page of history entries.
pub async fn get_history(
    offset: u32,
    limit: u32,
    search: Option<String>,
) -> Result<(Vec<HistoryEntry>, u32, Vec<Option<Vec<u8>>>)> {
    let response = snip_ipc::IpcClient::send(&IpcMessage::GetHistory {
        offset,
        limit,
        search,
    })
    .await?;
    match response {
        IpcMessage::HistoryData { entries, total, thumbnails } => Ok((entries, total, thumbnails)),
        IpcMessage::Error(e) => Err(anyhow::anyhow!("Server error: {e}")),
        _ => Err(anyhow::anyhow!("Unexpected response")),
    }
}

/// Delete a history entry.
pub async fn delete_entry(id: uuid::Uuid) -> Result<()> {
    let response = snip_ipc::IpcClient::send(&IpcMessage::DeleteEntry(id)).await?;
    match response {
        IpcMessage::EntryDeleted(_) => Ok(()),
        IpcMessage::Error(e) => Err(anyhow::anyhow!("Server error: {e}")),
        _ => Err(anyhow::anyhow!("Unexpected response")),
    }
}
