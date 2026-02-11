//! OCR via Windows.Media.Ocr (WinRT).

use anyhow::Result;
use ns_common::PixelBuffer;
use tracing::info;
use windows::Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap};
use windows::Media::Ocr::OcrEngine;
use windows::Storage::Streams::{DataWriter, InMemoryRandomAccessStream};

/// Perform OCR on a BGRA pixel buffer.
/// Returns recognized text with line breaks preserved.
pub async fn recognize(pixels: &PixelBuffer) -> Result<String> {
    info!("Starting OCR on {}x{} image", pixels.width, pixels.height);

    // Get the default OCR engine (user's language)
    let engine = OcrEngine::TryCreateFromUserProfileLanguages()?;

    // Create SoftwareBitmap from BGRA data
    let bitmap = SoftwareBitmap::Create(
        BitmapPixelFormat::Bgra8,
        pixels.width as i32,
        pixels.height as i32,
    )?;

    // Copy pixel data into the SoftwareBitmap
    // Convert from potentially-padded stride to packed BGRA
    let mut packed = Vec::with_capacity((pixels.width * pixels.height * 4) as usize);
    for y in 0..pixels.height {
        let row_start = (y * pixels.stride) as usize;
        let row_end = row_start + (pixels.width * 4) as usize;
        if row_end <= pixels.data.len() {
            packed.extend_from_slice(&pixels.data[row_start..row_end]);
        }
    }
    bitmap.CopyFromBuffer(&create_buffer(&packed)?)?;

    // Run OCR
    let result = engine.RecognizeAsync(&bitmap)?.await?;

    // Build text from lines
    let mut text = String::new();
    let lines = result.Lines()?;
    for i in 0..lines.Size()? {
        let line = lines.GetAt(i)?;
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&line.Text()?.to_string());
    }

    info!("OCR complete: {} chars, {} lines", text.len(), lines.Size()?);
    Ok(text)
}

fn create_buffer(data: &[u8]) -> Result<windows::Storage::Streams::IBuffer> {
    let stream = InMemoryRandomAccessStream::new()?;
    let writer = DataWriter::CreateDataWriter(&stream)?;
    writer.WriteBytes(data)?;
    let buffer = writer.DetachBuffer()?;
    Ok(buffer)
}
