/// Compile-time declaration of which per-note expression dimensions a plugin wants to receive
/// as `Event`s (VST3 note expression / CLAP note expression).
///
/// Example:
/// ```ignore
/// const NOTE_EXPRESSIONS: NoteExpressions = NoteExpressions::NONE
///     .with_tuning()
///     .with_brightness();
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NoteExpressions {
    volume: bool,
    pan: bool,
    tuning: bool,
    vibrato: bool,
    expression: bool,
    brightness: bool,
    pressure: bool,
}

impl Default for NoteExpressions {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl NoteExpressions {
    /// No note expressions.
    pub const NONE: Self = Self {
        volume: false,
        pan: false,
        tuning: false,
        vibrato: false,
        expression: false,
        brightness: false,
        pressure: false,
    };

    /// All note expressions.
    pub const ALL: Self = Self {
        volume: true,
        pan: true,
        tuning: true,
        vibrato: true,
        expression: true,
        brightness: true,
        pressure: true,
    };

    /// A sensible common subset: volume, pan, tuning, brightness and pressure.
    /// Excludes vibrato and expression, which are rarely (if at all) sent by hosts.
    pub const DEFAULT: Self = Self::NONE
        .with_volume()
        .with_pan()
        .with_tuning()
        .with_brightness()
        .with_pressure();

    /// Enable delivery of per-note volume as [`crate::Event::PolyVolume`].
    pub const fn with_volume(mut self) -> Self {
        self.volume = true;
        self
    }

    /// Enable delivery of per-note panning as [`crate::Event::PolyPan`].
    pub const fn with_pan(mut self) -> Self {
        self.pan = true;
        self
    }

    /// Enable delivery of per-note tuning offset as [`crate::Event::PolyTuning`].
    pub const fn with_tuning(mut self) -> Self {
        self.tuning = true;
        self
    }

    /// Enable delivery of per-note vibrato as [`crate::Event::PolyVibrato`].
    pub const fn with_vibrato(mut self) -> Self {
        self.vibrato = true;
        self
    }

    /// Enable delivery of per-note expression (MPE "slide") as [`crate::Event::PolyExpression`].
    pub const fn with_expression(mut self) -> Self {
        self.expression = true;
        self
    }

    /// Enable delivery of per-note brightness as [`crate::Event::PolyBrightness`].
    pub const fn with_brightness(mut self) -> Self {
        self.brightness = true;
        self
    }

    /// Enable delivery of per-note pressure (poly aftertouch) as [`crate::Event::PolyPressure`].
    pub const fn with_pressure(mut self) -> Self {
        self.pressure = true;
        self
    }

    /// Returns `true` when no note expressions are enabled.
    pub const fn is_empty(&self) -> bool {
        !self.volume
            && !self.pan
            && !self.tuning
            && !self.vibrato
            && !self.expression
            && !self.brightness
            && !self.pressure
    }

    /// Returns `true` if per-note volume is enabled.
    pub const fn volume(&self) -> bool {
        self.volume
    }

    /// Returns `true` if per-note panning is enabled.
    pub const fn pan(&self) -> bool {
        self.pan
    }

    /// Returns `true` if per-note tuning is enabled.
    pub const fn tuning(&self) -> bool {
        self.tuning
    }

    /// Returns `true` if per-note vibrato is enabled.
    pub const fn vibrato(&self) -> bool {
        self.vibrato
    }

    /// Returns `true` if per-note expression (MPE "slide") is enabled.
    pub const fn expression(&self) -> bool {
        self.expression
    }

    /// Returns `true` if per-note brightness is enabled.
    pub const fn brightness(&self) -> bool {
        self.brightness
    }

    /// Returns `true` if per-note pressure (poly aftertouch) is enabled.
    pub const fn pressure(&self) -> bool {
        self.pressure
    }
}
