//! snipui — Nano Snipper Settings & History UI.
//!
//! Built with iced. Connects to nanosnipper via named pipe IPC.

#![windows_subsystem = "windows"]

mod app;
mod ipc_client;
mod pages;
mod theme;
mod widgets;

use std::sync::OnceLock;
use tracing::info;

static INITIAL_PAGE: OnceLock<String> = OnceLock::new();

fn main() -> iced::Result {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_target(false)
        .init();

    // Parse --page=<settings|history|about> from command line
    let initial_page = std::env::args()
        .find(|a| a.starts_with("--page="))
        .and_then(|a| a.strip_prefix("--page=").map(String::from))
        .unwrap_or_default();

    // Store in a thread-local so NsApp::new() can read it
    INITIAL_PAGE.set(initial_page).ok();

    info!("snipui starting");

    iced::application(app::NsApp::title, app::NsApp::update, app::NsApp::view)
        .subscription(app::NsApp::subscription)
        .theme(app::NsApp::theme)
        .window(iced::window::Settings {
            size: iced::Size::new(820.0, 630.0),
            min_size: Some(iced::Size::new(640.0, 420.0)),
            position: iced::window::Position::Centered,
            visible: false, // Start hidden — shown after first render to avoid white flash
            icon: Some(
                iced::window::icon::from_rgba(
                    include_bytes!("../../../resources/tray_32x32.rgba").to_vec(),
                    32,
                    32,
                )
                .expect("valid icon data"),
            ),
            ..iced::window::Settings::default()
        })
        .default_font(theme::FONT_SEGOE)
        .run_with(app::NsApp::new)
}
