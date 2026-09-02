use std::cmp;
use std::collections::HashMap;

use vst3::{ComRef, Steinberg::{kResultOk, Vst::{IParamValueQueueTrait, IParameterChanges, IParameterChangesTrait, ParamID, ParamValue}}};

use crate::event::Event;
use crate::midi_capabilities::{MidiCapabilities, MIDI_CHANNEL_COUNT};
use crate::parameters::info::ParameterInfo;
use crate::ParameterId;

/// A hidden VST3 parameter which maps to a MIDI event.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum MidiParameter {
    PitchBend { channel: u8 },
    ChannelPressure { channel: u8 },
    ProgramChange { channel: u8 },
    ControlChange { channel: u8, controller: u8 },
}

/// The hidden VST3 parameters which map to MIDI events, and their lookup tables.
#[derive(Default)]
pub(super) struct MidiParameters {
    /// Hidden parameter ID -> MIDI event.
    parameters: HashMap<ParamID, MidiParameter>,
    /// MIDI event -> Hidden parameter ID.
    parameter_ids: HashMap<MidiParameter, ParamID>,
}

impl MidiParameters {
    /// Creates hidden VST3 MIDI parameters for each MIDI message type enabled in `capabilities`.
    ///
    /// This also appends one hidden [`ParameterInfo`] per MIDI channel to `parameter_infos`, so 16
    /// parameters are added in total per enabled MIDI message type.
    pub fn new(
        capabilities: &MidiCapabilities,
        user_ids: &[ParameterId],
        parameter_infos: &mut Vec<ParameterInfo>,
    ) -> Self {
        // Use any ids that do not collide with existing user_ids. MIDI parameters are not persistent,
        // so it shouldn't matter if they change, after e.g. new user parameters got added.
        let mut next_id: ParamID = 1;

        let mut alloc_block = |
            midi_parameters: &mut Self,
            infos: &mut Vec<ParameterInfo>,
            midi_parameter: &dyn Fn(u8) -> MidiParameter,
            name: &str| {
            for channel in 0..MIDI_CHANNEL_COUNT {
                while user_ids.contains(&next_id) {
                    next_id += 1;
                }
                infos.push(ParameterInfo::new(next_id,
                    format!("MIDI Channel {} {}", channel + 1, name)).hidden());
                let midi_parameter = midi_parameter(channel as u8);
                midi_parameters.parameters.insert(next_id, midi_parameter);
                midi_parameters.parameter_ids.insert(midi_parameter, next_id);
                next_id += 1;
            }
        };

        let mut midi_parameters = Self::default();

        if capabilities.midi_pitch_bend() {
            alloc_block(
                &mut midi_parameters,
                parameter_infos,
                &|channel| MidiParameter::PitchBend { channel },
                "Pitch Bend",
            );
        }

        if capabilities.midi_channel_pressure() {
            alloc_block(
                &mut midi_parameters,
                parameter_infos,
                &|channel| MidiParameter::ChannelPressure { channel },
                "Channel Pressure",
            );
        }

        if capabilities.midi_program_change() {
            alloc_block(
                &mut midi_parameters,
                parameter_infos,
                &|channel| MidiParameter::ProgramChange { channel },
                "Program Change",
            );
        }

        for cc in capabilities.midi_control_changes() {
            alloc_block(
                &mut midi_parameters,
                parameter_infos,
                &|channel| MidiParameter::ControlChange { channel, controller: cc },
                &format!("CC {}", cc),
            );
        }

        midi_parameters
    }

    /// Clear all memorized parameters and IDs.
    pub fn clear(&mut self) {
        self.parameters.clear();
        self.parameter_ids.clear();
    }

    /// Try to resolve a hidden parameter ID which represents the given MIDI event.
    pub fn parameter_id(&self, midi_parameter: &MidiParameter) -> Option<ParamID> {
        self.parameter_ids.get(midi_parameter).copied()
    }

    /// Map a VST3 parameter change event to an `Event`, converting reserved MIDI block
    /// parameters to `Event::Midi*`. All others default to `Event::ParameterValue` as
    /// regular user parameters.
    pub fn parameter_change_to_event(
        &self,
        id: ParamID,
        value: ParamValue,
        sample_offset: usize,
    ) -> Event {
        match self.parameters.get(&id) {
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
}

pub struct ParameterChangeIterator<'a> {
    parameter_changes: Option<ComRef<'a, IParameterChanges>>,
    midi_parameters: &'a MidiParameters,
    offset: usize,
    index: usize,
    finished: bool,
}

impl<'a> ParameterChangeIterator<'a> {
    pub fn new(parameter_changes: *mut IParameterChanges, midi_parameters: &'a MidiParameters) -> Self {
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

        let event = self.midi_parameters.parameter_change_to_event(id, value, offset);
        Some(event)
    }
}
