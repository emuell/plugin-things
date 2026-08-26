use std::marker::PhantomData;

use plinth_core::signals::{signal::SignalMut, slice::SignalSliceMut};

use crate::parameters::ParameterId;

#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum Event {
    // Note events
    //
    // Notes are addressed via a `(channel, key, note_id)` tuple, where:
    // * `Some` values are validated values.
    // * `None` values are wildcards, matching every voice regardless of that field.

    /// A note-on event.
    ///
    /// `note_id` usually will be `Some` in hosts with note expression support, else `None`.
    ///
    /// Requires [`Plugin::HAS_NOTE_INPUT`](crate::Plugin::HAS_NOTE_INPUT).
    NoteOn {
        sample_offset: usize,
        channel: u8,
        note_id: Option<u32>,
        key: u8,
        velocity: f64,
    },

    /// A note-off event.
    ///
    /// `note_id` likely will be `None` here even when the matching [`Event::NoteOn`] carried one,
    /// as hosts may issue a note id at note-on only.
    /// `channel` and `key` may be used as wildcards: a host may use this to release a whole
    /// channel or all active voices at once.
    ///
    /// Requires [`Plugin::HAS_NOTE_INPUT`](crate::Plugin::HAS_NOTE_INPUT).
    NoteOff {
        sample_offset: usize,
        channel: Option<u8>,
        note_id: Option<u32>,
        key: Option<u8>,
        velocity: f64,
    },

    // Per-note expression events (VST3/CLAP note expression)
    //
    // Addressed with the same `(channel, key, note_id)` tuple as note events, but:
    // * VST3 uses `note_id` only, so `channel` and `key` always are `None`.
    // * CLAP hosts usually pass `channel` and `key`, and optionally `note_id`.

    /// Per-note volume. `gain` is linear in [0, 4], where 1 is 0dB and 0 -INFdB.
    ///
    /// Requires [`Plugin::HAS_NOTE_INPUT`](crate::Plugin::HAS_NOTE_INPUT)
    /// and [`Plugin::NOTE_EXPRESSIONS::with_volume`](crate::NoteExpressions::with_volume).
    PolyVolume {
        sample_offset: usize,
        channel: Option<u8>,
        note_id: Option<u32>,
        key: Option<u8>,
        gain: f64,
    },

    /// Polyphonic key pressure (poly aftertouch, VST3's `kPolyPressureEvent` or
    /// CLAP's `CLAP_NOTE_EXPRESSION_PRESSURE`).
    ///
    /// `value` is in [0, 1]. The same value delivered as a raw MIDI byte message arrives as
    /// [`Event::MidiPolyPressure`].
    ///
    /// Requires [`Plugin::HAS_NOTE_INPUT`](crate::Plugin::HAS_NOTE_INPUT)
    /// and [`Plugin::NOTE_EXPRESSIONS::with_pressure`](crate::NoteExpressions::with_pressure).
    PolyPressure {
        sample_offset: usize,
        channel: Option<u8>,
        note_id: Option<u32>,
        key: Option<u8>,
        value: f64,
    },

    /// Per-note panning.
    ///
    /// `pan` is in [-1, 1] (left..right).
    ///
    /// Requires [`Plugin::HAS_NOTE_INPUT`](crate::Plugin::HAS_NOTE_INPUT)
    /// and [`Plugin::NOTE_EXPRESSIONS::with_pan`](crate::NoteExpressions::with_pan).
    PolyPan {
        sample_offset: usize,
        channel: Option<u8>,
        note_id: Option<u32>,
        key: Option<u8>,
        pan: f64,
    },

    /// Per-note tuning offset in semitones.
    ///
    /// `semitones` is in [-120, +120].
    ///
    /// Requires [`Plugin::HAS_NOTE_INPUT`](crate::Plugin::HAS_NOTE_INPUT)
    /// and [`Plugin::NOTE_EXPRESSIONS::with_tuning`](crate::NoteExpressions::with_tuning).
    PolyTuning {
        sample_offset: usize,
        channel: Option<u8>,
        note_id: Option<u32>,
        key: Option<u8>,
        semitones: f64,
    },

    /// Per-note vibrato. Rarely (if at all) used by hosts, but part of the CLAP specs.
    ///
    /// `amount` is in [0, 1].
    ///
    /// Requires [`Plugin::HAS_NOTE_INPUT`](crate::Plugin::HAS_NOTE_INPUT)
    /// and [`Plugin::NOTE_EXPRESSIONS::with_vibrato`](crate::NoteExpressions::with_vibrato).
    PolyVibrato {
        sample_offset: usize,
        channel: Option<u8>,
        note_id: Option<u32>,
        key: Option<u8>,
        amount: f64,
    },

    /// Per-note expression. Rarely (if at all) used by hosts, but part of the CLAP specs.
    /// You usually want [`Event::PolyBrightness`] instead.
    ///
    /// `amount` is in [0, 1].
    ///
    /// Requires [`Plugin::HAS_NOTE_INPUT`](crate::Plugin::HAS_NOTE_INPUT)
    /// and [`Plugin::NOTE_EXPRESSIONS::with_expression`](crate::NoteExpressions::with_expression).
    PolyExpression {
        sample_offset: usize,
        channel: Option<u8>,
        note_id: Option<u32>,
        key: Option<u8>,
        amount: f64,
    },

    /// Per-note brightness a.k.a. timbre (CLAP brightness / VST3 brightness).
    ///
    /// `amount` is in [0, 1].
    ///
    /// Requires [`Plugin::HAS_NOTE_INPUT`](crate::Plugin::HAS_NOTE_INPUT)
    /// and [`Plugin::NOTE_EXPRESSIONS::with_brightness`](crate::NoteExpressions::with_brightness).
    PolyBrightness {
        sample_offset: usize,
        channel: Option<u8>,
        note_id: Option<u32>,
        key: Option<u8>,
        amount: f64,
    },

    // MIDI channel based events
    //
    // NB: `channel` and `key` come from raw MIDI bytes or from per channel host parameters
    // and thus always are valid, never wildcards.

    /// Channel-wide MIDI pitch bend.
    /// Per-note pitch offset (VST3/CLAP note expression) arrives as [`Event::PolyTuning`].
    ///
    /// `semitones` is the current bend in semitones, using the standard +-2 semitone range.
    ///
    /// Requires [`Plugin::HAS_NOTE_INPUT`](crate::Plugin::HAS_NOTE_INPUT)
    /// and [`Plugin::MIDI_CAPABILITIES::with_pitch_bend`](crate::MidiCapabilities::with_pitch_bend).
    MidiPitchBend {
        sample_offset: usize,
        channel: u8,
        semitones: f64,
    },

    /// Channel-wide pressure (mono aftertouch).
    /// See also [`Event::MidiPolyPressure`].
    ///
    /// `value` is in [0, 1].
    ///
    /// Requires [`Plugin::HAS_NOTE_INPUT`](crate::Plugin::HAS_NOTE_INPUT)
    /// and [`Plugin::MIDI_CAPABILITIES::with_channel_pressure`](crate::MidiCapabilities::with_channel_pressure).
    MidiChannelPressure {
        sample_offset: usize,
        channel: u8,
        value: f64,
    },

    /// Polyphonic key pressure (poly aftertouch), delivered as a raw MIDI byte message.
    ///
    /// The same value delivered via a native per-note mechanism (VST3 `kPolyPressureEvent` or
    /// CLAP note expression) instead arrives as [`Event::PolyPressure`].
    /// See also [`Event::MidiChannelPressure`].
    ///
    /// `value` is in [0, 1].
    ///
    /// Requires [`Plugin::HAS_NOTE_INPUT`](crate::Plugin::HAS_NOTE_INPUT)
    /// and [`Plugin::MIDI_CAPABILITIES::with_poly_pressure`](crate::MidiCapabilities::with_poly_pressure).
    MidiPolyPressure {
        sample_offset: usize,
        channel: u8,
        key: u8,
        value: f64,
    },

    /// MIDI Program Change.
    ///
    /// `program` is the program number (0..=127).
    ///
    /// Requires [`Plugin::HAS_NOTE_INPUT`](crate::Plugin::HAS_NOTE_INPUT)
    /// and [`Plugin::MIDI_CAPABILITIES::with_program_change`](crate::MidiCapabilities::with_program_change).
    MidiProgramChange {
        sample_offset: usize,
        channel: u8,
        program: u8,
    },

    /// MIDI Control Change.
    ///
    /// `controller` is the CC number (0..=127).
    /// `value` is in [0, 1].
    ///
    /// Requires [`Plugin::HAS_NOTE_INPUT`](crate::Plugin::HAS_NOTE_INPUT) and the corresponding CC to be enabled
    /// via [`Plugin::MIDI_CAPABILITIES::with_control_change`](crate::MidiCapabilities::with_control_change).
    MidiControlChange {
        sample_offset: usize,
        channel: u8,
        controller: u8,
        value: f64,
    },

    // Parameter events

    StartParameterChange {
        id: ParameterId,
    },

    EndParameterChange {
        id: ParameterId,
    },

    ParameterValue {
        sample_offset: usize,
        id: ParameterId,
        value: f64,
    },

    ParameterModulation {
        sample_offset: usize,
        id: ParameterId,
        amount: f64,
    },
}

