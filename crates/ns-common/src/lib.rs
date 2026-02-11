pub mod config;
pub mod history;
pub mod hotkey;
pub mod ipc;
pub mod paths;

use std::sync::Arc;

/// Screen region in physical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl Rect {
    pub fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self { x, y, width, height }
    }

    pub fn right(&self) -> i32 {
        self.x + self.width as i32
    }

    pub fn bottom(&self) -> i32 {
        self.y + self.height as i32
    }

    pub fn area(&self) -> u64 {
        self.width as u64 * self.height as u64
    }

    /// Normalize a rect so width/height are always positive
    /// (handles selection drawn right-to-left or bottom-to-top).
    pub fn normalized(x1: i32, y1: i32, x2: i32, y2: i32) -> Self {
        let x = x1.min(x2);
        let y = y1.min(y2);
        let w = (x1 - x2).unsigned_abs();
        let h = (y1 - y2).unsigned_abs();
        Self { x, y, width: w, height: h }
    }
}

/// Capture mode determines how the capture region is selected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum CaptureMode {
    /// User draws a rectangular region on screen.
    Region,
    /// User clicks a window to capture it.
    Window,
    /// Captures the entire monitor(s).
    Fullscreen,
}

/// BGRA pixel buffer (matches DXGI_FORMAT_B8G8R8A8_UNORM).
///
/// Uses `Arc<Vec<u8>>` internally so `.clone()` is a cheap refcount bump
/// instead of copying potentially 8+ MB of pixel data.
#[derive(Debug, Clone)]
pub struct PixelBuffer {
    pub data: Arc<Vec<u8>>,
    pub width: u32,
    pub height: u32,
    /// Row pitch in bytes (may be larger than width * 4 due to alignment).
    pub stride: u32,
}

impl PixelBuffer {
    pub fn new(data: Vec<u8>, width: u32, height: u32, stride: u32) -> Self {
        Self { data: Arc::new(data), width, height, stride }
    }
}

/// Timing breakdown for a capture operation (for benchmarking).
#[derive(Debug, Clone)]
pub struct CaptureTiming {
    /// Time to acquire a frame via DXGI OutputDuplication.
    pub dxgi_acquire: std::time::Duration,
    /// Time for GPU crop (CopySubresourceRegion). None for fullscreen.
    pub gpu_crop: Option<std::time::Duration>,
    /// Time for CPU readback (Map + memcpy).
    pub cpu_readback: std::time::Duration,
    /// Total time from start of capture call to pixel data ready.
    pub total: std::time::Duration,
}

/// Result of a completed capture operation.
#[derive(Debug, Clone)]
pub struct CaptureResult {
    pub pixels: PixelBuffer,
    pub region: Rect,
    pub mode: CaptureMode,
    pub monitor_index: u32,
    /// Timing data for benchmarking. None if timing was not collected.
    pub timing: Option<CaptureTiming>,
}

#[derive(Debug, thiserror::Error)]
pub enum NsError {
    #[error("Windows API error: {0}")]
    Windows(#[from] windows_core::Error),
    #[error("Capture error: {0}")]
    Capture(String),
    #[error("Clipboard error: {0}")]
    Clipboard(String),
    #[error("Config error: {0}")]
    Config(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Other(String),
}
