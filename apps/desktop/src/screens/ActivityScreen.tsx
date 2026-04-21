import React from 'react';
import { useTranslation } from 'react-i18next';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import {
  getTokens, NC_FONT_MONO, NC_FONT_UI,
  NCBtn, TopBar,
  type Theme,
} from '@nanocrew/ui';
import { I } from '@nanocrew/ui';
import { useAuth } from '../context/auth.js';

/** One row as returned by the `list_activity` Tauri command. */
interface ActivityEntry {
  id: number;
  ts: number;         // unix seconds
  kind: string;       // auth | drive | mount | file | system | error
  action: string;     // sign_in, mount, unmount, ...
  severity: string;   // info | warn | error
  drive_id?: number | null;
  actor?: string | null;
  target?: string | null;
  message?: string | null;
}

/** Kinds shown in the filter row, in display order. `null` = "all". */
const KINDS: { key: string | null; label: string }[] = [
  { key: null,     label: 'ALL' },
  { key: 'mount',  label: 'MOUNT' },
  { key: 'drive',  label: 'DRIVE' },
  { key: 'auth',   label: 'AUTH' },
  { key: 'file',   label: 'FILE' },
  { key: 'system', label: 'SYSTEM' },
];

function severityColor(sev: string, t: ReturnType<typeof getTokens>) {
  switch (sev) {
    case 'error': return t.danger;
    case 'warn':  return t.textHi;
    default:      return t.lime;
  }
}

function actionLabel(action: string) {
  return action.replace(/_/g, ' ').toUpperCase();
}

function formatTime(ts: number) {
  const d = new Date(ts * 1000);
  return d.toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit', second: '2-digit' });
}
function formatDate(ts: number) {
  const d = new Date(ts * 1000);
  return d.toLocaleDateString(undefined, { month: 'short', day: '2-digit' });
}

