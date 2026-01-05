//! MIDI Looper - A GUI application for syncing to external MIDI clock.
//!
//! This application connects to a MIDI input (preferring IAC Driver on macOS),
//! loads a MIDI loop, and plays it back in sync with the external clock.

mod clock;
mod config;
mod midi;
mod playback;
mod ui;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use iced::keyboard::{self, key::Named, Key};
use iced::time::{self, milliseconds};
use iced::widget::{button, column, container, pick_list, row, text};
use iced::window::{self, Screenshot};
use iced::{Center, Element, Fill, Subscription, Task, Theme};
use midir::MidiInput;

use clock::ClockState;
use config::{LooperConfig, SlotConfig};
use midi::MidiOut;
use playback::{Loop, SequenceGrid, SequencePlayer, SlotId};
use ui::view_sequence_table;

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
    // Available loops for dropdown (name, optional path - None for built-ins)
    available_loops: Vec<(String, Option<PathBuf>)>,
    // MIDI output routing
    available_outputs: Vec<String>,
    selected_output: usize,
    output_channel: u8, // 0-15 (displayed as 1-16)
    // Config persistence
    config_path: PathBuf,
    // Display settings
    zero_indexed_countdown: bool,
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
    DecrementQuan(SlotId),
    IncrementQuan(SlotId),
    SetSlotLoop(SlotId, Option<usize>),
    SetOutputDevice(usize),
    SetOutputChannel(u8),
}

/// Built-in empty loop names (no MIDI file needed).
const EMPTY_LOOP_4: &str = "[Empty 4 bars]";
const EMPTY_LOOP_8: &str = "[Empty 8 bars]";
const EMPTY_LOOP_16: &str = "[Empty 16 bars]";

/// Scan for available MIDI loops in the data/out directory.
/// Returns (name, Option<path>) - path is None for built-in loops.
fn scan_available_loops() -> Vec<(String, Option<PathBuf>)> {
    let data_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap_or(std::path::Path::new("."))
        .join("data/out");

    // Start with built-in empty loops
    let mut loops: Vec<(String, Option<PathBuf>)> = vec![
        (EMPTY_LOOP_4.to_string(), None),
        (EMPTY_LOOP_8.to_string(), None),
        (EMPTY_LOOP_16.to_string(), None),
    ];

    // Add MIDI files from data/out
    if let Ok(entries) = std::fs::read_dir(&data_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map(|e| e == "mid").unwrap_or(false) {
                let name = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("Unknown")
                    .to_string();
                loops.push((name, Some(path)));
            }
        }
    }

    // Sort file loops by name (keep built-ins at top)
    let (built_in, mut file_loops): (Vec<_>, Vec<_>) = loops
        .into_iter()
        .partition(|(_, path)| path.is_none());
    file_loops.sort_by(|a, b| a.0.cmp(&b.0));

    let mut result = built_in;
    result.extend(file_loops);
    result
}

/// Wrapper for output device dropdown options.
#[derive(Debug, Clone, PartialEq, Eq)]
struct OutputDeviceOption {
    index: usize,
    name: String,
}

impl std::fmt::Display for OutputDeviceOption {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name)
    }
}

/// Wrapper for MIDI channel dropdown options (1-16).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ChannelOption(u8);

impl std::fmt::Display for ChannelOption {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Ch {}", self.0 + 1) // Display as 1-16
    }
}

