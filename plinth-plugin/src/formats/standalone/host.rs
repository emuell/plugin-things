use std::{cell::Cell, rc::Rc, sync::Arc};

use crate::{Host, ParameterId, ParameterValue, Parameters, Plugin};

use super::parameters::StandaloneParameterEventMap;

pub struct StandaloneHost<P: Plugin> {
    plugin: Rc<P>,
    parameter_event_map: Arc<StandaloneParameterEventMap>,
    requested_window_size: Rc<Cell<Option<(f64, f64)>>>,
}

impl<P: Plugin> StandaloneHost<P> {
    pub fn new(
        plugin: Rc<P>,
        parameter_event_map: Arc<StandaloneParameterEventMap>,
        requested_window_size: Rc<Cell<Option<(f64, f64)>>>,
    ) -> Self {
        Self {
            plugin,
            parameter_event_map,
            requested_window_size,
        }
    }
}

impl<P: Plugin> Host for StandaloneHost<P> {
    fn can_resize(&self) -> bool {
        true
    }

    fn resize_view(&self, width: f64, height: f64) -> bool {
        self.requested_window_size.set(Some((width, height)));
        true
    }

    fn change_parameter_value(&self, id: ParameterId, normalized: ParameterValue) {
        // Directly set the new value in the main thread.
        self.plugin.with_parameters(|parameters| {
            if let Some(parameter) = parameters.get(id) {
                parameter.set_normalized_value(normalized);
            } else {
                tracing::warn!("Unknown parameter: {id}");
            }
        });
        // Add parameter change event for the processor.
        self.parameter_event_map
            .change_parameter_value(id, normalized);
    }

    fn start_parameter_change(&self, _id: ParameterId) {}
    fn end_parameter_change(&self, _id: ParameterId) {}

    fn reload_parameters(&self) {}

    fn mark_state_dirty(&self) {}
}
