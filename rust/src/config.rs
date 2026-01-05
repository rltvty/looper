//! Configuration persistence for the MIDI looper.
//!
//! Saves and loads looper configuration to/from YAML files.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Configuration for a single sequence slot.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SlotConfig {
    /// Path to the loop file (relative to data/out or absolute)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub loop_file: Option<String>,
    /// Repeat count before advancing to next slot
    #[serde(default = "default_repeat_count")]
    pub repeat_count: u32,
    /// Next slot to play (None = stop)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_slot: Option<char>,
}

fn default_repeat_count() -> u32 {
    1
}

/// Complete looper configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LooperConfig {
    /// MIDI output device name
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_device: Option<String>,
    /// MIDI output channel (1-16, stored as 1-indexed for human readability)
    #[serde(default = "default_channel")]
    pub output_channel: u8,
    /// Use 0-indexed countdown display (programmer-friendly)
    /// When false (default), countdown shows 1.1 as last position
    /// When true, countdown shows 0.0 as last position
    #[serde(default)]
    pub zero_indexed_countdown: bool,
    /// Slot configurations, keyed by slot letter (A-Z)
    #[serde(default)]
    pub slots: BTreeMap<char, SlotConfig>,
}

fn default_channel() -> u8 {
    1 // 1-indexed for YAML readability
}

impl LooperConfig {
    /// Get the default config file path.
    pub fn default_path() -> PathBuf {
        let project_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        project_dir.join("looper_config.yaml")
    }

    /// Load configuration from a YAML file.
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, String> {
        let path = path.as_ref();
        if !path.exists() {
            return Ok(Self::default());
        }

        let content = fs::read_to_string(path)
            .map_err(|e| format!("Failed to read config file: {}", e))?;

        serde_yaml::from_str(&content)
            .map_err(|e| format!("Failed to parse config YAML: {}", e))
    }

    /// Save configuration to a YAML file.
    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<(), String> {
        let content = serde_yaml::to_string(self)
            .map_err(|e| format!("Failed to serialize config: {}", e))?;

        fs::write(path, content)
            .map_err(|e| format!("Failed to write config file: {}", e))
    }

    /// Get slot config, creating default if not present.
    pub fn get_slot(&self, slot_id: char) -> SlotConfig {
        self.slots.get(&slot_id).cloned().unwrap_or_default()
    }

    /// Set slot config.
    pub fn set_slot(&mut self, slot_id: char, config: SlotConfig) {
        // Only store non-default slots to keep the file clean
        if config.loop_file.is_some() || config.repeat_count != 1 || config.next_slot.is_some() {
            self.slots.insert(slot_id, config);
        } else {
            self.slots.remove(&slot_id);
        }
    }
}
