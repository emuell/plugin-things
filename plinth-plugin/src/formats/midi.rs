use crate::midi_capabilities::MIDI_CHANNEL_COUNT;
use crate::{Event, MidiCapabilities};

/// Convert and validate a raw CLAP/VST3 MIDI channel field.
///
/// `None` is the wildcard: a negative value is the `-1` sentinel both APIs define, and a
/// value outside `0..=15` is not an address we can represent, so degrade it to a wildcard as well.
pub(crate) fn note_channel(raw: i16) -> Option<u8> {
    u8::try_from(raw).ok().filter(|&channel| channel < MIDI_CHANNEL_COUNT as u8)
}

/// Convert and validate a raw CLAP/VST3 key field. See [`note_channel`] for the wildcard rule.
pub(crate) fn note_key(raw: i16) -> Option<u8> {
    u8::try_from(raw).ok().filter(|&key| key < 128)
}

/// Convert and validate a raw CLAP/VST3 note id. Negative means the host does not issue note ids.
pub(crate) fn note_id(raw: i32) -> Option<u32> {
    u32::try_from(raw).ok()
}

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
    let channel = data[0] & 0x0F;

    match status {
        // Note-on: velocity 0 is treated as note-off per MIDI spec.
        0x90 if data.len() >= 3 && data[2] > 0 => Some(Event::NoteOn {
            sample_offset,
            channel,
            key: data[1] & 0x7f,
            note_id: None,
            velocity: (data[2] & 0x7f) as f64 / 127.0,
        }),

        // Note-off (explicit 0x80 or 0x90 with vel=0).
        0x80 | 0x90 => {
            let velocity = if data.len() >= 3 {
                (data[2] & 0x7f) as f64 / 127.0
            } else {
                0.0
            };
            Some(Event::NoteOff {
                sample_offset,
                channel: Some(channel),
                key: Some(data[1] & 0x7f),
                note_id: None,
                velocity,
            })
        }

        // Polyphonic Key Pressure / poly aftertouch (0xA0)
        0xA0 if data.len() >= 3 && capabilities.midi_poly_pressure() => {
            Some(Event::MidiPolyPressure {
                sample_offset,
                channel,
                key: data[1] & 0x7f,
                value: (data[2] & 0x7f) as f64 / 127.0,
            })
        }

        // Program Change (0xC0)
        0xC0 if data.len() >= 2 && capabilities.midi_program_change() => {
            Some(Event::MidiProgramChange {
                sample_offset,
                channel,
                program: data[1] & 0x7f,
            })
        }

        // Control Change (0xB0)
        0xB0 if data.len() >= 3 => {
            // NB: No mask needed, has_midi_control_change rejects controllers above 127.
            let controller = data[1];
            if capabilities.has_midi_control_change(controller) {
                Some(Event::MidiControlChange {
                    sample_offset,
                    channel,
                    controller,
                    value: (data[2] & 0x7f) as f64 / 127.0,
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
                value: (data[1] & 0x7f) as f64 / 127.0,
            })
        }

        // Pitch Bend (0xE0)
        0xE0 if data.len() >= 3 && capabilities.midi_pitch_bend() => {
            let lsb = (data[1] & 0x7f) as u16;
            let msb = (data[2] & 0x7f) as u16;
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

/// Convert a note or `Midi*` event into a raw 3-byte MIDI message, for sending events to the host.
/// Returns the event's sample offset and MIDI bytes. Unused trailing bytes are zero.
///
/// Returns `None` for events which have no MIDI representation, such as note-offs with
/// wildcard channels or keys, note expressions, or parameter events.
pub(crate) fn midi_event_to_bytes(event: &Event) -> Option<(usize, [u8; 3])> {
    fn to_7bit(value: f64) -> u8 {
        (value * 127.0).round().clamp(0.0, 127.0) as u8
    }

    match *event {
        Event::NoteOn { sample_offset, channel, key, velocity, .. } => {
            // Velocity 0 would turn the note-on into a note-off
            let velocity = to_7bit(velocity).max(1);
            Some((sample_offset, [0x90 | (channel & 0x0F), key & 0x7F, velocity]))
        }

        Event::NoteOff { sample_offset, channel: Some(channel), key: Some(key), velocity, .. } => {
            Some((sample_offset, [0x80 | (channel & 0x0F), key & 0x7F, to_7bit(velocity)]))
        }

        Event::MidiPolyPressure { sample_offset, channel, key, value } => {
            Some((sample_offset, [0xA0 | (channel & 0x0F), key & 0x7F, to_7bit(value)]))
        }

        Event::MidiControlChange { sample_offset, channel, controller, value } => {
            Some((sample_offset, [0xB0 | (channel & 0x0F), controller & 0x7F, to_7bit(value)]))
        }

        Event::MidiProgramChange { sample_offset, channel, program } => {
            Some((sample_offset, [0xC0 | (channel & 0x0F), program & 0x7F, 0]))
        }

        Event::MidiChannelPressure { sample_offset, channel, value } => {
            Some((sample_offset, [0xD0 | (channel & 0x0F), to_7bit(value), 0]))
        }

        Event::MidiPitchBend { sample_offset, channel, semitones } => {
            // Assuming the standard +-2 semitone range
            let bend = (semitones / 2.0 * 8192.0 + 8192.0).round().clamp(0.0, 16383.0) as u16;
            Some((sample_offset, [0xE0 | (channel & 0x0F), (bend & 0x7F) as u8, (bend >> 7) as u8]))
        }

        _ => None,
    }
}
