use std::collections::BTreeMap;
use std::ffi::c_void;
use std::mem::size_of;

use clap_sys::events::{clap_event_header, clap_event_midi, clap_event_note, clap_event_note_expression, clap_event_param_mod, clap_event_param_value, clap_input_events, clap_output_events, CLAP_CORE_EVENT_SPACE_ID, CLAP_EVENT_IS_LIVE, CLAP_EVENT_MIDI, CLAP_EVENT_NOTE_EXPRESSION, CLAP_EVENT_NOTE_OFF, CLAP_EVENT_NOTE_ON, CLAP_EVENT_PARAM_MOD, CLAP_EVENT_PARAM_VALUE, CLAP_NOTE_EXPRESSION_TUNING};

use crate::{parameters::info::ParameterInfo, Event, ParameterId};

use super::parameters::map_parameter_value_from_clap;

pub fn send_note_event_to_host(event: &Event, out_events: *const clap_output_events) {
    if out_events.is_null() {
        return;
    }
    let out = unsafe { &*out_events };

    match event {
        Event::NoteOn { sample_offset, channel, key, note, velocity } => {
            let e = clap_event_note {
                header: clap_event_header {
                    size: size_of::<clap_event_note>() as u32,
                    time: *sample_offset as u32,
                    space_id: CLAP_CORE_EVENT_SPACE_ID,
                    type_: CLAP_EVENT_NOTE_ON,
                    flags: CLAP_EVENT_IS_LIVE,
                },
                note_id: *note,
                port_index: 0,
                channel: *channel,
                key: *key,
                velocity: *velocity,
            };
            unsafe { (out.try_push.unwrap())(out, &e as *const clap_event_note as _) };
        }

        Event::NoteOff { sample_offset, channel, key, note, velocity } => {
            let e = clap_event_note {
                header: clap_event_header {
                    size: size_of::<clap_event_note>() as u32,
                    time: *sample_offset as u32,
                    space_id: CLAP_CORE_EVENT_SPACE_ID,
                    type_: CLAP_EVENT_NOTE_OFF,
                    flags: CLAP_EVENT_IS_LIVE,
                },
                note_id: *note,
                port_index: 0,
                channel: *channel,
                key: *key,
                velocity: *velocity,
            };
            unsafe { (out.try_push.unwrap())(out, &e as *const clap_event_note as _) };
        }

        Event::PitchBend { sample_offset, channel, key, note, semitones } => {
            let e = clap_event_note_expression {
                header: clap_event_header {
                    size: size_of::<clap_event_note_expression>() as u32,
                    time: *sample_offset as u32,
                    space_id: CLAP_CORE_EVENT_SPACE_ID,
                    type_: CLAP_EVENT_NOTE_EXPRESSION,
                    flags: CLAP_EVENT_IS_LIVE,
                },
                expression_id: CLAP_NOTE_EXPRESSION_TUNING,
                note_id: *note,
                port_index: 0,
                channel: *channel,
                key: *key,
                value: *semitones,
            };
            unsafe { (out.try_push.unwrap())(out, &e as *const clap_event_note_expression as _) };
        }

        Event::Midi { sample_offset, data } => {
            let e = clap_event_midi {
                header: clap_event_header {
                    size: size_of::<clap_event_midi>() as u32,
                    time: *sample_offset as u32,
                    space_id: CLAP_CORE_EVENT_SPACE_ID,
                    type_: CLAP_EVENT_MIDI,
                    flags: CLAP_EVENT_IS_LIVE,
                },
                port_index: 0,
                data: *data,
            };
            unsafe { (out.try_push.unwrap())(out, &e as *const clap_event_midi as _) };
        }

        _ => {}
    }
}

pub struct EventIterator<'a> {
    parameter_info: &'a BTreeMap<ParameterId, ParameterInfo>,
    events: &'a clap_input_events,
    index: u32,
}

impl<'a> EventIterator<'a> {
    pub fn new(parameter_info: &'a BTreeMap<ParameterId, ParameterInfo>, events: &'a clap_input_events) -> Self {
        Self {
            parameter_info,
            events,
            index: 0,
        }
    }

    fn parameter_info(&self, parameter_id: u32, cookie: *mut c_void) -> &ParameterInfo {
        if !cookie.is_null() {
            unsafe { &*(cookie as *mut ParameterInfo) }
        } else {
            self.parameter_info.get(&parameter_id).unwrap()
        }
    }
}

impl Iterator for EventIterator<'_> {
    type Item = Event;

    fn next(&mut self) -> Option<Self::Item> {
        let events_size = unsafe { (self.events.size.unwrap())(self.events) };

        loop {
            if self.index >= events_size {
                return None;
            }
    
            let header = unsafe { (self.events.get.unwrap())(self.events, self.index) };
            self.index += 1;

            if unsafe { *header }.space_id != CLAP_CORE_EVENT_SPACE_ID {
                continue;
            }

            let event = match (unsafe { *header }).type_ {
                CLAP_EVENT_NOTE_ON => {
                    let event = unsafe { &*(header as *const clap_event_note) };

                    Event::NoteOn {
                        sample_offset: event.header.time as _,
                        channel: event.channel,
                        key: event.key,
                        note: event.note_id,
                        velocity: event.velocity,
                    }
                }

                CLAP_EVENT_NOTE_OFF => {
                    let event = unsafe { &*(header as *const clap_event_note) };

                    Event::NoteOff {
                        sample_offset: event.header.time as _,
                        channel: event.channel,
                        key: event.key,
                        note: event.note_id,
                        velocity: event.velocity,
                    }
                }

                CLAP_EVENT_NOTE_EXPRESSION => {
                    let event = unsafe { &*(header as *const clap_event_note_expression) };
                    if event.expression_id != CLAP_NOTE_EXPRESSION_TUNING {
                        continue;
                    }

                    Event::PitchBend {
                        sample_offset: event.header.time as _,
                        channel: event.channel,
                        key: event.key,
                        note: event.note_id,
                        semitones: event.value,
                    }
                }

                CLAP_EVENT_MIDI => {
                    let event = unsafe { &*(header as *const clap_event_midi) };
                    Event::Midi { 
                        sample_offset: event.header.time as _, 
                        data: event.data,
                    }
                }

                CLAP_EVENT_PARAM_VALUE => {
                    let event = unsafe { &*(header as *const clap_event_param_value) };
                    let parameter_info = self.parameter_info(event.param_id, event.cookie);

                    let value = map_parameter_value_from_clap(parameter_info, event.value);

                    Event::ParameterValue {
                        sample_offset: event.header.time as _,
                        id: event.param_id,
                        value,
                    }
                },
    
                CLAP_EVENT_PARAM_MOD => {
                    let event = unsafe { &*(header as *const clap_event_param_mod) };
                    let parameter_info = self.parameter_info(event.param_id, event.cookie);

                    let amount = map_parameter_value_from_clap(parameter_info, event.amount);

                    Event::ParameterModulation {
                        sample_offset: event.header.time as _,
                        id: event.param_id,
                        amount,
                    }
                },
    
                _ => {
                    continue;
                }
            };

            return Some(event);
        }
    }
}
