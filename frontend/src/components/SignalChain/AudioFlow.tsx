import { useEffect, useRef } from 'react';
import { useEngineStore } from '../../stores/engineStore';

/** Map plugin types to display colors and labels. */
function pluginMeta(pluginType: string) {
  const map: Record<string, { label: string; color: string; border: string }> = {
    Input: { label: 'IN', color: 'bg-zinc-600', border: 'border-zinc-500' },
    Output: { label: 'OUT', color: 'bg-zinc-600', border: 'border-zinc-500' },
    Boost: { label: 'BST', color: 'bg-amber-700', border: 'border-amber-500' },
    Amp: { label: 'AMP', color: 'bg-red-700', border: 'border-red-500' },
    BassAmp: { label: 'BASS', color: 'bg-indigo-700', border: 'border-indigo-500' },
    Cab: { label: 'CAB', color: 'bg-stone-600', border: 'border-stone-400' },
    Nam: { label: 'NAM', color: 'bg-violet-700', border: 'border-violet-500' },
    Delay: { label: 'DLY', color: 'bg-cyan-700', border: 'border-cyan-500' },
    Reverb: { label: 'REV', color: 'bg-sky-700', border: 'border-sky-500' },
    Tuner: { label: 'TUN', color: 'bg-emerald-700', border: 'border-emerald-500' },
    Metronome: { label: 'MET', color: 'bg-orange-700', border: 'border-orange-500' },
    Looper: { label: 'LOP', color: 'bg-pink-700', border: 'border-pink-500' },
    NoiseGate: { label: 'GATE', color: 'bg-slate-700', border: 'border-slate-500' },
    Compressor: { label: 'CMP', color: 'bg-teal-700', border: 'border-teal-500' },
    Overdrive: { label: 'OD', color: 'bg-lime-700', border: 'border-lime-500' },
    Eq: { label: 'EQ', color: 'bg-yellow-700', border: 'border-yellow-500' },
    Chorus: { label: 'CHO', color: 'bg-blue-700', border: 'border-blue-500' },
    Phaser: { label: 'PHS', color: 'bg-purple-700', border: 'border-purple-500' },
    Flanger: { label: 'FLG', color: 'bg-fuchsia-700', border: 'border-fuchsia-500' },
    Tremolo: { label: 'TRM', color: 'bg-rose-700', border: 'border-rose-500' },
  };
  return map[pluginType] ?? {
    label: pluginType.slice(0, 3).toUpperCase(),
    color: 'bg-zinc-500',
    border: 'border-zinc-400',
  };
}

/** Key parameters to show in the flow card for each plugin type. */
function keyParams(pluginType: string): string[] {
  switch (pluginType) {
    case 'Boost': return ['gain'];
    case 'Amp':
    case 'BassAmp':
      return ['gain', 'drive', 'master'];
    case 'Cab': return ['level', 'high_cut'];
    case 'Nam': return ['level'];
    case 'Delay': return ['time', 'mix'];
    case 'Reverb': return ['size', 'mix'];
    case 'Tuner': return ['mute'];
    case 'Metronome': return ['bpm'];
    case 'Looper': return ['mix'];
    case 'NoiseGate': return ['threshold'];
    case 'Compressor': return ['threshold', 'ratio'];
    case 'Overdrive': return ['drive', 'level'];
    case 'Eq': return ['low', 'mid', 'high'];
    case 'Chorus':
    case 'Phaser':
    case 'Flanger':
      return ['rate', 'mix'];
    case 'Tremolo': return ['rate', 'depth'];
    default: return [];
  }
}

/** Format a 0..1 parameter value for display. */
function fmtParam(value: number): string {
  if (value >= 1.0) return '100';
  if (value <= 0.0) return '0';
  return (value * 100).toFixed(0);
}

/** Returns a Tailwind colour class for the VU bar at a given RMS level. */
function vuColor(level: number): string {
  if (level > 0.92) return 'from-red-500 to-red-600';
  if (level > 0.80) return 'from-orange-400 to-amber-500';
  if (level > 0.55) return 'from-amber-400 to-yellow-500';
  if (level > 0.25) return 'from-green-400 to-green-500';
  return 'from-green-700 to-green-800';
}

function vuBg(level: number): string {
  if (level > 0.92) return 'bg-red-900/40';
  if (level > 0.80) return 'bg-amber-900/30';
  return 'bg-black/30';
}

/** Format a 0..1 RMS level as a dBFS-like label. */
function fmtLevel(level: number): string {
  if (level <= 0.001) return '-∞';
  const db = 20 * Math.log10(level);
  return `${db.toFixed(1)} dB`;
}

