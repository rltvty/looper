# MIDI Looper Project - Context Summary

## Project Goal
Build a MIDI looper application that:
- Either runs as the primary MIDI clock, and/or syncs to an external clock
- Sends and/or recieves MIDI transport info
- Maintains a count of the elapsed bars & measures (assumes 4:4 to start)
- Reads MIDI files
- Plays MIDI files in a loop fashion, quantized & time-stretched to bars & measures.
- Provides some interface for creating loop chains. Example: Play loop A 4 times, then loop B 4 times, then repeat the sequence.
- Provides some interface for cross-fading between two loops (midi velocity) playing in sync simultatneously.
- Sends MIDI notes to external MIDI devices (software or hardware)
- Provides a UI for control
- Runs on Mac initially, with future embedded deployment on Raspberry Pi

## Technology Decision: Rust

We evaluated two paths and chose **Rust** over JUCE (C++) for the following reasons:

### Why Rust
1. **Embedded support** - Excellent cross-compilation to Raspberry Pi, well-documented toolchain
2. **Clean MIDI ecosystem** - `midir` for hardware I/O, `midly` for file parsing
3. **Debugging** - Compiler catches most issues at build time; clear error messages
4. **Fast iteration** - `cargo` makes dependency management and builds trivial
5. **Safe concurrency** - Critical for timing-sensitive MIDI applications
6. **Modern GUI options** - `egui` (quick iteration), `iced` (polished), or `tauri` (web-based UI)

### When JUCE would have been better
- Complex audio processing (effects, synthesis)
- Commercial plugin development (VST/AU)
- DAW-style applications with complex audio routing

## Key Libraries to Use
| Purpose | Crate |
|---------|-------|
| MIDI hardware I/O | `midir` |
| MIDI file parsing | `midly` |
| GUI  | `iced` |

## Target Platforms
- **Primary**: macOS (development machine)
- **Future**: Raspberry Pi (embedded deployment)

## Next Steps
- Define architecture for the looper (file loading, scheduling, playback, UI)
- Set up Rust project with initial dependencies
- Prototype MIDI clock & transport sync

