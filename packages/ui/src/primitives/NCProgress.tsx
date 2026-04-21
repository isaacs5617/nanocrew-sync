import React from 'react';
import { getTokens, type Theme } from '../tokens.js';

interface NCProgressProps {
  value?: number;
  theme?: Theme;
  style?: React.CSSProperties;
}

export const NCProgress: React.FC<NCProgressProps> = ({ value = 0, theme = 'dark', style }) => {
  const t = getTokens(theme);
  return (
    <div style={{
      height: 4, background: t.surface3, borderRadius: 2,
      overflow: 'hidden', ...style,
    }}>
      <div style={{
        height: '100%', width: `${value}%`,
        background: t.lime, transition: 'width 300ms',
      }} />
    </div>
  );
};
