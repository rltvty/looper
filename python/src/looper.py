"""
Looper - Extract instrument loops from MIDI files using Guitar Pro metadata.

Usage (from python/ directory):
    uv run python src/looper.py analyze ../data/in/song.gp ../data/in/song.mid
    uv run python src/looper.py extract ../data/in/song.gp ../data/in/song.mid --track bass --bars 9-12
    uv run python src/looper.py suggest ../data/in/song.gp ../data/in/song.mid [--extract]
"""

import zipfile
import xml.etree.ElementTree as ET
from dataclasses import dataclass, field
from pathlib import Path
from typing import Optional
import mido

# Data directory is at repo root, two levels up from this file
DATA_DIR = Path(__file__).parent.parent.parent / "data"
OUT_DIR = DATA_DIR / "out"


# GM Drum Map (standard MIDI note numbers)
# https://www.midi.org/specifications-old/item/gm-level-1-sound-set
GM_DRUMS = {
    'kick': 36,        # C1 - Bass Drum 1
    'snare': 38,       # D1 - Acoustic Snare
    'sidestick': 37,   # C#1 - Side Stick
    'clap': 39,        # D#1 - Hand Clap
    'closed_hh': 42,   # F#1 - Closed Hi-Hat
    'open_hh': 46,     # A#1 - Open Hi-Hat
    'pedal_hh': 44,    # G#1 - Pedal Hi-Hat
    'crash': 49,       # C#2 - Crash Cymbal 1
    'ride': 51,        # D#2 - Ride Cymbal 1
    'tom_low': 41,     # F1 - Low Floor Tom
    'tom_mid': 47,     # B1 - Low-Mid Tom
    'tom_high': 50,    # D2 - High Tom
    'cowbell': 56,     # G#2 - Cowbell
    'bongo_hi': 60,    # C3 - Hi Bongo
    'bongo_lo': 61,    # C#3 - Low Bongo
    'conga_hi': 62,    # D3 - Mute Hi Conga
    'conga_lo': 64,    # E3 - Low Conga
    'tambourine': 54,  # F#2 - Tambourine
}

# Remapping rules for non-standard drum notes
# Maps (instrument_keyword, source_note) -> target_note
# Or (instrument_keyword, None) -> default mapping for all notes
DRUM_REMAP_RULES = {
    # Bongos: map to GM bongo range or toms for more punch
    ('bongo', 60): GM_DRUMS['tom_high'],    # Hi bongo -> high tom (more punch)
    ('bongo', 61): GM_DRUMS['tom_mid'],     # Lo bongo -> mid tom
    ('bongo', 69): GM_DRUMS['tom_low'],     # Extra bongo sound -> low tom

    # Cowbell: map to GM cowbell
    ('cowbell', 68): GM_DRUMS['cowbell'],   # Non-standard cowbell -> GM cowbell (56)

    # Generic fallbacks for percussion tracks
    ('percussion', None): None,  # No default remapping
}


def get_drum_remap(track_name: str, note: int) -> int:
    """Get remapped drum note, or return original if no mapping exists."""
    track_lower = track_name.lower()

    # Check specific mappings first
    for (keyword, src_note), target in DRUM_REMAP_RULES.items():
        if keyword in track_lower:
            if src_note == note:
                return target if target else note

    return note


@dataclass
class Track:
    index: int
    name: str
    instrument: str
    instrument_type: str  # bass, drums, lead, other
    note_count: int = 0
    midi_program: Optional[int] = None


@dataclass
class Section:
    bar_start: int
    name: str
    bar_end: Optional[int] = None  # Calculated from next section


@dataclass
class LoopSuggestion:
    """A suggested loop extraction."""
    track_index: int
    track_type: str
    track_name: str
    bar_start: int
    bar_end: int
    score: float
    reasons: list[str] = field(default_factory=list)

    @property
    def length(self) -> int:
        return self.bar_end - self.bar_start + 1


