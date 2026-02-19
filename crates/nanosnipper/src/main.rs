//! nanosnipper — Nano Snipper daemon.
//!
//! Runs as a system tray application, always in the background.
//! Handles hotkeys, capture pipeline, clipboard, overlay, action bar, and IPC.

#![windows_subsystem = "windows"]

use anyhow::Result;
use ns_common::config::NsConfig;
use ns_common::hotkey::HotkeyId;
use ns_common::ipc::IpcMessage;
use ns_common::{CaptureMode, CaptureResult, CaptureTiming};
use parking_lot::Mutex;
use std::sync::Arc;
use std::time::Instant;
use tracing::{debug, error, info, warn};
use windows::Win32::Foundation::*;
use windows::Win32::System::DataExchange::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Ole::CF_DIBV5;
use windows::Win32::System::Threading::*;

use windows::Win32::UI::WindowsAndMessaging::*;

/// Thread-safe shared state (config, history, paused flag).
/// Accessed by both the main thread and the IPC server thread.
struct SharedState {
    config: Mutex<NsConfig>,
    history: Option<Mutex<snip_history::HistoryStore>>,
    paused: Mutex<bool>,
    main_hwnd: isize, // HWND as isize for Send safety; used by IPC to signal main thread
}

/// Custom message ID for deferred GPU readback.
const WM_APP_DEFERRED_READBACK: u32 = WM_APP + 1;

/// Custom message ID to re-register hotkeys after config change via IPC.
const WM_APP_REREGISTER_HOTKEYS: u32 = WM_APP + 2;

/// Application state for the main thread's message loop.
/// Contains Win32 handles that are NOT Send.
struct AppState {
    shared: Arc<SharedState>,
    capture_engine: Option<snip_capture::CaptureEngine>,
    overlay: Option<snip_overlay::Overlay>,
    annotation_editor: Option<snip_ui::AnnotationEditor>,
    hotkey_manager: Option<snip_hotkeys::HotkeyManager>,
    last_capture: Option<CaptureResult>,
    /// GPU frame acquired but not yet read back to CPU.
    /// Used for deferred readback: clipboard is announced immediately after
    /// GPU acquire, and CPU readback happens asynchronously via WM_APP_DEFERRED_READBACK.
    pending_capture: Option<snip_capture::PendingCapture>,
    /// Pre-rendered DIB handle ready for immediate paste (Phase 6).
    pre_rendered_dib: Option<HANDLE>,
    main_hwnd: HWND,
    tray_pause_item: Option<tray_icon::menu::MenuItem>,
}

fn main() {
    let startup_t0 = Instant::now();

    // Log to file since #![windows_subsystem = "windows"] has no console.
    // Log file lives next to the exe for easy access.
    let log_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    let log_file = std::fs::File::create(log_dir.join("nanosnipper.log"))
        .or_else(|_| std::fs::File::create(std::env::temp_dir().join("nanosnipper.log")))
        .expect("Failed to create log file in both exe dir and temp dir");

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_target(false)
        .with_writer(std::sync::Mutex::new(log_file))
        .with_ansi(false)
        .init();

    let benchmark_startup = std::env::args().any(|a| a == "--benchmark-startup");

    info!("nanosnipper starting");

    if benchmark_startup {
        if let Err(e) = run_startup_benchmark(startup_t0) {
            error!("Startup benchmark error: {e}");
            std::process::exit(1);
        }
        return;
    }

    if let Err(e) = run() {
        error!("Fatal error: {e}");
        std::process::exit(1);
    }
}

fn run_startup_benchmark(startup_t0: Instant) -> Result<()> {
    let log_step = |name: &str, t0: Instant| -> Instant {
        let now = Instant::now();
        info!(
            step = name,
            elapsed_ms = now.duration_since(t0).as_secs_f64() * 1000.0,
            since_start_ms = now.duration_since(startup_t0).as_secs_f64() * 1000.0,
            "startup_timing"
        );
        now
    };

    let t = startup_t0;

    let _config = NsConfig::load();
    let t = log_step("config_load", t);

    let _instance = unsafe { GetModuleHandleW(None)? };
    let t = log_step("get_module_handle", t);

    let mut capture_engine = snip_capture::CaptureEngine::new()?;
    let t = log_step("d3d11_device_create", t);

    let _overlay = snip_overlay::Overlay::new()?;
    let t = log_step("overlay_precreate", t);

    let _history = snip_history::HistoryStore::new()?;
    let t = log_step("history_store_open", t);

    // Quick warmup capture to measure first-frame latency
    match capture_engine.capture_fullscreen() {
        Ok(result) => {
            if let Some(ref timing) = result.timing {
                info!(
                    dxgi_acquire_ms = timing.dxgi_acquire.as_secs_f64() * 1000.0,
                    cpu_readback_ms = timing.cpu_readback.as_secs_f64() * 1000.0,
                    total_ms = timing.total.as_secs_f64() * 1000.0,
                    "startup_warmup_capture"
                );
            }
        }
        Err(e) => warn!("Warmup capture failed: {e}"),
    }
    let _t = log_step("warmup_capture", t);

    let total = Instant::now().duration_since(startup_t0);
    info!(
        total_ms = total.as_secs_f64() * 1000.0,
        "startup_benchmark_complete"
    );

    // Print summary to stdout for easy scripting
    println!("startup_complete_ms={:.2}", total.as_secs_f64() * 1000.0);
    Ok(())
}

