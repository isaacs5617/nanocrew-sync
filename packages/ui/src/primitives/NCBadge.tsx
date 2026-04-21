import React from 'react';
import { getTokens, NC_FONT_MONO, type Theme } from '../tokens.js';

type BadgeColor = 'lime' | 'muted' | 'danger' | 'warn';

interface NCBadgeProps {
  children?: React.ReactNode;
  color?: BadgeColor;
  theme?: Theme;
  style?: React.CSSProperties;
}

export const NCBadge: React.FC<NCBadgeProps> = ({ children, color = 'muted', theme = 'dark', style }) => {
  const t = getTokens(theme);
  const map: Record<BadgeColor, { bg: string; fg: string; br: string }> = {
    lime:   { bg: t.limeSoft, fg: t.lime, br: t.lime },
    muted:  { bg: t.surface2, fg: t.textMd, br: t.border },
    danger: { bg: 'transparent', fg: t.danger, br: t.danger },
    warn:   { bg: 'transparent', fg: t.warn, br: t.warn },
  };
  const c = map[color];
  return (
    <span style={{
      display: 'inline-flex', alignItems: 'center', gap: 5,
      padding: '2px 7px', background: c.bg,
      border: `1px solid ${c.br}`, borderRadius: 2,
      fontFamily: NC_FONT_MONO, fontSize: 9, fontWeight: 500,
      letterSpacing: 1.5, textTransform: 'uppercase',
      color: c.fg, ...style,
    }}>
      {children}
    </span>
  );
};
