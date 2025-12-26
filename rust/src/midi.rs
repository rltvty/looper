//! MIDI protocol constants and utilities.
//!
//! This module defines the MIDI real-time messages used for clock synchronization
//! and transport control. These follow the standard MIDI 1.0 specification.

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

/// Beats per bar (assuming 4/4 time signature)
pub const BEATS_PER_BAR: u64 = 4;
