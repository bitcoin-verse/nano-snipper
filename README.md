# Nano Snipper

Low-latency, keyboard-first screenshot tool for Windows 11.

GPU-accelerated capture pipeline with a built-in annotation editor. No Electron, no .NET, no Python. Just Rust talking directly to DXGI and Direct2D.

## Why

Windows Snipping Tool takes 500ms+ at P50 (2,600ms+ at P95) to get data on the clipboard. Nano Snipper does it in **under 1ms** -- over **500x faster**. The secret: deferred GPU readback. The clipboard is announced via delayed rendering (CF_DIBV5) immediately after the GPU acquires the frame. The CPU never touches pixel data until something actually pastes, and by then a pre-rendered DIB is already waiting.

## Features

| Hotkey | Action |
|--------|--------|
| `Ctrl+Shift+1` | Region capture -- draw a rectangle, then annotate |
| `Ctrl+Shift+2` | Fullscreen capture -- instant, opens annotation editor |

Every capture opens the **annotation editor** where you can draw on the screenshot before saving.

### Annotation Editor

Draw directly on your capture before copying or saving:

| Tool | Shortcut | Description |
|------|----------|-------------|
| Arrow | `A` | Draw arrows with arrowheads |
| Rectangle | `R` | Draw rectangle outlines |
| Highlight | `H` | Semi-transparent highlight overlay |
| Pen | `P` | Freehand drawing with round caps |
| Blur | `B` | Real pixelation blur (NxN block averaging) |
| Text | `T` | Click to place, type, Enter to commit |

| Action | Shortcut |
|--------|----------|
| Undo | `Ctrl+Z` |
| Redo | `Ctrl+Y` |
| Done (save + copy) | Click checkmark or `Shift+Enter` |
| Cancel | `Escape` |

**Color palette**: 9 colors (red, orange, yellow, green, cyan, blue, purple, white, black) selectable from toolbar row 2.

**Thickness presets**: Thin (2px), Medium (5px), Thick (10px) selectable from toolbar row 2.

All annotations are baked into the output image via CPU rendering -- what you see in the editor is what gets saved and copied.

### History

Every capture is saved automatically:

- Full-resolution PNGs in `%LOCALAPPDATA%\NanoSnipper\captures\`
- SQLite metadata + JPEG thumbnails in `history.db`
- Thumbnail previews in the history list
- Configurable retention (age + size limits, both enforced)

### Settings

Configurable via tray menu or `snipui.exe`:

- Hotkey remapping (click a binding to re-record)
- Start on login (writes to Windows registry `HKCU\...\Run`)
- Retention policy (max age in days, max storage size in MB)

## Architecture

Three binaries, 13 crates.

```
snipd.exe       Daemon -- Win32 message loop, D3D11, Direct2D, DXGI
snipui.exe      Settings & history UI -- iced 0.13 (tiny-skia CPU renderer), named-pipe IPC
snip-bench.exe  Benchmark harness -- SendInput, clipboard polling, memory profiling
```

### How capture works

```
Ctrl+Shift+2 pressed (fullscreen)
  |
  |  DXGI acquire frame (persistent duplication)    ~5ms
  |  Open annotation editor (persistent, pre-created)  ~5ms  -- just bitmap upload + show
  |  User annotates screenshot
  |  Done -> clipboard set (image + file path)      ~3ms
  |         PNG save on background thread            ~50ms   -- non-blocking
  '  Done
```

```
Ctrl+Shift+1 pressed (region)
  |
  |  ShowWindow(overlay)                            < 20ms
  |  User draws selection rectangle
  |  DXGI acquire + GPU crop                        ~5-15ms
  |  Open annotation editor                         ~5ms
  |  User annotates screenshot
  |  Done -> clipboard set                          ~3ms
  |         PNG save on background thread           ~50ms    -- non-blocking
  '  Done
