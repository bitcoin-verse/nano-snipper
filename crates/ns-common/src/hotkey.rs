use serde::{Deserialize, Serialize};

/// A keyboard modifier key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Modifier {
    Alt,
    Ctrl,
    Shift,
    Win,
}

/// A hotkey combination: one or more modifiers + a virtual key code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HotkeyCombination {
    pub modifiers: Vec<Modifier>,
    /// Virtual key code (e.g., `0x31` for '1', `0x4F` for 'O').
    pub key: u32,
}

impl HotkeyCombination {
    /// Convert modifiers to Win32 `MOD_*` flags for `RegisterHotKey`.
    pub fn win32_modifiers(&self) -> u32 {
        let mut flags = 0u32;
        // MOD_NOREPEAT = 0x4000
        flags |= 0x4000;
        for m in &self.modifiers {
            flags |= match m {
                Modifier::Alt => 0x0001,   // MOD_ALT
                Modifier::Ctrl => 0x0002,  // MOD_CONTROL
                Modifier::Shift => 0x0004, // MOD_SHIFT
                Modifier::Win => 0x0008,   // MOD_WIN
            };
        }
        flags
    }
}

/// Identifiers for the different hotkey actions (used as `id` in `RegisterHotKey`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum HotkeyId {
    Region = 1,
    Fullscreen = 2,
}

impl HotkeyId {
    pub fn from_i32(id: i32) -> Option<Self> {
        match id {
            1 => Some(Self::Region),
            2 => Some(Self::Fullscreen),
            _ => None,
        }
    }
}