fn run() -> Result<()> {
    let _mutex = ensure_single_instance()?;

    let config = NsConfig::load();
    info!("Config loaded");

    sync_startup_registry(config.behavior.start_on_login);

    let instance = unsafe { GetModuleHandleW(None)? };

    let wc = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        lpfnWndProc: Some(wnd_proc),
        hInstance: instance.into(),
        lpszClassName: windows::core::w!("NanoSnipperDaemon"),
        ..Default::default()
    };
    unsafe { RegisterClassExW(&wc) };

    let hwnd = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            windows::core::w!("NanoSnipperDaemon"),
            windows::core::w!("NanoSnipper"),
            WINDOW_STYLE::default(),
            0, 0, 0, 0,
            Some(HWND_MESSAGE),
            None,
            Some(HINSTANCE(instance.0)),
            None,
        )?
    };

    info!("Message window created: {:?}", hwnd);

    // Initialize subsystems — overlap history DB init with D3D11 device creation
    // (history ~9ms on background thread while D3D11 ~93ms on main thread)
    let history_handle = std::thread::Builder::new()
        .name("history-init".into())
        .spawn(|| {
            match snip_history::HistoryStore::new() {
                Ok(h) => { info!("History store opened"); Some(h) }
                Err(e) => { error!("Failed to open history: {e}"); None }
            }
        })
        .ok();

    let capture_engine = match snip_capture::CaptureEngine::new() {
        Ok(e) => { info!("Capture engine initialized"); Some(e) }
        Err(e) => { error!("Failed to init capture engine: {e}"); None }
    };

    let overlay = match snip_overlay::Overlay::new() {
        Ok(o) => { info!("Overlay pre-created"); Some(o) }
        Err(e) => { error!("Failed to create overlay: {e}"); None }
    };

    let annotation_editor = match snip_ui::AnnotationEditor::create_hidden() {
        Ok(e) => { info!("Annotation editor pre-created"); Some(e) }
        Err(e) => { error!("Failed to create annotation editor: {e}"); None }
    };

    // Join history init thread (should be done by now since D3D11 took ~93ms)
    let history = history_handle
        .and_then(|h| h.join().ok())
        .flatten();

    let mut hotkey_manager = snip_hotkeys::HotkeyManager::new(hwnd);
    if let Err(e) = hotkey_manager.register_all(&config.hotkeys) {
        warn!("Some hotkeys failed to register: {e}");
    }

    // Shared state (thread-safe, used by IPC)
    let shared = Arc::new(SharedState {
        config: Mutex::new(config),
        history: history.map(Mutex::new),
        paused: Mutex::new(false),
        main_hwnd: hwnd.0 as isize,
    });

    // System tray — must be created on the main thread (which has the message loop)
    // so that tray-icon can process Win32 messages for right-click, etc.
    let tray_pause_item = setup_tray(Arc::clone(&shared), hwnd.0 as isize);

    // Main-thread state (contains Win32 handles)
    let state = Box::new(Mutex::new(AppState {
        shared: Arc::clone(&shared),
        capture_engine,
        overlay,
        annotation_editor,
        hotkey_manager: Some(hotkey_manager),
        last_capture: None,
        pending_capture: None,
        pre_rendered_dib: None,
        main_hwnd: hwnd,
        tray_pause_item,
    }));

    unsafe {
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(state) as isize);
    }

    // IPC server on background thread
    let ipc_shared = Arc::clone(&shared);
    std::thread::Builder::new()
        .name("ipc-server".into())
        .spawn(move || {
            let rt = match tokio::runtime::Runtime::new() {
                Ok(rt) => rt,
                Err(e) => {
                    error!("Failed to create tokio runtime: {e}");
                    return;
                }
            };
            rt.block_on(async {
                if let Err(e) = run_ipc_server(ipc_shared).await {
                    error!("IPC server error: {e}");
                }
            });
        })?;

    // Retention cleanup at startup
    if let Some(ref history) = shared.history {
        history.lock().cleanup_async();
    }

    info!("nanosnipper ready — entering message loop");

    unsafe {
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }

    info!("nanosnipper shutting down");
    Ok(())
}

fn ensure_single_instance() -> Result<HANDLE> {
    let name = windows::core::w!("Global\\NanoSnipperDaemon");
    let handle = unsafe { CreateMutexW(None, false, name)? };
    let last_error = unsafe { GetLastError() };
    if last_error == ERROR_ALREADY_EXISTS {
        anyhow::bail!("Another instance of nanosnipper is already running");
    }
    Ok(handle)
}

