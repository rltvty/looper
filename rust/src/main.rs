//! MIDI Looper - A GUI application for syncing to external MIDI clock.
//!
//! This application connects to a MIDI input (preferring IAC Driver on macOS)
//! and displays the current transport state, BPM, and bar/beat position.

mod clock;
mod midi;

use iced::time::{self, milliseconds};
use iced::widget::{column, container, text};
use iced::{Center, Element, Fill, Subscription, Theme};
use midir::MidiInput;

use clock::ClockState;

fn main() -> iced::Result {
    iced::application(Looper::new, Looper::update, Looper::view)
        .title("MIDI Looper")
        .subscription(Looper::subscription)
        .theme(Looper::theme)
        .run()
}

struct Looper {
    clock_state: ClockState,
    midi_connected: bool,
    port_name: String,
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
            text("").size(20),
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
