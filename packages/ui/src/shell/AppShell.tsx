import React from 'react';
import { getTokens, NC_FONT_MONO, type Theme } from '../tokens.js';
import { NCEyebrow } from '../primitives/NCEyebrow.js';
import { NCNavRow } from '../primitives/NCNavRow.js';
import { NCStatusDot } from '../primitives/NCStatusDot.js';
import { I } from '../icons.js';
import type { NavKey } from '../types.js';

interface AppShellProps {
  theme?: Theme;
  activeNav?: NavKey;
  children?: React.ReactNode;
  onNav?: (key: NavKey) => void;
  driveCount?: number;
  errCount?: number;
  version?: string;
}

export const AppShell: React.FC<AppShellProps> = ({
  theme = 'dark', activeNav = 'drives', children, onNav,
  driveCount = 0, errCount = 0, version = '',
}) => {
  const t = getTokens(theme);

  return (
    <>
      {/* Sidebar */}
      <div style={{
        width: 232, flexShrink: 0, background: t.surface1,
        borderRight: `1px solid ${t.border}`,
        display: 'flex', flexDirection: 'column',
      }}>
        {/* v0.2.16: removed the "Workspace" chip. It was a decorative mock
            with no click handler, no dropdown, no state — the product has
            no workspace concept (drives are per-machine). Deleted rather
            than wired because there's nothing meaningful to wire it to.
            The Account section further down already covers identity. */}

        <div style={{ padding: '8px 0', flex: 1 }}>
          <NCEyebrow theme={theme} style={{ padding: '12px 16px 8px' }}>Navigate</NCEyebrow>
          <NCNavRow theme={theme} icon={<I.home />} active={activeNav === 'home'} onClick={() => onNav?.('home')}>Home</NCNavRow>
          <NCNavRow theme={theme} icon={<I.drive />} active={activeNav === 'drives'} badge={driveCount > 0 ? String(driveCount) : undefined} onClick={() => onNav?.('drives')}>Drives</NCNavRow>
          <NCNavRow theme={theme} icon={<I.folder />} active={activeNav === 'files'} onClick={() => onNav?.('files')}>File Browser</NCNavRow>
          <NCNavRow theme={theme} icon={<I.activity />} active={activeNav === 'transfers'} onClick={() => onNav?.('transfers')}>Transfers</NCNavRow>
          <NCNavRow theme={theme} icon={<I.clock />} active={activeNav === 'activity'} onClick={() => onNav?.('activity')}>Activity</NCNavRow>

          <NCEyebrow theme={theme} style={{ padding: '16px 16px 8px' }}>Account</NCEyebrow>
          <NCNavRow theme={theme} icon={<I.user />} active={activeNav === 'account'} onClick={() => onNav?.('account')}>My account</NCNavRow>
          <NCNavRow theme={theme} icon={<I.settings />} active={activeNav === 'settings'} onClick={() => onNav?.('settings')}>Settings</NCNavRow>
        </div>

        {/* Status strip */}
        <div style={{
          padding: 16, borderTop: `1px solid ${t.border}`,
          fontFamily: NC_FONT_MONO, fontSize: 10, color: t.textMd, letterSpacing: 1,
        }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 6, marginBottom: 4 }}>
            <NCStatusDot state={errCount ? 'error' : 'mounted'} theme={theme} />
            <span style={{ color: errCount ? t.danger : t.lime }}>
              {errCount
                ? `${errCount} DRIVE${errCount > 1 ? 'S' : ''} NEED ATTENTION`
                : 'ALL DRIVES HEALTHY'}
            </span>
          </div>
          <div>SYNC {version ? `v${version}` : ''} · {theme === 'dark' ? 'DARK' : 'LIGHT'}</div>
        </div>
      </div>

      {/* Main content */}
      <div style={{ flex: 1, minWidth: 0, display: 'flex', flexDirection: 'column', background: t.bg }}>
        {children}
      </div>
    </>
  );
};