/// Create the tray icon on the calling thread (must be the main thread with a
/// message loop) and return the Pause `MenuItem` so the main thread can toggle
/// its text later via `WM_APP`.  A background thread handles `MenuEvent` dispatch.
fn setup_tray(shared: Arc<SharedState>, main_hwnd_raw: isize) -> Option<tray_icon::menu::MenuItem> {
    use tray_icon::menu::{Menu, MenuItem, PredefinedMenuItem};
    use tray_icon::TrayIconBuilder;

    let menu = Menu::new();
    let item_settings = MenuItem::new("Settings", true, None);
    let item_history = MenuItem::new("History", true, None);
    let item_pause = MenuItem::new("Pause", true, None);
    let item_exit = MenuItem::new("Exit", true, None);
    let item_about = MenuItem::new("About", true, None);

    let id_settings = item_settings.id().clone();
    let id_history = item_history.id().clone();
    let id_pause = item_pause.id().clone();
    let id_exit = item_exit.id().clone();
    let id_about = item_about.id().clone();

    let _ = menu.append(&item_settings);
    let _ = menu.append(&item_history);
    let _ = menu.append(&item_pause);
    let _ = menu.append(&item_exit);
    let _ = menu.append(&PredefinedMenuItem::separator());
    let _ = menu.append(&item_about);

    // Embedded 32x32 RGBA app icon (generated by scripts/gen_icon.py)
    let icon_rgba = include_bytes!("../../../resources/tray_32x32.rgba").to_vec();
    let icon = tray_icon::Icon::from_rgba(icon_rgba, 32, 32).ok();

    let mut builder = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip("Nano Snipper");

    if let Some(icon) = icon {
        builder = builder.with_icon(icon);
    }

    match builder.build() {
        Ok(tray) => {
            Box::leak(Box::new(tray));
            info!("System tray icon created");
        }
        Err(e) => {
            warn!("Failed to create tray icon: {e}");
            return None;
        }
    }

    // Background thread for menu event dispatch.
    // MenuItem is !Send, so we cannot call set_text here. Instead, for the pause
    // toggle we post WM_APP to the main window which does the set_text call.
    std::thread::Builder::new()
        .name("tray-events".into())
        .spawn(move || {
            use tray_icon::menu::MenuEvent;

            loop {
                if let Ok(event) = MenuEvent::receiver().recv() {
                    if event.id == id_exit {
                        info!("Tray: Exit requested");
                        let hwnd = HWND(main_hwnd_raw as *mut std::ffi::c_void);
                        unsafe {
                            let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
                        }
                    } else if event.id == id_pause {
                        let mut paused = shared.paused.lock();
                        *paused = !*paused;
                        info!("Tray: Hotkeys {}", if *paused { "paused" } else { "resumed" });
                        // Signal main thread to update the menu item text
                        let hwnd = HWND(main_hwnd_raw as *mut std::ffi::c_void);
                        unsafe {
                            let _ = PostMessageW(Some(hwnd), WM_APP, WPARAM(0), LPARAM(0));
                        }
                    } else if event.id == id_settings || event.id == id_history || event.id == id_about {
                        let page = if event.id == id_history {
                            "history"
                        } else if event.id == id_about {
                            "about"
                        } else {
                            "settings"
                        };
                        info!("Tray: Launching snipui (page={page})");
                        let exe_dir = std::env::current_exe()
                            .ok()
                            .and_then(|p| p.parent().map(|d| d.to_path_buf()));
                        if let Some(dir) = exe_dir {
                            let snipui = dir.join("snipui.exe");
                            if snipui.exists() {
                                if let Err(e) = std::process::Command::new(&snipui)
                                    .arg(format!("--page={page}"))
                                    .spawn()
                                {
                                    error!("Failed to launch snipui: {e}");
                                }
                            } else {
                                warn!("snipui.exe not found at {:?}", snipui);
                            }
                        }
                    }
                }
            }
        })
        .ok();

    Some(item_pause)
}

// ─── IPC server ──────────────────────────────────────────────────────────────

async fn run_ipc_server(shared: Arc<SharedState>) -> Result<()> {
    snip_ipc::IpcServer::run(move |msg| handle_ipc_message(&shared, msg)).await
}

fn handle_ipc_message(shared: &SharedState, msg: IpcMessage) -> Option<IpcMessage> {
    match msg {
        IpcMessage::GetConfig => {
            Some(IpcMessage::ConfigData(shared.config.lock().clone()))
        }
        IpcMessage::SetConfig(new_config) => {
            info!(
                "IPC SetConfig: region vk=0x{:X}, fullscreen vk=0x{:X}",
                new_config.hotkeys.region.key,
                new_config.hotkeys.fullscreen.key,
            );
            if let Err(e) = new_config.save() {
                error!("Failed to save config: {e}");
                return Some(IpcMessage::Error(format!("Failed to save: {e}")));
            }
            sync_startup_registry(new_config.behavior.start_on_login);
            *shared.config.lock() = new_config;
            // Signal main thread to re-register hotkeys with updated config
            let hwnd = HWND(shared.main_hwnd as *mut std::ffi::c_void);
            let post_result = unsafe {
                PostMessageW(Some(hwnd), WM_APP_REREGISTER_HOTKEYS, WPARAM(0), LPARAM(0))
            };
            if let Err(ref e) = post_result {
                error!("PostMessageW WM_APP_REREGISTER_HOTKEYS FAILED: {e}");
            } else {
                info!("PostMessageW WM_APP_REREGISTER_HOTKEYS: ok (hwnd={:?})", hwnd);
            }
            Some(IpcMessage::Ack)
        }
        IpcMessage::GetHistory { offset, limit, search } => {
            if let Some(ref history) = shared.history {
                let h = history.lock();
                match h.query(offset, limit, search.as_deref()) {
                    Ok((entries, total)) => {
                        let thumbnails: Vec<Option<Vec<u8>>> = entries
                            .iter()
                            .map(|e| h.get_thumbnail(&e.id).ok().flatten())
                            .collect();
                        Some(IpcMessage::HistoryData { entries, total, thumbnails })
                    }
                    Err(e) => Some(IpcMessage::Error(e.to_string())),
                }
            } else {
                Some(IpcMessage::HistoryData { entries: vec![], total: 0, thumbnails: vec![] })
            }
        }
        IpcMessage::DeleteEntry(id) => {
            if let Some(ref history) = shared.history {
                history.lock().delete_async(id);
            }
            Some(IpcMessage::EntryDeleted(id))
        }
        IpcMessage::SetPaused(paused) => {
            *shared.paused.lock() = paused;
            info!("Hotkeys {}", if paused { "paused" } else { "resumed" });
            Some(IpcMessage::Ack)
        }
        IpcMessage::TriggerCapture(mode) => {
            info!("IPC triggered capture: {:?}", mode);
            Some(IpcMessage::Ack)
        }
        _ => {
            debug!("Unexpected IPC message: {:?}", std::mem::discriminant(&msg));
            None
        }
    }
}

