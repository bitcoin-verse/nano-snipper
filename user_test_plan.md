# Nano Snipper — User Test Plan

Comprehensive manual test plan covering all features, edge cases, and failure modes. Tests are grouped by subsystem, ordered from foundational to advanced. Each test has a unique ID for tracking.

---

## Prerequisites

- **OS**: Windows 11 (or Windows 10 1803+)
- **Build**: `cargo build --workspace` succeeds, both `snipd.exe` and `snipui.exe` in `target/debug/`
- **Environment**: At least one monitor with desktop content visible. A text editor (Notepad), an image editor (Paint), and a web browser open for paste targets
- **Clean state**: Delete `%LOCALAPPDATA%\NanoSnipper\` before starting (ensures fresh DB and config)

---

## 1. Startup & Process Lifecycle

### 1.1 First Launch

| ID | Test | Steps | Expected |
|----|------|-------|----------|
| S-01 | Clean first launch | Delete `%LOCALAPPDATA%\NanoSnipper\`. Run `snipd.exe` | Tray icon appears (blue square). `config.toml` created with defaults. `history.db` created. No errors in console |
| S-02 | Default config values | Open `%LOCALAPPDATA%\NanoSnipper\config.toml` | `auto_copy_timeout_secs = 5`, `start_on_login = true`, `play_sound = false`, `max_age_days = 7`, `max_size_mb = 2048`. Hotkeys: Win+Shift+1/2/3/O/P |
| S-03 | Tray icon tooltip | Hover tray icon | Tooltip shows "Nano Snipper" |

### 1.2 Single Instance

| ID | Test | Steps | Expected |
|----|------|-------|----------|
| S-04 | Second instance prevented | Run `snipd.exe` while first is running | Second process exits immediately with "Another instance of snipd is already running" (check with `RUST_LOG=info`) |
| S-05 | Restart after exit | Exit first instance via tray > Exit. Run `snipd.exe` again | Starts normally, tray icon reappears |

### 1.3 Shutdown

| ID | Test | Steps | Expected |
|----|------|-------|----------|
| S-06 | Tray Exit | Right-click tray > Exit | `snipd.exe` exits cleanly. Tray icon disappears. Hotkeys unregistered (verify Win+Shift+1 does nothing) |
| S-07 | Hotkey cleanup on exit | Register hotkeys, then Exit. Open any app, press Win+Shift+1 | No capture overlay appears (hotkeys properly unregistered) |
| S-08 | Clipboard data preserved on exit | Capture a screenshot, then Exit via tray | Clipboard data still pasteable in Paint (WM_RENDERALLFORMATS provided data before destruction) |

---

## 2. Hotkeys

### 2.1 Registration

| ID | Test | Steps | Expected |
|----|------|-------|----------|
| H-01 | All hotkeys register | Run `snipd.exe` with `RUST_LOG=debug` | Log shows "Registered hotkey Region", Window, Fullscreen, Ocr, PinLast |
| H-02 | Conflict detection | Open another app that registers Win+Shift+1 first, then start `snipd` | Warning logged: "Failed to register hotkey Region". Other hotkeys still work |

### 2.2 Pause/Resume

| ID | Test | Steps | Expected |
|----|------|-------|----------|
| H-03 | Pause hotkeys | Right-click tray > Pause. Press Win+Shift+1 | No overlay appears. Log shows "Hotkeys paused, ignoring Region" |
| H-04 | Resume hotkeys | Right-click tray > Pause again (toggle). Press Win+Shift+1 | Overlay appears normally. Log shows "Hotkeys resumed" |

---

## 3. Region Capture (Win+Shift+1)

### 3.1 Basic Flow

| ID | Test | Steps | Expected |
|----|------|-------|----------|
| R-01 | Overlay appears | Press Win+Shift+1 | Fullscreen semi-transparent dark overlay covers all monitors. Crosshair cursor visible. Crosshair lines follow mouse |
| R-02 | Draw selection | Click and drag a rectangle (~300x200px) | Clear selection area visible (no dark tint inside). White border around selection. Dimension text (e.g., "300x200") shown above selection in rounded rectangle. Dark overlay outside selection (4 bands) |
| R-03 | Release completes capture | Release mouse button | Overlay disappears. Action bar appears below the selection area. Image data announced to clipboard |
| R-04 | Paste into Paint | After capture, open Paint, Ctrl+V | Pasted image matches the selected screen region exactly. Colors correct (no BGRA/RGBA swap). Dimensions match the selection |
| R-05 | Paste into browser | After capture, paste into a chat input or image upload | Image pastes correctly |

### 3.2 Selection Behaviors

| ID | Test | Steps | Expected |
|----|------|-------|----------|
| R-06 | Right-to-left selection | Press Win+Shift+1. Drag from bottom-right to top-left | Selection normalizes correctly. Captured region matches visible selection |
| R-07 | Very small selection (<5px) | Press Win+Shift+1. Click with minimal drag (<5px) | Selection ignored (minimum 5x5 threshold). Overlay stays visible |
| R-08 | Very large selection (full screen) | Press Win+Shift+1. Drag from corner to corner | Captures full screen. Paste matches screen content |
| R-09 | Selection across monitors | If multi-monitor: drag selection spanning two monitors | Selection captures pixels from both monitors correctly |
| R-10 | Cancel with Escape | Press Win+Shift+1. Press Escape | Overlay disappears. No capture made. No clipboard change. Log shows "Selection cancelled" |
| R-11 | Cancel mid-drag | Press Win+Shift+1. Start dragging, then press Escape | Overlay disappears. No capture made |
| R-12 | Crosshair rendering pre-selection | Press Win+Shift+1. Move mouse without clicking | Crosshair lines (horizontal + vertical) follow cursor across full overlay. No selection border visible yet |
| R-13 | Dimension text position | Press Win+Shift+1. Drag downward | Dimension text appears above the top-left corner of the selection, with dark background pill |

---

## 4. Window Capture (Win+Shift+2)

### 4.1 Basic Flow

| ID | Test | Steps | Expected |
|----|------|-------|----------|
| W-01 | Window mode overlay | Press Win+Shift+2 | Dark overlay appears. Cursor is crosshair |
| W-02 | Window hover highlight | Move mouse over different windows | Blue border (3px, ~0.2/0.5/1.0 blue) highlights the top-level window under cursor. Clear area inside the window rect. Dark overlay outside |
| W-03 | Window click captures | Click on a highlighted window | Overlay disappears. Action bar appears. Clipboard contains the window's content |
| W-04 | Paste window capture | After W-03, paste into Paint | Image matches the exact window rect (including title bar, borders) |

### 4.2 Edge Cases

| ID | Test | Steps | Expected |
|----|------|-------|----------|
| W-05 | Child window detection | Hover over a button inside a window | Highlights the top-level parent window (GA_ROOT), not the child control |
| W-06 | Minimized window | Hover over taskbar button of a minimized window | Does not highlight a zero-size window. Falls through to desktop or taskbar |
| W-07 | Cancel window capture | Press Win+Shift+2, then Escape | Overlay disappears. No capture |
| W-08 | No window under cursor | Move to empty desktop area (no windows) | Full dark overlay, no blue highlight. Click does nothing (hovered_hwnd == 0) |
| W-09 | Overlay hide/show flicker | Move mouse in window mode | Overlay briefly hides for WindowFromPoint then re-shows. Transition should be imperceptible (no visible flicker) |

---

## 5. Fullscreen Capture (Win+Shift+3)

| ID | Test | Steps | Expected |
|----|------|-------|----------|
| F-01 | Instant capture | Press Win+Shift+3 | No overlay shown. Clipboard immediately contains fullscreen image. Log: "Fullscreen captured: WxH" |
| F-02 | Paste fullscreen | After F-01, paste into Paint | Image matches entire primary monitor content. Correct resolution |
| F-03 | No action bar | Press Win+Shift+3 | No action bar shown (fullscreen skips action bar in current implementation) |
| F-04 | History entry created | Press Win+Shift+3. Check `history.db` | Entry exists with mode "Fullscreen", correct dimensions, PNG file created |

---

## 6. OCR Capture (Win+Shift+O)

### 6.1 Basic Flow

| ID | Test | Steps | Expected |
|----|------|-------|----------|
| O-01 | OCR mode overlay | Press Win+Shift+O | Same overlay as region capture appears (crosshair, dark overlay) |
| O-02 | Select text region | Open a webpage with text. Press Win+Shift+O. Draw rectangle around text | Overlay disappears. No action bar shown |
| O-03 | Text on clipboard | After O-02, paste into Notepad (Ctrl+V) | Recognized text from the selected region appears. Line breaks preserved |
| O-04 | OCR charset | Select region containing English text | Text recognized accurately |

### 6.2 Edge Cases

| ID | Test | Steps | Expected |
|----|------|-------|----------|
| O-05 | Image with no text | Select a region of a photo with no text | Empty or minimal text on clipboard |
| O-06 | Cancel OCR | Press Win+Shift+O, then Escape | No clipboard change |
| O-07 | Small text | Select a region with very small (8px) text | OCR attempts recognition. Quality depends on WinRT engine |
| O-08 | Mixed content | Select region with text and images | Text portions extracted, images ignored |

---

## 7. Action Bar

### 7.1 Appearance

| ID | Test | Steps | Expected |
|----|------|-------|----------|
| A-01 | Position | Region capture a 400x300 area | Action bar (300x48px) appears centered below the selection, 8px gap |
| A-02 | Visual design | Observe action bar | Dark background (RGB ~30,30,36). 4 buttons evenly spaced: Copy, Save, Pin, Edit. Each shows label + shortcut hint below (Enter, S, P, A) |
| A-03 | Hover effect | Hover over each button | Hovered button gets lighter background (rounded rect, 6px radius). Unhovered buttons normal |
| A-04 | Always on top | Click a window behind the action bar | Action bar remains visible on top |

### 7.2 Button Actions — Keyboard

| ID | Test | Steps | Expected |
|----|------|-------|----------|
| A-05 | Enter = Copy | Region capture, then press Enter | Action bar disappears. Log: "Action: Copy (already on clipboard)". Image still pasteable |
| A-06 | S = Save | Region capture, then press S | Native file save dialog opens. Default filename: `screenshot_<timestamp>.png`. Filter: "PNG Image (*.png)" |
| A-07 | P = Pin | Region capture, then press P | Pin window appears (see section 8). Action bar disappears |
| A-08 | A = Annotate | Region capture, then press A | Annotation editor opens (see section 9). Action bar disappears |
| A-09 | Escape = Cancel | Region capture, then press Escape | Action bar disappears. Log: "Action: Cancel" |

### 7.3 Button Actions — Mouse

| ID | Test | Steps | Expected |
|----|------|-------|----------|
| A-10 | Click Copy | Region capture, click the Copy button | Same as A-05 |
| A-11 | Click Save | Region capture, click the Save button | Same as A-06 |
| A-12 | Click Pin | Region capture, click the Pin button | Same as A-07 |
| A-13 | Click Edit | Region capture, click the Edit button | Same as A-08 |

### 7.4 Auto-Copy Timer

| ID | Test | Steps | Expected |
|----|------|-------|----------|
| A-14 | Auto-copy fires | Set `auto_copy_timeout_secs = 3` in config. Region capture. Wait 3 seconds | Action bar disappears automatically. Copy action triggered |
| A-15 | Auto-copy cancelled by action | Region capture. Press S before timeout | Save dialog opens. Timer cancelled (no second action after save completes) |
| A-16 | Auto-copy disabled | Set `auto_copy_timeout_secs = 0`. Region capture | Action bar stays indefinitely until manual action |

### 7.5 Save Dialog

| ID | Test | Steps | Expected |
|----|------|-------|----------|
| A-17 | Save PNG | Capture, press S, choose location, click Save | PNG file created at chosen location. Opens correctly in image viewer. Colors correct (BGRA properly converted to RGBA) |
| A-18 | Save cancel | Capture, press S, click Cancel in dialog | No file created. No error |
| A-19 | Default extension | Capture, press S, type "test" without extension | File saved as `test.png` (default extension applied) |
| A-20 | Overwrite existing | Save to same filename twice | System save dialog prompts for overwrite confirmation |

---

## 8. Pin to Screen

### 8.1 Creation

| ID | Test | Steps | Expected |
|----|------|-------|----------|
| P-01 | Pin from action bar | Region capture, press P | Pin window appears showing the captured image. Always-on-top. Size capped at 800x600 or image size (whichever smaller) |
| P-02 | Pin via hotkey | Capture something. Press Win+Shift+P | Pin window created from last capture |
| P-03 | Pin without capture | Restart snipd (fresh, no captures). Press Win+Shift+P | Warning logged: "No last capture to pin". No window created |

### 8.2 Interaction

| ID | Test | Steps | Expected |
|----|------|-------|----------|
| P-04 | Drag to move | Click and drag anywhere on pin window | Window moves with mouse (NCHITTEST returns HTCAPTION for client area) |
| P-05 | Resize | Drag window edges/corners | Window resizes. Image scales to fill (linear interpolation) |
| P-06 | Opacity up | Scroll mouse wheel up on pin window | Opacity increases by 0.1 per step, max 1.0 |
| P-07 | Opacity down | Scroll mouse wheel down on pin window | Opacity decreases by 0.1 per step, min 0.1 (never fully invisible) |
| P-08 | Close with Escape | Focus pin window, press Escape | Window closes and is destroyed |
| P-09 | Multiple pins | Capture, Pin. Capture again, Pin | Two independent pin windows exist. Each draggable, resizable, closeable independently |
| P-10 | Always on top | Open another window covering the pin area | Pin window remains visible above other windows |
| P-11 | Pin image quality | Resize pin larger than original capture | Image smoothly scaled via linear interpolation (not pixelated) |

---

## 9. Annotation Editor

### 9.1 Opening

| ID | Test | Steps | Expected |
|----|------|-------|----------|
| AN-01 | Open from action bar | Region capture, press A | Editor window opens, centered on screen. Title: "Annotate - Nano Snipper". Shows screenshot below toolbar. Toolbar at top (40px): Arrow, Rect, Hilite, Pen, Undo, Done |
| AN-02 | Default tool | Editor opens | Arrow tool selected (highlighted in blue) |
| AN-03 | Window sizing | Capture a large region (e.g., 1600x1200) | Editor window caps at screen size minus 100px on each dimension |
| AN-04 | Always on top | Editor window visible | Stays above other windows |

### 9.2 Tool Selection

| ID | Test | Steps | Expected |
|----|------|-------|----------|
| AN-05 | Click Arrow | Click "Arrow" in toolbar | Arrow button highlighted blue. Others deselected |
| AN-06 | Click Rect | Click "Rect" in toolbar | Rect button highlighted blue |
| AN-07 | Click Hilite | Click "Hilite" in toolbar | Hilite button highlighted blue |
| AN-08 | Click Pen | Click "Pen" in toolbar | Pen button highlighted blue |
| AN-09 | Toolbar hover | Hover over toolbar buttons | Hovered button gets darker background. Non-hovered buttons normal |

### 9.3 Drawing — Arrow

| ID | Test | Steps | Expected |
|----|------|-------|----------|
| AN-10 | Draw arrow | Select Arrow. Click-drag on canvas | Red arrow drawn from start to end point. Live preview while dragging. Arrowhead at endpoint (two wing lines) |
| AN-11 | Arrow arrowhead | Draw a long arrow (~200px) | Arrowhead length = min(thickness * 4, arrow_length * 0.3). Two wing lines visible at tip |
| AN-12 | Short arrow | Draw tiny arrow (<5px) | Arrow renders but no arrowhead (length check prevents degenerate geometry) |
| AN-13 | Arrow persists | Draw arrow, release mouse | Arrow committed to annotation layer. Remains visible |

### 9.4 Drawing — Rectangle

| ID | Test | Steps | Expected |
|----|------|-------|----------|
| AN-14 | Draw rectangle | Select Rect. Click-drag on canvas | Red rectangle outline drawn. Live preview while dragging |
| AN-15 | Rectangle orientation | Drag in any direction (left-to-right, right-to-left, etc.) | Rectangle normalizes correctly regardless of drag direction |

### 9.5 Drawing — Highlight

| ID | Test | Steps | Expected |
|----|------|-------|----------|
| AN-16 | Draw highlight | Select Hilite. Click-drag on canvas | Yellow semi-transparent filled rectangle appears over the screenshot. Alpha = 0.5 |
| AN-17 | Highlight over text | Draw highlight over text in screenshot | Text beneath visible through the yellow tint |

### 9.6 Drawing — Pen (Freehand)

| ID | Test | Steps | Expected |
|----|------|-------|----------|
| AN-18 | Draw freehand | Select Pen. Click-drag in a curved path | Red freehand stroke follows mouse path. Connected line segments between sampled points |
| AN-19 | Pen smoothness | Draw slowly vs quickly | More points captured when moving slowly (smoother curve). Faster movement = coarser segments |

### 9.7 Undo

| ID | Test | Steps | Expected |
|----|------|-------|----------|
| AN-20 | Undo via toolbar | Draw 3 annotations. Click "Undo" | Last annotation removed. First two remain |
| AN-21 | Undo via Ctrl+Z | Draw annotations. Press Ctrl+Z | Last annotation removed per press |
| AN-22 | Multiple undos | Draw 3 annotations. Press Ctrl+Z three times | All annotations removed. Canvas shows clean screenshot |
| AN-23 | Undo on empty | Press Ctrl+Z with no annotations | No crash, no error. Nothing changes |

### 9.8 Done / Cancel

| ID | Test | Steps | Expected |
|----|------|-------|----------|
| AN-24 | Done via toolbar | Draw annotations. Click "Done" | Editor closes. Callback fires with AnnotationLayer containing all annotations. Log: "Annotation complete: N annotations" |
| AN-25 | Done via Enter | Draw annotations. Press Enter | Same as AN-24 |
| AN-26 | Cancel via Escape | Draw annotations. Press Escape | Editor closes. Callback fires with None. Log: "Annotation cancelled" |
| AN-27 | Close button (X) | Draw annotations. Click window X button | Editor destroyed. Callback fires with None (WM_DESTROY fallback) |
| AN-28 | Done with no annotations | Open editor. Immediately press Enter | Callback fires with empty AnnotationLayer (valid, 0 annotations) |

### 9.9 Rendering

| ID | Test | Steps | Expected |
|----|------|-------|----------|
| AN-29 | Screenshot fidelity | Open editor on a colorful screenshot | Screenshot renders correctly below toolbar, fills canvas area |
| AN-30 | Window resize | Resize editor window | Screenshot and annotations scale to new window size. Render target resizes correctly |
| AN-31 | Overlapping annotations | Draw multiple overlapping shapes | Later annotations render on top of earlier ones. All visible |

---

## 10. Clipboard

### 10.1 Delayed Rendering

| ID | Test | Steps | Expected |
|----|------|-------|----------|
| C-01 | Delayed announce | Capture a region. Do NOT paste yet | Clipboard owns CF_DIBV5 format. Log: "Delayed clipboard rendering announced" |
| C-02 | Paste triggers render | After C-01, paste into Paint | WM_RENDERFORMAT fires. Log: "WM_RENDERFORMAT: providing CF_DIBV5 data". Image appears correctly |
| C-03 | Multiple pastes | Paste the same capture into Paint, then into another app | Both pastes produce identical images. WM_RENDERFORMAT fires each time a new app requests data |
| C-04 | Fallback to immediate | If delayed rendering fails (simulate by corrupting clipboard?), observe log | "Clipboard delayed announce error" followed by immediate `set_image` fallback |

### 10.2 Image Correctness

| ID | Test | Steps | Expected |
|----|------|-------|----------|
| C-05 | Color fidelity | Capture a screen region with known colors (e.g., solid red #FF0000 rectangle) | Pasted image shows exact same colors. No BGRA/RGBA channel swap |
| C-06 | Alpha channel | Capture a region with transparency (if applicable) | CF_DIBV5 preserves alpha channel (BITMAPV5HEADER with alpha mask 0xFF000000) |
| C-07 | Top-down orientation | Paste into Paint | Image is right-side-up (negative bV5Height = top-down DIB) |
| C-08 | Stride handling | Capture odd-width regions (e.g., 301x200) | Image correct — stride (RowPitch) may differ from width*4 due to GPU alignment. Row-by-row copy handles this |

### 10.3 OCR Text Clipboard

| ID | Test | Steps | Expected |
|----|------|-------|----------|
| C-09 | CF_UNICODETEXT | OCR capture text region. Paste into Notepad | Text pastes correctly as Unicode. Line breaks present |
| C-10 | Previous clipboard replaced | Have text on clipboard. Do OCR capture | Old clipboard content replaced with new OCR text |

### 10.4 WM_RENDERALLFORMATS

| ID | Test | Steps | Expected |
|----|------|-------|----------|
| C-11 | Data on exit | Capture a region. Exit snipd via tray. Paste into Paint | Image still pastes correctly (WM_RENDERALLFORMATS provided data before window destruction) |

---

## 11. History

### 11.1 Storage

| ID | Test | Steps | Expected |
|----|------|-------|----------|
| HI-01 | PNG file created | Region capture | PNG file appears at `%LOCALAPPDATA%\NanoSnipper\captures\<year>\<month>\<uuid>.png`. Opens in image viewer. Colors correct |
| HI-02 | Directory structure | Capture in February 2026 | File at `captures/2026/02/<uuid>.png` |
| HI-03 | DB entry | Check `history.db` with SQLite viewer | Row in `captures` table with: id (UUID), timestamp_ms, mode ("Region"), region coords, file_path (relative), file_size, width, height, ocr_text (NULL), annotated (0) |
| HI-04 | Thumbnail stored | Check `thumbnail` column in DB | BLOB contains JPEG data. 200px wide, proportional height |
| HI-05 | All modes recorded | Capture with Region, Window, Fullscreen, OCR | Each creates a history entry with correct `mode` value |

### 11.2 Non-Blocking

| ID | Test | Steps | Expected |
|----|------|-------|----------|
| HI-06 | Async write | Rapidly capture 5 screenshots in succession | All captures complete immediately (action bar appears instantly). PNGs appear in filesystem within seconds. No UI blocking |
| HI-07 | Large capture | Capture fullscreen at 4K resolution | Capture pipeline returns instantly. PNG write happens in background. File may take a moment to appear |

### 11.3 Deletion

| ID | Test | Steps | Expected |
|----|------|-------|----------|
| HI-08 | Delete entry via snipui | Open history in snipui. Click Delete on an entry | Entry disappears from list. DB row removed. PNG file deleted from disk |
| HI-09 | Delete nonexistent | Delete an entry, then try deleting again (via IPC) | No crash. Already-deleted entry is no-op |

### 11.4 Retention Cleanup

| ID | Test | Steps | Expected |
|----|------|-------|----------|
| HI-10 | Age-based cleanup | Set `max_age_days = 0` (no limit). Capture. Set `max_age_days = 1`. Manually set a capture's timestamp to 2 days ago in DB. Restart snipd | Old entry deleted on startup cleanup. Log: "Retention cleanup: removed 1 old entries" |
| HI-11 | Cleanup preserves recent | With `max_age_days = 7`, all captures <7 days old | No captures deleted |

---

## 12. IPC (snipd <-> snipui)

### 12.1 Connection

| ID | Test | Steps | Expected |
|----|------|-------|----------|
| I-01 | snipui connects | Start snipd. Start snipui | snipd logs "IPC client connected". snipui loads config from snipd |
| I-02 | snipui without snipd | Start snipui without snipd running | snipui shows IPC error gracefully. Falls back to local config file |
| I-03 | Reconnect | Start both. Close snipui. Open snipui again | Reconnects successfully. Config loads |

### 12.2 Config Sync

| ID | Test | Steps | Expected |
|----|------|-------|----------|
| I-04 | GetConfig | Start both. snipui Settings page | Shows current config values matching `config.toml` |
| I-05 | SetConfig | Change auto-copy timeout in snipui. Click Save | Config saved to `config.toml`. snipd acknowledges (Ack). Subsequent captures use new timeout |
| I-06 | Save button state | Open snipui Settings. No changes made | Save button is greyed out (disabled). Change a setting | Save button becomes active |

### 12.3 History Queries

| ID | Test | Steps | Expected |
|----|------|-------|----------|
| I-07 | GetHistory | Navigate to History tab in snipui | Shows list of captures, most recent first. Timestamp, dimensions, mode visible |
| I-08 | Pagination | Have >50 captures. Go to History | First page shows 50 entries. "Page 1 of N" displayed. "Next >" button visible |
| I-09 | Next/Prev page | Click "Next >". Then click "< Prev" | Page 2 loads different entries. Back to page 1 shows original entries |
| I-10 | Search | Type in search box | Only entries with matching OCR text shown. "Showing X of Y" updates. Page resets to 1 |
| I-11 | Empty search | Clear search box | All entries shown again |

### 12.4 History Actions

| ID | Test | Steps | Expected |
|----|------|-------|----------|
| I-12 | Open entry | Click entry row in history | Default image viewer opens the PNG file |
| I-13 | Delete entry from UI | Click Delete button on an entry | Entry removed from list. Reloads history. Count decremented |
| I-14 | Open deleted file | Delete entry, then try opening it again | File doesn't exist. No crash (path check) |

---

## 13. System Tray

### 13.1 Menu Items

| ID | Test | Steps | Expected |
|----|------|-------|----------|
| T-01 | Right-click menu | Right-click tray icon | Context menu with: Open Settings, History, Pause, Exit |
| T-02 | Open Settings | Click "Open Settings" | `snipui.exe` launches. Settings page visible |
| T-03 | History | Click "History" | `snipui.exe` launches with `--page=history` argument |
| T-04 | Pause | Click "Pause" | Hotkeys paused. Log: "Tray: Hotkeys paused". All capture hotkeys disabled |
| T-05 | Unpause | Click "Pause" again (toggle) | Hotkeys resumed. Log: "Tray: Hotkeys resumed" |
| T-06 | Exit | Click "Exit" | snipd shuts down cleanly |

### 13.2 snipui Launch

| ID | Test | Steps | Expected |
|----|------|-------|----------|
| T-07 | snipui not found | Rename snipui.exe. Click "Open Settings" in tray | Warning logged: "snipui.exe not found at <path>". No crash |
| T-08 | Multiple snipui instances | Click "Open Settings" twice | Two snipui windows open (no singleton check on snipui) |

---

## 14. snipui Application

### 14.1 Navigation

| ID | Test | Steps | Expected |
|----|------|-------|----------|
| U-01 | Default page | Launch snipui | Settings page shown. Nav bar at top with "Settings" and "History" buttons |
| U-02 | Switch to History | Click "History" button | History page loads. IPC request sent to snipd |
| U-03 | Switch back to Settings | Click "Settings" button | Settings page shown with current values |
| U-04 | Window title | Switch pages | Title updates: "Nano Snipper - Settings" or "Nano Snipper - History" |
| U-05 | Dark theme | Observe UI | Dark theme applied (iced Theme::Dark) |

### 14.2 Settings Page

| ID | Test | Steps | Expected |
|----|------|-------|----------|
| U-06 | Hotkeys display | View Settings > Hotkeys section | Shows all 5 hotkeys with their key combinations (display-only, not editable) |
| U-07 | Auto-copy slider | Drag auto-copy timeout slider | Value updates (0-30 range). Displayed value changes |
| U-08 | Start on login toggle | Click toggler | Toggler flips. Dirty flag set |
| U-09 | Play sound toggle | Click toggler | Toggler flips. Dirty flag set |
| U-10 | Max age slider | Drag max age slider | Value updates (0-90 range). Displayed value changes |
| U-11 | Max size display | View retention section | Shows current max_size_mb value (display-only in current implementation) |
| U-12 | Dirty tracking | Change a setting | Save button becomes active (was greyed out). Revert not available — user must save or close |
| U-13 | Save and verify | Change auto-copy to 10. Click Save | Button greys out (dirty = false). `config.toml` updated. snipd receives new config via IPC |

### 14.3 History Page

| ID | Test | Steps | Expected |
|----|------|-------|----------|
| U-14 | Empty state | No captures yet. Go to History | Shows "No captures yet. Use Win+Shift+1 to take a screenshot." |
| U-15 | Entry format | With captures present. Go to History | Each row: "YYYY-MM-DD HH:MM:SS - WxH - Mode" + Delete button |
| U-16 | Scrollable list | Have many captures | List is scrollable (iced scrollable widget) |
| U-17 | Status bar | View bottom of History page | "Showing X of Y captures" text |

---

## 15. Performance

### 15.1 Latency

| ID | Test | Steps | Expected |
|----|------|-------|----------|
| PF-01 | Overlay display time | Press Win+Shift+1 and observe | Overlay appears in <20ms (pre-created window, ShowWindow only). No perceivable delay |
| PF-02 | Capture to clipboard | Finish selection | Clipboard announced before action bar appears. Feel instant |
| PF-03 | Paste latency | Ctrl+V in Paint after capture | WM_RENDERFORMAT + GPU readback completes in ~15ms. Paste feels instant |
| PF-04 | Action bar display | Complete a region selection | Action bar appears immediately after overlay hides |
| PF-05 | Rapid captures | Capture 5 regions in quick succession (10 seconds) | All captures complete. No hangs. History records all 5 |

### 15.2 Memory

| ID | Test | Steps | Expected |
|----|------|-------|----------|
| PF-06 | Idle memory | Start snipd, no captures. Check Task Manager | <100 MB RSS (D3D11 device + overlay + SQLite) |
| PF-07 | After captures | Capture 10 regions. Check memory | Memory may spike during capture but returns to baseline. Only `last_capture` kept in memory |
| PF-08 | Pin window memory | Create 5 pin windows. Check memory | Each pin holds a D2D bitmap. Memory increases proportionally to pinned image sizes |

### 15.3 History Write Performance

| ID | Test | Steps | Expected |
|----|------|-------|----------|
| PF-09 | Non-blocking save | Time from selection-release to action-bar-visible | <50ms. PNG encoding happens on background thread |
| PF-10 | Large image save | Capture 4K fullscreen | PNG file appears within a few seconds. Main thread never blocked |

---

## 16. Error Handling & Edge Cases

### 16.1 GPU/D3D11 Failures

| ID | Test | Steps | Expected |
|----|------|-------|----------|
| E-01 | No GPU | (Simulated: run in VM without GPU acceleration) | Error logged: "Failed to init capture engine". snipd still starts (for tray/settings). Hotkeys don't crash |
| E-02 | DXGI frame timeout | Lock screen or remote desktop during capture | `AcquireNextFrame(500ms)` times out. Error logged. No crash |

### 16.2 Clipboard Conflicts

| ID | Test | Steps | Expected |
|----|------|-------|----------|
| E-03 | Clipboard locked by another app | Another app holds clipboard open. Capture | OpenClipboard fails. Error logged. Capture still saved to history |
| E-04 | Clipboard manager installed | Third-party clipboard manager active | Delayed rendering may trigger WM_RENDERFORMAT immediately (clipboard manager requests data). Should work correctly |

### 16.3 File System

| ID | Test | Steps | Expected |
|----|------|-------|----------|
| E-05 | Read-only captures dir | Set captures dir to read-only | History save fails. Error logged. Capture still on clipboard |
| E-06 | Disk full | Simulate full disk | PNG save fails. Error logged. Capture still on clipboard |
| E-07 | Long file path | Captures dir deep in filesystem | Works if within Windows MAX_PATH. Error if exceeds |

### 16.4 IPC Failures

| ID | Test | Steps | Expected |
|----|------|-------|----------|
| E-08 | Oversized message | Send >10MB IPC message (programmatic) | Server rejects: "IPC message too large: N bytes" |
| E-09 | Malformed JSON | Send invalid JSON via pipe (programmatic) | Serde error logged. Connection closed. Server continues accepting new connections |
| E-10 | Client disconnect | Close snipui while IPC request in flight | Server logs "IPC client disconnected". No crash. Ready for next connection |

### 16.5 Config Edge Cases

| ID | Test | Steps | Expected |
|----|------|-------|----------|
| E-11 | Corrupt config.toml | Write garbage to config.toml. Start snipd | Warning: "Failed to parse config, using defaults". Starts with default values |
| E-12 | Missing config.toml | Delete config.toml. Start snipd | Silently uses defaults. New config.toml created on next save |
| E-13 | Missing AppData dir | Delete `%LOCALAPPDATA%\NanoSnipper\`. Start snipd | Directory recreated automatically |

---

## 17. Multi-Monitor

| ID | Test | Steps | Expected |
|----|------|-------|----------|
| M-01 | Overlay spans all monitors | Multi-monitor setup. Press Win+Shift+1 | Overlay covers all monitors (SM_XVIRTUALSCREEN / SM_CXVIRTUALSCREEN) |
| M-02 | Selection on secondary monitor | Draw selection entirely on secondary monitor | Captured image matches secondary monitor content. Coordinates correct |
| M-03 | Selection spanning monitors | Draw selection across monitor boundary | Captures pixels from both monitors seamlessly |
| M-04 | Different DPI monitors | One 100% and one 150% monitor. Capture on each | Both captures correct in physical pixels. No DPI-related distortion |
| M-05 | Monitor added/removed | Plug in/unplug monitor while snipd running. Capture | Overlay re-reads virtual screen bounds on each show(). Adapts to new configuration |

---

## 18. Integration Scenarios

### 18.1 Full Workflow

| ID | Test | Steps | Expected |
|----|------|-------|----------|
| INT-01 | Capture-annotate-save | Win+Shift+1 > draw region > press A > draw arrow + rectangle > press Enter > press S > save file | PNG file contains screenshot with annotations rendered. File opens correctly |
| INT-02 | Capture-pin-capture-paste | Capture, Pin. Capture different region, Paste | First capture pinned. Second capture on clipboard. Both independent |
| INT-03 | OCR then region | Win+Shift+O > select text. Win+Shift+1 > select region | First clipboard = text. Second clipboard = image. Both operations complete independently |
| INT-04 | Settings change mid-session | Change auto-copy timeout in snipui while snipd running. Do capture | New timeout value used for next capture's action bar |

### 18.2 Rapid Succession

| ID | Test | Steps | Expected |
|----|------|-------|----------|
| INT-05 | Back-to-back captures | Capture region. Immediately press Win+Shift+1 again without interacting with action bar | Second overlay appears (may hide first action bar). Second capture works |
| INT-06 | Capture during annotation | Open annotation editor. Press Win+Shift+3 | Hotkey may not fire (editor has focus) or fullscreen capture occurs alongside editor. No crash |

---

## Test Results Template

| ID | Status | Date | Notes |
|----|--------|------|-------|
| S-01 | | | |
| S-02 | | | |
| ... | | | |

**Status values**: PASS, FAIL, SKIP, BLOCKED

---

## Known Limitations (not bugs)

- Hotkey remap UI not yet interactive (display-only in settings)
- History thumbnails not rendered in grid (text list only)
- Blur annotation tool defined in data model but not wired in editor
- `max_size_mb` retention not enforced (only `max_age_days` cleanup implemented)
- Magnifier not implemented in overlay
- Annotation layer not baked into saved PNG (callback receives layer data but save-with-annotations pipeline not wired end-to-end)
- Action bar not shown for fullscreen captures
- No sound effect implemented (`play_sound` config exists but unused)
