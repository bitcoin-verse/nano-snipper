//! Screen capture via Windows Graphics Capture API with D3D11 GPU crop.

use anyhow::Result;
use ns_common::{CaptureMode, CaptureResult, CaptureTiming, PixelBuffer, Rect};
use std::time::{Duration, Instant};
use tracing::{debug, info};
use windows::core::Interface;
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_11_1};
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;

/// Holds GPU state for a capture that hasn't been read back to CPU yet.
/// The DXGI frame is still held; caller must call `complete_readback()` to
/// finish the capture and release GPU resources.
pub struct PendingCapture {
    pub dup: IDXGIOutputDuplication,
    pub clean_texture: ID3D11Texture2D,
    pub width: u32,
    pub height: u32,
    pub mode: CaptureMode,
    pub region: Rect,
    pub acquire_duration: Duration,
}

/// RAII guard that releases a DXGI frame when dropped.
/// Prevents frame leaks when GPU operations fail after acquire_frame().
struct FrameGuard {
    dup: IDXGIOutputDuplication,
    released: bool,
}

impl FrameGuard {
    fn new(dup: IDXGIOutputDuplication) -> Self {
        Self { dup, released: false }
    }

    /// Explicitly release the frame. Returns the release result.
    fn release(mut self) -> Result<()> {
        self.released = true;
        unsafe { self.dup.ReleaseFrame()? };
        Ok(())
    }
}

impl Drop for FrameGuard {
    fn drop(&mut self) {
        if !self.released {
            tracing::warn!("DXGI frame not explicitly released — releasing in drop (error path)");
            let _ = unsafe { self.dup.ReleaseFrame() };
        }
    }
}

/// Pre-initialized D3D11 device, created at startup to avoid cold-start latency.
pub struct CaptureEngine {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    /// Persistent DXGI output duplication — reused across captures to avoid
    /// the ~3-5ms cost of DuplicateOutput on every frame.
    cached_dup: Option<IDXGIOutputDuplication>,
    /// Cached clean (DEFAULT usage) texture, reused when dimensions match.
    clean_texture_cache: Option<(ID3D11Texture2D, u32, u32)>,
    /// Cached staging (CPU-readable) texture, reused when dimensions match.
    staging_texture_cache: Option<(ID3D11Texture2D, u32, u32)>,
}

impl CaptureEngine {
    /// Create a new capture engine with a D3D11 device.
    ///
    /// Enumerates DXGI adapters and creates the device on the adapter that
    /// actually has display outputs connected. This is critical on hybrid-GPU
    /// laptops where `D3D11CreateDevice(None, HARDWARE)` picks the discrete GPU
    /// but the display is driven by the integrated GPU — Desktop Duplication
    /// only works on the adapter that owns the display output.
    pub fn new() -> Result<Self> {
        info!("Initializing D3D11 device for capture");

        // 1. Create DXGI factory to enumerate adapters
        let factory: IDXGIFactory1 = unsafe { CreateDXGIFactory1()? };

        // 2. Find the adapter that actually has display outputs
        let mut display_adapter = None;
        let mut idx = 0u32;
        while let Ok(adapter) = unsafe { factory.EnumAdapters1(idx) } {
            let desc = unsafe { adapter.GetDesc1()? };
            let name = String::from_utf16_lossy(
                &desc.Description[..desc.Description.iter().position(|&c| c == 0).unwrap_or(desc.Description.len())]
            );
            info!(
                "DXGI Adapter {}: {} (VRAM={}MB, Flags=0x{:X})",
                idx, name, desc.DedicatedVideoMemory / 1024 / 1024, desc.Flags
            );

            if unsafe { adapter.EnumOutputs(0) }.is_ok() {
                info!("  -> Has display outputs, selecting this adapter");
                display_adapter = Some(adapter);
                break;
            } else {
                info!("  -> No display outputs, skipping");
            }
            idx += 1;
        }
        let adapter = display_adapter
            .ok_or_else(|| anyhow::anyhow!("No DXGI adapter with display outputs found"))?;

        // 3. Create D3D11 device on the selected adapter
        //    When passing an explicit adapter, DriverType MUST be D3D_DRIVER_TYPE_UNKNOWN
        let mut device = None;
        let mut context = None;

        unsafe {
            D3D11CreateDevice(
                &adapter,
                D3D_DRIVER_TYPE_UNKNOWN,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                Some(&[D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0]),
                7, // D3D11_SDK_VERSION
                Some(&mut device),
                None,
                Some(&mut context),
            )?;
        }

        let device = device.ok_or_else(|| anyhow::anyhow!("Failed to create D3D11 device"))?;
        let context = context.ok_or_else(|| anyhow::anyhow!("Failed to get device context"))?;

        info!("D3D11 device created successfully on adapter with display outputs");

        Ok(Self {
            device,
            context,
            cached_dup: None,
            clean_texture_cache: None,
            staging_texture_cache: None,
        })
    }