// ─── Hotkey handling ─────────────────────────────────────────────────────────

fn handle_hotkey(state: &Mutex<AppState>, hotkey_id: HotkeyId) {
    let s = state.lock();
    if *s.shared.paused.lock() {
        debug!("Hotkeys paused, ignoring {:?}", hotkey_id);
        return;
    }

    let mode = match hotkey_id {
        HotkeyId::Region => CaptureMode::Region,
        HotkeyId::Fullscreen => CaptureMode::Fullscreen,
    };

    info!("Hotkey triggered: {:?} -> {:?}", hotkey_id, mode);
    drop(s);

    match mode {
        CaptureMode::Fullscreen => do_fullscreen_capture(state),
        _ => do_region_capture(state),
    }
}

fn sync_startup_registry(start_on_login: bool) {
    use windows::Win32::System::Registry::*;

    let run_key = windows::core::w!(
        "Software\\Microsoft\\Windows\\CurrentVersion\\Run"
    );
    let value_name = windows::core::w!("NanoSnipper");

    unsafe {
        let mut hkey = HKEY::default();
        let result = RegOpenKeyExW(HKEY_CURRENT_USER, run_key, Some(0), KEY_SET_VALUE, &mut hkey);
        if result.is_err() {
            error!("Failed to open Run registry key: {:?}", result);
            return;
        }

        if start_on_login {
            let exe_path = match std::env::current_exe() {
                Ok(p) => p.to_string_lossy().to_string(),
                Err(e) => {
                    error!("Failed to get exe path: {e}");
                    let _ = RegCloseKey(hkey);
                    return;
                }
            };
            let wide: Vec<u16> = exe_path.encode_utf16().chain(std::iter::once(0)).collect();
            let bytes = std::slice::from_raw_parts(
                wide.as_ptr() as *const u8,
                wide.len() * 2,
            );
            let result = RegSetValueExW(hkey, value_name, Some(0), REG_SZ, Some(bytes));
            if result.is_err() {
                error!("Failed to set startup registry value: {:?}", result);
            } else {
                info!("Startup registry entry created");
            }
        } else {
            let result = RegDeleteValueW(hkey, value_name);
            if result.is_err() {
                debug!("Failed to delete startup registry value (may not exist): {:?}", result);
            } else {
                info!("Startup registry entry removed");
            }
        }

        let _ = RegCloseKey(hkey);
    }
}

