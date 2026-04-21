import React from 'react';
import { getTokens, NC_FONT_UI, NC_FONT_MONO, type Theme } from '../tokens.js';

interface NCNavRowProps {
  icon?: React.ReactElement;
  children?: React.ReactNode;
  active?: boolean;
  theme?: Theme;
  onClick?: () => void;
  badge?: string;
  accent?: boolean;
}

export const NCNavRow: React.FC<NCNavRowProps> = ({
  icon, children, active, theme = 'dark', onClick, badge, accent,
}) => {
  const t = getTokens(theme);
  return (
    <div
      onClick={onClick}
      style={{
        display: 'flex', alignItems: 'center', gap: 10,
        padding: '8px 12px', cursor: 'pointer',
        background: active ? t.surface2 : 'transparent',
        color: active ? t.textHi : t.textMd,
        borderLeft: `2px solid ${active ? t.lime : 'transparent'}`,
        fontFamily: NC_FONT_UI, fontSize: 13, fontWeight: active ? 500 : 400,
        letterSpacing: -0.1,
      }}
    >
      {icon && React.cloneElement(icon, {
        size: 15,
        color: active ? (accent ? t.lime : t.textHi) : t.textMd,
      })}
      <span style={{ flex: 1 }}>{children}</span>
      {badge && (
        <span style={{
          fontFamily: NC_FONT_MONO, fontSize: 9, fontWeight: 500,
          color: t.lime, letterSpacing: 1,
        }}>
          {badge}
        </span>
      )}
    </div>
  );
};
