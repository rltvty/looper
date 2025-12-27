//! Screenshot trigger utility.
//!
//! Sends MIDI CC 119 value 127 to trigger a screenshot in the looper app.
//! Usage: cargo run --bin screenshot

use midir::MidiOutput;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let midi_out = MidiOutput::new("screenshot-trigger")?;
    let ports = midi_out.ports();

    if ports.is_empty() {
        eprintln!("No MIDI output ports found");
        return Ok(());
    }

    // Look for IAC Driver or use first port
    let port_idx = ports
        .iter()
        .position(|p| {
            midi_out
                .port_name(p)
                .map(|n| n.contains("IAC"))
                .unwrap_or(false)
        })
        .unwrap_or(0);

    let port = &ports[port_idx];
    let port_name = midi_out.port_name(port)?;

    let mut conn = midi_out.connect(port, "screenshot-trigger")?;

    // Send CC 119 value 127 on channel 1 (status byte 0xB0)
    // Format: [status, cc_number, value]
    let cc_message = [0xB0, 119, 127];
    conn.send(&cc_message)?;

    println!("Screenshot trigger sent to: {}", port_name);
    Ok(())
}