fn do_fullscreen_capture(state: &Mutex<AppState>) {
    let hotkey_received = Instant::now();

    // Scope the lock: capture + extract what we need, then drop before editor.show()
    let (pixels_for_annotate, editor_hwnd, main_hwnd, pixels_for_history, capture_mode, capture_region, capture_w, capture_h, history_arc) = {
        let mut s = state.lock();
        let Some(ref mut engine) = s.capture_engine else {
            error!("No capture engine");
            return;
        };

        let capture = match engine.capture_fullscreen() {
            Ok(c) => c,
            Err(e) => {
                error!("Fullscreen capture failed: {e}");
                return;
            }
        };

        let main_hwnd = s.main_hwnd;
        let capture_ms = hotkey_received.elapsed().as_secs_f64() * 1000.0;
        info!("Fullscreen capture: {}x{} in {:.2}ms", capture.pixels.width, capture.pixels.height, capture_ms);

        let pixels_for_annotate = capture.pixels.clone();
        let pixels_for_history = capture.pixels.clone();
        let capture_mode = capture.mode;
        let capture_region = capture.region;
        let capture_w = capture.pixels.width;
        let capture_h = capture.pixels.height;

        s.last_capture = Some(capture);

        let editor_hwnd = s.annotation_editor.as_ref().map(|e| e.hwnd());
        let history_arc: Arc<SharedState> = s.shared.clone();

        // Lock dropped here
        (pixels_for_annotate, editor_hwnd, main_hwnd, pixels_for_history, capture_mode, capture_region, capture_w, capture_h, history_arc)
    };

    // Immediate clipboard: user can paste the original capture right away
    copy_to_clipboard(main_hwnd, &pixels_for_annotate);

    // Show annotation editor WITHOUT holding the AppState lock
    let pixels_for_bake = pixels_for_annotate.clone();
    let main_hwnd_raw = main_hwnd.0 as isize;
    let callback: snip_ui::EditorCallback = Box::new(move |result| {
        let cb_hwnd = HWND(main_hwnd_raw as *mut std::ffi::c_void);
        match result {
            Some(layer) if !layer.is_empty() => {
                info!("Annotation complete: {} annotations — baking + saving final image", layer.annotations.len());
                let final_pixels = snip_annotate::render_annotations(&pixels_for_bake, &layer);
                save_and_copy(cb_hwnd, &final_pixels);
            }
            Some(_) => {
                info!("No annotations — saving original image");
                save_and_copy(cb_hwnd, &pixels_for_bake);
            }
            None => info!("Annotation cancelled"),
        }
    });

    // Live clipboard updates: re-bake and copy on each annotation change
    let on_change: snip_ui::EditorOnChange = Box::new(move |pixels, layer| {
        let cb_hwnd = HWND(main_hwnd_raw as *mut std::ffi::c_void);
        if layer.is_empty() {
            copy_to_clipboard(cb_hwnd, pixels);
        } else {
            let baked = snip_annotate::render_annotations(pixels, layer);
            copy_to_clipboard(cb_hwnd, &baked);
        }
    });

    if let Some(hwnd) = editor_hwnd {
        let editor = snip_ui::AnnotationEditor::from_hwnd(hwnd);
        if let Err(e) = editor.show(&pixels_for_annotate, callback, Some(on_change)) {
            error!("Failed to show annotation editor: {e}");
        }
    } else {
        info!("No persistent editor — creating new one");
        let _ = snip_ui::AnnotationEditor::new(&pixels_for_annotate, callback);
    }

    if let Some(ref history) = history_arc.history {
        let entry = ns_common::history::HistoryEntry::new(
            capture_mode,
            capture_region,
            String::new(),
            0,
            capture_w,
            capture_h,
        );
        history.lock().save_async(entry, pixels_for_history);
    }
}

fn do_region_capture(state: &Mutex<AppState>) {
    info!("do_region_capture: entering");
    let hotkey_received = Instant::now();
    let s = state.lock();
    let Some(ref overlay) = s.overlay else {
        error!("No overlay");
        return;
    };

    let state_ptr = state as *const Mutex<AppState>;

    overlay
        .show(CaptureMode::Region, Box::new(move |result| {
            let selection_complete = Instant::now();
            // SAFETY: state_ptr points to Mutex<AppState> heap-allocated via Box::into_raw
            // and stored in GWLP_USERDATA. It is freed only in WM_DESTROY, which runs after
            // the overlay callback completes (overlay is hidden before app teardown).
            let state = unsafe { &*state_ptr };
            match result {
                snip_overlay::OverlayResult::Region(region) => {
                    handle_region_selected(state, region, hotkey_received, selection_complete);
                }
                snip_overlay::OverlayResult::Cancelled => {
                    info!("Selection cancelled");
                }
                snip_overlay::OverlayResult::Window(_target_hwnd) => {
                    debug!("Window selected (unused): {_target_hwnd}");
                }
            }
        }))
        .unwrap_or_else(|e| error!("Failed to show overlay: {e}"));

    // Record overlay shown timestamp
    let overlay_shown = s.overlay.as_ref().and_then(|o| o.shown_at());
    if let Some(shown) = overlay_shown {
        let hotkey_to_overlay = shown.duration_since(hotkey_received);
        debug!("Hotkey to overlay shown: {:.2?}", hotkey_to_overlay);
    }
}

