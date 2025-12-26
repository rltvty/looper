use iced::time::{self, milliseconds};
use iced::widget::{column, container, text};
use iced::{Center, Element, Fill, Subscription, Theme};
use midir::MidiInput;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

// MIDI Clock messages
const MIDI_CLOCK: u8 = 0xF8;
const MIDI_START: u8 = 0xFA;
const MIDI_CONTINUE: u8 = 0xFB;
const MIDI_STOP: u8 = 0xFC;

const CLOCKS_PER_BEAT: u64 = 24;
const BEATS_PER_BAR: u64 = 4;

fn main() -> iced::Result {
    iced::application(Looper::new, Looper::update, Looper::view)
        .title("MIDI Looper")
        .subscription(Looper::subscription)
        .theme(Looper::theme)
        .run()
}

// 1 bar * 4 beats * 24 clocks = 96 clocks for rolling BPM window
const BPM_WINDOW_CLOCKS: usize = 96;

// Shared clock state between MIDI thread and GUI
#[derive(Clone)]
struct ClockState {
    running: Arc<AtomicBool>,
    // Once we see explicit transport, don't auto-start on clock pulses
    seen_transport: Arc<AtomicBool>,
    clock_count: Arc<AtomicU64>,
    // Store BPM as fixed-point (x100) to use atomics
    bpm_x100: Arc<AtomicU64>,
    // Ring buffer for rolling BPM calculation
    clock_times: Arc<std::sync::Mutex<ClockTimeBuffer>>,
}

struct ClockTimeBuffer {
    times: [Option<Instant>; BPM_WINDOW_CLOCKS],
    index: usize,
    count: usize, // Track how many samples we have
}

impl ClockTimeBuffer {
    fn new() -> Self {
        Self {
            times: [None; BPM_WINDOW_CLOCKS],
            index: 0,
            count: 0,
        }
    }

    fn push(&mut self, time: Instant) -> (Option<Instant>, usize) {
        // Get the oldest time before overwriting
        let oldest = self.times[self.index];
        self.times[self.index] = Some(time);
        self.index = (self.index + 1) % BPM_WINDOW_CLOCKS;
        if self.count < BPM_WINDOW_CLOCKS {
            self.count += 1;
        }
        (oldest, self.count)
    }

    // Get the oldest available timestamp for partial buffer calculation
    fn get_oldest(&self) -> Option<(Instant, usize)> {
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

    fn clear(&mut self) {
        self.times = [None; BPM_WINDOW_CLOCKS];
        self.index = 0;
        self.count = 0;
    }
}

impl ClockState {
    fn new() -> Self {
        Self {
            running: Arc::new(AtomicBool::new(false)),
            seen_transport: Arc::new(AtomicBool::new(false)),
            clock_count: Arc::new(AtomicU64::new(0)),
            bpm_x100: Arc::new(AtomicU64::new(0)),
            clock_times: Arc::new(std::sync::Mutex::new(ClockTimeBuffer::new())),
        }
    }

    fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    fn has_seen_transport(&self) -> bool {
        self.seen_transport.load(Ordering::SeqCst)
    }

    fn get_clock_count(&self) -> u64 {
        self.clock_count.load(Ordering::SeqCst)
    }

    fn get_position(&self) -> (u64, u64) {
        let count = self.clock_count.load(Ordering::SeqCst);
        let beats = count / CLOCKS_PER_BEAT;
        let bar = (beats / BEATS_PER_BAR) + 1;
        let beat_in_bar = (beats % BEATS_PER_BAR) + 1;
        (bar, beat_in_bar)
    }

    fn get_bpm(&self) -> f64 {
        self.bpm_x100.load(Ordering::SeqCst) as f64 / 100.0
    }

    /// Handle a MIDI message and update state accordingly
    fn handle_midi_message(&self, message: &[u8]) {
        self.handle_midi_message_at(message, Instant::now());
    }

