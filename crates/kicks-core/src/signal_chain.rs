use serde::{Deserialize, Serialize};

/// The complete signal chain: an ordered list of processing slots.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignalChain {
    pub slots: Vec<ChainSlot>,
}

/// A single slot in the signal chain.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainSlot {
    pub id: String,
    pub plugin_type: PluginType,
    pub enabled: bool,
    pub wet_dry: f32,
    pub parameters: std::collections::HashMap<String, f32>,
}

/// Types of plugins available in the signal chain.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PluginType {
    Input,
    Boost,
    Amp,
    /// Bass-specific amp with compressor, extended low-end, and shifted EQ.
    BassAmp,
    Cab,
    /// Neural Amp Modeler — deep learning amp/cabinet model.
    Nam,
    Delay,
    Reverb,
    /// Real-time chromatic tuner using YIN pitch detection.
    Tuner,
    /// Practice metronome with configurable BPM and time signature.
    Metronome,
    /// Audio looper with record, overdub, playback, and reverse.
    Looper,
    Output,
    Custom(String),
}

impl Default for SignalChain {
    fn default() -> Self {
        Self {
            slots: vec![
                ChainSlot {
                    id: "input".to_string(),
                    plugin_type: PluginType::Input,
                    enabled: true,
                    wet_dry: 1.0,
                    parameters: std::collections::HashMap::new(),
                },
                ChainSlot {
                    id: "tuner".to_string(),
                    plugin_type: PluginType::Tuner,
                    enabled: true,
                    wet_dry: 1.0,
                    parameters: [("mute".into(), 0.0)].into_iter().collect(),
                },
                ChainSlot {
                    id: "boost".to_string(),
                    plugin_type: PluginType::Boost,
                    enabled: true,
                    wet_dry: 1.0,
                    parameters: [("gain".into(), 0.75)].into_iter().collect(),
                },
                ChainSlot {
                    id: "amp".to_string(),
                    plugin_type: PluginType::Amp,
                    enabled: true,
                    wet_dry: 1.0,
                    parameters: [
                        ("gain".into(), 0.5),
                        ("master".into(), 0.7),
                        ("bass".into(), 0.5),
                        ("mid".into(), 0.5),
                        ("treble".into(), 0.5),
                        ("drive".into(), 0.5),
                    ]
                    .into_iter()
                    .collect(),
                },
                ChainSlot {
                    id: "cab".to_string(),
                    plugin_type: PluginType::Cab,
                    enabled: true,
                    wet_dry: 1.0,
                    parameters: [
                        ("level".into(), 1.0),
                        ("low_cut".into(), 0.0),
                        ("high_cut".into(), 0.6),
                    ]
                    .into_iter()
                    .collect(),
                },
                ChainSlot {
                    id: "delay".to_string(),
                    plugin_type: PluginType::Delay,
                    enabled: false,
                    // Level comes from the plugin's own `mix`; the slot
                    // wet/dry blend is fully wet so the two don't stack.
                    wet_dry: 1.0,
                    parameters: [
                        ("time".into(), 0.3),
                        ("feedback".into(), 0.4),
                        ("mix".into(), 0.3),
                    ]
                    .into_iter()
                    .collect(),
                },
                ChainSlot {
                    id: "reverb".to_string(),
                    plugin_type: PluginType::Reverb,
                    enabled: true,
                    // Level comes from the plugin's own `mix`; the slot
                    // wet/dry blend is fully wet so the two don't stack.
                    wet_dry: 1.0,
                    parameters: [
                        ("size".into(), 0.5),
                        ("damping".into(), 0.5),
                        ("mix".into(), 0.3),
                    ]
                    .into_iter()
                    .collect(),
                },
                ChainSlot {
                    id: "output".to_string(),
                    plugin_type: PluginType::Output,
                    enabled: true,
                    wet_dry: 1.0,
                    parameters: [("volume".into(), 0.8)].into_iter().collect(),
                },
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_delay_and_reverb_are_not_double_diluted() {
        let chain = SignalChain::default();
        for id in ["delay", "reverb"] {
            let slot = chain.slots.iter().find(|s| s.id == id).unwrap();
            let mix = slot.parameters["mix"];
            // Effective wet level = slot wet/dry * plugin mix.
            assert!(
                (slot.wet_dry * mix - 0.3).abs() < 1e-6,
                "{id}: {} * {mix}",
                slot.wet_dry
            );
        }
    }
}
