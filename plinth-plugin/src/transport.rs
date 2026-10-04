use std::ops::Range;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeSignature {
    pub numerator: u32,
    pub denominator: u32,
}

pub struct Transport {
    pub(crate) playing: bool,
    pub(crate) tempo: Option<f64>,
    pub(crate) time_signature: Option<TimeSignature>,
    pub(crate) position_samples: Option<i64>,
    pub(crate) position_beats: Option<f64>,
    pub(crate) loop_active: bool,
    pub(crate) loop_position_beats: Option<Range<f64>>,
}

impl Transport {
    pub fn new(
        playing: bool,
        tempo: Option<f64>,
        time_signature: Option<TimeSignature>,
        position_samples: Option<i64>,
        position_beats: Option<f64>,
        loop_active: bool,
        loop_position_beats: Option<Range<f64>>,
    ) -> Self {
        Self {
            playing,
            tempo,
            time_signature,
            position_samples,
            position_beats,
            loop_active,
            loop_position_beats,
        }
    }

    pub fn playing(&self) -> bool {
        self.playing
    }

    pub fn tempo(&self) -> Option<f64> {
        self.tempo
    }

    pub fn time_signature(&self) -> Option<TimeSignature> {
        self.time_signature
    }

    pub fn position_samples(&self) -> Option<i64> {
        self.position_samples
    }

    pub fn position_beats(&self) -> Option<f64> {
        self.position_beats
    }

    pub fn loop_active(&self) -> bool {
        self.loop_active
    }

    pub fn loop_position_beats(&self) -> Option<Range<f64>> {
        self.loop_position_beats.clone()
    }
}
