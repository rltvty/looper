//! MIDI loop playback engine.
//!
//! Loads MIDI files and plays them back in sync with the external clock,
//! looping continuously.

use midly::{MidiMessage, Smf, TrackEventKind};
use std::fs;
use std::path::Path;

use crate::midi::CLOCKS_PER_BEAT;

/// A single MIDI event to be played.
#[derive(Debug, Clone)]
pub struct LoopEvent {
    /// Position in MIDI clocks from start of loop (24 ppqn)
    pub clock_position: u64,
    /// MIDI channel (0-15)
    pub channel: u8,
    /// Raw MIDI message bytes (status + data)
    pub message: Vec<u8>,
}

/// A loaded MIDI loop ready for playback.
#[derive(Debug, Clone)]
pub struct Loop {
    /// Name of the loop (from filename)
    pub name: String,
    /// Length of the loop in MIDI clocks
    pub length_clocks: u64,
    /// All events in the loop, sorted by clock position
    pub events: Vec<LoopEvent>,
}

impl Loop {
    /// Load a MIDI file and convert it to a Loop.
    ///
    /// The `loop_length_bars` parameter specifies how many bars the loop should be.
    /// Events are quantized to 24 ppqn (MIDI clock resolution).
    pub fn from_file<P: AsRef<Path>>(path: P, loop_length_bars: u64) -> Result<Self, String> {
        let path = path.as_ref();
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();

        let data = fs::read(path).map_err(|e| format!("Failed to read file: {}", e))?;
        let smf = Smf::parse(&data).map_err(|e| format!("Failed to parse MIDI: {}", e))?;

        // Get the file's ticks per beat (PPQ)
        let file_ppq = match smf.header.timing {
            midly::Timing::Metrical(ppq) => ppq.as_int() as u64,
            midly::Timing::Timecode(_, _) => {
                return Err("Timecode-based MIDI files not supported".to_string())
            }
        };

        let mut events = Vec::new();

        // Parse all tracks
        for track in &smf.tracks {
            let mut tick: u64 = 0;

            for event in track.iter() {
                tick += event.delta.as_int() as u64;

                if let TrackEventKind::Midi { channel, message } = event.kind {
                    // Convert file ticks to 24 ppqn clock ticks
                    let clock_position = (tick * CLOCKS_PER_BEAT) / file_ppq;

                    // Build the raw MIDI message
                    let msg_bytes = match message {
                        MidiMessage::NoteOn { key, vel } => {
                            vec![0x90 | channel.as_int(), key.as_int(), vel.as_int()]
                        }
                        MidiMessage::NoteOff { key, vel } => {
                            vec![0x80 | channel.as_int(), key.as_int(), vel.as_int()]
                        }
                        MidiMessage::Aftertouch { key, vel } => {
                            vec![0xA0 | channel.as_int(), key.as_int(), vel.as_int()]
                        }
                        MidiMessage::Controller { controller, value } => {
                            vec![0xB0 | channel.as_int(), controller.as_int(), value.as_int()]
                        }
                        MidiMessage::ProgramChange { program } => {
                            vec![0xC0 | channel.as_int(), program.as_int()]
                        }
                        MidiMessage::ChannelAftertouch { vel } => {
                            vec![0xD0 | channel.as_int(), vel.as_int()]
                        }
                        MidiMessage::PitchBend { bend } => {
                            let value = bend.as_int() as u16;
                            vec![
                                0xE0 | channel.as_int(),
                                (value & 0x7F) as u8,
                                ((value >> 7) & 0x7F) as u8,
                            ]
                        }
                    };

                    events.push(LoopEvent {
                        clock_position,
                        channel: channel.as_int(),
                        message: msg_bytes,
                    });
                }
            }
        }

        // Sort events by position
        events.sort_by_key(|e| e.clock_position);

        // Calculate loop length: bars * beats_per_bar * clocks_per_beat
        let length_clocks = loop_length_bars * 4 * CLOCKS_PER_BEAT;

        Ok(Loop {
            name,
            length_clocks,
            events,
        })
    }

    /// Override the channel for all events (0-15).
    pub fn set_channel(&mut self, channel: u8) {
        for event in &mut self.events {
            if !event.message.is_empty() {
                // Status byte: high nibble is message type, low nibble is channel
                let status = event.message[0] & 0xF0;
                event.message[0] = status | (channel & 0x0F);
                event.channel = channel;
            }
        }
    }
}

/// Manages playback of a loop in sync with the clock.
pub struct LoopPlayer {
    /// The loop being played
    pub loop_data: Option<Loop>,
    /// Index of the next event to play
    next_event_idx: usize,
    /// Clock position at start of current loop iteration
    loop_start_clock: u64,
    /// Whether playback is enabled
    pub playing: bool,
}

impl LoopPlayer {
    pub fn new() -> Self {
        Self {
            loop_data: None,
            next_event_idx: 0,
            loop_start_clock: 0,
            playing: false,
        }
    }

    /// Load a loop for playback.
    pub fn load(&mut self, loop_data: Loop) {
        self.loop_data = Some(loop_data);
        self.next_event_idx = 0;
        self.loop_start_clock = 0;
    }

    /// Start playback from the beginning.
    pub fn start(&mut self) {
        self.next_event_idx = 0;
        self.loop_start_clock = 0;
        self.playing = true;
    }

    /// Stop playback.
    pub fn stop(&mut self) {
        self.playing = false;
    }

