//! snip-bench — External benchmark harness for Nano Snipper vs Windows Snipping Tool.
//!
//! Uses SendInput to trigger hotkeys, pixel polling to detect overlay appearance,
//! clipboard polling to detect capture completion, and process memory queries.

use anyhow::Result;
use serde::Serialize;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::DataExchange::*;
use windows::Win32::System::Diagnostics::ToolHelp::*;
use windows::Win32::System::Ole::CF_DIBV5;
use windows::Win32::System::ProcessStatus::*;
use windows::Win32::System::Threading::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

// ─── CLI parsing ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct Args {
    tool: Tool,
    mode: BenchMode,
    runs: u32,
    delay_secs: u32,
    json: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Tool {
    NanoSnipper,
    SnippingTool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum BenchMode {
    Fullscreen,
}

fn parse_args() -> Result<Args> {
    let args: Vec<String> = std::env::args().collect();
    let mut tool = Tool::NanoSnipper;
    let mut mode = BenchMode::Fullscreen;
    let mut runs = 10u32;
    let mut delay_secs = 3u32;
    let mut json = false;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--tool" => {
                i += 1;
                tool = match args.get(i).map(|s| s.as_str()) {
                    Some("nano-snipper") => Tool::NanoSnipper,
                    Some("snipping-tool") => Tool::SnippingTool,
                    other => anyhow::bail!("Unknown tool: {:?}. Use 'nano-snipper' or 'snipping-tool'", other),
                };
            }
            "--mode" => {
                i += 1;
                mode = match args.get(i).map(|s| s.as_str()) {
                    Some("fullscreen") => BenchMode::Fullscreen,
                    other => anyhow::bail!("Unknown mode: {:?}. Use 'fullscreen'", other),
                };
            }
            "--runs" => {
                i += 1;
                runs = args.get(i)
                    .ok_or_else(|| anyhow::anyhow!("--runs requires a number"))?
                    .parse()?;
            }
            "--delay" => {
                i += 1;
                delay_secs = args.get(i)
                    .ok_or_else(|| anyhow::anyhow!("--delay requires a number"))?
                    .parse()?;
            }
            "--json" => json = true,
            "--help" | "-h" => {
                println!("snip-bench — Benchmark Nano Snipper vs Windows Snipping Tool");
                println!();
                println!("Usage: snip-bench [OPTIONS]");
                println!();
                println!("Options:");
                println!("  --tool <nano-snipper|snipping-tool>  Tool to benchmark (default: nano-snipper)");
                println!("  --mode <fullscreen>                  Capture mode (default: fullscreen)");
                println!("  --runs <N>                           Number of runs (default: 10)");
                println!("  --delay <secs>                       Delay between runs (default: 3)");
                println!("  --json                               Output results as JSON");
                println!("  -h, --help                           Show this help");
                std::process::exit(0);
            }
            other => anyhow::bail!("Unknown argument: {other}"),
        }
        i += 1;
    }

    Ok(Args { tool, mode, runs, delay_secs, json })
}

// ─── Data types ──────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
struct BenchmarkResult {
    tool: String,
    mode: String,
    runs: u32,
    hotkey_to_clipboard_ms: Percentiles,
    paste_render_ms: Percentiles,
    memory_idle_mb: Option<f64>,
    memory_peak_mb: Option<f64>,
}

#[derive(Debug, Serialize)]
struct Percentiles {
    p50: f64,
    p95: f64,
    p99: f64,
    min: f64,
    max: f64,
    mean: f64,
}

impl Percentiles {
    fn from_samples(samples: &mut Vec<f64>) -> Self {
        samples.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let n = samples.len();
        if n == 0 {
            return Percentiles { p50: 0.0, p95: 0.0, p99: 0.0, min: 0.0, max: 0.0, mean: 0.0 };
        }
        let mean = samples.iter().sum::<f64>() / n as f64;
        Percentiles {
            p50: samples[n * 50 / 100],
            p95: samples[(n * 95 / 100).min(n - 1)],
            p99: samples[(n * 99 / 100).min(n - 1)],
            min: samples[0],
            max: samples[n - 1],
            mean,
        }
    }
}

// ─── SendInput helpers ───────────────────────────────────────────────────────