    /// Get a reference to the D3D11 device (for sharing with overlay/UI).
    pub fn device(&self) -> &ID3D11Device {
        &self.device
    }

    /// Get a reference to the device context.
    pub fn context(&self) -> &ID3D11DeviceContext {
        &self.context
    }

    /// Acquire a fullscreen DXGI frame (GPU only, no CPU readback).
    /// Returns a `PendingCapture` that must be completed via `complete_readback()`.
    /// This is the fast path: clipboard can be announced immediately after this returns.
    pub fn capture_fullscreen_gpu_only(&mut self) -> Result<PendingCapture> {
        debug!("Starting GPU-only fullscreen acquire");
        let t_acquire = Instant::now();
        let (dup, frame_tex, width, height) = self.acquire_frame()?;
        let acquire_duration = t_acquire.elapsed();

        let region = Rect::new(0, 0, width, height);
        debug!("GPU-only acquire complete: {}x{} in {:?}", width, height, acquire_duration);

        Ok(PendingCapture {
            dup,
            clean_texture: frame_tex,
            width,
            height,
            mode: CaptureMode::Fullscreen,
            region,
            acquire_duration,
        })
    }

    /// Complete the CPU readback for a pending capture.
    /// Reads pixels from GPU, fixes alpha, releases DXGI frame.
    pub fn complete_readback(&mut self, pending: PendingCapture) -> Result<CaptureResult> {
        debug!("Completing readback for pending capture");
        let guard = FrameGuard::new(pending.dup);

        let t_readback = Instant::now();
        let pixels = self.read_from_texture(&pending.clean_texture, pending.width, pending.height)?;
        let cpu_readback = t_readback.elapsed();

        // Diagnostic: sample first pixels
        let sample_count = (pending.width as usize).min(16);
        let all_sample_black = (0..sample_count).all(|x| {
            let off = x * 4;
            pixels.data[off] == 0 && pixels.data[off + 1] == 0 && pixels.data[off + 2] == 0
        });
        if all_sample_black {
            tracing::warn!("First {} pixels have ALL-ZERO RGB — likely black image (deferred readback)", sample_count);
        }

        // Release DXGI frame after Map completes
        guard.release()?;

        let total = pending.acquire_duration + cpu_readback;

        Ok(CaptureResult {
            pixels,
            region: pending.region,
            mode: pending.mode,
            monitor_index: 0,
            timing: Some(CaptureTiming {
                dxgi_acquire: pending.acquire_duration,
                gpu_crop: None,
                cpu_readback,
                total,
            }),
        })
    }

    /// Capture the full primary monitor.
    pub fn capture_fullscreen(&mut self) -> Result<CaptureResult> {
        debug!("Starting fullscreen capture");

        let t_acquire = Instant::now();
        let (dup, frame_tex, width, height) = self.acquire_frame()?;
        let guard = FrameGuard::new(dup);
        let dxgi_acquire = t_acquire.elapsed();

        let t_readback = Instant::now();
        let pixels = self.read_from_texture(&frame_tex, width, height)?;
        let cpu_readback = t_readback.elapsed();

        // Diagnostic: sample first pixels to detect all-black captures (RGB only, alpha is forced 255)
        let sample_count = (width as usize).min(16);
        let all_sample_black = (0..sample_count).all(|x| {
            let off = x * 4;
            pixels.data[off] == 0 && pixels.data[off + 1] == 0 && pixels.data[off + 2] == 0
        });
        if all_sample_black {
            tracing::warn!("First {} pixels have ALL-ZERO RGB — likely black image (fullscreen)", sample_count);
        }

        // Release AFTER Map — pixel data is safely in CPU memory
        guard.release()?;

        let total = dxgi_acquire + cpu_readback;

        let region = Rect::new(0, 0, width, height);
        debug!("Fullscreen capture complete: {}x{}", width, height);

        Ok(CaptureResult {
            pixels,
            region,
            mode: CaptureMode::Fullscreen,
            monitor_index: 0,
            timing: Some(CaptureTiming {
                dxgi_acquire,
                gpu_crop: None,
                cpu_readback,
                total,
            }),
        })
    }