@dataclass
class SongInfo:
    title: str
    artist: str
    tempo: int
    total_bars: int
    tracks: list[Track] = field(default_factory=list)
    sections: list[Section] = field(default_factory=list)
    ticks_per_beat: int = 480


def classify_instrument(name: str, instrument: str) -> str:
    """Classify instrument into bass, drums, lead, or other."""
    combined = f"{name} {instrument}".lower()

    if "bass" in combined:
        return "bass"
    if any(x in combined for x in ["drum", "percussion", "bongo", "cowbell", "conga", "hi-hat", "snare", "kick"]):
        return "drums"
    if any(x in combined for x in ["lead", "solo", "synth", "overdrive", "distortion"]):
        return "lead"
    if any(x in combined for x in ["guitar", "piano", "organ", "keys"]):
        return "other"
    return "other"


def parse_gp8(gp_path: Path) -> SongInfo:
    """Parse a Guitar Pro 8 file and extract metadata."""
    with zipfile.ZipFile(gp_path, 'r') as z:
        xml_data = z.read('Content/score.gpif')

    root = ET.fromstring(xml_data)

    # Basic info
    title_el = root.find('.//Title')
    title = title_el.text.strip() if title_el is not None and title_el.text else "Unknown"

    artist_el = root.find('.//Artist')
    artist = artist_el.text.strip() if artist_el is not None and artist_el.text else "Unknown"

    tempo_el = root.find('.//Automation[Type="Tempo"]/Value')
    tempo = int(tempo_el.text.split()[0]) if tempo_el is not None else 120

    total_bars = len(root.findall('.//MasterBar'))

    # Tracks
    tracks = []
    for track_el in root.findall('.//Track'):
        idx = int(track_el.get('id', 0))
        name_el = track_el.find('Name')
        name = name_el.text.strip() if name_el is not None and name_el.text else ""

        instr_el = track_el.find('.//InstrumentSet/Name')
        instrument = instr_el.text if instr_el is not None else ""

        instr_type = classify_instrument(name, instrument)
        tracks.append(Track(idx, name, instrument, instr_type))

    # Sections
    sections = []
    for i, mb in enumerate(root.findall('.//MasterBar')):
        section_el = mb.find('.//Section/Text')
        if section_el is not None and section_el.text:
            sections.append(Section(bar_start=i + 1, name=section_el.text.strip()))

    # Calculate section end bars
    for i, section in enumerate(sections):
        if i + 1 < len(sections):
            section.bar_end = sections[i + 1].bar_start - 1
        else:
            section.bar_end = total_bars

    return SongInfo(
        title=title,
        artist=artist,
        tempo=tempo,
        total_bars=total_bars,
        tracks=tracks,
        sections=sections
    )


def enrich_with_midi(song_info: SongInfo, midi_path: Path) -> SongInfo:
    """Add MIDI-specific info to the song analysis."""
    mid = mido.MidiFile(midi_path)
    song_info.ticks_per_beat = mid.ticks_per_beat

    for i, track in enumerate(mid.tracks):
        if i < len(song_info.tracks):
            notes = [m for m in track if m.type == 'note_on' and m.velocity > 0]
            song_info.tracks[i].note_count = len(notes)

            for msg in track:
                if msg.type == 'program_change':
                    song_info.tracks[i].midi_program = msg.program
                    break

    return song_info


