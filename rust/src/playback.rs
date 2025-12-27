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

// ============ Slot-based Sequence Grid ============

/// A slot identifier (A-Z) for the sequence grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SlotId(pub char);

impl SlotId {
    /// All 26 slot IDs from A to Z.
    pub const ALL: [SlotId; 26] = [
        SlotId('A'), SlotId('B'), SlotId('C'), SlotId('D'), SlotId('E'),
        SlotId('F'), SlotId('G'), SlotId('H'), SlotId('I'), SlotId('J'),
        SlotId('K'), SlotId('L'), SlotId('M'), SlotId('N'), SlotId('O'),
        SlotId('P'), SlotId('Q'), SlotId('R'), SlotId('S'), SlotId('T'),
        SlotId('U'), SlotId('V'), SlotId('W'), SlotId('X'), SlotId('Y'),
        SlotId('Z'),
    ];

    /// Convert slot ID to array index (A=0, B=1, ..., Z=25).
    pub fn index(&self) -> usize {
        (self.0 as usize) - ('A' as usize)
    }

    /// Create a slot ID from an array index.
    pub fn from_index(idx: usize) -> Option<Self> {
        if idx < 26 {
            Some(SlotId((b'A' + idx as u8) as char))
        } else {
            None
        }
    }
}

impl std::fmt::Display for SlotId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A single slot in the sequence grid.
#[derive(Debug, Clone)]
pub struct SequenceSlot {
    /// Slot identifier (A-Z)
    pub id: SlotId,
    /// Optional loaded loop
    pub loop_data: Option<Loop>,
    /// Repeat count before advancing to next slot
    pub repeat_count: u32,
    /// Next slot to play (None = stop playback)
    pub next_slot: Option<SlotId>,
}

impl SequenceSlot {
    /// Create an empty slot with the given ID.
    pub fn empty(id: SlotId) -> Self {
        Self {
            id,
            loop_data: None,
            repeat_count: 1,
            next_slot: None,
        }
    }

    /// Get loop name or "--" for empty slots.
    pub fn loop_name(&self) -> &str {
        self.loop_data
            .as_ref()
            .map(|l| l.name.as_str())
            .unwrap_or("--")
    }

    /// Get length in bars or "--" for empty slots.
    pub fn length_bars(&self) -> String {
        self.loop_data
            .as_ref()
            .map(|l| {
                let bars = l.length_clocks / (4 * CLOCKS_PER_BEAT);
                format!("{}", bars)
            })
            .unwrap_or_else(|| "--".to_string())
    }

    /// Check if this slot has a loop loaded.
    pub fn has_loop(&self) -> bool {
        self.loop_data.is_some()
    }
}

/// Grid of 26 sequence slots (A-Z) with playback configuration.
#[derive(Debug, Clone)]
pub struct SequenceGrid {
    /// All 26 slots (A-Z)
    pub slots: [SequenceSlot; 26],
    /// Starting slot when playback begins
    pub start_slot: SlotId,
}

impl SequenceGrid {
    /// Create a new empty grid.
    pub fn new() -> Self {
        let slots: [SequenceSlot; 26] = std::array::from_fn(|i| {
            SequenceSlot::empty(SlotId::from_index(i).unwrap())
        });
        Self {
            slots,
            start_slot: SlotId('A'),
        }
    }

    /// Get a reference to a slot.
    pub fn get(&self, id: SlotId) -> &SequenceSlot {
        &self.slots[id.index()]
    }

    /// Get a mutable reference to a slot.
    pub fn get_mut(&mut self, id: SlotId) -> &mut SequenceSlot {
        &mut self.slots[id.index()]
    }

    /// Load a loop into a slot.
    pub fn load_loop(&mut self, id: SlotId, loop_data: Loop) {
        self.slots[id.index()].loop_data = Some(loop_data);
    }

    /// Clear a slot's loop.
    pub fn clear_loop(&mut self, id: SlotId) {
        self.slots[id.index()].loop_data = None;
    }

    /// Set the NEXT pointer for a slot.
    pub fn set_next(&mut self, id: SlotId, next: Option<SlotId>) {
        self.slots[id.index()].next_slot = next;
    }

    /// Set repeat count for a slot.
    pub fn set_repeat_count(&mut self, id: SlotId, count: u32) {
        self.slots[id.index()].repeat_count = count.max(1);
    }
}

impl Default for SequenceGrid {
    fn default() -> Self {
        Self::new()
    }
}

