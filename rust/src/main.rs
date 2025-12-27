//! MIDI Looper - A GUI application for syncing to external MIDI clock.
//!
//! This application connects to a MIDI input (preferring IAC Driver on macOS),
//! loads a MIDI loop, and plays it back in sync with the external clock.

mod clock;
mod midi;
mod playback;
mod ui;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use iced::keyboard::{self, key::Named, Key};
use iced::time::{self, milliseconds};
use iced::widget::{button, column, container, row, text};
use iced::window::{self, Screenshot};
use iced::{Center, Element, Fill, Subscription, Task, Theme};
use midir::MidiInput;

use clock::ClockState;
use midi::MidiOut;
use playback::{Loop, Sequence, SequenceEntry, SequenceGrid, SequencePlayer, SlotId};
use ui::{view_sequence_table, QuanEditState};

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
    // Sequence grid for UI
    sequence_grid: SequenceGrid,
    // Screenshot request flag (set by MIDI CC 119)
    screenshot_requested: Arc<AtomicBool>,
    // QUAN editing state
    editing_quan: Option<SlotId>,
    quan_input: String,
    // Available loops for dropdown
    available_loops: Vec<(String, PathBuf)>,
}

#[derive(Debug, Clone)]
enum Message {
    Tick,
    Play,
    Stop,
    ToggleClockMode,
    KeyPressed(Key),
    ScreenshotCaptured(Screenshot),
    SetNextSlot(SlotId, Option<SlotId>),
    StartEditQuan(SlotId),
    EditQuanValue(String),
    CommitQuanEdit,
    SetSlotLoop(SlotId, Option<usize>),
}

/// Scan for available MIDI loops in the data/out directory.
fn scan_available_loops() -> Vec<(String, PathBuf)> {
    let data_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap_or(std::path::Path::new("."))
        .join("data/out");

    let mut loops = Vec::new();

    if let Ok(entries) = std::fs::read_dir(&data_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map(|e| e == "mid").unwrap_or(false) {
                let name = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("Unknown")
                    .to_string();
                loops.push((name, path));
            }
        }
    }

    // Sort by name for consistent ordering
    loops.sort_by(|a, b| a.0.cmp(&b.0));
    loops
}

impl Looper {
    fn new() -> Self {
        let clock_state = ClockState::new();
        let sequence_player = Arc::new(Mutex::new(SequencePlayer::new()));
        let midi_out = Arc::new(Mutex::new(MidiOut::new().ok()));
        let master_mode = Arc::new(AtomicBool::new(false));
        let screenshot_requested = Arc::new(AtomicBool::new(false));

        // Scan for available loops
        let available_loops = scan_available_loops();
        println!("Found {} loops in data/out/", available_loops.len());

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
            screenshot_requested.clone(),
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

        // Initialize sequence grid (currently empty - will be populated from UI)
        let sequence_grid = SequenceGrid::new();

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
            sequence_grid,
            screenshot_requested,
            editing_quan: None,
            quan_input: String::new(),
            available_loops,
        }
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Tick => {
                // Check for MIDI-triggered screenshot request
                if self.screenshot_requested.swap(false, Ordering::SeqCst) {
                    return window::oldest().and_then(|window_id| {
                        window::screenshot(window_id)
                    }).map(Message::ScreenshotCaptured);
                }
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
            Message::KeyPressed(key) => {
                // F12 triggers screenshot
                if key == Key::Named(Named::F12) {
                    return window::oldest().and_then(|window_id| {
                        window::screenshot(window_id)
                    }).map(Message::ScreenshotCaptured);
                }
            }
            Message::ScreenshotCaptured(screenshot) => {
                // Save screenshot to file
                if let Err(e) = save_screenshot(&screenshot) {
                    eprintln!("Failed to save screenshot: {}", e);
                }
            }
            Message::SetNextSlot(slot_id, next_slot) => {
                // Update the grid's NEXT pointer for this slot
                self.sequence_grid.set_next(slot_id, next_slot);
            }
            Message::StartEditQuan(slot_id) => {
                // Start editing QUAN for this slot
                let current_value = self.sequence_grid.get(slot_id).repeat_count;
                self.editing_quan = Some(slot_id);
                self.quan_input = current_value.to_string();
            }
            Message::EditQuanValue(value) => {
                // Update the input value (only digits allowed)
                if value.chars().all(|c| c.is_ascii_digit()) {
                    self.quan_input = value;
                }
            }
            Message::CommitQuanEdit => {
                // Commit the QUAN edit
                if let Some(slot_id) = self.editing_quan.take() {
                    if let Ok(count) = self.quan_input.parse::<u32>() {
                        // Clamp to valid range (1-999)
                        let count = count.max(1).min(999);
                        self.sequence_grid.set_repeat_count(slot_id, count);
                    }
                }
                self.quan_input.clear();
            }
            Message::SetSlotLoop(slot_id, loop_index) => {
                // Load or clear the loop for this slot
                match loop_index {
                    Some(idx) => {
                        if let Some((name, path)) = self.available_loops.get(idx) {
                            // Default to 4 bars - could be made configurable later
                            match Loop::from_file(path, 4) {
                                Ok(mut loaded_loop) => {
                                    loaded_loop.set_channel(0);
                                    println!("Loaded loop '{}' into slot {}", name, slot_id);
                                    self.sequence_grid.load_loop(slot_id, loaded_loop);
                                }
                                Err(e) => {
                                    eprintln!("Failed to load loop '{}': {}", name, e);
                                }
                            }
                        }
                    }
                    None => {
                        self.sequence_grid.clear_loop(slot_id);
                    }
                }
            }
        }
        Task::none()
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