def extract_loop(
    midi_path: Path,
    track_index: int,
    bar_start: int,
    bar_end: int,
    ticks_per_beat: int,
    output_path: Path,
    output_ticks_per_beat: int = 96,
    track_name: str = "",
    is_drum_track: bool = False
) -> None:
    """Extract a loop from a specific track and bar range."""
    mid = mido.MidiFile(midi_path)
    source_track = mid.tracks[track_index]

    ticks_per_bar = ticks_per_beat * 4  # Assuming 4/4
    start_tick = (bar_start - 1) * ticks_per_bar
    end_tick = bar_end * ticks_per_bar

    # Scale factor for output resolution
    scale = output_ticks_per_beat / ticks_per_beat

    # Create new MIDI file
    out_mid = mido.MidiFile(ticks_per_beat=output_ticks_per_beat)
    out_track = mido.MidiTrack()
    out_mid.tracks.append(out_track)

    # Add track name
    out_track.append(mido.MetaMessage('track_name', name=output_path.stem, time=0))

    # Add time signature
    out_track.append(mido.MetaMessage('time_signature', numerator=4, denominator=4, time=0))

    # Copy program change if present (skip for drums - let DAW handle sound selection)
    if not is_drum_track:
        for msg in source_track:
            if msg.type == 'program_change':
                out_track.append(msg.copy(time=0))
                break

    # Extract notes in range
    abs_time = 0
    last_out_time = 0
    remapped_notes = {}  # Track remappings for logging

    for msg in source_track:
        abs_time += msg.time

        if start_tick <= abs_time < end_tick:
            if msg.type in ('note_on', 'note_off'):
                # Convert to output time scale, relative to loop start
                out_abs_time = int((abs_time - start_tick) * scale)
                delta = out_abs_time - last_out_time

                # Remap drum notes if this is a drum track
                note = msg.note
                if is_drum_track:
                    new_note = get_drum_remap(track_name, note)
                    if new_note != note:
                        remapped_notes[note] = new_note
                        note = new_note

                out_track.append(msg.copy(time=delta, note=note))
                last_out_time = out_abs_time

    # Add end of track
    out_track.append(mido.MetaMessage('end_of_track', time=0))

    out_mid.save(output_path)

    # Log remappings
    remap_info = ""
    if remapped_notes:
        remap_str = ", ".join(f"{k}->{v}" for k, v in remapped_notes.items())
        remap_info = f" (remapped: {remap_str})"
    print(f"Saved loop to {output_path}{remap_info}")