/// Playback state for UI display.
#[derive(Debug, Clone, Copy)]
pub struct PlaybackState {
    /// Currently playing slot
    pub current_slot: SlotId,
    /// Which iteration of the current slot (1-indexed for display)
    pub current_iteration: u32,
    /// Total repeat count for current slot
    pub total_iterations: u32,
}

/// An entry in a sequence: a loop with a repeat count.
#[derive(Debug, Clone)]
pub struct SequenceEntry {
    pub loop_data: Loop,
    /// How many times to play this loop before advancing
    pub repeat_count: u32,
}

/// A sequence of loops to play in order.
#[derive(Debug, Clone)]
pub struct Sequence {
    pub entries: Vec<SequenceEntry>,
}

/// Manages playback of a sequence of loops.
pub struct SequencePlayer {
    // Legacy sequence-based playback
    sequence: Option<Sequence>,
    /// Index of current entry in sequence
    current_entry_idx: usize,

    // Grid-based playback (new)
    grid: Option<SequenceGrid>,
    /// Current slot being played (for grid mode)
    current_slot: Option<SlotId>,

    // Shared state
    /// Which iteration of the current loop (0-indexed)
    current_iteration: u32,
    /// Index of next event to play in current loop
    next_event_idx: usize,
    /// Clock position when current loop iteration started
    loop_start_clock: u64,
    /// Whether playback is enabled
    pub playing: bool,
}

impl SequencePlayer {
    pub fn new() -> Self {
        Self {
            sequence: None,
            current_entry_idx: 0,
            grid: None,
            current_slot: None,
            current_iteration: 0,
            next_event_idx: 0,
            loop_start_clock: 0,
            playing: false,
        }
    }

    /// Load a sequence for playback.
    pub fn load(&mut self, sequence: Sequence) {
        self.sequence = Some(sequence);
        self.current_entry_idx = 0;
        self.current_iteration = 0;
        self.next_event_idx = 0;
        self.loop_start_clock = 0;
    }

    /// Start playback from the beginning.
    pub fn start(&mut self) {
        self.current_entry_idx = 0;
        self.current_iteration = 0;
        self.next_event_idx = 0;
        self.loop_start_clock = 0;
        self.playing = true;
    }

    /// Stop playback.
    pub fn stop(&mut self) {
        self.playing = false;
    }

    /// Reset to sequence start (called when transport restarts).
    pub fn reset(&mut self) {
        self.current_entry_idx = 0;
        self.current_iteration = 0;
        self.next_event_idx = 0;
        self.loop_start_clock = 0;
    }

    /// Get the name of the currently playing loop.
    pub fn current_loop_name(&self) -> Option<&str> {
        let sequence = self.sequence.as_ref()?;
        let entry = sequence.entries.get(self.current_entry_idx)?;
        Some(&entry.loop_data.name)
    }

    /// Get current playback state: (entry_index, current_iteration, repeat_count)
    pub fn current_state(&self) -> Option<(usize, u32, u32)> {
        let sequence = self.sequence.as_ref()?;
        let entry = sequence.entries.get(self.current_entry_idx)?;
        Some((self.current_entry_idx, self.current_iteration + 1, entry.repeat_count))
    }

    // ============ Grid-based playback methods ============

    /// Load a grid for playback (replaces legacy sequence).
    pub fn load_grid(&mut self, grid: SequenceGrid) {
        self.grid = Some(grid.clone());
        self.current_slot = Some(grid.start_slot);
        self.current_iteration = 0;
        self.next_event_idx = 0;
        self.loop_start_clock = 0;
        // Clear legacy sequence
        self.sequence = None;
    }

    /// Update the grid from UI (preserves playback position if possible).
    pub fn update_grid(&mut self, grid: SequenceGrid) {
        let old_slot = self.current_slot;
        self.grid = Some(grid);

        // Validate current slot still has a loop
        if let Some(slot_id) = old_slot {
            if self.grid.as_ref()
                .and_then(|g| g.get(slot_id).loop_data.as_ref())
                .is_none()
            {
                // Current slot no longer valid, reset to start
                self.reset_grid();
            }
        }
    }

    /// Reset grid playback to start slot.
    pub fn reset_grid(&mut self) {
        if let Some(ref grid) = self.grid {
            self.current_slot = Some(grid.start_slot);
        }
        self.current_iteration = 0;
        self.next_event_idx = 0;
        self.loop_start_clock = 0;
    }

