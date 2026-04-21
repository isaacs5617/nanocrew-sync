import React from 'react';
import { getTokens, type Theme } from '../tokens.js';

export type DriveStatus = 'mounted' | 'syncing' | 'error' | 'offline';

interface NCStatusDotProps {
  state: DriveStatus;
  theme?: Theme;
}

export const NCStatusDot: React.FC<NCStatusDotProps> = ({ state, theme = 'dark' }) => {
  const t = getTokens(theme);
  const map: Record<DriveStatus, { fg: string; pulse: boolean }> = {
    mounted: { fg: t.lime, pulse: false },
    syncing: { fg: t.lime, pulse: true },
    error:   { fg: t.danger, pulse: false },
    offline: { fg: t.textLo, pulse: false },
  };
  const c = map[state] ?? map.offline;
  return (
    <span style={{ position: 'relative', display: 'inline-flex' }}>
      <span style={{ width: 8, height: 8, borderRadius: '50%', background: c.fg }} />
      {c.pulse && (
        <span style={{
          position: 'absolute', inset: 0, borderRadius: '50%',
          background: c.fg, opacity: 0.5,
          animation: 'ncPulse 1.4s ease-out infinite',
        }} />
      )}
    </span>
  );
};