def analyze_patterns(midi_path: Path, track_index: int, ticks_per_beat: int, max_bars: int = 32):
    """Analyze a track for repeating patterns."""
    mid = mido.MidiFile(midi_path)
    track = mid.tracks[track_index]

    ticks_per_bar = ticks_per_beat * 4

    # Build bar signatures
    notes_by_bar = {}
    abs_time = 0

    for msg in track:
        abs_time += msg.time
        if msg.type == 'note_on' and msg.velocity > 0:
            bar = (abs_time // ticks_per_bar) + 1
            if bar <= max_bars:
                beat = ((abs_time % ticks_per_bar) * 4) // ticks_per_bar + 1
                if bar not in notes_by_bar:
                    notes_by_bar[bar] = []
                notes_by_bar[bar].append((msg.note, beat))

    # Find repeated patterns
    bar_signatures = {}
    for bar, notes in notes_by_bar.items():
        sig = tuple(sorted(notes))
        if sig not in bar_signatures:
            bar_signatures[sig] = []
        bar_signatures[sig].append(bar)

    # Report patterns
    patterns = []
    for sig, bars in bar_signatures.items():
        if len(bars) > 1 and len(sig) > 0:
            patterns.append({
                'bars': bars,
                'note_count': len(sig),
                'notes': sig[:5]  # First 5 notes as preview
            })

    return sorted(patterns, key=lambda x: -len(x['bars']))


def get_notes_by_bar(midi_path: Path, track_index: int, ticks_per_beat: int) -> dict[int, list]:
    """Get notes organized by bar number."""
    mid = mido.MidiFile(midi_path)
    track = mid.tracks[track_index]
    ticks_per_bar = ticks_per_beat * 4

    notes_by_bar = {}
    abs_time = 0

    for msg in track:
        abs_time += msg.time
        if msg.type == 'note_on' and msg.velocity > 0:
            bar = (abs_time // ticks_per_bar) + 1
            if bar not in notes_by_bar:
                notes_by_bar[bar] = []
            # Store note with position within bar (0-1 normalized)
            pos_in_bar = (abs_time % ticks_per_bar) / ticks_per_bar
            notes_by_bar[bar].append({
                'note': msg.note,
                'velocity': msg.velocity,
                'position': pos_in_bar
            })

    return notes_by_bar


def find_repeating_sequences(notes_by_bar: dict, min_length: int = 4, max_length: int = 8) -> list[dict]:
    """Find repeating multi-bar sequences."""
    sequences = []
    bars = sorted(notes_by_bar.keys())

    if not bars:
        return sequences

    # Try different sequence lengths (prefer powers of 2)
    for length in [8, 4, 2, 1]:
        if length > max_length or length < min_length:
            continue

        # Build signatures for each possible sequence start
        seq_signatures = {}
        for start_bar in bars:
            end_bar = start_bar + length - 1
            # Check if all bars in sequence exist
            if all(b in notes_by_bar for b in range(start_bar, end_bar + 1)):
                # Create signature from all notes in sequence
                sig_parts = []
                for b in range(start_bar, end_bar + 1):
                    bar_notes = tuple(sorted((n['note'], round(n['position'], 2)) for n in notes_by_bar[b]))
                    sig_parts.append(bar_notes)
                sig = tuple(sig_parts)

                if sig not in seq_signatures:
                    seq_signatures[sig] = []
                seq_signatures[sig].append(start_bar)

        # Find repeated sequences
        for sig, start_bars in seq_signatures.items():
            if len(start_bars) >= 2 and any(len(part) > 0 for part in sig):
                # Count total notes
                total_notes = sum(len(part) for part in sig)
                sequences.append({
                    'length': length,
                    'occurrences': start_bars,
                    'repeat_count': len(start_bars),
                    'note_count': total_notes,
                    'first_bar': start_bars[0]
                })

    return sequences


def score_section_name(name: str) -> float:
    """Score a section name for loop potential."""
    name_lower = name.lower()

    # High value keywords
    if any(kw in name_lower for kw in ['riff', 'groove', 'hook', 'loop', 'main']):
        return 30.0
    # Medium value
    if any(kw in name_lower for kw in ['intro', 'verse', 'chorus', 'break']):
        return 20.0
    # Lower value
    if any(kw in name_lower for kw in ['bridge', 'outro', 'solo']):
        return 10.0
    return 15.0  # Unknown section


def suggest_loops(
    song_info: SongInfo,
    midi_path: Path,
    target_types: list[str] = None
) -> list[LoopSuggestion]:
    """Generate loop suggestions for a song."""
    if target_types is None:
        target_types = ['bass', 'drums', 'lead']

    suggestions = []

    for track in song_info.tracks:
        if track.instrument_type not in target_types:
            continue
        if track.note_count == 0:
            continue

        track_name = track.name or track.instrument
        notes_by_bar = get_notes_by_bar(midi_path, track.index, song_info.ticks_per_beat)

        if not notes_by_bar:
            continue

        # Strategy 1: Section-based suggestions
        for section in song_info.sections:
            section_length = section.bar_end - section.bar_start + 1

            # Trim to standard loop lengths (minimum 4 bars)
            for target_len in [8, 4]:
                if section_length >= target_len:
                    bar_start = section.bar_start
                    bar_end = section.bar_start + target_len - 1

                    # Count notes in this range and check bar coverage
                    bars_with_notes = 0
                    note_count = 0
                    for b in range(bar_start, bar_end + 1):
                        bar_notes = len(notes_by_bar.get(b, []))
                        note_count += bar_notes
                        if bar_notes > 0:
                            bars_with_notes += 1

                    if note_count == 0:
                        continue

                    # Skip if half or fewer bars have content (e.g. 8-bar section with only 4 bars of notes)
                    coverage = bars_with_notes / target_len
                    if coverage <= 0.5:
                        continue

                    # Calculate score
                    score = 0.0
                    reasons = []

                    # Section name bonus
                    name_score = score_section_name(section.name)
                    score += name_score
                    reasons.append(f"Section: {section.name}")

                    # Track type bonus
                    if track.instrument_type == 'bass':
                        score += 25.0
                        reasons.append("Bass track (high priority)")
                    elif track.instrument_type == 'drums':
                        score += 20.0
                        reasons.append("Drum track (high priority)")
                    elif track.instrument_type == 'lead':
                        score += 15.0
                        reasons.append("Lead track")

                    # Loop length bonus (prefer 4 and 8 bar loops)
                    if target_len == 4:
                        score += 15.0
                    elif target_len == 8:
                        score += 10.0
                    reasons.append(f"{target_len}-bar loop")

                    # Note density scoring
                    notes_per_bar = note_count / target_len
                    if 4 <= notes_per_bar <= 20:
                        score += 10.0
                        reasons.append(f"Good density ({notes_per_bar:.1f} notes/bar)")
                    elif notes_per_bar > 0:
                        score += 5.0

                    suggestions.append(LoopSuggestion(
                        track_index=track.index,
                        track_type=track.instrument_type,
                        track_name=track_name,
                        bar_start=bar_start,
                        bar_end=bar_end,
                        score=score,
                        reasons=reasons
                    ))
                    break  # Only suggest one length per section

        # Strategy 2: Pattern-based suggestions, snapped to section boundaries
        sequences = find_repeating_sequences(notes_by_bar)

        # Build list of section start bars for snapping
        section_starts = [s.bar_start for s in song_info.sections] if song_info.sections else [1]

        for seq in sequences:
            raw_bar_start = seq['first_bar']
            loop_length = seq['length']

            # Snap to nearest section-aligned position
            # Find which section this pattern falls into
            containing_section_start = 1
            for ss in section_starts:
                if ss <= raw_bar_start:
                    containing_section_start = ss
                else:
                    break

            # Snap to a loop_length boundary from the section start
            bars_into_section = raw_bar_start - containing_section_start
            snapped_offset = (bars_into_section // loop_length) * loop_length
            bar_start = containing_section_start + snapped_offset
            bar_end = bar_start + loop_length - 1

            # Recount notes for the snapped range
            note_count = sum(len(notes_by_bar.get(b, [])) for b in range(bar_start, bar_end + 1))
            if note_count == 0:
                continue

            score = 0.0
            reasons = []

            # Repetition bonus (main signal!)
            repeat_score = min(seq['repeat_count'] * 8, 40)  # Cap at 40
            score += repeat_score
            reasons.append(f"Pattern repeats {seq['repeat_count']}x")

            # Track type bonus
            if track.instrument_type == 'bass':
                score += 25.0
                reasons.append("Bass track")
            elif track.instrument_type == 'drums':
                score += 20.0
                reasons.append("Drum track")
            elif track.instrument_type == 'lead':
                score += 15.0
                reasons.append("Lead track")

            # Loop length bonus
            if loop_length == 4:
                score += 15.0
            elif loop_length == 8:
                score += 10.0
            reasons.append(f"{loop_length}-bar loop")

            # Note density
            notes_per_bar = note_count / loop_length
            if 4 <= notes_per_bar <= 20:
                score += 10.0
                reasons.append(f"Good density ({notes_per_bar:.1f} notes/bar)")

            suggestions.append(LoopSuggestion(
                track_index=track.index,
                track_type=track.instrument_type,
                track_name=track_name,
                bar_start=bar_start,
                bar_end=bar_end,
                score=score,
                reasons=reasons
            ))

    # Sort by score and deduplicate similar suggestions
    suggestions.sort(key=lambda x: -x.score)

    # Remove duplicates (same track, overlapping bars)
    filtered = []
    seen = set()
    for s in suggestions:
        key = (s.track_index, s.bar_start, s.bar_end)
        if key not in seen:
            # Also check for overlapping ranges on same track
            dominated = False
            for existing in filtered:
                if existing.track_index == s.track_index:
                    # Check overlap
                    if (s.bar_start <= existing.bar_end and s.bar_end >= existing.bar_start):
                        # Overlaps - keep the higher scored one (already in filtered)
                        if existing.score >= s.score:
                            dominated = True
                            break
            if not dominated:
                filtered.append(s)
                seen.add(key)

    # Merge adjacent loops in the same section into longer loops
    # Build section lookup
    def get_section_for_bar(bar: int) -> Optional[str]:
        for section in song_info.sections:
            if section.bar_start <= bar <= section.bar_end:
                return section.name
        return None

    merged = []
    filtered.sort(key=lambda x: (x.track_index, x.bar_start))

    i = 0
    while i < len(filtered):
        current = filtered[i]
        current_section = get_section_for_bar(current.bar_start)

        # Look for adjacent loops to merge
        j = i + 1
        merged_end = current.bar_end
        merged_reasons = list(current.reasons)
        loops_merged = 1

        while j < len(filtered):
            next_loop = filtered[j]
            # Check if adjacent (next starts right after current ends) and same track
            if (next_loop.track_index == current.track_index and
                next_loop.bar_start == merged_end + 1 and
                get_section_for_bar(next_loop.bar_start) == current_section):
                # Merge
                merged_end = next_loop.bar_end
                loops_merged += 1
                j += 1
            else:
                break

        if loops_merged > 1:
            # Create merged loop
            merged_length = merged_end - current.bar_start + 1
            merged.append(LoopSuggestion(
                track_index=current.track_index,
                track_type=current.track_type,
                track_name=current.track_name,
                bar_start=current.bar_start,
                bar_end=merged_end,
                score=current.score + 5,  # Small bonus for merged loops
                reasons=[f"Merged {loops_merged}x adjacent patterns", f"{merged_length}-bar loop"] + merged_reasons[2:]
            ))
            i = j
        else:
            merged.append(current)
            i += 1

    # Re-sort by score after merging
    merged.sort(key=lambda x: -x.score)

    return merged[:15]  # Return top 15 suggestions


def print_analysis(song_info: SongInfo):
    """Print a summary of the song analysis."""
    print(f"\n{'='*60}")
    print(f"{song_info.title} - {song_info.artist}")
    print(f"{'='*60}")
    print(f"Tempo: {song_info.tempo} BPM | Bars: {song_info.total_bars}")
    print(f"Ticks/beat: {song_info.ticks_per_beat}")

    print(f"\nTracks:")
    for t in song_info.tracks:
        type_icon = {"bass": "🎸", "drums": "🥁", "lead": "🎹", "other": "🎵"}[t.instrument_type]
        print(f"  [{t.index}] {type_icon} {t.name or '(unnamed)':<25} {t.instrument:<30} ({t.note_count} notes)")

    if song_info.sections:
        print(f"\nSections:")
        for s in song_info.sections:
            print(f"  Bar {s.bar_start:3}-{s.bar_end:3}: {s.name}")
    else:
        print("\nNo sections defined in GP file")


def main():
    import sys

    if len(sys.argv) < 2:
        print(__doc__)
        return

    command = sys.argv[1]

    if command == "analyze":
        if len(sys.argv) < 4:
            print("Usage: looper.py analyze <gp_file> <midi_file>")
            return

        gp_path = Path(sys.argv[2])
        midi_path = Path(sys.argv[3])

        song_info = parse_gp8(gp_path)
        song_info = enrich_with_midi(song_info, midi_path)
        print_analysis(song_info)

        # Show pattern analysis for bass/drum tracks
        print(f"\nPattern Analysis:")
        for t in song_info.tracks:
            if t.instrument_type in ("bass", "drums") and t.note_count > 0:
                print(f"\n  {t.name or t.instrument} (track {t.index}):")
                patterns = analyze_patterns(midi_path, t.index, song_info.ticks_per_beat)
                for p in patterns[:3]:
                    print(f"    Bars {p['bars'][:8]}{'...' if len(p['bars']) > 8 else ''}: {p['note_count']} notes")

    elif command == "extract":
        if len(sys.argv) < 6:
            print("Usage: looper.py extract <gp_file> <midi_file> --track <index|type> --bars <start>-<end> [--output <path>]")
            return

        gp_path = Path(sys.argv[2])
        midi_path = Path(sys.argv[3])

        # Parse args
        track_arg = None
        bars_arg = None
        output_arg = None

        i = 4
        while i < len(sys.argv):
            if sys.argv[i] == "--track":
                track_arg = sys.argv[i + 1]
                i += 2
            elif sys.argv[i] == "--bars":
                bars_arg = sys.argv[i + 1]
                i += 2
            elif sys.argv[i] == "--output":
                output_arg = sys.argv[i + 1]
                i += 2
            else:
                i += 1

        if not track_arg or not bars_arg:
            print("--track and --bars are required")
            return

        song_info = parse_gp8(gp_path)
        song_info = enrich_with_midi(song_info, midi_path)

        # Resolve track
        if track_arg.isdigit():
            track_index = int(track_arg)
        else:
            # Find by type
            matches = [t for t in song_info.tracks if t.instrument_type == track_arg]
            if not matches:
                print(f"No tracks found with type '{track_arg}'")
                return
            track_index = matches[0].index
            print(f"Using track {track_index}: {matches[0].instrument}")

        # Parse bars
        bar_start, bar_end = map(int, bars_arg.split('-'))

        # Output path
        if output_arg:
            output_path = Path(output_arg)
        else:
            track = song_info.tracks[track_index]
            safe_title = "".join(c for c in song_info.title if c.isalnum() or c in " -_")
            output_path = OUT_DIR / f"{safe_title} - {track.instrument_type} - bars {bar_start}-{bar_end}.mid"

        output_path.parent.mkdir(parents=True, exist_ok=True)

        extract_loop(
            midi_path,
            track_index,
            bar_start,
            bar_end,
            song_info.ticks_per_beat,
            output_path
        )

    elif command == "suggest":
        if len(sys.argv) < 4:
            print("Usage: looper.py suggest <gp_file> <midi_file> [--extract] [--top N]")
            return

        gp_path = Path(sys.argv[2])
        midi_path = Path(sys.argv[3])

        # Parse args
        do_extract = "--extract" in sys.argv
        top_n = 10
        for i, arg in enumerate(sys.argv):
            if arg == "--top" and i + 1 < len(sys.argv):
                top_n = int(sys.argv[i + 1])

        song_info = parse_gp8(gp_path)
        song_info = enrich_with_midi(song_info, midi_path)

        print(f"\n{'='*60}")
        print(f"Loop Suggestions: {song_info.title}")
        print(f"{'='*60}")
        print(f"Tempo: {song_info.tempo} BPM\n")

        suggestions = suggest_loops(song_info, midi_path)[:top_n]

        if not suggestions:
            print("No loop suggestions found.")
            return

        type_icons = {"bass": "🎸", "drums": "🥁", "lead": "🎹", "other": "🎵"}

        for i, s in enumerate(suggestions, 1):
            icon = type_icons.get(s.track_type, "🎵")
            print(f"{i:2}. {icon} [{s.track_type.upper()}] Bars {s.bar_start}-{s.bar_end} ({s.length} bars)")
            print(f"    Track: {s.track_name}")
            print(f"    Score: {s.score:.0f} - {', '.join(s.reasons)}")
            print()

        if do_extract:
            print(f"\n{'='*60}")
            print("Extracting suggested loops...")
            print(f"{'='*60}\n")

            OUT_DIR.mkdir(exist_ok=True)
            safe_title = "".join(c for c in song_info.title if c.isalnum() or c in " -_")

            for i, s in enumerate(suggestions, 1):
                safe_track = "".join(c for c in s.track_name if c.isalnum() or c in " -_")[:30]
                output_name = f"{safe_title} - {s.track_type} - {safe_track} - bars {s.bar_start}-{s.bar_end}.mid"
                output_path = OUT_DIR / output_name

                extract_loop(
                    midi_path,
                    s.track_index,
                    s.bar_start,
                    s.bar_end,
                    song_info.ticks_per_beat,
                    output_path,
                    track_name=s.track_name,
                    is_drum_track=(s.track_type == "drums")
                )

            print(f"\nExtracted {len(suggestions)} loops to {OUT_DIR}/")

    else:
        print(f"Unknown command: {command}")
        print(__doc__)


if __name__ == "__main__":
    main()
