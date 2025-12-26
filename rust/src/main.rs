//! MIDI Looper - A GUI application for syncing to external MIDI clock.
//!
//! This application connects to a MIDI input (preferring IAC Driver on macOS),
//! loads a MIDI loop, and plays it back in sync with the external clock.

mod clock;
mod midi;
mod playback;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use iced::time::{self, milliseconds};
use iced::widget::{button, column, container, row, text};
use iced::{Center, Element, Fill, Subscription, Theme};
use midir::MidiInput;

use clock::ClockState;
use midi::MidiOut;
use playback::{Loop, Sequence, SequenceEntry, SequencePlayer};

fn main() -> iced::Result {
    iced::application(Looper::new, Looper::update, Looper::view)
        .title("MIDI Looper")
        .subscription(Looper::subscription)
        .theme(Looper::theme)
        .run()
}

struct Looper {
    clock_state: ClockState,
    sequence_player: Arc<Mutex<SequencePlayer>>,
    midi_in_connected: bool,
    midi_out_connected: bool,
    in_port_name: String,
    out_port_name: String,
    master_mode: Arc<AtomicBool>,
    // Keep connections alive
    _midi_in_connection: Option<midir::MidiInputConnection<()>>,
    midi_out: Arc<Mutex<Option<MidiOut>>>,
}

#[derive(Debug, Clone, Copy)]
enum Message {
    Tick,
    Play,
    Stop,
    ToggleClockMode,
}

impl Looper {
    fn new() -> Self {
        let clock_state = ClockState::new();
        let sequence_player = Arc::new(Mutex::new(SequencePlayer::new()));
        let midi_out = Arc::new(Mutex::new(MidiOut::new().ok()));
        let master_mode = Arc::new(AtomicBool::new(false));

        // Define loops to load: (path, bar_length)
        let loop_configs = [
            ("../data/out/Billie Jean - bass - Bass finger - bars 15-26.mid", 12),
            ("../data/out/Psycho Killer - bass - Bass - Tina Weymouth - bars 107-110.mid", 4),
            ("../data/out/Rappers Delight - bass - Electric Bass finger - bars 13-16.mid", 4),
            ("../data/out/Seven Nation Army With Bass Guitar - bass - Jack White Bass Immitation - bars 1-4.mid", 4),
        ];
        let repeat_count = 2;

        // Load all loops
        let mut entries = Vec::new();
        for (path, bars) in &loop_configs {
            match Loop::from_file(path, *bars) {
                Ok(mut loaded_loop) => {
                    loaded_loop.set_channel(0); // MIDI channel 1 (0-indexed)
                    println!(
                        "Loaded loop: {} ({} events, {} clocks)",
                        loaded_loop.name,
                        loaded_loop.events.len(),
                        loaded_loop.length_clocks
                    );
                    entries.push(SequenceEntry {
                        loop_data: loaded_loop,
                        repeat_count,
                    });
                }
                Err(e) => {
                    eprintln!("Failed to load loop {}: {}", path, e);
                }
            }
        }

        if !entries.is_empty() {
            let sequence = Sequence { entries };
            let mut player = sequence_player.lock().unwrap();
            player.load(sequence);
            player.start();
        }

        let out_port_name = midi_out
            .lock()
            .unwrap()
            .as_ref()
            .map(|m| m.port_name.clone())
            .unwrap_or_else(|| "Not connected".to_string());
        let midi_out_connected = midi_out.lock().unwrap().is_some();

        // Start MIDI listener with playback callback
        let (midi_in_connection, in_port_name) = start_midi_listener(
            clock_state.clone(),
            sequence_player.clone(),
            midi_out.clone(),
            master_mode.clone(),
        );

        // Spawn clock generator thread for master mode
        {
            let clock_state = clock_state.clone();
            let sequence_player = sequence_player.clone();
            let midi_out = midi_out.clone();
            let master_mode = master_mode.clone();

            std::thread::spawn(move || {
                use std::time::Instant;

                const BPM: u64 = 120;
                const CLOCKS_PER_BEAT: u64 = 24;
                // Nanoseconds per clock = 60_000_000_000 / (BPM * 24)
                // For 120 BPM: = 60_000_000_000 / 2880 = 20_833_333.333... ns
                // We calculate target time from clock count to avoid cumulative drift

                let mut clock_count: u64 = 0;
                let mut start_time = Instant::now();
                let mut is_running = false;

                loop {
                    // Only generate clock when in master mode and running
                    if master_mode.load(Ordering::SeqCst) && clock_state.is_running() {
                        if !is_running {
                            println!("Clock generator: starting clocks");
                            is_running = true;
                            clock_count = 0;
                            start_time = Instant::now();
                        }

                        // Update internal clock state
                        clock_state.handle_midi_message(&[midi::MIDI_CLOCK]);

                        // Get events to play at current position
                        let events = {
                            let mut player = sequence_player.lock().unwrap();
                            player.tick(clock_state.get_clock_count())
                        };

                        // Send clock and events to MIDI output
                        if let Ok(mut out_guard) = midi_out.lock() {
                            if let Some(ref mut out) = *out_guard {
                                // Send clock pulse
                                if let Err(e) = out.send(&[midi::MIDI_CLOCK]) {
                                    eprintln!("Failed to send clock: {}", e);
                                }
                                // Send note events
                                for event in &events {
                                    // Debug: check for unexpected STOP bytes
                                    if !event.is_empty() && event[0] == midi::MIDI_STOP {
                                        eprintln!("WARNING: Event contains STOP byte: {:?}", event);
                                    }
                                    let _ = out.send(event);
                                }
                            }
                        } else {
                            eprintln!("Failed to lock midi_out");
                        }

                        // Calculate next tick time based on clock count (avoids cumulative drift)
                        clock_count += 1;
                        // target_nanos = clock_count * 60_000_000_000 / (BPM * CLOCKS_PER_BEAT)
                        let target_nanos = (clock_count * 60_000_000_000) / (BPM * CLOCKS_PER_BEAT);
                        let target_time = start_time + Duration::from_nanos(target_nanos);

                        // Sleep until target time
                        let now = Instant::now();
                        if target_time > now {
                            std::thread::sleep(target_time - now);
                        }
                    } else {
                        // Not running - sleep briefly
                        if is_running {
                            println!("Clock generator: stopped");
                            is_running = false;
                        }
                        std::thread::sleep(Duration::from_millis(1));
                    }
                }
            });
        }

        Self {
            clock_state,
            sequence_player,
            midi_in_connected: midi_in_connection.is_some(),
            midi_out_connected,
            in_port_name,
            out_port_name,
            master_mode,
            _midi_in_connection: midi_in_connection,
            midi_out,
        }
    }

