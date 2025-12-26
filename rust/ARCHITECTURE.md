# MIDI Looper - Rust Architecture

## Project Overview

A MIDI looper application that syncs to external MIDI clock (e.g., Ableton Live) and will eventually play back MIDI loops in time with the master clock. Built with Rust for performance and future Raspberry Pi deployment.

## Current State

**Implemented:**
- External MIDI clock sync (receives clock from DAW)
- Transport tracking (START/STOP/CONTINUE messages)
- Real-time BPM calculation using rolling 1-bar average
- Bar/beat position display
- GUI using iced framework

**Not Yet Implemented (from initial_plan.md):**
- MIDI file loading and parsing (midly crate is included but unused)
- Loop playback with quantization
- Loop chaining (A 4x → B 4x → repeat)
- Cross-fading between loops via velocity
- MIDI output to external devices
- Acting as clock master (currently only syncs to external clock)

## Module Structure

```
src/
├── main.rs     # Application entry point, GUI, MIDI connection setup
├── clock.rs    # Clock state management, BPM calculation, transport logic
└── midi.rs     # MIDI protocol constants
```

### main.rs
- `Looper` struct: iced application state
- `start_midi_listener()`: Connects to MIDI input (prefers IAC Driver on macOS)
- GUI renders: connection status, play/stop state, BPM, bar/beat position
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

## Dependencies

| Crate | Purpose | Status |
|-------|---------|--------|
| `midir` | MIDI hardware I/O | Used for clock input |
| `midly` | MIDI file parsing | Included, not yet used |
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

## Testing

All tests are in `clock.rs`:
```bash
cargo test
```

10 tests covering:
- Initial state
- Auto-start behavior
- Transport message handling (START/STOP/CONTINUE)
- Position calculation
- BPM calculation accuracy
- Edge cases (empty messages, unknown messages)

## Running

```bash
cd rust
cargo run
```

Requires:
- MIDI input device (or virtual port like IAC Driver on macOS)
- External clock source (e.g., Ableton Live sending MIDI clock)

## Next Steps (Suggested Priority)

1. **MIDI file loading** - Use `midly` to parse MIDI files from `../data/` directory
2. **Loop playback engine** - Schedule MIDI events relative to current bar/beat position
3. **Quantization** - Stretch/compress loops to match current tempo
4. **MIDI output** - Send notes to external devices
5. **Loop chaining UI** - Interface for sequencing loops

## Platform Notes

- **macOS**: Primary development platform. Uses IAC Driver for virtual MIDI.
- **Raspberry Pi**: Future target. Rust cross-compiles well to ARM.
