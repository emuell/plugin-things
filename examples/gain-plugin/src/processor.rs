use plinth_plugin::{Event, FloatParameter, Parameters, ProcessState, Processor, Transport, plinth_core::signals::signal::{Signal, SignalMut}, tracing};

use crate::parameters::{GainParameter, GainParameters};

fn db_to_amplitude(db: f32) -> f32 {
    10.0_f32.powf(db / 20.0)
}

pub struct GainPluginProcessor {
    parameters: GainParameters,
}

impl GainPluginProcessor {
    pub fn new(parameters: GainParameters) -> Self {
        Self {
            parameters,
        }
    }
}

impl Processor for GainPluginProcessor {
    fn reset(&mut self) {
    }

    fn process(
        &mut self,
        buffer: &mut impl SignalMut,
        _aux: Option<&impl Signal>,
        _transport: Option<Transport>,
        input_events: impl Iterator<Item = Event>,
        _output_events: &mut impl Extend<Event>,
    ) -> ProcessState {
        for event in input_events {
            match self.parameters.process_event(&event) {
                Ok(_) => {},
                Err(e) => {
                    tracing::error!("Error processing event: {e:?}");
                },
            }
        }

        let gain_db = self.parameters.value::<FloatParameter>(GainParameter::Gain);
        let gain = db_to_amplitude(gain_db as _);

        for channel in buffer.iter_channels_mut() {
            for sample in channel.iter_mut() {
                *sample *= gain;
            }
        }

        ProcessState::Normal
    }

    fn process_events(&mut self, input_events: impl Iterator<Item = Event>, _output_events: &mut impl Extend<Event>) {
        for event in input_events {
            match self.parameters.process_event(&event) {
                Ok(_) => {},
                Err(e) => {
                    tracing::error!("Error processing event: {e:?}");
                },
            }
        }
    }
}
