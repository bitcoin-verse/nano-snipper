use std::path::PathBuf;

const APP_DIR_NAME: &str = "NanoSnipper";

/// Returns `%LOCALAPPDATA%\NanoSnipper\`.
pub fn app_data_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(APP_DIR_NAME)
}

/// Returns `%LOCALAPPDATA%\NanoSnipper\config.toml`.
pub fn config_file() -> PathBuf {
    app_data_dir().join("config.toml")
}

/// Returns `%LOCALAPPDATA%\NanoSnipper\history.db`.
pub fn history_db() -> PathBuf {
    app_data_dir().join("history.db")
}

/// Returns `%LOCALAPPDATA%\NanoSnipper\captures\`.
pub fn captures_dir() -> PathBuf {
    app_data_dir().join("captures")
}

/// Returns `%LOCALAPPDATA%\NanoSnipper\logs\`.
pub fn logs_dir() -> PathBuf {
    app_data_dir().join("logs")
}
