use crate::{Event, MidiCapabilities};

/// Parse a raw MIDI message into an `Event`, filtered by the given MIDI `capabilities`.
/// `sample_offset` is the sample offset within the audio buffer.
///
/// Returns `None` for:
/// - Messages shorter than the minimum expected length.
/// - Messages whose capability is not enabled in `capabilities`.
/// - Unsupported Message types such as MIDI SysEx.
pub(crate) fn parse_midi_event(
    data: &[u8],
    sample_offset: usize,
    capabilities: MidiCapabilities,
) -> Option<Event> {
    if data.len() < 2 {
        return None;
    }

    let status = data[0] & 0xF0;
    let channel = (data[0] & 0x0F) as i16;

    match status {
        // Note-on: velocity 0 is treated as note-off per MIDI spec.
        0x90 if data.len() >= 3 && data[2] > 0 => Some(Event::NoteOn {
            sample_offset: 0,
            channel,
            key: data[1] as i16,
            note: -1,
            velocity: data[2] as f64 / 127.0,
        }),

        // Note-off (explicit 0x80 or 0x90 with vel=0).
        0x80 | 0x90 => {
            let velocity = if data.len() >= 3 {
                data[2] as f64 / 127.0
            } else {
                0.0
            };
            Some(Event::NoteOff {
                sample_offset: 0,
                channel,
                key: data[1] as i16,
                note: -1,
                velocity,
            })
        }
        
        // Polyphonic Key Pressure / poly aftertouch (0xA0)
        0xA0 if data.len() >= 3 && capabilities.midi_poly_pressure() => {
            Some(Event::MidiPolyPressure {
                sample_offset,
                channel,
                key: data[1] as i16,
                note: -1,
                value: data[2] as f64 / 127.0,
            })
        }

        // Program Change (0xC0)
        0xC0 if data.len() >= 2 && capabilities.midi_program_change() => {
            Some(Event::MidiProgramChange {
                sample_offset,
                channel,
                program: data[1],
            })
        }

        // Control Change (0xB0)
        0xB0 if data.len() >= 3 => {
            let controller = data[1];
            if capabilities.has_midi_control_change(controller) {
                Some(Event::MidiControlChange {
                    sample_offset,
                    channel,
                    controller,
                    value: data[2] as f64 / 127.0,
                })
            } else {
                None
            }
        }

        // Channel Pressure / mono aftertouch (0xD0)
        0xD0 if data.len() >= 2 && capabilities.midi_channel_pressure() => {
            Some(Event::MidiChannelPressure {
                sample_offset,
                channel,
                value: data[1] as f64 / 127.0,
            })
        }

        // Pitch Bend (0xE0)
        0xE0 if data.len() >= 3 && capabilities.midi_pitch_bend() => {
            let lsb = data[1] as u16;
            let msb = data[2] as u16;
            let raw = (msb << 7) | lsb;
            let semitones = (raw as f64 - 8192.0) / 8192.0 * 2.0;
            Some(Event::MidiPitchBend {
                sample_offset,
                channel,
                semitones,
            })
        }

        _ => None,
    }
}
