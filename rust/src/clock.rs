//! MIDI clock synchronization and transport state management.
//!
//! This module handles:
//! - Tracking playback state (running/stopped)
//! - Counting clock pulses to determine bar/beat position
//! - Calculating BPM from incoming clock pulses using a rolling average
//!
//! # Thread Safety
//! [`ClockState`] is designed to be shared between a MIDI input thread
//! and the GUI thread. All state is wrapped in atomic types or mutexes.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use crate::midi::{
    BEATS_PER_BAR, CLOCKS_PER_BEAT, MIDI_CLOCK, MIDI_CONTINUE, MIDI_START, MIDI_STOP,
};

/// Size of the rolling window for BPM calculation (1 bar = 96 clocks at 24 ppqn in 4/4)
const BPM_WINDOW_CLOCKS: usize = 96;

/// Ring buffer for storing clock pulse timestamps.
///
/// Used to calculate a rolling average BPM over the most recent bar of music.
/// The buffer starts calculating BPM immediately with partial data, becoming
/// more accurate as it fills.
pub struct ClockTimeBuffer {
    times: [Option<Instant>; BPM_WINDOW_CLOCKS],
    index: usize,
    count: usize,
}

impl ClockTimeBuffer {
    pub fn new() -> Self {
        Self {
            times: [None; BPM_WINDOW_CLOCKS],
            index: 0,
            count: 0,
        }
    }

    /// Add a timestamp to the buffer, returning the oldest timestamp and sample count.
    pub fn push(&mut self, time: Instant) -> (Option<Instant>, usize) {
        let oldest = self.times[self.index];
        self.times[self.index] = Some(time);
        self.index = (self.index + 1) % BPM_WINDOW_CLOCKS;
        if self.count < BPM_WINDOW_CLOCKS {
            self.count += 1;
        }
        (oldest, self.count)
    }

    /// Get the oldest available timestamp for partial buffer calculation.
    pub fn get_oldest(&self) -> Option<(Instant, usize)> {
        if self.count == 0 {
            return None;
        }
        if self.count < BPM_WINDOW_CLOCKS {
            // Buffer not full yet - oldest is at index 0
            self.times[0].map(|t| (t, self.count))
        } else {
            // Buffer is full - oldest is at current index (about to be overwritten)
            self.times[self.index].map(|t| (t, self.count))
        }
    }

    pub fn clear(&mut self) {
        self.times = [None; BPM_WINDOW_CLOCKS];
        self.index = 0;
        self.count = 0;
    }
}

impl Default for ClockTimeBuffer {
    fn default() -> Self {
        Self::new()
    }
}

/// Shared clock state between MIDI thread and GUI.
///
/// This struct tracks:
/// - Whether playback is running
/// - Whether explicit transport messages have been received
/// - Current position (clock count)
/// - Calculated BPM
///
/// # Auto-start Behavior
/// If the application connects to a MIDI source that's already sending clock
/// pulses (but hasn't sent a START message), it will auto-start. Once any
/// explicit transport message (START/STOP/CONTINUE) is received, auto-start
/// is disabled and only explicit transport controls playback.
#[derive(Clone)]
pub struct ClockState {
    running: Arc<AtomicBool>,
    seen_transport: Arc<AtomicBool>,
    clock_count: Arc<AtomicU64>,
    bpm_x100: Arc<AtomicU64>,
    clock_times: Arc<std::sync::Mutex<ClockTimeBuffer>>,
}

impl ClockState {
    pub fn new() -> Self {
        Self {
            running: Arc::new(AtomicBool::new(false)),
            seen_transport: Arc::new(AtomicBool::new(false)),
            clock_count: Arc::new(AtomicU64::new(0)),
            bpm_x100: Arc::new(AtomicU64::new(0)),
            clock_times: Arc::new(std::sync::Mutex::new(ClockTimeBuffer::new())),
        }
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    pub fn get_clock_count(&self) -> u64 {
        self.clock_count.load(Ordering::SeqCst)
    }

    /// Get current position as (bar, beat) tuple, both 1-indexed.
    pub fn get_position(&self) -> (u64, u64) {
        let count = self.clock_count.load(Ordering::SeqCst);
        let beats = count / CLOCKS_PER_BEAT;
        let bar = (beats / BEATS_PER_BAR) + 1;
        let beat_in_bar = (beats % BEATS_PER_BAR) + 1;
        (bar, beat_in_bar)
    }

    pub fn get_bpm(&self) -> f64 {
        self.bpm_x100.load(Ordering::SeqCst) as f64 / 100.0
    }

    /// Handle a MIDI message and update state accordingly.
    pub fn handle_midi_message(&self, message: &[u8]) {
        self.handle_midi_message_at(message, Instant::now());
    }

    /// Handle a MIDI message with a specific timestamp (for testing).
    pub fn handle_midi_message_at(&self, message: &[u8], now: Instant) {
        if message.is_empty() {
            return;
        }

        match message[0] {
            MIDI_START => {
                self.seen_transport.store(true, Ordering::SeqCst);
                self.running.store(true, Ordering::SeqCst);
                self.clock_count.store(0, Ordering::SeqCst);
                self.bpm_x100.store(0, Ordering::SeqCst);
                self.clock_times.lock().unwrap().clear();
            }
            MIDI_CONTINUE => {
                self.seen_transport.store(true, Ordering::SeqCst);
                self.running.store(true, Ordering::SeqCst);
            }
            MIDI_STOP => {
                self.seen_transport.store(true, Ordering::SeqCst);
                self.running.store(false, Ordering::SeqCst);
            }
            MIDI_CLOCK => {
                // Auto-start on clock only if we haven't seen explicit transport yet
                if !self.seen_transport.load(Ordering::SeqCst) {
                    self.running.store(true, Ordering::SeqCst);
                }

                // Always calculate BPM from clock pulses (even when stopped)
                let mut buffer = self.clock_times.lock().unwrap();
                buffer.push(now);

                if let Some((oldest_time, sample_count)) = buffer.get_oldest() {
                    if sample_count > 1 {
                        let elapsed = now.duration_since(oldest_time).as_secs_f64();
                        if elapsed > 0.0 {
                            let clocks = (sample_count - 1) as f64;
                            let beats = clocks / CLOCKS_PER_BEAT as f64;
                            let minutes = elapsed / 60.0;
                            let bpm = beats / minutes;
                            self.bpm_x100.store((bpm * 100.0) as u64, Ordering::SeqCst);
                        }
                    }
                }

                // Only count position when running
                if self.running.load(Ordering::SeqCst) {
                    self.clock_count.fetch_add(1, Ordering::SeqCst);
                }
            }
            _ => {}
        }
    }
}

impl Default for ClockState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_initial_state() {
        let state = ClockState::new();
        assert!(!state.is_running());
        assert_eq!(state.get_clock_count(), 0);
        assert_eq!(state.get_position(), (1, 1));
        assert_eq!(state.get_bpm(), 0.0);
    }