    /// Get playback state for UI display (grid mode).
    pub fn grid_playback_state(&self) -> Option<PlaybackState> {
        let grid = self.grid.as_ref()?;
        let slot_id = self.current_slot?;
        let slot = grid.get(slot_id);

        Some(PlaybackState {
            current_slot: slot_id,
            current_iteration: self.current_iteration + 1,
            total_iterations: slot.repeat_count,
        })
    }

    /// Get the next slot that will play (for UI highlighting).
    pub fn next_slot_id(&self) -> Option<SlotId> {
        let grid = self.grid.as_ref()?;
        let current = self.current_slot?;
        grid.get(current).next_slot
    }

    /// Get current slot ID (for UI).
    pub fn current_slot_id(&self) -> Option<SlotId> {
        self.current_slot
    }

    /// Check if using grid mode.
    pub fn is_grid_mode(&self) -> bool {
        self.grid.is_some()
    }

    /// Called on each clock tick. Returns events that should be sent now.
    pub fn tick(&mut self, clock_count: u64) -> Vec<Vec<u8>> {
        if !self.playing {
            return Vec::new();
        }

        // Use grid mode if available
        if self.grid.is_some() {
            return self.tick_grid(clock_count);
        }

        // Legacy sequence mode
        let sequence = match &self.sequence {
            Some(s) => s,
            None => return Vec::new(),
        };

        if sequence.entries.is_empty() {
            return Vec::new();
        }

        let entry = &sequence.entries[self.current_entry_idx];
        let repeat_count = entry.repeat_count;
        let length_clocks = entry.loop_data.length_clocks;

        if entry.loop_data.events.is_empty() || length_clocks == 0 {
            return Vec::new();
        }

        // Calculate position within current loop
        let elapsed = clock_count.saturating_sub(self.loop_start_clock);
        let position_in_loop = elapsed % length_clocks;
        let iteration = elapsed / length_clocks;

        // Check if we need to advance to next entry
        if iteration >= repeat_count as u64 {
            self.advance_to_next_entry(clock_count);
            // Return events at position 0 of the new entry
            return self.collect_events_at_position(0);
        }

        // Check if we've wrapped to a new iteration within current loop
        if iteration as u32 > self.current_iteration {
            self.current_iteration = iteration as u32;
            self.next_event_idx = 0;
        }

        // Collect events at current position
        self.collect_events_at_position(position_in_loop)
    }

    /// Tick for grid-based playback.
    fn tick_grid(&mut self, clock_count: u64) -> Vec<Vec<u8>> {
        let grid = match &self.grid {
            Some(g) => g,
            None => return Vec::new(),
        };

        let slot_id = match self.current_slot {
            Some(id) => id,
            None => return Vec::new(),
        };

        let slot = grid.get(slot_id);
        let loop_data = match &slot.loop_data {
            Some(l) => l,
            None => return Vec::new(), // Empty slot, no events
        };

        let repeat_count = slot.repeat_count;
        let length_clocks = loop_data.length_clocks;

        if loop_data.events.is_empty() || length_clocks == 0 {
            return Vec::new();
        }

        // Calculate position within current loop
        let elapsed = clock_count.saturating_sub(self.loop_start_clock);
        let position_in_loop = elapsed % length_clocks;
        let iteration = elapsed / length_clocks;

        // Check if we need to advance to next slot
        if iteration >= repeat_count as u64 {
            self.advance_to_next_slot(clock_count);
            // Return events at position 0 of the new slot
            return self.collect_grid_events_at_position(0);
        }

        // Check if we've wrapped to a new iteration within current loop
        if iteration as u32 > self.current_iteration {
            self.current_iteration = iteration as u32;
            self.next_event_idx = 0;
        }

        // Collect events at current position
        self.collect_grid_events_at_position(position_in_loop)
    }

    fn advance_to_next_slot(&mut self, clock_count: u64) {
        if let Some(grid) = &self.grid {
            if let Some(current) = self.current_slot {
                let next = grid.get(current).next_slot;
                self.current_slot = next;
                // If next is None, playback stops (slot will be None)
            }
        }
        self.current_iteration = 0;
        self.next_event_idx = 0;
        self.loop_start_clock = clock_count;
    }

