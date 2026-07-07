use std::marker::PhantomData;


use plinth_core::signals::{signal::SignalMut, slice::SignalSliceMut};

use crate::parameters::ParameterId;

#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum Event {
    // Note events

    NoteOn {
        sample_offset: usize,
        channel: i16,
        key: i16,
        note: i32,
        velocity: f64,
    },

    NoteOff {
        sample_offset: usize,
        channel: i16,
        key: i16,
        note: i32,
        velocity: f64,
    },

    // Per-note expression events (VST3/CLAP note expression)

    /// Per-note volume. `gain` is linear in [0, 4], where 1 is 0dB and 0 -INFdB.
    /// Requires [`crate::NoteExpressions::with_volume`].
    PolyVolume {
        sample_offset: usize,
        channel: i16,
        key: i16,
        note: i32,
        gain: f64,
    },

    /// Polyphonic key pressure (poly aftertouch) (VST3 `kPolyPressureEvent` or CLAP's `CLAP_NOTE_EXPRESSION_PRESSURE`). 
    /// `value` is in [0, 1]. The same value delivered as a raw MIDI byte message arrives as [`Event::MidiPolyPressure`].
    /// Requires [`crate::NoteExpressions::with_pressure`].
    PolyPressure {
        sample_offset: usize,
        channel: i16,
        key: i16,
        note: i32,
        value: f64,
    },

    /// Per-note panning. `pan` is in [-1, 1] (left..right).
    /// Requires [`crate::NoteExpressions::with_pan`].
    PolyPan {
        sample_offset: usize,
        channel: i16,
        key: i16,
        note: i32,
        pan: f64,
    },

    /// Per-note tuning offset in semitones. `semitones` is in [-120, +120].
    /// Requires [`crate::NoteExpressions::with_tuning`].
    PolyTuning {
        sample_offset: usize,
        channel: i16,
        key: i16,
        note: i32,
        semitones: f64,
    },

    /// Per-note vibrato. `amount` is in [0, 1].
    /// Rarely (if at all) used by hosts, but part of the CLAP specs.
    /// Requires [`crate::NoteExpressions::with_vibrato`].
    PolyVibrato {
        sample_offset: usize,
        channel: i16,
        key: i16,
        note: i32,
        amount: f64,
    },

    /// Per-note expression (MIDI MPE "slide" / CC 74 equivalent). `amount` is in [0, 1].
    /// Rarely (if at all) used by hosts, but part of the CLAP specs. Use `PolyBrightness` instead.
    /// Requires [`crate::NoteExpressions::with_expression`].
    PolyExpression {
        sample_offset: usize,
        channel: i16,
        key: i16,
        note: i32,
        amount: f64,
    },

    /// Per-note brightness a.k.a. timbre (CLAP brightness / VST3 brightness). `amount` is in [0, 1].
    /// Requires [`crate::NoteExpressions::with_brightness`].
    PolyBrightness {
        sample_offset: usize,
        channel: i16,
        key: i16,
        note: i32,
        amount: f64,
    },

    // MIDI channel based events

    /// Channel-wide MIDI pitch bend.
    /// `semitones` is the current bend in semitones, using the standard +-2 semitone range.
    /// Per-note pitch offset (VST3/CLAP note expression) arrives as [`Event::PolyTuning`].
    /// Requires [`MidiCapabilities::with_pitch_bend`].
    MidiPitchBend {
        sample_offset: usize,
        channel: i16,
        semitones: f64,
    },

    /// Channel-wide pressure (mono aftertouch). `value` is in [0, 1].
    /// Requires [`MidiCapabilities::with_channel_pressure`]. See also [`Event::MidiPolyPressure`]. 
    MidiChannelPressure {
        sample_offset: usize,
        channel: i16,
        value: f64,
    },

    /// Polyphonic key pressure (poly aftertouch), delivered as a raw MIDI byte message.
    /// `value` is in [0, 1]. Same value delivered via a native per-note mechanism (VST3 `kPolyPressureEvent`
    /// or CLAP note expression) instead arrives as [`Event::PolyPressure`].
    /// Requires [`MidiCapabilities::with_poly_pressure`]. See also [`Event::MidiChannelPressure`].
    MidiPolyPressure {
        sample_offset: usize,
        channel: i16,
        key: i16,
        note: i32,
        value: f64,
    },

    /// MIDI Program Change. `program` is the program number (0..=127).
    /// Requires [`MidiCapabilities::with_program_change`].
    MidiProgramChange {
        sample_offset: usize,
        channel: i16,
        program: u8,
    },

    /// MIDI Control Change. `controller` is the CC number (0..=127), `value` is in [0, 1].
    /// Requires the corresponding CC to be enabled via [`MidiCapabilities::with_control_change`].
    MidiControlChange {
        sample_offset: usize,
        channel: i16,
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
