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
    /// Noise gate with hysteresis.
    NoiseGate,
    /// Feed-forward compressor.
    Compressor,
    /// Overdrive pedal.
    Overdrive,
    /// 3-band parametric EQ (low shelf, sweepable mid, high shelf).
    Eq,
    Chorus,
    Phaser,
    Flanger,
    Tremolo,
    Output,
    Custom(String),
}

impl SignalChain {
    /// Add any default slots missing from a chain saved by an older version
    /// (e.g. pedals added later). Missing slots are inserted bypassed, just
    /// before the Output slot, so the saved sound is unchanged. Returns the
    /// ids that were added.
    pub fn add_missing_default_slots(&mut self) -> Vec<String> {
        let mut added = Vec::new();
        for slot in SignalChain::default().slots {
            if self.slots.iter().any(|s| s.id == slot.id) {
                continue;
            }
            let mut slot = slot;
            if !matches!(slot.plugin_type, PluginType::Input | PluginType::Output) {
                slot.enabled = false;
            }
            let at = self
                .slots
                .iter()
                .position(|s| s.plugin_type == PluginType::Output)
                .unwrap_or(self.slots.len());
            added.push(slot.id.clone());
            self.slots.insert(at, slot);
        }
        added
    }
}

/// An optional pedal slot that starts bypassed.
fn pedal(id: &str, plugin_type: PluginType, params: &[(&str, f32)]) -> ChainSlot {
    ChainSlot {
        id: id.to_string(),
        plugin_type,
        enabled: false,
        wet_dry: 1.0,
        parameters: params.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
    }
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
                pedal(
                    "gate",
                    PluginType::NoiseGate,
                    &[("threshold", 0.3), ("release", 0.3)],
                ),
                pedal(
                    "comp",
                    PluginType::Compressor,
                    &[
                        ("threshold", 0.6),
                        ("ratio", 0.3),
                        ("attack", 0.3),
                        ("release", 0.4),
                        ("makeup", 0.2),
                    ],
                ),
                pedal(
                    "drive",
                    PluginType::Overdrive,
                    &[("drive", 0.4), ("tone", 0.5), ("level", 0.5)],
                ),
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
                pedal(
                    "eq",
                    PluginType::Eq,
                    &[
                        ("low", 0.5),
                        ("mid", 0.5),
                        ("mid_freq", 0.5),
                        ("q", 0.3),
                        ("high", 0.5),
                    ],
                ),
                pedal(
                    "chorus",
                    PluginType::Chorus,
                    &[("rate", 0.3), ("depth", 0.5), ("mix", 0.5)],
                ),
                pedal(
                    "phaser",
                    PluginType::Phaser,
                    &[
                        ("rate", 0.3),
                        ("depth", 0.7),
                        ("feedback", 0.4),
                        ("mix", 0.5),
                    ],
                ),
                pedal(
                    "flanger",
                    PluginType::Flanger,
                    &[
                        ("rate", 0.3),
                        ("depth", 0.5),
                        ("feedback", 0.5),
                        ("mix", 0.5),
                    ],
                ),
                pedal(
                    "tremolo",
                    PluginType::Tremolo,
                    &[("rate", 0.4), ("depth", 0.5)],
                ),
                ChainSlot {
                    id: "delay".to_string(),
                    plugin_type: PluginType::Delay,
                    enabled: false,
                    wet_dry: 0.3,
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
                    wet_dry: 0.3,
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
mod migration_tests {
    use super::*;

    #[test]
    fn old_chain_gains_new_pedals_bypassed_before_output() {
        let mut chain = SignalChain::default();
        chain
            .slots
            .retain(|s| !["gate", "eq", "tremolo"].contains(&s.id.as_str()));
        let before: Vec<String> = chain.slots.iter().map(|s| s.id.clone()).collect();
        let added = chain.add_missing_default_slots();
        assert_eq!(added, ["gate", "eq", "tremolo"]);
        let ids: Vec<&str> = chain.slots.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids.last(), Some(&"output"));
        assert_eq!(
            &ids[ids.len() - 4..ids.len() - 1],
            ["gate", "eq", "tremolo"]
        );
        for id in ["gate", "eq", "tremolo"] {
            assert!(!chain.slots.iter().find(|s| s.id == id).unwrap().enabled);
        }
        // Existing order untouched.
        let kept: Vec<String> = chain
            .slots
            .iter()
            .map(|s| s.id.clone())
            .filter(|id| before.contains(id))
            .collect();
        assert_eq!(kept, before);
        assert!(chain.add_missing_default_slots().is_empty());
    }

    #[test]
    fn new_pedals_default_bypassed() {
        let chain = SignalChain::default();
        for id in [
            "gate", "comp", "drive", "eq", "chorus", "phaser", "flanger", "tremolo",
        ] {
            let s = chain.slots.iter().find(|s| s.id == id).expect(id);
            assert!(!s.enabled, "{id}");
            assert_eq!(s.wet_dry, 1.0);
        }
    }
}
