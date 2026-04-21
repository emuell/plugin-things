use std::mem;

use vst3::Steinberg::Vst::ControllerNumbers_::{kAfterTouch, kCtrlPolyPressure, kCtrlProgramChange, kPitchBend};
use vst3::Steinberg::Vst::NoteExpressionTypeIDs_::{kBrightnessTypeID, kExpressionTypeID, kPanTypeID, kTuningTypeID, kVibratoTypeID, kVolumeTypeID};
use vst3::{ComRef, Steinberg::{kResultOk, Vst::{self, Event as Vst3Event, Event_::EventTypes_, Event__type0, IEventList, IEventListTrait, LegacyMIDICCOutEvent, NoteExpressionTypeID, NoteExpressionValueEvent, NoteOffEvent, NoteOnEvent, PolyPressureEvent}}};

use crate::formats::midi::{midi_event_to_bytes, note_channel, note_id, note_key};
use crate::{Event, NoteExpressions};

use super::note_expressions::NoteExpressionDescriptor;

/// Convert a note, note expression or MIDI event to a VST3 output event. `max_sample_offset` is the
/// last valid sample offset in the current block: events past it get clamped to it.
///
/// Returns `None` for events which can't be represented in VST3, such as note expressions without
/// a note id, or MIDI messages which are not supported by `LegacyMIDICCOutEvent`.
pub fn event_to_vst3_event(event: &Event, max_sample_offset: usize) -> Option<Vst3Event> {
    let vst3_event = |sample_offset: usize, r#type: Vst::Event_::EventTypes, field: Event__type0| {
        if sample_offset > max_sample_offset {
            tracing::debug!("Clamping output event sample offset {sample_offset} to {max_sample_offset}");
        }
        Vst3Event {
            busIndex: 0,
            sampleOffset: sample_offset.min(max_sample_offset) as _,
            ppqPosition: 0.0,
            // Plugin generated events are no live user events
            flags: 0,
            r#type: r#type as _,
            __field0: field,
        }
    };

    let note_expression = |sample_offset: usize, note_id: u32, type_id: NoteExpressionTypeID, value: f64| {
        Some(vst3_event(sample_offset, EventTypes_::kNoteExpressionValueEvent, Event__type0 { noteExpressionValue: NoteExpressionValueEvent {
            typeId: type_id,
            noteId: note_id as _,
            value,
        }}))
    };

    match *event {
        Event::NoteOn { sample_offset, channel, key, note_id, velocity } => {
            Some(vst3_event(sample_offset, EventTypes_::kNoteOnEvent, Event__type0 { noteOn: NoteOnEvent {
                channel: channel as _,
                pitch: key as _,
                tuning: 0.0,
                velocity: velocity as _,
                length: 0,
                noteId: note_id.map_or(-1, |id| id as _),
            }}))
        }

        Event::NoteOff { sample_offset, channel, key, note_id, velocity } => {
            Some(vst3_event(sample_offset, EventTypes_::kNoteOffEvent, Event__type0 { noteOff: NoteOffEvent {
                channel: channel.map_or(-1, |channel| channel as _),
                pitch: key.map_or(-1, |key| key as _),
                velocity: velocity as _,
                noteId: note_id.map_or(-1, |id| id as _),
                tuning: 0.0,
            }}))
        }

        // VST3 has no pressure note expression, but a dedicated poly pressure event
        Event::PolyPressure { sample_offset, channel, key, note_id, value } => {
            Some(vst3_event(sample_offset, EventTypes_::kPolyPressureEvent, Event__type0 { polyPressure: PolyPressureEvent {
                channel: channel.map_or(-1, |channel| channel as _),
                pitch: key.map_or(-1, |key| key as _),
                pressure: value as _,
                noteId: note_id.map_or(-1, |id| id as _),
            }}))
        }

        // VST3 note expressions are addressed by note id only
        Event::PolyVolume { sample_offset, note_id: Some(note_id), gain, .. } => {
            note_expression(sample_offset, note_id, kVolumeTypeID, NoteExpressionDescriptor::gain_to_normalized(gain))
        }
        Event::PolyPan { sample_offset, note_id: Some(note_id), pan, .. } => {
            note_expression(sample_offset, note_id, kPanTypeID, NoteExpressionDescriptor::pan_to_normalized(pan))
        }
        Event::PolyTuning { sample_offset, note_id: Some(note_id), semitones, .. } => {
            note_expression(sample_offset, note_id, kTuningTypeID, NoteExpressionDescriptor::semitones_to_normalized(semitones))
        }
        Event::PolyVibrato { sample_offset, note_id: Some(note_id), amount, .. } => {
            note_expression(sample_offset, note_id, kVibratoTypeID, amount.clamp(0.0, 1.0))
        }
        Event::PolyExpression { sample_offset, note_id: Some(note_id), amount, .. } => {
            note_expression(sample_offset, note_id, kExpressionTypeID, amount.clamp(0.0, 1.0))
        }
        Event::PolyBrightness { sample_offset, note_id: Some(note_id), amount, .. } => {
            note_expression(sample_offset, note_id, kBrightnessTypeID, amount.clamp(0.0, 1.0))
        }

        _ => {
            let (sample_offset, data) = midi_event_to_bytes(event)?;
            let channel = (data[0] & 0x0F) as i8;

            // VST3 only supports LegacyMIDICCOutEvent for MIDI output - map what we can.
            let (control_number, value, value2) = match data[0] & 0xF0 {
                0xA0 => (kCtrlPolyPressure as u8, data[1] as i8, data[2] as i8), // Poly Pressure (key, pressure)
                0xB0 => (data[1], data[2] as i8, 0),                             // Control Change
                0xC0 => (kCtrlProgramChange as u8, data[1] as i8, 0),            // Program Change
                0xD0 => (kAfterTouch as u8, data[1] as i8, 0),                   // Channel Pressure
                0xE0 => (kPitchBend as u8, data[1] as i8, data[2] as i8),        // Pitch Bend (LSB, MSB)
                _ => return None,
            };

            Some(vst3_event(sample_offset, EventTypes_::kLegacyMIDICCOutEvent, Event__type0 { midiCCOut: LegacyMIDICCOutEvent {
                controlNumber: control_number,
                channel,
                value,
                value2,
            }}))
        }
    }
}

