use std::mem;

use vst3::Steinberg::Vst::NoteExpressionTypeIDs_::{kBrightnessTypeID, kExpressionTypeID, kPanTypeID, kTuningTypeID, kVibratoTypeID, kVolumeTypeID};
use vst3::{ComRef, Steinberg::{kResultOk, Vst::{self, IEventList, IEventListTrait}}};

use crate::formats::midi::{note_channel, note_id, note_key};
use crate::{Event, NoteExpressions};

use super::note_expression::NoteExpressionDescriptor;

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

            let event = match event.r#type as _ {
                Vst::Event_::EventTypes_::kNoteOnEvent => unsafe {
                    let note_on = event.__field0.noteOn;

                    // VST3 always supplies a valid channel and key on a note-on, but a wildcard
                    // or out of range value is invalid and should get skipped.
                    let (Some(channel), Some(key)) = (note_channel(note_on.channel), note_key(note_on.pitch)) else {
                        continue;
                    };

                    Some(Event::NoteOn {
                        sample_offset: event.sampleOffset as _,
                        channel,
                        key,
                        note_id: note_id(note_on.noteId),
                        velocity: note_on.velocity as _,
                    })
                },

                Vst::Event_::EventTypes_::kNoteOffEvent => unsafe {
                    let note_off = event.__field0.noteOff;

                    Some(Event::NoteOff {
                        sample_offset: event.sampleOffset as _,
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
                        sample_offset: event.sampleOffset as _,
                        channel: note_channel(poly_pressure.channel),
                        key: note_key(poly_pressure.pitch),
                        note_id: note_id(poly_pressure.noteId),
                        value: poly_pressure.pressure as _,
                    })
                },

                Vst::Event_::EventTypes_::kNoteExpressionValueEvent => unsafe {
                    let sample_offset = event.sampleOffset as usize;
                    let note_expression = event.__field0.noteExpressionValue;
                    let value = note_expression.value;

                    // Key and channel are not provided for VST3, just the note_id
                    let channel: Option<u8> = None;
                    let key: Option<u8> = None;

                    // An expression with a missing note id addresses nothing at all, so skip it.
                    let note_id = note_id(note_expression.noteId);
                    if note_id.is_none() {
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
                                // NB: CLAP's PolyVolume is 0..4, where 1 is 0db, so we only use the 0..1 volume range here
                                gain: value,
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