```

The annotation editor window is pre-created at startup and reused across captures. D2D factory, render target, DWrite factory, and all toolbar brushes persist between sessions -- only the bitmap is re-uploaded on each capture.

### State machine

```
Idle --[hotkey]--> Selecting (region) or Capturing (fullscreen)
  ^                   |
  |    [Esc]----------'
  |                   |
  |    [done]---------'--> Annotating
  |                            |
  |              .-------------.
  |              |             |
  |            Done         Cancel
  |              |             |
  |         Copy + Save        |
  |              |             |
  '--------------'-------------'
```

### Crate map

```
crates/
  ns-common/         Shared types -- config, IPC protocol, history model, paths
  snipd/             Daemon binary -- message loop, state machine, tray
  snip-capture/      DXGI Output Duplication, D3D11 GPU crop, persistent dup, texture caching
  snip-overlay/      Fullscreen transparent overlay (Win32 + Direct2D)
  snip-clipboard/    CF_DIBV5 delayed rendering, bulk HGLOBAL copy
  snip-hotkeys/      RegisterHotKey wrapper
  snip-ui/           Annotation editor (Win32 + Direct2D), action bar
  snip-history/      SQLite + PNG storage, JPEG thumbnails, background writer
  snip-ipc/          Named pipe server/client (length-prefixed JSON)
  snip-annotate/     Annotation data model + CPU renderer (arrow, rect, highlight, pen, blur, text)
  snip-bench/        External benchmark harness (SendInput, clipboard polling)
  snipui/            Settings & history GUI (iced)
```

### IPC protocol

`snipd` and `snipui` communicate over `\\.\pipe\NanoSnipper` using length-prefixed (4-byte LE) JSON messages:

```
snipui -> snipd:  GetConfig, SetConfig, GetHistory, DeleteEntry, TriggerCapture, SetPaused
snipd  -> snipui: ConfigData, HistoryData, CaptureCompleted, EntryDeleted, Error, Ack
```

## Building

### Prerequisites

- **Rust 1.75+** (2021 edition)
- **Windows 11** (or Windows 10 1803+)
- **Visual Studio Build Tools** with Windows SDK

### Build

```
cargo build --workspace
```

Outputs to `target/debug/`:
- `snipd.exe` -- the daemon
- `snipui.exe` -- settings/history UI
- `snip-bench.exe` -- benchmark harness

### Release

```
cargo build --workspace --release
```

## Usage

1. Run `snipd.exe` -- appears in the system tray
2. Press a hotkey to capture
3. Annotate in the editor, then press Done (checkmark) or `Shift+Enter`
4. Image is on clipboard + saved as PNG automatically
5. Right-click the tray icon for Settings, History, Pause, or Exit

## Configuration

`%LOCALAPPDATA%\NanoSnipper\config.toml`

```toml
[hotkeys]
region = { modifiers = ["Ctrl", "Shift"], key = 49 }      # Ctrl+Shift+1
fullscreen = { modifiers = ["Ctrl", "Shift"], key = 50 }   # Ctrl+Shift+2

[behavior]
start_on_login = true

[retention]
max_age_days = 7
max_size_mb = 2048
```

## Data storage

```
%LOCALAPPDATA%\NanoSnipper\
  config.toml                           Settings
  history.db                            SQLite -- metadata + JPEG thumbnails
  captures/
    screenshot_1739312400.png           Full-resolution capture
