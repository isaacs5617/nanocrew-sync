import React from 'react';
import { getTokens, NC_FONT_MONO, type Theme } from '../tokens.js';

interface NCLabelProps {
  children?: React.ReactNode;
  theme?: Theme;
  style?: React.CSSProperties;
}

export const NCLabel: React.FC<NCLabelProps> = ({ children, theme = 'dark', style }) => {
  const t = getTokens(theme);
  return (
    <div style={{
      fontFamily: NC_FONT_MONO, fontSize: 9, fontWeight: 500,
      letterSpacing: 2, textTransform: 'uppercase',
      color: t.textMd, marginBottom: 8,
      ...style,
    }}>
      {children}
    </div>
  );
};
