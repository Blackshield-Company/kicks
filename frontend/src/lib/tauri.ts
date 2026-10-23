/**
 * Resolve the Tauri invoke function.
 *
 * Tauri v2 with `withGlobalTauri` exposes `window.__TAURI__.core.invoke`;
 * v1 exposed `window.__TAURI__.invoke`. Support both.
 */
function resolveInvoke(): ((cmd: string, args?: Record<string, unknown>) => Promise<unknown>) | undefined {
  const w = window as unknown as {
    __TAURI__?: {
      invoke?: (cmd: string, args?: Record<string, unknown>) => Promise<unknown>;
      core?: { invoke?: (cmd: string, args?: Record<string, unknown>) => Promise<unknown> };
    };
  };
  return w.__TAURI__?.invoke ?? w.__TAURI__?.core?.invoke;
}

/**
 * Safely invoke a Tauri command, falling back gracefully when running in a browser.
 */
/**
 * Tauri 2 matches command arguments by their camelCase name (`slot_id` in
 * Rust is `slotId` over IPC). This file writes snake_case keys, which the
 * real backend rejected with "missing required key". Convert top-level keys
 * only: nested values are serde structs that keep their own field names.
 */
export function toCommandArgs(args: Record<string, unknown> = {}): Record<string, unknown> {
  const out: Record<string, unknown> = {};
  for (const [k, v] of Object.entries(args)) {
    out[k.replace(/_([a-z0-9])/g, (_, c: string) => c.toUpperCase())] = v;
  }
  return out;
}

async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const tauriInvoke = resolveInvoke();

  if (!tauriInvoke) {
    console.warn(`[tauri] Running outside Tauri — simulating '${cmd}'`);
    return simulate<T>(cmd, args);
  }

  return tauriInvoke(cmd, toCommandArgs(args)) as Promise<T>;
}

// ── Simulated responses for dev mode (outside Tauri) ──

