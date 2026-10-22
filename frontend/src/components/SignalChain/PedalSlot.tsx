import { useState } from 'react';
import type { ChainSlot } from '../../types/tauri';
import { ParamSlider } from './ParamSlider';
import { AmpPresetSelector } from '../AmpPresets/AmpPresetSelector';

interface PedalSlotProps {
  slot: ChainSlot;
  index: number;
  isDragging: boolean;
  onToggle: () => void;
  onParamChange: (param: string, value: number) => void;
  onDragStart: (e: React.DragEvent) => void;
  onDragOver: (e: React.DragEvent) => void;
  onDrop: (e: React.DragEvent) => void;
}

/** Map plugin types to labels and colors. */
function pluginMeta(pluginType: string) {
  const map: Record<string, { label: string; color: string }> = {
    Input: { label: 'IN', color: 'bg-zinc-600' },
    Output: { label: 'OUT', color: 'bg-zinc-600' },
    Boost: { label: 'BST', color: 'bg-amber-600' },
    Amp: { label: 'AMP', color: 'bg-red-700' },
    BassAmp: { label: 'BASS', color: 'bg-indigo-700' },
    Cab: { label: 'CAB', color: 'bg-stone-600' },
    Nam: { label: 'NAM', color: 'bg-violet-700' },
    Delay: { label: 'DLY', color: 'bg-cyan-700' },
    Reverb: { label: 'REV', color: 'bg-sky-700' },
    Tuner: { label: 'TUN', color: 'bg-emerald-700' },
    Metronome: { label: 'MET', color: 'bg-orange-700' },
    Looper: { label: 'LOP', color: 'bg-pink-700' },
    NoiseGate: { label: 'GATE', color: 'bg-slate-700' },
    Compressor: { label: 'CMP', color: 'bg-teal-700' },
    Overdrive: { label: 'OD', color: 'bg-lime-700' },
    Eq: { label: 'EQ', color: 'bg-yellow-700' },
    Chorus: { label: 'CHO', color: 'bg-blue-700' },
    Phaser: { label: 'PHS', color: 'bg-purple-700' },
    Flanger: { label: 'FLG', color: 'bg-fuchsia-700' },
    Tremolo: { label: 'TRM', color: 'bg-rose-700' },
  };
  return map[pluginType] ?? { label: pluginType.slice(0, 3).toUpperCase(), color: 'bg-zinc-500' };
}

/** Plugin-type → parameter schema for default knobs. */
function pluginParams(pluginType: string): { id: string; label: string }[] {
  switch (pluginType) {
    case 'Boost':
      return [{ id: 'gain', label: 'Gain' }];
    case 'Amp':
    case 'BassAmp':
      return [
        { id: 'gain', label: 'Gain' },
        { id: 'bass', label: 'Bass' },
        { id: 'mid', label: 'Mid' },
        { id: 'treble', label: 'Treble' },
        { id: 'drive', label: 'Drive' },
        { id: 'master', label: 'Master' },
      ];
    case 'Cab':
      return [
        { id: 'level', label: 'Level' },
        { id: 'low_cut', label: 'Low Cut' },
        { id: 'high_cut', label: 'High Cut' },
      ];
    case 'Nam':
      return [
        { id: 'level', label: 'Level' },
      ];
    case 'Delay':
      return [
        { id: 'time', label: 'Time' },
        { id: 'feedback', label: 'Feedback' },
        { id: 'mix', label: 'Mix' },
      ];
    case 'Reverb':
      return [
        { id: 'size', label: 'Size' },
        { id: 'damping', label: 'Damping' },
        { id: 'mix', label: 'Mix' },
      ];
    case 'Tuner':
      return [{ id: 'mute', label: 'Mute' }];
    case 'Metronome':
      return [
        { id: 'bpm', label: 'BPM' },
        { id: 'volume', label: 'Volume' },
      ];
    case 'Looper':
      return [
        { id: 'feedback', label: 'Feedback' },
        { id: 'mix', label: 'Mix' },
        { id: 'speed', label: 'Speed' },
      ];
    case 'NoiseGate':
      return [
        { id: 'threshold', label: 'Threshold' },
        { id: 'release', label: 'Release' },
      ];
    case 'Compressor':
      return [
        { id: 'threshold', label: 'Threshold' },
        { id: 'ratio', label: 'Ratio' },
        { id: 'attack', label: 'Attack' },
        { id: 'release', label: 'Release' },
        { id: 'makeup', label: 'Makeup' },
      ];
    case 'Overdrive':
      return [
        { id: 'drive', label: 'Drive' },
        { id: 'tone', label: 'Tone' },
        { id: 'level', label: 'Level' },
      ];
    case 'Eq':
      return [
        { id: 'low', label: 'Low' },
        { id: 'mid', label: 'Mid' },
        { id: 'mid_freq', label: 'Mid Freq' },
        { id: 'q', label: 'Q' },
        { id: 'high', label: 'High' },
      ];
    case 'Chorus':
      return [
        { id: 'rate', label: 'Rate' },
        { id: 'depth', label: 'Depth' },
        { id: 'mix', label: 'Mix' },
      ];
    case 'Phaser':
    case 'Flanger':
      return [
        { id: 'rate', label: 'Rate' },
        { id: 'depth', label: 'Depth' },
        { id: 'feedback', label: 'Feedback' },
        { id: 'mix', label: 'Mix' },
      ];
    case 'Tremolo':
      return [
        { id: 'rate', label: 'Rate' },
        { id: 'depth', label: 'Depth' },
      ];
    case 'Output':
      return [
        { id: 'volume', label: 'Volume' },
      ];
    default:
      return [];
  }
}