    fn update(&mut self, message: Message) {
        match message {
            Message::Tick => {
                // State is updated by MIDI thread, just trigger re-render
            }
            Message::Play => {
                let is_master = self.master_mode.load(Ordering::SeqCst);
                let was_running = self.clock_state.is_running();
                println!("Play clicked: is_master={}, was_running={}", is_master, was_running);

                // Send MIDI messages
                if let Ok(mut out_guard) = self.midi_out.lock() {
                    if let Some(ref mut out) = *out_guard {
                        if was_running {
                            println!("Sending STOP (restart)");
                            let _ = out.send_stop();
                        }
                        println!("Sending START");
                        if let Err(e) = out.send_start() {
                            eprintln!("Failed to send START: {}", e);
                        }
                    } else {
                        eprintln!("No MIDI output available!");
                    }
                } else {
                    eprintln!("Failed to lock midi_out!");
                }

                // In master mode, directly update clock state (no external clock to trigger it)
                if is_master {
                    if was_running {
                        self.clock_state.handle_midi_message(&[midi::MIDI_STOP]);
                    }
                    self.clock_state.handle_midi_message(&[midi::MIDI_START]);
                    self.sequence_player.lock().unwrap().reset();
                }
            }
            Message::Stop => {
                let is_master = self.master_mode.load(Ordering::SeqCst);
                println!("Stop clicked: is_master={}", is_master);

                if let Ok(mut out_guard) = self.midi_out.lock() {
                    if let Some(ref mut out) = *out_guard {
                        println!("Sending STOP");
                        let _ = out.send_stop();
                    }
                }

                // In master mode, directly update clock state
                if is_master {
                    self.clock_state.handle_midi_message(&[midi::MIDI_STOP]);
                }
            }
            Message::ToggleClockMode => {
                let current = self.master_mode.load(Ordering::SeqCst);
                self.master_mode.store(!current, Ordering::SeqCst);

                // If switching to master mode while stopped, ensure clean state
                if !current {
                    // Switching to master mode - mark that we've seen transport
                    // so clock pulses from external source don't auto-start
                    self.clock_state.handle_midi_message(&[midi::MIDI_STOP]);
                }
            }
        }
    }