function simulate<T>(cmd: string, _args?: Record<string, unknown>): T {
  switch (cmd) {
    case 'list_audio_devices':
      return [
        { name: 'Built-in Audio Analog Stereo', is_input: true, is_output: true },
        { name: 'USB Audio Interface', is_input: true, is_output: true },
        { name: 'HDMI Audio Output', is_input: false, is_output: true },
        { name: 'PipeWire Stream', is_input: true, is_output: true },
      ] as T;
    case 'get_version':
      return '0.1.0 (dev)' as T;
    case 'engine_status':
      return { running: true, sample_rate: 48000, buffer_size: 256 } as T;
    case 'get_audio_levels': {
      // Simulate per-plugin RMS levels with slight frame-to-frame jitter
      const t = Date.now() / 500;
      const jitter = (base: number, amp: number) =>
        Math.max(0, Math.min(1, base + amp * Math.sin(t + base * 10)));
      return [
        1.0,                   // Input (pass-through)
        jitter(0.65, 0.08),    // Boost
        jitter(0.45, 0.12),    // Amp
        jitter(0.50, 0.06),    // Cab
        0.0,                   // BassAmp (disabled)
        jitter(0.12, 0.04),    // Delay
        jitter(0.22, 0.03),    // Reverb
        jitter(0.48, 0.10),    // Output
      ] as T;
    }
    case 'get_signal_chain':
      return {
        slots: [
          { id: 'input', plugin_type: 'Input', enabled: true, wet_dry: 1.0, parameters: {} },
          { id: 'tuner', plugin_type: 'Tuner', enabled: true, wet_dry: 1.0, parameters: { mute: 0.0 } },
          { id: 'boost', plugin_type: 'Boost', enabled: true, wet_dry: 1.0, parameters: { gain: 0.75 } },
          { id: 'amp', plugin_type: 'Amp', enabled: true, wet_dry: 1.0, parameters: { gain: 0.5, master: 0.7, bass: 0.5, mid: 0.5, treble: 0.5, drive: 0.5 } },
          { id: 'cab', plugin_type: 'Cab', enabled: true, wet_dry: 1.0, parameters: { level: 1.0, low_cut: 0.0, high_cut: 0.6 } },
          { id: 'bass-amp', plugin_type: 'BassAmp', enabled: false, wet_dry: 1.0, parameters: { gain: 0.25, master: 0.78, bass: 0.65, mid: 0.55, treble: 0.40, drive: 0.12 } },
          { id: 'delay', plugin_type: 'Delay', enabled: false, wet_dry: 1.0, parameters: { time: 0.3, feedback: 0.4, mix: 0.3 } },
          { id: 'reverb', plugin_type: 'Reverb', enabled: true, wet_dry: 1.0, parameters: { size: 0.5, damping: 0.5, mix: 0.3 } },
          { id: 'output', plugin_type: 'Output', enabled: true, wet_dry: 1.0, parameters: { volume: 0.8 } },
        ],
      } as T;
    case 'list_presets':
      return [] as T;
    case 'recording_status':
      return { recording: false, error: null, path: null, seconds: 0, dropped_frames: 0 } as T;
    case 'start_recording':
      throw new Error('Recording needs the desktop app');
    case 'get_audio_status':
      return {
        running: false,
        backend: '',
        latency_ms: 0,
        underruns: 0,
        overruns: 0,
        callback_frames: 0,
        input_ok: false,
        input_error: null,
      } as T;
    case 'get_settings':
      return {
        engine_mode: 'Auto',
        jack_client_name: 'kicks',
        sample_rate: 48000,
        buffer_size: 256,
        audio_device: '',
        audio_backend: 'Cpal',
        input_device: '',
        output_device: '',
        input_channel: 0,
        ir_directories: [],
        nam_directories: [],
        preset_directories: [],
        ai_provider: 'Anthropic',
        ai_endpoint_url: 'https://api.anthropic.com/v1/messages',
        ai_api_key: '',
        ai_model: 'claude-sonnet-4-20250514',
      } as T;
    case 'load_ir_to_cab':
      return {
        path: _args?.path ?? '',
        file_name: (_args?.path as string ?? '').split('/').pop() ?? 'unknown.wav',
        sample_rate: 48000,
        length_samples: 1024,
        length_ms: 21,
      } as T;
    case 'get_cab_ir_info':
      return null as T;
    case 'clear_cab_ir':
      return undefined as T;
    case 'load_nam_model':
      return {
        path: _args?.path ?? '',
        file_name: (_args?.path as string ?? '').split('/').pop() ?? 'unknown.nam',
        architecture: 'WaveNet',
        sample_rate: 48000,
        num_parameters: 42000,
      } as T;
    case 'get_nam_info':
      return null as T;
    case 'clear_nam_model':
      return undefined as T;
    case 'list_nam_files':
      return [] as T;
    case 'scan_nam_directory':
      return [] as T;
    case 'list_scenes':
      return [
        { index: 0, name: 'Clean', slot_count: 7, is_active: true },
        { index: 1, name: 'Crunch', slot_count: 7, is_active: false },
        { index: 2, name: 'Lead', slot_count: 7, is_active: false },
      ] as T;
    case 'get_scene':
      return {
        index: (_args?.index as number) ?? 0,
        name: 'Scene',
        signal_chain: {
          slots: [
            { id: 'input', plugin_type: 'Input', enabled: true, wet_dry: 1.0, parameters: {} },
            { id: 'boost', plugin_type: 'Boost', enabled: true, wet_dry: 1.0, parameters: { gain: 0.75 } },
            { id: 'amp', plugin_type: 'Amp', enabled: true, wet_dry: 1.0, parameters: { gain: 0.5, master: 0.7, bass: 0.5, mid: 0.5, treble: 0.5, drive: 0.5 } },
            { id: 'cab', plugin_type: 'Cab', enabled: true, wet_dry: 1.0, parameters: { level: 1.0, low_cut: 0.0, high_cut: 0.6 } },
            { id: 'delay', plugin_type: 'Delay', enabled: false, wet_dry: 1.0, parameters: { time: 0.3, feedback: 0.4, mix: 0.3 } },
            { id: 'reverb', plugin_type: 'Reverb', enabled: true, wet_dry: 1.0, parameters: { size: 0.5, damping: 0.5, mix: 0.3 } },
            { id: 'output', plugin_type: 'Output', enabled: true, wet_dry: 1.0, parameters: { volume: 0.8 } },
          ],
        },
      } as T;
    case 'save_scene':
      return { index: 3, name: _args?.name ?? 'New Scene', slot_count: 7, is_active: false } as T;
    case 'update_scene':
    case 'load_scene':
    case 'delete_scene':
    case 'rename_scene':
    case 'reorder_scene':
      return undefined as T;
    case 'next_scene':
    case 'prev_scene':
      return { index: 0, name: 'Clean', slot_count: 7, is_active: true } as T;
    // ── Tuner ──
    case 'get_tuner_info':
      return { frequency: 440.0, note: 'A4', cents: 0.2, confidence: 0.95, active: true } as T;
    // ── Metronome ──
    case 'get_metronome_state':
      return { bpm: 120.0, beats_per_bar: 4, running: false } as T;
    // ── Looper ──
    case 'get_looper_state':
      return { mode: 'idle', loop_time_seconds: 0.0, has_loop: false } as T;
    case 'trigger_looper_mode':
    case 'looper_undo':
    case 'looper_clear':
      return true as T;
    // ── Bass Amp / Chain modes ──
    case 'switch_to_bass_chain':
    case 'switch_to_practice_chain':
    case 'switch_to_looper_chain':
      return undefined as T;
    case 'list_amp_presets':
      return [
        { name: 'American Clean', description: 'Crisp, sparkling Fender-style clean tone. Bright highs and punchy lows.', tags: ['clean', 'fender', 'american'], gain: 0.15, master: 0.80, bass: 0.60, mid: 0.50, treble: 0.60, drive: 0.10 },
        { name: 'Jazz Clean', description: 'Warm, round, and mellow. Rolled-off treble for smooth jazz.', tags: ['clean', 'jazz', 'warm'], gain: 0.20, master: 0.70, bass: 0.70, mid: 0.70, treble: 0.30, drive: 0.05 },
        { name: 'Twin Clean', description: 'Fender Twin Reverb style. Big headroom, glassy highs.', tags: ['clean', 'fender', 'twin'], gain: 0.10, master: 0.85, bass: 0.50, mid: 0.35, treble: 0.55, drive: 0.05 },
        { name: 'Boutique Clean', description: 'Dumble-style warm, complex midrange. Responds to dynamics.', tags: ['clean', 'boutique', 'dumble'], gain: 0.25, master: 0.75, bass: 0.55, mid: 0.65, treble: 0.50, drive: 0.15 },
        { name: 'AC30 Chime', description: 'Vox AC30 top-boost chime. Jangly highs that cut through.', tags: ['clean', 'vox', 'chime'], gain: 0.20, master: 0.70, bass: 0.40, mid: 0.60, treble: 0.70, drive: 0.10 },
        { name: 'Deluxe Clean', description: 'Fender Deluxe Reverb style. Sweet, compressed, warm.', tags: ['clean', 'fender', 'deluxe'], gain: 0.22, master: 0.65, bass: 0.50, mid: 0.55, treble: 0.50, drive: 0.08 },
        { name: 'British Crunch', description: 'Marshall Plexi pushed into natural breakup. Classic rock crunch.', tags: ['crunch', 'marshall', 'plexi'], gain: 0.55, master: 0.60, bass: 0.60, mid: 0.50, treble: 0.55, drive: 0.50 },
        { name: 'Blues Breakup', description: 'Edge-of-breakup sweetness. Touch-sensitive blues tone.', tags: ['crunch', 'blues', 'edge-of-breakup'], gain: 0.40, master: 0.65, bass: 0.55, mid: 0.60, treble: 0.50, drive: 0.35 },
        { name: 'Texas Blues', description: 'SRV-inspired hot blues. Big low-end, punchy mids.', tags: ['crunch', 'blues', 'texas'], gain: 0.50, master: 0.60, bass: 0.50, mid: 0.70, treble: 0.60, drive: 0.45 },
        { name: '800 Crunch', description: 'JCM800 rhythm crunch. Tight, aggressive, percussive.', tags: ['crunch', 'marshall', 'jcm800'], gain: 0.50, master: 0.55, bass: 0.55, mid: 0.45, treble: 0.60, drive: 0.55 },
        { name: 'AC30 Crunch', description: 'Vox AC30 pushed into breakup. Snarling harmonic-rich crunch.', tags: ['crunch', 'vox', 'ac30'], gain: 0.50, master: 0.55, bass: 0.40, mid: 0.55, treble: 0.65, drive: 0.50 },
        { name: 'Plexi Drive', description: 'Marshall Super Lead cranked. Raw, punchy, alive.', tags: ['crunch', 'marshall', 'plexi'], gain: 0.60, master: 0.50, bass: 0.55, mid: 0.50, treble: 0.60, drive: 0.60 },
        { name: 'Hot Rod Lead', description: 'Hot-rodded Marshall lead. Singing sustain for days.', tags: ['lead', 'hot-rodded', 'marshall'], gain: 0.70, master: 0.50, bass: 0.50, mid: 0.40, treble: 0.60, drive: 0.70 },
        { name: '800 Lead', description: 'JCM800 boosted for lead. Tight low-end, cutting mids.', tags: ['lead', 'marshall', 'jcm800'], gain: 0.65, master: 0.50, bass: 0.50, mid: 0.35, treble: 0.65, drive: 0.65 },
        { name: 'Brown Sound', description: 'Eddie Van Halen\'s iconic sound. Mid-scoop, percussive attack.', tags: ['lead', 'van-halen', 'brown'], gain: 0.60, master: 0.55, bass: 0.60, mid: 0.30, treble: 0.70, drive: 0.60 },
        { name: 'Solo Boost', description: 'Boosted lead with extra presence to cut through any mix.', tags: ['lead', 'solo', 'boosted'], gain: 0.75, master: 0.70, bass: 0.50, mid: 0.60, treble: 0.55, drive: 0.75 },
        { name: 'Liquid Lead', description: 'Saturated, compressed lead. Violin-like sustain.', tags: ['lead', 'liquid', 'smooth'], gain: 0.80, master: 0.50, bass: 0.55, mid: 0.50, treble: 0.55, drive: 0.80 },
        { name: 'Modern Lead', description: 'Modern high-gain lead. Fluid runs, singing harmonics.', tags: ['lead', 'modern', 'high-gain'], gain: 0.75, master: 0.50, bass: 0.60, mid: 0.30, treble: 0.60, drive: 0.75 },
        { name: 'Modern Metal', description: '5150-inspired high-gain. Tight, aggressive, brutal.', tags: ['high-gain', 'metal', '5150'], gain: 0.85, master: 0.40, bass: 0.70, mid: 0.20, treble: 0.60, drive: 0.85 },
        { name: 'Rectifier', description: 'Mesa Dual Rectifier saturation. Wall of gain, massive low-end.', tags: ['high-gain', 'mesa', 'rectifier'], gain: 0.80, master: 0.45, bass: 0.65, mid: 0.15, treble: 0.55, drive: 0.80 },
        { name: 'Death Metal', description: 'Extreme gain. Maximum saturation, bone-crushing low-end.', tags: ['high-gain', 'death-metal', 'extreme'], gain: 0.95, master: 0.35, bass: 0.75, mid: 0.10, treble: 0.50, drive: 0.95 },
        { name: 'Nu-Metal', description: 'Deeply scooped mids. Boosted lows and highs for that 2000s sound.', tags: ['high-gain', 'nu-metal', 'scooped'], gain: 0.70, master: 0.50, bass: 0.80, mid: 0.05, treble: 0.70, drive: 0.70 },
        { name: 'Djent', description: 'Tight, percussive low-end. Engineered for polyrhythms.', tags: ['high-gain', 'djent', 'tight'], gain: 0.75, master: 0.45, bass: 0.65, mid: 0.25, treble: 0.65, drive: 0.80 },
        { name: 'Black Metal', description: 'Cold, trebly, raw. Minimal bass, frost-bitten Scandinavian tone.', tags: ['high-gain', 'black-metal', 'trebly'], gain: 0.70, master: 0.40, bass: 0.20, mid: 0.30, treble: 0.85, drive: 0.75 },
        { name: 'Boutique OD', description: 'Dumble-style overdrive. Smooth, touch-sensitive, complex.', tags: ['specialty', 'boutique', 'overdrive'], gain: 0.55, master: 0.60, bass: 0.55, mid: 0.75, treble: 0.50, drive: 0.50 },
        { name: 'Tweed', description: 'Fender Tweed style. Fat, warm, compressed — early rock and roll.', tags: ['specialty', 'fender', 'tweed'], gain: 0.45, master: 0.60, bass: 0.50, mid: 0.60, treble: 0.40, drive: 0.40 },
        { name: 'Voxy', description: 'Vox chime pushed into overdrive. Jingle-jangle with attitude.', tags: ['specialty', 'vox', 'chime'], gain: 0.50, master: 0.55, bass: 0.35, mid: 0.55, treble: 0.75, drive: 0.50 },
        { name: 'American High-Gain', description: 'Mesa/Boogie Mark series. Tight, focused, signature mid-voice.', tags: ['specialty', 'mesa', 'high-gain'], gain: 0.80, master: 0.40, bass: 0.60, mid: 0.25, treble: 0.55, drive: 0.80 },
        { name: 'British Metal', description: 'JCM900-driven metal. Thick British distortion, modern edge.', tags: ['specialty', 'marshall', 'british'], gain: 0.75, master: 0.45, bass: 0.60, mid: 0.30, treble: 0.60, drive: 0.75 },
        { name: 'Doom', description: 'Thick, dark, sludgy. Massive low-end, rolled-off highs.', tags: ['specialty', 'doom', 'sludge'], gain: 0.70, master: 0.65, bass: 0.90, mid: 0.40, treble: 0.15, drive: 0.65 },
        { name: 'Punk', description: 'Angry mid-forward distortion. Raw, aggressive, cutting.', tags: ['specialty', 'punk', 'aggressive'], gain: 0.60, master: 0.55, bass: 0.50, mid: 0.65, treble: 0.55, drive: 0.60 },
        { name: 'Grunge', description: 'Raw, unpolished, aggressive. A broken-in amp pushed past its limits.', tags: ['specialty', 'grunge', 'raw'], gain: 0.60, master: 0.50, bass: 0.55, mid: 0.50, treble: 0.45, drive: 0.65 },
      ] as T;
    case 'get_cpu_load':
      return 0.0 as T;
    default:
      return undefined as T;
  }
}