    /// Reset to loop start (called when transport restarts).
    pub fn reset(&mut self) {
        self.next_event_idx = 0;
        self.loop_start_clock = 0;
    }

    /// Called on each clock tick. Returns events that should be sent now.
    pub fn tick(&mut self, clock_count: u64) -> Vec<Vec<u8>> {
        if !self.playing {
            return Vec::new();
        }

        let loop_data = match &self.loop_data {
            Some(l) => l,
            None => return Vec::new(),
        };

        if loop_data.events.is_empty() || loop_data.length_clocks == 0 {
            return Vec::new();
        }

        // Calculate position within the loop
        let position_in_loop = (clock_count - self.loop_start_clock) % loop_data.length_clocks;

        // Check if we've wrapped around to a new loop iteration
        let expected_iteration =
            (clock_count - self.loop_start_clock) / loop_data.length_clocks;
        if expected_iteration > 0 && position_in_loop == 0 {
            // We just started a new iteration
            self.next_event_idx = 0;
        }

        // Collect events that should play at this position
        let mut events_to_send = Vec::new();

        while self.next_event_idx < loop_data.events.len() {
            let event = &loop_data.events[self.next_event_idx];
            if event.clock_position <= position_in_loop {
                events_to_send.push(event.message.clone());
                self.next_event_idx += 1;
            } else {
                break;
            }
        }

        events_to_send
    }
}

impl Default for LoopPlayer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Create a simple test loop with note on/off events
    fn make_test_loop() -> Loop {
        // A simple 1-bar loop (96 clocks) with 2 notes
        Loop {
            name: "test".to_string(),
            length_clocks: 96,
            events: vec![
                // Note on at clock 0
                LoopEvent {
                    clock_position: 0,
                    channel: 0,
                    message: vec![0x90, 60, 100], // Note On, C4, vel 100
                },
                // Note off at clock 24 (1 beat)
                LoopEvent {
                    clock_position: 24,
                    channel: 0,
                    message: vec![0x80, 60, 0], // Note Off, C4
                },
                // Note on at clock 48 (beat 3)
                LoopEvent {
                    clock_position: 48,
                    channel: 0,
                    message: vec![0x90, 64, 100], // Note On, E4
                },
                // Note off at clock 72 (beat 4)
                LoopEvent {
                    clock_position: 72,
                    channel: 0,
                    message: vec![0x80, 64, 0], // Note Off, E4
                },
            ],
        }
    }

    #[test]
    fn test_player_not_playing_returns_empty() {
        let mut player = LoopPlayer::new();
        player.load(make_test_loop());
        // Don't call start() - playing is false
        assert!(player.tick(0).is_empty());
        assert!(player.tick(10).is_empty());
    }

    #[test]
    fn test_player_no_loop_returns_empty() {
        let mut player = LoopPlayer::new();
        player.playing = true;
        assert!(player.tick(0).is_empty());
    }

    #[test]
    fn test_player_emits_events_at_correct_time() {
        let mut player = LoopPlayer::new();
        player.load(make_test_loop());
        player.start();

        // Clock 0: should get first note on
        let events = player.tick(0);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0], vec![0x90, 60, 100]);

        // Clock 1-23: no events
        for clock in 1..24 {
            assert!(player.tick(clock).is_empty());
        }

        // Clock 24: note off
        let events = player.tick(24);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0], vec![0x80, 60, 0]);
    }

    #[test]
    fn test_player_loops_correctly() {
        let mut player = LoopPlayer::new();
        player.load(make_test_loop());
        player.start();

        // Play through first iteration
        player.tick(0); // Note on
        player.tick(24); // Note off
        player.tick(48); // Note on
        player.tick(72); // Note off

        // At clock 96, we should loop back and get the first event again
        let events = player.tick(96);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0], vec![0x90, 60, 100]); // First note on again
    }

    #[test]
    fn test_player_reset() {
        let mut player = LoopPlayer::new();
        player.load(make_test_loop());
        player.start();

        // Advance partway through
        player.tick(0);
        player.tick(24);
        player.tick(48);

        // Reset
        player.reset();

        // Should get first event again at clock 0
        let events = player.tick(0);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0], vec![0x90, 60, 100]);
    }

    #[test]
    fn test_set_channel() {
        let mut loop_data = make_test_loop();
        loop_data.set_channel(5);

        // Check all events are on channel 5
        for event in &loop_data.events {
            assert_eq!(event.channel, 5);
            assert_eq!(event.message[0] & 0x0F, 5);
        }
    }

    #[test]
    fn test_multiple_events_same_clock() {
        let mut player = LoopPlayer::new();
        player.load(Loop {
            name: "test".to_string(),
            length_clocks: 96,
            events: vec![
                LoopEvent {
                    clock_position: 0,
                    channel: 0,
                    message: vec![0x90, 60, 100],
                },
                LoopEvent {
                    clock_position: 0,
                    channel: 0,
                    message: vec![0x90, 64, 100],
                },
                LoopEvent {
                    clock_position: 0,
                    channel: 0,
                    message: vec![0x90, 67, 100],
                },
            ],
        });
        player.start();

        // Should get all 3 events at clock 0
        let events = player.tick(0);
        assert_eq!(events.len(), 3);
    }

    #[test]
    fn test_empty_loop() {
        let mut player = LoopPlayer::new();
        player.load(Loop {
            name: "empty".to_string(),
            length_clocks: 96,
            events: vec![],
        });
        player.start();

        assert!(player.tick(0).is_empty());
        assert!(player.tick(96).is_empty());
    }
}
