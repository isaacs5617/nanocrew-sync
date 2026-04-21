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

// ── Error aggregation ───────────────────────────────────────────────────────
//
// "Group repeated errors" — two failed mounts of drive W: with the same
// message collapse into one row showing occurrence count + first/latest time.
// The grouping key is (kind, action, target, message) so genuinely distinct
// failures still show separately. Grouping is client-side only — the DB still
// stores every row for audit fidelity and CSV export.

interface ErrorGroup {
  sig: string;              // aggregation key
  entries: ActivityEntry[]; // newest first
  first: ActivityEntry;     // oldest in the group
  latest: ActivityEntry;    // newest in the group
  count: number;
  drive_id: number | null;  // retry target (from latest)
  canRetry: boolean;        // mount failures with a drive_id
}

function groupKey(e: ActivityEntry): string {
  return [e.kind, e.action, e.target ?? '', e.message ?? ''].join('\x1f');
}

function aggregateErrors(rows: ActivityEntry[]): ErrorGroup[] {
  const map = new Map<string, ErrorGroup>();
  for (const e of rows) {
    const sig = groupKey(e);
    const g = map.get(sig);
    if (g) {
      g.entries.push(e); // rows already newest-first
      if (e.ts < g.first.ts) g.first = e;
      if (e.ts > g.latest.ts) { g.latest = e; g.drive_id = e.drive_id ?? g.drive_id; }
      g.count += 1;
    } else {
      const canRetry = e.kind === 'mount' && e.action === 'mount_failed' && !!e.drive_id;
      map.set(sig, {
        sig,
        entries: [e],
        first: e,
        latest: e,
        count: 1,
        drive_id: e.drive_id ?? null,
        canRetry,
      });
    }
  }
  // Newest activity first (by latest occurrence in each group).
  return Array.from(map.values()).sort((a, b) => b.latest.ts - a.latest.ts);
}

/** Per-user dismissed-group memory. We record the signature + the latest
 *  timestamp at dismissal time; any new occurrence with a newer ts revives
 *  the group automatically. Persisted to localStorage so the hide survives
 *  reloads without polluting the server-side audit log. */
const DISMISSED_KEY = 'nanocrew_activity_dismissed_v1';

function loadDismissed(): Record<string, number> {
  try {
    const raw = localStorage.getItem(DISMISSED_KEY);
    return raw ? JSON.parse(raw) as Record<string, number> : {};
  } catch { return {}; }
}
function saveDismissed(d: Record<string, number>) {
  try { localStorage.setItem(DISMISSED_KEY, JSON.stringify(d)); } catch {/* quota */}
}

interface ActivityScreenProps { theme: Theme }

