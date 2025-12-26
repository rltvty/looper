# MIDI Looper - Rust Architecture

## Project Overview

A MIDI looper application that syncs to external MIDI clock (e.g., Ableton Live) and plays back MIDI loops in time with the master clock. Built with Rust for performance and future Raspberry Pi deployment.

## Current State

**Implemented:**
- External MIDI clock sync (receives clock from DAW)
- Transport tracking (START/STOP/CONTINUE messages)
- Real-time BPM calculation using rolling 1-bar average
- Bar/beat position display
- MIDI file loading and parsing via midly
- Loop playback synced to external clock
- MIDI output to external devices
- GUI using iced framework

**Not Yet Implemented (from initial_plan.md):**
- Loop chaining (A 4x → B 4x → repeat)
- Cross-fading between loops via velocity
- Acting as clock master (currently only syncs to external clock)
- Dynamic loop loading from UI

## Module Structure

```
src/
├── main.rs      # Application entry point, GUI, MIDI I/O wiring
├── clock.rs     # Clock state management, BPM calculation, transport logic
├── midi.rs      # MIDI protocol constants, MIDI output wrapper
└── playback.rs  # Loop loading, event scheduling, playback engine
```

### main.rs
- `Looper` struct: iced application state
- `start_midi_listener()`: Connects MIDI input, wires up clock + playback callback
- Loads loop file at startup, sets MIDI channel
- GUI renders: MIDI I/O status, loop name, play/stop state, BPM, bar/beat position
- 50ms tick subscription triggers UI refresh

### clock.rs
- `ClockState`: Thread-safe state shared between MIDI callback and GUI
  - Uses `Arc<AtomicBool/AtomicU64>` for lock-free reads
  - Uses `Arc<Mutex<ClockTimeBuffer>>` for BPM timestamp buffer
- `ClockTimeBuffer`: Ring buffer storing last 96 clock timestamps (1 bar at 24 ppqn)
- `handle_midi_message()`: Core MIDI message processing

**Key behavior - Auto-start logic:**
1. If app connects while DAW is already playing (no START message), it auto-starts on first clock pulse
2. Once any transport message (START/STOP/CONTINUE) is received, `seen_transport` flag is set
3. After that, only explicit transport messages control playback (clock pulses don't auto-start)

### midi.rs
MIDI real-time message constants:
- `MIDI_CLOCK` (0xF8): 24 pulses per quarter note
- `MIDI_START` (0xFA): Reset to beginning, start playback
- `MIDI_CONTINUE` (0xFB): Resume from current position
- `MIDI_STOP` (0xFC): Stop playback

`MidiOut` struct: Wrapper for midir output connection
- Auto-selects IAC Driver if available
- `send()` method for transmitting MIDI messages

### playback.rs
- `LoopEvent`: Single MIDI event with clock position, channel, and raw bytes
- `Loop`: Loaded MIDI file with events sorted by clock position
  - `from_file()`: Parses MIDI file, converts ticks to 24 ppqn clock resolution
  - `set_channel()`: Override MIDI channel for all events
- `LoopPlayer`: Manages playback state
  - `tick()`: Called on each clock pulse, returns events to send
  - Handles loop wrapping automatically
  - `reset()`: Called on transport START to restart from beginning

**Playback flow:**
1. MIDI clock callback receives clock pulse
2. Updates `ClockState` with new position
3. Calls `LoopPlayer::tick(clock_count)`
4. Returns any events at current position
5. Sends events via `MidiOut`

## Dependencies

| Crate | Purpose | Status |
|-------|---------|--------|
| `midir` | MIDI hardware I/O | Used for clock input + note output |
| `midly` | MIDI file parsing | Used for loading loops |
| `iced` | GUI framework (v0.14) | Used, requires `tokio` feature |
| `tokio` | Async runtime | Required by iced |
| `anyhow` | Error handling | Included, not yet used |

## Technical Decisions

### Why iced 0.14 API pattern
```rust
iced::application(Looper::new, Looper::update, Looper::view)
    .title("MIDI Looper")
    .subscription(Looper::subscription)
    .theme(Looper::theme)
    .run()
```
iced 0.14 changed its API significantly. The application takes function references, not a struct implementing Application trait.

### Why fixed-point BPM storage
```rust
bpm_x100: Arc<AtomicU64>  // BPM * 100, e.g., 12050 = 120.50 BPM
```
Atomics don't support f64. Storing as integer × 100 gives 2 decimal precision without locks.

### Why 1-bar BPM window (96 clocks)
- 4-bar window (384 clocks) took too long to stabilize
- 1-bar provides quick response while smoothing jitter
- Partial buffer calculation shows BPM immediately, improves as buffer fills

### Why playback happens in MIDI callback
- Tight timing: events sent immediately when clock pulse received
- No scheduling jitter from GUI thread
- Uses `Arc<Mutex<LoopPlayer>>` for thread-safe access

### MIDI tick conversion
MIDI files use variable PPQ (pulses per quarter note), typically 480 or 960. The looper converts to 24 ppqn (MIDI clock resolution):
```rust
let clock_position = (file_tick * 24) / file_ppq;
```

## Testing

```bash
cargo test
```

18 tests total:
- **clock.rs** (10 tests): Transport handling, BPM calculation, position tracking
- **playback.rs** (8 tests): Event timing, loop wrapping, channel override, edge cases

## Running

```bash
cd rust
cargo run
```

**Requirements:**
- MIDI input device (or virtual port like IAC Driver on macOS)
- External clock source (e.g., Ableton Live sending MIDI clock)
- MIDI output routed back to DAW to hear the loop

**Current hardcoded loop:**
`../data/out/Rappers Delight - bass - Electric Bass finger - bars 13-16.mid`

## Next Steps (Suggested Priority)

1. **Loop selection UI** - Browse and load loops from data/out directory
2. **Loop chaining** - Play A 4x, then B 4x, repeat
3. **Multiple simultaneous loops** - Layer drums + bass
4. **Cross-fading** - Velocity-based blend between two loops
5. **Clock master mode** - Generate clock instead of syncing

## Platform Notes

- **macOS**: Primary development platform. Uses IAC Driver for virtual MIDI.
- **Raspberry Pi**: Future target. Rust cross-compiles well to ARM.
