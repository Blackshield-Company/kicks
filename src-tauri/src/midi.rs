use std::sync::{mpsc, Arc, Mutex};

use kicks_core::midi::MidiConfig;
use kicks_dsp::param::ParamSender;
use midir::{MidiInput, MidiInputConnection, MidiInputPort};

use crate::sync::LockOrRecover;

/// What happened to one incoming CC message.
#[derive(Debug, Clone, PartialEq)]
pub struct MidiEvent {
    pub channel: u8,
    pub cc: u8,
    pub raw_value: u8,
    /// `Some((parameter_id, value))` when a mapping matched and the value
    /// was sent to the engine.
    pub mapped: Option<(String, f32)>,
    /// True if the message was captured by MIDI learn instead of routed.
    pub learned: bool,
}

/// Shared handles the MIDI input thread needs to route CCs on its own,
/// without waiting for the UI to poll.
#[derive(Clone)]
pub struct MidiRoute {
    pub config: Arc<Mutex<MidiConfig>>,
    pub param_tx: Arc<Mutex<Option<ParamSender>>>,
}

/// Handle one raw MIDI message on the MIDI input thread.
///
/// Control Change messages are matched against the mappings and sent
/// straight to the engine's lock-free param queue (the audio thread applies
/// them at the next block). In learn mode the CC number is recorded instead.
/// Returns an event for the UI, or `None` for non-CC messages.
pub fn handle_message(route: &MidiRoute, message: &[u8]) -> Option<MidiEvent> {
    if message.len() < 3 || message[0] & 0xF0 != 0xB0 {
        return None;
    }
    let (channel, cc, raw_value) = (message[0] & 0x0F, message[1], message[2]);
    let mut event = MidiEvent {
        channel,
        cc,
        raw_value,
        mapped: None,
        learned: false,
    };

    let mapped = {
        let mut cfg = route.config.lock_or_recover();
        if cfg.learn_mode {
            cfg.last_cc = Some(cc);
            event.learned = true;
            None
        } else {
            let global = cfg.channel == 0;
            cfg.mappings
                .iter()
                .find(|m| m.cc == cc && (global || m.channel == channel))
                .map(|m| {
                    let v = m.min + (raw_value as f32 / 127.0) * (m.max - m.min);
                    (m.parameter_id.clone(), v)
                })
        }
    };
    if let Some((param, value)) = &mapped {
        if let Some(tx) = route.param_tx.lock_or_recover().as_ref() {
            if let Err(e) = tx.send(param, *value) {
                tracing::debug!("MIDI param {param} not queued: {e:?}");
            }
        }
    }
    event.mapped = mapped;
    Some(event)
}

/// Manages a connection to a MIDI input device.
///
/// CCs are routed to the engine on the MIDI input thread (see
/// [`handle_message`]). Events are also queued for the UI, which drains
/// them with `poll_midi_events` for learn mode and activity display only.
pub struct MidiManager {
    connection: Option<MidiInputConnection<()>>,
    event_rx: mpsc::Receiver<MidiEvent>,
    event_tx: mpsc::SyncSender<MidiEvent>,
}

/// UI events kept while nobody polls (older ones are dropped).
const UI_EVENT_BACKLOG: usize = 1024;

impl Default for MidiManager {
    fn default() -> Self {
        Self::new()
    }
}

