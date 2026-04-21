use std::collections::BTreeMap;
use std::ffi::c_void;
use std::mem::size_of;

use clap_sys::events::{clap_event_header, clap_event_midi, clap_event_note, clap_event_note_expression, clap_event_param_mod, clap_event_param_value, clap_input_events, clap_note_expression, clap_output_events, CLAP_CORE_EVENT_SPACE_ID, CLAP_EVENT_MIDI, CLAP_EVENT_NOTE_EXPRESSION, CLAP_EVENT_NOTE_OFF, CLAP_EVENT_NOTE_ON, CLAP_EVENT_PARAM_MOD, CLAP_EVENT_PARAM_VALUE, CLAP_NOTE_EXPRESSION_BRIGHTNESS, CLAP_NOTE_EXPRESSION_EXPRESSION, CLAP_NOTE_EXPRESSION_PAN, CLAP_NOTE_EXPRESSION_PRESSURE, CLAP_NOTE_EXPRESSION_TUNING, CLAP_NOTE_EXPRESSION_VIBRATO, CLAP_NOTE_EXPRESSION_VOLUME};

use crate::{formats::midi::{midi_event_to_bytes, note_channel, note_id, note_key, parse_midi_event}, parameters::info::ParameterInfo, Event, MidiCapabilities, NoteExpressions, ParameterId};

use super::parameters::map_parameter_value_from_clap;