/** RFC 4180 field escape — wraps fields that contain ,"\r\n in quotes. */
function csvField(s: string): string {
  if (/[,"\r\n]/.test(s)) return `"${s.replace(/"/g, '""')}"`;
  return s;
}

/** Build a CSV dump from the rows currently visible in the UI. */
function buildCsv(rows: ActivityEntry[]): string {
  const header = 'id,ts,iso_time,kind,action,severity,drive_id,actor,target,message\n';
  const body = rows.map(r => [
    r.id,
    r.ts,
    csvField(new Date(r.ts * 1000).toISOString()),
    csvField(r.kind),
    csvField(r.action),
    csvField(r.severity),
    r.drive_id ?? '',
    csvField(r.actor ?? ''),
    csvField(r.target ?? ''),
    csvField(r.message ?? ''),
  ].join(',')).join('\n');
  return header + body + '\n';
}

/** Trigger a browser download of `text` as `filename` via a transient <a>. */
function downloadCsv(filename: string, text: string) {
  const blob = new Blob([text], { type: 'text/csv;charset=utf-8' });
  const url = URL.createObjectURL(blob);
  const a = document.createElement('a');
  a.href = url;
  a.download = filename;
  document.body.appendChild(a);
  a.click();
  document.body.removeChild(a);
  setTimeout(() => URL.revokeObjectURL(url), 0);
}

interface ActivityScreenProps { theme: Theme }

export const ActivityScreen: React.FC<ActivityScreenProps> = ({ theme }) => {
  const t = getTokens(theme);
  const { t: tr } = useTranslation();
  const { token } = useAuth();

  const [entries, setEntries] = React.useState<ActivityEntry[]>([]);
  const [kind, setKind] = React.useState<string | null>(null);
  const [errorsOnly, setErrorsOnly] = React.useState(false);
  const [search, setSearch] = React.useState('');
  const [loading, setLoading] = React.useState(true);

  // Load from backend whenever filters change. Kind/severity filtering could
  // happen client-side too, but pushing it to SQL keeps the fetch cheap even
  // when the log gets large.
  const reload = React.useCallback(() => {
    setLoading(true);
    invoke<ActivityEntry[]>('list_activity', {
      token,
      kinds: kind ? [kind] : null,
      severity: errorsOnly ? 'error' : null,
      since: null,
      limit: 1000,
    })
      .then(rows => setEntries(rows))
      .catch(() => setEntries([]))
      .finally(() => setLoading(false));
  }, [token, kind, errorsOnly]);

  React.useEffect(() => { reload(); }, [reload]);

  // Live updates via Tauri event. We ignore entries that don't match the
  // current filter so the UI doesn't flicker with unrelated rows.
  React.useEffect(() => {
    const unlisten = listen<ActivityEntry>('activity_appended', e => {
      const row = e.payload;
      if (kind && row.kind !== kind) return;
      if (errorsOnly && row.severity !== 'error') return;
      setEntries(prev => [row, ...prev].slice(0, 1000));
    });
    return () => { unlisten.then(fn => fn()); };
  }, [kind, errorsOnly]);

  // Free-text filter runs client-side across action/actor/target/message.
  const visible = React.useMemo(() => {
    const q = search.trim().toLowerCase();
    if (!q) return entries;
    return entries.filter(e =>
      e.action.toLowerCase().includes(q) ||
      (e.actor ?? '').toLowerCase().includes(q) ||
      (e.target ?? '').toLowerCase().includes(q) ||
      (e.message ?? '').toLowerCase().includes(q),
    );
  }, [entries, search]);

  const onClear = async () => {
    if (!confirm('Clear the entire activity log? This cannot be undone.')) return;
    try {
      await invoke('clear_activity', { token });
      setEntries([]);
    } catch {/* ignore */}
  };

  const onExport = () => {
    const csv = buildCsv(visible);
    const stamp = new Date().toISOString().replace(/[:.]/g, '-').slice(0, 19);
    downloadCsv(`nanocrew-activity-${stamp}.csv`, csv);
  };

  const errorCount = entries.filter(e => e.severity === 'error').length;

  return (
    <>
      <TopBar
        theme={theme}
        crumbs={[tr('activity.title')]}
        title={<>Drive <span style={{ color: t.lime }}>{tr('activity.title')}</span></>}
        subtitle={tr('activity.subtitle')}
        actions={
          <div style={{ display: 'flex', gap: 8 }}>
            <NCBtn theme={theme} small ghost iconLeft={<I.download size={13} />}
                   onClick={onExport} disabled={visible.length === 0}>
              {tr('activity.exportCsv')}
            </NCBtn>
            <NCBtn theme={theme} small ghost iconLeft={<I.x size={13} />} onClick={onClear}>
              {tr('activity.clearLog')}
            </NCBtn>
          </div>
        }
      />

      <div style={{ flex: 1, display: 'flex', flexDirection: 'column', overflow: 'hidden' }}>
        {/* Filter bar */}
        <div style={{
          display: 'flex', alignItems: 'center', gap: 14,
          padding: '10px 20px', borderBottom: `1px solid ${t.border}`,
          background: t.surface1, flexShrink: 0,
          fontFamily: NC_FONT_MONO, fontSize: 10, letterSpacing: 1,
          flexWrap: 'wrap',
        }}>
          {KINDS.map(k => {
            const active = kind === k.key;
            return (
              <button
                key={k.label}
                onClick={() => setKind(k.key)}
                style={{
                  background: active ? t.lime : 'transparent',
                  color: active ? t.bg : t.textMd,
                  border: `1px solid ${active ? t.lime : t.border}`,
                  padding: '4px 10px', borderRadius: 3, cursor: 'pointer',
                  fontFamily: NC_FONT_MONO, fontSize: 10, letterSpacing: 1.2,
                }}
              >{k.label}</button>
            );
          })}

          <span style={{ color: t.textFaint }}>|</span>

          <label style={{ display: 'flex', alignItems: 'center', gap: 6, cursor: 'pointer' }}>
            <input
              type="checkbox"
              checked={errorsOnly}
              onChange={e => setErrorsOnly(e.target.checked)}
            />
            <span style={{ color: errorsOnly ? t.danger : t.textMd }}>
              ERRORS ONLY {errorCount > 0 && <span style={{ color: t.danger }}>({errorCount})</span>}
            </span>
          </label>

          <span style={{ color: t.textFaint }}>|</span>

          <input
            type="search"
            placeholder="Filter…"
            value={search}
            onChange={e => setSearch(e.target.value)}
            style={{
              flex: 1, minWidth: 140, maxWidth: 320,
              padding: '4px 8px', borderRadius: 3,
              background: t.bg, color: t.textHi,
              border: `1px solid ${t.border}`,
              fontFamily: NC_FONT_MONO, fontSize: 11,
            }}
          />

          <span style={{ color: t.textFaint, marginLeft: 'auto' }}>
            {visible.length}/{entries.length}
          </span>
        </div>

        {/* Table */}
        <div style={{ flex: 1, overflow: 'auto' }}>
          {loading ? (
            <div style={{
              padding: 60, textAlign: 'center',
              color: t.textMd, fontSize: 13, fontFamily: NC_FONT_UI,
            }}>Loading…</div>
          ) : visible.length === 0 ? (
            <div style={{
              flex: 1, display: 'flex', flexDirection: 'column',
              alignItems: 'center', justifyContent: 'center', gap: 12,
              color: t.textMd, fontSize: 13, padding: 60,
            }}>
              <I.cloud size={36} color={t.textLo} />
              <div>No events match the current filter.</div>
            </div>
          ) : (
            <>
              {/* Header */}
              <div style={{
                display: 'grid',
                gridTemplateColumns: '110px 70px 140px 120px 1fr',
                gap: 14, padding: '9px 20px',
                borderBottom: `1px solid ${t.border}`,
                fontFamily: NC_FONT_MONO, fontSize: 9, letterSpacing: 1.5,
                color: t.textMd, textTransform: 'uppercase',
                position: 'sticky', top: 0, background: t.bg, zIndex: 1,
              }}>
                <span>Time</span>
                <span>Kind</span>
                <span>Action</span>
                <span>Target</span>
                <span>Detail</span>
              </div>

              {visible.map(e => (
                <div
                  key={e.id}
                  style={{
                    display: 'grid',
                    gridTemplateColumns: '110px 70px 140px 120px 1fr',
                    gap: 14, padding: '9px 20px', alignItems: 'center',
                    borderBottom: `1px solid ${t.border}`,
                  }}
                  title={e.message ?? ''}
                >
                  <div style={{ fontFamily: NC_FONT_MONO, fontSize: 10, color: t.textFaint }}>
                    <span style={{ color: t.textMd }}>{formatDate(e.ts)}</span>{' '}
                    {formatTime(e.ts)}
                  </div>
                  <div style={{
                    fontFamily: NC_FONT_MONO, fontSize: 9, letterSpacing: 1.5,
                    color: t.textMd, textTransform: 'uppercase',
                  }}>
                    {e.kind}
                  </div>
                  <div style={{
                    fontFamily: NC_FONT_MONO, fontSize: 9, letterSpacing: 1.5,
                    color: severityColor(e.severity, t),
                  }}>
                    {actionLabel(e.action)}
                  </div>
                  <div style={{ fontFamily: NC_FONT_UI, fontSize: 12, color: t.textHi,
                    whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>
                    {e.target ?? '—'}
                  </div>
                  <div style={{ fontFamily: NC_FONT_MONO, fontSize: 11, color: t.textMd,
                    whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>
                    {e.message ?? (e.actor ? `by ${e.actor}` : '—')}
                  </div>
                </div>
              ))}
            </>
          )}
        </div>
      </div>
    </>
  );
};
