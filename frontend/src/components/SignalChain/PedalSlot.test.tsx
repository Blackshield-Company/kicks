import { describe, it, expect, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import { PedalSlot } from './PedalSlot';
import type { ChainSlot } from '../../types/tauri';

const noop = () => {};

function renderSlot(plugin_type: string, parameters: Record<string, number>) {
  const slot: ChainSlot = { id: plugin_type.toLowerCase(), plugin_type, enabled: false, wet_dry: 1, parameters };
  return render(
    <PedalSlot
      slot={slot}
      index={2}
      isDragging={false}
      onToggle={noop}
      onParamChange={vi.fn()}
      onDragStart={noop}
      onDragOver={noop}
      onDrop={noop}
    />,
  );
}

describe('PedalSlot workstation pedals', () => {
  it.each([
    ['NoiseGate', 'GATE', ['Threshold', 'Release']],
    ['Compressor', 'CMP', ['Threshold', 'Ratio', 'Attack', 'Release', 'Makeup']],
    ['Overdrive', 'OD', ['Drive', 'Tone', 'Level']],
    ['Eq', 'EQ', ['Low', 'Mid', 'Mid Freq', 'Q', 'High']],
    ['Chorus', 'CHO', ['Rate', 'Depth', 'Mix']],
    ['Phaser', 'PHS', ['Rate', 'Depth', 'Feedback', 'Mix']],
    ['Flanger', 'FLG', ['Rate', 'Depth', 'Feedback', 'Mix']],
    ['Tremolo', 'TRM', ['Rate', 'Depth']],
  ])('%s shows its label and knobs', (type, label, knobs) => {
    renderSlot(type, {});
    expect(screen.getAllByText(label).length).toBeGreaterThan(0);
    for (const k of knobs) {
      expect(screen.getAllByText(k).length).toBeGreaterThan(0);
    }
  });
});
