//! IPC via Windows Named Pipes.
//! Protocol: length-prefixed (4-byte LE u32) JSON messages.

use anyhow::Result;
use ns_common::ipc::{IpcMessage, PIPE_NAME};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeServer, ServerOptions};
use tracing::{debug, info, warn};

/// Server side (nanosnipper): listens for connections from snipui.
pub struct IpcServer {
    _private: (),
}

impl IpcServer {
    /// Start the IPC server. Calls `handler` for each received message.
    pub async fn run<F>(handler: F) -> Result<()>
    where
        F: Fn(IpcMessage) -> Option<IpcMessage> + Send + 'static,
    {
        info!("IPC server starting on {PIPE_NAME}");

        loop {
            let server = ServerOptions::new()
                .first_pipe_instance(true)
                .create(PIPE_NAME)?;

            debug!("Waiting for IPC client connection...");
            server.connect().await?;
            info!("IPC client connected");

            let handler_ref = &handler;
            if let Err(e) = Self::handle_connection(server, handler_ref).await {
                warn!("IPC connection error: {e}");
            }
        }
    }

    async fn handle_connection<F>(
        mut pipe: NamedPipeServer,
        handler: &F,
    ) -> Result<()>
    where
        F: Fn(IpcMessage) -> Option<IpcMessage>,
    {
        loop {
            // Read length prefix
            let mut len_buf = [0u8; 4];
            match pipe.read_exact(&mut len_buf).await {
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                    debug!("IPC client disconnected");
                    return Ok(());
                }
                Err(e) => return Err(e.into()),
            }
            let len = u32::from_le_bytes(len_buf) as usize;

            if len > 10 * 1024 * 1024 {
                anyhow::bail!("IPC message too large: {len} bytes");
            }

            // Read message body
            let mut body = vec![0u8; len];
            pipe.read_exact(&mut body).await?;

            let msg: IpcMessage = serde_json::from_slice(&body)?;
            debug!("IPC received: {:?}", std::mem::discriminant(&msg));

            // Handle and optionally respond
            if let Some(response) = handler(msg) {
                Self::write_message(&mut pipe, &response).await?;
            }
        }
    }

    async fn write_message(
        pipe: &mut NamedPipeServer,
        msg: &IpcMessage,
    ) -> Result<()> {
        let body = serde_json::to_vec(msg)?;
        let len = (body.len() as u32).to_le_bytes();
        pipe.write_all(&len).await?;
        pipe.write_all(&body).await?;
        pipe.flush().await?;
        Ok(())
    }
}

/// Client side (snipui): connects to nanosnipper.
pub struct IpcClient {
    _private: (),
}

impl IpcClient {
    /// Send a message to nanosnipper and wait for a response.
    pub async fn send(msg: &IpcMessage) -> Result<IpcMessage> {
        let mut pipe = ClientOptions::new().open(PIPE_NAME)?;

        // Write message
        let body = serde_json::to_vec(msg)?;
        let len = (body.len() as u32).to_le_bytes();
        pipe.write_all(&len).await?;
        pipe.write_all(&body).await?;
        pipe.flush().await?;

        // Read response
        let mut len_buf = [0u8; 4];
        pipe.read_exact(&mut len_buf).await?;
        let len = u32::from_le_bytes(len_buf) as usize;

        if len > 10 * 1024 * 1024 {
            anyhow::bail!("IPC response too large: {len} bytes");
        }

        let mut body = vec![0u8; len];
        pipe.read_exact(&mut body).await?;

        let response: IpcMessage = serde_json::from_slice(&body)?;
        Ok(response)
    }
}