/// Send a note, note expression or MIDI event to the host. `max_sample_offset` is the last valid
/// sample offset in the current block. Events get clamped to it.
pub fn send_note_event_to_host(event: &Event, out_events: *const clap_output_events, max_sample_offset: usize) {
    if out_events.is_null() {
        return;
    }
    let out = unsafe { &*out_events };

    let header = |size: usize, sample_offset: usize, type_: u16| {
        if sample_offset > max_sample_offset {
            tracing::debug!("Clamping output event sample offset {sample_offset} to {max_sample_offset}");
        }
        clap_event_header {
            size: size as u32,
            time: sample_offset.min(max_sample_offset) as u32,
            space_id: CLAP_CORE_EVENT_SPACE_ID,
            type_,
            // Plugin generated events are no live user events
            flags: 0,
        }
    };

    let push = |header: &clap_event_header| {
        if !unsafe { (out.try_push.unwrap())(out, header) } {
            tracing::debug!("Host rejected output event: event queue is full");
        }
    };

    let note_expression = |sample_offset: usize, channel: Option<u8>, key: Option<u8>, note_id: Option<u32>, expression_id: clap_note_expression, value: f64| {
        let clap_event = clap_event_note_expression {
            header: header(size_of::<clap_event_note_expression>(), sample_offset, CLAP_EVENT_NOTE_EXPRESSION),
            expression_id,
            note_id: note_id.map_or(-1, |id| id as i32),
            port_index: 0,
            channel: channel.map_or(-1, |channel| channel as i16),
            key: key.map_or(-1, |key| key as i16),
            value,
        };
        push(&clap_event.header);
    };

    match *event {
        Event::NoteOn { sample_offset, channel, key, note_id, velocity } => {
            let clap_event = clap_event_note {
                header: header(size_of::<clap_event_note>(), sample_offset, CLAP_EVENT_NOTE_ON),
                note_id: note_id.map_or(-1, |id| id as i32),
                port_index: 0,
                channel: channel as i16,
                key: key as i16,
                velocity,
            };
            push(&clap_event.header);
        }

        Event::NoteOff { sample_offset, channel, key, note_id, velocity } => {
            let clap_event = clap_event_note {
                header: header(size_of::<clap_event_note>(), sample_offset, CLAP_EVENT_NOTE_OFF),
                note_id: note_id.map_or(-1, |id| id as i32),
                port_index: 0,
                channel: channel.map_or(-1, |channel| channel as i16),
                key: key.map_or(-1, |key| key as i16),
                velocity,
            };
            push(&clap_event.header);
        }

        Event::PolyVolume { sample_offset, channel, key, note_id, gain } => {
            // pass value in [0..4] as it is
            note_expression(sample_offset, channel, key, note_id, CLAP_NOTE_EXPRESSION_VOLUME, gain);
        }
        Event::PolyPressure { sample_offset, channel, key, note_id, value } => {
            note_expression(sample_offset, channel, key, note_id, CLAP_NOTE_EXPRESSION_PRESSURE, value);
        }
        Event::PolyPan { sample_offset, channel, key, note_id, pan } => {
            // [-1, +1] -> CLAP pan: 0=left, 0.5=center, 1=right
            note_expression(sample_offset, channel, key, note_id, CLAP_NOTE_EXPRESSION_PAN, (pan + 1.0) / 2.0);
        }
        Event::PolyTuning { sample_offset, channel, key, note_id, semitones } => {
            note_expression(sample_offset, channel, key, note_id, CLAP_NOTE_EXPRESSION_TUNING, semitones);
        }
        Event::PolyVibrato { sample_offset, channel, key, note_id, amount } => {
            note_expression(sample_offset, channel, key, note_id, CLAP_NOTE_EXPRESSION_VIBRATO, amount);
        }
        Event::PolyExpression { sample_offset, channel, key, note_id, amount } => {
            note_expression(sample_offset, channel, key, note_id, CLAP_NOTE_EXPRESSION_EXPRESSION, amount);
        }
        Event::PolyBrightness { sample_offset, channel, key, note_id, amount } => {
            note_expression(sample_offset, channel, key, note_id, CLAP_NOTE_EXPRESSION_BRIGHTNESS, amount);
        }

        _ => {
            if let Some((sample_offset, data)) = midi_event_to_bytes(event) {
                let clap_event = clap_event_midi {
                    header: header(size_of::<clap_event_midi>(), sample_offset, CLAP_EVENT_MIDI),
                    port_index: 0,
                    data,
                };
                push(&clap_event.header);
            }
        }
    }
}

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

    fn parameter_info(&self, parameter_id: u32, cookie: *mut c_void) -> Option<&ParameterInfo> {
        if !cookie.is_null() {
            Some(unsafe { &*(cookie as *mut ParameterInfo) })
        } else {
            self.parameter_info.get(&parameter_id)
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

                    let (Some(channel), Some(key)) = (note_channel(event.channel), note_key(event.key)) else {
                        // CLAP spec requires that valid channels and keys are specified for note-ons.
                        tracing::debug!("Ignoring note-on with invalid channel {} or key {}", event.channel, event.key);
                        continue;
                    };

                    Some(Event::NoteOn {
                        sample_offset: event.header.time as _,
                        channel,
                        key,
                        note_id: note_id(event.note_id),
                        velocity: event.velocity,
                    })
                }

                CLAP_EVENT_NOTE_OFF => {
                    let event = unsafe { &*(header as *const clap_event_note) };

                    Some(Event::NoteOff {
                        sample_offset: event.header.time as _,
                        channel: note_channel(event.channel),
                        key: note_key(event.key),
                        note_id: note_id(event.note_id),
                        velocity: event.velocity,
                    })
                }

                CLAP_EVENT_NOTE_EXPRESSION => {
                    let event = unsafe { &*(header as *const clap_event_note_expression) };
                    let note_expressions = self.note_expressions;
                    let channel = note_channel(event.channel);
                    let note_id = note_id(event.note_id);
                    let key = note_key(event.key);
                    let value = event.value;
                    let sample_offset = event.header.time as usize;

                    match event.expression_id {
                        CLAP_NOTE_EXPRESSION_TUNING if note_expressions.tuning() => {
                            Some(Event::PolyTuning {
                                sample_offset,
                                channel,
                                key,
                                note_id,
                                // fractional semitones, -120 to +120
                                semitones: value,
                            })
                        }

                        CLAP_NOTE_EXPRESSION_PRESSURE if note_expressions.pressure() => {
                            Some(Event::PolyPressure {
                                sample_offset,
                                channel,
                                key,
                                note_id,
                                // pass value in [0..1] as it is
                                value,
                            })
                        }

                        CLAP_NOTE_EXPRESSION_VOLUME if note_expressions.volume() => {
                            Some(Event::PolyVolume {
                                sample_offset,
                                channel,
                                note_id,
                                key,
                                // pass value in [0..4] as it is
                                gain: value,
                            })
                        }

                        CLAP_NOTE_EXPRESSION_PAN if note_expressions.pan() => {
                            Some(Event::PolyPan {
                                sample_offset,
                                channel,
                                note_id,
                                key,
                                // CLAP pan: 0=left, 0.5=center, 1=right -> map to [-1, +1]
                                pan: value * 2.0 - 1.0,
                            })
                        }

                        CLAP_NOTE_EXPRESSION_VIBRATO if note_expressions.vibrato() => {
                            Some(Event::PolyVibrato {
                                sample_offset,
                                channel,
                                note_id,
                                key,
                                // pass value in [0..1] as it is
                                amount: value,
                            })
                        }

                        CLAP_NOTE_EXPRESSION_EXPRESSION if note_expressions.expression() => {
                            Some(Event::PolyExpression {
                                sample_offset,
                                channel,
                                note_id,
                                key,
                                // pass value in [0..1] as it is
                                amount: value,
                            })
                        }

                        CLAP_NOTE_EXPRESSION_BRIGHTNESS if note_expressions.brightness() => {
                            Some(Event::PolyBrightness {
                                sample_offset,
                                channel,
                                note_id,
                                key,
                                // pass value in [0..1] as it is
                                amount: value,
                            })
                        }

                        // Unknown or unsupported expression ID
                        _ => None,
                    }
                }

                // Convert raw MIDI bytes to CC / channel pressure / pitch bend / poly pressure events.
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
                    let Some(parameter_info) = self.parameter_info(event.param_id, event.cookie) else {
                        tracing::debug!("Ignoring parameter value event for unknown parameter id {}", event.param_id);
                        continue;
                    };

                    let value = map_parameter_value_from_clap(parameter_info, event.value);

                    Some(Event::ParameterValue {
                        sample_offset: event.header.time as _,
                        id: event.param_id,
                        value,
                    })
                }

                CLAP_EVENT_PARAM_MOD => {
                    let event = unsafe { &*(header as *const clap_event_param_mod) };
                    let Some(parameter_info) = self.parameter_info(event.param_id, event.cookie) else {
                        tracing::debug!("Ignoring parameter modulation event for unknown parameter id {}", event.param_id);
                        continue;
                    };

                    let amount = map_parameter_value_from_clap(parameter_info, event.amount);

                    Some(Event::ParameterModulation {
                        sample_offset: event.header.time as _,
                        id: event.param_id,
                        amount,
                    })
                }

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
