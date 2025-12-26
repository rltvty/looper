# Looper Project Notes

## Goal

Build an AI-assisted workflow to extract interesting instrument loops from MIDI files, using Guitar Pro metadata to inform extraction decisions. Target use case: finding bass, drum, and lead loops suitable for house/techno production.

## File Formats

### Guitar Pro 8 (.gp)

GP8 files are ZIP archives containing XML. Key file: `Content/score.gpif`

**Useful metadata:**
- Song title, artist
- Tempo (BPM)
- Time signature per bar
- Track definitions with instrument types
- Section markers (Intro, Verse, Chorus, etc.) with bar numbers
- Repeat markers

**Parsing approach:**
```python
import zipfile
import xml.etree.ElementTree as ET

with zipfile.ZipFile('song.gp', 'r') as z:
    xml_data = z.read('Content/score.gpif')
root = ET.fromstring(xml_data)
```

**Key XML paths:**
- `//Title` - song title
- `//Artist` - artist name
- `//Automation[Type="Tempo"]/Value` - tempo (format: "120 2")
- `//Track` - track definitions (id, Name, InstrumentSet/Name)
- `//MasterBar` - bar definitions with time signatures
- `//MasterBar//Section/Text` - section markers

### MIDI Files (.mid)

Standard MIDI format 1 (multiple synchronized tracks).

**Key properties:**
- `ticks_per_beat` - timing resolution (16384 common from GP export)
- Track names (sometimes match GP, sometimes empty)
- Note events, program changes, tempo changes

**Library:** `mido`

## Sample File Analysis

### Rapper's Delight (110 BPM, 107 bars)

| Track | GP Name | Instrument | Loop Potential |
|-------|---------|------------|----------------|
| 0 | (unnamed) | Electric Bass (finger) | HIGH - iconic bassline |
| 1 | (unnamed) | Electric Guitar (jazz) | MEDIUM - rhythm guitar |
| 2 | Bongos | Drums | MEDIUM - percussion |
| 3 | Cowbell | Drums | LOW - accent |
| 4 | Drum Machine | Drums | HIGH - main beat |
| 5 | (unnamed) | Acoustic Grand Piano | MEDIUM - chords |

**Sections:**
- Bar 1-4: Intro
- Bar 5-12: Bass Riff (prime loop material!)
- Bar 13-20: Verse1
- Bar 21-29: Chorus
- Bar 30+: Verse 2

### Seven Nation Army (124 BPM, 117 bars)

| Track | GP Name | Instrument | Loop Potential |
|-------|---------|------------|----------------|
| 0 | Jack White (Lead Guitar) | Overdriven Guitar | HIGH - main riff |
| 1 | Jack White (Rhythm Guitar) | Overdriven Guitar | MEDIUM |
| 2 | Jack White (Solo Guitar) | Distortion Guitar | LOW - solo sections |
| 3 | Jack White (Bass Immitation) | Electric Guitar (muted) | HIGH - bass-like |
| 4 | Jack White (Bass Immitation Overdrive) | Overdriven Guitar | HIGH - bass-like |
| 5 | Meg White | Drums | HIGH - simple beat |
| 6 | (unnamed) | Electric Bass (finger) | HIGH |

### The Message (100 BPM, 181 bars)

| Track | GP Name | Instrument | Loop Potential |
|-------|---------|------------|----------------|
| 0 | (unnamed) | Electric Guitar (clean) | MEDIUM - lead line |
| 1 | (unnamed) | Synth Bass 1 | HIGH - synth bass |
| 2 | (unnamed) | Drums | HIGH |

No section markers in GP file.

## Track Type Detection Heuristics

Based on instrument names from GP files:

**Bass (HIGH priority):**
- Contains "bass" (case insensitive)
- Synth Bass, Electric Bass, etc.

**Drums (HIGH priority):**
- Contains "drum" or instrument type is "Drums"
- Percussion instruments (bongos, cowbell, etc.)

**Lead (MEDIUM priority):**
- Guitar with "lead", "solo", or "overdriven"
- Synth leads
- Piano/keys (potential chord stabs)

**Skip:**
- Rhythm guitar (unless interesting)
- Backing vocals
- Sound effects

