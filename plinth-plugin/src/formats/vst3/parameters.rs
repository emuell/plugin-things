use std::cmp;
use std::collections::HashMap;

use vst3::{ComRef, Steinberg::{kResultOk, Vst::{IParamValueQueueTrait, IParameterChanges, IParameterChangesTrait, ParamID, ParamValue}}};

use crate::event::Event;

/// A hidden VST3 parameter which maps to a MIDI event.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum MidiParameter {
    PitchBend { channel: u8 },
    ChannelPressure { channel: u8 },
    ProgramChange { channel: u8 },
    ControlChange { channel: u8, controller: u8 },
}

/// Map a VST3 parameter change event to an `Event`, recognising reserved MIDI blocks.
///
/// Parameter IDs that belong to a hidden MIDI block resolve to matching `Event::Midi*`.
/// All others default to `Event::ParameterValue` as regular user parameters.
pub(super) fn parameter_change_to_event(
    id: ParamID,
    value: ParamValue,
    sample_offset: usize,
    midi_parameters: &HashMap<ParamID, MidiParameter>,
) -> Event {
    match midi_parameters.get(&id) {
        // Pitch bend: VST3 normalizes to [0, 1]: map to [-2, +2] semitones.
        Some(&MidiParameter::PitchBend { channel }) => Event::MidiPitchBend {
            sample_offset,
            channel,
            semitones: (value - 0.5) * 4.0,
        },
        // Channel pressure: VST3 normalizes to [0, 1]: passed through as it is.
        Some(&MidiParameter::ChannelPressure { channel }) => Event::MidiChannelPressure {
            sample_offset,
            channel,
            value,
        },
        // Program change: VST3 normalizes to [0, 1]: round to the nearest program number.
        Some(&MidiParameter::ProgramChange { channel }) => Event::MidiProgramChange {
            sample_offset,
            channel,
            program: (value * 127.0).round() as u8,
        },
        // MIDI CC: VST3 normalizes to [0, 1]: passed through as it is.
        Some(&MidiParameter::ControlChange { channel, controller }) => Event::MidiControlChange {
            sample_offset,
            channel,
            controller,
            value,
        },
        // Ordinary user parameter
        None => Event::ParameterValue {
            sample_offset,
            id,
            value,
        },
    }
}

pub struct ParameterChangeIterator<'a> {
    parameter_changes: Option<ComRef<'a, IParameterChanges>>,
    midi_parameters: &'a HashMap<ParamID, MidiParameter>,
    offset: usize,
    index: usize,
    finished: bool,
}

impl<'a> ParameterChangeIterator<'a> {
    pub fn new(parameter_changes: *mut IParameterChanges, midi_parameters: &'a HashMap<ParamID, MidiParameter>) -> Self {
        Self {
            parameter_changes: unsafe { ComRef::from_raw(parameter_changes) },
            midi_parameters,
            offset: 0,
            index: 0,
            finished: false,
        }
    }
}

impl Iterator for ParameterChangeIterator<'_> {
    type Item = Event;

    fn next(&mut self) -> Option<Self::Item> {
        if self.finished {
            return None;
        }
        let parameter_changes = self.parameter_changes?;

        let parameter_count = unsafe { parameter_changes.getParameterCount() };
        assert!(parameter_count >= 0);
        if parameter_count == 0 {
            return None;
        }

        let current_offset = self.offset;
        let current_index = self.index;
        let mut nth = 0;

        let event = (0..unsafe { parameter_changes.getParameterCount() })
            .flat_map(|parameter_index| {
                let Some(value_queue) = (unsafe { ComRef::from_raw(parameter_changes.getParameterData(parameter_index)) }) else {
                    panic!();
                };

                let id = unsafe { value_queue.getParameterId() };

                (0..unsafe { value_queue.getPointCount() })
                .filter_map(move |point_index| {
                    let mut offset = 0;
                    let mut value = 0.0;
                    let result = unsafe { value_queue.getPoint(point_index, &mut offset, &mut value) };
                    if result != kResultOk {
                        panic!();
                    }

                    assert!(offset >= 0);
                    let offset = offset as usize;

                    match offset.cmp(&current_offset) {
                        cmp::Ordering::Equal => {
                            if nth >= current_index {
                                Some((id, offset, value))
                            } else {
                                nth += 1;
                                None
                            }
                        },

                        cmp::Ordering::Greater => Some((id, offset, value)),

                        cmp::Ordering::Less => None,
                    }
                })
            })
            .filter(|(_, offset, _)| *offset >= current_offset)
            .min_by_key(|(_, offset, _)| *offset);

        let Some(event) = event else {
            self.finished = true;
            return None;
        };

        let (id, offset, value) = event;

        if offset > self.offset {
            self.offset = offset;
            self.index = 0;
        } else {
            self.index += 1;
        }

        let event = parameter_change_to_event(id, value, offset, self.midi_parameters);
        Some(event)
    }
}