    /// Capture a specific region by first capturing fullscreen then GPU-cropping.
    pub fn capture_region(&mut self, region: Rect) -> Result<CaptureResult> {
        debug!("Starting region capture: {:?}", region);

        let t_acquire = Instant::now();
        let (dup, frame_tex, full_w, full_h) = self.acquire_frame()?;
        let guard = FrameGuard::new(dup);
        let dxgi_acquire = t_acquire.elapsed();

        let (pixels, gpu_crop, cpu_readback) = if region.x == 0
            && region.y == 0
            && region.width == full_w
            && region.height == full_h
        {
            let t = Instant::now();
            let p = self.read_from_texture(&frame_tex, full_w, full_h)?;
            (p, None, t.elapsed())
        } else {
            let t = Instant::now();
            let p = self.gpu_crop_texture(&frame_tex, &region)?;
            (p, Some(t.elapsed()), Duration::ZERO)
        };

        // Diagnostic: sample first pixels to detect all-black captures (RGB only, alpha is forced 255)
        let sample_count = (region.width as usize).min(16);
        let all_sample_black = (0..sample_count).all(|x| {
            let off = x * 4;
            pixels.data[off] == 0 && pixels.data[off + 1] == 0 && pixels.data[off + 2] == 0
        });
        if all_sample_black {
            tracing::warn!("First {} pixels have ALL-ZERO RGB — likely black image (region)", sample_count);
        }

        // Release AFTER all GPU reads complete
        guard.release()?;

        let total = dxgi_acquire + gpu_crop.unwrap_or(Duration::ZERO) + cpu_readback;

        Ok(CaptureResult {
            pixels,
            region,
            mode: CaptureMode::Region,
            monitor_index: 0,
            timing: Some(CaptureTiming {
                dxgi_acquire,
                gpu_crop,
                cpu_readback,
                total,
            }),
        })
    }

    /// Capture a specific window by its HWND.
    /// Uses fullscreen DXGI capture + GPU crop to the window rect.
    pub fn capture_window(&mut self, hwnd_raw: isize) -> Result<CaptureResult> {
        let hwnd = windows::Win32::Foundation::HWND(hwnd_raw as *mut std::ffi::c_void);

        // Get window rect in screen coordinates
        let mut win_rect = windows::Win32::Foundation::RECT::default();
        unsafe {
            windows::Win32::UI::WindowsAndMessaging::GetWindowRect(hwnd, &mut win_rect)?;
        }

        let region = Rect::new(
            win_rect.left,
            win_rect.top,
            (win_rect.right - win_rect.left) as u32,
            (win_rect.bottom - win_rect.top) as u32,
        );

        if region.width == 0 || region.height == 0 {
            anyhow::bail!("Window has zero size");
        }

        debug!("Capturing window at {:?}", region);

        let t_acquire = Instant::now();
        let (dup, frame_tex, _full_w, _full_h) = self.acquire_frame()?;
        let guard = FrameGuard::new(dup);
        let dxgi_acquire = t_acquire.elapsed();

        let t_crop = Instant::now();
        let pixels = self.gpu_crop_texture(&frame_tex, &region)?;
        let gpu_crop = t_crop.elapsed();

        // Diagnostic: sample first pixels to detect all-black captures (RGB only, alpha is forced 255)
        let sample_count = (region.width as usize).min(16);
        let all_sample_black = (0..sample_count).all(|x| {
            let off = x * 4;
            pixels.data[off] == 0 && pixels.data[off + 1] == 0 && pixels.data[off + 2] == 0
        });
        if all_sample_black {
            tracing::warn!("First {} pixels have ALL-ZERO RGB — likely black image (window)", sample_count);
        }

        // Release AFTER all GPU reads complete
        guard.release()?;

        let total = dxgi_acquire + gpu_crop;

        Ok(CaptureResult {
            pixels,
            region,
            mode: CaptureMode::Window,
            monitor_index: 0,
            timing: Some(CaptureTiming {
                dxgi_acquire,
                gpu_crop: Some(gpu_crop),
                cpu_readback: Duration::ZERO,
                total,
            }),
        })
    }

