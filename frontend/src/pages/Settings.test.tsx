import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import { Settings } from './Settings';
import { useSettingsStore } from '../stores/settingsStore';
import * as api from '../lib/tauri';

describe('Settings audio routing', () => {
  beforeEach(async () => {
    await useSettingsStore.getState().fetchSettings();
  });

  it('shows backend, input/output device and input channel controls', async () => {
    render(<Settings />);
    expect(await screen.findByLabelText('Audio backend')).toBeInTheDocument();
    expect(screen.getByLabelText('Input device')).toBeInTheDocument();
    expect(screen.getByLabelText('Output device')).toBeInTheDocument();
    expect(screen.getByLabelText('Input channel')).toBeInTheDocument();
  });

  it('selecting JACK and input 2 updates the settings payload', async () => {
    render(<Settings />);
    fireEvent.change(await screen.findByLabelText('Audio backend'), { target: { value: 'Jack' } });
    fireEvent.change(screen.getByLabelText('Input channel'), { target: { value: '1' } });
    const s = useSettingsStore.getState().settings!;
    expect(s.audio_backend).toBe('Jack');
    expect(s.input_channel).toBe(1);
  });

  it('reports latency, xruns and a missing input', async () => {
    vi.spyOn(api, 'getAudioStatus').mockResolvedValue({
      running: true,
      backend: 'Cpal',
      latency_ms: 16,
      underruns: 2,
      overruns: 1,
      callback_frames: 256,
      input_ok: false,
      input_error: 'No input device found',
    });
    render(<Settings />);
    await waitFor(() =>
      expect(screen.getByTestId('audio-status')).toHaveTextContent('~16.0 ms'),
    );
    const status = screen.getByTestId('audio-status');
    expect(status).toHaveTextContent('xruns: 3');
    expect(status).toHaveTextContent('No input: No input device found');
  });
});
