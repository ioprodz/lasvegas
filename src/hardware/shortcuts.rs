//! Performance-page shortcut slots: a 9-slot mapping from slot index (0..8)
//! to an animation name. Persisted to `shortcuts.conf` as a single comma-
//! separated line — empty fields = unassigned. Auto-saved on every set.

use std::fs;
use std::path::Path;

const SHORTCUTS_FILE: &str = "shortcuts.conf";
pub const SLOT_COUNT: usize = 9;

#[derive(Debug, Clone)]
pub struct Shortcuts {
    /// `None` = empty slot; `Some(name)` = animation key
    pub slots: [Option<String>; SLOT_COUNT],
}

impl Default for Shortcuts {
    fn default() -> Self {
        const NONE: Option<String> = None;
        Self {
            slots: [NONE; SLOT_COUNT],
        }
    }
}

impl Shortcuts {
    /// Parse from a wire/file CSV form: `name1,,name3,name4,,,,,`
    /// (always 9 fields; trailing commas may be implicit).
    pub fn from_csv(s: &str) -> Self {
        let mut sc = Self::default();
        for (i, part) in s.split(',').take(SLOT_COUNT).enumerate() {
            let trimmed = part.trim();
            if !trimmed.is_empty() {
                sc.slots[i] = Some(trimmed.to_string());
            }
        }
        sc
    }

    /// Wire/file CSV serialization. Always emits exactly `SLOT_COUNT-1` commas.
    pub fn to_csv(&self) -> String {
        self.slots
            .iter()
            .map(|s| s.as_deref().unwrap_or(""))
            .collect::<Vec<_>>()
            .join(",")
    }

    pub fn load() -> Self {
        let path = Path::new(SHORTCUTS_FILE);
        if !path.exists() {
            return Self::default();
        }
        match fs::read_to_string(path) {
            Ok(contents) => Self::from_csv(contents.trim()),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self) -> Result<(), String> {
        fs::write(SHORTCUTS_FILE, self.to_csv()).map_err(|e| e.to_string())
    }
}