    /// Get or create a clean DEFAULT texture with the given dimensions.
    fn get_or_create_clean_texture(&mut self, width: u32, height: u32, format: DXGI_FORMAT) -> Result<ID3D11Texture2D> {
        if let Some((ref tex, w, h)) = self.clean_texture_cache {
            if w == width && h == height {
                return Ok(tex.clone());
            }
        }
        let desc = D3D11_TEXTURE2D_DESC {
            Width: width,
            Height: height,
            MipLevels: 1,
            ArraySize: 1,
            Format: format,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: 0,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };
        let mut tex: Option<ID3D11Texture2D> = None;
        unsafe { self.device.CreateTexture2D(&desc, None, Some(&mut tex))? };
        let tex = tex.ok_or_else(|| anyhow::anyhow!("Failed to create clean texture"))?;
        self.clean_texture_cache = Some((tex.clone(), width, height));
        debug!("Created new clean texture cache: {}x{}", width, height);
        Ok(tex)
    }

    /// Get or create a staging texture with the given dimensions.
    fn get_or_create_staging_texture(&mut self, width: u32, height: u32, format: DXGI_FORMAT) -> Result<ID3D11Texture2D> {
        if let Some((ref tex, w, h)) = self.staging_texture_cache {
            if w == width && h == height {
                return Ok(tex.clone());
            }
        }
        let desc = D3D11_TEXTURE2D_DESC {
            Width: width,
            Height: height,
            MipLevels: 1,
            ArraySize: 1,
            Format: format,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Usage: D3D11_USAGE_STAGING,
            BindFlags: 0,
            CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
            MiscFlags: 0,
        };
        let mut tex: Option<ID3D11Texture2D> = None;
        unsafe { self.device.CreateTexture2D(&desc, None, Some(&mut tex))? };
        let tex = tex.ok_or_else(|| anyhow::anyhow!("Failed to create staging texture"))?;
        self.staging_texture_cache = Some((tex.clone(), width, height));
        debug!("Created new staging texture cache: {}x{}", width, height);
        Ok(tex)
    }

    /// Ensure we have a valid DXGI output duplication, creating one if needed.
    /// Returns DXGI_ERROR_ACCESS_LOST if the existing one was invalidated.
    fn ensure_duplication(&mut self) -> Result<IDXGIOutputDuplication> {
        if let Some(ref dup) = self.cached_dup {
            return Ok(dup.clone());
        }

        debug!("Creating new DXGI output duplication");
        let dxgi_device: IDXGIDevice = self.device.cast()?;
        let adapter = unsafe { dxgi_device.GetAdapter()? };
        let output = unsafe { adapter.EnumOutputs(0)? };

        let supported_formats = [DXGI_FORMAT_B8G8R8A8_UNORM];
        let dup = if let Ok(output5) = output.cast::<IDXGIOutput5>() {
            debug!("Using DuplicateOutput1 (DXGI 1.5)");
            unsafe { output5.DuplicateOutput1(&self.device, 0, &supported_formats)? }
        } else {
            debug!("IDXGIOutput5 not available, falling back to DuplicateOutput");
            let output1: IDXGIOutput1 = output.cast()?;
            unsafe { output1.DuplicateOutput(&self.device)? }
        };

        self.cached_dup = Some(dup.clone());
        Ok(dup)
    }

    /// Invalidate the cached duplication (e.g., after ACCESS_LOST).
    fn invalidate_duplication(&mut self) {
        if self.cached_dup.take().is_some() {
            debug!("DXGI duplication invalidated");
        }
    }

    /// Release the DXGI frame. The duplication handle stays alive for reuse.
    pub fn release_frame(&self) -> Result<()> {
        if let Some(ref dup) = self.cached_dup {
            unsafe { dup.ReleaseFrame()? };
        }
        Ok(())
    }

