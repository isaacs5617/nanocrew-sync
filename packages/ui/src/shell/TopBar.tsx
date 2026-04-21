import React from 'react';
import { getTokens, NC_FONT_DISPLAY, NC_FONT_MONO, NC_FONT_UI, type Theme } from '../tokens.js';

interface TopBarProps {
  theme?: Theme;
  title: React.ReactNode;
  subtitle?: string;
  actions?: React.ReactNode;
  crumbs?: string[];
}

export const TopBar: React.FC<TopBarProps> = ({ theme = 'dark', title, subtitle, actions, crumbs }) => {
  const t = getTokens(theme);
  return (
    <div style={{
      padding: '20px 28px 18px', borderBottom: `1px solid ${t.border}`,
      display: 'flex', alignItems: 'flex-end', gap: 20,
      background: t.bg, flexShrink: 0,
    }}>
      <div style={{ flex: 1, minWidth: 0 }}>
        {crumbs && (
          <div style={{
            fontFamily: NC_FONT_MONO, fontSize: 10, color: t.textMd,
            letterSpacing: 1.5, marginBottom: 8, display: 'flex', gap: 6, alignItems: 'center',
          }}>
            {crumbs.map((c, i) => (
              <React.Fragment key={i}>
                <span style={{ color: i === crumbs.length - 1 ? t.textHi : t.textMd }}>{c}</span>
                {i < crumbs.length - 1 && (
                  <span style={{ color: t.textFaint }}>/</span>
                )}
              </React.Fragment>
            ))}
          </div>
        )}
        <div style={{
          fontFamily: NC_FONT_DISPLAY, fontWeight: 800,
          fontSize: 28, letterSpacing: -0.8, color: t.textHi,
          lineHeight: 1, marginBottom: subtitle ? 8 : 0,
        }}>
          {title}
        </div>
        {subtitle && (
          <div style={{ fontFamily: NC_FONT_UI, fontSize: 13, color: t.textMd, lineHeight: 1.5 }}>
            {subtitle}
          </div>
        )}
      </div>
      {actions && <div style={{ display: 'flex', gap: 8 }}>{actions}</div>}
    </div>
  );
};