pub struct EventIterator<'a> {
    event_list: Option<ComRef<'a, IEventList>>,
    index: usize,
    note_expressions: NoteExpressions,
}

impl<'a> EventIterator<'a> {
    pub fn new(event_list: *mut IEventList, note_expressions: NoteExpressions) -> Self {
        Self {
            event_list: unsafe { ComRef::from_raw(event_list) },
            index: 0,
            note_expressions,
        }
    }
}

impl Iterator for EventIterator<'_> {
    type Item = Event;

    fn next(&mut self) -> Option<Self::Item> {
        let event_list = self.event_list?;

        loop {
            if self.index >= unsafe { event_list.getEventCount() } as usize {
                return None;
            }

            let mut event: vst3::Steinberg::Vst::Event = unsafe { mem::zeroed() };
            let result = unsafe { event_list.getEvent(self.index as _, &mut event) };
            if result != kResultOk {
                return None;
            }

            self.index += 1;

            // Avoid panics when the host passes a negative sample offset and default to 0 instead.
            let sample_offset = usize::try_from(event.sampleOffset).unwrap_or(0);

            let event = match event.r#type as _ {
                Vst::Event_::EventTypes_::kNoteOnEvent => unsafe {
                    let note_on = event.__field0.noteOn;

                    // VST3 always supplies a valid channel and key on a note-on, but a wildcard
                    // or out of range value is invalid and should get skipped.
                    let (Some(channel), Some(key)) = (note_channel(note_on.channel), note_key(note_on.pitch)) else {
                        tracing::debug!("Ignoring note-on with invalid channel {} or key {}", note_on.channel, note_on.pitch);
                        continue;
                    };

                    Some(Event::NoteOn {
                        sample_offset,
                        channel,
                        key,
                        note_id: note_id(note_on.noteId),
                        velocity: note_on.velocity as _,
                    })
                },

                Vst::Event_::EventTypes_::kNoteOffEvent => unsafe {
                    let note_off = event.__field0.noteOff;

                    Some(Event::NoteOff {
                        sample_offset,
                        channel: note_channel(note_off.channel),
                        key: note_key(note_off.pitch),
                        note_id: note_id(note_off.noteId),
                        velocity: note_off.velocity as _,
                    })
                },

                Vst::Event_::EventTypes_::kPolyPressureEvent if self.note_expressions.pressure() =>
                unsafe {
                    let poly_pressure = event.__field0.polyPressure;

                    Some(Event::PolyPressure {
                        sample_offset,
                        channel: note_channel(poly_pressure.channel),
                        key: note_key(poly_pressure.pitch),
                        note_id: note_id(poly_pressure.noteId),
                        value: poly_pressure.pressure as _,
                    })
                },

                Vst::Event_::EventTypes_::kNoteExpressionValueEvent => unsafe {
                    let note_expression = event.__field0.noteExpressionValue;
                    let value = note_expression.value;

                    // Key and channel are not provided for VST3, just the note_id
                    let channel: Option<u8> = None;
                    let key: Option<u8> = None;

                    // An expression with a missing note id addresses nothing at all, so skip it.
                    let note_id = note_id(note_expression.noteId);
                    if note_id.is_none() {
                        tracing::debug!("Ignoring note expression with invalid note id {}", note_expression.noteId);
                        continue;
                    }

                    // NB: All VST3 note-expression values arrive normalized to [0, 1].
                    #[allow(non_upper_case_globals)]
                    match note_expression.typeId {
                        kVolumeTypeID if self.note_expressions.volume() => {
                            Some(Event::PolyVolume {
                                sample_offset,
                                channel,
                                key,
                                note_id,
                                gain: NoteExpressionDescriptor::normalized_to_gain(value),
                            })
                        }
                        kPanTypeID if self.note_expressions.pan() => Some(Event::PolyPan {
                            sample_offset,
                            channel,
                            note_id,
                            key,
                            pan: NoteExpressionDescriptor::normalized_to_pan(value),
                        }),
                        kTuningTypeID if self.note_expressions.tuning() => {
                            Some(Event::PolyTuning {
                                sample_offset,
                                channel,
                                note_id,
                                key,
                                semitones: NoteExpressionDescriptor::normalized_to_semitones(value),
                            })
                        }
                        kVibratoTypeID if self.note_expressions.vibrato() => {
                            Some(Event::PolyVibrato {
                                sample_offset,
                                channel,
                                note_id,
                                key,
                                amount: value,
                            })
                        }
                        kExpressionTypeID if self.note_expressions.expression() => {
                            Some(Event::PolyExpression {
                                sample_offset,
                                channel,
                                note_id,
                                key,
                                amount: value,
                            })
                        }
                        kBrightnessTypeID if self.note_expressions.brightness() => {
                            Some(Event::PolyBrightness {
                                sample_offset,
                                channel,
                                note_id,
                                key,
                                amount: value,
                            })
                        }
                        // Unknown, unsupported, or gated-off type ID. Skip, but do not stop iteration.
                        _ => {
                            None
                        }
                    }
                },

                // Unhandled event type (or bypassed via capabilities). Skip to next event.
                _ => None,
            };

            if event.is_some() {
                return event;
            } else {
                continue;
            }
        }
    }
}