impl Event {
    pub fn split_signal_at_events<I, S>(signal: &mut S, events: I) -> SignalSplitter<'_, I, S>
    where
        I: Iterator<Item = Event>,
        S: SignalMut,
    {
        SignalSplitter::new(signal, events)
    }

    pub fn sample_offset(&self) -> usize {
        match self {
            Event::NoteOn { sample_offset, .. } => *sample_offset,
            Event::NoteOff { sample_offset, .. } => *sample_offset,
            Event::PolyVolume { sample_offset, .. } => *sample_offset,
            Event::PolyPan { sample_offset, .. } => *sample_offset,
            Event::PolyTuning { sample_offset, .. } => *sample_offset,
            Event::PolyVibrato { sample_offset, .. } => *sample_offset,
            Event::PolyExpression { sample_offset, .. } => *sample_offset,
            Event::PolyBrightness { sample_offset, .. } => *sample_offset,
            Event::PolyPressure { sample_offset, .. } => *sample_offset,
            Event::MidiPitchBend { sample_offset, .. } => *sample_offset,
            Event::MidiChannelPressure { sample_offset, .. } => *sample_offset,
            Event::MidiPolyPressure { sample_offset, .. } => *sample_offset,
            Event::MidiProgramChange { sample_offset, .. } => *sample_offset,
            Event::MidiControlChange { sample_offset, .. } => *sample_offset,
            Event::ParameterValue { sample_offset, .. } => *sample_offset,
            Event::ParameterModulation { sample_offset, .. } => *sample_offset,

            _ => 0
        }
    }
}

