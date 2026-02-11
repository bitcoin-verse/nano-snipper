//! Hotkey registration using Win32 RegisterHotKey.

use anyhow::Result;
use ns_common::config::HotkeyConfig;
use ns_common::hotkey::{HotkeyCombination, HotkeyId};
use tracing::{info, warn};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Input::KeyboardAndMouse::*;

/// Manages hotkey registration and cleanup.
pub struct HotkeyManager {
    hwnd: HWND,
    registered: Vec<HotkeyId>,
}

impl HotkeyManager {
    pub fn new(hwnd: HWND) -> Self {
        Self {
            hwnd,
            registered: Vec::new(),
        }
    }

    /// Register all hotkeys from config.
    pub fn register_all(&mut self, config: &HotkeyConfig) -> Result<()> {
        info!(
            "register_all: Region vk=0x{:X} mods=0x{:X}, Fullscreen vk=0x{:X} mods=0x{:X}",
            config.region.key, config.region.win32_modifiers(),
            config.fullscreen.key, config.fullscreen.win32_modifiers(),
        );
        self.unregister_all();

        self.register(HotkeyId::Region, &config.region)?;
        self.register(HotkeyId::Fullscreen, &config.fullscreen)?;

        info!("All hotkeys registered successfully");
        Ok(())
    }

    /// Register a single hotkey.
    fn register(&mut self, id: HotkeyId, combo: &HotkeyCombination) -> Result<()> {
        let mods = HOT_KEY_MODIFIERS(combo.win32_modifiers());
        let result = unsafe { RegisterHotKey(Some(self.hwnd), id as i32, mods, combo.key) };

        if result.is_err() {
            warn!(
                "Failed to register hotkey {:?} (vk=0x{:X}), may conflict with another app",
                id, combo.key
            );
            return Err(anyhow::anyhow!("RegisterHotKey failed for {:?}", id));
        }

        info!("Registered hotkey {:?}: mods=0x{:X} vk=0x{:X}", id, combo.win32_modifiers(), combo.key);
        self.registered.push(id);
        Ok(())
    }

    /// Unregister all previously registered hotkeys.
    pub fn unregister_all(&mut self) {
        for id in self.registered.drain(..) {
            let result = unsafe { UnregisterHotKey(Some(self.hwnd), id as i32) };
            if result.is_err() {
                warn!("UnregisterHotKey failed for {:?} (id={})", id, id as i32);
            } else {
                info!("Unregistered hotkey {:?} (id={})", id, id as i32);
            }
        }
    }
}

impl Drop for HotkeyManager {
    fn drop(&mut self) {
        self.unregister_all();
    }
}
