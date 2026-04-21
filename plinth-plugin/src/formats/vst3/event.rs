use std::mem;

use vst3::{ComRef, Steinberg::{kResultOk, Vst::{self, Event as Vst3Event, Event_::{EventFlags_, EventTypes_}, Event__type0, IEventList, IEventListTrait, LegacyMIDICCOutEvent, NoteExpressionValueEvent, NoteExpressionTypeIDs_::kTuningTypeID, NoteOffEvent, NoteOnEvent}}};

use crate::Event;

pub fn event_to_vst3_event(event: &Event) -> Option<Vst3Event> {
    match event {
        Event::NoteOn { sample_offset, channel, key, note, velocity } => {
            Some(Vst3Event {
                busIndex: 0,
                sampleOffset: *sample_offset as _,
                ppqPosition: 0.0,
                flags: EventFlags_::kIsLive as _,
                r#type: EventTypes_::kNoteOnEvent as _,
                __field0: Event__type0 { noteOn: NoteOnEvent {
                    channel: *channel,
                    pitch: *key,
                    tuning: 0.0,
                    velocity: *velocity as _,
                    length: 0,
                    noteId: *note,
                }},
            })
        }

        Event::NoteOff { sample_offset, channel, key, note, velocity } => {
            Some(Vst3Event {
                busIndex: 0,
                sampleOffset: *sample_offset as _,
                ppqPosition: 0.0,
                flags: EventFlags_::kIsLive as _,
                r#type: EventTypes_::kNoteOffEvent as _,
                __field0: Event__type0 { noteOff: NoteOffEvent {
                    channel: *channel,
                    pitch: *key,
                    velocity: *velocity as _,
                    noteId: *note,
                    tuning: 0.0,
                }},
            })
        }

        Event::PitchBend { sample_offset, note, semitones, .. } => {
            // VST3 kTuningTypeID normalises ±120 semitones to [0.0, 1.0]
            Some(Vst3Event {
                busIndex: 0,
                sampleOffset: *sample_offset as _,
                ppqPosition: 0.0,
                flags: EventFlags_::kIsLive as _,
                r#type: EventTypes_::kNoteExpressionValueEvent as _,
                __field0: Event__type0 { noteExpressionValue: NoteExpressionValueEvent {
                    typeId: kTuningTypeID,
                    noteId: *note,
                    value: (*semitones + 120.0) / 240.0,
                }},
            })
        }

        Event::Midi { sample_offset, data } => {
            let status = data[0] & 0xF0;
            let channel = (data[0] & 0x0F) as i8;

            // VST3 only supports LegacyMIDICCOutEvent for MIDI output - map what we can.
            let (control_number, value, value2) = match status {
                0xB0 => (data[1], data[2] as i8, 0i8),         // Control Change
                0xC0 => (129u8, data[1] as i8, 0i8),           // Program Change
                0xD0 => (131u8, data[1] as i8, 0i8),           // Channel Pressure
                0xE0 => (128u8, data[2] as i8, data[1] as i8), // Pitch Bend (MSB, LSB)
                _ => return None,
            };

            Some(Vst3Event {
                busIndex: 0,
                sampleOffset: *sample_offset as _,
                ppqPosition: 0.0,
                flags: EventFlags_::kIsLive as _,
                r#type: EventTypes_::kLegacyMIDICCOutEvent as _,
                __field0: Event__type0 { midiCCOut: LegacyMIDICCOutEvent {
                    controlNumber: control_number,
                    channel,
                    value,
                    value2,
                }},
            })
        }

        _ => None,
    }
}

pub struct EventIterator<'a> {
    event_list: Option<ComRef<'a, IEventList>>,
    index: usize,
}

impl EventIterator<'_> {
    pub fn new(event_list: *mut IEventList) -> Self {
        Self {
            event_list: unsafe { ComRef::from_raw(event_list) },
            index: 0,
        }        
    }
}

impl Iterator for EventIterator<'_> {
    type Item = Event;
    
    fn next(&mut self) -> Option<Self::Item> {
        let event_list = self.event_list?;

        if self.index >= unsafe { event_list.getEventCount() } as usize {
            return None;
        }

        let mut event: vst3::Steinberg::Vst::Event = unsafe { mem::zeroed() };
        let result = unsafe { event_list.getEvent(self.index as _, &mut event) };
        if result != kResultOk {
            return None;
        }

        self.index += 1;

        match event.r#type as _ {
            Vst::Event_::EventTypes_::kNoteOnEvent => unsafe {
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

            Vst::Event_::EventTypes_::kLegacyMIDICCOutEvent => unsafe {
                match event.__field0.midiCCOut.controlNumber {
                    0..=127 => Some(Event::Midi { // Controller Change
                        sample_offset: event.sampleOffset as _,
                        data: [
                            0xB0 | event.__field0.midiCCOut.channel as u8,
                            event.__field0.midiCCOut.controlNumber as u8,
                            event.__field0.midiCCOut.value as u8,
                        ]
                    }),
                    128 => Some(Event::Midi { // Pitch Bend
                        sample_offset: event.sampleOffset as _,
                        data: [
                            0xE0 | event.__field0.midiCCOut.channel as u8,
                            event.__field0.midiCCOut.value2 as u8, // LSB
                            event.__field0.midiCCOut.value as u8,  // MSB
                        ]
                    }),
                    129 => Some(Event::Midi { // Program Change
                        sample_offset: event.sampleOffset as _,
                        data: [
                            0xC0 | event.__field0.midiCCOut.channel as u8,
                            event.__field0.midiCCOut.value as u8,
                            0,
                        ]
                    }),
                    131 => Some(Event::Midi { // Channel Pressure
                        sample_offset: event.sampleOffset as _,
                        data: [
                            0xD0 | event.__field0.midiCCOut.channel as u8,
                            event.__field0.midiCCOut.value as u8,
                            0,
                        ]
                    }),
                    _ => None
                }
            },
            _ => None
        }
    }
}
