//! Clipboard operations: CF_DIBV5 (and delayed rendering in Phase 7).

use anyhow::Result;
use ns_common::PixelBuffer;
use std::mem;
use std::time::{Duration, Instant};
use tracing::{debug, info};
use windows::Win32::Foundation::*;
use windows::Win32::System::DataExchange::*;
use windows::Win32::System::Memory::*;
use windows::Win32::System::Ole::CF_DIBV5;
use windows::Win32::Graphics::Gdi::*;

/// Build a BITMAPV5HEADER + pixel data HGLOBAL from a PixelBuffer.
/// Uses bulk copy when stride == width*4 (tight stride).
unsafe fn build_dibv5_hglobal(pixels: &PixelBuffer) -> Result<HGLOBAL> {
    let header_size = mem::size_of::<BITMAPV5HEADER>();
    let pixel_data_size = (pixels.width * 4 * pixels.height) as usize;
    let total_size = header_size + pixel_data_size;

    let hglobal = GlobalAlloc(GMEM_MOVEABLE | GMEM_ZEROINIT, total_size)?;
    let ptr = GlobalLock(hglobal);
    if ptr.is_null() {
        GlobalFree(Some(hglobal)).ok();
        anyhow::bail!("GlobalLock failed");
    }

    // Write BITMAPV5HEADER
    let header = ptr as *mut BITMAPV5HEADER;
    (*header).bV5Size = header_size as u32;
    (*header).bV5Width = pixels.width as i32;
    (*header).bV5Height = -(pixels.height as i32); // top-down
    (*header).bV5Planes = 1;
    (*header).bV5BitCount = 32;
    (*header).bV5Compression = BI_RGB;
    (*header).bV5SizeImage = pixel_data_size as u32;
    (*header).bV5XPelsPerMeter = 3780; // 96 DPI
    (*header).bV5YPelsPerMeter = 3780;
    (*header).bV5CSType = 0x73524742; // LCS_sRGB
    (*header).bV5Intent = 4; // LCS_GM_IMAGES
    (*header).bV5RedMask = 0x00FF0000;
    (*header).bV5GreenMask = 0x0000FF00;
    (*header).bV5BlueMask = 0x000000FF;
    (*header).bV5AlphaMask = 0x00000000;

    // Copy pixel data — bulk when possible
    let dst = (ptr as *mut u8).add(header_size);
    let row_bytes = (pixels.width * 4) as usize;

    if pixels.stride == pixels.width * 4 {
        // Tight stride: single bulk copy
        std::ptr::copy_nonoverlapping(pixels.data.as_ptr(), dst, pixel_data_size);
    } else {
        // Row-by-row (handle stride != width * 4)
        for row in 0..pixels.height as usize {
            let src_offset = row * pixels.stride as usize;
            let dst_offset = row * row_bytes;
            std::ptr::copy_nonoverlapping(
                pixels.data.as_ptr().add(src_offset),
                dst.add(dst_offset),
                row_bytes,
            );
        }
    }

    GlobalUnlock(hglobal).ok();
    Ok(hglobal)
}

/// Set image data to the clipboard as CF_DIBV5.
///
/// The PixelBuffer must be BGRA format (DXGI_FORMAT_B8G8R8A8_UNORM).
pub fn set_image(hwnd: HWND, pixels: &PixelBuffer) -> Result<()> {
    info!(
        "Setting clipboard image: {}x{}, stride={}",
        pixels.width, pixels.height, pixels.stride
    );

    let hglobal = unsafe { build_dibv5_hglobal(pixels)? };

    unsafe {
        OpenClipboard(Some(hwnd))?;
        if let Err(e) = EmptyClipboard() {
            CloseClipboard().ok();
            anyhow::bail!("EmptyClipboard failed: {e}");
        }

        let result = SetClipboardData(CF_DIBV5.0 as u32, Some(HANDLE(hglobal.0)));
        if result.is_err() {
            GlobalFree(Some(hglobal)).ok();
            CloseClipboard().ok();
            anyhow::bail!("SetClipboardData failed");
        }

        CloseClipboard()?;
    }

    debug!("Clipboard set successfully");
    Ok(())
}