    /// Handle a MIDI message with a specific timestamp (for testing)
    fn handle_midi_message_at(&self, message: &[u8], now: Instant) {
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

struct Looper {
    clock_state: ClockState,
    midi_connected: bool,
    port_name: String,
    // Keep connection alive
    _midi_connection: Option<midir::MidiInputConnection<()>>,
}

#[derive(Debug, Clone, Copy)]
enum Message {
    Tick,
}

impl Looper {
    fn new() -> Self {
        let clock_state = ClockState::new();
        let (midi_connection, port_name) = start_midi_listener(clock_state.clone());

        Self {
            clock_state,
            midi_connected: midi_connection.is_some(),
            port_name,
            _midi_connection: midi_connection,
        }
    }

    fn update(&mut self, message: Message) {
        match message {
            Message::Tick => {
                // State is updated by MIDI thread, just trigger re-render
            }
        }
    }

    fn view(&self) -> Element<'_, Message> {
        let (bar, beat) = self.clock_state.get_position();
        let bpm = self.clock_state.get_bpm();
        let running = self.clock_state.is_running();

        let status = if running { "▶ PLAYING" } else { "⏹ STOPPED" };
        let status_color = if running {
            iced::Color::from_rgb(0.2, 0.8, 0.2)
        } else {
            iced::Color::from_rgb(0.6, 0.6, 0.6)
        };

        let connection_status = if self.midi_connected {
            format!("🎵 {}", self.port_name)
        } else {
            "❌ No MIDI input".to_string()
        };

        let content = column![
            text("MIDI Looper").size(40),
            text(connection_status).size(16),
            text("").size(20), // spacer
            text(status).size(30).color(status_color),
            text("").size(10), // spacer
            text(format!("BPM: {:.1}", bpm)).size(60),
            text("").size(10), // spacer
            text(format!("Bar {} · Beat {}", bar, beat)).size(40),
        ]
        .align_x(Center);

        container(content)
            .width(Fill)
            .height(Fill)
            .center_x(Fill)
            .center_y(Fill)
            .into()
    }

    fn subscription(&self) -> Subscription<Message> {
        time::every(milliseconds(50)).map(|_| Message::Tick)
    }

    fn theme(&self) -> Theme {
        Theme::Dark
    }
}

impl Default for Looper {
    fn default() -> Self {
        Self::new()
    }
}

fn start_midi_listener(
    clock_state: ClockState,
) -> (Option<midir::MidiInputConnection<()>>, String) {
    let midi_in = match MidiInput::new("looper-clock") {
        Ok(m) => m,
        Err(_) => return (None, "Failed to create MIDI input".to_string()),
    };

    let in_ports = midi_in.ports();
    if in_ports.is_empty() {
        return (None, "No MIDI ports found".to_string());
    }

    // Look for IAC Driver or use first port
    let port_idx = in_ports
        .iter()
        .position(|p| {
            midi_in
                .port_name(p)
                .map(|n| n.contains("IAC"))
                .unwrap_or(false)
        })
        .unwrap_or(0);

    let port = &in_ports[port_idx];
    let port_name = midi_in.port_name(port).unwrap_or_else(|_| "Unknown".into());

    let state = clock_state.clone();

    let connection = midi_in.connect(
        port,
        "looper-clock-in",
        move |_timestamp, message, _| {
            state.handle_midi_message(message);
        },
        (),
    );

    match connection {
        Ok(conn) => (Some(conn), port_name),
        Err(_) => (None, "Failed to connect".to_string()),
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
        assert!(!state.has_seen_transport());
        assert_eq!(state.get_clock_count(), 0);
        assert_eq!(state.get_position(), (1, 1)); // Bar 1, Beat 1
        assert_eq!(state.get_bpm(), 0.0);
    }

    #[test]
    fn test_auto_start_on_clock_without_transport() {
        let state = ClockState::new();

        // Before any messages, should not be running
        assert!(!state.is_running());

        // Clock pulse should auto-start when no transport seen
        state.handle_midi_message(&[MIDI_CLOCK]);
        assert!(state.is_running());
        assert!(!state.has_seen_transport()); // Still false - clock doesn't set this
        assert_eq!(state.get_clock_count(), 1);
    }

    #[test]
    fn test_stop_prevents_auto_start() {
        let state = ClockState::new();

        // Start with explicit START
        state.handle_midi_message(&[MIDI_START]);
        assert!(state.is_running());
        assert!(state.has_seen_transport());

        // STOP should stop
        state.handle_midi_message(&[MIDI_STOP]);
        assert!(!state.is_running());

        // Clock pulses should NOT auto-start after we've seen transport
        state.handle_midi_message(&[MIDI_CLOCK]);
        assert!(!state.is_running());

        // Clock count should not increase when stopped
        assert_eq!(state.get_clock_count(), 0); // START resets to 0, STOP doesn't increment
    }

    #[test]
    fn test_continue_restarts_after_stop() {
        let state = ClockState::new();

        state.handle_midi_message(&[MIDI_START]);
        state.handle_midi_message(&[MIDI_CLOCK]);
        state.handle_midi_message(&[MIDI_STOP]);
        assert!(!state.is_running());

        // CONTINUE should restart
        state.handle_midi_message(&[MIDI_CONTINUE]);
        assert!(state.is_running());

        // Clock should count again
        let count_before = state.get_clock_count();
        state.handle_midi_message(&[MIDI_CLOCK]);
        assert_eq!(state.get_clock_count(), count_before + 1);
    }

    #[test]
    fn test_start_resets_position() {
        let state = ClockState::new();

        // Accumulate some clocks
        state.handle_midi_message(&[MIDI_START]);
        for _ in 0..50 {
            state.handle_midi_message(&[MIDI_CLOCK]);
        }
        assert!(state.get_clock_count() > 0);

        // START should reset
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
        assert_eq!(state.get_position(), (1, 2)); // Bar 1, Beat 2

        // 96 clocks = 4 beats = 1 bar
        for _ in 0..(96 - 24) {
            state.handle_midi_message(&[MIDI_CLOCK]);
        }
        assert_eq!(state.get_position(), (2, 1)); // Bar 2, Beat 1
    }

    #[test]
    fn test_bpm_calculation() {
        let state = ClockState::new();
        state.handle_midi_message(&[MIDI_START]);

        let start = Instant::now();

        // Simulate 120 BPM: 24 clocks per beat, 2 beats per second
        // So 48 clocks per second, or ~20.83ms per clock
        let clock_interval = Duration::from_micros(20833); // ~48 clocks/sec = 120 BPM

        for i in 0..100 {
            let time = start + clock_interval * i;
            state.handle_midi_message_at(&[MIDI_CLOCK], time);
        }

        let bpm = state.get_bpm();
        // Allow 1% tolerance
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
        let clock_interval = Duration::from_micros(20833); // 120 BPM

        // Send clocks while stopped
        for i in 0..50 {
            let time = start + clock_interval * i;
            state.handle_midi_message_at(&[MIDI_CLOCK], time);
        }

        // BPM should still be calculated
        let bpm = state.get_bpm();
        assert!(bpm > 100.0, "BPM should be calculated even when stopped");

        // But position should not advance
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