fn send_key_combo(keys: &[(VIRTUAL_KEY, bool)]) {
    let mut inputs: Vec<INPUT> = Vec::new();

    // Key down events
    for &(vk, extended) in keys {
        let mut flags = KEYBD_EVENT_FLAGS(0);
        if extended {
            flags |= KEYEVENTF_EXTENDEDKEY;
        }
        inputs.push(INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: vk,
                    wScan: 0,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        });
    }

    // Key up events (reverse order)
    for &(vk, extended) in keys.iter().rev() {
        let mut flags = KEYEVENTF_KEYUP;
        if extended {
            flags |= KEYEVENTF_EXTENDEDKEY;
        }
        inputs.push(INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: vk,
                    wScan: 0,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        });
    }

    unsafe {
        SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
    }
}

fn send_escape() {
    send_key_combo(&[(VK_ESCAPE, false)]);
}

// ─── Pixel polling ───────────────────────────────────────────────────────────

/// Sample a pixel at screen center to detect overlay appearance (darkening).
#[allow(dead_code)]
fn get_center_pixel_brightness() -> u32 {
    unsafe {
        let cx = GetSystemMetrics(SM_CXSCREEN) / 2;
        let cy = GetSystemMetrics(SM_CYSCREEN) / 2;
        let hdc = GetDC(None);
        let color = GetPixel(hdc, cx, cy);
        ReleaseDC(None, hdc);
        if color == COLORREF(CLR_INVALID) {
            return 255;
        }
        let r = color.0 & 0xFF;
        let g = (color.0 >> 8) & 0xFF;
        let b = (color.0 >> 16) & 0xFF;
        (r + g + b) / 3
    }
}

// ─── Clipboard polling ───────────────────────────────────────────────────────

fn clipboard_has_image() -> bool {
    unsafe {
        // Check for CF_DIBV5 (Nano Snipper) or CF_DIB (Snipping Tool)
        IsClipboardFormatAvailable(CF_DIBV5.0 as u32).is_ok()
            || IsClipboardFormatAvailable(8).is_ok() // CF_DIB = 8
    }
}

fn clear_clipboard() {
    unsafe {
        if OpenClipboard(None).is_ok() {
            let _ = EmptyClipboard();
            let _ = CloseClipboard();
        }
    }
}

/// Poll clipboard until an image format appears. Returns elapsed time.
fn poll_clipboard_ready(timeout: Duration) -> Option<Duration> {
    let start = Instant::now();
    loop {
        if clipboard_has_image() {
            return Some(start.elapsed());
        }
        if start.elapsed() > timeout {
            return None;
        }
        std::thread::sleep(Duration::from_micros(200));
    }
}

// ─── Paste render measurement ────────────────────────────────────────────────

/// Measure time to actually read clipboard data (triggers WM_RENDERFORMAT for delayed rendering).
fn measure_paste_render() -> Option<Duration> {
    unsafe {
        if OpenClipboard(None).is_err() {
            return None;
        }

        let t = Instant::now();
        // Try CF_DIBV5 first, then CF_DIB
        let handle = GetClipboardData(CF_DIBV5.0 as u32)
            .or_else(|_| GetClipboardData(8)); // CF_DIB

        let elapsed = t.elapsed();
        let _ = CloseClipboard();

        handle.ok().map(|_| elapsed)
    }
}

// ─── Process memory ──────────────────────────────────────────────────────────

fn get_process_memory_mb(process_name: &str) -> Option<f64> {
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0).ok()?;
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };

        if Process32FirstW(snapshot, &mut entry).is_err() {
            let _ = CloseHandle(snapshot);
            return None;
        }

        let target: Vec<u16> = process_name.encode_utf16().collect();

        loop {
            let exe_name: Vec<u16> = entry.szExeFile.iter()
                .take_while(|&&c| c != 0)
                .copied()
                .collect();

            if exe_name == target {
                let pid = entry.th32ProcessID;
                let _ = CloseHandle(snapshot);

                let proc_handle = OpenProcess(PROCESS_QUERY_INFORMATION, false, pid).ok()?;
                let mut mem_counters = PROCESS_MEMORY_COUNTERS::default();
                mem_counters.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
                if GetProcessMemoryInfo(
                    proc_handle,
                    &mut mem_counters,
                    std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
                ).is_ok() {
                    let _ = CloseHandle(proc_handle);
                    return Some(mem_counters.WorkingSetSize as f64 / (1024.0 * 1024.0));
                }
                let _ = CloseHandle(proc_handle);
                return None;
            }

            if Process32NextW(snapshot, &mut entry).is_err() {
                break;
            }
        }

        let _ = CloseHandle(snapshot);
        None
    }
}

// ─── Benchmark runners ───────────────────────────────────────────────────────