// ── Engine ──

export async function startEngine(): Promise<void> {
  return invoke('start_engine');
}

export async function stopEngine(): Promise<void> {
  return invoke('stop_engine');
}

export async function engineStatus() {
  return invoke<import('../types/tauri').EngineStatus>('engine_status');
}

export async function setParameter(id: string, value: number): Promise<void> {
  return invoke('set_parameter', { id, value });
}

export async function getParameter(id: string): Promise<number | null> {
  return invoke<number | null>('get_parameter', { id });
}

// ── Signal Chain ──

export async function getSignalChain() {
  return invoke<import('../types/tauri').ChainSnapshot>('get_signal_chain');
}

export async function buildDefaultChain(): Promise<void> {
  return invoke('build_default_chain');
}

export async function updateSlot(
  slotId: string,
  changes: { enabled?: boolean; wet_dry?: number; parameters?: Record<string, number> },
): Promise<void> {
  return invoke('update_slot', { slot_id: slotId, ...changes });
}

export async function toggleSlot(slotId: string): Promise<boolean> {
  return invoke<boolean>('toggle_slot', { slot_id: slotId });
}

export async function moveSlot(fromIdx: number, toIdx: number): Promise<void> {
  return invoke('move_slot', { from_idx: fromIdx, to_idx: toIdx });
}