impl Looper {
    fn new() -> Self {
        let clock_state = ClockState::new();
        let sequence_player = Arc::new(Mutex::new(SequencePlayer::new()));
        let master_mode = Arc::new(AtomicBool::new(false));
        let screenshot_requested = Arc::new(AtomicBool::new(false));

        // Load saved configuration
        let config_path = LooperConfig::default_path();
        let config = match LooperConfig::load(&config_path) {
            Ok(c) => {
                println!("Loaded config from {}", config_path.display());
                c
            }
            Err(e) => {
                eprintln!("Failed to load config: {}", e);
                LooperConfig::default()
            }
        };

        // Scan for available MIDI output ports
        let available_outputs = midi::scan_output_ports();
        println!("Found {} MIDI output ports", available_outputs.len());
        for (i, name) in available_outputs.iter().enumerate() {
            println!("  {}: {}", i, name);
        }

        // Find output device from config, or fall back to IAC Driver, or first port
        let selected_output = if let Some(ref device_name) = config.output_device {
            available_outputs
                .iter()
                .position(|n| n == device_name)
                .unwrap_or_else(|| {
                    println!("Configured device '{}' not found, using default", device_name);
                    available_outputs.iter().position(|n| n.contains("IAC")).unwrap_or(0)
                })
        } else {
            available_outputs.iter().position(|n| n.contains("IAC")).unwrap_or(0)
        };

        // Channel from config (config stores 1-indexed, we use 0-indexed internally)
        let output_channel: u8 = config.output_channel.saturating_sub(1).min(15);

        // Display settings from config
        let zero_indexed_countdown = config.zero_indexed_countdown;

        // Connect to the selected output
        let midi_out = Arc::new(Mutex::new(
            MidiOut::connect_to_port(available_outputs.get(selected_output).map(|s| s.as_str())).ok()
        ));

        // Scan for available loops
        let available_loops = scan_available_loops();
        println!("Found {} loops in data/out/", available_loops.len());

        // Initialize grid and load loops from config
        let mut sequence_grid = SequenceGrid::new();

        // Apply slot configurations from saved config
        for c in 'A'..='Z' {
            let slot_config = config.get_slot(c);
            let slot_id = SlotId(c);

            // Load loop if configured
            if let Some(ref loop_name) = slot_config.loop_file {
                // Try to find the loop in available_loops by name
                if let Some((_, path_opt)) = available_loops.iter().find(|(name, _)| name == loop_name) {
                    let loaded_loop = match path_opt {
                        Some(path) => {
                            // File-based loop
                            Loop::from_file(path, 4).ok()
                        }
                        None => {
                            // Built-in empty loop
                            Some(match loop_name.as_str() {
                                EMPTY_LOOP_4 => Loop::empty(loop_name, 4),
                                EMPTY_LOOP_8 => Loop::empty(loop_name, 8),
                                EMPTY_LOOP_16 => Loop::empty(loop_name, 16),
                                _ => Loop::empty(loop_name, 4),
                            })
                        }
                    };

                    if let Some(mut lp) = loaded_loop {
                        lp.set_channel(output_channel);
                        println!("Loaded loop '{}' into slot {} from config", loop_name, c);
                        sequence_grid.load_loop(slot_id, lp);
                    }
                } else {
                    eprintln!("Configured loop '{}' not found", loop_name);
                }
            }

            // Set repeat count
            sequence_grid.set_repeat_count(slot_id, slot_config.repeat_count);

            // Set next slot
            if let Some(next_char) = slot_config.next_slot {
                sequence_grid.set_next(slot_id, Some(SlotId(next_char)));
            }
        }

        // Load grid into player
        {
            let mut player = sequence_player.lock().unwrap();
            player.load_grid(sequence_grid.clone());
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
                            // Send All Notes Off to avoid stuck notes
                            if let Ok(mut out_guard) = midi_out.lock() {
                                if let Some(ref mut out) = *out_guard {
                                    let _ = out.send(&[0xB0, 123, 0]);
                                }
                            }
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
            sequence_grid,
            screenshot_requested,
            available_loops,
            available_outputs,
            selected_output,
            output_channel,
            config_path,
            zero_indexed_countdown,
        }
    }