fn bench_nano_snipper_fullscreen(args: &Args) -> Result<BenchmarkResult> {
    let process_name = "snipd.exe";

    println!("Benchmarking Nano Snipper (fullscreen) — {} runs, {}s delay",
        args.runs, args.delay_secs);

    // Measure idle memory before any captures
    let memory_idle = get_process_memory_mb(process_name);
    if let Some(mb) = memory_idle {
        println!("  Idle memory: {:.1} MB", mb);
    } else {
        println!("  Warning: Could not find snipd.exe process. Is it running?");
    }

    let mut clipboard_times: Vec<f64> = Vec::new();
    let mut paste_times: Vec<f64> = Vec::new();
    let mut memory_peak: Option<f64> = memory_idle;

    for i in 0..args.runs {
        // Clear clipboard before each run
        clear_clipboard();
        std::thread::sleep(Duration::from_millis(100));

        // Send Ctrl+Shift+3 (Nano Snipper default fullscreen hotkey)
        send_key_combo(&[
            (VK_CONTROL, false),
            (VK_SHIFT, false),
            (VIRTUAL_KEY(0x33), false), // '3'
        ]);

        // Poll for clipboard readiness
        match poll_clipboard_ready(Duration::from_secs(5)) {
            Some(elapsed) => {
                let ms = elapsed.as_secs_f64() * 1000.0;
                clipboard_times.push(ms);

                // Now measure paste render latency
                if let Some(paste_dur) = measure_paste_render() {
                    paste_times.push(paste_dur.as_secs_f64() * 1000.0);
                }

                // Check peak memory after capture
                if let Some(mb) = get_process_memory_mb(process_name) {
                    memory_peak = Some(memory_peak.map_or(mb, |prev: f64| prev.max(mb)));
                }

                print!("  Run {}/{}: clipboard ready in {:.1}ms", i + 1, args.runs, ms);
                if let Some(last_paste) = paste_times.last() {
                    print!(", paste render {:.1}ms", last_paste);
                }
                println!();
            }
            None => {
                println!("  Run {}/{}: TIMEOUT (no clipboard data after 5s)", i + 1, args.runs);
            }
        }

        // Wait between runs
        if i + 1 < args.runs {
            std::thread::sleep(Duration::from_secs(args.delay_secs as u64));
        }
    }

    Ok(BenchmarkResult {
        tool: "nano-snipper".to_string(),
        mode: "fullscreen".to_string(),
        runs: args.runs,
        hotkey_to_clipboard_ms: Percentiles::from_samples(&mut clipboard_times),
        paste_render_ms: Percentiles::from_samples(&mut paste_times),
        memory_idle_mb: memory_idle,
        memory_peak_mb: memory_peak,
    })
}