export const ActivityScreen: React.FC<ActivityScreenProps> = ({ theme }) => {
  const t = getTokens(theme);
  const { t: tr } = useTranslation();
  const { token } = useAuth();

  const [entries, setEntries] = React.useState<ActivityEntry[]>([]);
  const [kind, setKind] = React.useState<string | null>(null);
  const [errorsOnly, setErrorsOnly] = React.useState(false);
  const [grouped, setGrouped] = React.useState(true);
  const [search, setSearch] = React.useState('');
  const [loading, setLoading] = React.useState(true);
  const [dismissed, setDismissed] = React.useState<Record<string, number>>(loadDismissed);
  const [expanded, setExpanded] = React.useState<Set<string>>(new Set());
  const [retrying, setRetrying] = React.useState<Set<string>>(new Set());

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

  // Build error groups when grouping is on AND we're looking at an error-heavy
  // slice (errorsOnly checkbox) — otherwise show the flat stream. Grouping a
  // mixed info/error feed hides routine activity without adding much value.
  const errorGroups = React.useMemo<ErrorGroup[]>(() => {
    if (!grouped || !errorsOnly) return [];
    const rows = visible.filter(e => !dismissed[groupKey(e)] || e.ts > dismissed[groupKey(e)]);
    return aggregateErrors(rows);
  }, [grouped, errorsOnly, visible, dismissed]);

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

  const onDismissGroup = (g: ErrorGroup) => {
    const next = { ...dismissed, [g.sig]: g.latest.ts };
    setDismissed(next);
    saveDismissed(next);
  };

  const onUndismissAll = () => {
    setDismissed({});
    saveDismissed({});
  };

  const onRetryGroup = async (g: ErrorGroup) => {
    if (!g.canRetry || g.drive_id == null) return;
    const key = g.sig;
    setRetrying(prev => new Set(prev).add(key));
    try {
      await invoke('mount_drive', { token, driveId: g.drive_id });
      // Hide the group once we've asked to retry — if the mount fails again
      // the fresh event's ts > dismissed-ts and the group pops back up.
      onDismissGroup(g);
    } catch {
      // The mount failure itself will arrive via activity_appended; no UI
      // action needed here beyond un-spinning the button.
    } finally {
      setRetrying(prev => { const n = new Set(prev); n.delete(key); return n; });
    }
  };

  const toggleExpand = (sig: string) => {
    setExpanded(prev => {
      const n = new Set(prev);
      if (n.has(sig)) n.delete(sig); else n.add(sig);
      return n;
    });
  };

  const errorCount = entries.filter(e => e.severity === 'error').length;
  const dismissedCount = Object.keys(dismissed).length;

  const showGrouped = grouped && errorsOnly;

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

          <label style={{
            display: 'flex', alignItems: 'center', gap: 6,
            cursor: errorsOnly ? 'pointer' : 'not-allowed',
            opacity: errorsOnly ? 1 : 0.45,
          }}>
            <input
              type="checkbox"
              checked={grouped}
              disabled={!errorsOnly}
              onChange={e => setGrouped(e.target.checked)}
            />
            <span style={{ color: t.textMd }}>GROUP REPEATS</span>
          </label>

          {dismissedCount > 0 && (
            <button
              onClick={onUndismissAll}
              title="Show previously dismissed groups"
              style={{
                background: 'transparent', color: t.textMd,
                border: `1px solid ${t.border}`,
                padding: '4px 10px', borderRadius: 3, cursor: 'pointer',
                fontFamily: NC_FONT_MONO, fontSize: 10, letterSpacing: 1.2,
              }}
            >
              UNDISMISS ({dismissedCount})
            </button>
          )}

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
            {showGrouped ? `${errorGroups.length} groups` : `${visible.length}/${entries.length}`}
          </span>
        </div>

        {/* Table */}
        <div style={{ flex: 1, overflow: 'auto' }}>
          {loading ? (
            <div style={{
              padding: 60, textAlign: 'center',
              color: t.textMd, fontSize: 13, fontFamily: NC_FONT_UI,
            }}>Loading…</div>
          ) : showGrouped ? (
            errorGroups.length === 0 ? (
              <div style={{
                flex: 1, display: 'flex', flexDirection: 'column',
                alignItems: 'center', justifyContent: 'center', gap: 12,
                color: t.textMd, fontSize: 13, padding: 60,
              }}>
                <I.check size={36} color={t.lime} />
                <div>No active errors.</div>
                {dismissedCount > 0 && (
                  <div style={{ fontSize: 11, color: t.textLo }}>
                    {dismissedCount} group{dismissedCount === 1 ? '' : 's'} dismissed — click UNDISMISS to restore.
                  </div>
                )}
              </div>
            ) : (
              <>
                {errorGroups.map(g => {
                  const open = expanded.has(g.sig);
                  const isRetrying = retrying.has(g.sig);
                  return (
                    <div key={g.sig} style={{ borderBottom: `1px solid ${t.border}` }}>
                      <div style={{
                        display: 'grid',
                        gridTemplateColumns: '60px 140px 120px 1fr auto',
                        gap: 14, padding: '11px 20px', alignItems: 'center',
                      }}>
                        <button
                          onClick={() => toggleExpand(g.sig)}
                          title={g.count > 1 ? 'Show all occurrences' : ''}
                          style={{
                            background: t.surface2, color: t.danger,
                            border: `1px solid ${t.border}`,
                            padding: '4px 8px', borderRadius: 3, cursor: g.count > 1 ? 'pointer' : 'default',
                            fontFamily: NC_FONT_MONO, fontSize: 10, letterSpacing: 1,
                          }}
                        >×{g.count}</button>
                        <div style={{
                          fontFamily: NC_FONT_MONO, fontSize: 9, letterSpacing: 1.5,
                          color: t.danger,
                        }}>
                          {actionLabel(g.latest.action)}
                        </div>
                        <div style={{ fontFamily: NC_FONT_UI, fontSize: 12, color: t.textHi,
                          whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>
                          {g.latest.target ?? '—'}
                        </div>
                        <div style={{ display: 'flex', flexDirection: 'column', gap: 2, minWidth: 0 }}>
                          <div style={{ fontFamily: NC_FONT_MONO, fontSize: 11, color: t.textMd,
                            whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}
                               title={g.latest.message ?? ''}>
                            {g.latest.message ?? '—'}
                          </div>
                          <div style={{ fontFamily: NC_FONT_MONO, fontSize: 9, color: t.textFaint, letterSpacing: 1 }}>
                            {g.count === 1
                              ? <>at {formatDate(g.latest.ts)} {formatTime(g.latest.ts)}</>
                              : <>first {formatDate(g.first.ts)} {formatTime(g.first.ts)} · latest {formatDate(g.latest.ts)} {formatTime(g.latest.ts)}</>}
                          </div>
                        </div>
                        <div style={{ display: 'flex', gap: 6 }}>
                          {g.canRetry && (
                            <NCBtn
                              theme={theme} small ghost
                              disabled={isRetrying}
                              onClick={() => onRetryGroup(g)}
                            >
                              {isRetrying ? 'Retrying…' : 'Retry'}
                            </NCBtn>
                          )}
                          <NCBtn theme={theme} small ghost onClick={() => onDismissGroup(g)}>
                            Dismiss
                          </NCBtn>
                        </div>
                      </div>

                      {open && g.count > 1 && (
                        <div style={{
                          padding: '6px 20px 10px 92px',
                          background: t.surface1,
                          fontFamily: NC_FONT_MONO, fontSize: 10, color: t.textMd,
                        }}>
                          {g.entries.map(oc => (
                            <div key={oc.id} style={{ padding: '2px 0' }}>
                              <span style={{ color: t.textFaint }}>
                                {formatDate(oc.ts)} {formatTime(oc.ts)}
                              </span>
                              {oc.actor && <span style={{ color: t.textLo }}> · {oc.actor}</span>}
                            </div>
                          ))}
                        </div>
                      )}
                    </div>
                  );
                })}
              </>
            )
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
