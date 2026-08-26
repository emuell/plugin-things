use std::cmp;
use std::collections::BTreeMap;

use vst3::{ComRef, Steinberg::{kResultOk, Vst::{IParamValueQueueTrait, IParameterChanges, IParameterChangesTrait, ParamID, ParamValue}}};

use crate::{event::Event, ParameterId, midi_capabilities::MIDI_CHANNEL_COUNT};

/// Reserved VST3 parameter-ID blocks.
///
/// Each block holds 16 hidden parameters, one per MIDI channel.
#[derive(Default)]
pub(super) struct MidiParameterIds {
    pub pitch_bend: Option<[ParameterId; MIDI_CHANNEL_COUNT]>,
    pub channel_pressure: Option<[ParameterId; MIDI_CHANNEL_COUNT]>,
    pub program_change: Option<[ParameterId; MIDI_CHANNEL_COUNT]>,
    pub cc: BTreeMap<u8, [ParameterId; MIDI_CHANNEL_COUNT]>,
}

/// Map a VST3 parameter change event to an `Event`, recognising reserved MIDI blocks.
///
/// Checks pitch-bend, channel-pressure, CC blocks in that order, defaulting to `Event::ParameterValue` for user parameters.
pub(super) fn parameter_change_to_event(
    id: ParamID,
    value: ParamValue,
    sample_offset: usize,
    midi_ids: &MidiParameterIds,
) -> Event {
    let channel_of = |ids: Option<&[ParameterId; MIDI_CHANNEL_COUNT]>, id: ParamID| -> Option<i16> {
        ids.and_then(|ids| ids.iter().position(|&pid| pid == id).map(|pos| pos as i16))
    };

    // Pitch bend: VST3 normalizes to [0, 1]: map to [-2, +2] semitones
    if let Some(channel) = channel_of(midi_ids.pitch_bend.as_ref(), id) {
        let semitones = (value - 0.5) * 4.0;
        return Event::MidiPitchBend {
            sample_offset,
            channel,
            semitones,
        };
    }

    // Channel pressure: VST3 normalizes to [0, 1]: passed through as it is.
    if let Some(channel) = channel_of(midi_ids.channel_pressure.as_ref(), id) {
        return Event::MidiChannelPressure {
            sample_offset,
            channel,
            value,
        };
    }

    // Program change: VST3 normalizes to [0, 1]: round to the nearest program number.
    if let Some(channel) = channel_of(midi_ids.program_change.as_ref(), id) {
        let program = (value * 127.0).round() as u8;
        return Event::MidiProgramChange {
            sample_offset,
            channel,
            program,
        };
    }

    // MIDI CC: VST3 normalizes to [0, 1]: passed through as it is.
    for (&controller, controller_ids) in &midi_ids.cc {
        if let Some(channel) = channel_of(Some(controller_ids), id) {
            return Event::MidiControlChange {
                sample_offset,
                channel,
                controller,
                value,
            };
        }
    }

    // Fall through to ordinary parameter
    Event::ParameterValue {
        sample_offset,
        id,
        value,
    }
}

pub struct ParameterChangeIterator<'a> {
    parameter_changes: Option<ComRef<'a, IParameterChanges>>,
    midi_parameter_ids: &'a MidiParameterIds,
    offset: usize,
    index: usize,
    finished: bool,
}

impl<'a> ParameterChangeIterator<'a> {
    pub fn new(parameter_changes: *mut IParameterChanges, midi_parameter_ids: &'a MidiParameterIds) -> Self {
        Self {
            parameter_changes: unsafe { ComRef::from_raw(parameter_changes) },
            midi_parameter_ids,
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

        let event = parameter_change_to_event(id, value, offset, self.midi_parameter_ids);
        Some(event)
    }
}
