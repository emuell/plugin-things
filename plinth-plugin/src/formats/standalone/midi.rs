use std::sync::mpsc::Sender;

use midir::{MidiInput, MidiInputConnection};

use super::config::MidiInputConfig;
use crate::formats::midi::parse_midi_event;
use crate::{Event, MidiCapabilities};

/// Connect MIDI input ports and translate raw MIDI bytes into `Event`s, filtered by `capabilities`. Each enabled port gets its own `MidiInputConnection`.
pub fn connect_inputs(
    config: &MidiInputConfig,
    sender: Sender<Event>,
    capabilities: MidiCapabilities,
) -> Vec<MidiInputConnection<()>> {
    let midi_in = match MidiInput::new("plinth-standalone") {
        Ok(m) => m,
        Err(err) => {
            tracing::warn!("Failed to create MIDI input: {err}");
            return vec![];
        }
    };

    let ports = midi_in.ports();

    if ports.is_empty() {
        tracing::info!("No MIDI input ports available");
    }

    let mut connections = Vec::with_capacity(ports.len());

    for port in &ports {
        let port_name = midi_in.port_name(port).unwrap_or_else(|_| port.id());

        if config.port_names.as_ref().is_some_and(|names| !names.iter().any(|n| n == &port_name)) {
            continue;
        }

        let midi_in = match MidiInput::new("plinth-standalone") {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!("Failed to create MIDI input for port '{port_name}': {e}");
                continue;
            }
        };

        let sender = sender.clone();
        match midi_in.connect(
            port,
            "plinth-midi-input",
            move |_timestamp, data, _| {
                if let Some(event) = parse_midi_event(data, 0, capabilities) {
                    let _ = sender.send(event);
                }
            },
            (),
        ) {
            Ok(connection) => {
                tracing::info!("Connected MIDI input port '{port_name}'");
                connections.push(connection);
            }
            Err(err) => tracing::warn!("Failed to connect MIDI input port '{port_name}': {err}"),
        }
    }

    connections
}
