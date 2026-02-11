//! Pin-to-screen windows: always-on-top screenshot previews.
//! Features: move, resize, opacity via mouse wheel.

use anyhow::Result;
use ns_common::PixelBuffer;
use tracing::info;
use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;

const PIN_CLASS: PCWSTR = w!("NanoSnipperPin");

struct PinState {
    d2d_factory: Option<ID2D1Factory1>,
    render_target: Option<ID2D1HwndRenderTarget>,
    bitmap: Option<ID2D1Bitmap>,
    _original_width: u32,
    _original_height: u32,
    opacity: f32,
}

pub struct PinWindow {
    hwnd: HWND,
}

impl PinWindow {
    /// Create a new pin window displaying the given pixel buffer.
    pub fn new(pixels: &PixelBuffer) -> Result<Self> {
        static REGISTERED: std::sync::Once = std::sync::Once::new();

        let instance = unsafe { GetModuleHandleW(None)? };

        REGISTERED.call_once(|| {
            let wc = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                style: CS_HREDRAW | CS_VREDRAW,
                lpfnWndProc: Some(Self::wnd_proc),
                hInstance: instance.into(),
                lpszClassName: PIN_CLASS,
                hCursor: unsafe { LoadCursorW(None, IDC_ARROW).unwrap_or_default() },
                ..Default::default()
            };
            unsafe { RegisterClassExW(&wc) };
        });

        let state = Box::new(PinState {
            d2d_factory: None,
            render_target: None,
            bitmap: None,
            _original_width: pixels.width,
            _original_height: pixels.height,
            opacity: 1.0,
        });

        // Size the window to the image, capped at 800x600
        let w = pixels.width.min(800) as i32;
        let h = pixels.height.min(600) as i32;

        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
                PIN_CLASS,
                w!("Pin"),
                WS_POPUP | WS_THICKFRAME,
                100,
                100,
                w,
                h,
                None,
                None,
                Some(HINSTANCE(instance.0)),
                Some(Box::into_raw(state) as *const std::ffi::c_void),
            )?
        };

        // Create D2D bitmap from pixel data
        unsafe {
            let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut PinState;
            if !state_ptr.is_null() {
                if let Some(rt) = &(*state_ptr).render_target {
                    let props = D2D1_BITMAP_PROPERTIES {
                        pixelFormat: D2D1_PIXEL_FORMAT {
                            format: DXGI_FORMAT_B8G8R8A8_UNORM,
                            alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                        },
                        dpiX: 96.0,
                        dpiY: 96.0,
                    };
                    let size = D2D_SIZE_U {
                        width: pixels.width,
                        height: pixels.height,
                    };
                    if let Ok(bmp) = rt.CreateBitmap(size, Some(pixels.data.as_ptr() as *const _), pixels.stride, &props) {
                        (*state_ptr).bitmap = Some(bmp);
                    }
                }
            }
        }

        unsafe {
            let _ = ShowWindow(hwnd, SW_SHOW);
        }

        info!("Pin window created: {}x{}", pixels.width, pixels.height);
        Ok(Self { hwnd })
    }

    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }

    /// Close and destroy this pin window.
    pub fn close(&self) {
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }

    fn render(state: &PinState) {
        let Some(rt) = &state.render_target else { return };

        unsafe {
            rt.BeginDraw();
            rt.Clear(Some(&D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 1.0 }));

            if let Some(bmp) = &state.bitmap {
                let size = rt.GetSize();
                let dst_rect = D2D_RECT_F {
                    left: 0.0,
                    top: 0.0,
                    right: size.width,
                    bottom: size.height,
                };
                rt.DrawBitmap(
                    bmp,
                    Some(&dst_rect),
                    state.opacity,
                    D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
                    None,
                );
            }

            let _ = rt.EndDraw(None, None);
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

                    let state = &mut *(cs.lpCreateParams as *mut PinState);

                    let options = D2D1_FACTORY_OPTIONS::default();
                    if let Ok(factory) = D2D1CreateFactory::<ID2D1Factory1>(
                        D2D1_FACTORY_TYPE_SINGLE_THREADED,
                        Some(&options),
                    ) {
                        let mut rect = RECT::default();
                        GetClientRect(hwnd, &mut rect).ok();
                        let w = (rect.right - rect.left).max(1) as u32;
                        let h = (rect.bottom - rect.top).max(1) as u32;

                        let render_props = D2D1_RENDER_TARGET_PROPERTIES {
                            r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
                            pixelFormat: D2D1_PIXEL_FORMAT {
                                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                            },
                            ..Default::default()
                        };
                        let hwnd_props = D2D1_HWND_RENDER_TARGET_PROPERTIES {
                            hwnd,
                            pixelSize: D2D_SIZE_U { width: w, height: h },
                            presentOptions: D2D1_PRESENT_OPTIONS_IMMEDIATELY,
                        };
                        if let Ok(rt) = factory.CreateHwndRenderTarget(&render_props, &hwnd_props) {
                            state.render_target = Some(rt);
                        }
                        state.d2d_factory = Some(factory);
                    }
                }
                LRESULT(0)
            }

            WM_PAINT => {
                let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut PinState;
                if !state_ptr.is_null() {
                    Self::render(&*state_ptr);
                }
                let _ = ValidateRect(Some(hwnd), None);
                LRESULT(0)
            }

            WM_SIZE => {
                let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut PinState;
                if !state_ptr.is_null() {
                    if let Some(rt) = &(*state_ptr).render_target {
                        let w = (lparam.0 & 0xFFFF) as u32;
                        let h = ((lparam.0 >> 16) & 0xFFFF) as u32;
                        let _ = rt.Resize(&D2D_SIZE_U { width: w.max(1), height: h.max(1) });
                    }
                }
                LRESULT(0)
            }

            // Allow dragging by clicking anywhere
            WM_NCHITTEST => {
                let result = DefWindowProcW(hwnd, msg, wparam, lparam);
                if result == LRESULT(1) {
                    // HTCLIENT -> HTCAPTION for drag
                    return LRESULT(2); // HTCAPTION
                }
                result
            }

            // Mouse wheel for opacity
            WM_MOUSEWHEEL => {
                let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut PinState;
                if !state_ptr.is_null() {
                    let delta = (wparam.0 >> 16) as i16;
                    let state = &mut *state_ptr;
                    if delta > 0 {
                        state.opacity = (state.opacity + 0.1).min(1.0);
                    } else {
                        state.opacity = (state.opacity - 0.1).max(0.1);
                    }
                    let _ = InvalidateRect(Some(hwnd), None, false);
                }
                LRESULT(0)
            }

            // Close on Escape
            WM_KEYDOWN => {
                if wparam.0 == 0x1B {
                    // VK_ESCAPE
                    let _ = DestroyWindow(hwnd);
                }
                LRESULT(0)
            }

            WM_DESTROY => {
                let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut PinState;
                if !state_ptr.is_null() {
                    let _ = Box::from_raw(state_ptr);
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                }
                LRESULT(0)
            }

            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

unsafe impl Send for PinWindow {}
unsafe impl Sync for PinWindow {}
