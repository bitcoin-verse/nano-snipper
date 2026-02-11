//! Small popup action bar shown after capture.
//! Buttons: Copy (Enter), Save (S), Annotate (A), Cancel (Esc).

use std::time::Instant;

use anyhow::Result;
use ns_common::Rect;
use tracing::{debug, info};
use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

/// User's action choice from the action bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionChoice {
    Copy,
    Save,
    Annotate,
    Cancel,
}

pub type ActionCallback = Box<dyn FnOnce(ActionChoice) + 'static>;

/// Cached D2D/DWrite resources for the action bar.
struct CachedActionBarBrushes {
    hover_brush: ID2D1SolidColorBrush,    // hover highlight
    label_brush: ID2D1SolidColorBrush,    // white label text
    shortcut_brush: ID2D1SolidColorBrush, // gray shortcut text
    border_orange: ID2D1SolidColorBrush,  // countdown active
    border_grey: ID2D1SolidColorBrush,    // countdown elapsed
    label_format: IDWriteTextFormat,       // 13pt semi-bold, centered
    shortcut_format: IDWriteTextFormat,    // 10pt normal, centered
}

struct ActionBarState {
    d2d_factory: Option<ID2D1Factory1>,
    render_target: Option<ID2D1HwndRenderTarget>,
    dwrite_factory: Option<IDWriteFactory>,
    brushes: Option<CachedActionBarBrushes>,
    callback: Option<ActionCallback>,
    hovered_button: i32, // -1 = none, 0-3 = button index
    auto_copy_timer: bool,
    countdown_start: Option<Instant>,
    countdown_total_secs: f32,
}

const ACTION_BAR_CLASS: PCWSTR = w!("NanoSnipperActionBar");
const BAR_WIDTH: i32 = 300;
const BAR_HEIGHT: i32 = 48;
const BUTTON_COUNT: i32 = 2;
const BUTTON_WIDTH: f32 = BAR_WIDTH as f32 / BUTTON_COUNT as f32;
const TIMER_AUTO_COPY: usize = 100;
const TIMER_ANIMATE: usize = 101;

pub struct ActionBar {
    hwnd: HWND,
}

impl ActionBar {
    /// Pre-create the action bar window (hidden).
    pub fn new() -> Result<Self> {
        let instance = unsafe { GetModuleHandleW(None)? };

        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(Self::wnd_proc),
            hInstance: instance.into(),
            lpszClassName: ACTION_BAR_CLASS,
            hCursor: unsafe { LoadCursorW(None, IDC_ARROW)? },
            ..Default::default()
        };

        unsafe { RegisterClassExW(&wc) };