## Loop Extraction Strategy

### 1. Section-Based Extraction
When GP file has section markers:
- Extract each section as potential loop
- Prioritize: Intro, Riff, Hook sections
- Standard lengths: 2, 4, 8 bars

### 2. Pattern Detection
When no sections or for finer control:
- Analyze note patterns for repetition
- Find loop points where patterns repeat
- Common loop lengths in dance music: 1, 2, 4, 8 bars

### 3. Bar Boundary Calculation

```python
def ticks_per_bar(ticks_per_beat, numerator=4, denominator=4):
    return ticks_per_beat * numerator * (4 / denominator)

def bar_to_ticks(bar_number, ticks_per_beat, time_sig=(4,4)):
    return (bar_number - 1) * ticks_per_bar(ticks_per_beat, *time_sig)
```

### 4. Quality Signals for "Interesting" Loops

- Note density (not too sparse, not too busy)
- Rhythmic consistency (quantized feel)
- Melodic movement (for bass/leads)
- Clear start/end points (loop-friendly)

## Pattern Analysis Findings

### Rapper's Delight Bass Track

The bass enters at bar 1-2 (intro), then bars 5-8, with the main groove starting at bar 9.

**Detected repeating patterns:**
- Bars 9, 13, 17 share identical 3-note pattern (root notes)
- Bars 10, 14, 18 share identical 10-note pattern (main bass run)
- Bars 11, 15, 19 share identical 3-note pattern
- This suggests a **4-bar loop** from bars 9-12 that repeats

**Loop recommendation:** Extract bars 9-12 as a 4-bar bass loop

### Rapper's Delight Drum Machine Track

Very consistent 16th-note hi-hat pattern throughout.

**Note distribution:**
- Note 42 (Closed HH): 624 hits - constant 16th notes
- Note 36 (Kick): 104 hits - on beats
- Note 39 (Clap/Snare): 64 hits - backbeat
- Note 46 (Open HH): 16 hits - accents

**Density analysis:**
- Bars 1-8: 16 notes/bar (simpler intro pattern)
- Bars 9+: 21 notes/bar (full groove)

**Loop recommendation:** Extract bars 9-12 as a 4-bar drum loop (matches bass)

## Ideas for AI-Assisted Workflow

### Phase 1: Metadata Extraction & Basic Slicing
- Parse GP8 files for all metadata
- Map MIDI tracks to GP tracks
- Extract loops at section boundaries
- Filter by instrument type (bass, drums, leads)

### Phase 2: Pattern Analysis
- Detect repeating patterns within tracks
- Score loops by "interestingness"
- Suggest optimal loop points

### Phase 3: AI Evaluation (future)
- Use audio rendering + AI to evaluate musical quality
- Train on user preferences (which loops get used)
- Automatic BPM/key detection for compatibility

## Output Format

Current outputs use:
- 96 ticks per beat (standard)
- Single track per file
- Named by song + instrument type
- Program change for instrument sound

## Project Structure

```
looper/
├── in/                  # Input GP + MIDI file pairs
├── out/                 # Tool-generated loop extractions
├── sample_out/          # Manually created reference loops
├── looper.py            # Main CLI tool
├── NOTES.md             # This file
└── pyproject.toml       # uv project config
```

## Dependencies

```
mido>=1.3.3
pyguitarpro>=0.10.1  # Note: only supports GP3-5, not GP7+
```

For GP7+ files, use direct XML parsing via zipfile + xml.etree.

## Current Implementation: looper.py

A CLI tool for analyzing and extracting loops from MIDI files using GP metadata.

### Usage

**Analyze a song:**
```bash
uv run python looper.py analyze <gp_file> <midi_file>
```

Example output:
```
============================================================
Rapper's Delight - The Sugarhill Gang
============================================================
Tempo: 110 BPM | Bars: 107
Ticks/beat: 16384

Tracks:
  [0] 🎸 (unnamed)                 Electric Bass (finger)         (228 notes)
  [1] 🎵 (unnamed)                 Electric Guitar (jazz)         (462 notes)
  [2] 🥁 Bongos                    Drums                          (72 notes)
  ...

Sections:
  Bar   1-  4: Intro
  Bar   5- 12: Bass Riff
  ...

Pattern Analysis:
  Electric Bass (finger) (track 0):
    Bars [9, 13, 17, 21, 25, 29]: 3 notes
    Bars [10, 14, 18, 22, 26, 30]: 10 notes
```