fn handle_region_selected(
    state: &Mutex<AppState>,
    region: ns_common::Rect,
    hotkey_received: Instant,
    selection_complete: Instant,
) {
    info!("handle_region_selected: entering");

    // Scope the lock: capture image + extract what we need, then drop the lock
    // BEFORE calling editor.show() to avoid any reentrancy deadlock.
    let (pixels_for_annotate, editor_hwnd, main_hwnd, pixels_for_history, capture_mode, capture_region, capture_w, capture_h, history_arc) = {
        let mut s = state.lock();

        // Get overlay shown timestamp before mutable borrow of capture engine
        let overlay_shown = s.overlay.as_ref().and_then(|o| o.shown_at());

        let Some(ref mut engine) = s.capture_engine else {
            error!("No capture engine");
            return;
        };

        let capture = match engine.capture_region(region) {
            Ok(c) => c,
            Err(e) => {
                error!("Region capture failed: {e}");
                return;
            }
        };

        let main_hwnd = s.main_hwnd;
        let pixels_for_annotate = capture.pixels.clone();
        let pixels_for_history = capture.pixels.clone();
        let capture_timing = capture.timing.clone();
        let capture_mode = capture.mode;
        let capture_region = capture.region;
        let capture_w = capture.pixels.width;
        let capture_h = capture.pixels.height;

        s.last_capture = Some(capture);

        log_capture_timing(
            "region",
            hotkey_received,
            overlay_shown,
            Some(selection_complete),
            capture_timing.as_ref(),
            None,
        );

        let editor_hwnd = s.annotation_editor.as_ref().map(|e| e.hwnd());
        let history_arc: Arc<SharedState> = s.shared.clone();

        // Lock dropped here
        (pixels_for_annotate, editor_hwnd, main_hwnd, pixels_for_history, capture_mode, capture_region, capture_w, capture_h, history_arc)
    };

    // Immediate clipboard: user can paste the original capture right away
    copy_to_clipboard(main_hwnd, &pixels_for_annotate);

    // Show annotation editor WITHOUT holding the AppState lock
    let pixels_for_bake = pixels_for_annotate.clone();
    let main_hwnd_raw = main_hwnd.0 as isize;
    let callback: snip_ui::EditorCallback = Box::new(move |result| {
        let cb_hwnd = HWND(main_hwnd_raw as *mut std::ffi::c_void);
        match result {
            Some(layer) if !layer.is_empty() => {
                info!("Annotation complete: {} annotations — baking + saving final image", layer.annotations.len());
                let final_pixels = snip_annotate::render_annotations(&pixels_for_bake, &layer);
                save_and_copy(cb_hwnd, &final_pixels);
            }
            Some(_) => {
                info!("No annotations — saving original image");
                save_and_copy(cb_hwnd, &pixels_for_bake);
            }
            None => info!("Annotation cancelled"),
        }
    });

    // Live clipboard updates: re-bake and copy on each annotation change
    let on_change: snip_ui::EditorOnChange = Box::new(move |pixels, layer| {
        let cb_hwnd = HWND(main_hwnd_raw as *mut std::ffi::c_void);
        if layer.is_empty() {
            copy_to_clipboard(cb_hwnd, pixels);
        } else {
            let baked = snip_annotate::render_annotations(pixels, layer);
            copy_to_clipboard(cb_hwnd, &baked);
        }
    });

    if let Some(hwnd) = editor_hwnd {
        let editor = snip_ui::AnnotationEditor::from_hwnd(hwnd);
        if let Err(e) = editor.show(&pixels_for_annotate, callback, Some(on_change)) {
            error!("Failed to show annotation editor: {e}");
        }
    } else {
        info!("No persistent editor — creating new one");
        let _ = snip_ui::AnnotationEditor::new(&pixels_for_annotate, callback);
    }

    if let Some(ref history) = history_arc.history {
        let entry = ns_common::history::HistoryEntry::new(
            capture_mode,
            capture_region,
            String::new(),
            0,
            capture_w,
            capture_h,
        );
        history.lock().save_async(entry, pixels_for_history);
    }
}

/// Save image to disk and copy both image + path to clipboard.
/// Clipboard is set FIRST (fast, 3-8ms), then PNG is saved on a background thread.
/// Update clipboard with an image (no disk save, no text).
/// Used for live clipboard updates during annotation.
fn copy_to_clipboard(hwnd: HWND, pixels: &ns_common::PixelBuffer) {
    if let Err(e) = snip_clipboard::set_image(hwnd, pixels) {
        error!("Failed to update clipboard: {e}");
    }
}

fn save_and_copy(hwnd: HWND, pixels: &ns_common::PixelBuffer) {
    let path = generate_save_path();
    let path_str = path.display().to_string();

    // 1. Set clipboard FIRST — user can paste immediately
    if let Err(e) = snip_clipboard::set_image_and_text(hwnd, pixels, &path_str) {
        error!("Failed to set clipboard image+text: {e}");
        // Fallback: try image-only clipboard
        if let Err(e2) = snip_clipboard::set_image(hwnd, pixels) {
            error!("Clipboard fallback also failed: {e2}");
        }
    }

    // 2. PNG save on background thread (50-200ms, non-blocking)
    let pixels_bg = pixels.clone(); // Arc clone, cheap
    std::thread::Builder::new()
        .name("png-save".into())
        .spawn(move || {
            let t0 = Instant::now();
            if let Err(e) = save_to_disk_at(&pixels_bg, &path) {
                error!("Background PNG save failed: {e}");
            } else {
                info!("PNG saved in {:.1}ms: {}", t0.elapsed().as_secs_f64() * 1000.0,
                    path.file_name().map(|f| f.to_string_lossy()).unwrap_or_default());
            }
        })
        .ok();
}

/// Generate the save path for a new screenshot.
fn generate_save_path() -> std::path::PathBuf {
    let dir = ns_common::paths::captures_dir();
    let _ = std::fs::create_dir_all(&dir);
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    dir.join(format!("screenshot_{timestamp}.png"))
}

