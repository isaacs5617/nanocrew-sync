import React from 'react';
import { getTokens, type Theme } from '../tokens.js';

interface NCToggleProps {
  on?: boolean;
  onChange?: (value: boolean) => void;
  theme?: Theme;
}

export const NCToggle: React.FC<NCToggleProps> = ({ on, onChange, theme = 'dark' }) => {
  const t = getTokens(theme);
  return (
    <button
      onClick={() => onChange?.(!on)}
      style={{
        width: 34, height: 20, borderRadius: 10, border: 'none',
        background: on ? t.lime : t.surface3, position: 'relative',
        cursor: 'pointer', transition: 'background 150ms', padding: 0,
        flexShrink: 0,
      }}
    >
      <span style={{
        position: 'absolute', top: 2, left: on ? 16 : 2,
        width: 16, height: 16, borderRadius: '50%',
        background: on ? '#0A0A0A' : t.textMd,
        transition: 'left 150ms',
      }} />
    </button>
  );
};
