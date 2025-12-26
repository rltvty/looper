//! MIDI Looper - A GUI application for syncing to external MIDI clock.
//!
//! This application connects to a MIDI input (preferring IAC Driver on macOS),
//! loads a MIDI loop, and plays it back in sync with the external clock.

mod clock;
mod midi;
mod playback;

use std::sync::{Arc, Mutex};

use iced::time::{self, milliseconds};
use iced::widget::{column, container, text};
use iced::{Center, Element, Fill, Subscription, Theme};
use midir::MidiInput;

use clock::ClockState;
use midi::MidiOut;
use playback::{Loop, LoopPlayer};

fn main() -> iced::Result {
    iced::application(Looper::new, Looper::update, Looper::view)
        .title("MIDI Looper")
        .subscription(Looper::subscription)
        .theme(Looper::theme)
        .run()
}

struct Looper {
    clock_state: ClockState,
    loop_player: Arc<Mutex<LoopPlayer>>,
    midi_in_connected: bool,
    midi_out_connected: bool,
    in_port_name: String,
    out_port_name: String,
    loop_name: String,
    // Keep connections alive
    _midi_in_connection: Option<midir::MidiInputConnection<()>>,
    _midi_out: Arc<Mutex<Option<MidiOut>>>,
}

#[derive(Debug, Clone, Copy)]
enum Message {
    Tick,
}

impl Looper {
    fn new() -> Self {
        let clock_state = ClockState::new();
        let loop_player = Arc::new(Mutex::new(LoopPlayer::new()));
        let midi_out = Arc::new(Mutex::new(MidiOut::new().ok()));

        // Load the bass loop
        let loop_path = "../data/out/Rappers Delight - bass - Electric Bass finger - bars 13-16.mid";
        let mut loop_name = "No loop loaded".to_string();

        match Loop::from_file(loop_path, 4) {
            Ok(mut loaded_loop) => {
                // Set to MIDI channel 1 (0-indexed = 0)
                loaded_loop.set_channel(0);
                loop_name = loaded_loop.name.clone();
                println!(
                    "Loaded loop: {} ({} events, {} clocks)",
                    loop_name,
                    loaded_loop.events.len(),
                    loaded_loop.length_clocks
                );
                let mut player = loop_player.lock().unwrap();
                player.load(loaded_loop);
                player.start();
            }
            Err(e) => {
                eprintln!("Failed to load loop: {}", e);
            }
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
            loop_player.clone(),
            midi_out.clone(),
        );

        Self {
            clock_state,
            loop_player,
            midi_in_connected: midi_in_connection.is_some(),
            midi_out_connected,
            in_port_name,
            out_port_name,
            loop_name,
            _midi_in_connection: midi_in_connection,
            _midi_out: midi_out,
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

        let content = column![
            text("MIDI Looper").size(40),
            text(in_status).size(14),
            text(out_status).size(14),
            text("").size(10),
            text(format!("Loop: {}", self.loop_name)).size(16),
            text("").size(10),
            text(status).size(30).color(status_color),
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
    loop_player: Arc<Mutex<LoopPlayer>>,
    midi_out: Arc<Mutex<Option<MidiOut>>>,
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
            // Update clock state
            clock_state.handle_midi_message(message);

            // Handle playback on clock ticks
            if !message.is_empty() && message[0] == midi::MIDI_CLOCK {
                let clock_count = clock_state.get_clock_count();

                // Get events to play
                let events = {
                    let mut player = loop_player.lock().unwrap();
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

            // Reset loop player on transport start
            if !message.is_empty() && message[0] == midi::MIDI_START {
                let mut player = loop_player.lock().unwrap();
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
