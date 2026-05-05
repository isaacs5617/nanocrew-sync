import React from 'react';
import { check } from '@tauri-apps/plugin-updater';
import { relaunch } from '@tauri-apps/plugin-process';
import { invoke } from '@tauri-apps/api/core';
import {
  getTokens, NC_FONT_MONO,
  NCBtn,
  type Theme,
} from '@nanocrew/ui';
import { useAuth } from '../context/auth.js';

const RELEASES_URL = 'https://github.com/isaacs5617/nanocrew-sync/releases/latest';

type UpdateState =
  | { kind: 'idle' }
  | { kind: 'checking' }
  | { kind: 'none' }
  | { kind: 'available'; version: string; notes?: string }
  | { kind: 'downloading'; downloaded: number; total: number; version: string }
  | { kind: 'ready' }
  | { kind: 'error'; message: string };

function formatBytes(b: number) {
  if (b < 1024) return `${b} B`;
  if (b < 1048576) return `${(b / 1024).toFixed(1)} KB`;
  if (b < 1073741824) return `${(b / 1048576).toFixed(1)} MB`;
  return `${(b / 1073741824).toFixed(2)} GB`;
}

export const UpdateButton: React.FC<{ theme: Theme }> = ({ theme }) => {
  const t = getTokens(theme);
  const { token } = useAuth();
  const [state, setState] = React.useState<UpdateState>({ kind: 'idle' });

  const check_ = React.useCallback(async () => {
    setState({ kind: 'checking' });
    try {
      const update = await check();
      if (!update) {
        setState({ kind: 'none' });
        return;
      }

      let downloaded = 0;
      let total = 0;
      setState({ kind: 'available', version: update.version, notes: update.body });
      await update.downloadAndInstall(ev => {
        switch (ev.event) {
          case 'Started':
            total = ev.data.contentLength ?? 0;
            setState({ kind: 'downloading', downloaded: 0, total, version: update.version });
            break;
          case 'Progress':
            downloaded += ev.data.chunkLength;
            setState({ kind: 'downloading', downloaded, total, version: update.version });
            break;
          case 'Finished':
            setState({ kind: 'ready' });
            break;
        }
      });

      // On Windows NSIS passive mode the installer has been launched — exit so
      // it can replace the binary, then the installer restarts the app.
      await relaunch();
    } catch (e: any) {
      setState({ kind: 'error', message: e?.message ?? String(e) });
    }
  }, []);

  const openReleases = () =>
    invoke('open_path', { token, path: RELEASES_URL }).catch(() => {});

  const label = (() => {
    switch (state.kind) {
      case 'idle':        return 'Check for updates';
      case 'checking':    return 'Checking…';
      case 'none':        return 'Check again';
      case 'available':   return `Updating to ${state.version}…`;
      case 'downloading': {
        const pct = state.total > 0 ? Math.round((state.downloaded / state.total) * 100) : 0;
        return `Downloading ${pct}%`;
      }
      case 'ready':       return 'Restarting…';
      case 'error':       return 'Retry';
    }
  })();

  const busy = state.kind === 'checking' || state.kind === 'available'
    || state.kind === 'downloading' || state.kind === 'ready';

  const subtitle = (() => {
    switch (state.kind) {
      case 'idle':
        return 'Check if a newer signed build is available.';
      case 'checking':
        return 'Contacting update server…';
      case 'none':
        return <span style={{ color: t.lime }}>You're on the latest version.</span>;
      case 'available':
        return (
          <span>
            New version <strong style={{ color: t.textHi }}>{state.version}</strong> found — downloading…
          </span>
        );
      case 'downloading':
        return (
          <span style={{ fontFamily: NC_FONT_MONO }}>
            {formatBytes(state.downloaded)}
            {state.total > 0 ? ` / ${formatBytes(state.total)}` : ''}
          </span>
        );
      case 'ready':
        return 'Update installed — relaunching.';
      case 'error':
        return <span style={{ color: t.danger }}>{state.message}</span>;
    }
  })();

  // Show the manual-download escape hatch once the auto-updater has
  // confirmed "up to date" or failed — gives users a way out.
  const showFallback = state.kind === 'none' || state.kind === 'error';

  return (
    <div style={{ marginTop: 14 }}>
      <div style={{ display: 'flex', alignItems: 'center', gap: 14 }}>
        <div style={{ flex: 1 }}>
          <div style={{ fontSize: 13, color: t.textHi, fontWeight: 500 }}>Application updates</div>
          <div style={{ fontSize: 11, color: t.textMd, marginTop: 2 }}>{subtitle}</div>
        </div>
        <NCBtn theme={theme} small ghost onClick={check_} disabled={busy}>{label}</NCBtn>
      </div>
      {showFallback && (
        <div style={{ marginTop: 6, fontSize: 11, color: t.textLo }}>
          Update not working?{' '}
          <span
            onClick={openReleases}
            style={{ color: t.lime, cursor: 'pointer', textDecoration: 'underline' }}
          >
            Download manually from GitHub
          </span>
        </div>
      )}
    </div>
  );
};