```

## Benchmarks

Measured with `snip-bench.exe` on Windows 11 (release build, fullscreen capture):

| Metric | Nano Snipper | Windows Snipping Tool | Speedup |
|--------|-------------|----------------------|---------|
| Hotkey -> clipboard P50 | **< 1 ms** | 513 ms | **>500x** |
| Hotkey -> clipboard P95 | **10 ms** | 2,652 ms | **~265x** |
| Paste render P50 | **13.5 ms** | 24 ms | **~1.8x** |
| Memory (idle / peak) | **79 / 125 MB** | -- / 180 MB | **31% less** |

### Optimization pipeline

Fourteen optimization phases across two rounds bring every path to near-instant:

**Round 1: Capture hot path (8 phases)**

| Phase | Technique | Savings |
|-------|-----------|---------|
| 1 | `Arc<Vec<u8>>` PixelBuffer -- clone is refcount bump, not 8MB memcpy | -2ms, -40MB peak |
| 2 | SSE2 SIMD alpha fixup -- 4 pixels per iteration | -1 to -3ms |
| 3 | Deferred GPU readback -- clipboard announced after GPU acquire | **-15ms** |
| 4 | Persistent DXGI duplication -- reuse across captures | -3 to -5ms |
| 5 | Pre-allocated textures -- cache when dimensions match | -0.5 to -1ms |
| 6 | Pre-rendered DIB -- build before paste request arrives | -8ms on paste |
| 7 | Cached D2D brushes (overlay + action bar) | smoother rendering |
| 8 | Parallel startup -- history DB overlapped with D3D11 | -9ms startup |

**Round 2: Editor + save path (6 phases)**

| Phase | Technique | Savings |
|-------|-----------|---------|
| 1 | Async PNG save -- clipboard first, PNG on background thread | **Done->ready: 200ms -> 8ms** |
| 2 | Cached editor brushes -- 20+ brushes created once, zero per-frame allocs | **Frame: 15ms -> 1ms** |
| 3 | Persistent editor window -- pre-created hidden, reused per capture | **Open: 50ms -> 5ms** |
| 4 | Fast PNG encoding -- bulk BGRA->RGBA + fast compression | **Save: 200ms -> 50ms** |
| 5 | Optimized CPU rendering -- dist^2 (no sqrt), opaque fast-path, row-wise fill | **Bake: 100ms -> 30ms** |
| 6 | Clipboard bulk copy -- single memcpy when stride is tight | **Clipboard: 8ms -> 2ms** |

### Startup time

Breakdown via `snipd --benchmark-startup`:

| Stage | Duration |
|-------|----------|
| D3D11 device creation | ~93 ms |
| Overlay pre-create | ~26 ms |
| Annotation editor pre-create | ~13 ms |
| History store open | ~9 ms (parallel with D3D11) |
| **Total startup** | **~130 ms** |

### Running benchmarks

```bash
# Startup timing (standalone, exits after measuring)
snipd.exe --benchmark-startup

# Nano Snipper fullscreen (requires snipd running)
snip-bench.exe --tool nano-snipper --mode fullscreen --runs 10 --json

# Windows Snipping Tool comparison
snip-bench.exe --tool snipping-tool --mode fullscreen --runs 5 --delay 5
```

## Performance targets

| Metric | Target | Achieved | How |
|--------|--------|----------|-----|
| Overlay visible | < 20ms | ~26ms | Pre-created hidden window, `ShowWindow` on hotkey |
| Clipboard ready (fullscreen) | < 10ms | **< 1ms P50** | Deferred GPU readback |
| Editor open (from capture) | < 10ms | **~5ms** | Persistent window, bitmap upload only |
| Done -> ready | < 10ms | **~8ms** | Async PNG, clipboard-first |
| Paste render | < 15ms | **13.5ms P50** | Pre-rendered DIB + SIMD alpha fixup |
| Memory (idle) | < 100 MB | 79 MB | D3D11 ~30 MB, overlay ~30 MB, SQLite ~5 MB |
| Memory (peak) | < 150 MB | **125 MB** | Arc<Vec<u8>> eliminates redundant pixel copies |
| History write | 0ms blocking | 0ms | Background writer thread via mpsc channel |
| Startup | < 150ms | **~130ms** | History DB init parallelized with D3D11 creation |

## Dependencies

| Crate | Version | Role |
|-------|---------|------|
| `windows` | 0.62 | Win32 + WinRT bindings |
| `iced` | 0.13 | snipui GUI framework (tiny-skia software renderer) |
| `tokio` | 1 | Async runtime (IPC) |
| `rusqlite` | 0.32 | SQLite (bundled) |
| `image` | 0.25 | PNG encoding, thumbnails |
| `tray-icon` | 0.19 | System tray |
| `serde` + `toml` | 1 / 0.8 | Serialization |
| `tracing` | 0.1 | Structured logging |
| `uuid` | 1 (v7) | Time-sortable history IDs |

## License

MIT
