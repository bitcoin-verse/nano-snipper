//! Annotation editor window: Draw on top of a captured screenshot.
//!
//! Tools: Arrow, Rectangle, Highlight, Pen, Blur, Text.
//! Features: Color picker (9 colors), thickness presets (3), undo/redo,
//! keyboard shortcuts, icon toolbar, real pixelation blur preview.
//! Built with Win32 + Direct2D.

use anyhow::Result;
use ns_common::PixelBuffer;
use snip_annotate::{Annotation, AnnotationLayer, AnnotationTool, Color, Point, UndoAction};
use tracing::info;
use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::Graphics::Dwm::*;
use windows::Win32::UI::WindowsAndMessaging::*;

// ─── Layout constants ────────────────────────────────────────────────────────

const EDITOR_CLASS: PCWSTR = w!("NanoSnipperAnnotation");
const TOOLBAR_HEIGHT: f32 = 90.0;

// Row 1: tool + action buttons
const ROW1_TOP: f32 = 5.0;
const ROW1_BTN_SIZE: f32 = 45.0;
const ROW1_BTN_STEP: f32 = 48.0; // size + 3px gap
const SEP_W: f32 = 14.0;

// Row 2: color swatches + thickness presets
const ROW2_TOP: f32 = 56.0;
const SWATCH_SIZE: f32 = 20.0;
const SWATCH_STEP: f32 = 24.0; // 20 + 4px gap
const THICKNESS_PRESETS: [f32; 3] = [2.0, 5.0, 10.0];

const MARGIN_X: f32 = 12.0;

// Emoji palette for the emoji stamp tool
const EMOJI_PALETTE: &[&str] = &[
    "👍", "❤️", "😀", "😂", "🤔", "😱", "🎉", "⭐",
    "🔥", "✅", "❌", "⚠️", "💡", "🐛", "📌", "👀",
];

// Minimum window dimensions so the toolbar is never clipped.
// Row 1: MARGIN_X + 7 tools*48 + 2 + SEP + 2 undo/redo*48 + 2 + SEP + 2 done/cancel*48 + MARGIN_X
const MIN_CLIENT_W: i32 = 590;
const MIN_CLIENT_H: i32 = TOOLBAR_HEIGHT as i32 + 120;

/// Callback when annotation editor is done.
pub type EditorCallback = Box<dyn FnOnce(Option<AnnotationLayer>) + 'static>;

/// Callback fired after each annotation mutation (draw, undo, redo, text commit).
/// Receives the original pixels and the current annotation layer so the caller
/// can bake and update the clipboard in real time.
pub type EditorOnChange = Box<dyn Fn(&PixelBuffer, &AnnotationLayer) + 'static>;

// ─── Toolbar hit testing ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy)]
enum ToolbarHit {
    None,
    Tool(AnnotationTool),
    Undo,
    Redo,
    Done,
    Cancel,
    Color(usize),
    Thickness(usize),
    Emoji(usize),
}

// ─── Cached D2D brushes (created once, reused every frame) ───────────────────

struct CachedEditorBrushes {
    // Toolbar
    toolbar_bg: ID2D1SolidColorBrush,
    separator: ID2D1SolidColorBrush,
    divider: ID2D1SolidColorBrush,
    icon_normal: ID2D1SolidColorBrush,
    icon_white: ID2D1SolidColorBrush,
    active_bg: ID2D1SolidColorBrush,
    hover_bg: ID2D1SolidColorBrush,
    disabled: ID2D1SolidColorBrush,
    hint_text: ID2D1SolidColorBrush,
    outline: ID2D1SolidColorBrush,
    selection_ring: ID2D1SolidColorBrush,
    // Done/Cancel
    done_bg: ID2D1SolidColorBrush,
    done_icon: ID2D1SolidColorBrush,
    done_icon_hover: ID2D1SolidColorBrush,
    cancel_bg: ID2D1SolidColorBrush,
    cancel_icon: ID2D1SolidColorBrush,
    cancel_icon_hover: ID2D1SolidColorBrush,

    // Color palette brushes (9 colors)
    palette: [ID2D1SolidColorBrush; 9],

    // Cached DWrite text format for hint labels
    hint_format: IDWriteTextFormat,
}

impl CachedEditorBrushes {
    fn create(rt: &ID2D1HwndRenderTarget, dwrite: &IDWriteFactory) -> Option<Self> {
        unsafe {
            let b = |r, g, b, a| -> Option<ID2D1SolidColorBrush> {
                rt.CreateSolidColorBrush(
                    &D2D1_COLOR_F { r, g, b, a },
                    None,
                ).ok()
            };
            let palette_colors = Color::PALETTE;
            let palette_vec: Option<Vec<ID2D1SolidColorBrush>> = palette_colors.iter()
                .map(|c| rt.CreateSolidColorBrush(&to_d2d_color(c), None).ok())
                .collect();
            let palette: [ID2D1SolidColorBrush; 9] = palette_vec?
                .try_into().ok()?;

            let hint_format = dwrite.CreateTextFormat(
                w!("Segoe UI"), None,
                DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL, 11.0, w!("en-US"),
            ).ok()?;
            let _ = hint_format.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_TRAILING);
            let _ = hint_format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_FAR);

            Some(Self {
                toolbar_bg: b(0.1, 0.1, 0.12, 1.0)?,
                separator: b(0.25, 0.25, 0.28, 1.0)?,
                divider: b(0.25, 0.25, 0.28, 1.0)?,
                icon_normal: b(0.88, 0.88, 0.92, 1.0)?,
                icon_white: b(1.0, 1.0, 1.0, 1.0)?,
                active_bg: b(0.2, 0.45, 0.85, 1.0)?,
                hover_bg: b(0.2, 0.2, 0.25, 1.0)?,
                disabled: b(0.4, 0.4, 0.42, 1.0)?,
                hint_text: b(0.55, 0.55, 0.6, 1.0)?,
                outline: b(0.7, 0.7, 0.72, 1.0)?,
                selection_ring: b(1.0, 1.0, 1.0, 1.0)?,
                done_bg: b(0.15, 0.55, 0.25, 1.0)?,
                done_icon: b(0.3, 0.8, 0.4, 1.0)?,
                done_icon_hover: b(0.3, 0.9, 0.4, 1.0)?,
                cancel_bg: b(0.6, 0.15, 0.15, 1.0)?,
                cancel_icon: b(0.9, 0.35, 0.35, 1.0)?,
                cancel_icon_hover: b(1.0, 0.4, 0.4, 1.0)?,
                palette,
                hint_format,
            })
        }
    }
}

// ─── Editor state ────────────────────────────────────────────────────────────

struct EditorState {
    // D2D resources
    d2d_factory: Option<ID2D1Factory1>,
    render_target: Option<ID2D1HwndRenderTarget>,
    dwrite_factory: Option<IDWriteFactory>,
    bitmap: Option<ID2D1Bitmap>,
    cached_brushes: Option<CachedEditorBrushes>,

    // Original pixels (for blur preview sampling)
    pixels: Option<PixelBuffer>,
    img_width: u32,
    img_height: u32,

    // Annotation state
    layer: AnnotationLayer,
    undo_stack: Vec<UndoAction>,
    redo_stack: Vec<UndoAction>,
    tool: AnnotationTool,
    drawing: bool,
    draw_start: Point,
    draw_current: Point,
    pen_points: Vec<Point>,

    // Right-click move selection
    selected_index: Option<usize>,
    dragging_selected: bool,
    drag_start_img: Point,
    drag_last_img: Point,

    // Text editing
    text_editing: bool,
    text_position: Point,
    text_buffer: String,

    // Tool settings
    stroke_color: Color,
    highlight_color: Color,
    thickness: f32,
    color_index: usize,
    thickness_index: usize,
    emoji_index: usize,

    // Toolbar hover
    hovered_btn: i32, // row 1 button index, -1 if none

    // Cached stroke style for round-capped pen lines
    pen_stroke_style: Option<ID2D1StrokeStyle>,

    // Canvas transform: maps image-space to screen-space
    canvas_scale: f32,
    canvas_offset_x: f32,
    canvas_offset_y: f32,

    // Callbacks
    callback: Option<EditorCallback>,
    on_change: Option<EditorOnChange>,
}

pub struct AnnotationEditor {
    hwnd: HWND,
}

impl AnnotationEditor {
    /// Create a persistent, hidden annotation editor window.
    /// Call `show()` to display it with a new screenshot.
    pub fn create_hidden() -> Result<Self> {
        static REGISTERED: std::sync::Once = std::sync::Once::new();

        let instance = unsafe { GetModuleHandleW(None)? };

        REGISTERED.call_once(|| {
            let wc = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                style: CS_HREDRAW | CS_VREDRAW,
                lpfnWndProc: Some(Self::wnd_proc),
                hInstance: instance.into(),
                lpszClassName: EDITOR_CLASS,
                hCursor: HCURSOR::default(), // handled via WM_SETCURSOR
                ..Default::default()
            };
            unsafe { RegisterClassExW(&wc) };
        });

