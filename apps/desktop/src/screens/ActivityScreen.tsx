import React from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import {
  getTokens, NC_FONT_MONO, NC_FONT_UI,
  NCBtn, TopBar,
  type Theme,
} from '@nanocrew/ui';
import { I } from '@nanocrew/ui';
import { useAuth } from '../context/auth.js';

interface ActivityEntry {
  id: number;
  time: Date;
  driveId: number;
  driveName: string;
  status: string;
  message?: string;
}

interface Drive {
  id: number;
  name: string;
  letter: string;
  status: string;
}

function statusColor(status: string, t: ReturnType<typeof getTokens>) {
  switch (status) {
    case 'mounted':  return t.lime;
    case 'mounting': return t.textMd;
    case 'offline':  return t.textLo;
    case 'error':    return t.danger;
    default:         return t.textMd;
  }
}

function statusLabel(status: string) {
  switch (status) {
    case 'mounted':  return 'MOUNTED';
    case 'mounting': return 'MOUNTING';
    case 'offline':  return 'OFFLINE';
    case 'error':    return 'ERROR';
    default:         return status.toUpperCase();
  }
}

function formatTime(d: Date) {
  return d.toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit', second: '2-digit' });
}

interface ActivityScreenProps { theme: Theme }

export const ActivityScreen: React.FC<ActivityScreenProps> = ({ theme }) => {
  const t = getTokens(theme);
  const { token } = useAuth();

  const [drives, setDrives] = React.useState<Drive[]>([]);
  const [log, setLog] = React.useState<ActivityEntry[]>([]);
  const entryId = React.useRef(0);
  const driveMapRef = React.useRef<Map<number, string>>(new Map());

  // Load drives on mount for name lookup
  React.useEffect(() => {
    invoke<Drive[]>('list_drives', { token })
      .then(all => {
        setDrives(all);
        driveMapRef.current = new Map(all.map(d => [d.id, `${d.letter} ${d.name}`]));
        // Seed log with current mount states
        const now = new Date();
        const initial: ActivityEntry[] = all.map(d => ({
          id: entryId.current++,
          time: now,
          driveId: d.id,
          driveName: `${d.letter} ${d.name}`,
          status: d.status,
        }));
        setLog(initial.reverse());
      })
      .catch(() => {});
  }, [token]);

  // Listen for live drive status events
  React.useEffect(() => {
    const unlisten = listen<{ drive_id: number; status: string; message?: string }>(
      'drive_status_changed',
      e => {
        const { drive_id, status, message } = e.payload;
        const driveName = driveMapRef.current.get(drive_id) ?? `Drive #${drive_id}`;

        // Update drives list
        setDrives(prev => prev.map(d =>
          d.id === drive_id ? { ...d, status } : d
        ));

        // Append log entry
        setLog(prev => [{
          id: entryId.current++,
          time: new Date(),
          driveId: drive_id,
          driveName,
          status,
          message,
        }, ...prev].slice(0, 200));
      }
    );
    return () => { unlisten.then(fn => fn()); };
  }, []);

  const mounted = drives.filter(d => d.status === 'mounted');
  const errors  = drives.filter(d => d.status === 'error');

  return (
    <>
      <TopBar
        theme={theme}
        crumbs={['Activity']}
        title={<>Drive <span style={{ color: t.lime }}>Activity</span></>}
        subtitle="Live mount events and drive status"
        actions={
          <NCBtn theme={theme} small ghost onClick={() => setLog([])}>
            Clear log
          </NCBtn>
        }
      />

      <div style={{ flex: 1, display: 'flex', flexDirection: 'column', overflow: 'hidden' }}>
        {/* Status summary bar */}
        <div style={{
          display: 'flex', alignItems: 'center', gap: 20,
          padding: '10px 20px', borderBottom: `1px solid ${t.border}`,
          background: t.surface1, flexShrink: 0,
          fontFamily: NC_FONT_MONO, fontSize: 10, letterSpacing: 1,
        }}>
          <span>
            <span style={{ color: t.lime }}>{mounted.length}</span>
            <span style={{ color: t.textLo }}> MOUNTED</span>
          </span>
          {errors.length > 0 && (
            <span>
              <span style={{ color: t.danger }}>{errors.length}</span>
              <span style={{ color: t.textLo }}> ERROR{errors.length !== 1 ? 'S' : ''}</span>
            </span>
          )}
          <span style={{ color: t.textFaint }}>|</span>
          {drives.map(d => (
            <span key={d.id} style={{ display: 'flex', alignItems: 'center', gap: 5 }}>
              <span style={{
                display: 'inline-block', width: 6, height: 6, borderRadius: '50%',
                background: statusColor(d.status, t),
              }} />
              <span style={{ color: t.textMd }}>{d.letter}</span>
            </span>
          ))}
          {drives.length === 0 && (
            <span style={{ color: t.textFaint }}>No drives configured</span>
          )}
        </div>

        {/* Event log */}
        <div style={{ flex: 1, overflow: 'auto' }}>
          {log.length === 0 ? (
            <div style={{
              flex: 1, display: 'flex', flexDirection: 'column',
              alignItems: 'center', justifyContent: 'center', gap: 12,
              color: t.textMd, fontSize: 13, padding: 60,
            }}>
              <I.cloud size={36} color={t.textLo} />
              <div>No events yet. Mount or unmount a drive to see activity.</div>
            </div>
          ) : (
            <>
              {/* Header */}
              <div style={{
                display: 'grid',
                gridTemplateColumns: '90px 180px 90px 1fr',
                gap: 14, padding: '9px 20px',
                borderBottom: `1px solid ${t.border}`,
                fontFamily: NC_FONT_MONO, fontSize: 9, letterSpacing: 1.5,
                color: t.textMd, textTransform: 'uppercase',
                position: 'sticky', top: 0, background: t.bg, zIndex: 1,
              }}>
                <span>Time</span>
                <span>Drive</span>
                <span>Status</span>
                <span>Detail</span>
              </div>

              {log.map(entry => (
                <div
                  key={entry.id}
                  style={{
                    display: 'grid',
                    gridTemplateColumns: '90px 180px 90px 1fr',
                    gap: 14, padding: '9px 20px', alignItems: 'center',
                    borderBottom: `1px solid ${t.border}`,
                  }}
                >
                  <div style={{ fontFamily: NC_FONT_MONO, fontSize: 10, color: t.textFaint }}>
                    {formatTime(entry.time)}
                  </div>
                  <div style={{ fontFamily: NC_FONT_UI, fontSize: 13, color: t.textHi,
                    whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>
                    {entry.driveName}
                  </div>
                  <div style={{
                    fontFamily: NC_FONT_MONO, fontSize: 9, letterSpacing: 1.5,
                    color: statusColor(entry.status, t),
                  }}>
                    {statusLabel(entry.status)}
                  </div>
                  <div style={{ fontFamily: NC_FONT_MONO, fontSize: 11, color: t.textMd,
                    whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>
                    {entry.message ?? '—'}
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
