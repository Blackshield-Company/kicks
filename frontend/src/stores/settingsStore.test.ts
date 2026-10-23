import { describe, it, expect, vi, beforeEach } from 'vitest';
import * as api from '../lib/tauri';
import { useSettingsStore } from './settingsStore';

vi.mock('../lib/tauri', () => ({
  getSettings: vi.fn(),
  saveSettings: vi.fn(),
}));

const mockedApi = vi.mocked(api);

describe('useSettingsStore', () => {
  beforeEach(() => {
    useSettingsStore.setState({
      settings: null,
      loading: false,
      dirty: false,
    });
    vi.clearAllMocks();
  });

  it('initializes with null settings', () => {
    const state = useSettingsStore.getState();
    expect(state.settings).toBeNull();
    expect(state.loading).toBe(false);
    expect(state.dirty).toBe(false);
  });

  it('fetchSettings loads settings', async () => {
    const payload = {
      engine_mode: 'Auto',
      jack_client_name: 'kicks',
      sample_rate: 48000,
      buffer_size: 256,
      audio_device: '',
      ir_directories: [],
      nam_directories: [],
      preset_directories: [],
      ai_provider: 'Anthropic',
      ai_endpoint_url: 'https://api.anthropic.com/v1/messages',
      ai_api_key: '',
      ai_model: 'claude-sonnet-4-20250514',
    };
    mockedApi.getSettings.mockResolvedValueOnce(payload);

    await useSettingsStore.getState().fetchSettings();

    expect(mockedApi.getSettings).toHaveBeenCalledOnce();
    expect(useSettingsStore.getState().settings).toEqual(payload);
    expect(useSettingsStore.getState().dirty).toBe(false);
    expect(useSettingsStore.getState().loading).toBe(false);
  });

  it('saveSettings sends payload to backend', async () => {
    const payload = {
      engine_mode: 'Manual',
      jack_client_name: 'kicks-test',
      sample_rate: 44100,
      buffer_size: 512,
      audio_device: 'USB Audio',
      ir_directories: ['/irs'],
      nam_directories: [],
      preset_directories: [],
      ai_provider: 'Anthropic',
      ai_endpoint_url: 'https://api.anthropic.com/v1/messages',
      ai_api_key: 'sk-test',
      ai_model: 'claude-opus-4',
    };
    mockedApi.saveSettings.mockResolvedValueOnce(undefined);

    await useSettingsStore.getState().saveSettings(payload);

    expect(mockedApi.saveSettings).toHaveBeenCalledWith(payload);
    expect(useSettingsStore.getState().settings).toEqual(payload);
    expect(useSettingsStore.getState().dirty).toBe(false);
    expect(useSettingsStore.getState().loading).toBe(false);
  });

  it('updateSetting mutates local state and marks dirty', () => {
    const base = {
      engine_mode: 'Auto',
      jack_client_name: 'kicks',
      sample_rate: 48000,
      buffer_size: 256,
      audio_device: '',
      ir_directories: [],
      nam_directories: [],
      preset_directories: [],
      ai_provider: 'Anthropic',
      ai_endpoint_url: 'https://api.anthropic.com/v1/messages',
      ai_api_key: '',
      ai_model: 'claude-sonnet-4-20250514',
    };
    useSettingsStore.setState({ settings: base });

    useSettingsStore.getState().updateSetting('buffer_size', 512);

    expect(useSettingsStore.getState().settings?.buffer_size).toBe(512);
    expect(useSettingsStore.getState().dirty).toBe(true);
  });

  it('updateSetting does nothing when settings are null', () => {
    useSettingsStore.getState().updateSetting('buffer_size', 512);
    expect(useSettingsStore.getState().settings).toBeNull();
    expect(useSettingsStore.getState().dirty).toBe(false);
  });
});