export async function undoSignalChain(): Promise<void> {
  return invoke('undo_signal_chain');
}

export async function redoSignalChain(): Promise<void> {
  return invoke('redo_signal_chain');
}

// ── Presets ──

export async function listPresets() {
  return invoke<import('../types/tauri').BankDescriptor[]>('list_presets');
}

export async function savePreset(
  bankName: string,
  presetName: string,
  description?: string,
  tags?: string[],
): Promise<void> {
  return invoke('save_preset', { bank_name: bankName, preset_name: presetName, description, tags });
}

export async function loadPreset(bankName: string, presetName: string): Promise<void> {
  return invoke('load_preset', { bank_name: bankName, preset_name: presetName });
}

export async function deletePreset(bankName: string, presetName: string): Promise<void> {
  return invoke('delete_preset', { bank_name: bankName, preset_name: presetName });
}

export async function renamePreset(bankName: string, oldName: string, newName: string): Promise<void> {
  return invoke('rename_preset', { bank_name: bankName, old_name: oldName, new_name: newName });
}

// ── Settings ──

export async function getSettings() {
  return invoke<import('../types/tauri').SettingsPayload>('get_settings');
}

export async function saveSettings(settings: import('../types/tauri').SettingsPayload): Promise<void> {
  return invoke('save_settings', { settings });
}