/// Set both an image (CF_DIBV5) and text (CF_UNICODETEXT) on the clipboard in one session.
pub fn set_image_and_text(hwnd: HWND, pixels: &PixelBuffer, text: &str) -> Result<()> {
    info!(
        "Setting clipboard image+text: {}x{}, text={} chars",
        pixels.width, pixels.height, text.len()
    );

    let hglobal_img = unsafe { build_dibv5_hglobal(pixels)? };

    // Prepare text data
    let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    let byte_len = wide.len() * 2;

    let hglobal_txt = unsafe { GlobalAlloc(GMEM_MOVEABLE, byte_len)? };
    let ptr_txt = unsafe { GlobalLock(hglobal_txt) };
    if ptr_txt.is_null() {
        unsafe { GlobalFree(Some(hglobal_img)).ok() };
        unsafe { GlobalFree(Some(hglobal_txt)).ok() };
        anyhow::bail!("GlobalLock failed for text");
    }

    unsafe {
        std::ptr::copy_nonoverlapping(wide.as_ptr() as *const u8, ptr_txt as *mut u8, byte_len);
        GlobalUnlock(hglobal_txt).ok();
    }

    // Set both in one clipboard session
    unsafe {
        OpenClipboard(Some(hwnd))?;
        if let Err(e) = EmptyClipboard() {
            CloseClipboard().ok();
            anyhow::bail!("EmptyClipboard failed: {e}");
        }

        if SetClipboardData(CF_DIBV5.0 as u32, Some(HANDLE(hglobal_img.0))).is_err() {
            GlobalFree(Some(hglobal_img)).ok();
            GlobalFree(Some(hglobal_txt)).ok();
            CloseClipboard().ok();
            anyhow::bail!("SetClipboardData (image) failed");
        }

        // CF_UNICODETEXT = 13
        if SetClipboardData(13, Some(HANDLE(hglobal_txt.0))).is_err() {
            GlobalFree(Some(hglobal_txt)).ok();
            CloseClipboard().ok();
            anyhow::bail!("SetClipboardData (text) failed");
        }

        CloseClipboard()?;
    }

    debug!("Clipboard set with image + text successfully");
    Ok(())
}

/// Announce delayed rendering for CF_DIBV5 (Phase 7).
/// The actual data is provided via WM_RENDERFORMAT.
/// Returns the duration of the clipboard announce operation.
pub fn set_delayed(hwnd: HWND) -> Result<Duration> {
    let t_start = Instant::now();
    unsafe {
        OpenClipboard(Some(hwnd))?;
        EmptyClipboard()?;
        // None = delayed rendering. SetClipboardData returns NULL when hMem is
        // NULL, which is expected (not an error). The windows crate wrapper
        // treats NULL return as Err, so we ignore the result here.
        let _ = SetClipboardData(CF_DIBV5.0 as u32, None);
        CloseClipboard()?;
    }
    let elapsed = t_start.elapsed();
    debug!("Delayed clipboard rendering announced in {:?}", elapsed);
    Ok(elapsed)
}

/// Build a CF_DIBV5 HGLOBAL from a PixelBuffer for WM_RENDERFORMAT.
///
/// The caller must NOT free the returned handle — Windows takes ownership
/// after `SetClipboardData` is called inside the WM_RENDERFORMAT handler.
/// Returns `(handle, render_duration)`.
pub fn render_dibv5(pixels: &PixelBuffer) -> Result<(HANDLE, Duration)> {
    let t_start = Instant::now();
    let hglobal = unsafe { build_dibv5_hglobal(pixels)? };
    let elapsed = t_start.elapsed();
    let total_size = mem::size_of::<BITMAPV5HEADER>() + (pixels.width * 4 * pixels.height) as usize;
    debug!("Built CF_DIBV5 data: {} bytes in {:?}", total_size, elapsed);
    Ok((HANDLE(hglobal.0), elapsed))
}

/// Set plain text to the clipboard (for OCR results).
pub fn set_text(hwnd: HWND, text: &str) -> Result<()> {
    info!("Setting clipboard text: {} chars", text.len());

    let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    let byte_len = wide.len() * 2;

    let hglobal = unsafe { GlobalAlloc(GMEM_MOVEABLE, byte_len)? };
    let ptr = unsafe { GlobalLock(hglobal) };
    if ptr.is_null() {
        unsafe { GlobalFree(Some(hglobal)).ok() };
        anyhow::bail!("GlobalLock failed");
    }

    unsafe {
        std::ptr::copy_nonoverlapping(wide.as_ptr() as *const u8, ptr as *mut u8, byte_len);
        GlobalUnlock(hglobal).ok();
    }

    unsafe {
        OpenClipboard(Some(hwnd))?;
        EmptyClipboard()?;
        // CF_UNICODETEXT = 13
        SetClipboardData(13, Some(HANDLE(hglobal.0)))?;
        CloseClipboard()?;
    }

    debug!("Clipboard text set successfully");
    Ok(())
}
