use plinth_core::signals::signal::{Signal, SignalMut};

use crate::{event::Event, transport::Transport};

#[derive(Clone, Default)]
pub struct ProcessorConfig {
    pub sample_rate: f64,
    pub min_block_size: usize,
    pub max_block_size: usize,
    pub process_mode: ProcessMode,
}

#[derive(Clone, Copy, Default, PartialEq)]
pub enum ProcessMode {
    #[default]
    Realtime,
    Offline,
}

pub enum ProcessState {
    Error,
    Normal,
    Tail(usize),
    KeepAlive,
}

pub trait Processor: Send {
    fn reset(&mut self);
    fn process(&mut self, buffer: &mut impl SignalMut, aux: Option<&impl Signal>, transport: Option<Transport>, input_events: impl Iterator<Item = Event>, output_events: &mut impl Extend<Event>) -> ProcessState;
    // Called when there's no audio to process
    fn process_events(&mut self, input_events: impl Iterator<Item = Event>, output_events: &mut impl Extend<Event>);
}