**Extract a loop:**
```bash
uv run python looper.py extract <gp_file> <midi_file> --track <index|type> --bars <start>-<end> [--output <path>]
```

Examples:
```bash
# Extract by track type
uv run python looper.py extract in/song.gp in/song.mid --track bass --bars 9-12

# Extract by track index
uv run python looper.py extract in/song.gp in/song.mid --track 4 --bars 1-8

# Custom output path
uv run python looper.py extract in/song.gp in/song.mid --track drums --bars 9-12 --output out/my-loop.mid
```

**Get loop suggestions:**
```bash
uv run python looper.py suggest <gp_file> <midi_file> [--extract] [--top N]
```

Example output:
```
============================================================
Loop Suggestions: Rapper's Delight
============================================================
Tempo: 110 BPM

 1. 🎸 [BASS] Bars 13-16 (4 bars)
    Track: Electric Bass (finger)
    Score: 90 - Pattern repeats 7x, Bass track, 4-bar loop, Good density (6.8 notes/bar)

 2. 🥁 [DRUMS] Bars 1-4 (4 bars)
    Track: Drum Machine
    Score: 85 - Pattern repeats 5x, Drum track, 4-bar loop, Good density (16.0 notes/bar)
...
```

Use `--extract` to automatically extract all suggested loops to `out/`:
```bash
uv run python looper.py suggest in/song.gp in/song.mid --extract --top 5
```

### Features Implemented

- [x] GP8 XML parsing (title, artist, tempo, sections, tracks)
- [x] Track type classification (bass, drums, lead, other)
- [x] MIDI track correlation with GP tracks
- [x] Section boundary detection from GP markers
- [x] Pattern detection (finds repeating bar sequences)
- [x] Loop extraction by bar range
- [x] Output normalization (96 ticks/beat standard)
- [x] Loop suggestion with scoring
- [x] Auto-extraction of suggested loops

### Scoring System

The `suggest` command scores loops based on:

| Factor | Points | Notes |
|--------|--------|-------|
| Pattern repetition | 8 per repeat (max 40) | More repeats = core groove |
| Bass track | +25 | High priority for dance music |
| Drum track | +20 | High priority for dance music |
| Lead track | +15 | Medium priority |
| 4-bar loop | +15 | Ideal length |
| 8-bar loop | +10 | Good length |
| Section name "riff/groove/hook" | +30 | Strong indicator |
| Section name "intro/verse/chorus" | +20 | Good indicator |
| Good note density (4-20/bar) | +10 | Not too sparse/busy |

**Note:** Minimum loop length is 4 bars. Even if a pattern is 1 or 2 bars repeating, we extract 4 bars to capture a complete musical phrase that's easier to use in production.

### Key Functions

| Function | Description |
|----------|-------------|
| `parse_gp8(path)` | Parse GP8 file, return `SongInfo` with metadata |
| `enrich_with_midi(song_info, path)` | Add MIDI note counts and program info |
| `classify_instrument(name, instrument)` | Classify track as bass/drums/lead/other |
| `analyze_patterns(path, track, ticks)` | Find repeating bar patterns |
| `suggest_loops(song_info, midi_path)` | Generate scored loop suggestions |
| `extract_loop(...)` | Extract bar range from track to new MIDI file |

## Next Steps

1. [x] Build GP8 parser module
2. [x] Build MIDI track mapper (GP track <-> MIDI track)
3. [x] Implement section-based loop extraction
4. [x] Add instrument type filtering
5. [x] Add pattern detection for files without sections
6. [x] Add `suggest` command to auto-recommend loops
7. [x] Add batch extraction (all interesting loops from a song)
8. [ ] Batch processing multiple songs at once
9. [ ] Improve pattern detection (rhythmic position awareness)
10. [ ] LLM integration for "danceability" evaluation
11. [ ] Learn from user preferences (track which extractions get used)