impl MidiManager {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::sync_channel(UI_EVENT_BACKLOG);
        Self {
            connection: None,
            event_rx: rx,
            event_tx: tx,
        }
    }

    /// List all available MIDI input port names.
    pub fn list_ports() -> Result<Vec<String>, String> {
        let midi_in =
            MidiInput::new("Kicks").map_err(|e| format!("Failed to create MidiInput: {}", e))?;
        let ports = midi_in.ports();
        let names: Result<Vec<_>, _> = ports.iter().map(|p| midi_in.port_name(p)).collect();
        names.map_err(|e| format!("Failed to get port name: {}", e))
    }

    /// Connect to a MIDI input port by name.
    /// Returns an error if the port is not found or connection fails.
    pub fn connect(&mut self, port_name: &str, route: MidiRoute) -> Result<(), String> {
        // Drop any existing connection first
        self.disconnect();

        let midi_in =
            MidiInput::new("Kicks").map_err(|e| format!("Failed to create MidiInput: {}", e))?;
        let ports = midi_in.ports();

        let port: MidiInputPort = ports
            .into_iter()
            .find(|p| midi_in.port_name(p).ok().as_deref() == Some(port_name))
            .ok_or_else(|| format!("MIDI port '{}' not found", port_name))?;

        let tx = self.event_tx.clone();
        let conn = midi_in
            .connect(
                &port,
                port_name,
                move |_stamp, message, _| {
                    if let Some(ev) = handle_message(&route, message) {
                        // Never block the MIDI thread on a slow UI.
                        let _ = tx.try_send(ev);
                    }
                },
                (),
            )
            .map_err(|e| format!("Failed to connect MIDI: {}", e))?;

        self.connection = Some(conn);
        tracing::info!("Connected to MIDI device: {}", port_name);
        Ok(())
    }

    /// Disconnect from the active MIDI input device.
    pub fn disconnect(&mut self) {
        if self.connection.is_some() {
            // Dropping the connection closes it gracefully
            self.connection = None;
            tracing::info!("Disconnected from MIDI device");
        }
    }

    /// Check if currently connected to a MIDI device.
    pub fn is_connected(&self) -> bool {
        self.connection.is_some()
    }

    /// Drain all pending MIDI events for the UI.
    pub fn drain_events(&self) -> Vec<MidiEvent> {
        self.event_rx.try_iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kicks_core::midi::MidiMapping;
    use kicks_dsp::param::{param_channel, try_recv};

    fn route_with(
        mappings: Vec<MidiMapping>, channel: u8,
    ) -> (MidiRoute, kicks_dsp::param::ParamConsumer) {
        let (tx, rx) = param_channel();
        let cfg = MidiConfig {
            channel,
            mappings,
            ..MidiConfig::default()
        };
        (
            MidiRoute {
                config: Arc::new(Mutex::new(cfg)),
                param_tx: Arc::new(Mutex::new(Some(tx))),
            },
            rx,
        )
    }

    fn map(cc: u8, ch: u8, p: &str, min: f32, max: f32) -> MidiMapping {
        MidiMapping {
            cc,
            channel: ch,
            parameter_id: p.into(),
            label: p.into(),
            min,
            max,
        }
    }

    #[test]
    fn mapped_cc_goes_straight_to_the_param_queue() {
        let (route, mut rx) = route_with(vec![map(7, 0, "amp/master", 0.0, 1.0)], 0);
        let ev = handle_message(&route, &[0xB0, 7, 127]).unwrap();
        assert_eq!(ev.mapped, Some(("amp/master".to_string(), 1.0)));
        let (key, v) = try_recv(&mut rx).expect("queued without any UI poll");
        assert_eq!(key.as_str(), "amp/master");
        assert_eq!(v, 1.0);
    }

    #[test]
    fn value_is_scaled_into_mapping_range() {
        let (route, mut rx) = route_with(vec![map(1, 0, "delay/mix", 0.2, 0.6)], 0);
        handle_message(&route, &[0xB0, 1, 0]).unwrap();
        assert_eq!(try_recv(&mut rx).unwrap().1, 0.2);
        handle_message(&route, &[0xB0, 1, 127]).unwrap();
        assert!((try_recv(&mut rx).unwrap().1 - 0.6).abs() < 1e-6);
    }

    #[test]
    fn channel_filter_applies_unless_omni() {
        let (route, mut rx) = route_with(vec![map(7, 2, "amp/gain", 0.0, 1.0)], 1);
        assert!(handle_message(&route, &[0xB0, 7, 64])
            .unwrap()
            .mapped
            .is_none());
        assert!(try_recv(&mut rx).is_none());
        assert!(handle_message(&route, &[0xB2, 7, 64])
            .unwrap()
            .mapped
            .is_some());
        route.config.lock().unwrap().channel = 0; // omni
        assert!(handle_message(&route, &[0xB5, 7, 64])
            .unwrap()
            .mapped
            .is_some());
    }

    #[test]
    fn learn_mode_records_cc_and_does_not_route() {
        let (route, mut rx) = route_with(vec![map(7, 0, "amp/gain", 0.0, 1.0)], 0);
        route.config.lock().unwrap().learn_mode = true;
        let ev = handle_message(&route, &[0xB0, 7, 10]).unwrap();
        assert!(ev.learned);
        assert_eq!(route.config.lock().unwrap().last_cc, Some(7));
        assert!(try_recv(&mut rx).is_none());
    }

    #[test]
    fn non_cc_and_short_messages_are_ignored() {
        let (route, _rx) = route_with(vec![], 0);
        assert!(handle_message(&route, &[0x90, 60, 100]).is_none()); // note on
        assert!(handle_message(&route, &[0xB0, 7]).is_none());
    }

    #[test]
    fn no_engine_is_fine() {
        let (route, _rx) = route_with(vec![map(7, 0, "amp/gain", 0.0, 1.0)], 0);
        *route.param_tx.lock().unwrap() = None;
        assert!(handle_message(&route, &[0xB0, 7, 64])
            .unwrap()
            .mapped
            .is_some());
    }
}