export function PedalSlot({ slot, isDragging, onToggle, onParamChange, onDragStart, onDragOver, onDrop }: PedalSlotProps) {
  const [expanded, setExpanded] = useState(true);
  const [showAmpPresets, setShowAmpPresets] = useState(false);
  const meta = pluginMeta(slot.plugin_type);
  const params = pluginParams(slot.plugin_type);
  const isFixed = slot.plugin_type === 'Input' || slot.plugin_type === 'Output';
  const isAmp = slot.plugin_type === 'Amp' || slot.plugin_type === 'BassAmp';

  const handleApplyAmpPreset = (presetName: string) => {
    // Refetch chain to reflect the newly applied parameters
    window.dispatchEvent(new CustomEvent('amp-preset-applied', { detail: { presetName } }));
    setShowAmpPresets(false);
  };

  return (
    <div
      draggable={!isFixed}
      onDragStart={onDragStart}
      onDragOver={onDragOver}
      onDrop={onDrop}
      className={`
        flex-shrink-0 w-52 rounded-lg border 
        ${slot.enabled ? 'border-[var(--border)]' : 'border-dashed border-zinc-700 opacity-60'}
        ${isDragging ? 'opacity-50 ring-2 ring-[var(--accent)]' : ''}
        bg-[var(--bg-surface)] overflow-hidden cursor-default
        transition-all duration-150
      `}
    >
      {/* Header */}
      <div className={`flex items-center gap-2 px-3 py-2 ${meta.color} bg-opacity-60`}>
        <span className="text-xs font-bold text-white tracking-wider">{meta.label}</span>
        <span className="flex-1 text-xs text-white/80 truncate">{slot.id}</span>
        {!isFixed && (
          <button
            onClick={(e) => { e.stopPropagation(); onToggle(); }}
            className={`w-5 h-5 rounded-full border border-white/40 flex items-center justify-center transition-colors cursor-pointer
              ${slot.enabled ? 'bg-green-500 border-green-500' : 'bg-transparent'}`}
            title={slot.enabled ? 'Disable' : 'Enable'}
          >
            {slot.enabled && <span className="text-white text-[10px]">✓</span>}
          </button>
        )}
      </div>

      {/* Wet/dry indicator */}
      {slot.wet_dry < 1.0 && (
        <div className="px-3 py-1 text-[10px] text-[var(--text-muted)] bg-black/20">
          Wet: {(slot.wet_dry * 100).toFixed(0)}%
        </div>
      )}

      {/* Expand button */}
      {params.length > 0 && (
        <button
          onClick={() => setExpanded(!expanded)}
          className="w-full px-3 py-1.5 text-[11px] text-[var(--text-muted)] hover:text-[var(--text)] transition-colors text-left cursor-pointer"
        >
          {expanded ? '▲ Less' : '▼ More'}
        </button>
      )}

      {/* Amp Presets button */}
      {isAmp && (
        <button
          onClick={() => setShowAmpPresets(true)}
          className="w-full px-3 py-1.5 text-[10px] font-medium text-[var(--accent)] hover:bg-[var(--accent-bg)] transition-colors text-left cursor-pointer border-t border-[var(--border)]"
        >
          Amp Presets
        </button>
      )}

      {/* Parameters */}
      {expanded && (
        <div className="px-3 pb-3 flex flex-col gap-2">
          {params.map((p) => (
            <ParamSlider
              key={p.id}
              label={p.label}
              value={slot.parameters[p.id] ?? 0.5}
              onChange={(v) => onParamChange(p.id, v)}
            />
          ))}
        </div>
      )}

      {/* Amp Preset Selector modal */}
      {showAmpPresets && (
        <AmpPresetSelector
          onApplyPreset={handleApplyAmpPreset}
          onClose={() => setShowAmpPresets(false)}
        />
      )}
    </div>
  );
}
