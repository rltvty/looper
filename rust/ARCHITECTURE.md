# MIDI Looper - Rust Architecture

## Project Overview

A MIDI looper application that syncs to external MIDI clock (e.g., Ableton Live) and plays back MIDI loops in time with the master clock. Built with Rust for performance and future Raspberry Pi deployment.

## Current State

**Implemented:**
- External MIDI clock sync (receives clock from DAW)
- Transport tracking (START/STOP/CONTINUE messages)
- Transport control (Play/Stop buttons send MIDI transport messages)
- Real-time BPM calculation using rolling 1-bar average
- Bar/beat position display
- MIDI file loading and parsing via midly
- Loop playback synced to external clock
- MIDI output to external devices
- GUI using iced framework with transport control buttons
- Sequence playback (loop chaining with configurable repeat counts)

**Not Yet Implemented (from initial_plan.md):**
- Cross-fading between loops via velocity
- Dynamic loop loading from UI
- Configurable BPM in master mode (currently fixed at 120 BPM)

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
- GUI renders: MIDI I/O status, clock mode, loop name, transport controls, BPM, bar/beat position
- Clock mode toggle (External/Master):
  - External: Syncs to incoming MIDI clock from DAW
  - Master: Generates 120 BPM clock, drives external devices
  - Background thread generates clock pulses in master mode
- Transport control buttons:
  - Play: Sends START (or STOP+START if already playing)
  - Stop: Sends STOP
  - Button colors: Play=green/grey, Stop=grey/red based on transport state
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
- `send_start()` / `send_stop()` for transport control

### playback.rs
- `LoopEvent`: Single MIDI event with clock position, channel, and raw bytes
- `Loop`: Loaded MIDI file with events sorted by clock position
  - `from_file()`: Parses MIDI file, converts ticks to 24 ppqn clock resolution
  - `set_channel()`: Override MIDI channel for all events
- `SequenceEntry`: A loop paired with its repeat count
- `Sequence`: Ordered list of `SequenceEntry` items
- `SequencePlayer`: Manages playback of a sequence
  - `tick()`: Returns events at current position, handles loop repeats and sequence advancement
  - `reset()`: Called on transport START to restart from beginning
  - `current_loop_name()`: Returns name of currently playing loop
  - `current_state()`: Returns (entry_index, current_iteration, repeat_count)
  - Automatically cycles back to first entry after last entry completes

**Playback flow:**
1. MIDI clock callback receives clock pulse
2. Updates `ClockState` with new position
3. Calls `SequencePlayer::tick(clock_count)`
4. Returns any events at current position
5. Sends events via `MidiOut`
6. When repeat count reached, advances to next sequence entry

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
- Uses `Arc<Mutex<SequencePlayer>>` for thread-safe access

### MIDI tick conversion
MIDI files use variable PPQ (pulses per quarter note), typically 480 or 960. The looper converts to 24 ppqn (MIDI clock resolution):
```rust
let clock_position = (file_tick * 24) / file_ppq;
```

### Clock master mode architecture
In master mode, a background thread generates clock pulses:
- Calculates absolute target time for each tick: `target_nanos = clock_count * 60_000_000_000 / (BPM * 24)`
- Sleeps until target time (avoids cumulative drift from individual sleep jitter)
- Calls `ClockState::handle_midi_message()` to update internal position
- Sends MIDI clock + note events to output
- MIDI input callback ignores incoming clock/transport in master mode
- `Arc<AtomicBool>` flag for lock-free mode switching

## Debugging Tools

### midi_monitor
A CLI tool for debugging MIDI messages:
```bash
cargo run --bin midi_monitor -- --duration 10
```
- Displays all incoming MIDI messages with timestamps, type, hex bytes, and human-readable details
- Useful for debugging MIDI routing issues
- `--duration <secs>` sets monitoring duration (optional)

## Testing

```bash
cargo test
```

24 tests total:
- **clock.rs** (10 tests): Transport handling, BPM calculation, position tracking
- **playback.rs** (14 tests): Channel override, sequence playback, event timing, advancement, cycling

## Running

```bash
cd rust
cargo run
```

**Requirements:**
- MIDI input device (or virtual port like IAC Driver on macOS)
- In External mode: clock source (e.g., Ableton Live sending MIDI clock)
- In Master mode: no external clock needed (looper generates 120 BPM)
- MIDI output routed back to DAW to hear the loop

**Current hardcoded sequence (2x each, then cycles):**
1. `Billie Jean - bass - Bass finger - bars 15-26.mid` (12 bars)
2. `Psycho Killer - bass - Bass - Tina Weymouth - bars 107-110.mid` (4 bars)
3. `Rappers Delight - bass - Electric Bass finger - bars 13-16.mid` (4 bars)
4. `Seven Nation Army With Bass Guitar - bass - Jack White Bass Immitation - bars 1-4.mid` (4 bars)

## Next Steps (Suggested Priority)

1. **Configurable BPM** - Adjust tempo in master mode (currently fixed at 120)
2. **Loop selection UI** - Browse and load loops from data/out directory
3. **Multiple simultaneous loops** - Layer drums + bass
4. **Cross-fading** - Velocity-based blend between two loops

## Platform Notes

- **macOS**: Primary development platform. Uses IAC Driver for virtual MIDI.
- **Raspberry Pi**: Future target. Rust cross-compiles well to ARM.