    #[test]
    fn test_auto_start_on_clock_without_transport() {
        let state = ClockState::new();
        assert!(!state.is_running());

        // Without any transport messages, clock pulses auto-start playback
        state.handle_midi_message(&[MIDI_CLOCK]);
        assert!(state.is_running());
        assert_eq!(state.get_clock_count(), 1);
    }

    #[test]
    fn test_stop_prevents_auto_start() {
        let state = ClockState::new();

        state.handle_midi_message(&[MIDI_START]);
        assert!(state.is_running());

        state.handle_midi_message(&[MIDI_STOP]);
        assert!(!state.is_running());

        // Clock pulses should NOT auto-start after we've seen transport messages
        state.handle_midi_message(&[MIDI_CLOCK]);
        assert!(!state.is_running());
        assert_eq!(state.get_clock_count(), 0);
    }

    #[test]
    fn test_continue_restarts_after_stop() {
        let state = ClockState::new();

        state.handle_midi_message(&[MIDI_START]);
        state.handle_midi_message(&[MIDI_CLOCK]);
        state.handle_midi_message(&[MIDI_STOP]);
        assert!(!state.is_running());

        state.handle_midi_message(&[MIDI_CONTINUE]);
        assert!(state.is_running());

        let count_before = state.get_clock_count();
        state.handle_midi_message(&[MIDI_CLOCK]);
        assert_eq!(state.get_clock_count(), count_before + 1);
    }

    #[test]
    fn test_start_resets_position() {
        let state = ClockState::new();

        state.handle_midi_message(&[MIDI_START]);
        for _ in 0..50 {
            state.handle_midi_message(&[MIDI_CLOCK]);
        }
        assert!(state.get_clock_count() > 0);

        state.handle_midi_message(&[MIDI_START]);
        assert_eq!(state.get_clock_count(), 0);
        assert_eq!(state.get_position(), (1, 1));
    }

    #[test]
    fn test_position_calculation() {
        let state = ClockState::new();
        state.handle_midi_message(&[MIDI_START]);

        // 24 clocks = 1 beat
        for _ in 0..24 {
            state.handle_midi_message(&[MIDI_CLOCK]);
        }
        assert_eq!(state.get_position(), (1, 2));

        // 96 clocks = 4 beats = 1 bar
        for _ in 0..(96 - 24) {
            state.handle_midi_message(&[MIDI_CLOCK]);
        }
        assert_eq!(state.get_position(), (2, 1));
    }

    #[test]
    fn test_bpm_calculation() {
        let state = ClockState::new();
        state.handle_midi_message(&[MIDI_START]);

        let start = Instant::now();
        // 120 BPM: 24 clocks per beat, 2 beats per second = 48 clocks/sec
        let clock_interval = Duration::from_micros(20833);

        for i in 0..100 {
            let time = start + clock_interval * i;
            state.handle_midi_message_at(&[MIDI_CLOCK], time);
        }

        let bpm = state.get_bpm();
        assert!(
            (bpm - 120.0).abs() < 1.2,
            "Expected BPM ~120, got {}",
            bpm
        );
    }

    #[test]
    fn test_bpm_updates_even_when_stopped() {
        let state = ClockState::new();
        state.handle_midi_message(&[MIDI_START]);
        state.handle_midi_message(&[MIDI_STOP]);
        assert!(!state.is_running());

        let start = Instant::now();
        let clock_interval = Duration::from_micros(20833);

        for i in 0..50 {
            let time = start + clock_interval * i;
            state.handle_midi_message_at(&[MIDI_CLOCK], time);
        }

        let bpm = state.get_bpm();
        assert!(bpm > 100.0, "BPM should be calculated even when stopped");
        assert_eq!(state.get_clock_count(), 0);
    }

    #[test]
    fn test_empty_message_ignored() {
        let state = ClockState::new();
        state.handle_midi_message(&[]);
        assert!(!state.is_running());
        assert_eq!(state.get_clock_count(), 0);
    }

    #[test]
    fn test_unknown_message_ignored() {
        let state = ClockState::new();
        state.handle_midi_message(&[0x90, 60, 100]); // Note On
        assert!(!state.is_running());
        assert_eq!(state.get_clock_count(), 0);
    }
}
