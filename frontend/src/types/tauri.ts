/** Matches backend EngineStatus struct */
export interface EngineStatus {
  running: boolean;
  sample_rate: number;
  buffer_size: number;
}

/** A slot in the signal chain */
export interface ChainSlot {
  id: string;
  plugin_type: string;
  enabled: boolean;
  wet_dry: number;
  parameters: Record<string, number>;
}

/** Full signal chain snapshot */
export interface ChainSnapshot {
  slots: ChainSlot[];
}

/** A preset descriptor for list views */
export interface PresetDescriptor {
  name: string;
  description: string;
  tags: string[];
  created: string;
  modified: string;
}

/** A bank of presets */
export interface BankDescriptor {
  name: string;
  presets: PresetDescriptor[];
}

/** An impulse response file discovered or picked */
export interface IrFileInfo {
  path: string;
  name: string;
  sample_rate: number;
  sample_count: number;
  duration_ms: number;
  channels: number;
}

/** Settings payload */
export interface SettingsPayload {
  engine_mode: string;
  jack_client_name: string;
  sample_rate: number;
  buffer_size: number;
  audio_device: string;
  /** "Cpal" or "Jack" */
  audio_backend: string;
  input_device: string;
  output_device: string;
  /** 0-based input channel that carries the guitar */
  input_channel: number;
  ir_directories: string[];
  nam_directories: string[];
  preset_directories: string[];
  ai_provider: string;
  ai_endpoint_url: string;
  ai_api_key: string;
  ai_model: string;
}

/** Live audio I/O health reported by the backend */
export interface AudioStatus {
  running: boolean;
  backend: string;
  latency_ms: number;
  underruns: number;
  overruns: number;
  callback_frames: number;
  input_ok: boolean;
  input_error: string | null;
}

/** Live WAV recording state (DI on channel 1, processed on channel 2). */
export interface RecordingStatus {
  recording: boolean;
  error: string | null;
  path: string | null;
  seconds: number;
  dropped_frames: number;
}

/** Result of a finished recording. */
export interface RecordingSummary {
  path: string;
  frames: number;
  dropped_frames: number;
  seconds: number;
}

/** An audio device discovered via CPAL */
export interface AudioDeviceInfo {
  name: string;
  is_input: boolean;
  is_output: boolean;
}

/** A MIDI device discovered on the system */
export interface MidiDeviceInfo {
  name: string;
  connected: boolean;
}

/** A single CC-to-parameter mapping */
export interface MidiMappingPayload {
  cc: number;
  channel: number;
  parameter_id: string;
  label: string;
  min: number;
  max: number;
}

/** Full MIDI config sent to/from backend */
export interface MidiConfigPayload {
  active_device: string | null;
  channel: number;
  mappings: MidiMappingPayload[];
  learn_mode: boolean;
  last_cc: number | null;
}

/** An event report from polling MIDI — used for learn mode UI */
export interface MidiEventReport {
  cc: number;
  channel: number;
  raw_value: number;
  normalized: number;
  mapped_parameter: string | null;
  mapped_value: number | null;
}

/** A single slot in an AI-generated signal chain */
export interface AiSlotInfo {
  plugin_type: string;
  enabled: boolean;
  wet_dry: number;
  parameters: Record<string, number>;
}

/** Signal chain info from an AI preset */
export interface AiSignalChainInfo {
  slots: AiSlotInfo[];
}

/** The result from the AI preset generator */
export interface AiPresetResult {
  name: string;
  description: string;
  signal_chain: AiSignalChainInfo;
}

/** A scene descriptor for list views */
export interface SceneInfo {
  index: number;
  name: string;
  slot_count: number;
  is_active: boolean;
}

/** Full scene detail including signal chain */
export interface SceneDetail {
  index: number;
  name: string;
  signal_chain: ChainSnapshot;
}

/** Result of loading a NAM model into the Nam plugin */
export interface NamModelInfo {
  path: string;
  file_name: string;
  architecture: string;
  sample_rate: number;
  num_parameters: number;
}

/** Result of loading an IR file into the Cab plugin */
export interface IrLoadResult {
  path: string;
  file_name: string;
  sample_rate: number;
  length_samples: number;
  length_ms: number;
}

/** A built-in amp preset — parameter values that can be applied to the Amp slot. */
export interface AmpPresetInfo {
  name: string;
  description: string;
  tags: string[];
  gain: number;
  master: number;
  bass: number;
  mid: number;
  treble: number;
  drive: number;
}