        let initial_color = Color::YELLOW_OPAQUE;
        let state = Box::new(EditorState {
            d2d_factory: None,
            render_target: None,
            dwrite_factory: None,
            bitmap: None,
            cached_brushes: None,
            pixels: None,
            img_width: 0,
            img_height: 0,
            layer: AnnotationLayer::new(),
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            tool: AnnotationTool::Arrow,
            drawing: false,
            draw_start: Point { x: 0.0, y: 0.0 },
            draw_current: Point { x: 0.0, y: 0.0 },
            pen_points: Vec::new(),
            selected_index: None,
            dragging_selected: false,
            drag_start_img: Point { x: 0.0, y: 0.0 },
            drag_last_img: Point { x: 0.0, y: 0.0 },
            text_editing: false,
            text_position: Point { x: 0.0, y: 0.0 },
            text_buffer: String::new(),
            stroke_color: initial_color,
            highlight_color: initial_color.as_highlight(),
            thickness: THICKNESS_PRESETS[1],
            color_index: 2,
            thickness_index: 1,
            emoji_index: 0,
            hovered_btn: -1,
            pen_stroke_style: None,
            canvas_scale: 1.0,
            canvas_offset_x: 0.0,
            canvas_offset_y: 0.0,
            callback: None,
            on_change: None,
        });

        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_TOPMOST,
                EDITOR_CLASS,
                w!("Nano Snipper - Sponsored by Bitcoin.com"),
                WS_POPUP | WS_CAPTION | WS_SYSMENU | WS_THICKFRAME,
                0, 0, 400, 300, // will be resized on show()
                None,
                None,
                Some(HINSTANCE(instance.0)),
                Some(Box::into_raw(state) as *const std::ffi::c_void),
            )?
        };

        // Enable dark title bar
        unsafe {
            let dark: windows_core::BOOL = true.into();
            let _ = DwmSetWindowAttribute(
                hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE,
                &dark as *const _ as *const std::ffi::c_void,
                std::mem::size_of::<windows_core::BOOL>() as u32,
            );
        }

        // Set window icon
        unsafe {
            let hinstance: HINSTANCE = instance.into();
            if let Ok(icon) = LoadIconW(Some(hinstance), PCWSTR(1 as *const u16)) {
                SendMessageW(hwnd, WM_SETICON, Some(WPARAM(0)), Some(LPARAM(icon.0 as isize)));
                SendMessageW(hwnd, WM_SETICON, Some(WPARAM(1)), Some(LPARAM(icon.0 as isize)));
            }
        }

        info!("Annotation editor pre-created (hidden)");
        Ok(Self { hwnd })
    }

    /// Legacy API: create and immediately show. Used when no persistent editor exists.
    pub fn new(pixels: &PixelBuffer, callback: EditorCallback) -> Result<Self> {
        let editor = Self::create_hidden()?;
        editor.show(pixels, callback, None)?;
        Ok(editor)
    }

    /// Show the persistent editor with a new screenshot. Resets all annotation state.
    pub fn show(
        &self,
        pixels: &PixelBuffer,
        callback: EditorCallback,
        on_change: Option<EditorOnChange>,
    ) -> Result<()> {
        let screen_w = unsafe { GetSystemMetrics(SM_CXSCREEN) };
        let screen_h = unsafe { GetSystemMetrics(SM_CYSCREEN) };
        let w = (pixels.width as i32).max(MIN_CLIENT_W).min(screen_w - 100);
        let h = (pixels.height as i32 + TOOLBAR_HEIGHT as i32).max(MIN_CLIENT_H).min(screen_h - 100);

        // Resize and center window
        unsafe {
            let _ = SetWindowPos(
                self.hwnd, Some(HWND_TOPMOST),
                (screen_w - w) / 2, (screen_h - h) / 2, w, h,
                SWP_NOACTIVATE,
            );
        }

        // Reset editor state and upload new bitmap
        unsafe {
            let state_ptr = GetWindowLongPtrW(self.hwnd, GWLP_USERDATA) as *mut EditorState;
            if state_ptr.is_null() {
                info!("Annotation editor state is NULL — window was destroyed, cannot show");
                return Err(anyhow::anyhow!("Editor state destroyed (GWLP_USERDATA is null)"));
            }
            {
                let state = &mut *state_ptr;

                // Reset annotation state
                state.layer = AnnotationLayer::new();
                state.undo_stack.clear();
                state.redo_stack.clear();
                state.tool = AnnotationTool::Arrow;
                state.drawing = false;
                state.pen_points.clear();
                state.selected_index = None;
                state.dragging_selected = false;
                state.text_editing = false;
                state.text_buffer.clear();
                state.hovered_btn = -1;
                state.pixels = Some(pixels.clone());
                state.img_width = pixels.width;
                state.img_height = pixels.height;
                state.callback = Some(callback);
                state.on_change = on_change;

                // Reset color/thickness to defaults
                state.color_index = 2;
                state.stroke_color = Color::YELLOW_OPAQUE;
                state.highlight_color = Color::YELLOW_OPAQUE.as_highlight();
                state.thickness_index = 1;
                state.thickness = THICKNESS_PRESETS[1];
                state.emoji_index = 0;

                // Resize render target and recompute canvas transform
                if let Some(rt) = &state.render_target {
                    let mut rect = RECT::default();
                    GetClientRect(self.hwnd, &mut rect).ok();
                    let cw = (rect.right - rect.left).max(1) as u32;
                    let ch = (rect.bottom - rect.top).max(1) as u32;
                    let _ = rt.Resize(&D2D_SIZE_U { width: cw, height: ch });

                    // Create new D2D bitmap from pixel data
                    let props = D2D1_BITMAP_PROPERTIES {
                        pixelFormat: D2D1_PIXEL_FORMAT {
                            format: DXGI_FORMAT_B8G8R8A8_UNORM,
                            alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                        },
                        dpiX: 96.0,
                        dpiY: 96.0,
                    };
                    let size = D2D_SIZE_U { width: pixels.width, height: pixels.height };
                    state.bitmap = rt.CreateBitmap(
                        size,
                        Some(pixels.data.as_ptr() as *const _),
                        pixels.stride,
                        &props,
                    ).ok();
                }

                // Recompute canvas transform (outside rt borrow)
                let mut rect = RECT::default();
                GetClientRect(self.hwnd, &mut rect).ok();
                let cw = (rect.right - rect.left).max(1) as f32;
                let ch = (rect.bottom - rect.top).max(1) as f32;
                Self::recompute_canvas_transform(state, cw, ch);
            }
        }

        unsafe {
            let _ = ShowWindow(self.hwnd, SW_SHOW);

            // SetForegroundWindow has strict restrictions on Windows. When it fails,
            // the window is visible (SW_SHOW + WS_EX_TOPMOST) but not activated.
            // Use the AttachThreadInput trick: temporarily attach to the foreground
            // thread so Windows allows us to take foreground.
            let fg_hwnd = GetForegroundWindow();
            let fg_thread = if !fg_hwnd.is_invalid() && fg_hwnd != self.hwnd {
                let tid = windows::Win32::System::Threading::GetCurrentThreadId();
                let fg_tid = GetWindowThreadProcessId(fg_hwnd, None);
                if fg_tid != 0 && fg_tid != tid {
                    let _ = windows::Win32::System::Threading::AttachThreadInput(tid, fg_tid, true);
                    Some((tid, fg_tid))
                } else {
                    None
                }
            } else {
                None
            };

            let _ = SetForegroundWindow(self.hwnd);

            if let Some((tid, fg_tid)) = fg_thread {
                let _ = windows::Win32::System::Threading::AttachThreadInput(tid, fg_tid, false);
            }

            let _ = InvalidateRect(Some(self.hwnd), None, false);
        }

        info!("Annotation editor shown: {}x{}", pixels.width, pixels.height);
        Ok(())
    }

    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }

    /// Reconstruct an AnnotationEditor from a raw HWND.
    /// Used when the lock must be dropped before calling show().
    pub fn from_hwnd(hwnd: HWND) -> Self {
        Self { hwnd }
    }

    /// Recompute canvas transform so the image fits the window at correct aspect ratio.
    fn recompute_canvas_transform(state: &mut EditorState, client_w: f32, client_h: f32) {
        let canvas_w = client_w;
        let canvas_h = client_h - TOOLBAR_HEIGHT;
        if state.img_width == 0 || state.img_height == 0 || canvas_w <= 0.0 || canvas_h <= 0.0 {
            state.canvas_scale = 1.0;
            state.canvas_offset_x = 0.0;
            state.canvas_offset_y = 0.0;
            return;
        }
        let scale_x = canvas_w / state.img_width as f32;
        let scale_y = canvas_h / state.img_height as f32;
        let scale = scale_x.min(scale_y);
        let drawn_w = state.img_width as f32 * scale;
        let drawn_h = state.img_height as f32 * scale;
        state.canvas_scale = scale;
        state.canvas_offset_x = (canvas_w - drawn_w) / 2.0;
        state.canvas_offset_y = (canvas_h - drawn_h) / 2.0;
    }

    // ─── Rendering ──────────────────────────────────────────────────────────

    fn render(state: &EditorState) {
        let Some(rt) = &state.render_target else {
            return;
        };

        unsafe {
            rt.BeginDraw();
            rt.Clear(Some(&D2D1_COLOR_F {
                r: 0.15,
                g: 0.15,
                b: 0.17,
                a: 1.0,
            }));

            // Set image-space transform: scale + translate so image is centered below toolbar
            let img_transform = windows_numerics::Matrix3x2 {
                M11: state.canvas_scale, M12: 0.0,
                M21: 0.0, M22: state.canvas_scale,
                M31: state.canvas_offset_x,
                M32: TOOLBAR_HEIGHT + state.canvas_offset_y,
            };
            rt.SetTransform(&img_transform);

            // Draw screenshot at native size (transform handles positioning)
            if let Some(bmp) = &state.bitmap {
                let dst = D2D_RECT_F {
                    left: 0.0,
                    top: 0.0,
                    right: state.img_width as f32,
                    bottom: state.img_height as f32,
                };
                rt.DrawBitmap(
                    bmp,
                    Some(&dst),
                    1.0,
                    D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
                    None,
                );
            }

            // Draw committed annotations (in image space, y_offset=0)
            for ann in &state.layer.annotations {
                Self::draw_annotation(rt, ann, 0.0, state.pixels.as_ref(), state.pen_stroke_style.as_ref(), state.dwrite_factory.as_ref());
            }

            // Draw live preview
            if state.drawing {
                if let Some(preview) = Self::build_current_annotation(state) {
                    Self::draw_annotation(rt, &preview, 0.0, state.pixels.as_ref(), state.pen_stroke_style.as_ref(), state.dwrite_factory.as_ref());
                }
            }

            // Draw text being edited (in image space)
            if state.text_editing && !state.text_buffer.is_empty() {
                if let Some(dwrite) = &state.dwrite_factory {
                    Self::draw_text_preview(
                        rt,
                        dwrite,
                        &state.text_buffer,
                        state.text_position.x,
                        state.text_position.y,
                        &state.stroke_color,
                        state.thickness * 4.0,
                    );
                }
            }

            // Draw text cursor (in image space)
            if state.text_editing {
                Self::draw_text_cursor(
                    rt,
                    state,
                    0.0,
                );
            }

            // Draw selection highlight around selected annotation
            if let Some(idx) = state.selected_index {
                if let Some(ann) = state.layer.annotations.get(idx) {
                    let (min_x, min_y, max_x, max_y) = ann.bounding_box();
                    let pad = 4.0;
                    let sel_rect = D2D_RECT_F {
                        left: min_x - pad,
                        top: min_y - pad,
                        right: max_x + pad,
                        bottom: max_y + pad,
                    };
                    if let Ok(brush) = rt.CreateSolidColorBrush(
                        &D2D1_COLOR_F { r: 0.2, g: 0.5, b: 1.0, a: 0.9 },
                        None,
                    ) {
                        // Create dashed stroke style
                        if let Some(factory) = &state.d2d_factory {
                            if let Ok(base_factory) = factory.cast::<ID2D1Factory>() {
                                let dash_props = D2D1_STROKE_STYLE_PROPERTIES {
                                    dashStyle: D2D1_DASH_STYLE_DASH,
                                    ..Default::default()
                                };
                                if let Ok(dash_style) = base_factory.CreateStrokeStyle(&dash_props, None) {
                                    rt.DrawRectangle(&sel_rect, &brush, 2.0 / state.canvas_scale, Some(&dash_style));
                                }
                            }
                        }
                    }
                }
            }

            // Reset transform for toolbar (drawn in screen space)
            let identity = windows_numerics::Matrix3x2 {
                M11: 1.0, M12: 0.0,
                M21: 0.0, M22: 1.0,
                M31: 0.0, M32: 0.0,
            };
            rt.SetTransform(&identity);

            // Draw toolbar
            Self::draw_toolbar(rt, state);

            let _ = rt.EndDraw(None, None);
        }
    }

    fn draw_annotation(
        rt: &ID2D1HwndRenderTarget,
        ann: &Annotation,
        y_offset: f32,
        pixels: Option<&PixelBuffer>,
        pen_stroke_style: Option<&ID2D1StrokeStyle>,
        dwrite_factory: Option<&IDWriteFactory>,
    ) {
        unsafe {
            match ann {
                Annotation::Arrow {
                    start,
                    end,
                    color,
                    thickness,
                } => {
                    if let Ok(brush) = rt.CreateSolidColorBrush(&to_d2d_color(color), None) {
                        let s = windows_numerics::Vector2 {
                            X: start.x,
                            Y: start.y + y_offset,
                        };
                        let e = windows_numerics::Vector2 {
                            X: end.x,
                            Y: end.y + y_offset,
                        };
                        rt.DrawLine(s, e, &brush, *thickness, None);

                        let dx = end.x - start.x;
                        let dy = end.y - start.y;
                        let len = (dx * dx + dy * dy).sqrt();
                        if len > 5.0 {
                            let nx = dx / len;
                            let ny = dy / len;
                            let head_len = (*thickness * 4.0).min(len * 0.3);
                            let head_w = head_len * 0.5;
                            let tip_x = end.x;
                            let tip_y = end.y + y_offset;
                            let l = windows_numerics::Vector2 {
                                X: tip_x - nx * head_len + ny * head_w,
                                Y: tip_y - ny * head_len - nx * head_w,
                            };
                            let r = windows_numerics::Vector2 {
                                X: tip_x - nx * head_len - ny * head_w,
                                Y: tip_y - ny * head_len + nx * head_w,
                            };
                            let t = windows_numerics::Vector2 {
                                X: tip_x,
                                Y: tip_y,
                            };
                            rt.DrawLine(t, l, &brush, *thickness * 0.7, None);
                            rt.DrawLine(t, r, &brush, *thickness * 0.7, None);
                        }
                    }
                }

                Annotation::Rectangle {
                    x,
                    y,
                    width,
                    height,
                    color,
                    thickness,
                } => {
                    if let Ok(brush) = rt.CreateSolidColorBrush(&to_d2d_color(color), None) {
                        let rect = D2D_RECT_F {
                            left: *x,
                            top: *y + y_offset,
                            right: *x + *width,
                            bottom: *y + *height + y_offset,
                        };
                        rt.DrawRectangle(&rect, &brush, *thickness, None);
                    }
                }

                Annotation::Highlight {
                    x,
                    y,
                    width,
                    height,
                    color,
                } => {
                    if let Ok(brush) = rt.CreateSolidColorBrush(&to_d2d_color(color), None) {
                        let rect = D2D_RECT_F {
                            left: *x,
                            top: *y + y_offset,
                            right: *x + *width,
                            bottom: *y + *height + y_offset,
                        };
                        rt.FillRectangle(&rect, &brush);
                    }
                }

                Annotation::Pen {
                    points,
                    color,
                    thickness,
                } => {
                    if points.len() >= 2 {
                        if let Ok(brush) = rt.CreateSolidColorBrush(&to_d2d_color(color), None) {
                            for pair in points.windows(2) {
                                let a = windows_numerics::Vector2 {
                                    X: pair[0].x,
                                    Y: pair[0].y + y_offset,
                                };
                                let b = windows_numerics::Vector2 {
                                    X: pair[1].x,
                                    Y: pair[1].y + y_offset,
                                };
                                rt.DrawLine(a, b, &brush, *thickness, pen_stroke_style);
                            }
                        }
                    }
                }

                Annotation::Blur {
                    x,
                    y,
                    width,
                    height,
                    radius,
                } => {
                    // Real pixelation: sample actual pixels and draw averaged blocks
                    let block_size = (*radius).max(4.0);
                    let bx = *x;
                    let by = *y + y_offset;

                    if let Some(px) = pixels {
                        let mut cy = 0.0f32;
                        while cy < *height {
                            let mut cx = 0.0f32;
                            while cx < *width {
                                let bw = block_size.min(*width - cx);
                                let bh = block_size.min(*height - cy);

                                // Sample average color from pixel buffer
                                let avg = sample_block_average(
                                    px,
                                    (*x + cx) as i32,
                                    (*y + cy) as i32,
                                    bw as u32,
                                    bh as u32,
                                );

                                if let Ok(brush) = rt.CreateSolidColorBrush(&avg, None) {
                                    let rect = D2D_RECT_F {
                                        left: bx + cx,
                                        top: by + cy,
                                        right: (bx + cx + bw).min(bx + *width),
                                        bottom: (by + cy + bh).min(by + *height),
                                    };
                                    rt.FillRectangle(&rect, &brush);
                                }
                                cx += block_size;
                            }
                            cy += block_size;
                        }
                    } else {
                        // Fallback: gray blocks
                        if let Ok(brush) = rt.CreateSolidColorBrush(
                            &D2D1_COLOR_F { r: 0.5, g: 0.5, b: 0.5, a: 0.9 },
                            None,
                        ) {
                            let rect = D2D_RECT_F {
                                left: bx,
                                top: by,
                                right: bx + *width,
                                bottom: by + *height,
                            };
                            rt.FillRectangle(&rect, &brush);
                        }
                    }
                }

                Annotation::Text {
                    position,
                    text,
                    color,
                    font_size,
                } => {
                    if let Ok(brush) = rt.CreateSolidColorBrush(&to_d2d_color(color), None) {
                        let wide: Vec<u16> = text.encode_utf16().collect();
                        if !wide.is_empty() {
                            let rect = D2D_RECT_F {
                                left: position.x,
                                top: position.y + y_offset,
                                right: position.x + wide.len() as f32 * font_size * 0.6,
                                bottom: position.y + y_offset + font_size * 1.3,
                            };
                            // Use passed-in DWrite factory (cached in EditorState)
                            if let Some(dwrite) = dwrite_factory {
                                if let Ok(fmt) = dwrite.CreateTextFormat(
                                    w!("Segoe UI"), None,
                                    DWRITE_FONT_WEIGHT_BOLD, DWRITE_FONT_STYLE_NORMAL,
                                    DWRITE_FONT_STRETCH_NORMAL, *font_size, w!("en-US"),
                                ) {
                                    rt.DrawText(
                                        &wide, &fmt, &rect, &brush,
                                        D2D1_DRAW_TEXT_OPTIONS_NONE,
                                        DWRITE_MEASURING_MODE_NATURAL,
                                    );
                                }
                            }
                        }
                    }
                }

                Annotation::Emoji {
                    position,
                    emoji,
                    size,
                } => {
                    if let Some(dwrite) = dwrite_factory {
                        if let Ok(brush) = rt.CreateSolidColorBrush(
                            &D2D1_COLOR_F { r: 1.0, g: 1.0, b: 1.0, a: 1.0 }, None,
                        ) {
                            let wide: Vec<u16> = emoji.encode_utf16().collect();
                            if let Ok(fmt) = dwrite.CreateTextFormat(
                                w!("Segoe UI Emoji"), None,
                                DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_STYLE_NORMAL,
                                DWRITE_FONT_STRETCH_NORMAL, *size, w!("en-US"),
                            ) {
                                let rect = D2D_RECT_F {
                                    left: position.x,
                                    top: position.y + y_offset,
                                    right: position.x + *size * 1.5,
                                    bottom: position.y + y_offset + *size * 1.5,
                                };
                                rt.DrawText(
                                    &wide, &fmt, &rect, &brush,
                                    D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT,
                                    DWRITE_MEASURING_MODE_NATURAL,
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    fn draw_text_preview(
        rt: &ID2D1HwndRenderTarget,
        dwrite: &IDWriteFactory,
        text: &str,
        x: f32,
        y: f32,
        color: &Color,
        font_size: f32,
    ) {
        unsafe {
            let wide: Vec<u16> = text.encode_utf16().collect();
            if wide.is_empty() {
                return;
            }
            if let Ok(brush) = rt.CreateSolidColorBrush(&to_d2d_color(color), None) {
                if let Ok(fmt) = dwrite.CreateTextFormat(
                    w!("Segoe UI"),
                    None,
                    DWRITE_FONT_WEIGHT_BOLD,
                    DWRITE_FONT_STYLE_NORMAL,
                    DWRITE_FONT_STRETCH_NORMAL,
                    font_size,
                    w!("en-US"),
                ) {
                    let rect = D2D_RECT_F {
                        left: x,
                        top: y,
                        right: x + 2000.0,
                        bottom: y + font_size * 1.5,
                    };
                    rt.DrawText(
                        &wide,
                        &fmt,
                        &rect,
                        &brush,
                        D2D1_DRAW_TEXT_OPTIONS_NONE,
                        DWRITE_MEASURING_MODE_NATURAL,
                    );
                }
            }
        }
    }

    fn draw_text_cursor(
        rt: &ID2D1HwndRenderTarget,
        state: &EditorState,
        y_offset: f32,
    ) {
        unsafe {
            let font_size = state.thickness * 4.0;
            // Estimate cursor x position based on text width
            let cursor_x = if state.text_buffer.is_empty() {
                state.text_position.x
            } else {
                // Rough estimate: ~0.55 * font_size per character
                state.text_position.x + state.text_buffer.len() as f32 * font_size * 0.55
            };
            let cursor_y = state.text_position.y + y_offset;

            if let Ok(brush) = rt.CreateSolidColorBrush(&to_d2d_color(&state.stroke_color), None) {
                let top = windows_numerics::Vector2 {
                    X: cursor_x,
                    Y: cursor_y,
                };
                let bottom = windows_numerics::Vector2 {
                    X: cursor_x,
                    Y: cursor_y + font_size * 1.2,
                };
                rt.DrawLine(top, bottom, &brush, 2.0, None);
            }
        }
    }

    // ─── Toolbar rendering ──────────────────────────────────────────────────

    fn draw_toolbar(rt: &ID2D1HwndRenderTarget, state: &EditorState) {
        let Some(cb) = &state.cached_brushes else {
            return; // No cached brushes — can't draw toolbar
        };

        unsafe {
            let win_size = rt.GetSize();

            // Toolbar background
            rt.FillRectangle(
                &D2D_RECT_F { left: 0.0, top: 0.0, right: win_size.width, bottom: TOOLBAR_HEIGHT },
                &cb.toolbar_bg,
            );

            // Divider line
            rt.DrawLine(
                windows_numerics::Vector2 { X: 0.0, Y: TOOLBAR_HEIGHT - 0.5 },
                windows_numerics::Vector2 { X: win_size.width, Y: TOOLBAR_HEIGHT - 0.5 },
                &cb.divider, 1.0, None,
            );

            // ── Row 1: Tool buttons + Action buttons ──

            let tools = [
                AnnotationTool::Arrow, AnnotationTool::Rectangle, AnnotationTool::Highlight,
                AnnotationTool::Pen, AnnotationTool::Blur, AnnotationTool::Text,
                AnnotationTool::Emoji,
            ];

            for (i, tool) in tools.iter().enumerate() {
                let x = MARGIN_X + i as f32 * ROW1_BTN_STEP;
                let is_active = state.tool == *tool;
                let is_hovered = state.hovered_btn == i as i32;

                Self::draw_button_bg_cached(rt, x, ROW1_TOP, ROW1_BTN_SIZE, is_active, is_hovered, &cb.active_bg, &cb.hover_bg);

                let brush = if is_active { &cb.icon_white } else { &cb.icon_normal };
                let cx = x + ROW1_BTN_SIZE / 2.0;
                let cy = ROW1_TOP + ROW1_BTN_SIZE / 2.0;
                Self::draw_tool_icon(rt, *tool, cx, cy, brush);
            }

            // Separator after tools
            let sep1_x = MARGIN_X + tools.len() as f32 * ROW1_BTN_STEP + 2.0;
            rt.DrawLine(
                windows_numerics::Vector2 { X: sep1_x, Y: ROW1_TOP + 6.0 },
                windows_numerics::Vector2 { X: sep1_x, Y: ROW1_TOP + ROW1_BTN_SIZE - 6.0 },
                &cb.separator, 1.0, None,
            );

            // Undo / Redo
            let action_x = sep1_x + SEP_W;
            let undo_disabled = state.layer.is_empty();
            let redo_disabled = state.redo_stack.is_empty();

            {
                let x = action_x;
                let is_hovered = state.hovered_btn == tools.len() as i32;
                Self::draw_button_bg_cached(rt, x, ROW1_TOP, ROW1_BTN_SIZE, false, is_hovered && !undo_disabled, &cb.active_bg, &cb.hover_bg);
                let brush = if undo_disabled { &cb.disabled } else { &cb.icon_normal };
                if let Some(factory) = &state.d2d_factory {
                    Self::draw_undo_icon(rt, factory, x + ROW1_BTN_SIZE / 2.0, ROW1_TOP + ROW1_BTN_SIZE / 2.0, brush);
                }
            }
            {
                let x = action_x + ROW1_BTN_STEP;
                let is_hovered = state.hovered_btn == tools.len() as i32 + 1;
                Self::draw_button_bg_cached(rt, x, ROW1_TOP, ROW1_BTN_SIZE, false, is_hovered && !redo_disabled, &cb.active_bg, &cb.hover_bg);
                let brush = if redo_disabled { &cb.disabled } else { &cb.icon_normal };
                if let Some(factory) = &state.d2d_factory {
                    Self::draw_redo_icon(rt, factory, x + ROW1_BTN_SIZE / 2.0, ROW1_TOP + ROW1_BTN_SIZE / 2.0, brush);
                }
            }

            // Separator before Done/Cancel
            let sep2_x = action_x + 2.0 * ROW1_BTN_STEP + 2.0;
            rt.DrawLine(
                windows_numerics::Vector2 { X: sep2_x, Y: ROW1_TOP + 6.0 },
                windows_numerics::Vector2 { X: sep2_x, Y: ROW1_TOP + ROW1_BTN_SIZE - 6.0 },
                &cb.separator, 1.0, None,
            );

            // Done
            let done_x = sep2_x + SEP_W;
            {
                let is_hovered = state.hovered_btn == tools.len() as i32 + 2;
                Self::draw_button_bg_cached(rt, done_x, ROW1_TOP, ROW1_BTN_SIZE, false, is_hovered, &cb.done_bg, &cb.hover_bg);
                let brush = if is_hovered { &cb.done_icon_hover } else { &cb.done_icon };
                Self::draw_checkmark_icon(rt, done_x + ROW1_BTN_SIZE / 2.0, ROW1_TOP + ROW1_BTN_SIZE / 2.0, brush);
            }
            // Cancel
            {
                let x = done_x + ROW1_BTN_STEP;
                let is_hovered = state.hovered_btn == tools.len() as i32 + 3;
                Self::draw_button_bg_cached(rt, x, ROW1_TOP, ROW1_BTN_SIZE, false, is_hovered, &cb.cancel_bg, &cb.hover_bg);
                let brush = if is_hovered { &cb.cancel_icon_hover } else { &cb.cancel_icon };
                Self::draw_x_icon(rt, x + ROW1_BTN_SIZE / 2.0, ROW1_TOP + ROW1_BTN_SIZE / 2.0, brush);
            }

            // ── Row 2: Color swatches + Thickness presets ──

            // Shortcut hints for tool buttons
            let shortcuts = ["A", "R", "H", "P", "B", "T", "E"];
            for (i, key) in shortcuts.iter().enumerate() {
                let x = MARGIN_X + i as f32 * ROW1_BTN_STEP;
                let wide: Vec<u16> = key.encode_utf16().collect();
                let rect = D2D_RECT_F {
                    left: x + 2.0, top: ROW1_TOP + 2.0,
                    right: x + ROW1_BTN_SIZE - 2.0, bottom: ROW1_TOP + ROW1_BTN_SIZE - 1.0,
                };
                rt.DrawText(&wide, &cb.hint_format, &rect, &cb.hint_text, D2D1_DRAW_TEXT_OPTIONS_NONE, DWRITE_MEASURING_MODE_NATURAL);
            }

            // Shortcut hints for action buttons (Undo, Redo, Done, Cancel)
            let action_hints = [("Ctrl+Z", action_x), ("Ctrl+Y", action_x + ROW1_BTN_STEP),
                                ("Enter", done_x), ("Esc", done_x + ROW1_BTN_STEP)];
            for (key, x) in action_hints {
                let wide: Vec<u16> = key.encode_utf16().collect();
                let rect = D2D_RECT_F {
                    left: x + 2.0, top: ROW1_TOP + 2.0,
                    right: x + ROW1_BTN_SIZE - 2.0, bottom: ROW1_TOP + ROW1_BTN_SIZE - 1.0,
                };
                rt.DrawText(&wide, &cb.hint_format, &rect, &cb.hint_text, D2D1_DRAW_TEXT_OPTIONS_NONE, DWRITE_MEASURING_MODE_NATURAL);
            }

            if state.tool == AnnotationTool::Emoji {
                // ── Row 2: Emoji picker + Thickness presets ──
                for (i, emoji_str) in EMOJI_PALETTE.iter().enumerate() {
                    let x = MARGIN_X + i as f32 * SWATCH_STEP;
                    let y = ROW2_TOP + (26.0 - SWATCH_SIZE) / 2.0;

                    // Selection ring or outline
                    if i == state.emoji_index {
                        let rect = D2D_RECT_F {
                            left: x, top: y, right: x + SWATCH_SIZE, bottom: y + SWATCH_SIZE,
                        };
                        rt.DrawRoundedRectangle(
                            &D2D1_ROUNDED_RECT { rect, radiusX: 4.0, radiusY: 4.0 },
                            &cb.selection_ring, 2.0, None,
                        );
                    } else {
                        let rect = D2D_RECT_F {
                            left: x + 0.5, top: y + 0.5,
                            right: x + SWATCH_SIZE - 0.5, bottom: y + SWATCH_SIZE - 0.5,
                        };
                        rt.DrawRoundedRectangle(
                            &D2D1_ROUNDED_RECT { rect, radiusX: 3.5, radiusY: 3.5 },
                            &cb.outline, 0.5, None,
                        );
                    }

                    // Draw emoji text using DirectWrite
                    if let Some(dwrite) = &state.dwrite_factory {
                        if let Ok(fmt) = dwrite.CreateTextFormat(
                            w!("Segoe UI Emoji"), None,
                            DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_STYLE_NORMAL,
                            DWRITE_FONT_STRETCH_NORMAL, 14.0, w!("en-US"),
                        ) {
                            let _ = fmt.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER);
                            let _ = fmt.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER);
                            let wide: Vec<u16> = emoji_str.encode_utf16().collect();
                            let text_rect = D2D_RECT_F {
                                left: x, top: y,
                                right: x + SWATCH_SIZE, bottom: y + SWATCH_SIZE,
                            };
                            rt.DrawText(
                                &wide, &fmt, &text_rect, &cb.icon_white,
                                D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT,
                                DWRITE_MEASURING_MODE_NATURAL,
                            );
                        }
                    }
                }

                // Separator between emojis and thickness
                let emoji_end_x = MARGIN_X + EMOJI_PALETTE.len() as f32 * SWATCH_STEP + 2.0;
                rt.DrawLine(
                    windows_numerics::Vector2 { X: emoji_end_x, Y: ROW2_TOP + 3.0 },
                    windows_numerics::Vector2 { X: emoji_end_x, Y: ROW2_TOP + 23.0 },
                    &cb.separator, 1.0, None,
                );

                // Thickness presets
                let thick_start_x = emoji_end_x + SEP_W;
                for (i, &t) in THICKNESS_PRESETS.iter().enumerate() {
                    let x = thick_start_x + i as f32 * SWATCH_STEP;
                    let y = ROW2_TOP + (26.0 - SWATCH_SIZE) / 2.0;

                    if state.thickness_index == i {
                        let center = windows_numerics::Vector2 {
                            X: x + SWATCH_SIZE / 2.0, Y: y + SWATCH_SIZE / 2.0,
                        };
                        rt.DrawEllipse(
                            &D2D1_ELLIPSE { point: center, radiusX: SWATCH_SIZE / 2.0, radiusY: SWATCH_SIZE / 2.0 },
                            &cb.selection_ring, 2.0, None,
                        );
                    }

                    let dot_radius = (t / 2.0).clamp(1.5, 8.0);
                    let center = windows_numerics::Vector2 {
                        X: x + SWATCH_SIZE / 2.0, Y: y + SWATCH_SIZE / 2.0,
                    };
                    rt.FillEllipse(
                        &D2D1_ELLIPSE { point: center, radiusX: dot_radius, radiusY: dot_radius },
                        &cb.icon_normal,
                    );
                }
            } else {
                // ── Row 2: Color swatches + Thickness presets ──
                for (i, _color) in Color::PALETTE.iter().enumerate() {
                    let x = MARGIN_X + i as f32 * SWATCH_STEP;
                    let y = ROW2_TOP + (26.0 - SWATCH_SIZE) / 2.0;

                    let rect = D2D_RECT_F {
                        left: x + 1.0, top: y + 1.0,
                        right: x + SWATCH_SIZE - 1.0, bottom: y + SWATCH_SIZE - 1.0,
                    };
                    rt.FillRoundedRectangle(
                        &D2D1_ROUNDED_RECT { rect, radiusX: 3.0, radiusY: 3.0 },
                        &cb.palette[i],
                    );

                    if i == state.color_index {
                        let rect = D2D_RECT_F {
                            left: x, top: y, right: x + SWATCH_SIZE, bottom: y + SWATCH_SIZE,
                        };
                        rt.DrawRoundedRectangle(
                            &D2D1_ROUNDED_RECT { rect, radiusX: 4.0, radiusY: 4.0 },
                            &cb.selection_ring, 2.0, None,
                        );
                    } else {
                        let rect = D2D_RECT_F {
                            left: x + 0.5, top: y + 0.5,
                            right: x + SWATCH_SIZE - 0.5, bottom: y + SWATCH_SIZE - 0.5,
                        };
                        rt.DrawRoundedRectangle(
                            &D2D1_ROUNDED_RECT { rect, radiusX: 3.5, radiusY: 3.5 },
                            &cb.outline, 0.5, None,
                        );
                    }
                }

                // Separator between colors and thickness
                let color_end_x = MARGIN_X + Color::PALETTE.len() as f32 * SWATCH_STEP + 2.0;
                rt.DrawLine(
                    windows_numerics::Vector2 { X: color_end_x, Y: ROW2_TOP + 3.0 },
                    windows_numerics::Vector2 { X: color_end_x, Y: ROW2_TOP + 23.0 },
                    &cb.separator, 1.0, None,
                );

                // Thickness presets
                let thick_start_x = color_end_x + SEP_W;
                for (i, &t) in THICKNESS_PRESETS.iter().enumerate() {
                    let x = thick_start_x + i as f32 * SWATCH_STEP;
                    let y = ROW2_TOP + (26.0 - SWATCH_SIZE) / 2.0;

                    if state.thickness_index == i {
                        let center = windows_numerics::Vector2 {
                            X: x + SWATCH_SIZE / 2.0, Y: y + SWATCH_SIZE / 2.0,
                        };
                        rt.DrawEllipse(
                            &D2D1_ELLIPSE { point: center, radiusX: SWATCH_SIZE / 2.0, radiusY: SWATCH_SIZE / 2.0 },
                            &cb.selection_ring, 2.0, None,
                        );
                    }

                    let dot_radius = (t / 2.0).clamp(1.5, 8.0);
                    let center = windows_numerics::Vector2 {
                        X: x + SWATCH_SIZE / 2.0, Y: y + SWATCH_SIZE / 2.0,
                    };
                    rt.FillEllipse(
                        &D2D1_ELLIPSE { point: center, radiusX: dot_radius, radiusY: dot_radius },
                        &cb.icon_normal,
                    );
                }
            }
        }
    }

    fn draw_button_bg_cached(
        rt: &ID2D1HwndRenderTarget,
        x: f32,
        y: f32,
        size: f32,
        is_active: bool,
        is_hovered: bool,
        active_brush: &ID2D1SolidColorBrush,
        hover_brush: &ID2D1SolidColorBrush,
    ) {
        if !is_active && !is_hovered {
            return;
        }
        let brush = if is_active { active_brush } else { hover_brush };
        unsafe {
            let rect = D2D_RECT_F { left: x, top: y, right: x + size, bottom: y + size };
            let rounded = D2D1_ROUNDED_RECT { rect, radiusX: 8.0, radiusY: 8.0 };
            rt.FillRoundedRectangle(&rounded, brush);
        }
    }

    // ─── Icon drawing ───────────────────────────────────────────────────────

    fn draw_tool_icon(
        rt: &ID2D1HwndRenderTarget,
        tool: AnnotationTool,
        cx: f32,
        cy: f32,
        brush: &ID2D1SolidColorBrush,
    ) {
        unsafe {
            match tool {
                AnnotationTool::Arrow => {
                    let s = windows_numerics::Vector2 { X: cx - 10.5, Y: cy + 10.5 };
                    let e = windows_numerics::Vector2 { X: cx + 10.5, Y: cy - 10.5 };
                    rt.DrawLine(s, e, brush, 3.0, None);
                    let t = windows_numerics::Vector2 { X: cx + 10.5, Y: cy - 10.5 };
                    let l = windows_numerics::Vector2 { X: cx + 1.5, Y: cy - 9.0 };
                    let r = windows_numerics::Vector2 { X: cx + 9.0, Y: cy - 1.5 };
                    rt.DrawLine(t, l, brush, 3.0, None);
                    rt.DrawLine(t, r, brush, 3.0, None);
                }
                AnnotationTool::Rectangle => {
                    let rect = D2D_RECT_F {
                        left: cx - 12.0,
                        top: cy - 9.0,
                        right: cx + 12.0,
                        bottom: cy + 9.0,
                    };
                    rt.DrawRectangle(&rect, brush, 3.0, None);
                }
                AnnotationTool::Highlight => {
                    let rect = D2D_RECT_F {
                        left: cx - 12.0,
                        top: cy - 7.5,
                        right: cx + 12.0,
                        bottom: cy + 7.5,
                    };
                    rt.FillRectangle(&rect, brush);
                    // Use a temporary brush for the lines inside highlight icon
                    // This is only called once per frame per tool button, so the cost is negligible
                    if let Ok(line_brush) = rt.CreateSolidColorBrush(
                        &D2D1_COLOR_F { r: 0.15, g: 0.15, b: 0.17, a: 0.6 },
                        None,
                    ) {
                        rt.DrawLine(
                            windows_numerics::Vector2 { X: cx - 7.5, Y: cy - 2.25 },
                            windows_numerics::Vector2 { X: cx + 7.5, Y: cy - 2.25 },
                            &line_brush, 2.25, None,
                        );
                        rt.DrawLine(
                            windows_numerics::Vector2 { X: cx - 7.5, Y: cy + 3.0 },
                            windows_numerics::Vector2 { X: cx + 4.5, Y: cy + 3.0 },
                            &line_brush, 2.25, None,
                        );
                    }
                }
                AnnotationTool::Pen => {
                    let pts: [(f32, f32); 5] = [
                        (cx - 12.0, cy + 4.5),
                        (cx - 6.0, cy - 7.5),
                        (cx, cy + 4.5),
                        (cx + 6.0, cy - 7.5),
                        (cx + 12.0, cy + 4.5),
                    ];
                    for pair in pts.windows(2) {
                        rt.DrawLine(
                            windows_numerics::Vector2 { X: pair[0].0, Y: pair[0].1 },
                            windows_numerics::Vector2 { X: pair[1].0, Y: pair[1].1 },
                            brush,
                            3.0,
                            None,
                        );
                    }
                }
                AnnotationTool::Blur => {
                    let block = 9.0f32;
                    let gap = 3.0f32;
                    let start_x = cx - (block + gap / 2.0);
                    let start_y = cy - (block + gap / 2.0);
                    for row in 0..2 {
                        for col in 0..2 {
                            let rect = D2D_RECT_F {
                                left: start_x + col as f32 * (block + gap),
                                top: start_y + row as f32 * (block + gap),
                                right: start_x + col as f32 * (block + gap) + block,
                                bottom: start_y + row as f32 * (block + gap) + block,
                            };
                            rt.FillRectangle(&rect, brush);
                        }
                    }
                }
                AnnotationTool::Text => {
                    rt.DrawLine(
                        windows_numerics::Vector2 { X: cx - 10.5, Y: cy - 10.5 },
                        windows_numerics::Vector2 { X: cx + 10.5, Y: cy - 10.5 },
                        brush,
                        3.75,
                        None,
                    );
                    rt.DrawLine(
                        windows_numerics::Vector2 { X: cx, Y: cy - 10.5 },
                        windows_numerics::Vector2 { X: cx, Y: cy + 12.0 },
                        brush,
                        3.75,
                        None,
                    );
                    rt.DrawLine(
                        windows_numerics::Vector2 { X: cx - 4.5, Y: cy + 12.0 },
                        windows_numerics::Vector2 { X: cx + 4.5, Y: cy + 12.0 },
                        brush,
                        3.0,
                        None,
                    );
                }
                AnnotationTool::Emoji => {
                    // Smiley face icon
                    let r = 11.0f32;
                    let center = windows_numerics::Vector2 { X: cx, Y: cy };
                    rt.DrawEllipse(
                        &D2D1_ELLIPSE { point: center, radiusX: r, radiusY: r },
                        brush, 2.0, None,
                    );
                    // Eyes
                    let eye_y = cy - 3.0;
                    let left_eye = windows_numerics::Vector2 { X: cx - 4.0, Y: eye_y };
                    let right_eye = windows_numerics::Vector2 { X: cx + 4.0, Y: eye_y };
                    rt.FillEllipse(&D2D1_ELLIPSE { point: left_eye, radiusX: 1.5, radiusY: 1.5 }, brush);
                    rt.FillEllipse(&D2D1_ELLIPSE { point: right_eye, radiusX: 1.5, radiusY: 1.5 }, brush);
                    // Smile (arc approximated with a line)
                    rt.DrawLine(
                        windows_numerics::Vector2 { X: cx - 5.0, Y: cy + 3.5 },
                        windows_numerics::Vector2 { X: cx - 2.0, Y: cy + 5.5 },
                        brush, 1.5, None,
                    );
                    rt.DrawLine(
                        windows_numerics::Vector2 { X: cx - 2.0, Y: cy + 5.5 },
                        windows_numerics::Vector2 { X: cx + 2.0, Y: cy + 5.5 },
                        brush, 1.5, None,
                    );
                    rt.DrawLine(
                        windows_numerics::Vector2 { X: cx + 2.0, Y: cy + 5.5 },
                        windows_numerics::Vector2 { X: cx + 5.0, Y: cy + 3.5 },
                        brush, 1.5, None,
                    );
                }
            }
        }
    }

    /// Undo icon: smooth counterclockwise curved arrow using bezier path geometry.
    fn draw_undo_icon(rt: &ID2D1HwndRenderTarget, factory: &ID2D1Factory1, cx: f32, cy: f32, brush: &ID2D1SolidColorBrush) {
        unsafe {
            // Smooth arc: semicircle from right to left, opening downward
            // Uses two quarter-circle cubic beziers (k ≈ 0.552 for circle approx)
            let r: f32 = 8.0;
            let k: f32 = 0.552;
            let ay = cy + 2.0; // arc center Y (shifted down so arrow sits nicely)

            if let Ok(geom) = factory.CreatePathGeometry() {
                if let Ok(sink) = geom.Open() {
                    // Start at right side of arc
                    sink.BeginFigure(
                        windows_numerics::Vector2 { X: cx + r, Y: ay },
                        D2D1_FIGURE_BEGIN_HOLLOW,
                    );
                    // Quarter 1: right → top
                    sink.AddBezier(&D2D1_BEZIER_SEGMENT {
                        point1: windows_numerics::Vector2 { X: cx + r, Y: ay - r * k },
                        point2: windows_numerics::Vector2 { X: cx + r * k, Y: ay - r },
                        point3: windows_numerics::Vector2 { X: cx, Y: ay - r },
                    });
                    // Quarter 2: top → left
                    sink.AddBezier(&D2D1_BEZIER_SEGMENT {
                        point1: windows_numerics::Vector2 { X: cx - r * k, Y: ay - r },
                        point2: windows_numerics::Vector2 { X: cx - r, Y: ay - r * k },
                        point3: windows_numerics::Vector2 { X: cx - r, Y: ay },
                    });
                    sink.EndFigure(D2D1_FIGURE_END_OPEN);
                    let _ = sink.Close();

                    // Round-cap stroke style for smooth arc ends
                    let props = D2D1_STROKE_STYLE_PROPERTIES {
                        startCap: D2D1_CAP_STYLE_ROUND,
                        endCap: D2D1_CAP_STYLE_ROUND,
                        lineJoin: D2D1_LINE_JOIN_ROUND,
                        ..Default::default()
                    };
                    if let Ok(base_factory) = factory.cast::<ID2D1Factory>() {
                        if let Ok(style) = base_factory.CreateStrokeStyle(&props, None) {
                            rt.DrawGeometry(&geom, brush, 2.5, Some(&style));

                            // Arrowhead at left end — tangent points straight down
                            let tip = windows_numerics::Vector2 { X: cx - r, Y: ay };
                            let wing_l = windows_numerics::Vector2 { X: cx - r - 4.0, Y: ay - 5.0 };
                            let wing_r = windows_numerics::Vector2 { X: cx - r + 4.0, Y: ay - 5.0 };
                            rt.DrawLine(tip, wing_l, brush, 2.5, Some(&style));
                            rt.DrawLine(tip, wing_r, brush, 2.5, Some(&style));
                        }
                    }
                }
            }
        }
    }

    /// Redo icon: smooth clockwise curved arrow (mirror of undo).
    fn draw_redo_icon(rt: &ID2D1HwndRenderTarget, factory: &ID2D1Factory1, cx: f32, cy: f32, brush: &ID2D1SolidColorBrush) {
        unsafe {
            let r: f32 = 8.0;
            let k: f32 = 0.552;
            let ay = cy + 2.0;

            if let Ok(geom) = factory.CreatePathGeometry() {
                if let Ok(sink) = geom.Open() {
                    // Start at left side of arc
                    sink.BeginFigure(
                        windows_numerics::Vector2 { X: cx - r, Y: ay },
                        D2D1_FIGURE_BEGIN_HOLLOW,
                    );
                    // Quarter 1: left → top
                    sink.AddBezier(&D2D1_BEZIER_SEGMENT {
                        point1: windows_numerics::Vector2 { X: cx - r, Y: ay - r * k },
                        point2: windows_numerics::Vector2 { X: cx - r * k, Y: ay - r },
                        point3: windows_numerics::Vector2 { X: cx, Y: ay - r },
                    });
                    // Quarter 2: top → right
                    sink.AddBezier(&D2D1_BEZIER_SEGMENT {
                        point1: windows_numerics::Vector2 { X: cx + r * k, Y: ay - r },
                        point2: windows_numerics::Vector2 { X: cx + r, Y: ay - r * k },
                        point3: windows_numerics::Vector2 { X: cx + r, Y: ay },
                    });
                    sink.EndFigure(D2D1_FIGURE_END_OPEN);
                    let _ = sink.Close();

                    let props = D2D1_STROKE_STYLE_PROPERTIES {
                        startCap: D2D1_CAP_STYLE_ROUND,
                        endCap: D2D1_CAP_STYLE_ROUND,
                        lineJoin: D2D1_LINE_JOIN_ROUND,
                        ..Default::default()
                    };
                    if let Ok(base_factory) = factory.cast::<ID2D1Factory>() {
                        if let Ok(style) = base_factory.CreateStrokeStyle(&props, None) {
                            rt.DrawGeometry(&geom, brush, 2.5, Some(&style));

                            // Arrowhead at right end — tangent points straight down
                            let tip = windows_numerics::Vector2 { X: cx + r, Y: ay };
                            let wing_l = windows_numerics::Vector2 { X: cx + r - 4.0, Y: ay - 5.0 };
                            let wing_r = windows_numerics::Vector2 { X: cx + r + 4.0, Y: ay - 5.0 };
                            rt.DrawLine(tip, wing_l, brush, 2.5, Some(&style));
                            rt.DrawLine(tip, wing_r, brush, 2.5, Some(&style));
                        }
                    }
                }
            }
        }
    }

    fn draw_checkmark_icon(rt: &ID2D1HwndRenderTarget, cx: f32, cy: f32, brush: &ID2D1SolidColorBrush) {
        unsafe {
            rt.DrawLine(
                windows_numerics::Vector2 { X: cx - 9.0, Y: cy },
                windows_numerics::Vector2 { X: cx - 3.0, Y: cy + 7.5 },
                brush, 3.75, None,
            );
            rt.DrawLine(
                windows_numerics::Vector2 { X: cx - 3.0, Y: cy + 7.5 },
                windows_numerics::Vector2 { X: cx + 10.5, Y: cy - 7.5 },
                brush, 3.75, None,
            );
        }
    }

    fn draw_x_icon(rt: &ID2D1HwndRenderTarget, cx: f32, cy: f32, brush: &ID2D1SolidColorBrush) {
        unsafe {
            rt.DrawLine(
                windows_numerics::Vector2 { X: cx - 7.5, Y: cy - 7.5 },
                windows_numerics::Vector2 { X: cx + 7.5, Y: cy + 7.5 },
                brush, 3.75, None,
            );
            rt.DrawLine(
                windows_numerics::Vector2 { X: cx + 7.5, Y: cy - 7.5 },
                windows_numerics::Vector2 { X: cx - 7.5, Y: cy + 7.5 },
                brush, 3.75, None,
            );
        }
    }

    // ─── Annotation construction ────────────────────────────────────────────

    fn build_current_annotation(state: &EditorState) -> Option<Annotation> {
        let s = state.draw_start;
        let e = state.draw_current;

        match state.tool {
            AnnotationTool::Arrow => Some(Annotation::Arrow {
                start: s,
                end: e,
                color: state.stroke_color,
                thickness: state.thickness,
            }),
            AnnotationTool::Rectangle => {
                let x = s.x.min(e.x);
                let y = s.y.min(e.y);
                let w = (s.x - e.x).abs();
                let h = (s.y - e.y).abs();
                Some(Annotation::Rectangle {
                    x,
                    y,
                    width: w,
                    height: h,
                    color: state.stroke_color,
                    thickness: state.thickness,
                })
            }
            AnnotationTool::Highlight => {
                let x = s.x.min(e.x);
                let y = s.y.min(e.y);
                let w = (s.x - e.x).abs();
                let h = (s.y - e.y).abs();
                Some(Annotation::Highlight {
                    x,
                    y,
                    width: w,
                    height: h,
                    color: state.highlight_color,
                })
            }
            AnnotationTool::Pen => {
                if state.pen_points.len() >= 2 {
                    Some(Annotation::Pen {
                        points: state.pen_points.clone(),
                        color: state.stroke_color,
                        thickness: state.thickness,
                    })
                } else {
                    None
                }
            }
            AnnotationTool::Blur => {
                let x = s.x.min(e.x);
                let y = s.y.min(e.y);
                let w = (s.x - e.x).abs();
                let h = (s.y - e.y).abs();
                if w > 1.0 && h > 1.0 {
                    Some(Annotation::Blur {
                        x,
                        y,
                        width: w,
                        height: h,
                        radius: 8.0,
                    })
                } else {
                    None
                }
            }
            AnnotationTool::Text => None, // Text is handled separately via text_editing
            AnnotationTool::Emoji => {
                let dx = (e.x - s.x).abs();
                let dy = (e.y - s.y).abs();
                let span = dx.max(dy).max(state.thickness * 4.0);
                // Render rect is size*1.5, so solve for size
                let size = span / 1.5;
                let left = s.x.min(e.x);
                let top = s.y.min(e.y);
                Some(Annotation::Emoji {
                    position: Point { x: left, y: top },
                    emoji: EMOJI_PALETTE[state.emoji_index].to_string(),
                    size,
                })
            }
        }
    }

    // ─── Hit testing ────────────────────────────────────────────────────────

    fn hit_test(x: i32, y: i32, current_tool: AnnotationTool) -> ToolbarHit {
        let fx = x as f32;
        let fy = y as f32;

        // Row 1 buttons
        if fy >= ROW1_TOP && fy <= ROW1_TOP + ROW1_BTN_SIZE {
            let tools = [
                AnnotationTool::Arrow,
                AnnotationTool::Rectangle,
                AnnotationTool::Highlight,
                AnnotationTool::Pen,
                AnnotationTool::Blur,
                AnnotationTool::Text,
                AnnotationTool::Emoji,
            ];

            // Tool buttons
            for (i, tool) in tools.iter().enumerate() {
                let bx = MARGIN_X + i as f32 * ROW1_BTN_STEP;
                if fx >= bx && fx <= bx + ROW1_BTN_SIZE {
                    return ToolbarHit::Tool(*tool);
                }
            }

            // Action buttons (Undo, Redo, Done, Cancel)
            let sep1_x = MARGIN_X + tools.len() as f32 * ROW1_BTN_STEP + 2.0;
            let action_x = sep1_x + SEP_W;

            let undo_x = action_x;
            if fx >= undo_x && fx <= undo_x + ROW1_BTN_SIZE {
                return ToolbarHit::Undo;
            }
            let redo_x = action_x + ROW1_BTN_STEP;
            if fx >= redo_x && fx <= redo_x + ROW1_BTN_SIZE {
                return ToolbarHit::Redo;
            }

            let sep2_x = action_x + 2.0 * ROW1_BTN_STEP + 2.0;
            let done_x = sep2_x + SEP_W;
            if fx >= done_x && fx <= done_x + ROW1_BTN_SIZE {
                return ToolbarHit::Done;
            }
            let cancel_x = done_x + ROW1_BTN_STEP;
            if fx >= cancel_x && fx <= cancel_x + ROW1_BTN_SIZE {
                return ToolbarHit::Cancel;
            }
        }

        // Row 2
        if fy >= ROW2_TOP && fy <= ROW2_TOP + 26.0 {
            let swatch_y = ROW2_TOP + (26.0 - SWATCH_SIZE) / 2.0;
            if fy >= swatch_y && fy <= swatch_y + SWATCH_SIZE {
                if current_tool == AnnotationTool::Emoji {
                    // Emoji picker cells
                    for i in 0..EMOJI_PALETTE.len() {
                        let sx = MARGIN_X + i as f32 * SWATCH_STEP;
                        if fx >= sx && fx <= sx + SWATCH_SIZE {
                            return ToolbarHit::Emoji(i);
                        }
                    }
                    // Thickness presets after emoji palette
                    let emoji_end_x = MARGIN_X + EMOJI_PALETTE.len() as f32 * SWATCH_STEP + 2.0;
                    let thick_start_x = emoji_end_x + SEP_W;
                    for i in 0..THICKNESS_PRESETS.len() {
                        let tx = thick_start_x + i as f32 * SWATCH_STEP;
                        if fx >= tx && fx <= tx + SWATCH_SIZE {
                            return ToolbarHit::Thickness(i);
                        }
                    }
                } else {
                    // Color swatches
                    for i in 0..Color::PALETTE.len() {
                        let sx = MARGIN_X + i as f32 * SWATCH_STEP;
                        if fx >= sx && fx <= sx + SWATCH_SIZE {
                            return ToolbarHit::Color(i);
                        }
                    }
                    // Thickness presets after color palette
                    let color_end_x = MARGIN_X + Color::PALETTE.len() as f32 * SWATCH_STEP + 2.0;
                    let thick_start_x = color_end_x + SEP_W;
                    for i in 0..THICKNESS_PRESETS.len() {
                        let tx = thick_start_x + i as f32 * SWATCH_STEP;
                        if fx >= tx && fx <= tx + SWATCH_SIZE {
                            return ToolbarHit::Thickness(i);
                        }
                    }
                }
            }
        }

        ToolbarHit::None
    }

    /// Map x position to row-1 button index (for hover tracking).
    fn row1_button_index_at(x: i32, y: i32) -> i32 {
        let fx = x as f32;
        let fy = y as f32;
        if fy < ROW1_TOP || fy > ROW1_TOP + ROW1_BTN_SIZE {
            return -1;
        }

        // Tool buttons (0..6)
        for i in 0..7 {
            let bx = MARGIN_X + i as f32 * ROW1_BTN_STEP;
            if fx >= bx && fx <= bx + ROW1_BTN_SIZE {
                return i as i32;
            }
        }

        // Action buttons (7=Undo, 8=Redo, 9=Done, 10=Cancel)
        let sep1_x = MARGIN_X + 7.0 * ROW1_BTN_STEP + 2.0;
        let action_x = sep1_x + SEP_W;

        let undo_x = action_x;
        if fx >= undo_x && fx <= undo_x + ROW1_BTN_SIZE { return 7; }
        let redo_x = action_x + ROW1_BTN_STEP;
        if fx >= redo_x && fx <= redo_x + ROW1_BTN_SIZE { return 8; }

        let sep2_x = action_x + 2.0 * ROW1_BTN_STEP + 2.0;
        let done_x = sep2_x + SEP_W;
        if fx >= done_x && fx <= done_x + ROW1_BTN_SIZE { return 9; }
        let cancel_x = done_x + ROW1_BTN_STEP;
        if fx >= cancel_x && fx <= cancel_x + ROW1_BTN_SIZE { return 10; }

        -1
    }

    fn perform_undo(state: &mut EditorState) {
        if let Some(action) = state.undo_stack.pop() {
            match &action {
                UndoAction::Add(_) => {
                    // Remove the last annotation
                    state.layer.annotations.pop();
                }
                UndoAction::Move { index, dx, dy } => {
                    // Reverse the move
                    if let Some(ann) = state.layer.annotations.get_mut(*index) {
                        ann.offset_by(-dx, -dy);
                    }
                }
            }
            state.redo_stack.push(action);
            state.selected_index = None;
            Self::notify_change(state);
        }
    }

    fn perform_redo(state: &mut EditorState) {
        if let Some(action) = state.redo_stack.pop() {
            match &action {
                UndoAction::Add(ann) => {
                    state.layer.add(ann.clone());
                }
                UndoAction::Move { index, dx, dy } => {
                    if let Some(a) = state.layer.annotations.get_mut(*index) {
                        a.offset_by(*dx, *dy);
                    }
                }
            }
            state.undo_stack.push(action);
            state.selected_index = None;
            Self::notify_change(state);
        }
    }

    fn commit_text(state: &mut EditorState) {
        if !state.text_buffer.is_empty() {
            let annotation = Annotation::Text {
                position: state.text_position,
                text: state.text_buffer.clone(),
                color: state.stroke_color,
                font_size: state.thickness * 4.0,
            };
            state.layer.add(annotation.clone());
            state.undo_stack.push(UndoAction::Add(annotation));
            state.redo_stack.clear();
            Self::notify_change(state);
        }
        state.text_editing = false;
        state.text_buffer.clear();
    }

    /// Fire the on_change callback so the caller can update the clipboard
    /// with the latest annotated image.
    fn notify_change(state: &EditorState) {
        if let (Some(cb), Some(pixels)) = (&state.on_change, &state.pixels) {
            cb(pixels, &state.layer);
        }
    }

    // ─── Window procedure ───────────────────────────────────────────────────

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

                    let state = &mut *(cs.lpCreateParams as *mut EditorState);

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
                            dpiX: 96.0,
                            dpiY: 96.0,
                            ..Default::default()
                        };
                        let hwnd_props = D2D1_HWND_RENDER_TARGET_PROPERTIES {
                            hwnd,
                            pixelSize: D2D_SIZE_U {
                                width: w,
                                height: h,
                            },
                            presentOptions: D2D1_PRESENT_OPTIONS_IMMEDIATELY,
                        };
                        if let Ok(rt) =
                            factory.CreateHwndRenderTarget(&render_props, &hwnd_props)
                        {
                            state.render_target = Some(rt);
                        }
                        // Create round-cap stroke style for pen tool
                        let pen_props = D2D1_STROKE_STYLE_PROPERTIES {
                            startCap: D2D1_CAP_STYLE_ROUND,
                            endCap: D2D1_CAP_STYLE_ROUND,
                            lineJoin: D2D1_LINE_JOIN_ROUND,
                            ..Default::default()
                        };
                        if let Ok(base_factory) = factory.cast::<ID2D1Factory>() {
                            if let Ok(ss) = base_factory.CreateStrokeStyle(&pen_props, None) {
                                state.pen_stroke_style = Some(ss);
                            }
                        }

                        state.d2d_factory = Some(factory);
                    }

                    if let Ok(dwrite) =
                        DWriteCreateFactory::<IDWriteFactory>(DWRITE_FACTORY_TYPE_SHARED)
                    {
                        // Create cached brushes now that both RT and DWrite are available
                        if let Some(rt) = &state.render_target {
                            state.cached_brushes = CachedEditorBrushes::create(rt, &dwrite);
                        }
                        state.dwrite_factory = Some(dwrite);
                    }
                }
                LRESULT(0)
            }

            WM_PAINT => {
                let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut EditorState;
                if !state_ptr.is_null() {
                    Self::render(&*state_ptr);
                }
                let _ = ValidateRect(Some(hwnd), None);
                LRESULT(0)
            }

            WM_SIZE => {
                let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut EditorState;
                if !state_ptr.is_null() {
                    let state = &mut *state_ptr;
                    let w = (lparam.0 & 0xFFFF) as u32;
                    let h = ((lparam.0 >> 16) & 0xFFFF) as u32;
                    if let Some(rt) = &state.render_target {
                        let _ = rt.Resize(&D2D_SIZE_U {
                            width: w.max(1),
                            height: h.max(1),
                        });
                    }
                    Self::recompute_canvas_transform(state, w.max(1) as f32, h.max(1) as f32);
                }
                LRESULT(0)
            }

            WM_SETCURSOR => {
                // LOWORD of lparam is the hit-test result
                let hit_test_code = (lparam.0 & 0xFFFF) as u16;
                // HTCLIENT = 1
                if hit_test_code == 1 {
                    let state_ptr =
                        GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut EditorState;
                    if !state_ptr.is_null() {
                        let mut pt = POINT::default();
                        let _ = GetCursorPos(&mut pt);
                        let _ = ScreenToClient(hwnd, &mut pt);

                        if (pt.y as f32) < TOOLBAR_HEIGHT {
                            let _ = SetCursor(LoadCursorW(None, IDC_ARROW).ok());
                        } else if (*state_ptr).text_editing {
                            let _ = SetCursor(LoadCursorW(None, IDC_IBEAM).ok());
                        } else {
                            let _ = SetCursor(LoadCursorW(None, IDC_CROSS).ok());
                        }
                        return LRESULT(1);
                    }
                }
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }

            WM_LBUTTONDOWN => {
                let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut EditorState;
                if !state_ptr.is_null() {
                    let state = &mut *state_ptr;
                    let x = (lparam.0 & 0xFFFF) as i16 as i32;
                    let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;

                    if (y as f32) < TOOLBAR_HEIGHT {
                        // Toolbar click
                        match Self::hit_test(x, y, state.tool) {
                            ToolbarHit::Tool(tool) => {
                                // Commit any pending text before switching tools
                                if state.text_editing {
                                    Self::commit_text(state);
                                }
                                state.tool = tool;
                            }
                            ToolbarHit::Undo => {
                                Self::perform_undo(state);
                            }
                            ToolbarHit::Redo => {
                                Self::perform_redo(state);
                            }
                            ToolbarHit::Done => {
                                if state.text_editing {
                                    Self::commit_text(state);
                                }
                                if let Some(cb) = state.callback.take() {
                                    let layer = state.layer.clone();
                                    let _ = ShowWindow(hwnd, SW_HIDE);
                                    cb(Some(layer));
                                    return LRESULT(0);
                                }
                            }
                            ToolbarHit::Cancel => {
                                if let Some(cb) = state.callback.take() {
                                    let _ = ShowWindow(hwnd, SW_HIDE);
                                    cb(None);
                                    return LRESULT(0);
                                }
                            }
                            ToolbarHit::Color(idx) => {
                                state.color_index = idx;
                                state.stroke_color = Color::PALETTE[idx];
                                state.highlight_color = state.stroke_color.as_highlight();
                            }
                            ToolbarHit::Thickness(idx) => {
                                state.thickness_index = idx;
                                state.thickness = THICKNESS_PRESETS[idx];
                            }
                            ToolbarHit::Emoji(idx) => {
                                state.emoji_index = idx;
                            }
                            ToolbarHit::None => {}
                        }
                        let _ = InvalidateRect(Some(hwnd), None, false);
                    } else {
                        // Transform screen coords to image space
                        let img_x = (x as f32 - state.canvas_offset_x) / state.canvas_scale;
                        let img_y = (y as f32 - TOOLBAR_HEIGHT - state.canvas_offset_y) / state.canvas_scale;

                        // Clear right-click selection on any left-click on canvas
                        state.selected_index = None;

                        if state.tool == AnnotationTool::Text {
                            // Commit previous text if any, then start new text
                            if state.text_editing {
                                Self::commit_text(state);
                            }
                            state.text_editing = true;
                            state.text_position = Point {
                                x: img_x,
                                y: img_y,
                            };
                            state.text_buffer.clear();
                            let _ = InvalidateRect(Some(hwnd), None, false);
                        } else {
                            // Commit any pending text before drawing
                            if state.text_editing {
                                Self::commit_text(state);
                            }
                            // Start drawing on canvas (image-space coords)
                            state.drawing = true;
                            state.draw_start = Point {
                                x: img_x,
                                y: img_y,
                            };
                            state.draw_current = state.draw_start;
                            state.pen_points.clear();
                            state.pen_points.push(state.draw_start);
                            SetCapture(hwnd);
                        }
                    }
                }
                LRESULT(0)
            }

            WM_MOUSEMOVE => {
                let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut EditorState;
                if !state_ptr.is_null() {
                    let state = &mut *state_ptr;
                    let x = (lparam.0 & 0xFFFF) as i16 as i32;
                    let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;

                    if state.dragging_selected {
                        // Right-click drag: move the selected annotation
                        let img_x = (x as f32 - state.canvas_offset_x) / state.canvas_scale;
                        let img_y = (y as f32 - TOOLBAR_HEIGHT - state.canvas_offset_y) / state.canvas_scale;
                        let dx = img_x - state.drag_last_img.x;
                        let dy = img_y - state.drag_last_img.y;
                        if let Some(idx) = state.selected_index {
                            if let Some(ann) = state.layer.annotations.get_mut(idx) {
                                ann.offset_by(dx, dy);
                            }
                        }
                        state.drag_last_img = Point { x: img_x, y: img_y };
                        let _ = InvalidateRect(Some(hwnd), None, false);
                    } else if state.drawing {
                        // Transform screen coords to image space
                        let img_x = (x as f32 - state.canvas_offset_x) / state.canvas_scale;
                        let img_y = (y as f32 - TOOLBAR_HEIGHT - state.canvas_offset_y) / state.canvas_scale;
                        state.draw_current = Point {
                            x: img_x,
                            y: img_y,
                        };
                        if state.tool == AnnotationTool::Pen {
                            state.pen_points.push(state.draw_current);
                        }
                        Self::render(state);
                    } else if (y as f32) < TOOLBAR_HEIGHT {
                        let btn = Self::row1_button_index_at(x, y);
                        if state.hovered_btn != btn {
                            state.hovered_btn = btn;
                            let _ = InvalidateRect(Some(hwnd), None, false);
                        }
                    } else if state.hovered_btn != -1 {
                        state.hovered_btn = -1;
                        let _ = InvalidateRect(Some(hwnd), None, false);
                    }
                }
                LRESULT(0)
            }

            WM_LBUTTONUP => {
                let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut EditorState;
                if !state_ptr.is_null() {
                    let state = &mut *state_ptr;
                    if state.drawing {
                        state.drawing = false;
                        let _ = ReleaseCapture();

                        if let Some(annotation) = Self::build_current_annotation(state) {
                            state.layer.add(annotation.clone());
                            state.undo_stack.push(UndoAction::Add(annotation));
                            state.redo_stack.clear();
                            Self::notify_change(state);
                        }
                        state.pen_points.clear();
                        Self::render(state);
                    }
                }
                LRESULT(0)
            }

            WM_RBUTTONDOWN => {
                let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut EditorState;
                if !state_ptr.is_null() {
                    let state = &mut *state_ptr;
                    let x = (lparam.0 & 0xFFFF) as i16 as i32;
                    let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;

                    if (y as f32) >= TOOLBAR_HEIGHT {
                        // Transform screen coords to image space
                        let img_x = (x as f32 - state.canvas_offset_x) / state.canvas_scale;
                        let img_y = (y as f32 - TOOLBAR_HEIGHT - state.canvas_offset_y) / state.canvas_scale;

                        // Hit-test annotations in reverse order (topmost first)
                        let mut hit = None;
                        for i in (0..state.layer.annotations.len()).rev() {
                            if state.layer.annotations[i].hit_test_point(img_x, img_y) {
                                hit = Some(i);
                                break;
                            }
                        }

                        if let Some(idx) = hit {
                            state.selected_index = Some(idx);
                            state.dragging_selected = true;
                            state.drag_start_img = Point { x: img_x, y: img_y };
                            state.drag_last_img = Point { x: img_x, y: img_y };
                            SetCapture(hwnd);
                        } else {
                            state.selected_index = None;
                        }
                        let _ = InvalidateRect(Some(hwnd), None, false);
                    }
                }
                LRESULT(0)
            }

            WM_RBUTTONUP => {
                let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut EditorState;
                if !state_ptr.is_null() {
                    let state = &mut *state_ptr;
                    if state.dragging_selected {
                        let _ = ReleaseCapture();
                        state.dragging_selected = false;

                        if let Some(idx) = state.selected_index {
                            let total_dx = state.drag_last_img.x - state.drag_start_img.x;
                            let total_dy = state.drag_last_img.y - state.drag_start_img.y;
                            // Only push undo if actually moved
                            if total_dx.abs() > 0.5 || total_dy.abs() > 0.5 {
                                state.undo_stack.push(UndoAction::Move {
                                    index: idx,
                                    dx: total_dx,
                                    dy: total_dy,
                                });
                                state.redo_stack.clear();
                                Self::notify_change(state);
                            }
                        }
                        let _ = InvalidateRect(Some(hwnd), None, false);
                    }
                }
                LRESULT(0)
            }

            WM_KEYDOWN => {
                let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut EditorState;
                if !state_ptr.is_null() {
                    let state = &mut *state_ptr;
                    let vk = wparam.0 as u16;
                    let ctrl = GetKeyState(VK_CONTROL.0 as i32) as u16 & 0x8000 != 0;
                    let shift = GetKeyState(VK_SHIFT.0 as i32) as u16 & 0x8000 != 0;

                    if state.text_editing {
                        // Text editing mode key handling
                        if vk == VK_RETURN.0 && shift {
                            // Shift+Enter → commit text then save
                            Self::commit_text(state);
                            if let Some(cb) = state.callback.take() {
                                let layer = state.layer.clone();
                                let _ = ShowWindow(hwnd, SW_HIDE);
                                cb(Some(layer));
                                return LRESULT(0);
                            }
                        } else if vk == VK_ESCAPE.0 {
                            // Cancel text editing
                            state.text_editing = false;
                            state.text_buffer.clear();
                            let _ = InvalidateRect(Some(hwnd), None, false);
                        } else if vk == VK_RETURN.0 {
                            // Enter → Commit text annotation
                            Self::commit_text(state);
                            let _ = InvalidateRect(Some(hwnd), None, false);
                        } else if vk == VK_BACK.0 {
                            state.text_buffer.pop();
                            let _ = InvalidateRect(Some(hwnd), None, false);
                        }
                        // Other keys handled by WM_CHAR
                    } else {
                        // Normal mode key handling
                        if vk == VK_ESCAPE.0 {
                            if let Some(cb) = state.callback.take() {
                                let _ = ShowWindow(hwnd, SW_HIDE);
                                cb(None);
                                return LRESULT(0);
                            }
                        } else if vk == VK_RETURN.0 {
                            // Enter → Done (save)
                            if let Some(cb) = state.callback.take() {
                                let layer = state.layer.clone();
                                let _ = ShowWindow(hwnd, SW_HIDE);
                                cb(Some(layer));
                                return LRESULT(0);
                            }
                        } else if vk == 0x5A && ctrl {
                            // Ctrl+Z → Undo
                            Self::perform_undo(state);
                            let _ = InvalidateRect(Some(hwnd), None, false);
                        } else if vk == 0x59 && ctrl {
                            // Ctrl+Y → Redo
                            Self::perform_redo(state);
                            let _ = InvalidateRect(Some(hwnd), None, false);
                        } else if !ctrl {
                            // Single-key tool shortcuts
                            let new_tool = match vk {
                                0x41 => Some(AnnotationTool::Arrow),     // A
                                0x52 => Some(AnnotationTool::Rectangle), // R
                                0x48 => Some(AnnotationTool::Highlight), // H
                                0x50 => Some(AnnotationTool::Pen),       // P
                                0x42 => Some(AnnotationTool::Blur),      // B
                                0x54 => Some(AnnotationTool::Text),      // T
                                0x45 => Some(AnnotationTool::Emoji),     // E
                                _ => None,
                            };
                            if let Some(tool) = new_tool {
                                state.tool = tool;
                                let _ = InvalidateRect(Some(hwnd), None, false);
                            }
                        }
                    }
                }
                LRESULT(0)
            }

            WM_CHAR => {
                let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut EditorState;
                if !state_ptr.is_null() {
                    let state = &mut *state_ptr;
                    if state.text_editing {
                        let ch = char::from_u32(wparam.0 as u32);
                        if let Some(c) = ch {
                            // Filter out control characters
                            if !c.is_control() {
                                state.text_buffer.push(c);
                                let _ = InvalidateRect(Some(hwnd), None, false);
                            }
                        }
                    }
                }
                LRESULT(0)
            }

            WM_CLOSE => {
                // Intercept WM_CLOSE to HIDE instead of DESTROY.
                // The editor is persistent — DestroyWindow would free the state
                // and break all subsequent captures.
                info!("Annotation editor WM_CLOSE — hiding (not destroying)");
                let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut EditorState;
                if !state_ptr.is_null() {
                    let state = &mut *state_ptr;
                    if state.text_editing {
                        Self::commit_text(state);
                    }
                    if let Some(cb) = state.callback.take() {
                        let _ = ShowWindow(hwnd, SW_HIDE);
                        cb(None);
                        return LRESULT(0);
                    }
                }
                let _ = ShowWindow(hwnd, SW_HIDE);
                LRESULT(0) // Do NOT call DefWindowProcW — that calls DestroyWindow
            }

            WM_DESTROY => {
                // Only reached during actual app shutdown (not normal editor close).
                let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut EditorState;
                if !state_ptr.is_null() {
                    let mut state = Box::from_raw(state_ptr);
                    if state.text_editing && !state.text_buffer.is_empty() {
                        Self::commit_text(&mut state);
                    }
                    if let Some(cb) = state.callback.take() {
                        cb(None);
                    }
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                }
                LRESULT(0)
            }

            WM_GETMINMAXINFO => {
                let mmi = &mut *(lparam.0 as *mut MINMAXINFO);
                // Convert client minimums to window minimums (account for title bar + borders)
                let mut rc = RECT { left: 0, top: 0, right: MIN_CLIENT_W, bottom: MIN_CLIENT_H };
                let _ = AdjustWindowRectEx(&mut rc, WS_POPUP | WS_CAPTION | WS_SYSMENU | WS_THICKFRAME, false, WS_EX_TOPMOST);
                mmi.ptMinTrackSize.x = rc.right - rc.left;
                mmi.ptMinTrackSize.y = rc.bottom - rc.top;
                LRESULT(0)
            }

            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

unsafe impl Send for AnnotationEditor {}
unsafe impl Sync for AnnotationEditor {}

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn to_d2d_color(c: &Color) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: c.r,
        g: c.g,
        b: c.b,
        a: c.a,
    }
}

