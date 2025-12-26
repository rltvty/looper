//! MIDI Monitor - Console tool for debugging MIDI messages
//!
//! Usage: cargo run --bin midi_monitor

use midir::MidiInput;
use std::io::{self, Write};

fn main() {
    println!("=== MIDI Monitor ===");
    println!("Listening for MIDI messages...\n");

    let midi_in = match MidiInput::new("midi-monitor") {
        Ok(m) => m,
        Err(e) => {
            eprintln!("Failed to create MIDI input: {}", e);
            return;
        }
    };

    let ports = midi_in.ports();
    if ports.is_empty() {
        eprintln!("No MIDI input ports found!");
        return;
    }

    // List available ports
    println!("Available MIDI input ports:");
    for (i, port) in ports.iter().enumerate() {
        let name = midi_in.port_name(port).unwrap_or_else(|_| "Unknown".to_string());
        println!("  {}: {}", i, name);
    }
    println!();

    // Prefer IAC Driver, otherwise use first port
    let port_idx = ports
        .iter()
        .position(|p| {
            midi_in
                .port_name(p)
                .map(|n| n.contains("IAC"))
                .unwrap_or(false)
        })
        .unwrap_or(0);

    let port = &ports[port_idx];
    let port_name = midi_in.port_name(port).unwrap_or_else(|_| "Unknown".to_string());
    println!("Connecting to: {}\n", port_name);
    println!("{:<12} {:<20} {:<30} {}", "TIMESTAMP", "TYPE", "DATA (HEX)", "DETAILS");
    println!("{}", "-".repeat(80));

    let _connection = midi_in.connect(
        port,
        "midi-monitor-in",
        move |timestamp, message, _| {
            print_midi_message(timestamp, message);
        },
        (),
    );

    match _connection {
        Ok(conn) => {
            // Check for --duration argument, default to waiting for Enter
            let args: Vec<String> = std::env::args().collect();
            let duration_secs: Option<u64> = args
                .iter()
                .position(|a| a == "--duration")
                .and_then(|i| args.get(i + 1))
                .and_then(|s| s.parse().ok());

            if let Some(secs) = duration_secs {
                println!("\nMonitoring for {} seconds...\n", secs);
                std::thread::sleep(std::time::Duration::from_secs(secs));
            } else {
                println!("\nPress Enter to quit (or use --duration <secs>)...\n");
                let mut input = String::new();
                io::stdin().read_line(&mut input).unwrap();
            }
            drop(conn);
        }
        Err(e) => {
            eprintln!("Failed to connect: {}", e);
        }
    }
}

fn print_midi_message(timestamp: u64, message: &[u8]) {
    if message.is_empty() {
        return;
    }

    let hex_str: String = message.iter().map(|b| format!("{:02X} ", b)).collect();

    let (msg_type, details) = parse_midi_message(message);

    // Flush to ensure immediate output
    println!("{:<12} {:<20} {:<30} {}", timestamp, msg_type, hex_str.trim(), details);
    io::stdout().flush().unwrap();
}

fn parse_midi_message(message: &[u8]) -> (&'static str, String) {
    if message.is_empty() {
        return ("EMPTY", String::new());
    }

    let status = message[0];

    // Real-time messages (single byte, 0xF8-0xFF)
    match status {
        0xF8 => return ("CLOCK", "MIDI Clock pulse (24 ppqn)".to_string()),
        0xFA => return ("START", "Start playback from beginning".to_string()),
        0xFB => return ("CONTINUE", "Continue playback".to_string()),
        0xFC => return ("STOP", "Stop playback".to_string()),
        0xFE => return ("ACTIVE_SENSE", "Active sensing".to_string()),
        0xFF => return ("RESET", "System reset".to_string()),
        _ => {}
    }

    // Channel messages
    let msg_type = status & 0xF0;
    let channel = (status & 0x0F) + 1; // 1-indexed for display

    match msg_type {
        0x80 => {
            if message.len() >= 3 {
                let note = message[1];
                let velocity = message[2];
                return ("NOTE_OFF", format!("Ch:{} Note:{} Vel:{}", channel, note_name(note), velocity));
            }
            ("NOTE_OFF", format!("Ch:{}", channel))
        }
        0x90 => {
            if message.len() >= 3 {
                let note = message[1];
                let velocity = message[2];
                let msg_type = if velocity == 0 { "NOTE_OFF" } else { "NOTE_ON" };
                return (msg_type, format!("Ch:{} Note:{} Vel:{}", channel, note_name(note), velocity));
            }
            ("NOTE_ON", format!("Ch:{}", channel))
        }
        0xA0 => ("POLY_PRESSURE", format!("Ch:{}", channel)),
        0xB0 => {
            if message.len() >= 3 {
                let cc = message[1];
                let value = message[2];
                return ("CONTROL_CHANGE", format!("Ch:{} CC:{} Val:{}", channel, cc, value));
            }
            ("CONTROL_CHANGE", format!("Ch:{}", channel))
        }
        0xC0 => {
            if message.len() >= 2 {
                let program = message[1];
                return ("PROGRAM_CHANGE", format!("Ch:{} Prog:{}", channel, program));
            }
            ("PROGRAM_CHANGE", format!("Ch:{}", channel))
        }
        0xD0 => ("CHANNEL_PRESSURE", format!("Ch:{}", channel)),
        0xE0 => {
            if message.len() >= 3 {
                let value = ((message[2] as u16) << 7) | (message[1] as u16);
                return ("PITCH_BEND", format!("Ch:{} Val:{}", channel, value));
            }
            ("PITCH_BEND", format!("Ch:{}", channel))
        }
        0xF0 => ("SYSEX", format!("{} bytes", message.len())),
        _ => ("UNKNOWN", format!("Status: 0x{:02X}", status)),
    }
}

fn note_name(note: u8) -> String {
    let names = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
    let octave = (note / 12) as i8 - 1;
    let name = names[(note % 12) as usize];
    format!("{}{}", name, octave)
}
