import React from 'react';
import { getTokens, NC_FONT_MONO, type Theme } from '../tokens.js';

interface NCEyebrowProps {
  children?: React.ReactNode;
  theme?: Theme;
  accent?: boolean;
  style?: React.CSSProperties;
}

export const NCEyebrow: React.FC<NCEyebrowProps> = ({ children, theme = 'dark', accent, style }) => {
  const t = getTokens(theme);
  return (
    <div style={{
      fontFamily: NC_FONT_MONO, fontSize: 10, fontWeight: 500,
      letterSpacing: 2, textTransform: 'uppercase',
      color: accent ? t.lime : t.textMd,
      ...style,
    }}>
      {children}
    </div>
  );
};
