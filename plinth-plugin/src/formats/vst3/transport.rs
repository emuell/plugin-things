use vst3::Steinberg::{uint32, Vst::{ProcessContext, ProcessContext_::{StatesAndFlags, StatesAndFlags_::{kCycleActive, kCycleValid, kPlaying, kProjectTimeMusicValid, kTempoValid, kTimeSigValid}}}};

use crate::{TimeSignature, Transport};

impl From<&ProcessContext> for Transport {
    fn from(context: &ProcessContext) -> Self {
        let has_flag = |flag: StatesAndFlags| context.state & (flag as uint32) != 0;

        Self {
            playing: has_flag(kPlaying),
            tempo: has_flag(kTempoValid).then_some(context.tempo),
            time_signature: has_flag(kTimeSigValid).then_some(TimeSignature {
                numerator: context.timeSigNumerator as _,
                denominator: context.timeSigDenominator as _,
            }),
            position_samples: Some(context.projectTimeSamples), // Always valid in VST3
            position_beats: has_flag(kProjectTimeMusicValid).then_some(context.projectTimeMusic),
            loop_active: has_flag(kCycleActive),
            loop_position_beats: has_flag(kCycleValid).then_some(context.cycleStartMusic..context.cycleEndMusic),
        }
    }
}
