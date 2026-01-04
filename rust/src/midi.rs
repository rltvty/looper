//! MIDI protocol constants and utilities.
//!
//! This module defines the MIDI real-time messages used for clock synchronization
//! and transport control. These follow the standard MIDI 1.0 specification.

use midir::{MidiOutput, MidiOutputConnection};

/// MIDI Clock tick - sent 24 times per quarter note (24 ppqn)
pub const MIDI_CLOCK: u8 = 0xF8;

/// MIDI Start - resets position to beginning and starts playback
pub const MIDI_START: u8 = 0xFA;

/// MIDI Continue - resumes playback from current position
pub const MIDI_CONTINUE: u8 = 0xFB;

/// MIDI Stop - stops playback, maintains current position
pub const MIDI_STOP: u8 = 0xFC;

/// Number of MIDI clock pulses per quarter note (beat)
pub const CLOCKS_PER_BEAT: u64 = 24;

/// MIDI Control Change status byte mask (0xBn where n is channel)
pub const MIDI_CC_MASK: u8 = 0xB0;

/// CC number for screenshot trigger
pub const CC_SCREENSHOT: u8 = 119;

/// Check if a MIDI message is CC 119 with value 127 (screenshot trigger)
pub fn is_screenshot_trigger(message: &[u8]) -> bool {
    message.len() == 3
        && (message[0] & 0xF0) == MIDI_CC_MASK
        && message[1] == CC_SCREENSHOT
        && message[2] == 127
}

/// Beats per bar (assuming 4/4 time signature)
pub const BEATS_PER_BAR: u64 = 4;

/// Scan for available MIDI output ports.
/// Returns a list of (port_name, display_name) tuples.
pub fn scan_output_ports() -> Vec<String> {
    let midi_out = match MidiOutput::new("looper-scan") {
        Ok(m) => m,
        Err(_) => return Vec::new(),
    };

    midi_out
        .ports()
        .iter()
        .filter_map(|p| midi_out.port_name(p).ok())
        .collect()
}

/// Wrapper for MIDI output connection.
pub struct MidiOut {
    connection: MidiOutputConnection,
    pub port_name: String,
}

impl MidiOut {
    /// Create a new MIDI output, preferring IAC Driver on macOS.
    pub fn new() -> Result<Self, String> {
        Self::connect_to_port(None)
    }

    /// Connect to a specific port by name, or IAC Driver if None.
    pub fn connect_to_port(port_name: Option<&str>) -> Result<Self, String> {
        let midi_out = MidiOutput::new("looper-out")
            .map_err(|e| format!("Failed to create MIDI output: {}", e))?;

        let ports = midi_out.ports();
        if ports.is_empty() {
            return Err("No MIDI output ports found".to_string());
        }

        // Find the requested port, or fall back to IAC Driver, or first port
        let port_idx = if let Some(name) = port_name {
            ports
                .iter()
                .position(|p| {
                    midi_out
                        .port_name(p)
                        .map(|n| n == name)
                        .unwrap_or(false)
                })
                .unwrap_or(0)
        } else {
            // Look for IAC Driver or use first port
            ports
                .iter()
                .position(|p| {
                    midi_out
                        .port_name(p)
                        .map(|n| n.contains("IAC"))
                        .unwrap_or(false)
                })
                .unwrap_or(0)
        };

        let port = &ports[port_idx];
        let actual_port_name = midi_out
            .port_name(port)
            .unwrap_or_else(|_| "Unknown".to_string());

        let connection = midi_out
            .connect(port, "looper-out")
            .map_err(|e| format!("Failed to connect MIDI output: {}", e))?;

        println!("MIDI Output connected to: {}", actual_port_name);
        Ok(Self {
            connection,
            port_name: actual_port_name,
        })
    }

    /// Send a MIDI message.
    pub fn send(&mut self, message: &[u8]) -> Result<(), String> {
        self.connection
            .send(message)
            .map_err(|e| format!("Failed to send MIDI: {}", e))
    }

    /// Send MIDI Start message.
    pub fn send_start(&mut self) -> Result<(), String> {
        self.send(&[MIDI_START])
    }

    /// Send MIDI Stop message.
    pub fn send_stop(&mut self) -> Result<(), String> {
        self.send(&[MIDI_STOP])
    }
}