pub struct SignalSplitter<'signal, I, S>
where
    I: Iterator<Item = Event>,
    S: SignalMut,
{
    signal: *mut S,
    events: I,
    offset: usize,

    _phantom_lifetime: PhantomData<&'signal S>,
}

impl<'signal, I, S> SignalSplitter<'signal, I, S>
where
    I: Iterator<Item = Event>,
    S: SignalMut,
{
    pub fn new(signal: &'signal mut S, events: I) -> Self {
        Self {
            signal,
            events,
            offset: 0,

            _phantom_lifetime: PhantomData,
        }
    }
}

impl<'signal, I, S> Iterator for SignalSplitter<'signal, I, S>
where
    I: Iterator<Item = Event>,
    S: SignalMut,
{
    type Item = (SignalSliceMut<'signal, S>, Option<Event>);

    fn next(&mut self) -> Option<Self::Item> {
        let signal = unsafe { &mut *self.signal };

        loop {
            let Some(next_event) = self.events.next() else {
                if self.offset < signal.len() {
                    let signal_len = signal.len();
                    let signal_slice = signal.slice_mut(self.offset..);
                    self.offset = signal_len;

                    return Some((signal_slice, None));
                } else {
                    return None;
                }
            };

            match next_event {
                Event::ParameterValue { sample_offset, .. } |
                Event::ParameterModulation { sample_offset, .. } => {
                    let sample_offset = usize::min(sample_offset, signal.len());

                    let result = (signal.slice_mut(self.offset..sample_offset), Some(next_event));
                    self.offset = sample_offset;
                    return Some(result);
                },

                _ => { continue; },
            }
        }
    }
}
