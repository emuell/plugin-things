/// Number of MIDI channels on the plugin's event bus.
pub(super) const MIDI_CHANNEL_COUNT: usize = 16;
/// Number of MIDI CC controller values that can be enabled.
pub(super) const MIDI_CONTROLLER_COUNT: usize = 128;

/// Compile-time declaration of which raw MIDI messages a plugin wants to receive as `Event`s.
///
/// Only covers plain MIDI wire messages. Per-note expression are a separate mechanism enabled
/// via [`crate::NoteExpressions`].
///
/// Midi events do register dummy, hidden, per channel parameters in VST3 plugins, so only necessary
/// event types and CCs should be enabled to avoid adding lots of dummy parameters!
///
/// Example:
/// ```ignore
/// const MIDI_CAPABILITIES: MidiCapabilities = MidiCapabilities::NONE
///     .with_pitch_bend()
///     .with_channel_pressure()
///     .with_control_change(1)
///     .with_control_change_range(20, 31);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MidiCapabilities {
    pitch_bend: bool,
    channel_pressure: bool,
    poly_pressure: bool,
    program_change: bool,
    cc_mask: u128,
}

impl Default for MidiCapabilities {
    fn default() -> Self {
        Self::NONE
    }
}

impl MidiCapabilities {
    /// No MIDI capabilities.
    pub const NONE: Self = Self {
        pitch_bend: false,
        channel_pressure: false,
        program_change: false,
        poly_pressure: false,
        cc_mask: 0,
    };

    /// Enable delivery of channel-wide pitch bend as [`Event::MidiPitchBend`].
    pub const fn with_pitch_bend(mut self) -> Self {
        self.pitch_bend = true;
        self
    }

    /// Enable delivery of channel pressure (mono aftertouch) as [`Event::MidiChannelPressure`].
    pub const fn with_channel_pressure(mut self) -> Self {
        self.channel_pressure = true;
        self
    }

    /// Enable delivery of polyphonic key pressure (poly aftertouch) delivered as a raw MIDI byte
    /// message, as [`Event::MidiPolyPressure`].
    pub const fn with_poly_pressure(mut self) -> Self {
        self.poly_pressure = true;
        self
    }

    /// Enable delivery of program change messages as [`Event::MidiProgramChange`].
    pub const fn with_program_change(mut self) -> Self {
        self.program_change = true;
        self
    }

    /// Enable delivery of the given CC number as [`Event::MidiControlChange`].
    pub const fn with_control_change(mut self, cc: u8) -> Self {
        assert!(cc < MIDI_CONTROLLER_COUNT as u8, "MIDI CC number must be 0..=127");
        self.cc_mask |= 1u128 << cc;
        self
    }

    /// Enable delivery of all CC numbers in the inclusive range `[start, end]` as [`Event::MidiControlChange`].
    pub const fn with_control_change_range(mut self, start: u8, end: u8) -> Self {
        assert!(
            start <= end && end < MIDI_CONTROLLER_COUNT as u8,
            "invalid CC range: must be start <= end <= 127"
        );
        let mut cc = start;
        while cc <= end {
            self = self.with_control_change(cc);
            cc += 1;
        }
        self
    }

    /// Returns `true` when no capabilities are enabled.
    pub const fn is_empty(&self) -> bool {
        self.cc_mask == 0
            && !self.pitch_bend
            && !self.channel_pressure
            && !self.program_change
            && !self.poly_pressure
    }

    /// Returns `true` if pitch bend is enabled.
    pub const fn midi_pitch_bend(&self) -> bool {
        self.pitch_bend
    }

    /// Returns `true` if channel pressure (mono aftertouch) is enabled.
    pub const fn midi_channel_pressure(&self) -> bool {
        self.channel_pressure
    }

    /// Returns `true` if raw-MIDI-delivered polyphonic key pressure is enabled.
    pub const fn midi_poly_pressure(&self) -> bool {
        self.poly_pressure
    }

    /// Returns `true` if program change is enabled.
    pub const fn midi_program_change(&self) -> bool {
        self.program_change
    }

    /// Returns `true` if the given CC number is enabled.
    pub const fn has_midi_control_change(&self, cc: u8) -> bool {
        if cc >= MIDI_CONTROLLER_COUNT as u8 {
            return false;
        }
        (self.cc_mask >> cc) & 1 != 0
    }

    /// Returns the number of enabled CC numbers.
    pub const fn midi_control_change_count(&self) -> u32 {
        self.cc_mask.count_ones()
    }

    /// Iterates over enabled CC numbers in ascending order.
    pub fn midi_control_changes(&self) -> impl Iterator<Item = u8> + '_ {
        (0u8..MIDI_CONTROLLER_COUNT as u8).filter(move |&cc| self.has_midi_control_change(cc))
    }
}