        let state = Box::new(ActionBarState {
            d2d_factory: None,
            render_target: None,
            dwrite_factory: None,
            brushes: None,
            callback: None,
            hovered_button: -1,
            auto_copy_timer: false,
            countdown_start: None,
            countdown_total_secs: 0.0,
        });

        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
                ACTION_BAR_CLASS,
                w!(""),
                WS_POPUP,
                0,
                0,
                BAR_WIDTH,
                BAR_HEIGHT,
                None,
                None,
                Some(HINSTANCE(instance.0)),
                Some(Box::into_raw(state) as *const std::ffi::c_void),
            )?
        };

        info!("Action bar pre-created");
        Ok(Self { hwnd })
    }

    /// Show the action bar below the given region.
    pub fn show(&self, region: &Rect, auto_copy_secs: u32, callback: ActionCallback) -> Result<()> {
        let x = region.x + (region.width as i32 / 2) - (BAR_WIDTH / 2);
        let y = region.bottom() + 8;

        unsafe {
            let state_ptr = GetWindowLongPtrW(self.hwnd, GWLP_USERDATA) as *mut ActionBarState;
            if !state_ptr.is_null() {
                (*state_ptr).callback = Some(callback);
                (*state_ptr).hovered_button = -1;
            }

            SetWindowPos(self.hwnd, Some(HWND_TOPMOST), x, y, BAR_WIDTH, BAR_HEIGHT, SWP_SHOWWINDOW)?;
            let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
            let _ = SetForegroundWindow(self.hwnd);

            if auto_copy_secs > 0 {
                SetTimer(Some(self.hwnd), TIMER_AUTO_COPY, auto_copy_secs * 1000, None);
                SetTimer(Some(self.hwnd), TIMER_ANIMATE, 16, None); // ~60fps
                if !state_ptr.is_null() {
                    (*state_ptr).auto_copy_timer = true;
                    (*state_ptr).countdown_start = Some(Instant::now());
                    (*state_ptr).countdown_total_secs = auto_copy_secs as f32;
                }
            } else if !state_ptr.is_null() {
                (*state_ptr).countdown_start = None;
            }
        }

        debug!("Action bar shown at ({}, {})", x, y);
        Ok(())
    }

    /// Hide the action bar.
    pub fn hide(&self) {
        unsafe {
            let state_ptr = GetWindowLongPtrW(self.hwnd, GWLP_USERDATA) as *mut ActionBarState;
            if !state_ptr.is_null() {
                if (*state_ptr).auto_copy_timer {
                    KillTimer(Some(self.hwnd), TIMER_AUTO_COPY).ok();
                    (*state_ptr).auto_copy_timer = false;
                }
                KillTimer(Some(self.hwnd), TIMER_ANIMATE).ok();
                (*state_ptr).countdown_start = None;
            }
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
    }

    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }

    fn render(state: &ActionBarState, _hwnd: HWND) {
        let Some(rt) = &state.render_target else { return };
        let Some(br) = &state.brushes else { return };

        unsafe {
            rt.BeginDraw();
            rt.Clear(Some(&D2D1_COLOR_F { r: 0.12, g: 0.12, b: 0.14, a: 0.95 }));

            let labels = ["Save", "Annotate"];
            let shortcuts = ["S", "A"];

            for i in 0..BUTTON_COUNT {
                let x = i as f32 * BUTTON_WIDTH;
                let is_hovered = state.hovered_button == i;

                // Button background on hover
                if is_hovered {
                    let rect = D2D_RECT_F {
                        left: x + 2.0,
                        top: 2.0,
                        right: x + BUTTON_WIDTH - 2.0,
                        bottom: BAR_HEIGHT as f32 - 2.0,
                    };
                    let rounded = D2D1_ROUNDED_RECT { rect, radiusX: 6.0, radiusY: 6.0 };
                    rt.FillRoundedRectangle(&rounded, &br.hover_brush);
                }

                // Label text
                let label = labels[i as usize];
                let wide: Vec<u16> = label.encode_utf16().collect();
                let rect = D2D_RECT_F {
                    left: x, top: 6.0, right: x + BUTTON_WIDTH, bottom: 24.0,
                };
                rt.DrawText(&wide, &br.label_format, &rect, &br.label_brush,
                    D2D1_DRAW_TEXT_OPTIONS_NONE, DWRITE_MEASURING_MODE_NATURAL);

                // Shortcut hint
                let shortcut = shortcuts[i as usize];
                let wide: Vec<u16> = shortcut.encode_utf16().collect();
                let rect = D2D_RECT_F {
                    left: x, top: 26.0, right: x + BUTTON_WIDTH, bottom: 42.0,
                };
                rt.DrawText(&wide, &br.shortcut_format, &rect, &br.shortcut_brush,
                    D2D1_DRAW_TEXT_OPTIONS_NONE, DWRITE_MEASURING_MODE_NATURAL);
            }

            Self::draw_countdown_border(state, rt, br);

            let _ = rt.EndDraw(None, None);
        }
    }

    fn draw_countdown_border(state: &ActionBarState, rt: &ID2D1HwndRenderTarget, br: &CachedActionBarBrushes) {
        let w = BAR_WIDTH as f32;
        let h = BAR_HEIGHT as f32;
        let t = 2.0_f32; // border thickness
        let perimeter = 2.0 * w + 2.0 * h;

        // Calculate progress (fraction of perimeter that should be grey)
        let grey_dist = if let Some(start) = state.countdown_start {
            let elapsed = start.elapsed().as_secs_f32();
            let progress = (elapsed / state.countdown_total_secs).min(1.0);
            progress * perimeter
        } else {
            0.0 // no countdown — full orange
        };

        // Segments going clockwise from top-center:
        // Seg 0: top edge, center→right   length = W/2
        // Seg 1: right edge, top→bottom   length = H
        // Seg 2: bottom edge, right→left  length = W
        // Seg 3: left edge, bottom→top    length = H
        // Seg 4: top edge, left→center    length = W/2
        let seg_lengths = [w / 2.0, h, w, h, w / 2.0];
        let mut cum = 0.0_f32;

        for (i, &seg_len) in seg_lengths.iter().enumerate() {
            let seg_start = cum;
            let seg_end = cum + seg_len;
            cum = seg_end;

            // How much of this segment is grey vs orange
            if grey_dist >= seg_end {
                // Fully grey
                Self::fill_border_segment(rt, &br.border_grey, i, 0.0, seg_len, w, h, t);
            } else if grey_dist <= seg_start {
                // Fully orange
                Self::fill_border_segment(rt, &br.border_orange, i, 0.0, seg_len, w, h, t);
            } else {
                // Split: grey portion then orange portion
                let grey_part = grey_dist - seg_start;
                Self::fill_border_segment(rt, &br.border_grey, i, 0.0, grey_part, w, h, t);
                Self::fill_border_segment(rt, &br.border_orange, i, grey_part, seg_len, w, h, t);
            }
        }
    }

    /// Fill a portion of a border segment with the given brush.
    /// `from` and `to` are distances along the segment (0..seg_len).
    fn fill_border_segment(
        rt: &ID2D1HwndRenderTarget,
        brush: &ID2D1SolidColorBrush,
        seg_idx: usize,
        from: f32,
        to: f32,
        w: f32,
        h: f32,
        t: f32,
    ) {
        if to <= from { return; }

        let rect = match seg_idx {
            0 => {
                // Top edge, center → right: x goes from W/2 to W
                D2D_RECT_F {
                    left: w / 2.0 + from,
                    top: 0.0,
                    right: w / 2.0 + to,
                    bottom: t,
                }
            }
            1 => {
                // Right edge, top → bottom
                D2D_RECT_F {
                    left: w - t,
                    top: from,
                    right: w,
                    bottom: to,
                }
            }
            2 => {
                // Bottom edge, right → left: x goes from W to 0
                D2D_RECT_F {
                    left: w - to,
                    top: h - t,
                    right: w - from,
                    bottom: h,
                }
            }
            3 => {
                // Left edge, bottom → top: y goes from H to 0
                D2D_RECT_F {
                    left: 0.0,
                    top: h - to,
                    right: t,
                    bottom: h - from,
                }
            }
            4 => {
                // Top edge left half, left → center: x goes from 0 to W/2
                D2D_RECT_F {
                    left: from,
                    top: 0.0,
                    right: to,
                    bottom: t,
                }
            }
            _ => return,
        };

        unsafe { rt.FillRectangle(&rect, brush); }
    }

    fn create_brushes(rt: &ID2D1HwndRenderTarget, dwrite: &IDWriteFactory) -> anyhow::Result<CachedActionBarBrushes> {
        unsafe {
            let hover_brush = rt.CreateSolidColorBrush(
                &D2D1_COLOR_F { r: 0.25, g: 0.25, b: 0.28, a: 1.0 }, None)?;
            let label_brush = rt.CreateSolidColorBrush(
                &D2D1_COLOR_F { r: 0.95, g: 0.95, b: 0.97, a: 1.0 }, None)?;
            let shortcut_brush = rt.CreateSolidColorBrush(
                &D2D1_COLOR_F { r: 0.6, g: 0.6, b: 0.65, a: 1.0 }, None)?;
            let border_orange = rt.CreateSolidColorBrush(
                &D2D1_COLOR_F { r: 1.0, g: 0.55, b: 0.0, a: 1.0 }, None)?;
            let border_grey = rt.CreateSolidColorBrush(
                &D2D1_COLOR_F { r: 0.3, g: 0.3, b: 0.32, a: 1.0 }, None)?;

            let label_format = dwrite.CreateTextFormat(
                w!("Segoe UI"), None,
                DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL, 13.0, w!("en-US"))?;
            label_format.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;

            let shortcut_format = dwrite.CreateTextFormat(
                w!("Segoe UI"), None,
                DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL, 10.0, w!("en-US"))?;
            shortcut_format.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;

            Ok(CachedActionBarBrushes {
                hover_brush, label_brush, shortcut_brush,
                border_orange, border_grey,
                label_format, shortcut_format,
            })
        }
    }

    fn button_index_at(x: i32) -> i32 {
        let idx = x / BUTTON_WIDTH as i32;
        if idx >= 0 && idx < BUTTON_COUNT { idx } else { -1 }
    }

    fn fire_action(state: &mut ActionBarState, choice: ActionChoice, hwnd: HWND) {
        if let Some(cb) = state.callback.take() {
            unsafe {
                if state.auto_copy_timer {
                    KillTimer(Some(hwnd), TIMER_AUTO_COPY).ok();
                    state.auto_copy_timer = false;
                }
                KillTimer(Some(hwnd), TIMER_ANIMATE).ok();
                state.countdown_start = None;
                let _ = ShowWindow(hwnd, SW_HIDE);
            }
            cb(choice);
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

                    let state = &mut *(cs.lpCreateParams as *mut ActionBarState);

                    let options = D2D1_FACTORY_OPTIONS::default();
                    if let Ok(factory) = D2D1CreateFactory::<ID2D1Factory1>(
                        D2D1_FACTORY_TYPE_SINGLE_THREADED,
                        Some(&options),
                    ) {
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
                            pixelSize: D2D_SIZE_U {
                                width: BAR_WIDTH as u32,
                                height: BAR_HEIGHT as u32,
                            },
                            presentOptions: D2D1_PRESENT_OPTIONS_IMMEDIATELY,
                        };
                        if let Ok(rt) = factory.CreateHwndRenderTarget(&render_props, &hwnd_props) {
                            // Create cached brushes
                            if let Ok(dwrite) = DWriteCreateFactory::<IDWriteFactory>(DWRITE_FACTORY_TYPE_SHARED) {
                                state.brushes = Self::create_brushes(&rt, &dwrite).ok();
                                state.dwrite_factory = Some(dwrite);
                            }
                            state.render_target = Some(rt);
                        }
                        state.d2d_factory = Some(factory);
                    }
                }
                LRESULT(0)
            }

            WM_PAINT => {
                let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut ActionBarState;
                if !state_ptr.is_null() {
                    Self::render(&*state_ptr, hwnd);
                }
                let _ = ValidateRect(Some(hwnd), None);
                LRESULT(0)
            }

            WM_MOUSEMOVE => {
                let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut ActionBarState;
                if !state_ptr.is_null() {
                    let x = (lparam.0 & 0xFFFF) as i16 as i32;
                    let new_idx = Self::button_index_at(x);
                    if (*state_ptr).hovered_button != new_idx {
                        (*state_ptr).hovered_button = new_idx;
                        let _ = InvalidateRect(Some(hwnd), None, false);
                    }
                }
                LRESULT(0)
            }

            WM_LBUTTONUP => {
                let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut ActionBarState;
                if !state_ptr.is_null() {
                    let x = (lparam.0 & 0xFFFF) as i16 as i32;
                    let idx = Self::button_index_at(x);
                    let choice = match idx {
                        0 => ActionChoice::Save,
                        1 => ActionChoice::Annotate,
                        _ => return LRESULT(0),
                    };
                    Self::fire_action(&mut *state_ptr, choice, hwnd);
                }
                LRESULT(0)
            }

            WM_KEYDOWN => {
                let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut ActionBarState;
                if !state_ptr.is_null() {
                    let choice = match wparam.0 as u16 {
                        k if k == VK_ESCAPE.0 => Some(ActionChoice::Cancel),
                        0x53 => Some(ActionChoice::Save),   // 'S'
                        0x41 => Some(ActionChoice::Annotate),// 'A'
                        _ => None,
                    };
                    if let Some(choice) = choice {
                        Self::fire_action(&mut *state_ptr, choice, hwnd);
                    }
                }
                LRESULT(0)
            }

            WM_TIMER => {
                if wparam.0 == TIMER_AUTO_COPY {
                    let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut ActionBarState;
                    if !state_ptr.is_null() {
                        Self::fire_action(&mut *state_ptr, ActionChoice::Copy, hwnd);
                    }
                } else if wparam.0 == TIMER_ANIMATE {
                    let _ = InvalidateRect(Some(hwnd), None, false);
                }
                LRESULT(0)
            }

            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

unsafe impl Send for ActionBar {}
unsafe impl Sync for ActionBar {}