/// Save a PixelBuffer to disk as PNG at the given path.
fn save_to_disk_at(pixels: &ns_common::PixelBuffer, path: &std::path::Path) -> anyhow::Result<()> {
    let w = pixels.width as usize;
    let h = pixels.height as usize;
    let src_stride = pixels.stride as usize;

    // Bulk BGRA→RGBA conversion
    let mut rgba_data = vec![0u8; w * h * 4];
    for row in 0..h {
        let src_row = &pixels.data[row * src_stride..row * src_stride + w * 4];
        let dst_row = &mut rgba_data[row * w * 4..(row + 1) * w * 4];
        for px in 0..w {
            let si = px * 4;
            dst_row[si]     = src_row[si + 2]; // R
            dst_row[si + 1] = src_row[si + 1]; // G
            dst_row[si + 2] = src_row[si];     // B
            dst_row[si + 3] = src_row[si + 3]; // A
        }
    }

    // Use png crate directly with fast compression
    let file = std::fs::File::create(path)?;
    let buf_writer = std::io::BufWriter::new(file);
    let encoder = image::codecs::png::PngEncoder::new_with_quality(
        buf_writer,
        image::codecs::png::CompressionType::Fast,
        image::codecs::png::FilterType::Sub,
    );
    use image::ImageEncoder;
    encoder.write_image(&rgba_data, pixels.width, pixels.height, image::ExtendedColorType::Rgba8)?;
    Ok(())
}

// ─── Timing instrumentation ──────────────────────────────────────────────────

fn log_capture_timing(
    mode: &str,
    hotkey_received: Instant,
    overlay_shown: Option<Instant>,
    selection_complete: Option<Instant>,
    capture_timing: Option<&CaptureTiming>,
    clipboard_announce: Option<std::time::Duration>,
) {
    let now = Instant::now();
    let total_ms = now.duration_since(hotkey_received).as_secs_f64() * 1000.0;

    let hotkey_to_overlay_ms = overlay_shown
        .map(|t| t.duration_since(hotkey_received).as_secs_f64() * 1000.0);

    let overlay_to_selection_ms = match (overlay_shown, selection_complete) {
        (Some(o), Some(s)) => Some(s.duration_since(o).as_secs_f64() * 1000.0),
        _ => None,
    };

    let (dxgi_ms, gpu_crop_ms, cpu_readback_ms, capture_total_ms) = match capture_timing {
        Some(t) => (
            t.dxgi_acquire.as_secs_f64() * 1000.0,
            t.gpu_crop.map(|d| d.as_secs_f64() * 1000.0),
            t.cpu_readback.as_secs_f64() * 1000.0,
            t.total.as_secs_f64() * 1000.0,
        ),
        None => (0.0, None, 0.0, 0.0),
    };

    let clipboard_ms = clipboard_announce.map(|d| d.as_secs_f64() * 1000.0);

    info!(
        mode,
        hotkey_to_overlay_ms = hotkey_to_overlay_ms.unwrap_or(-1.0),
        overlay_to_selection_ms = overlay_to_selection_ms.unwrap_or(-1.0),
        dxgi_acquire_ms = dxgi_ms,
        gpu_crop_ms = gpu_crop_ms.unwrap_or(-1.0),
        cpu_readback_ms = cpu_readback_ms,
        capture_total_ms = capture_total_ms,
        clipboard_announce_ms = clipboard_ms.unwrap_or(-1.0),
        total_ms = total_ms,
        "capture_timing"
    );
}

