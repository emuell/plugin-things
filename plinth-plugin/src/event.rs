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

    PitchBend {
        sample_offset: usize,
        channel: i16,
        key: i16,
        note: i32,
        semitones: f64,
    },

    /// Raw 3-byte MIDI message.
    /// 
    /// Note events never will be *received* as `Midi` event but as dedicated `NoteOn`, `NoteOff` or `Pitchbend` event.
    ///
    /// Note: VST3 maps `Midi` events via `LegacyMIDICCOutEvent` which only supports the following status bytes:
    /// `0xB0` Control Change, `0xC0` Program Change, `0xD0` Channel Pressure, `0xE0` Pitch Bend.
    /// Other MIDI events will silently be dropped. CLAP and AU pass the raw bytes through unchanged.
    Midi {
        sample_offset: usize,
        data: [u8; 3],
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
    pub fn midi_control_change(sample_offset: usize, channel: u8, controller: u8, value: u8) -> Self {
        Self::Midi { sample_offset, data: [0xB0 | (channel & 0x0F), controller, value] }
    }

    pub fn midi_pitch_bend(sample_offset: usize, channel: u8, lsb: u8, msb: u8) -> Self {
        Self::Midi { sample_offset, data: [0xE0 | (channel & 0x0F), lsb, msb] }
    }

    pub fn midi_channel_pressure(sample_offset: usize, channel: u8, pressure: u8) -> Self {
        Self::Midi { sample_offset, data: [0xD0 | (channel & 0x0F), pressure, 0] }
    }

    pub fn midi_program_change(sample_offset: usize, channel: u8, program: u8) -> Self {
        Self::Midi { sample_offset, data: [0xC0 | (channel & 0x0F), program, 0] }
    }

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
            Event::PitchBend { sample_offset, .. } => *sample_offset,
            Event::Midi { sample_offset, .. } => *sample_offset,
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
