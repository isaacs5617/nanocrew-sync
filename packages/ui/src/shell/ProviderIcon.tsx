import React from 'react';
import { getTokens, NC_FONT_MONO, type Theme } from '../tokens.js';

interface ProviderIconProps {
  id: string;
  size?: number;
  theme?: Theme;
}

export const ProviderIcon: React.FC<ProviderIconProps> = ({ id, size = 20, theme = 'dark' }) => {
  const t = getTokens(theme);
  const map: Record<string, { mono: string; color: string }> = {
    wasabi:   { mono: 'W', color: t.lime },
    s3:       { mono: 'S3', color: t.textHi },
    b2:       { mono: 'B2', color: t.textHi },
    gdrive:   { mono: 'G', color: t.textHi },
    onedrive: { mono: '1', color: t.textHi },
    dropbox:  { mono: 'D', color: t.textHi },
    sftp:     { mono: '~', color: t.textHi },
    webdav:   { mono: 'W', color: t.textHi },
  };
  const p = map[id] ?? map['s3']!;
  return (
    <div style={{
      width: size + 8, height: size + 8, borderRadius: 3,
      border: `1px solid ${id === 'wasabi' ? t.lime : t.border}`,
      background: id === 'wasabi' ? t.limeSoft : t.surface2,
      display: 'flex', alignItems: 'center', justifyContent: 'center',
      flexShrink: 0,
      fontFamily: NC_FONT_MONO, fontSize: size * 0.55, fontWeight: 500,
      color: p.color, letterSpacing: -0.5,
    }}>
      {p.mono}
    </div>
  );
};
