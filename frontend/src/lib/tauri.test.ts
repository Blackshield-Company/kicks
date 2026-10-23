import { describe, it, expect } from 'vitest';

// We test the simulate fallback by invoking commands directly.
// Because `invoke` is module-private, we exercise it via the exported helpers.
import {
  engineStatus,
  getVersion,
  getSignalChain,
  listPresets,
  getSettings,
  listAudioDevices,
  listScenes,
  listAmpPresets,
  getAudioStatus,
  toCommandArgs,
} from './tauri';

describe('tauri API simulation (outside Tauri)', () => {
  it('getVersion returns dev version', async () => {
    const v = await getVersion();
    expect(v).toBe('0.1.0 (dev)');
  });

  it('engineStatus returns simulated status', async () => {
    const status = await engineStatus();
    expect(status.running).toBe(true);
    expect(status.sample_rate).toBe(48000);
    expect(status.buffer_size).toBe(256);
  });

  it('getSignalChain returns default chain snapshot', async () => {
    const chain = await getSignalChain();
    expect(chain.slots).toHaveLength(9);
    expect(chain.slots[0].plugin_type).toBe('Input');
    expect(chain.slots[1].plugin_type).toBe('Tuner');
    expect(chain.slots[8].plugin_type).toBe('Output');
  });

  it('listPresets returns empty array', async () => {
    const presets = await listPresets();
    expect(presets).toEqual([]);
  });

  it('getSettings returns simulated defaults', async () => {
    const settings = await getSettings();
    expect(settings.guitarix_host).toBe('127.0.0.1');
    expect(settings.engine_mode).toBe('Auto');
    expect(settings.ai_provider).toBe('Anthropic');
  });

  it('listAudioDevices returns simulated devices', async () => {
    const devices = await listAudioDevices();
    expect(devices.length).toBeGreaterThan(0);
    expect(devices[0]).toHaveProperty('name');
    expect(devices[0]).toHaveProperty('is_input');
    expect(devices[0]).toHaveProperty('is_output');
  });

  it('listScenes returns simulated scenes', async () => {
    const scenes = await listScenes();
    expect(scenes).toHaveLength(3);
    expect(scenes[0].name).toBe('Clean');
    expect(scenes[0].is_active).toBe(true);
  });

  it('listAmpPresets returns built-in presets', async () => {
    const presets = await listAmpPresets();
    expect(presets.length).toBeGreaterThan(0);
    expect(presets[0]).toHaveProperty('name');
    expect(presets[0]).toHaveProperty('gain');
    expect(presets[0]).toHaveProperty('tags');
  });

  it('getSettings includes audio routing fields', async () => {
    const s = await getSettings();
    expect(s.audio_backend).toBe('Cpal');
    expect(s.input_channel).toBe(0);
    expect(s.input_device).toBe('');
    expect(s.output_device).toBe('');
  });

  it('getAudioStatus reports stopped audio outside Tauri', async () => {
    const st = await getAudioStatus();
    expect(st.running).toBe(false);
    expect(st.underruns).toBe(0);
  });

  it('toCommandArgs camelCases top-level keys only', () => {
    expect(toCommandArgs({ slot_id: 'amp', wet_dry: 0.5, enabled: true })).toEqual({
      slotId: 'amp',
      wetDry: 0.5,
      enabled: true,
    });
    const settings = { input_channel: 1 };
    expect(toCommandArgs({ settings })).toEqual({ settings: { input_channel: 1 } });
    expect(toCommandArgs({ from_idx: 1, to_idx: 3 })).toEqual({ fromIdx: 1, toIdx: 3 });
    expect(toCommandArgs()).toEqual({});
  });
});
