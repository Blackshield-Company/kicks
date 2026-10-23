import { useEffect, useCallback, useState } from 'react';
import { useSettingsStore } from '../stores/settingsStore';
import { useHotkeys } from '../hooks/useHotkeys';
import * as api from '../lib/tauri';
import { RecordControl } from '../components/Recording/RecordControl';
import type { AudioDeviceInfo, AudioStatus } from '../types/tauri';
import { THEMES, applyTheme, getSavedThemeId, saveThemeId, getThemeById } from '../theme/theme';

export function Settings() {
  const { settings, loading, dirty, fetchSettings, saveSettings, updateSetting } =
    useSettingsStore();
  const [directoryError, setDirectoryError] = useState<string | null>(null);
  const [activeTheme, setActiveTheme] = useState(getSavedThemeId());
  const [devices, setDevices] = useState<AudioDeviceInfo[]>([]);
  const [devicesLoading, setDevicesLoading] = useState(true);
  const [audioStatus, setAudioStatus] = useState<AudioStatus | null>(null);

  useEffect(() => {
    let alive = true;
    const poll = () =>
      api.getAudioStatus()
        .then((s) => alive && setAudioStatus(s))
        .catch(() => {});
    poll();
    const id = setInterval(poll, 1000);
    return () => {
      alive = false;
      clearInterval(id);
    };
  }, []);

  useEffect(() => {
    fetchSettings();
  }, [fetchSettings]);

  useEffect(() => {
    api.listAudioDevices()
      .then(setDevices)
      .catch(() => {
        // Running outside Tauri / no CPAL — leave empty
      })
      .finally(() => setDevicesLoading(false));
  }, []);

  const handleSave = useCallback(() => {
    if (settings) saveSettings(settings);
  }, [settings, saveSettings]);

  const handlePickDirectory = async (key: 'ir_directories' | 'nam_directories') => {
    setDirectoryError(null);
    try {
      const directory = await api.pickLibraryDirectory();
      if (directory) updateSetting(key, [directory]);
    } catch (err) {
      setDirectoryError(typeof err === 'string' ? err : 'Failed to select directory');
    }
  };

  const handleThemeChange = useCallback((themeId: string) => {
    setActiveTheme(themeId);
    saveThemeId(themeId);
    applyTheme(getThemeById(themeId));
  }, []);

  useHotkeys([
    { key: 's', ctrl: true, handler: handleSave, ignoreInput: true },
  ]);

  if (!settings) {
    return <div className="text-[var(--text-muted)] text-sm">Loading settings...</div>;
  }

  return (
    <div className="flex flex-col gap-6 max-w-xl">
      <div className="flex items-center justify-between">
        <div>
          <h2 className="text-2xl font-bold text-[var(--text-h)]">Settings</h2>
          <p className="text-sm text-[var(--text-muted)] mt-1">
            Configure audio device, engine mode, AI assistant, and file directories.
          </p>
        </div>
        <button
          onClick={handleSave}
          disabled={!dirty || loading}
          className="px-4 py-2 rounded-lg bg-[var(--accent)] text-white text-sm font-medium hover:opacity-90 transition-opacity disabled:opacity-40 cursor-pointer"
        >
          {loading ? 'Saving...' : 'Save'}
        </button>
      </div>

      {/* Engine Mode */}
      <section className="border border-[var(--border)] rounded-xl p-4 bg-[var(--bg-surface)]">
        <h3 className="text-sm font-medium text-[var(--text)] mb-3">Audio Engine</h3>
        <div className="flex flex-col gap-3">
          <label className="flex items-center justify-between">
            <span className="text-sm text-[var(--text)]">Engine Mode</span>
            <select
              value={settings.engine_mode}
              onChange={(e) => updateSetting('engine_mode', e.target.value)}
              className="w-40 px-3 py-2 rounded-lg border border-[var(--border)] bg-[var(--bg)] text-[var(--text)] text-sm outline-none focus:border-[var(--accent)]"
            >
              <option value="Auto">Auto</option>
              <option value="Internal">Internal</option>
            </select>
          </label>
          <label className="flex items-center justify-between">
            <span className="text-sm text-[var(--text)]">JACK Client Name</span>
            <input
              type="text"
              value={settings.jack_client_name}
              onChange={(e) => updateSetting('jack_client_name', e.target.value)}
              className="w-40 px-3 py-2 rounded-lg border border-[var(--border)] bg-[var(--bg)] text-[var(--text)] text-sm outline-none focus:border-[var(--accent)]"
            />
          </label>
        </div>
      </section>

      {/* CPAL Audio Device */}
      <section className="border border-[var(--border)] rounded-xl p-4 bg-[var(--bg-surface)]">
        <h3 className="text-sm font-medium text-[var(--text)] mb-3">Audio Device</h3>
        <div className="flex flex-col gap-3">
          <label className="flex items-center justify-between">
            <span className="text-sm text-[var(--text)]">Sample Rate</span>
            <select
              value={settings.sample_rate}
              onChange={(e) => updateSetting('sample_rate', parseInt(e.target.value) || 48000)}
              className="w-40 px-3 py-2 rounded-lg border border-[var(--border)] bg-[var(--bg)] text-[var(--text)] text-sm outline-none focus:border-[var(--accent)]"
            >
              <option value={44100}>44100 Hz</option>
              <option value={48000}>48000 Hz</option>
              <option value={96000}>96000 Hz</option>
            </select>
          </label>
          <label className="flex items-center justify-between">
            <span className="text-sm text-[var(--text)]">Buffer Size</span>
            <select
              value={settings.buffer_size}
              onChange={(e) => updateSetting('buffer_size', parseInt(e.target.value) || 256)}
              className="w-40 px-3 py-2 rounded-lg border border-[var(--border)] bg-[var(--bg)] text-[var(--text)] text-sm outline-none focus:border-[var(--accent)]"
            >
              <option value={64}>64 samples</option>
              <option value={128}>128 samples</option>
              <option value={256}>256 samples</option>
              <option value={512}>512 samples</option>
              <option value={1024}>1024 samples</option>
            </select>
          </label>
          <label className="flex items-center justify-between">
            <span className="text-sm text-[var(--text)]">Device</span>
            <div className="flex items-center gap-2">
              <select
                value={settings.audio_device}
                onChange={(e) => updateSetting('audio_device', e.target.value)}
                className="w-48 px-3 py-2 rounded-lg border border-[var(--border)] bg-[var(--bg)] text-[var(--text)] text-sm outline-none focus:border-[var(--accent)]"
              >
                <option value="">System Default</option>
                {devices.map((d) => (
                  <option key={d.name} value={d.name}>
                    {d.name}
                    {d.is_input && d.is_output ? '' : d.is_input ? ' (input only)' : ' (output only)'}
                  </option>
                ))}
              </select>
              {devicesLoading && (
                <span className="text-[10px] text-[var(--text-muted)]">Scanning...</span>
              )}
            </div>
          </label>
        </div>
      </section>

      {/* Audio routing */}
      <section className="border border-[var(--border)] rounded-xl p-4 bg-[var(--bg-surface)]">
        <h3 className="text-sm font-medium text-[var(--text)] mb-3">Audio Routing</h3>
        <div className="flex flex-col gap-3">
          <label className="flex items-center justify-between">
            <span className="text-sm text-[var(--text)]">Backend</span>
            <select
              aria-label="Audio backend"
              value={settings.audio_backend || 'Cpal'}
              onChange={(e) => updateSetting('audio_backend', e.target.value)}
              className="w-48 px-3 py-2 rounded-lg border border-[var(--border)] bg-[var(--bg)] text-[var(--text)] text-sm outline-none focus:border-[var(--accent)]"
            >
              <option value="Cpal">System (ALSA / PipeWire / CoreAudio / WASAPI)</option>
              <option value="Jack">JACK / PipeWire-JACK</option>
            </select>
          </label>
          <label className="flex items-center justify-between">
            <span className="text-sm text-[var(--text)]">Input Device</span>
            <select
              aria-label="Input device"
              value={settings.input_device ?? ''}
              onChange={(e) => updateSetting('input_device', e.target.value)}
              className="w-48 px-3 py-2 rounded-lg border border-[var(--border)] bg-[var(--bg)] text-[var(--text)] text-sm outline-none focus:border-[var(--accent)]"
            >
              <option value="">System Default</option>
              {devices.filter((d) => d.is_input).map((d) => (
                <option key={d.name} value={d.name}>{d.name}</option>
              ))}
            </select>
          </label>
          <label className="flex items-center justify-between">
            <span className="text-sm text-[var(--text)]">Input Channel</span>
            <select
              aria-label="Input channel"
              value={settings.input_channel ?? 0}
              onChange={(e) => updateSetting('input_channel', parseInt(e.target.value) || 0)}
              className="w-48 px-3 py-2 rounded-lg border border-[var(--border)] bg-[var(--bg)] text-[var(--text)] text-sm outline-none focus:border-[var(--accent)]"
            >
              {[0, 1, 2, 3, 4, 5, 6, 7].map((c) => (
                <option key={c} value={c}>Input {c + 1}</option>
              ))}
            </select>
          </label>
          <label className="flex items-center justify-between">
            <span className="text-sm text-[var(--text)]">Output Device</span>
            <select
              aria-label="Output device"
              value={settings.output_device ?? ''}
              onChange={(e) => updateSetting('output_device', e.target.value)}
              className="w-48 px-3 py-2 rounded-lg border border-[var(--border)] bg-[var(--bg)] text-[var(--text)] text-sm outline-none focus:border-[var(--accent)]"
            >
              <option value="">System Default</option>
              {devices.filter((d) => d.is_output).map((d) => (
                <option key={d.name} value={d.name}>{d.name}</option>
              ))}
            </select>
          </label>
          <div data-testid="audio-status" className="text-xs text-[var(--text-muted)] flex flex-wrap gap-x-4 gap-y-1">
            {audioStatus?.running ? (
              <>
                <span>{audioStatus.backend}</span>
                <span>~{audioStatus.latency_ms.toFixed(1)} ms</span>
                <span>{audioStatus.callback_frames || '—'} frames/callback</span>
                <span>xruns: {audioStatus.underruns + audioStatus.overruns}</span>
                {!audioStatus.input_ok && (
                  <span className="text-red-400">
                    No input: {audioStatus.input_error ?? 'unknown error'}
                  </span>
                )}
              </>
            ) : (
              <span>Audio stopped</span>
            )}
          </div>
          <RecordControl />
        </div>
      </section>

      {/* AI Assistant */}
      <section className="border border-[var(--border)] rounded-xl p-4 bg-[var(--bg-surface)]">
        <h3 className="text-sm font-medium text-[var(--text)] mb-3">AI Tone Assistant</h3>
        <div className="flex flex-col gap-3">
          {/* Provider */}
          <label className="flex items-center justify-between">
            <span className="text-sm text-[var(--text)]">Provider</span>
            <select
              value={settings.ai_provider}
              onChange={(e) => {
                const provider = e.target.value;
                updateSetting('ai_provider', provider);
                // Set default endpoint URL when switching provider
                if (provider === 'Anthropic') {
                  updateSetting('ai_endpoint_url', 'https://api.anthropic.com/v1/messages');
                } else if (provider === 'OpenAI') {
                  updateSetting('ai_endpoint_url', 'https://api.openai.com/v1/chat/completions');
                }
              }}
              className="w-64 px-3 py-2 rounded-lg border border-[var(--border)] bg-[var(--bg)] text-[var(--text)] text-sm outline-none focus:border-[var(--accent)]"
            >
              <option value="Anthropic">Anthropic</option>
              <option value="OpenAI">OpenAI / OpenRouter / Local</option>
            </select>
          </label>

          {/* Endpoint URL */}
          <label className="flex items-center justify-between">
            <span className="text-sm text-[var(--text)]">Endpoint URL</span>
            <input
              type="text"
              value={settings.ai_endpoint_url}
              onChange={(e) => updateSetting('ai_endpoint_url', e.target.value)}
              placeholder={settings.ai_provider === 'OpenAI' ? 'http://localhost:8080/v1/chat/completions' : 'https://api.anthropic.com/v1/messages'}
              className="w-64 px-3 py-2 rounded-lg border border-[var(--border)] bg-[var(--bg)] text-[var(--text)] text-sm outline-none focus:border-[var(--accent)] font-mono text-[11px]"
            />
          </label>

          {/* API Key */}
          <label className="flex items-center justify-between">
            <span className="text-sm text-[var(--text)]">API Key</span>
            <input
              type="password"
              value={settings.ai_api_key}
              onChange={(e) => updateSetting('ai_api_key', e.target.value)}
              placeholder={settings.ai_provider === 'OpenAI' ? 'sk-... (leave empty for local models)' : 'sk-ant-...'}
              className="w-64 px-3 py-2 rounded-lg border border-[var(--border)] bg-[var(--bg)] text-[var(--text)] text-sm outline-none focus:border-[var(--accent)]"
            />
          </label>

          {/* Model */}
          <label className="flex items-center justify-between">
            <span className="text-sm text-[var(--text)]">Model</span>
            <input
              type="text"
              value={settings.ai_model}
              onChange={(e) => updateSetting('ai_model', e.target.value)}
              placeholder={settings.ai_provider === 'OpenAI' ? 'gpt-4o, gemini-2.0-flash, glm-5.1, etc.' : 'claude-sonnet-4-20250514'}
              className="w-64 px-3 py-2 rounded-lg border border-[var(--border)] bg-[var(--bg)] text-[var(--text)] text-sm outline-none focus:border-[var(--accent)]"
            />
          </label>

          {/* Provider info hint */}
          <div className="mt-1 text-[11px] text-[var(--text-muted)] leading-relaxed">
            {settings.ai_provider === 'OpenAI' ? (
              <p>
                Works with any OpenAI-compatible API: <strong>OpenAI</strong>, <strong>OpenRouter</strong>,
                <strong> Ollama</strong>, <strong>llama-server</strong>, <strong>llama-swap</strong>,
                <strong> GLM</strong>, <strong>Kimi</strong>, <strong>Together</strong>, <strong>Groq</strong>, and more.
                Set the endpoint URL above to your provider's chat completions URL.
              </p>
            ) : (
              <p>
                Uses the Anthropic Messages API directly. Requires an <strong>Anthropic API key</strong>.
              </p>
            )}
          </div>
        </div>
      </section>

      {/* Theme */}
      <section className="border border-[var(--border)] rounded-xl p-4 bg-[var(--bg-surface)]">
        <h3 className="text-sm font-medium text-[var(--text)] mb-3">Appearance</h3>
        <div className="flex flex-wrap gap-2">
          {THEMES.map((theme) => (
            <button
              key={theme.id}
              onClick={() => handleThemeChange(theme.id)}
              className={`px-3 py-2 rounded-lg text-xs font-medium transition-all cursor-pointer border ${
                activeTheme === theme.id
                  ? 'bg-[var(--accent)] text-white border-[var(--accent)] neon-button'
                  : 'bg-[var(--bg-elevated)] text-[var(--text-muted)] border-[var(--border)] hover:text-[var(--text)]'
              }`}
            >
              <span className="flex items-center gap-2">
                <span
                  className="w-3 h-3 rounded-full border border-white/20"
                  style={{ background: theme.colors.accent }}
                />
                {theme.name}
              </span>
            </button>
          ))}
        </div>
      </section>

      {/* File Directories */}
      <section className="border border-[var(--border)] rounded-xl p-4 bg-[var(--bg-surface)]">
        <h3 className="text-sm font-medium text-[var(--text)] mb-3">File Directories</h3>
        <p className="text-xs text-[var(--text-muted)] mb-3">Choose library folders each session to approve access, then save settings.</p>
        {directoryError && <p role="alert" className="text-sm text-[var(--danger)]">{directoryError}</p>}
        <div className="flex flex-col gap-3">
          <label className="flex items-center justify-between">
            <span className="text-sm text-[var(--text)]">IR Directory</span>
            <input
              type="text"
              value={settings.ir_directories[0] ?? ''}
              readOnly
              className="flex-1 ml-4 px-3 py-2 rounded-lg border border-[var(--border)] bg-[var(--bg)] text-[var(--text)] text-sm outline-none focus:border-[var(--accent)]"
            />
            <button type="button" onClick={() => handlePickDirectory('ir_directories')}
              className="ml-2 px-3 py-2 rounded-lg bg-[var(--accent)] text-white text-sm cursor-pointer">
              Choose IR Folder
            </button>
          </label>
          <label className="flex items-center justify-between">
            <span className="text-sm text-[var(--text)]">NAM Directory</span>
            <input
              type="text"
              value={settings.nam_directories[0] ?? ''}
              readOnly
              className="flex-1 ml-4 px-3 py-2 rounded-lg border border-[var(--border)] bg-[var(--bg)] text-[var(--text)] text-sm outline-none focus:border-[var(--accent)]"
            />
            <button type="button" onClick={() => handlePickDirectory('nam_directories')}
              className="ml-2 px-3 py-2 rounded-lg bg-[var(--accent)] text-white text-sm cursor-pointer">
              Choose NAM Folder
            </button>
          </label>
          <label className="flex items-center justify-between">
            <span className="text-sm text-[var(--text)]">Preset Directory</span>
            <input
              type="text"
              value={settings.preset_directories[0] ?? ''}
              onChange={(e) => updateSetting('preset_directories', [e.target.value])}
              className="flex-1 ml-4 px-3 py-2 rounded-lg border border-[var(--border)] bg-[var(--bg)] text-[var(--text)] text-sm outline-none focus:border-[var(--accent)]"
            />
          </label>
        </div>
      </section>
    </div>
  );
}