    fn view(&self) -> Element<'_, Message> {
        let (bar, beat) = self.clock_state.get_position();
        let bpm = self.clock_state.get_bpm();
        let running = self.clock_state.is_running();
        let is_master = self.master_mode.load(Ordering::SeqCst);

        // Button colors based on transport state
        let (play_color, stop_color) = if running {
            (
                iced::Color::from_rgb(0.2, 0.8, 0.2), // Green when playing
                iced::Color::from_rgb(0.6, 0.6, 0.6), // Grey
            )
        } else {
            (
                iced::Color::from_rgb(0.6, 0.6, 0.6), // Grey
                iced::Color::from_rgb(0.8, 0.2, 0.2), // Red when stopped
            )
        };

        let in_status = if self.midi_in_connected {
            format!("IN: {}", self.in_port_name)
        } else {
            "IN: ❌ Not connected".to_string()
        };

        let out_status = if self.midi_out_connected {
            format!("OUT: {}", self.out_port_name)
        } else {
            "OUT: ❌ Not connected".to_string()
        };

        // Clock mode toggle
        let clock_mode_label = if is_master {
            "Clock: MASTER (120 BPM)"
        } else {
            "Clock: EXTERNAL"
        };
        let clock_mode_color = if is_master {
            iced::Color::from_rgb(0.8, 0.6, 0.2) // Orange for master
        } else {
            iced::Color::from_rgb(0.4, 0.6, 0.8) // Blue for external
        };
        let clock_mode_button = button(text(clock_mode_label).size(14).color(clock_mode_color))
            .padding(8)
            .on_press(Message::ToggleClockMode);

        // Get current sequence state
        let (loop_name, loop_progress) = {
            let player = self.sequence_player.lock().unwrap();
            let name = player
                .current_loop_name()
                .unwrap_or("No sequence loaded")
                .to_string();
            let progress = player
                .current_state()
                .map(|(_, iter, total)| format!("{}/{}", iter, total))
                .unwrap_or_default();
            (name, progress)
        };

        // Transport control buttons
        let play_button = button(text("▶").size(30).color(play_color))
            .padding(15)
            .on_press(Message::Play);
        let stop_button = button(text("⏹").size(30).color(stop_color))
            .padding(15)
            .on_press(Message::Stop);
        let transport_controls = row![play_button, stop_button].spacing(20);

        let content = column![
            text("MIDI Looper").size(40),
            text(in_status).size(14),
            text(out_status).size(14),
            clock_mode_button,
            text("").size(10),
            text(format!("Loop: {} ({})", loop_name, loop_progress)).size(16),
            text("").size(10),
            transport_controls,
            text("").size(10),
            text(format!("BPM: {:.1}", bpm)).size(60),
            text("").size(10),
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
    sequence_player: Arc<Mutex<SequencePlayer>>,
    midi_out: Arc<Mutex<Option<MidiOut>>>,
    master_mode: Arc<AtomicBool>,
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

    let connection = midi_in.connect(
        port,
        "looper-clock-in",
        move |_timestamp, message, _| {
            // In master mode, ignore incoming clock and transport - we generate our own
            if master_mode.load(Ordering::SeqCst) {
                return;
            }

            // Update clock state
            clock_state.handle_midi_message(message);

            // Handle playback on clock ticks
            if !message.is_empty() && message[0] == midi::MIDI_CLOCK {
                let clock_count = clock_state.get_clock_count();

                // Get events to play
                let events = {
                    let mut player = sequence_player.lock().unwrap();
                    // Only play when clock is running
                    if clock_state.is_running() {
                        player.tick(clock_count)
                    } else {
                        Vec::new()
                    }
                };

                // Send events to MIDI output
                if !events.is_empty() {
                    if let Ok(mut out_guard) = midi_out.lock() {
                        if let Some(ref mut out) = *out_guard {
                            for event in events {
                                let _ = out.send(&event);
                            }
                        }
                    }
                }
            }

            // Reset sequence player on transport start
            if !message.is_empty() && message[0] == midi::MIDI_START {
                let mut player = sequence_player.lock().unwrap();
                player.reset();
            }
        },
        (),
    );

    match connection {
        Ok(conn) => (Some(conn), port_name),
        Err(_) => (None, "Failed to connect".to_string()),
    }
}