    /// Acquire a DXGI desktop frame using persistent duplication.
    /// Returns the duplication handle (caller MUST call `release_frame()` or
    /// `dup.ReleaseFrame()` after all GPU reads complete) and a clean copy
    /// of the frame texture.
    fn acquire_frame(&mut self) -> Result<(IDXGIOutputDuplication, ID3D11Texture2D, u32, u32)> {
        // Try with cached duplication first, recreate on ACCESS_LOST
        let dup = match self.ensure_duplication() {
            Ok(d) => d,
            Err(e) => {
                self.invalidate_duplication();
                return Err(e);
            }
        };

        // Retry loop: the first AcquireNextFrame after DuplicateOutput can return
        // accumulated_frames=0 (empty frame) if the DWM hasn't composed yet.
        let mut texture = None;
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        let mut last_accumulated = 0;
        let mut access_lost = false;
        for attempt in 0..4u32 {
            let mut frame_info = DXGI_OUTDUPL_FRAME_INFO::default();
            let mut resource = None;
            match unsafe { dup.AcquireNextFrame(500, &mut frame_info, &mut resource) } {
                Ok(()) => {},
                Err(e) => {
                    // DXGI_ERROR_ACCESS_LOST = 0x887A0026
                    if e.code().0 as u32 == 0x887A0026 {
                        debug!("DXGI_ERROR_ACCESS_LOST — recreating duplication");
                        access_lost = true;
                        break;
                    }
                    return Err(e.into());
                }
            }
            last_accumulated = frame_info.AccumulatedFrames;

            let res = resource.ok_or_else(|| anyhow::anyhow!("No frame resource"))?;
            let tex: ID3D11Texture2D = res.cast()?;
            unsafe { tex.GetDesc(&mut desc) };

            debug!(
                "DXGI frame (attempt {}): {}x{} format={} misc_flags={} accumulated_frames={}",
                attempt, desc.Width, desc.Height, desc.Format.0, desc.MiscFlags,
                frame_info.AccumulatedFrames
            );

            if frame_info.AccumulatedFrames > 0 {
                texture = Some(tex);
                break;
            }

            // Empty frame — release and retry after a short wait
            debug!("accumulated_frames=0, releasing and retrying...");
            unsafe { dup.ReleaseFrame()? };
            std::thread::sleep(Duration::from_millis(16)); // ~1 vsync interval
        }

        // If access was lost, invalidate cache and retry once with fresh duplication
        if access_lost {
            self.invalidate_duplication();
            let fresh_dup = self.ensure_duplication()?;
            let mut frame_info = DXGI_OUTDUPL_FRAME_INFO::default();
            let mut resource = None;
            unsafe { fresh_dup.AcquireNextFrame(500, &mut frame_info, &mut resource)? };
            let res = resource.ok_or_else(|| anyhow::anyhow!("No frame resource after recreate"))?;
            let tex: ID3D11Texture2D = res.cast()?;
            unsafe { tex.GetDesc(&mut desc) };
            texture = Some(tex);
        }

        let texture = texture.ok_or_else(|| {
            anyhow::anyhow!(
                "DXGI returned empty frames (accumulated_frames={}) after 4 attempts",
                last_accumulated
            )
        })?;

        // Copy to a clean DEFAULT texture (cached) to normalize DXGI-specific flags
        let copy_texture = self.get_or_create_clean_texture(desc.Width, desc.Height, desc.Format)?;

        unsafe {
            self.context.CopyResource(&copy_texture, &texture);
            self.context.Flush();
        }

        Ok((dup, copy_texture, desc.Width, desc.Height))
    }

    /// Read pixels from a GPU texture using a cached staging texture.
    fn read_from_texture(&mut self, texture: &ID3D11Texture2D, width: u32, height: u32) -> Result<PixelBuffer> {
        let mut src_desc = D3D11_TEXTURE2D_DESC::default();
        unsafe { texture.GetDesc(&mut src_desc) };

        let staging = self.get_or_create_staging_texture(width, height, src_desc.Format)?;

        unsafe { self.context.CopyResource(&staging, texture) };

        self.read_staging_texture(&staging, width, height)
    }

    /// GPU crop: copy a sub-region from a GPU texture using CopySubresourceRegion.
    fn gpu_crop_texture(&self, src_texture: &ID3D11Texture2D, region: &Rect) -> Result<PixelBuffer> {
        debug!("GPU crop: {:?}", region);

        let mut src_desc = D3D11_TEXTURE2D_DESC::default();
        unsafe { src_texture.GetDesc(&mut src_desc) };

        // Create destination staging texture for the cropped region
        let dst_desc = D3D11_TEXTURE2D_DESC {
            Width: region.width,
            Height: region.height,
            MipLevels: 1,
            ArraySize: 1,
            Format: src_desc.Format,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Usage: D3D11_USAGE_STAGING,
            BindFlags: 0,
            CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
            MiscFlags: 0,
        };

        let mut dst_texture: Option<ID3D11Texture2D> = None;
        unsafe { self.device.CreateTexture2D(&dst_desc, None, Some(&mut dst_texture))? };
        let dst_texture = dst_texture.ok_or_else(|| anyhow::anyhow!("Failed to create destination texture"))?;

        // CopySubresourceRegion for GPU crop — only copies the selected pixels
        let src_box = D3D11_BOX {
            left: region.x as u32,
            top: region.y as u32,
            front: 0,
            right: region.right() as u32,
            bottom: region.bottom() as u32,
            back: 1,
        };

        unsafe {
            self.context.CopySubresourceRegion(
                &dst_texture,
                0,
                0,
                0,
                0,
                src_texture,
                0,
                Some(&src_box),
            );
        }

        self.read_staging_texture(&dst_texture, region.width, region.height)
    }

