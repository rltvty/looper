# Looper

A MIDI looper for dance music production. Syncs to external MIDI clock and plays back loops quantized to bar/beat positions.

## Project Structure

```
looper/
├── rust/           # Main application (Rust + iced GUI)
├── python/         # Utilities for extracting loops from Guitar Pro files
├── data/           # MIDI files and loop library
└── initial_plan.md # Original project goals and tech decisions
```

## Rust Application

The core looper application. Currently implements MIDI clock sync with a GUI showing BPM and position.

```bash
cd rust
cargo run
```

See [rust/ARCHITECTURE.md](rust/ARCHITECTURE.md) for detailed technical documentation.

### Requirements
- Rust toolchain (install via [rustup](https://rustup.rs))
- MIDI input source (hardware or virtual like IAC Driver on macOS)
- External MIDI clock (e.g., Ableton Live)

## Python Utilities

Tools for preparing loop content from Guitar Pro files.

```bash
cd python
uv run <script>
```

## Goals

From `initial_plan.md`:
- Sync to external MIDI clock or act as master
- Track elapsed bars/measures (4/4 time)
- Load and play MIDI files as quantized loops
- Loop chaining (A 4x, then B 4x, repeat)
- Cross-fade between loops via velocity
- GUI control interface
- Target: macOS now, Raspberry Pi later