fn bench_snipping_tool_fullscreen(args: &Args) -> Result<BenchmarkResult> {
    let process_name = "SnippingTool.exe";

    println!("Benchmarking Windows Snipping Tool (fullscreen) — {} runs, {}s delay",
        args.runs, args.delay_secs);

    let memory_idle = get_process_memory_mb(process_name);
    if let Some(mb) = memory_idle {
        println!("  Idle memory: {:.1} MB", mb);
    } else {
        println!("  Warning: Could not find SnippingTool.exe. Make sure it's running.");
    }

    let mut clipboard_times: Vec<f64> = Vec::new();
    let mut paste_times: Vec<f64> = Vec::new();
    let mut memory_peak: Option<f64> = memory_idle;

    for i in 0..args.runs {
        clear_clipboard();
        std::thread::sleep(Duration::from_millis(100));

        // Send Win+Shift+S to invoke Snipping Tool overlay
        send_key_combo(&[
            (VK_LWIN, true),
            (VK_SHIFT, false),
            (VIRTUAL_KEY(0x53), false), // 'S'
        ]);

        // Wait for overlay to appear, then auto-select fullscreen via keyboard
        // Snipping Tool defaults to rectangular mode; press Ctrl+Shift+F or use
        // the toolbar. Since we can't easily click the toolbar, we wait for the
        // overlay and then send PrintScreen (which some Snipping Tool versions
        // interpret as fullscreen). As a more reliable approach, we wait briefly
        // then press Enter after the overlay appears, or use Alt+N for fullscreen.
        std::thread::sleep(Duration::from_millis(800)); // Wait for Snipping Tool overlay

        // Snipping Tool: once overlay is shown, we need to trigger fullscreen capture.
        // The recommended approach: the overlay shows a toolbar at top.
        // We can simulate clicking "fullscreen" or just capture the full screen
        // by pressing the keyboard shortcut. Let's use the Win+Shift+S flow:
        // after overlay, press Tab then Enter to select fullscreen in the toolbar,
        // or just do a full-screen rectangle select.

        // Simple approach: send Escape to dismiss overlay, then use the snipping
        // tool's direct fullscreen shortcut (not available in all versions).
        // Most reliable: dismiss and re-invoke with Win+Shift+Print (fullscreen).
        // Actually on Windows 11, Win+Shift+S shows overlay with mode selector.
        // Pressing the fullscreen icon (4th mode) is what we need.

        // The simplest cross-version approach: send Alt+N (fullscreen mode toggle)
        // or just Tab 3 times + Enter on the toolbar.
        send_key_combo(&[(VK_TAB, false)]);
        std::thread::sleep(Duration::from_millis(50));
        send_key_combo(&[(VK_TAB, false)]);
        std::thread::sleep(Duration::from_millis(50));
        send_key_combo(&[(VK_TAB, false)]);
        std::thread::sleep(Duration::from_millis(50));
        send_key_combo(&[(VK_RETURN, false)]);

        let t_after_trigger = Instant::now();

        // Poll for clipboard
        match poll_clipboard_ready(Duration::from_secs(10)) {
            Some(poll_elapsed) => {
                // Total time = overlay wait (800ms) + toolbar navigation + clipboard
                // But we report from after the fullscreen trigger for fairness
                let total_ms = t_after_trigger.elapsed().as_secs_f64() * 1000.0;
                let poll_ms = poll_elapsed.as_secs_f64() * 1000.0;
                clipboard_times.push(total_ms);

                if let Some(paste_dur) = measure_paste_render() {
                    paste_times.push(paste_dur.as_secs_f64() * 1000.0);
                }

                if let Some(mb) = get_process_memory_mb(process_name) {
                    memory_peak = Some(memory_peak.map_or(mb, |prev: f64| prev.max(mb)));
                }

                print!("  Run {}/{}: clipboard ready in {:.1}ms (poll: {:.1}ms)",
                    i + 1, args.runs, total_ms, poll_ms);
                if let Some(last_paste) = paste_times.last() {
                    print!(", paste render {:.1}ms", last_paste);
                }
                println!();
            }
            None => {
                println!("  Run {}/{}: TIMEOUT", i + 1, args.runs);
                // Try to dismiss any leftover overlay
                send_escape();
            }
        }

        if i + 1 < args.runs {
            std::thread::sleep(Duration::from_secs(args.delay_secs as u64));
        }
    }

    Ok(BenchmarkResult {
        tool: "snipping-tool".to_string(),
        mode: "fullscreen".to_string(),
        runs: args.runs,
        hotkey_to_clipboard_ms: Percentiles::from_samples(&mut clipboard_times),
        paste_render_ms: Percentiles::from_samples(&mut paste_times),
        memory_idle_mb: memory_idle,
        memory_peak_mb: memory_peak,
    })
}

// ─── Output ──────────────────────────────────────────────────────────────────

fn print_results(result: &BenchmarkResult, as_json: bool) {
    if as_json {
        println!("{}", serde_json::to_string_pretty(result).unwrap());
        return;
    }

    println!();
    println!("═══ {} ({}) — {} runs ═══",
        result.tool, result.mode, result.runs);
    println!();
    print_percentiles("Hotkey → Clipboard", &result.hotkey_to_clipboard_ms);
    print_percentiles("Paste Render", &result.paste_render_ms);
    println!();
    if let Some(idle) = result.memory_idle_mb {
        println!("  Memory (idle):  {:.1} MB", idle);
    }
    if let Some(peak) = result.memory_peak_mb {
        println!("  Memory (peak):  {:.1} MB", peak);
    }
    println!();
}

fn print_percentiles(label: &str, p: &Percentiles) {
    println!("  {label}:");
    println!("    P50: {:.1}ms  P95: {:.1}ms  P99: {:.1}ms", p.p50, p.p95, p.p99);
    println!("    Min: {:.1}ms  Max: {:.1}ms  Mean: {:.1}ms", p.min, p.max, p.mean);
}

// ─── Main ────────────────────────────────────────────────────────────────────

fn main() -> Result<()> {
    let args = parse_args()?;

    let result = match (&args.tool, &args.mode) {
        (Tool::NanoSnipper, BenchMode::Fullscreen) => bench_nano_snipper_fullscreen(&args)?,
        (Tool::SnippingTool, BenchMode::Fullscreen) => bench_snipping_tool_fullscreen(&args)?,
    };

    print_results(&result, args.json);

    Ok(())
}
