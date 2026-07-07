use std::cmp;
use std::collections::BTreeMap;

use vst3::{ComRef, Steinberg::{kResultOk, Vst::{IParamValueQueueTrait, IParameterChanges, IParameterChangesTrait, ParamID, ParamValue}}};

use crate::{event::Event, ParameterId};

/// Reserved VST3 parameter-ID blocks.
///
/// Each block holds 16 hidden parameters, one per MIDI channel.
#[derive(Default)]
pub(super) struct MidiParameterIds {
    pub pitch_bend: Option<[ParameterId; 16]>,
    pub channel_pressure: Option<[ParameterId; 16]>,
    pub program_change: Option<[ParameterId; 16]>,
    pub cc: BTreeMap<u8, [ParameterId; 16]>,
}

/// Map a VST3 parameter change event to an `Event`, recognising reserved MIDI blocks.
///
/// Checks pitch-bend, channel-pressure, CC blocks in that order, defaulting to `Event::ParameterValue` for user parameters.
pub(super) fn parameter_change_to_event(
    id: ParamID,
    value: ParamValue,
    offset: usize,
    midi_ids: &MidiParameterIds,
) -> Event {
    // Pitch bend: VST3 normalizes to [0, 1]: map to [-2, +2] semitones
    if let Some(channel) = &midi_ids
        .pitch_bend
        .and_then(|pb_ids| pb_ids.iter().position(|&pid| pid == id))
    {
        let semitones = (value - 0.5) * 4.0;
        return Event::MidiPitchBend {
            sample_offset: offset,
            channel: *channel as _,
            semitones,
        };
    }

    // Channel pressure: VST3 normalizes to [0, 1]: passed through as it is.
    if let Some(channel) = &midi_ids
        .channel_pressure
        .and_then(|cp_ids| cp_ids.iter().position(|&pid| pid == id))
    {
        return Event::MidiChannelPressure {
            sample_offset: offset,
            channel: *channel as _,
            value,
        };
    }

    // Program change: VST3 normalizes to [0, 1]: round to the nearest program number.
    if let Some(channel) = &midi_ids
        .program_change
        .and_then(|pc_ids| pc_ids.iter().position(|&pid| pid == id))
    {
        return Event::MidiProgramChange {
            sample_offset: offset,
            channel: *channel as _,
            program: (value * 127.0).round() as u8,
        };
    }

    // MIDI CC: VST3 normalizes to [0, 1]: passed through as it is.
    for (&cc, cc_ids) in &midi_ids.cc {
        if let Some(channel) = cc_ids.iter().position(|&pid| pid == id) {
            return Event::MidiControlChange {
                sample_offset: offset,
                channel: channel as _,
                controller: cc,
                value,
            };
        }
    }

    // Fall through to ordinary parameter
    Event::ParameterValue {
        sample_offset: offset,
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
