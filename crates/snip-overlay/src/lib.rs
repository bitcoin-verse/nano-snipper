//! Fullscreen transparent overlay for region/window selection.
//! Uses Win32 popup window + Direct2D rendering.

use anyhow::Result;
use ns_common::{CaptureMode, Rect};
use parking_lot::Mutex;
use std::sync::Arc;
use std::time::Instant;
use tracing::{debug, info};
use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dwm::DwmFlush;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

/// Result of the overlay selection process.
#[derive(Debug, Clone)]
pub enum OverlayResult {
    /// User selected a rectangular region.
    Region(Rect),
    /// User selected a window (HWND handle as raw pointer).
    Window(isize),
    /// User cancelled (Escape).
    Cancelled,
}

/// Callback type for when overlay selection is complete.
pub type OverlayCallback = Box<dyn FnOnce(OverlayResult) + 'static>;

/// Cached D2D brushes and DWrite text formats to avoid per-paint allocations.
struct CachedBrushes {
    dim_brush: ID2D1SolidColorBrush,       // dark overlay: rgba(0,0,0,0.4)
    selection_border: ID2D1SolidColorBrush, // white: rgba(1,1,1,0.9)
    crosshair_active: ID2D1SolidColorBrush, // white: rgba(1,1,1,0.3)
    crosshair_idle: ID2D1SolidColorBrush,   // white: rgba(1,1,1,0.5)
    window_border: ID2D1SolidColorBrush,    // blue: rgba(0.2,0.5,1,0.9)
    dim_text_bg: ID2D1SolidColorBrush,      // black: rgba(0,0,0,0.7)
    dim_text_fg: ID2D1SolidColorBrush,      // white: rgba(1,1,1,1)
    dim_text_format: IDWriteTextFormat,
}

/// State shared between the overlay window proc and the main logic.
struct OverlayState {
    mode: CaptureMode,
    // Selection tracking
    selecting: bool,
    start_x: i32,
    start_y: i32,
    current_x: i32,
    current_y: i32,
    // Virtual screen origin offset
    screen_origin_x: i32,
    screen_origin_y: i32,
    // Window mode: hovered window
    hovered_hwnd: isize,
    hovered_rect: Option<RECT>,
    overlay_hwnd: HWND,
    // Direct2D resources
    d2d_factory: Option<ID2D1Factory1>,
    render_target: Option<ID2D1HwndRenderTarget>,
    dwrite_factory: Option<IDWriteFactory>,
    // Cached brushes (created once, reused per paint)
    brushes: Option<CachedBrushes>,
    // Callback
    callback: Option<OverlayCallback>,
    // Timing: when the overlay was shown (for benchmark instrumentation)
    shown_at: Option<Instant>,
}

/// Manages the overlay window lifecycle.
pub struct Overlay {
    hwnd: HWND,
    state: Arc<Mutex<OverlayState>>,
}

// Window class name
const OVERLAY_CLASS: PCWSTR = w!("NanoSnipperOverlay");

impl Overlay {
    /// Pre-create the overlay window (hidden). Call once at startup.
    pub fn new() -> Result<Self> {
        let instance = unsafe { GetModuleHandleW(None)? };

        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(Self::wnd_proc),
            hInstance: instance.into(),
            lpszClassName: OVERLAY_CLASS,
            hCursor: unsafe { LoadCursorW(None, IDC_CROSS)? },
            ..Default::default()
        };

        unsafe { RegisterClassExW(&wc) };