/// Sample the average color of a block from a PixelBuffer (BGRA).
fn sample_block_average(
    pixels: &PixelBuffer,
    x0: i32,
    y0: i32,
    bw: u32,
    bh: u32,
) -> D2D1_COLOR_F {
    let mut sum_r: u64 = 0;
    let mut sum_g: u64 = 0;
    let mut sum_b: u64 = 0;
    let mut count: u64 = 0;

    for py in y0..y0 + bh as i32 {
        for px in x0..x0 + bw as i32 {
            if px >= 0 && px < pixels.width as i32 && py >= 0 && py < pixels.height as i32 {
                let off = (py as u32 * pixels.stride + px as u32 * 4) as usize;
                if off + 2 < pixels.data.len() {
                    sum_b += pixels.data[off] as u64;
                    sum_g += pixels.data[off + 1] as u64;
                    sum_r += pixels.data[off + 2] as u64;
                    count += 1;
                }
            }
        }
    }

    if count == 0 {
        return D2D1_COLOR_F { r: 0.5, g: 0.5, b: 0.5, a: 1.0 };
    }

    D2D1_COLOR_F {
        r: (sum_r as f32 / count as f32) / 255.0,
        g: (sum_g as f32 / count as f32) / 255.0,
        b: (sum_b as f32 / count as f32) / 255.0,
        a: 1.0,
    }
}
