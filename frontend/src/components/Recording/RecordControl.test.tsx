import { describe, it, expect, vi, afterEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import { RecordControl } from './RecordControl';
import * as api from '../../lib/tauri';

afterEach(() => vi.restoreAllMocks());

describe('RecordControl', () => {
  it('starts recording and shows the elapsed time', async () => {
    let recording = false;
    vi.spyOn(api, 'getRecordingStatus').mockImplementation(async () => ({
      recording,
      error: null,
      path: recording ? '/r/kicks-1.wav' : null,
      seconds: recording ? 65 : 0,
      dropped_frames: 0,
    }));
    const start = vi.spyOn(api, 'startRecording').mockImplementation(async () => {
      recording = true;
      return '/r/kicks-1.wav';
    });
    render(<RecordControl />);
    fireEvent.click(await screen.findByRole('button', { name: 'Record DI + amp' }));
    await waitFor(() => expect(start).toHaveBeenCalled());
    expect(await screen.findByTestId('record-time')).toHaveTextContent('1:05');
    expect(screen.getByRole('button', { name: 'Stop recording' })).toHaveAttribute('aria-pressed', 'true');
  });

  it('stops and reports the saved file and dropped frames', async () => {
    vi.spyOn(api, 'getRecordingStatus').mockResolvedValue({
      recording: true, error: null, path: '/r/a.wav', seconds: 3, dropped_frames: 0,
    });
    vi.spyOn(api, 'stopRecording').mockResolvedValue({
      path: '/r/a.wav', frames: 144000, dropped_frames: 12, seconds: 3,
    });
    render(<RecordControl />);
    fireEvent.click(await screen.findByRole('button', { name: 'Stop recording' }));
    expect(await screen.findByText(/Saved 0:03 to \/r\/a.wav \(12 frames dropped\)/)).toBeInTheDocument();
  });

  it('shows writer failures reported by polling and allows another recording', async () => {
    vi.spyOn(api, 'getRecordingStatus').mockResolvedValue({
      recording: false, error: 'disk full', path: '/r/a.wav', seconds: 3, dropped_frames: 0,
    });
    render(<RecordControl />);
    expect(await screen.findByRole('alert')).toHaveTextContent('Recording failed: disk full');
    expect(screen.getByRole('button', { name: 'Record DI + amp' })).toHaveAttribute('aria-pressed', 'false');
  });

  it('shows an error when recording cannot start', async () => {
    render(<RecordControl />);
    fireEvent.click(await screen.findByRole('button', { name: 'Record DI + amp' }));
    expect(await screen.findByText('Recording needs the desktop app')).toBeInTheDocument();
  });
});