        // Get virtual screen bounds (all monitors)
        let x = unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) };
        let y = unsafe { GetSystemMetrics(SM_YVIRTUALSCREEN) };
        let cx = unsafe { GetSystemMetrics(SM_CXVIRTUALSCREEN) };
        let cy = unsafe { GetSystemMetrics(SM_CYVIRTUALSCREEN) };

        let state = Arc::new(Mutex::new(OverlayState {
            mode: CaptureMode::Region,
            selecting: false,
            start_x: 0,
            start_y: 0,
            current_x: 0,
            current_y: 0,
            screen_origin_x: x,
            screen_origin_y: y,
            hovered_hwnd: 0,
            hovered_rect: None,
            overlay_hwnd: HWND::default(),
            d2d_factory: None,
            render_target: None,
            dwrite_factory: None,
            brushes: None,
            callback: None,
            shown_at: None,
        }));

        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_LAYERED,
                OVERLAY_CLASS,
                w!("NanoSnipper Overlay"),
                WS_POPUP,
                x,
                y,
                cx,
                cy,
                None,
                None,
                Some(HINSTANCE(instance.0)),
                Some(Arc::as_ptr(&state) as *const std::ffi::c_void),
            )?
        };

        // Set layered window to semi-transparent
        unsafe {
            SetLayeredWindowAttributes(hwnd, COLORREF(0), 1, LWA_ALPHA)?;
        }

        // Initialize Direct2D
        {
            let mut s = state.lock();
            s.overlay_hwnd = hwnd;
            s.d2d_factory = Some(Self::create_d2d_factory()?);
            s.dwrite_factory = Some(Self::create_dwrite_factory()?);

            let rt = Self::create_render_target(s.d2d_factory.as_ref().unwrap(), hwnd, cx as u32, cy as u32)?;

            // Pre-create cached brushes
            s.brushes = Self::create_brushes(&rt, s.dwrite_factory.as_ref().unwrap()).ok();

            s.render_target = Some(rt);
        }

        info!("Overlay window pre-created (hidden)");

        Ok(Self { hwnd, state })
    }

    /// Show the overlay for selection.
    pub fn show(&self, mode: CaptureMode, callback: OverlayCallback) -> Result<()> {
        unsafe {
            // Refresh virtual screen bounds
            let x = GetSystemMetrics(SM_XVIRTUALSCREEN);
            let y = GetSystemMetrics(SM_YVIRTUALSCREEN);
            let cx = GetSystemMetrics(SM_CXVIRTUALSCREEN);
            let cy = GetSystemMetrics(SM_CYVIRTUALSCREEN);

            {
                let mut s = self.state.lock();
                s.mode = mode;
                s.selecting = false;
                s.screen_origin_x = x;
                s.screen_origin_y = y;
                s.callback = Some(callback);

                // Resize render target if screen size changed
                if let Some(ref rt) = s.render_target {
                    let _ = rt.Resize(&D2D_SIZE_U {
                        width: cx as u32,
                        height: cy as u32,
                    });
                }
            }

            SetWindowPos(
                self.hwnd,
                Some(HWND_TOPMOST),
                x,
                y,
                cx,
                cy,
                SWP_SHOWWINDOW,
            )?;
            let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
            let _ = SetForegroundWindow(self.hwnd);
            // Make overlay semi-transparent for drawing
            SetLayeredWindowAttributes(self.hwnd, COLORREF(0), 200, LWA_ALPHA)?;
        }

        // Record timestamp after overlay is visible
        self.state.lock().shown_at = Some(Instant::now());

        debug!("Overlay shown in {:?} mode", mode);
        Ok(())
    }

    /// Hide the overlay.
    pub fn hide(&self) {
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
            // Reset to alpha=1 when hidden (saves GPU)
            let _ = SetLayeredWindowAttributes(self.hwnd, COLORREF(0), 1, LWA_ALPHA);
        }
        debug!("Overlay hidden");
    }

    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }

    /// Returns the timestamp when the overlay was last shown (for benchmarking).
    pub fn shown_at(&self) -> Option<Instant> {
        self.state.lock().shown_at
    }

    fn create_brushes(rt: &ID2D1HwndRenderTarget, dwrite: &IDWriteFactory) -> Result<CachedBrushes> {
        unsafe {
            let dim_brush = rt.CreateSolidColorBrush(
                &D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.4 }, None)?;
            let selection_border = rt.CreateSolidColorBrush(
                &D2D1_COLOR_F { r: 1.0, g: 1.0, b: 1.0, a: 0.9 }, None)?;
            let crosshair_active = rt.CreateSolidColorBrush(
                &D2D1_COLOR_F { r: 1.0, g: 1.0, b: 1.0, a: 0.3 }, None)?;
            let crosshair_idle = rt.CreateSolidColorBrush(
                &D2D1_COLOR_F { r: 1.0, g: 1.0, b: 1.0, a: 0.5 }, None)?;
            let window_border = rt.CreateSolidColorBrush(
                &D2D1_COLOR_F { r: 0.2, g: 0.5, b: 1.0, a: 0.9 }, None)?;
            let dim_text_bg = rt.CreateSolidColorBrush(
                &D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.7 }, None)?;
            let dim_text_fg = rt.CreateSolidColorBrush(
                &D2D1_COLOR_F { r: 1.0, g: 1.0, b: 1.0, a: 1.0 }, None)?;
            let dim_text_format = dwrite.CreateTextFormat(
                w!("Segoe UI"), None,
                DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL, 14.0, w!("en-US"))?;
            Ok(CachedBrushes {
                dim_brush, selection_border, crosshair_active, crosshair_idle,
                window_border, dim_text_bg, dim_text_fg, dim_text_format,
            })
        }
    }

    fn create_d2d_factory() -> Result<ID2D1Factory1> {
        let options = D2D1_FACTORY_OPTIONS::default();
        let factory: ID2D1Factory1 =
            unsafe { D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, Some(&options))? };
        Ok(factory)
    }

    fn create_dwrite_factory() -> Result<IDWriteFactory> {
        let factory: IDWriteFactory =
            unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)? };
        Ok(factory)
    }

    fn create_render_target(
        factory: &ID2D1Factory1,
        hwnd: HWND,
        width: u32,
        height: u32,
    ) -> Result<ID2D1HwndRenderTarget> {
        let render_props = D2D1_RENDER_TARGET_PROPERTIES {
            r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 96.0,
            dpiY: 96.0,
            ..Default::default()
        };

        let hwnd_props = D2D1_HWND_RENDER_TARGET_PROPERTIES {
            hwnd,
            pixelSize: D2D_SIZE_U { width, height },
            presentOptions: D2D1_PRESENT_OPTIONS_IMMEDIATELY,
        };

        let rt = unsafe { factory.CreateHwndRenderTarget(&render_props, &hwnd_props)? };
        Ok(rt)
    }

    fn render(state: &OverlayState) {
        let Some(rt) = &state.render_target else { return };
        let Some(br) = &state.brushes else { return };

        unsafe {
            rt.BeginDraw();

            let size = rt.GetSize();

            if state.mode == CaptureMode::Window {
                rt.Clear(Some(&D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }));

                if let Some(ref wr) = state.hovered_rect {
                    let origin_x = state.screen_origin_x as f32;
                    let origin_y = state.screen_origin_y as f32;
                    let wl = wr.left as f32 - origin_x;
                    let wt = wr.top as f32 - origin_y;
                    let wr_right = wr.right as f32 - origin_x;
                    let wb = wr.bottom as f32 - origin_y;

                    // Dark overlay around the window (4 bands)
                    rt.FillRectangle(&D2D_RECT_F {
                        left: 0.0, top: 0.0, right: size.width, bottom: wt,
                    }, &br.dim_brush);
                    rt.FillRectangle(&D2D_RECT_F {
                        left: 0.0, top: wb, right: size.width, bottom: size.height,
                    }, &br.dim_brush);
                    rt.FillRectangle(&D2D_RECT_F {
                        left: 0.0, top: wt, right: wl, bottom: wb,
                    }, &br.dim_brush);
                    rt.FillRectangle(&D2D_RECT_F {
                        left: wr_right, top: wt, right: size.width, bottom: wb,
                    }, &br.dim_brush);

                    // Blue border around the window
                    let rect = D2D_RECT_F {
                        left: wl, top: wt, right: wr_right, bottom: wb,
                    };
                    rt.DrawRectangle(&rect, &br.window_border, 3.0, None);
                } else {
                    rt.FillRectangle(&D2D_RECT_F {
                        left: 0.0, top: 0.0, right: size.width, bottom: size.height,
                    }, &br.dim_brush);
                }

                let _ = rt.EndDraw(None, None);
                return;
            }

            if state.selecting {
                rt.Clear(Some(&D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }));

                let sel_left = state.start_x.min(state.current_x) as f32;
                let sel_top = state.start_y.min(state.current_y) as f32;
                let sel_right = state.start_x.max(state.current_x) as f32;
                let sel_bottom = state.start_y.max(state.current_y) as f32;

                // Dark overlay around the selection (4 bands)
                rt.FillRectangle(&D2D_RECT_F {
                    left: 0.0, top: 0.0, right: size.width, bottom: sel_top,
                }, &br.dim_brush);
                rt.FillRectangle(&D2D_RECT_F {
                    left: 0.0, top: sel_bottom, right: size.width, bottom: size.height,
                }, &br.dim_brush);
                rt.FillRectangle(&D2D_RECT_F {
                    left: 0.0, top: sel_top, right: sel_left, bottom: sel_bottom,
                }, &br.dim_brush);
                rt.FillRectangle(&D2D_RECT_F {
                    left: sel_right, top: sel_top, right: size.width, bottom: sel_bottom,
                }, &br.dim_brush);

                // White border around the clear selection area
                let rect = D2D_RECT_F {
                    left: sel_left, top: sel_top, right: sel_right, bottom: sel_bottom,
                };
                rt.DrawRectangle(&rect, &br.selection_border, 2.0, None);

                // Crosshair lines through current mouse position
                let cx = state.current_x as f32;
                let cy = state.current_y as f32;
                rt.DrawLine(
                    windows_numerics::Vector2 { X: cx, Y: 0.0 },
                    windows_numerics::Vector2 { X: cx, Y: size.height },
                    &br.crosshair_active, 1.0, None,
                );
                rt.DrawLine(
                    windows_numerics::Vector2 { X: 0.0, Y: cy },
                    windows_numerics::Vector2 { X: size.width, Y: cy },
                    &br.crosshair_active, 1.0, None,
                );

                // Dimension text
                Self::draw_dimensions(rt, br, state);
            } else {
                // Pre-selection: full dark overlay with crosshair
                rt.Clear(Some(&D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.3 }));

                let cx = state.current_x as f32;
                let cy = state.current_y as f32;
                rt.DrawLine(
                    windows_numerics::Vector2 { X: cx, Y: 0.0 },
                    windows_numerics::Vector2 { X: cx, Y: size.height },
                    &br.crosshair_idle, 1.0, None,
                );
                rt.DrawLine(
                    windows_numerics::Vector2 { X: 0.0, Y: cy },
                    windows_numerics::Vector2 { X: size.width, Y: cy },
                    &br.crosshair_idle, 1.0, None,
                );
            }

            let _ = rt.EndDraw(None, None);
        }
    }

    /// Find the top-level window under a screen point, skipping our overlay.
    fn window_at_point(screen_x: i32, screen_y: i32, overlay_hwnd: HWND) -> Option<(isize, RECT)> {
        use windows::Win32::Foundation::POINT;

        let pt = POINT { x: screen_x, y: screen_y };

        unsafe {
            // Temporarily hide overlay so WindowFromPoint finds the real window
            let _ = ShowWindow(overlay_hwnd, SW_HIDE);
            let found = WindowFromPoint(pt);
            let _ = ShowWindow(overlay_hwnd, SW_SHOWNOACTIVATE);
            let _ = SetForegroundWindow(overlay_hwnd);

            if found.0.is_null() || found == overlay_hwnd {
                return None;
            }

            // Walk up to the top-level (root owner) window
            let mut top = found;
            loop {
                let parent = GetAncestor(top, GA_ROOT);
                if parent.0.is_null() || parent == top {
                    break;
                }
                top = parent;
            }

            let mut rect = RECT::default();
            if GetWindowRect(top, &mut rect).is_ok() && rect.right > rect.left && rect.bottom > rect.top {
                Some((top.0 as isize, rect))
            } else {
                None
            }
        }
    }

    fn draw_dimensions(
        rt: &ID2D1HwndRenderTarget,
        br: &CachedBrushes,
        state: &OverlayState,
    ) {
        let w = (state.current_x - state.start_x).unsigned_abs();
        let h = (state.current_y - state.start_y).unsigned_abs();
        let text = format!("{}×{}", w, h);
        let wide: Vec<u16> = text.encode_utf16().collect();

        unsafe {
            let text_x = state.start_x.min(state.current_x) as f32;
            let text_y = state.start_y.min(state.current_y) as f32 - 24.0;

            // Background for text
            let bg_rect = D2D_RECT_F {
                left: text_x,
                top: text_y,
                right: text_x + wide.len() as f32 * 8.0 + 16.0,
                bottom: text_y + 22.0,
            };
            let rounded = D2D1_ROUNDED_RECT {
                rect: bg_rect,
                radiusX: 4.0,
                radiusY: 4.0,
            };
            rt.FillRoundedRectangle(&rounded, &br.dim_text_bg);

            let text_rect = D2D_RECT_F {
                left: text_x + 8.0,
                top: text_y + 3.0,
                right: text_x + 200.0,
                bottom: text_y + 22.0,
            };

            rt.DrawText(&wide, &br.dim_text_format, &text_rect, &br.dim_text_fg,
                D2D1_DRAW_TEXT_OPTIONS_NONE, DWRITE_MEASURING_MODE_NATURAL);
        }
    }

    unsafe extern "system" fn wnd_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match msg {
            WM_CREATE => {
                let cs = &*(lparam.0 as *const CREATESTRUCTW);
                if !cs.lpCreateParams.is_null() {
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, cs.lpCreateParams as isize);
                }
                LRESULT(0)
            }

            WM_LBUTTONDOWN => {
                let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Mutex<OverlayState>;
                if !state_ptr.is_null() {
                    let state = &*state_ptr;
                    let mut s = state.lock();

                    if s.mode == CaptureMode::Window {
                        // In Window mode, click selects the hovered window
                        if s.hovered_hwnd != 0 {
                            let target = s.hovered_hwnd;
                            if let Some(cb) = s.callback.take() {
                                drop(s);
                                let _ = ShowWindow(hwnd, SW_HIDE);
                                let _ = DwmFlush();
                                cb(OverlayResult::Window(target));
                                return LRESULT(0);
                            }
                        }
                    } else {
                        // Region/OCR mode: start rubber-band selection
                        let x = (lparam.0 & 0xFFFF) as i16 as i32;
                        let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
                        s.selecting = true;
                        s.start_x = x;
                        s.start_y = y;
                        s.current_x = x;
                        s.current_y = y;
                        SetCapture(hwnd);
                    }
                }
                LRESULT(0)
            }

            WM_MOUSEMOVE => {
                let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Mutex<OverlayState>;
                if !state_ptr.is_null() {
                    let state = &*state_ptr;
                    let mut s = state.lock();
                    let x = (lparam.0 & 0xFFFF) as i16 as i32;
                    let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
                    s.current_x = x;
                    s.current_y = y;

                    if s.mode == CaptureMode::Window && !s.selecting {
                        // Detect window under cursor
                        let screen_x = x + s.screen_origin_x;
                        let screen_y = y + s.screen_origin_y;
                        let overlay_hwnd = s.overlay_hwnd;
                        drop(s);

                        if let Some((target_hwnd, rect)) = Self::window_at_point(screen_x, screen_y, overlay_hwnd) {
                            let mut s = state.lock();
                            s.hovered_hwnd = target_hwnd;
                            s.hovered_rect = Some(rect);
                            Self::render(&s);
                        } else {
                            let mut s = state.lock();
                            s.hovered_hwnd = 0;
                            s.hovered_rect = None;
                            Self::render(&s);
                        }
                    } else {
                        Self::render(&s);
                    }
                }
                LRESULT(0)
            }

            WM_LBUTTONUP => {
                let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Mutex<OverlayState>;
                if !state_ptr.is_null() {
                    let state = &*state_ptr;
                    let mut s = state.lock();
                    if s.selecting {
                        s.selecting = false;
                        let _ = ReleaseCapture();

                        // Client coordinates map directly to screen coordinates
                        // offset by the virtual screen origin
                        let origin_x = s.screen_origin_x;
                        let origin_y = s.screen_origin_y;

                        let rect = Rect::normalized(
                            s.start_x + origin_x,
                            s.start_y + origin_y,
                            s.current_x + origin_x,
                            s.current_y + origin_y,
                        );

                        // Minimum selection size
                        if rect.width > 5 && rect.height > 5 {
                            if let Some(cb) = s.callback.take() {
                                drop(s);
                                let _ = ShowWindow(hwnd, SW_HIDE);
                                let _ = DwmFlush();
                                cb(OverlayResult::Region(rect));
                                return LRESULT(0);
                            }
                        }
                    }
                }
                LRESULT(0)
            }

            WM_KEYDOWN => {
                // Escape to cancel
                if wparam.0 == VK_ESCAPE.0 as usize {
                    let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Mutex<OverlayState>;
                    if !state_ptr.is_null() {
                        let state = &*state_ptr;
                        let mut s = state.lock();
                        s.selecting = false;
                        if let Some(cb) = s.callback.take() {
                            drop(s);
                            let _ = ShowWindow(hwnd, SW_HIDE);
                            let _ = DwmFlush();
                            cb(OverlayResult::Cancelled);
                            return LRESULT(0);
                        }
                    }
                }
                LRESULT(0)
            }

            WM_PAINT => {
                let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Mutex<OverlayState>;
                if !state_ptr.is_null() {
                    let state = &*state_ptr;
                    let s = state.lock();
                    Self::render(&s);
                }
                let _ = ValidateRect(Some(hwnd), None);
                LRESULT(0)
            }

            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

unsafe impl Send for Overlay {}
unsafe impl Sync for Overlay {}