export async function listAudioDevices() {
  return invoke<import('../types/tauri').AudioDeviceInfo[]>('list_audio_devices');
}

export async function getVersion(): Promise<string> {
  return invoke<string>('get_version');
}

// ── IR Browser ──

export async function listIrFiles() {
  return invoke<import('../types/tauri').IrFileInfo[]>('list_ir_files');
}

export async function pickIrFile() {
  return invoke<import('../types/tauri').IrFileInfo | null>('pick_ir_file');
}

export async function scanIrDirectory(dirPath: string) {
  return invoke<import('../types/tauri').IrFileInfo[]>('scan_ir_directory', { dir_path: dirPath });
}

export async function loadIrToCab(path: string) {
  return invoke<import('../types/tauri').IrLoadResult>('load_ir_to_cab', { path });
}

export async function getCabIrInfo() {
  return invoke<import('../types/tauri').IrLoadResult | null>('get_cab_ir_info');
}

export async function clearCabIr(): Promise<void> {
  return invoke('clear_cab_ir');
}

// ── MIDI ──

export async function listMidiDevices() {
  return invoke<import('../types/tauri').MidiDeviceInfo[]>('list_midi_devices');
}

export async function getMidiConfig() {
  return invoke<import('../types/tauri').MidiConfigPayload>('get_midi_config');
}

