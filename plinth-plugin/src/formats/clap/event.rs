use std::collections::BTreeMap;
use std::ffi::c_void;

use clap_sys::events::{clap_event_note, clap_event_note_expression, clap_event_param_mod, clap_event_param_value, clap_event_midi, clap_input_events, CLAP_CORE_EVENT_SPACE_ID, CLAP_EVENT_NOTE_EXPRESSION, CLAP_EVENT_NOTE_OFF, CLAP_EVENT_NOTE_ON, CLAP_EVENT_PARAM_MOD, CLAP_EVENT_PARAM_VALUE, CLAP_NOTE_EXPRESSION_TUNING, CLAP_EVENT_MIDI, CLAP_NOTE_EXPRESSION_BRIGHTNESS, CLAP_NOTE_EXPRESSION_EXPRESSION, CLAP_NOTE_EXPRESSION_PAN, CLAP_NOTE_EXPRESSION_PRESSURE, CLAP_NOTE_EXPRESSION_VIBRATO, CLAP_NOTE_EXPRESSION_VOLUME};

use crate::{formats::midi::parse_midi_event, parameters::info::ParameterInfo, Event, MidiCapabilities, NoteExpressions, ParameterId};

use super::parameters::map_parameter_value_from_clap;

pub struct EventIterator<'a> {
    note_expressions: NoteExpressions,
    midi_capabilities: MidiCapabilities,
    parameter_info: &'a BTreeMap<ParameterId, ParameterInfo>,
    events: &'a clap_input_events,
    index: u32,
}

impl<'a> EventIterator<'a> {
    pub fn new(parameter_info: &'a BTreeMap<ParameterId, ParameterInfo>, events: &'a clap_input_events, midi_capabilities: MidiCapabilities, note_expressions: NoteExpressions) -> Self {
        Self {
            midi_capabilities,
            note_expressions,
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

            let event: Option<Event> = match (unsafe { *header }).type_ {
                CLAP_EVENT_NOTE_ON => {
                    let event = unsafe { &*(header as *const clap_event_note) };

                    Some(Event::NoteOn {
                        sample_offset: event.header.time as _,
                        channel: event.channel,
                        key: event.key,
                        note: event.note_id,
                        velocity: event.velocity,
                    })
                }

                CLAP_EVENT_NOTE_OFF => {
                    let event = unsafe { &*(header as *const clap_event_note) };

                    Some(Event::NoteOff {
                        sample_offset: event.header.time as _,
                        channel: event.channel,
                        key: event.key,
                        note: event.note_id,
                        velocity: event.velocity,
                    })
                }

                CLAP_EVENT_NOTE_EXPRESSION => {
                    let event = unsafe { &*(header as *const clap_event_note_expression) };
                    let note_expressions = self.note_expressions;
                    let channel = event.channel;
                    let key = event.key;
                    let note = event.note_id;
                    let value = event.value;
                    let sample_offset = event.header.time as usize;

                    match event.expression_id {
                        CLAP_NOTE_EXPRESSION_TUNING if note_expressions.tuning() => {
                            Some(Event::PolyTuning {
                                sample_offset,
                                channel,
                                key,
                                note,
                                // fractional semitones, -120 to +120
                                semitones: value,
                            })
                        }

                        CLAP_NOTE_EXPRESSION_PRESSURE if note_expressions.pressure() => {
                            Some(Event::PolyPressure {
                                sample_offset,
                                channel,
                                key,
                                note,
                                // pass value in [0..1] as it is
                                value,
                            })
                        }

                        CLAP_NOTE_EXPRESSION_VOLUME if note_expressions.volume() => {
                            Some(Event::PolyVolume {
                                sample_offset,
                                channel,
                                key,
                                note,
                                // pass value in [0..4] as it is
                                gain: value,
                            })
                        }

                        CLAP_NOTE_EXPRESSION_PAN if note_expressions.pan() => {
                            Some(Event::PolyPan {
                                sample_offset,
                                channel,
                                key,
                                note,
                                // CLAP pan: 0=left, 0.5=center, 1=right -> map to [-1, +1]
                                pan: value * 2.0 - 1.0,
                            })
                        }

                        CLAP_NOTE_EXPRESSION_VIBRATO if note_expressions.vibrato() => {
                            Some(Event::PolyVibrato {
                                sample_offset,
                                channel,
                                key,
                                note,
                                // pass value in [0..1] as it is
                                amount: value,
                            })
                        }

                        CLAP_NOTE_EXPRESSION_EXPRESSION if note_expressions.expression() => {
                            Some(Event::PolyExpression {
                                sample_offset,
                                channel,
                                key,
                                note,
                                // pass value in [0..1] as it is
                                amount: value,
                            })
                        }

                        CLAP_NOTE_EXPRESSION_BRIGHTNESS if note_expressions.brightness() => {
                            Some(Event::PolyBrightness {
                                sample_offset,
                                channel,
                                key,
                                note,
                                // pass value in [0..1] as it is
                                amount: value,
                            })
                        }

                        // Unknown or unsupported expression ID
                        _ => None,
                    }
                }

                // Covert raw MIDI bytes to CC / channel pressure / pitch bend / poly pressure events.
                CLAP_EVENT_MIDI => {
                    let event = unsafe { &*(header as *const clap_event_midi) };
                    parse_midi_event(
                        &event.data,
                        event.header.time as usize,
                        self.midi_capabilities,
                    )
                }

                CLAP_EVENT_PARAM_VALUE => {
                    let event = unsafe { &*(header as *const clap_event_param_value) };
                    let parameter_info = self.parameter_info(event.param_id, event.cookie);

                    let value = map_parameter_value_from_clap(parameter_info, event.value);

                    Some(Event::ParameterValue {
                        sample_offset: event.header.time as _,
                        id: event.param_id,
                        value,
                    })
                },

                CLAP_EVENT_PARAM_MOD => {
                    let event = unsafe { &*(header as *const clap_event_param_mod) };
                    let parameter_info = self.parameter_info(event.param_id, event.cookie);

                    let amount = map_parameter_value_from_clap(parameter_info, event.amount);

                    Some(Event::ParameterModulation {
                        sample_offset: event.header.time as _,
                        id: event.param_id,
                        amount,
                    })
                },

                // All other event types (MIDI2, sysex, etc.) are unsupported and skipped.
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