    /// Read pixel data from a staging texture.
    fn read_staging_texture(
        &self,
        texture: &ID3D11Texture2D,
        width: u32,
        height: u32,
    ) -> Result<PixelBuffer> {
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        unsafe {
            self.context
                .Map(texture, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;
        }

        let stride = mapped.RowPitch;
        let data_size = stride * height;
        let mut data = vec![0u8; data_size as usize];

        if mapped.pData.is_null() {
            unsafe { self.context.Unmap(texture, 0) };
            anyhow::bail!("Map returned null pData — GPU readback failed");
        }

        unsafe {
            std::ptr::copy_nonoverlapping(
                mapped.pData as *const u8,
                data.as_mut_ptr(),
                data_size as usize,
            );
            self.context.Unmap(texture, 0);
        }

        // Diagnostic: sample first pixel BEFORE alpha fix
        if width > 0 && height > 0 {
            let b = data[0];
            let g = data[1];
            let r = data[2];
            let a = data[3];
            debug!("First pixel (raw DXGI): B={} G={} R={} A={}", b, g, r, a);
        }

        // DXGI Desktop Duplication alpha is unreliable (often 0).
        // Force alpha=255 so clipboard/save/display always shows opaque pixels.
        // Uses SSE2 SIMD (guaranteed on x86_64) to OR 0xFF into every alpha byte,
        // processing 4 pixels per iteration (~3x faster than scalar).
        fix_alpha_simd(&mut data, width, height, stride);

        Ok(PixelBuffer::new(data, width, height, stride))
    }
}

/// Force alpha=255 on every pixel using SSE2 SIMD.
/// Processes 4 BGRA pixels (16 bytes) per iteration.
/// SSE2 is guaranteed available on all x86_64 CPUs.
fn fix_alpha_simd(data: &mut [u8], width: u32, height: u32, stride: u32) {
    #[cfg(target_arch = "x86_64")]
    {
        use std::arch::x86_64::*;
        unsafe {
            // Mask: 0xFF in every 4th byte (alpha channel of BGRA)
            let alpha_mask = _mm_set_epi8(
                -1, 0, 0, 0, -1, 0, 0, 0, -1, 0, 0, 0, -1, 0, 0, 0,
            );

            let row_bytes = (width * 4) as usize;
            if stride as usize == row_bytes {
                // Contiguous: single pass over all pixels
                let total = row_bytes * height as usize;
                let chunks = total / 16;
                let ptr = data.as_mut_ptr();
                for i in 0..chunks {
                    let p = ptr.add(i * 16) as *mut __m128i;
                    let v = _mm_loadu_si128(p);
                    _mm_storeu_si128(p, _mm_or_si128(v, alpha_mask));
                }
                // Handle remaining pixels (< 4 pixels) — checked indexing for safety
                let remainder_start = chunks * 16;
                let mut idx = remainder_start + 3;
                while idx < total {
                    data[idx] = 255;
                    idx += 4;
                }
            } else {
                // Padded rows: process each row independently
                for row in 0..height as usize {
                    let row_start = row * stride as usize;
                    let chunks = row_bytes / 16;
                    let ptr = data.as_mut_ptr().add(row_start);
                    for i in 0..chunks {
                        let p = ptr.add(i * 16) as *mut __m128i;
                        let v = _mm_loadu_si128(p);
                        _mm_storeu_si128(p, _mm_or_si128(v, alpha_mask));
                    }
                    // Remaining pixels in this row — checked indexing for safety
                    let remainder_start = chunks * 16;
                    let mut idx = row_start + remainder_start + 3;
                    let row_end = row_start + row_bytes;
                    while idx < row_end {
                        data[idx] = 255;
                        idx += 4;
                    }
                }
            }
        }
    }

    #[cfg(not(target_arch = "x86_64"))]
    {
        // Scalar fallback for non-x86_64
        for row in 0..height as usize {
            let row_start = row * stride as usize;
            for x in 0..width as usize {
                data[row_start + x * 4 + 3] = 255;
            }
        }
    }
}

unsafe impl Send for CaptureEngine {}
unsafe impl Sync for CaptureEngine {}
