import React from 'react';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { getTokens, NC_FONT_UI, type Theme } from '@nanocrew/ui';
import { NCMark } from '@nanocrew/ui';

interface TitleBarProps {
  theme: Theme;
  title?: string;
}

const appWindow = getCurrentWindow();

const WinBtn: React.FC<{
  onClick: () => void;
  isClose?: boolean;
  theme: Theme;
  children: React.ReactNode;
}> = ({ onClick, isClose, theme, children }) => {
  const t = getTokens(theme);
  const [hovered, setHovered] = React.useState(false);
  return (
    <div
      onClick={onClick}
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
      style={{
        width: 46, height: 36,
        display: 'flex', alignItems: 'center', justifyContent: 'center',
        color: t.textMd,
        background: hovered ? (isClose ? '#C42B1C' : t.surface3) : 'transparent',
        cursor: 'pointer',
        transition: 'background 80ms',
      }}
    >
      {children}
    </div>
  );
};

export const TitleBar: React.FC<TitleBarProps> = ({ theme, title = 'NanoCrew Sync' }) => {
  const t = getTokens(theme);
  return (
    <div
      data-tauri-drag-region
      style={{
        height: 36,
        display: 'flex', alignItems: 'center', justifyContent: 'space-between',
        padding: '0 0 0 12px',
        background: t.surface1, borderBottom: `1px solid ${t.border}`,
        flexShrink: 0, userSelect: 'none',
      }}
    >
      <div
        data-tauri-drag-region
        style={{ display: 'flex', alignItems: 'center', gap: 10, pointerEvents: 'none' }}
      >
        <NCMark size={16} dark={theme === 'dark'} />
        <span style={{
          fontFamily: NC_FONT_UI, fontSize: 12, fontWeight: 500,
          color: t.textHi, letterSpacing: -0.1,
        }}>
          {title}
        </span>
      </div>
      <div style={{ display: 'flex' }}>
        <WinBtn theme={theme} onClick={() => appWindow.minimize()}>
          <svg width="10" height="10" viewBox="0 0 10 10">
            <line x1="0" y1="5" x2="10" y2="5" stroke="currentColor" />
          </svg>
        </WinBtn>
        <WinBtn theme={theme} onClick={() => appWindow.toggleMaximize()}>
          <svg width="10" height="10" viewBox="0 0 10 10">
            <rect x="0.5" y="0.5" width="9" height="9" fill="none" stroke="currentColor" />
          </svg>
        </WinBtn>
        <WinBtn theme={theme} isClose onClick={() => appWindow.close()}>
          <svg width="10" height="10" viewBox="0 0 10 10">
            <line x1="0" y1="0" x2="10" y2="10" stroke="currentColor" />
            <line x1="10" y1="0" x2="0" y2="10" stroke="currentColor" />
          </svg>
        </WinBtn>
      </div>
    </div>
  );
};