export async function saveMidiConfig(config: import('../types/tauri').MidiConfigPayload): Promise<void> {
  return invoke('save_midi_config', { config });
}

export async function connectMidiDevice(deviceName: string): Promise<void> {
  return invoke('connect_midi_device', { device_name: deviceName });
}

export async function disconnectMidiDevice(): Promise<void> {
  return invoke('disconnect_midi_device');
}

export async function pollMidiEvents() {
  return invoke<import('../types/tauri').MidiEventReport[]>('poll_midi_events');
}

export async function setMidiLearn(active: boolean): Promise<void> {
  return invoke('set_midi_learn', { active });
}

// ── AI Tone Assistant ──

export async function generateAiPreset(description: string) {
  return invoke<import('../types/tauri').AiPresetResult>('generate_ai_preset', { description });
}

export async function applyAiPreset(signalChain: import('../types/tauri').AiSignalChainInfo): Promise<void> {
  return invoke('apply_ai_preset', { signal_chain: signalChain });
}

// ── NAM Model ──

export async function listNamFiles() {
  return invoke<{ path: string; name: string }[]>('list_nam_files');
}

export async function scanNamDirectory(dirPath: string) {
  return invoke<{ path: string; name: string }[]>('scan_nam_directory', { dir_path: dirPath });
}

