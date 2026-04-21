import React from 'react';
import { getTokens, type Theme } from '../tokens.js';
import { I } from '../icons.js';

interface NCCheckboxProps {
  on?: boolean;
  onChange?: (value: boolean) => void;
  theme?: Theme;
}

export const NCCheckbox: React.FC<NCCheckboxProps> = ({ on, onChange, theme = 'dark' }) => {
  const t = getTokens(theme);
  return (
    <button
      onClick={() => onChange?.(!on)}
      style={{
        width: 16, height: 16, borderRadius: 2,
        background: on ? t.lime : 'transparent',
        border: `1px solid ${on ? t.lime : t.borderStrong}`,
        cursor: 'pointer', padding: 0,
        display: 'flex', alignItems: 'center', justifyContent: 'center',
        flexShrink: 0,
      }}
    >
      {on && <I.check size={10} color={theme === 'dark' ? '#0A0A0A' : '#FFFFFF'} />}
    </button>
  );
};