    /// Save the current configuration to disk.
    fn save_config(&self) {
        let mut config = LooperConfig::default();

        // Save output device
        if let Some(name) = self.available_outputs.get(self.selected_output) {
            config.output_device = Some(name.clone());
        }

        // Save channel (1-indexed for YAML readability)
        config.output_channel = self.output_channel + 1;

        // Save display settings
        config.zero_indexed_countdown = self.zero_indexed_countdown;

        // Save slot configurations
        for slot in &self.sequence_grid.slots {
            let slot_config = SlotConfig {
                loop_file: slot.loop_data.as_ref().map(|l| l.name.clone()),
                repeat_count: slot.repeat_count,
                next_slot: slot.next_slot.map(|s| s.0),
            };
            config.set_slot(slot.id.0, slot_config);
        }

        // Write to file
        if let Err(e) = config.save(&self.config_path) {
            eprintln!("Failed to save config: {}", e);
        } else {
            println!("Config saved to {}", self.config_path.display());
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
                        // Send All Notes Off (CC 123) on channel 0 to avoid stuck notes
                        let _ = out.send(&[0xB0, 123, 0]);
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
                // Sync grid to player
                self.sequence_player.lock().unwrap().update_grid(self.sequence_grid.clone());
                self.save_config();
            }
            Message::DecrementQuan(slot_id) => {
                let current = self.sequence_grid.get(slot_id).repeat_count;
                if current > 1 {
                    self.sequence_grid.set_repeat_count(slot_id, current - 1);
                    self.sequence_player.lock().unwrap().update_grid(self.sequence_grid.clone());
                    self.save_config();
                }
            }
            Message::IncrementQuan(slot_id) => {
                let current = self.sequence_grid.get(slot_id).repeat_count;
                if current < 999 {
                    self.sequence_grid.set_repeat_count(slot_id, current + 1);
                    self.sequence_player.lock().unwrap().update_grid(self.sequence_grid.clone());
                    self.save_config();
                }
            }
            Message::SetSlotLoop(slot_id, loop_index) => {
                // Load or clear the loop for this slot
                match loop_index {
                    Some(idx) => {
                        if let Some((name, path_opt)) = self.available_loops.get(idx) {
                            let loaded_loop = match path_opt {
                                Some(path) => {
                                    // File-based loop
                                    Loop::from_file(path, 4).ok()
                                }
                                None => {
                                    // Built-in empty loop
                                    Some(match name.as_str() {
                                        EMPTY_LOOP_4 => Loop::empty(name, 4),
                                        EMPTY_LOOP_8 => Loop::empty(name, 8),
                                        EMPTY_LOOP_16 => Loop::empty(name, 16),
                                        _ => Loop::empty(name, 4),
                                    })
                                }
                            };

                            if let Some(mut lp) = loaded_loop {
                                lp.set_channel(self.output_channel);
                                println!("Loaded loop '{}' into slot {}", name, slot_id);
                                self.sequence_grid.load_loop(slot_id, lp);
                            } else {
                                eprintln!("Failed to load loop '{}'", name);
                            }
                        }
                    }
                    None => {
                        self.sequence_grid.clear_loop(slot_id);
                    }
                }
                // Sync grid to player
                self.sequence_player.lock().unwrap().update_grid(self.sequence_grid.clone());
                self.save_config();
            }
            Message::SetOutputDevice(idx) => {
                if idx < self.available_outputs.len() {
                    // Send All Notes Off on current output before switching
                    if let Ok(mut out_guard) = self.midi_out.lock() {
                        if let Some(ref mut out) = *out_guard {
                            let _ = out.send(&[0xB0 | self.output_channel, 123, 0]);
                        }
                    }

                    self.selected_output = idx;
                    let port_name = &self.available_outputs[idx];
                    println!("Switching MIDI output to: {}", port_name);

                    // Reconnect to new output
                    match MidiOut::connect_to_port(Some(port_name)) {
                        Ok(new_out) => {
                            self.out_port_name = new_out.port_name.clone();
                            self.midi_out_connected = true;
                            *self.midi_out.lock().unwrap() = Some(new_out);
                        }
                        Err(e) => {
                            eprintln!("Failed to connect to {}: {}", port_name, e);
                            self.midi_out_connected = false;
                            *self.midi_out.lock().unwrap() = None;
                        }
                    }
                    self.save_config();
                }
            }
            Message::SetOutputChannel(channel) => {
                let old_channel = self.output_channel;
                let new_channel = channel.min(15); // Clamp to 0-15

                // Send All Notes Off on OLD channel before switching
                if let Ok(mut out_guard) = self.midi_out.lock() {
                    if let Some(ref mut out) = *out_guard {
                        let _ = out.send(&[0xB0 | old_channel, 123, 0]);
                    }
                }

                self.output_channel = new_channel;
                println!("Set MIDI output channel to: {}", new_channel + 1);

                // Update channel for all loaded loops in the grid
                for slot in &mut self.sequence_grid.slots {
                    if let Some(ref mut loop_data) = slot.loop_data {
                        loop_data.set_channel(self.output_channel);
                    }
                }
                // Sync grid to player
                self.sequence_player.lock().unwrap().update_grid(self.sequence_grid.clone());
                self.save_config();
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

        // Transport control buttons
        let play_button = button(text("▶").size(30).color(play_color))
            .padding(15)
            .on_press(Message::Play);
        let stop_button = button(text("⏹").size(30).color(stop_color))
            .padding(15)
            .on_press(Message::Stop);
        let transport_controls = row![play_button, stop_button].spacing(20);

        // Output device dropdown
        let output_options: Vec<OutputDeviceOption> = self
            .available_outputs
            .iter()
            .enumerate()
            .map(|(i, name)| OutputDeviceOption {
                index: i,
                name: name.clone(),
            })
            .collect();
        let selected_device = output_options.get(self.selected_output).cloned();
        let output_picker = pick_list(
            output_options,
            selected_device,
            |opt| Message::SetOutputDevice(opt.index),
        )
        .placeholder("Select output...")
        .width(200);

        // Channel dropdown (1-16)
        let channel_options: Vec<ChannelOption> = (0..16).map(ChannelOption).collect();
        let selected_channel = Some(ChannelOption(self.output_channel));
        let channel_picker = pick_list(
            channel_options,
            selected_channel,
            |opt| Message::SetOutputChannel(opt.0),
        )
        .width(80);

        let output_row = row![
            text("Output:").size(14),
            output_picker,
            channel_picker,
        ]
        .spacing(10)
        .align_y(iced::Center);

        // Get playback state for grid highlighting and countdown display
        let playback_state = {
            let player = self.sequence_player.lock().unwrap();
            player.grid_playback_state()
        };

        // Build countdown display text (bars.beats remaining until transition)
        let countdown_text = if running {
            if let Some(ref state) = playback_state {
                let slot_name = self.sequence_grid.get(state.current_slot).loop_name();
                // Truncate long names
                let display_name = if slot_name.len() > 25 {
                    format!("{}...", &slot_name[..22])
                } else {
                    slot_name.to_string()
                };
                // Calculate remaining bars (including current) with repeats multiplied out
                let offset = if self.zero_indexed_countdown { 0 } else { 1 };
                let bars_remaining = (state.total_iterations - state.current_iteration) * state.total_bars
                    + (state.total_bars - state.current_bar + offset);
                let beats_remaining = 4 + offset - state.current_beat;
                format!("{}: {}.{}", display_name, bars_remaining, beats_remaining)
            } else {
                "No loop playing".to_string()
            }
        } else {
            "Stopped".to_string()
        };

        // Sequence table
        let sequence_table: Element<'_, Message> = view_sequence_table(
            &self.sequence_grid,
            playback_state,
            &self.available_loops,
            |slot_id, loop_idx| Message::SetSlotLoop(slot_id, loop_idx),
            |slot_id, next_slot| Message::SetNextSlot(slot_id, next_slot),
            |slot_id| Message::DecrementQuan(slot_id),
            |slot_id| Message::IncrementQuan(slot_id),
        );

        let content = column![
            text("MIDI Looper").size(32),
            text(in_status).size(12),
            clock_mode_button,
            text("").size(5),
            row![
                text(format!("BPM: {:.1}", bpm)).size(24),
                text(format!("Bar {} · Beat {}", bar, beat)).size(24),
            ].spacing(20),
            text("").size(5),
            transport_controls,
            text("").size(5),
            text(countdown_text).size(16),
            text("").size(5),
            output_row,
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

            // Send All Notes Off on transport stop to avoid stuck notes
            if !message.is_empty() && message[0] == midi::MIDI_STOP {
                if let Ok(mut out_guard) = midi_out.lock() {
                    if let Some(ref mut out) = *out_guard {
                        let _ = out.send(&[0xB0, 123, 0]); // All Notes Off on channel 0
                    }
                }
            }
        },
        (),
    );

    match connection {
        Ok(conn) => (Some(conn), port_name),
        Err(_) => (None, "Failed to connect".to_string()),
    }
}