        // Get playback state for grid highlighting
        let playback_state = {
            let player = self.sequence_player.lock().unwrap();
            player.grid_playback_state()
        };

        // Sequence table with QUAN editing state
        let quan_edit = QuanEditState {
            editing_slot: self.editing_quan,
            input_value: &self.quan_input,
        };
        let sequence_table: Element<'_, Message> = view_sequence_table(
            &self.sequence_grid,
            playback_state,
            &self.available_loops,
            quan_edit,
            |slot_id, loop_idx| Message::SetSlotLoop(slot_id, loop_idx),
            |slot_id, next_slot| Message::SetNextSlot(slot_id, next_slot),
            |slot_id| Message::StartEditQuan(slot_id),
            Message::EditQuanValue,
            Message::CommitQuanEdit,
        );

        let content = column![
            text("MIDI Looper").size(32),
            text(in_status).size(12),
            text(out_status).size(12),
            clock_mode_button,
            text("").size(5),
            row![
                text(format!("BPM: {:.1}", bpm)).size(24),
                text(format!("Bar {} · Beat {}", bar, beat)).size(24),
            ].spacing(20),
            text("").size(5),
            transport_controls,
            text("").size(5),
            text(format!("Loop: {} ({})", loop_name, loop_progress)).size(14),
            text("").size(10),
            sequence_table,
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
        let tick = time::every(milliseconds(50)).map(|_| Message::Tick);
        let keyboard = keyboard::listen().map(|event| {
            if let keyboard::Event::KeyPressed { key, .. } = event {
                Message::KeyPressed(key)
            } else {
                Message::Tick // Ignore other keyboard events
            }
        });
        Subscription::batch([tick, keyboard])
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

/// Save a screenshot to the screenshots/ directory with timestamp filename.
fn save_screenshot(screenshot: &Screenshot) -> Result<std::path::PathBuf, Box<dyn std::error::Error>> {
    // Use manifest directory at compile time for consistent path
    let project_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let screenshots_dir = project_dir.join("screenshots");
    std::fs::create_dir_all(&screenshots_dir)?;

    // Generate timestamp filename
    let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
    let filename = format!("looper_{}.png", timestamp);
    let path = screenshots_dir.join(&filename);

    // Get screenshot dimensions and bytes
    let width = screenshot.size.width;
    let height = screenshot.size.height;
    let rgba_bytes: &[u8] = screenshot.as_ref();

    // Save as PNG using image crate
    let img = image::RgbaImage::from_raw(width, height, rgba_bytes.to_vec())
        .ok_or("Failed to create image from screenshot bytes")?;
    img.save(&path)?;

    println!("Screenshot saved: {}", path.display());
    Ok(path)
}

fn start_midi_listener(
    clock_state: ClockState,
    sequence_player: Arc<Mutex<SequencePlayer>>,
    midi_out: Arc<Mutex<Option<MidiOut>>>,
    master_mode: Arc<AtomicBool>,
    screenshot_requested: Arc<AtomicBool>,
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
            // Check for screenshot trigger (CC 119 value 127)
            if midi::is_screenshot_trigger(message) {
                screenshot_requested.store(true, Ordering::SeqCst);
                return;
            }

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