export function AudioFlow() {
  const { chain, fetchChain, pollLevels, levels, status } = useEngineStore();
  const scrollRef = useRef<HTMLDivElement>(null);
  const slots = chain?.slots ?? [];
  const running = status.running;

  useEffect(() => {
    fetchChain();
  }, [fetchChain]);

  // Poll audio levels at ~20 Hz while engine is running
  useEffect(() => {
    if (!running) return;
    const interval = setInterval(pollLevels, 50);
    return () => clearInterval(interval);
  }, [running, pollLevels]);

  // Auto-scroll to the end when chain changes (new plugin added)
  useEffect(() => {
    if (scrollRef.current) {
      scrollRef.current.scrollLeft = scrollRef.current.scrollWidth;
    }
  }, [slots.length]);

  if (slots.length === 0) {
    return (
      <div className="text-sm text-[var(--text-muted)] text-center py-8">
        No signal chain loaded. Start the engine or build a default chain.
      </div>
    );
  }

  return (
    <div className="space-y-3">
      <div className="flex items-center justify-between">
        <h3 className="text-sm font-medium text-[var(--text)]">Signal Flow</h3>
        <div className="flex items-center gap-3">
          {running && levels.length > 0 && (
            <span className="text-[10px] text-[var(--text-muted)] tabular-nums">
              OUT {fmtLevel(levels[levels.length - 1] ?? 0)}
            </span>
          )}
          <span className="text-[10px] text-[var(--text-muted)]">
            {slots.filter((s) => s.enabled).length} of {slots.length} active
          </span>
        </div>
      </div>

      <div
        ref={scrollRef}
        className="flex items-stretch gap-1 overflow-x-auto pb-2 scroll-smooth"
      >
        {slots.map((slot, i) => {
          const meta = pluginMeta(slot.plugin_type);
          const showParams = keyParams(slot.plugin_type);
          const isLast = i === slots.length - 1;
          const level = levels[i] ?? 0;

          return (
            <div key={slot.id} className="flex items-stretch gap-1 shrink-0">
              {/* Node card */}
              <div
                className={`
                  flex flex-col rounded-lg border min-w-[130px] max-w-[170px]
                  ${slot.enabled
                    ? `${meta.border} bg-[var(--bg-surface)]`
                    : 'border-dashed border-zinc-700 bg-[var(--bg-surface)]/60'
                  }
                  ${slot.enabled ? 'opacity-100' : 'opacity-50'}
                  transition-all duration-200
                `}
              >
                {/* Header badge */}
                <div className={`flex items-center gap-1.5 px-2.5 py-1.5 rounded-t-lg ${meta.color}`}>
                  <span className="text-[10px] font-bold text-white tracking-wider">
                    {meta.label}
                  </span>
                  <span className="flex-1 text-[10px] text-white/70 truncate">
                    {slot.id}
                  </span>
                  {!slot.enabled && (
                    <span className="text-[9px] text-white/50 font-medium">OFF</span>
                  )}
                </div>

                {/* Key parameters */}
                {showParams.length > 0 && (
                  <div className="px-2.5 py-1.5 flex flex-wrap gap-x-2 gap-y-0.5">
                    {showParams.map((pid) => {
                      const val = slot.parameters[pid];
                      return (
                        <div key={pid} className="flex items-center gap-1">
                          <span className="text-[9px] text-[var(--text-muted)] uppercase">
                            {pid}
                          </span>
                          <span className="text-[10px] text-[var(--text)] font-medium tabular-nums">
                            {val !== undefined ? fmtParam(val) : '--'}
                          </span>
                        </div>
                      );
                    })}
                  </div>
                )}

                <div className={`h-4 rounded-sm ${vuBg(level)} overflow-hidden relative`}>
                    <div
                      className={`h-full rounded-sm bg-gradient-to-r ${vuColor(level)} transition-all duration-75`}
                      style={{ width: `${Math.min(level * 100, 100)}%` }}
                    />
                    <div className="absolute top-0 right-2 h-full w-px bg-red-400/40" />
                  </div>
                  <div className="flex justify-between mt-0.5">
                    <span className="text-[8px] text-[var(--text-muted)] tabular-nums">
                      {fmtLevel(level)}
                    </span>
                    <span className="text-[8px] text-[var(--text-muted)]">
                      {(level * 100).toFixed(0)}%
                    </span>
                  </div>

                {/* Wet/dry indicator if not 100% */}
                {slot.wet_dry < 1.0 && (
                  <div className="px-2.5 pb-1.5 text-[9px] text-[var(--text-muted)]">
                    Wet {(slot.wet_dry * 100).toFixed(0)}%
                  </div>
                )}
              </div>

              {/* Arrow connector */}
              {!isLast && (
                <div className="flex items-center justify-center w-5 shrink-0">
                  <svg
                    className={`w-4 h-4 ${slot.enabled ? 'text-[var(--accent)]' : 'text-zinc-600'}`}
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    strokeWidth="2"
                    strokeLinecap="round"
                    strokeLinejoin="round"
                  >
                    <path d="M5 12h14" />
                    <path d="M15 6l6 6-6 6" />
                  </svg>
                </div>
              )}
            </div>
          );
        })}
      </div>

      {running && (
        <div className="flex items-center gap-3 text-[9px] text-[var(--text-muted)]">
          <span className="flex items-center gap-1">
            <span className="w-2 h-2 rounded-sm bg-green-500" />
            Safe
          </span>
          <span className="flex items-center gap-1">
            <span className="w-2 h-2 rounded-sm bg-amber-400" />
            Warm
          </span>
          <span className="flex items-center gap-1">
            <span className="w-2 h-2 rounded-sm bg-red-500" />
            Hot
          </span>
          <span className="flex items-center gap-1 ml-1">
            <span className="w-4 h-px bg-red-400/40" />
            0 dBFS
          </span>
        </div>
      )}
    </div>
  );
}