// ─── Window procedure ────────────────────────────────────────────────────────

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_HOTKEY => {
            let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Mutex<AppState>;
            if !state_ptr.is_null() {
                let state = &*state_ptr;
                if let Some(id) = HotkeyId::from_i32(wparam.0 as i32) {
                    handle_hotkey(state, id);
                }
            }
            LRESULT(0)
        }

        WM_RENDERFORMAT => {
            // Delayed rendering: provide actual clipboard data on demand
            let format = wparam.0 as u32;
            if format == CF_DIBV5.0 as u32 {
                let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Mutex<AppState>;
                if !state_ptr.is_null() {
                    let state = &*state_ptr;
                    let mut s = state.lock();

                    // Phase 6: If we have a pre-rendered DIB, use it instantly
                    if let Some(handle) = s.pre_rendered_dib.take() {
                        info!("WM_RENDERFORMAT: using pre-rendered DIB (0ms)");
                        let _ = SetClipboardData(format, Some(handle));
                    }
                    // Phase 3: If pending capture exists but no last_capture yet,
                    // complete readback synchronously (eager clipboard manager fallback)
                    else if s.pending_capture.is_some() && s.last_capture.is_none() {
                        info!("WM_RENDERFORMAT: completing deferred readback synchronously");
                        if let Some(pending) = s.pending_capture.take() {
                            if let Some(ref mut engine) = s.capture_engine {
                                match engine.complete_readback(pending) {
                                    Ok(result) => {
                                        match snip_clipboard::render_dibv5(&result.pixels) {
                                            Ok((handle, render_dur)) => {
                                                info!(
                                                    render_ms = render_dur.as_secs_f64() * 1000.0,
                                                    "WM_RENDERFORMAT: sync fallback render"
                                                );
                                                let _ = SetClipboardData(format, Some(handle));
                                            }
                                            Err(e) => error!("Failed to render clipboard data: {e}"),
                                        }
                                        s.last_capture = Some(result);
                                    }
                                    Err(e) => error!("Deferred readback failed in WM_RENDERFORMAT: {e}"),
                                }
                            }
                        }
                    }
                    // Normal path: render from last_capture
                    else if let Some(ref capture) = s.last_capture {
                        match snip_clipboard::render_dibv5(&capture.pixels) {
                            Ok((handle, render_dur)) => {
                                info!(
                                    render_ms = render_dur.as_secs_f64() * 1000.0,
                                    "WM_RENDERFORMAT: paste render"
                                );
                                let _ = SetClipboardData(format, Some(handle));
                            }
                            Err(e) => error!("Failed to render clipboard data: {e}"),
                        }
                    } else {
                        warn!("WM_RENDERFORMAT but no capture data available");
                    }
                }
            }
            LRESULT(0)
        }

        WM_RENDERALLFORMATS => {
            // Window being destroyed while owning delayed-render clipboard
            let _ = OpenClipboard(Some(hwnd));
            let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Mutex<AppState>;
            if !state_ptr.is_null() {
                let state = &*state_ptr;
                let mut s = state.lock();

                // Complete pending capture if needed
                if s.pending_capture.is_some() && s.last_capture.is_none() {
                    if let Some(pending) = s.pending_capture.take() {
                        if let Some(ref mut engine) = s.capture_engine {
                            if let Ok(result) = engine.complete_readback(pending) {
                                s.last_capture = Some(result);
                            }
                        }
                    }
                }

                if let Some(ref capture) = s.last_capture {
                    if let Ok((handle, _dur)) = snip_clipboard::render_dibv5(&capture.pixels) {
                        let _ = SetClipboardData(CF_DIBV5.0 as u32, Some(handle));
                    }
                }
            }
            let _ = CloseClipboard();
            LRESULT(0)
        }

        WM_APP_DEFERRED_READBACK => {
            // Deferred GPU readback: complete the pending capture
            let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Mutex<AppState>;
            if !state_ptr.is_null() {
                let state = &*state_ptr;
                let mut s = state.lock();
                if let Some(pending) = s.pending_capture.take() {
                    let mode = pending.mode;
                    let region = pending.region;
                    let width = pending.width;
                    let height = pending.height;

                    if let Some(ref mut engine) = s.capture_engine {
                        match engine.complete_readback(pending) {
                            Ok(result) => {
                                info!(
                                    "Deferred readback complete: {}x{}, timing={:?}",
                                    result.pixels.width, result.pixels.height, result.timing
                                );

                                let pixels_for_history = result.pixels.clone();

                                // Phase 6: Pre-render DIB so WM_RENDERFORMAT is instant
                                // Free old pre-rendered DIB if WM_RENDERFORMAT never consumed it
                                if let Some(old) = s.pre_rendered_dib.take() {
                                    snip_clipboard::free_dib_handle(old);
                                }
                                match snip_clipboard::render_dibv5(&result.pixels) {
                                    Ok((handle, render_dur)) => {
                                        info!(
                                            render_ms = render_dur.as_secs_f64() * 1000.0,
                                            "Pre-rendered DIB for instant paste"
                                        );
                                        s.pre_rendered_dib = Some(handle);
                                    }
                                    Err(e) => error!("Pre-render DIB failed: {e}"),
                                }

                                s.last_capture = Some(result);

                                // Save to history (non-blocking)
                                if let Some(ref history) = s.shared.history {
                                    let entry = ns_common::history::HistoryEntry::new(
                                        mode,
                                        region,
                                        String::new(),
                                        0,
                                        width,
                                        height,
                                    );
                                    history.lock().save_async(entry, pixels_for_history);
                                }
                            }
                            Err(e) => error!("Deferred readback failed: {e}"),
                        }
                    }
                }
            }
            LRESULT(0)
        }

        WM_APP_REREGISTER_HOTKEYS => {
            // IPC thread signals us to re-register hotkeys after config change
            info!("WM_APP_REREGISTER_HOTKEYS handler entered");
            let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Mutex<AppState>;
            if !state_ptr.is_null() {
                let state = &*state_ptr;
                let mut s = state.lock();
                let config = s.shared.config.lock().clone();
                info!(
                    "Re-registering hotkeys: region vk=0x{:X}, fullscreen vk=0x{:X}",
                    config.hotkeys.region.key,
                    config.hotkeys.fullscreen.key,
                );
                if let Some(ref mut hk) = s.hotkey_manager {
                    if let Err(e) = hk.register_all(&config.hotkeys) {
                        warn!("Failed to re-register hotkeys: {e}");
                    } else {
                        info!("Hotkeys re-registered from updated config");
                    }
                } else {
                    warn!("No hotkey_manager in AppState!");
                }
            } else {
                warn!("WM_APP_REREGISTER_HOTKEYS: state_ptr is null!");
            }
            LRESULT(0)
        }

        WM_APP => {
            // Tray-events thread signals us to update the Pause/Resume menu text
            let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Mutex<AppState>;
            if !state_ptr.is_null() {
                let state = &*state_ptr;
                let s = state.lock();
                let paused = *s.shared.paused.lock();
                if let Some(ref item) = s.tray_pause_item {
                    item.set_text(if paused { "Resume" } else { "Pause" });
                }
            }
            LRESULT(0)
        }

        WM_DESTROY => {
            let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Mutex<AppState>;
            if !state_ptr.is_null() {
                let state = Box::from_raw(state_ptr);
                let mut s = state.lock();
                if let Some(mut hk) = s.hotkey_manager.take() {
                    hk.unregister_all();
                }
                drop(s);
            }
            PostQuitMessage(0);
            LRESULT(0)
        }

        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