export async function loadNamModel(path: string) {
  return invoke<import('../types/tauri').NamModelInfo>('load_nam_model', { path });
}

export async function getNamInfo() {
  return invoke<import('../types/tauri').NamModelInfo | null>('get_nam_info');
}

export async function clearNamModel(): Promise<void> {
  return invoke('clear_nam_model');
}

// ── Audio Levels ──

export async function getAudioLevels(): Promise<number[]> {
  return invoke<number[]>('get_audio_levels');
}

export async function startRecording() {
  return invoke<string>('start_recording');
}

export async function stopRecording() {
  return invoke<import('../types/tauri').RecordingSummary>('stop_recording');
}

export async function getRecordingStatus() {
  return invoke<import('../types/tauri').RecordingStatus>('recording_status');
}

export async function getAudioStatus() {
  return invoke<import('../types/tauri').AudioStatus>('get_audio_status');
}

export async function getCpuLoad(): Promise<number> {
  return invoke<number>('get_cpu_load');
}

// ── Tuner ──

export interface TunerInfo {
  frequency: number;
  note: string;
  cents: number;
  confidence: number;
  active: boolean;
}

export async function getTunerInfo(): Promise<TunerInfo> {
  return invoke<TunerInfo>('get_tuner_info');
}

// ── Metronome ──

export interface MetronomeState {
  bpm: number;
  beats_per_bar: number;
  running: boolean;
}

export async function getMetronomeState(): Promise<MetronomeState> {
  return invoke<MetronomeState>('get_metronome_state');
}

// ── Looper ──

export interface LooperState {
  mode: string;
  loop_time_seconds: number;
  has_loop: boolean;
}

export async function getLooperState(): Promise<LooperState> {
  return invoke<LooperState>('get_looper_state');
}

export async function triggerLooperMode(mode: number): Promise<boolean> {
  return invoke<boolean>('trigger_looper_mode', { mode });
}

export async function looperUndo(): Promise<boolean> {
  return invoke<boolean>('looper_undo');
}

export async function looperClear(): Promise<boolean> {
  return invoke<boolean>('looper_clear');
}

// ── Chain Mode Switching ──

export async function switchToBassChain(): Promise<void> {
  return invoke('switch_to_bass_chain');
}

export async function switchToPracticeChain(): Promise<void> {
  return invoke('switch_to_practice_chain');
}

export async function switchToLooperChain(): Promise<void> {
  return invoke('switch_to_looper_chain');
}

// ── Amp Presets ──

export async function listAmpPresets() {
  return invoke<import('../types/tauri').AmpPresetInfo[]>('list_amp_presets');
}

export async function applyAmpPreset(presetName: string): Promise<void> {
  return invoke('apply_amp_preset', { preset_name: presetName });
}

// ── Scenes (Live Mode) ──

export async function listScenes() {
  return invoke<import('../types/tauri').SceneInfo[]>('list_scenes');
}

export async function getScene(index: number) {
  return invoke<import('../types/tauri').SceneDetail>('get_scene', { index });
}

export async function saveScene(name: string) {
  return invoke<import('../types/tauri').SceneInfo>('save_scene', { name });
}

export async function updateScene(index: number): Promise<void> {
  return invoke('update_scene', { index });
}

export async function loadScene(index: number): Promise<void> {
  return invoke('load_scene', { index });
}

export async function deleteScene(index: number): Promise<void> {
  return invoke('delete_scene', { index });
}

export async function renameScene(index: number, newName: string): Promise<void> {
  return invoke('rename_scene', { index, new_name: newName });
}

export async function reorderScene(fromIndex: number, toIndex: number): Promise<void> {
  return invoke('reorder_scene', { from_index: fromIndex, to_index: toIndex });
}

export async function nextScene() {
  return invoke<import('../types/tauri').SceneInfo | null>('next_scene');
}

export async function prevScene() {
  return invoke<import('../types/tauri').SceneInfo | null>('prev_scene');
}
