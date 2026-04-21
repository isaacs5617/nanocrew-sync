import React from 'react';
import { check } from '@tauri-apps/plugin-updater';
import { relaunch } from '@tauri-apps/plugin-process';
import {
  getTokens, NC_FONT_MONO,
  NCBtn,
  type Theme,
} from '@nanocrew/ui';

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
  const [state, setState] = React.useState<UpdateState>({ kind: 'idle' });

  const check_ = React.useCallback(async () => {
    setState({ kind: 'checking' });
    try {
      const update = await check();
      if (!update) {
        setState({ kind: 'none' });
        return;
      }
      setState({ kind: 'available', version: update.version, notes: update.body });

      // Begin download + install immediately — user already clicked "check"
      let downloaded = 0;
      let total = 0;
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

      // Installer finished — on Windows NSIS 'passive' mode, it has already
      // launched the installer in the background. Relaunch the app to pick up
      // the new binary once the installer completes.
      await relaunch();
    } catch (e: any) {
      setState({ kind: 'error', message: e?.message ?? String(e) });
    }
  }, []);

  const label = (() => {
    switch (state.kind) {
      case 'idle':        return 'Check for updates';
      case 'checking':    return 'Checking…';
      case 'none':        return 'Up to date';
      case 'available':   return `Updating to ${state.version}…`;
      case 'downloading': {
        const pct = state.total > 0 ? Math.round((state.downloaded / state.total) * 100) : 0;
        return `Downloading ${pct}%`;
      }
      case 'ready':       return 'Restarting…';
      case 'error':       return 'Retry';
    }
  })();

  const busy = state.kind === 'checking' || state.kind === 'available' || state.kind === 'downloading' || state.kind === 'ready';

  return (
    <div style={{ display: 'flex', alignItems: 'center', gap: 14, marginTop: 14 }}>
      <div style={{ flex: 1 }}>
        <div style={{ fontSize: 13, color: t.textHi, fontWeight: 500 }}>Application updates</div>
        <div style={{ fontSize: 11, color: t.textMd, marginTop: 2 }}>
          {state.kind === 'none'     && <span style={{ color: t.lime }}>You're on the latest version.</span>}
          {state.kind === 'error'    && <span style={{ color: t.danger }}>{state.message}</span>}
          {state.kind === 'downloading' && (
            <span style={{ fontFamily: NC_FONT_MONO }}>
              {formatBytes(state.downloaded)}
              {state.total > 0 ? ` / ${formatBytes(state.total)}` : ''}
            </span>
          )}
          {state.kind === 'idle' && 'Check if a newer signed build is available.'}
          {state.kind === 'checking' && 'Contacting update server…'}
          {state.kind === 'available' && `New version ${state.version} found, installing…`}
          {state.kind === 'ready' && 'Update installed — relaunching.'}
        </div>
      </div>
      <NCBtn theme={theme} small ghost onClick={check_} disabled={busy}>{label}</NCBtn>
    </div>
  );
};