    fn collect_grid_events_at_position(&mut self, position: u64) -> Vec<Vec<u8>> {
        let grid = match &self.grid {
            Some(g) => g,
            None => return Vec::new(),
        };

        let slot_id = match self.current_slot {
            Some(id) => id,
            None => return Vec::new(),
        };

        let events_ref = match &grid.get(slot_id).loop_data {
            Some(l) => &l.events,
            None => return Vec::new(),
        };

        let mut events = Vec::new();
        while self.next_event_idx < events_ref.len() {
            let event = &events_ref[self.next_event_idx];
            if event.clock_position <= position {
                events.push(event.message.clone());
                self.next_event_idx += 1;
            } else {
                break;
            }
        }
        events
    }

    // Legacy sequence helpers
    fn advance_to_next_entry(&mut self, clock_count: u64) {
        let num_entries = self.sequence.as_ref().unwrap().entries.len();
        self.current_entry_idx = (self.current_entry_idx + 1) % num_entries;
        self.current_iteration = 0;
        self.next_event_idx = 0;
        self.loop_start_clock = clock_count;
    }

    fn collect_events_at_position(&mut self, position: u64) -> Vec<Vec<u8>> {
        let events_ref = &self.sequence.as_ref().unwrap().entries[self.current_entry_idx]
            .loop_data
            .events;

        let mut events = Vec::new();
        while self.next_event_idx < events_ref.len() {
            let event = &events_ref[self.next_event_idx];
            if event.clock_position <= position {
                events.push(event.message.clone());
                self.next_event_idx += 1;
            } else {
                break;
            }
        }
        events
    }
}

impl Default for SequencePlayer {
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
    fn test_set_channel() {
        let mut loop_data = make_test_loop();
        loop_data.set_channel(5);

        // Check all events are on channel 5
        for event in &loop_data.events {
            assert_eq!(event.channel, 5);
            assert_eq!(event.message[0] & 0x0F, 5);
        }
    }

    // ============ Sequence Player Tests ============

    fn make_test_loop_named(name: &str, note: u8) -> Loop {
        Loop {
            name: name.to_string(),
            length_clocks: 96, // 1 bar
            events: vec![
                LoopEvent {
                    clock_position: 0,
                    channel: 0,
                    message: vec![0x90, note, 100],
                },
                LoopEvent {
                    clock_position: 48,
                    channel: 0,
                    message: vec![0x80, note, 0],
                },
            ],
        }
    }

    #[test]
    fn test_sequence_not_playing_returns_empty() {
        let mut player = SequencePlayer::new();
        player.load(Sequence {
            entries: vec![SequenceEntry {
                loop_data: make_test_loop(),
                repeat_count: 2,
            }],
        });
        // Don't call start() - playing is false
        assert!(player.tick(0).is_empty());
    }

    #[test]
    fn test_sequence_no_sequence_returns_empty() {
        let mut player = SequencePlayer::new();
        player.playing = true;
        assert!(player.tick(0).is_empty());
    }

