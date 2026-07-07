use std::mem;

use vst3::Steinberg::Vst::NoteExpressionTypeIDs_::{kBrightnessTypeID, kExpressionTypeID, kPanTypeID, kTuningTypeID, kVibratoTypeID, kVolumeTypeID};
use vst3::{ComRef, Steinberg::{kResultOk, Vst::{self, IEventList, IEventListTrait}}};

use crate::{Event, NoteExpressions};

#[derive(Clone, Copy)]
struct NoteIdEntry {
    note_id: i32,
    channel: i16,
    key: i16,
}

/// Fixed-capacity mapping of VST3 `note_id -> (channel, key)`.
pub(super) struct NoteIdMap {
    entries: [NoteIdEntry; Self::CAPACITY],
    /// Index of the next slot to write into in the ring buffer.
    write_index: usize,
    /// Number of valid entries, capped at CAPACITY.
    count: usize,
}

impl NoteIdMap {
    const CAPACITY: usize = 128; // max note count

    pub fn new() -> Self {
        const EMPTY_ENTRY: NoteIdEntry = NoteIdEntry {
            note_id: 0,
            channel: 0,
            key: 0,
        };
        Self {
            entries: [EMPTY_ENTRY; Self::CAPACITY],
            write_index: 0,
            count: 0,
        }
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    fn insert(&mut self, note_id: i32, channel: i16, key: i16) {
        self.entries[self.write_index] = NoteIdEntry {
            note_id,
            channel,
            key,
        };
        self.write_index = (self.write_index + 1) % Self::CAPACITY;
        if self.count < Self::CAPACITY {
            self.count += 1;
        }
    }

    fn lookup(&self, note_id: i32) -> Option<(i16, i16)> {
        for i in 0..self.count {
            let index = (self.write_index + Self::CAPACITY - 1 - i) % Self::CAPACITY;
            let entry = &self.entries[index];
            if entry.note_id == note_id {
                return Some((entry.channel, entry.key));
            }
        }
        None
    }
}

impl Default for NoteIdMap {
    fn default() -> Self {
        Self::new()
    }
}

pub struct EventIterator<'a> {
    event_list: Option<ComRef<'a, IEventList>>,
    index: usize,
    note_id_map: &'a mut NoteIdMap,
    note_expressions: NoteExpressions,
}

impl<'a> EventIterator<'a> {
    pub fn new(event_list: *mut IEventList, note_id_map: &'a mut NoteIdMap, note_expressions: NoteExpressions) -> Self {
        Self {
            event_list: unsafe { ComRef::from_raw(event_list) },
            index: 0,
            note_id_map,
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
                    // Track note ID -> (channel, key) for later note-expression lookups
                    self.note_id_map.insert(
                        event.__field0.noteOn.noteId,
                        event.__field0.noteOn.channel,
                        event.__field0.noteOn.pitch,
                    );
                    Some(Event::NoteOn {
                        sample_offset: event.sampleOffset as _,
                        channel: event.__field0.noteOn.channel,
                        key: event.__field0.noteOn.pitch,
                        note: event.__field0.noteOn.noteId,
                        velocity: event.__field0.noteOn.velocity as _,
                    })
                },

                Vst::Event_::EventTypes_::kNoteOffEvent => unsafe {
                    Some(Event::NoteOff {
                        sample_offset: event.sampleOffset as _,
                        channel: event.__field0.noteOff.channel,
                        key: event.__field0.noteOff.pitch,
                        note: event.__field0.noteOff.noteId,
                        velocity: event.__field0.noteOff.velocity as _,
                    })
                },

                Vst::Event_::EventTypes_::kPolyPressureEvent if self.note_expressions.pressure() =>
                unsafe {
                    Some(Event::PolyPressure {
                        sample_offset: event.sampleOffset as _,
                        channel: event.__field0.polyPressure.channel,
                        key: event.__field0.polyPressure.pitch,
                        note: event.__field0.polyPressure.noteId,
                        value: event.__field0.polyPressure.pressure as _,
                    })
                },

                Vst::Event_::EventTypes_::kNoteExpressionValueEvent => unsafe {
                    let sample_offset = event.sampleOffset as usize;
                    let note_expression = event.__field0.noteExpressionValue;
                    let note = note_expression.noteId;
                    let value  = note_expression.value;

                    // Resolve the note's channel and key from the note map.
                    let (channel, key) = self.note_id_map.lookup(note).unwrap_or((-1, -1));

                    // NB: All VST3 note-expression values arrive normalized to [0, 1].
                    #[allow(non_upper_case_globals)]
                    match note_expression.typeId {
                        kVolumeTypeID if self.note_expressions.volume() => {
                            Some(Event::PolyVolume {
                                sample_offset,
                                channel,
                                key,
                                note,
                                // NB: CLAP's PolyVolume is 0..4, where 1 is 0db, so we only use the 0..1 volume range here
                                gain: value,
                            })
                        }
                        kPanTypeID if self.note_expressions.pan() => Some(Event::PolyPan {
                            sample_offset,
                            channel,
                            key,
                            note,
                            // Registered min=0, max=1, center=0.5 -> map to [-1, 1]
                            pan: value * 2.0 - 1.0,
                        }),
                        kTuningTypeID if self.note_expressions.tuning() => {
                            Some(Event::PolyTuning {
                                sample_offset,
                                channel,
                                key,
                                note,
                                // Registered min=0, max=1, center=0.5 -> map to [-120, +120] semitones
                                semitones: value * 240.0 - 120.0,
                            })
                        }
                        kVibratoTypeID if self.note_expressions.vibrato() => {
                            Some(Event::PolyVibrato {
                                sample_offset,
                                channel,
                                key,
                                note,
                                amount: value,
                            })
                        }
                        kExpressionTypeID if self.note_expressions.expression() => {
                            Some(Event::PolyExpression {
                                sample_offset,
                                channel,
                                key,
                                note,
                                amount: value,
                            })
                        }
                        kBrightnessTypeID if self.note_expressions.brightness() => {
                            Some(Event::PolyBrightness {
                                sample_offset,
                                channel,
                                key,
                                note,
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
