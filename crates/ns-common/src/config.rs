use crate::hotkey::HotkeyCombination;
use serde::{Deserialize, Serialize};

/// Top-level application configuration, persisted as `config.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct NsConfig {
    pub hotkeys: HotkeyConfig,
    pub retention: RetentionConfig,
    pub behavior: BehaviorConfig,
}

impl Default for NsConfig {
    fn default() -> Self {
        Self {
            hotkeys: HotkeyConfig::default(),
            retention: RetentionConfig::default(),
            behavior: BehaviorConfig::default(),
        }
    }
}

impl NsConfig {
    /// Load config from disk, or return defaults if file is missing/corrupt.
    pub fn load() -> Self {
        let path = crate::paths::config_file();
        match std::fs::read_to_string(&path) {
            Ok(contents) => toml::from_str(&contents).unwrap_or_else(|e| {
                tracing::warn!("Failed to parse config: {e}, using defaults");
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    /// Save config to disk.
    pub fn save(&self) -> Result<(), crate::NsError> {
        let path = crate::paths::config_file();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let contents = toml::to_string_pretty(self)
            .map_err(|e| crate::NsError::Config(e.to_string()))?;
        std::fs::write(&path, contents)?;
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HotkeyConfig {
    pub region: HotkeyCombination,
    pub fullscreen: HotkeyCombination,
}

impl Default for HotkeyConfig {
    fn default() -> Self {
        use crate::hotkey::{HotkeyCombination, Modifier};
        Self {
            region: HotkeyCombination {
                modifiers: vec![Modifier::Ctrl, Modifier::Shift],
                key: '1' as u32,
            },
            fullscreen: HotkeyCombination {
                modifiers: vec![Modifier::Ctrl, Modifier::Shift],
                key: '2' as u32,
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RetentionConfig {
    /// Maximum age of captures in days. 0 = unlimited.
    pub max_age_days: u32,
    /// Maximum total size of captures in megabytes. 0 = unlimited.
    pub max_size_mb: u64,
}

impl Default for RetentionConfig {
    fn default() -> Self {
        Self {
            max_age_days: 0,
            max_size_mb: 100,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BehaviorConfig {
    /// Auto-copy to clipboard after this many seconds (0 = disabled).
    pub auto_copy_timeout_secs: u32,
    /// Start snipd on Windows login.
    pub start_on_login: bool,
}

impl Default for BehaviorConfig {
    fn default() -> Self {
        Self {
            auto_copy_timeout_secs: 4,
            start_on_login: true,
        }
    }
}