    #[test]
    fn test_sequence_plays_first_entry() {
        let mut player = SequencePlayer::new();
        player.load(Sequence {
            entries: vec![
                SequenceEntry {
                    loop_data: make_test_loop_named("loop1", 60),
                    repeat_count: 2,
                },
                SequenceEntry {
                    loop_data: make_test_loop_named("loop2", 64),
                    repeat_count: 2,
                },
            ],
        });
        player.start();

        // Should get first loop's first event (note 60)
        let events = player.tick(0);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0], vec![0x90, 60, 100]);
    }

    #[test]
    fn test_sequence_repeats_before_advancing() {
        let mut player = SequencePlayer::new();
        player.load(Sequence {
            entries: vec![
                SequenceEntry {
                    loop_data: make_test_loop_named("loop1", 60),
                    repeat_count: 2,
                },
                SequenceEntry {
                    loop_data: make_test_loop_named("loop2", 64),
                    repeat_count: 2,
                },
            ],
        });
        player.start();

        // First iteration of loop1
        player.tick(0); // Note on 60
        player.tick(48); // Note off 60

        // Second iteration of loop1 (at clock 96)
        let events = player.tick(96);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0], vec![0x90, 60, 100]); // Still loop1's note
    }

    #[test]
    fn test_sequence_advances_after_repeat_count() {
        let mut player = SequencePlayer::new();
        player.load(Sequence {
            entries: vec![
                SequenceEntry {
                    loop_data: make_test_loop_named("loop1", 60),
                    repeat_count: 2,
                },
                SequenceEntry {
                    loop_data: make_test_loop_named("loop2", 64),
                    repeat_count: 2,
                },
            ],
        });
        player.start();

        // Play through loop1 twice (2 bars = 192 clocks)
        player.tick(0); // Bar 1 note on
        player.tick(48); // Bar 1 note off
        player.tick(96); // Bar 2 note on
        player.tick(144); // Bar 2 note off

        // At clock 192, should advance to loop2
        let events = player.tick(192);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0], vec![0x90, 64, 100]); // Loop2's note
    }

    #[test]
    fn test_sequence_cycles_back_to_first() {
        let mut player = SequencePlayer::new();
        player.load(Sequence {
            entries: vec![
                SequenceEntry {
                    loop_data: make_test_loop_named("loop1", 60),
                    repeat_count: 1, // Just 1 repeat each
                },
                SequenceEntry {
                    loop_data: make_test_loop_named("loop2", 64),
                    repeat_count: 1,
                },
            ],
        });
        player.start();

        // Loop1 (96 clocks)
        player.tick(0);
        player.tick(48);

        // Loop2 starts at 96
        let events = player.tick(96);
        assert_eq!(events[0], vec![0x90, 64, 100]);
        player.tick(144);

        // Back to loop1 at 192
        let events = player.tick(192);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0], vec![0x90, 60, 100]); // Back to loop1
    }

    #[test]
    fn test_sequence_reset() {
        let mut player = SequencePlayer::new();
        player.load(Sequence {
            entries: vec![
                SequenceEntry {
                    loop_data: make_test_loop_named("loop1", 60),
                    repeat_count: 1,
                },
                SequenceEntry {
                    loop_data: make_test_loop_named("loop2", 64),
                    repeat_count: 1,
                },
            ],
        });
        player.start();

        // Advance to loop2
        player.tick(0);
        player.tick(96); // Now on loop2

        // Reset
        player.reset();

        // Should be back at loop1
        let events = player.tick(0);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0], vec![0x90, 60, 100]);
    }

    #[test]
    fn test_sequence_current_loop_name() {
        let mut player = SequencePlayer::new();
        player.load(Sequence {
            entries: vec![
                SequenceEntry {
                    loop_data: make_test_loop_named("First Loop", 60),
                    repeat_count: 1,
                },
                SequenceEntry {
                    loop_data: make_test_loop_named("Second Loop", 64),
                    repeat_count: 1,
                },
            ],
        });
        player.start();

        assert_eq!(player.current_loop_name(), Some("First Loop"));

        // Advance to second loop
        player.tick(0);
        player.tick(96);

        assert_eq!(player.current_loop_name(), Some("Second Loop"));
    }

    #[test]
    fn test_sequence_current_state() {
        let mut player = SequencePlayer::new();
        player.load(Sequence {
            entries: vec![SequenceEntry {
                loop_data: make_test_loop_named("test", 60),
                repeat_count: 3,
            }],
        });
        player.start();

        // First iteration
        assert_eq!(player.current_state(), Some((0, 1, 3)));

        // Tick through first iteration
        player.tick(0);
        player.tick(96); // Now on second iteration

        assert_eq!(player.current_state(), Some((0, 2, 3)));

        player.tick(192); // Third iteration
        assert_eq!(player.current_state(), Some((0, 3, 3)));
    }

    #[test]
    fn test_sequence_emits_events_at_correct_time() {
        let mut player = SequencePlayer::new();
        player.load(Sequence {
            entries: vec![SequenceEntry {
                loop_data: make_test_loop(), // Has events at 0, 24, 48, 72
                repeat_count: 1,
            }],
        });
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
    fn test_sequence_multiple_events_same_clock() {
        let mut player = SequencePlayer::new();
        player.load(Sequence {
            entries: vec![SequenceEntry {
                loop_data: Loop {
                    name: "chord".to_string(),
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
                },
                repeat_count: 1,
            }],
        });
        player.start();

        // Should get all 3 events at clock 0
        let events = player.tick(0);
        assert_eq!(events.len(), 3);
    }

    #[test]
    fn test_sequence_empty_entries() {
        let mut player = SequencePlayer::new();
        player.load(Sequence { entries: vec![] });
        player.start();

        assert!(player.tick(0).is_empty());
        assert!(player.tick(96).is_empty());
    }

    #[test]
    fn test_sequence_with_empty_loop() {
        let mut player = SequencePlayer::new();
        player.load(Sequence {
            entries: vec![SequenceEntry {
                loop_data: Loop {
                    name: "empty".to_string(),
                    length_clocks: 96,
                    events: vec![],
                },
                repeat_count: 2,
            }],
        });
        player.start();

        assert!(player.tick(0).is_empty());
        assert!(player.tick(96).is_empty());
    }
}
