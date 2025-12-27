# Related Crates

Reference for MIDI-related Rust crates relevant to this project.

## Currently Used

### midly
**MIDI file parsing** - https://docs.rs/midly

We use this to load MIDI files and extract note events. Lightweight, no-std compatible.

```rust
let data = std::fs::read("file.mid")?;
let smf = midly::Smf::parse(&data)?;
// Access header.timing for PPQ, tracks for events
```

### midir
**MIDI I/O** - https://docs.rs/midir

Cross-platform MIDI input/output. We use it for:
- Receiving external clock (MidiInput)
- Sending notes and clock to DAW (MidiOutput)

```rust
let midi_out = MidiOutput::new("app-name")?;
let conn = midi_out.connect(&port, "port-name")?;
conn.send(&[0x90, 60, 100])?; // Note on
```

## Evaluated

### nodi
**MIDI playback abstraction** - https://docs.rs/nodi

By the same author as midly. Provides:
- `Sheet` - organizes events by tick, can merge parallel tracks
- `Timer` - calculates sleep durations between ticks
- `Player` - self-driven playback (sleep-based)
- `Connection` trait - output abstraction

**Why we don't use it:** Nodi uses self-driven timing (sleeps between events). Our looper is clock-driven (responds to external MIDI clock pulses or generates them). These are fundamentally different timing models.

**Potentially useful for:**
- `Sheet::parallel()` - merging multiple tracks when we add multi-track support
- Reference for event organization patterns

## Not Yet Evaluated

### wmidi
**MIDI message types** - https://docs.rs/wmidi

Strongly-typed MIDI message parsing/creation. Could replace our raw byte handling:
```rust
let msg = MidiMessage::NoteOn(Channel::Ch1, Note::C4, Velocity::MAX);
```

### cpal
**Cross-platform audio** - https://docs.rs/cpal

Low-level audio I/O. Not needed for MIDI-only looper, but would be required if we ever wanted to:
- Generate audio directly (software synth)
- Handle audio alongside MIDI

### rodio
**Audio playback** - https://docs.rs/rodio

Higher-level audio playback built on cpal. Same considerations as cpal.
