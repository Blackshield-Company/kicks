import { useCallback, useEffect, useState } from 'react';
import * as api from '../../lib/tauri';
import type { RecordingStatus } from '../../types/tauri';

function fmtTime(seconds: number): string {
  const m = Math.floor(seconds / 60);
  const s = Math.floor(seconds % 60);
  return `${m}:${s.toString().padStart(2, '0')}`;
}

/**
 * Record button: captures the dry DI (channel 1) and the processed sound
 * (channel 2) to a WAV file in the Kicks recordings folder.
 */
export function RecordControl() {
  const [status, setStatus] = useState<RecordingStatus | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const refresh = useCallback(
    () =>
      api
        .getRecordingStatus()
        .then(setStatus)
        .catch(() => {
          /* status is best-effort */
        }),
    [],
  );

  useEffect(() => {
    let alive = true;
    const poll = () =>
      api
        .getRecordingStatus()
        .then((s) => alive && setStatus(s))
        .catch(() => {});
    poll();
    const id = setInterval(poll, 500);
    return () => {
      alive = false;
      clearInterval(id);
    };
  }, []);

  const toggle = async () => {
    setBusy(true);
    setMessage(null);
    try {
      if (status?.recording) {
        const sum = await api.stopRecording();
        setMessage(
          `Saved ${fmtTime(sum.seconds)} to ${sum.path}` +
            (sum.dropped_frames > 0 ? ` (${sum.dropped_frames} frames dropped)` : ''),
        );
      } else {
        await api.startRecording();
      }
    } catch (e) {
      setMessage(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
      refresh();
    }
  };

  const recording = status?.recording ?? false;
  return (
    <div className="flex flex-col gap-1" data-testid="record-control">
      <div className="flex items-center gap-3">
        <button
          type="button"
          onClick={toggle}
          disabled={busy}
          aria-pressed={recording}
          className={`px-3 py-1.5 rounded-md text-xs font-medium ${
            recording ? 'bg-red-600 text-white' : 'bg-[var(--bg-raised,#333)] text-[var(--text)]'
          }`}
        >
          {recording ? 'Stop recording' : 'Record DI + amp'}
        </button>
        {recording && (
          <span className="text-xs text-red-400" data-testid="record-time">
            ● {fmtTime(status?.seconds ?? 0)}
            {(status?.dropped_frames ?? 0) > 0 && ` · ${status?.dropped_frames} dropped`}
          </span>
        )}
      </div>
      {message && <span className="text-xs text-[var(--text-muted)]">{message}</span>}
    </div>
  );
}
