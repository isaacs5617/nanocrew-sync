import React from 'react';
import { invoke } from '@tauri-apps/api/core';
import {
  getTokens, NC_FONT_MONO, NC_FONT_UI,
  NCBtn, FileIcon, TopBar,
  type Theme,
} from '@nanocrew/ui';
import { I } from '@nanocrew/ui';
import { useAuth } from '../context/auth.js';

interface Drive {
  id: number;
  name: string;
  letter: string;
  bucket: string;
  region: string;
  status: string;
}

interface S3Entry {
  name: string;
  key: string;
  is_dir: boolean;
  size: number;
  modified: number;
}

interface FileLockEntry {
  key: string;
  machine: string;
  owner: string;
  acquired_at: number;
  expires_at: number;
  is_ours: boolean;
}

function formatSize(bytes: number): string {
  if (!bytes) return '—';
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1048576) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1073741824) return `${(bytes / 1048576).toFixed(1)} MB`;
  return `${(bytes / 1073741824).toFixed(2)} GB`;
}

function formatDate(secs: number): string {
  if (!secs) return '—';
  return new Date(secs * 1000).toLocaleDateString(undefined, {
    year: 'numeric', month: 'short', day: 'numeric',
  });
}

type FileKind = 'file' | 'image' | 'folder' | 'video' | 'doc';

function inferKind(name: string, isDir: boolean): FileKind {
  if (isDir) return 'folder';
  const ext = name.split('.').pop()?.toLowerCase() ?? '';
  if (['jpg','jpeg','png','gif','webp','svg','bmp','tiff','heic'].includes(ext)) return 'image';
  if (['mp4','mov','avi','mkv','webm','m4v'].includes(ext)) return 'video';
  if (['doc','docx','odt','rtf','pdf'].includes(ext)) return 'doc';
  return 'file';
}

interface FileBrowserScreenProps {
  theme: Theme;
}

export const FileBrowserScreen: React.FC<FileBrowserScreenProps> = ({ theme }) => {
  const t = getTokens(theme);
  const { token } = useAuth();

  const [drives, setDrives] = React.useState<Drive[]>([]);
  const [selectedDrive, setSelectedDrive] = React.useState<Drive | null>(null);
  const [prefix, setPrefix] = React.useState('');
  const [entries, setEntries] = React.useState<S3Entry[]>([]);
  const [loading, setLoading] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const [refreshKey, setRefreshKey] = React.useState(0);
  const [pinned, setPinned] = React.useState<Set<string>>(new Set());
  const [locks, setLocks] = React.useState<Map<string, FileLockEntry>>(new Map());
  const [menu, setMenu] = React.useState<{ x: number; y: number; entry: S3Entry } | null>(null);

  // Load mounted drives
  React.useEffect(() => {
    invoke<Drive[]>('list_drives', { token })
      .then(all => {
        const mounted = all.filter(d => d.status === 'mounted');
        setDrives(mounted);
        if (mounted.length > 0 && !selectedDrive) {
          setSelectedDrive(mounted[0]);
        }
      })
      .catch(e => setError(String(e)));
  }, [token]);

  // Load entries when drive, prefix, or refreshKey changes
  React.useEffect(() => {
    if (!selectedDrive) return;
    setLoading(true);
    setError(null);
    invoke<S3Entry[]>('list_drive_objects', {
      token,
      driveId: selectedDrive.id,
      prefix,
    })
      .then(result => {
        const sorted = [...result].sort((a, b) => {
          if (a.is_dir !== b.is_dir) return a.is_dir ? -1 : 1;
          return a.name.localeCompare(b.name);
        });
        setEntries(sorted);
      })
      .catch(e => setError(String(e)))
      .finally(() => setLoading(false));
  }, [selectedDrive, prefix, token, refreshKey]);

  const navigateInto = (entry: S3Entry) => {
    if (entry.is_dir) {
      setPrefix(entry.key);
    }
  };

  const navigateUp = () => {
    if (!prefix) return;
    const parts = prefix.replace(/\/$/, '').split('/');
    parts.pop();
    setPrefix(parts.length > 0 ? parts.join('/') + '/' : '');
  };

  const changeDrive = (drive: Drive) => {
    setSelectedDrive(drive);
    setPrefix('');
    setEntries([]);
  };

  // Pull the drive's pin list any time the drive changes or a pin toggles.
  // Small set — cheap to fetch up-front and keep in memory.
  const reloadPinned = React.useCallback(() => {
    if (!selectedDrive) {
      setPinned(new Set());
      return;
    }
    invoke<string[]>('list_pinned_files', { token, driveId: selectedDrive.id })
      .then(keys => setPinned(new Set(keys)))
      .catch(() => setPinned(new Set()));
  }, [selectedDrive, token]);

  React.useEffect(reloadPinned, [reloadPinned, refreshKey]);

  // Fetch the drive's active sentinels so we can paint padlocks and expose
  // a "Break lock" admin action. Cheap — sentinel count ≪ file count.
  const reloadLocks = React.useCallback(() => {
    if (!selectedDrive) { setLocks(new Map()); return; }
    invoke<FileLockEntry[]>('list_file_locks', { token, driveId: selectedDrive.id })
      .then(rows => setLocks(new Map(rows.map(r => [r.key, r]))))
      .catch(() => setLocks(new Map()));
  }, [selectedDrive, token]);
  React.useEffect(reloadLocks, [reloadLocks, refreshKey]);

  const breakLock = async (entry: S3Entry) => {
    if (!selectedDrive) return;
    const info = locks.get(entry.key);
    if (!info) return;
    const who = info.is_ours ? 'this machine' : `${info.owner} on ${info.machine.slice(0, 8)}…`;
    const ok = window.confirm(
      `Force-release the lock on "${entry.name}"?\n\nHeld by: ${who}\n` +
      `Breaking an active lock may cause the other writer's upload to fail or ` +
      `leave a partial object. Only do this if you're sure the other writer has ` +
      `crashed or disconnected. This action is logged.`
    );
    if (!ok) return;
    try {
      await invoke('break_file_lock', { token, driveId: selectedDrive.id, key: entry.key });
      setLocks(prev => { const n = new Map(prev); n.delete(entry.key); return n; });
    } catch (e) {
      setError(String(e));
    }
    setMenu(null);
  };

  const togglePin = async (entry: S3Entry) => {
    if (!selectedDrive) return;
    const cmd = pinned.has(entry.key) ? 'unpin_file' : 'pin_file';
    try {
      await invoke(cmd, { token, driveId: selectedDrive.id, key: entry.key });
      setPinned(prev => {
        const next = new Set(prev);
        if (cmd === 'pin_file') next.add(entry.key); else next.delete(entry.key);
        return next;
      });
    } catch (e) {
      setError(String(e));
    }
    setMenu(null);
  };

  // Dismiss the context menu on any click outside or on Escape.
  React.useEffect(() => {
    if (!menu) return;
    const onClick = () => setMenu(null);
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') setMenu(null); };
    window.addEventListener('click', onClick);
    window.addEventListener('keydown', onKey);
    return () => {
      window.removeEventListener('click', onClick);
      window.removeEventListener('keydown', onKey);
    };
  }, [menu]);

  // Breadcrumbs from prefix
  const breadcrumbs: { label: string; prefix: string }[] = [{ label: selectedDrive?.letter ?? '—', prefix: '' }];
  if (prefix) {
    const parts = prefix.replace(/\/$/, '').split('/');
    let cur = '';
    for (const part of parts) {
      cur += part + '/';
      breadcrumbs.push({ label: part, prefix: cur });
    }
  }

  const titleCrumbs = selectedDrive
    ? ['Files', `${selectedDrive.letter} ${selectedDrive.name}`]
    : ['Files'];

  const subtitle = selectedDrive
    ? `${selectedDrive.bucket} · ${selectedDrive.region} · ${entries.length} item${entries.length !== 1 ? 's' : ''}`
    : 'Select a mounted drive';

  return (
    <>
      <TopBar
        theme={theme}
        crumbs={titleCrumbs}
        title={selectedDrive
          ? <>{selectedDrive.letter} <span style={{ color: t.lime }}>{selectedDrive.name}</span></>
          : <>File <span style={{ color: t.lime }}>Browser</span></>
        }
        subtitle={subtitle}
        actions={<>
          <NCBtn
            theme={theme} small iconLeft={<I.refresh size={13} />}
            onClick={() => setRefreshKey(k => k + 1)}
          >Refresh</NCBtn>
        </>}
      />

      <div style={{ flex: 1, display: 'flex', flexDirection: 'column', overflow: 'hidden' }}>
        {/* Toolbar */}
        <div style={{
          display: 'flex', alignItems: 'center', gap: 10,
          padding: '10px 20px', borderBottom: `1px solid ${t.border}`,
          background: t.surface1, flexShrink: 0,
        }}>
          {/* Drive selector */}
          {drives.length > 1 && (
            <select
              value={selectedDrive?.id ?? ''}
              onChange={e => {
                const d = drives.find(x => x.id === Number(e.target.value));
                if (d) changeDrive(d);
              }}
              style={{
                background: t.surface2, border: `1px solid ${t.border}`,
                color: t.textHi, fontFamily: NC_FONT_MONO, fontSize: 12,
                borderRadius: 3, padding: '5px 8px', outline: 'none', cursor: 'pointer',
              }}
            >
              {drives.map(d => (
                <option key={d.id} value={d.id}>{d.letter} {d.name}</option>
              ))}
            </select>
          )}

          {/* Nav buttons */}
          <NCBtn theme={theme} small ghost iconLeft={<I.chevL size={14} />} onClick={navigateUp} />
          <NCBtn
            theme={theme} small ghost iconLeft={<I.refresh size={13} />}
            onClick={() => setRefreshKey(k => k + 1)}
          />

          {/* Breadcrumb path bar */}
          <div style={{
            flex: 1, display: 'flex', alignItems: 'center', gap: 4,
            padding: '5px 10px', background: t.bg,
            border: `1px solid ${t.border}`, borderRadius: 3,
            fontFamily: NC_FONT_MONO, fontSize: 12, overflow: 'hidden',
          }}>
            {breadcrumbs.map((bc, i) => (
              <React.Fragment key={i}>
                {i > 0 && <span style={{ color: t.textFaint }}>\</span>}
                <span
                  onClick={() => setPrefix(bc.prefix)}
                  style={{
                    color: i === breadcrumbs.length - 1 ? t.textHi : t.lime,
                    cursor: i === breadcrumbs.length - 1 ? 'default' : 'pointer',
                    whiteSpace: 'nowrap',
                  }}
                >{bc.label}</span>
              </React.Fragment>
            ))}
          </div>
        </div>

        {/* No drives */}
        {drives.length === 0 && !loading && (
          <div style={{
            flex: 1, display: 'flex', flexDirection: 'column',
            alignItems: 'center', justifyContent: 'center', gap: 12,
            color: t.textMd, fontSize: 13,
          }}>
            <I.cloud size={36} color={t.textLo} />
            <div>No mounted drives. Mount a drive from the Drives tab first.</div>
          </div>
        )}

        {/* Error */}
        {error && (
          <div style={{
            margin: '16px 20px', padding: '10px 14px',
            background: `${t.danger}18`, border: `1px solid ${t.danger}50`,
            borderRadius: 3, fontSize: 12, color: t.danger,
            display: 'flex', gap: 8, alignItems: 'center',
          }}>
            <I.warn size={13} color={t.danger} style={{ flexShrink: 0 }} />
            {error}
          </div>
        )}

        {/* File list */}
        {selectedDrive && (
          <div style={{ flex: 1, overflow: 'auto' }}>
            {/* Header row */}
            <div style={{
              display: 'grid',
              gridTemplateColumns: '22px 1fr 110px 150px',
              gap: 14, padding: '9px 20px',
              borderBottom: `1px solid ${t.border}`,
              fontFamily: NC_FONT_MONO, fontSize: 9, letterSpacing: 1.5,
              color: t.textMd, textTransform: 'uppercase',
              position: 'sticky', top: 0, background: t.bg, zIndex: 1,
            }}>
              <span />
              <span>Name</span>
              <span>Size</span>
              <span>Modified</span>
            </div>

            {loading ? (
              <div style={{
                padding: 40, textAlign: 'center',
                fontFamily: NC_FONT_MONO, fontSize: 11, letterSpacing: 1.5, color: t.textLo,
              }}>
                LOADING…
              </div>
            ) : entries.length === 0 ? (
              <div style={{
                padding: 40, textAlign: 'center',
                fontSize: 13, color: t.textMd,
              }}>
                This folder is empty.
              </div>
            ) : (
              entries.map((entry, i) => {
                const isPinned = pinned.has(entry.key);
                const lockInfo = locks.get(entry.key);
                return (
                <div
                  key={entry.key + i}
                  onDoubleClick={() => navigateInto(entry)}
                  onContextMenu={e => {
                    if (entry.is_dir) return;
                    e.preventDefault();
                    setMenu({ x: e.clientX, y: e.clientY, entry });
                  }}
                  style={{
                    display: 'grid',
                    gridTemplateColumns: '22px 1fr 110px 150px',
                    gap: 14, padding: '9px 20px', alignItems: 'center',
                    borderBottom: `1px solid ${t.border}`,
                    cursor: entry.is_dir ? 'pointer' : 'default',
                    background: 'transparent',
                    transition: 'background 0.1s',
                  }}
                  onMouseEnter={e => (e.currentTarget.style.background = t.surface1)}
                  onMouseLeave={e => (e.currentTarget.style.background = 'transparent')}
                >
                  <FileIcon kind={inferKind(entry.name, entry.is_dir)} size={15} theme={theme} />
                  <div style={{
                    fontSize: 13, color: entry.is_dir ? t.lime : t.textHi,
                    fontWeight: entry.is_dir ? 500 : 400,
                    whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis',
                    fontFamily: NC_FONT_UI,
                    display: 'flex', alignItems: 'center', gap: 6,
                  }}>
                    <span style={{ overflow: 'hidden', textOverflow: 'ellipsis' }}>{entry.name}</span>
                    {isPinned && (
                      <span
                        title="Pinned — kept on device"
                        style={{ color: t.lime, fontSize: 11, flexShrink: 0 }}
                      >●</span>
                    )}
                    {lockInfo && (
                      <span
                        title={lockInfo.is_ours
                          ? `Locked by this machine (${lockInfo.owner})`
                          : `Locked by ${lockInfo.owner} on ${lockInfo.machine.slice(0, 8)}… — expires ${new Date(lockInfo.expires_at * 1000).toLocaleTimeString()}`}
                        style={{ color: lockInfo.is_ours ? t.textMd : t.danger, fontSize: 11, flexShrink: 0, display: 'inline-flex' }}
                      >
                        <I.lock size={11} />
                      </span>
                    )}
                  </div>
                  <div style={{ fontFamily: NC_FONT_MONO, fontSize: 11, color: t.textMd }}>
                    {entry.is_dir ? '—' : formatSize(entry.size)}
                  </div>
                  <div style={{ fontSize: 12, color: t.textMd }}>
                    {entry.is_dir ? '—' : formatDate(entry.modified)}
                  </div>
                </div>
                );
              })
            )}
          </div>
        )}

        {/* Status footer */}
        {selectedDrive && !loading && (
          <div style={{
            borderTop: `1px solid ${t.border}`, padding: '8px 20px',
            display: 'flex', alignItems: 'center', gap: 16,
            fontFamily: NC_FONT_MONO, fontSize: 10, letterSpacing: 0.5,
            color: t.textMd, background: t.surface1, flexShrink: 0,
          }}>
            <span>
              {entries.filter(e => e.is_dir).length} FOLDERS · {entries.filter(e => !e.is_dir).length} FILES
            </span>
            {prefix && (
              <>
                <span style={{ color: t.textFaint }}>|</span>
                <span style={{ color: t.textMd, fontFamily: NC_FONT_MONO, fontSize: 10 }}>
                  {prefix || '(root)'}
                </span>
              </>
            )}
          </div>
        )}
      </div>

      {/* Context menu — pin/unpin a file. Positioned at the cursor and
          dismissed by Escape or any outside click (see effect above). */}
      {menu && (
        <div
          onClick={e => e.stopPropagation()}
          style={{
            position: 'fixed', left: menu.x, top: menu.y, zIndex: 100,
            background: t.surface2, border: `1px solid ${t.border}`,
            borderRadius: 3, padding: 4, minWidth: 180,
            fontFamily: NC_FONT_UI, fontSize: 12,
            boxShadow: '0 4px 12px rgba(0,0,0,0.4)',
          }}
        >
          <div
            onClick={() => togglePin(menu.entry)}
            style={{
              padding: '7px 12px', cursor: 'pointer',
              color: t.textHi, borderRadius: 2,
              display: 'flex', alignItems: 'center', gap: 8,
            }}
            onMouseEnter={e => (e.currentTarget.style.background = t.surface1)}
            onMouseLeave={e => (e.currentTarget.style.background = 'transparent')}
          >
            <span style={{ color: t.lime, width: 10 }}>
              {pinned.has(menu.entry.key) ? '●' : '○'}
            </span>
            <span>
              {pinned.has(menu.entry.key) ? 'Unpin from device' : 'Keep on device'}
            </span>
          </div>
          {locks.has(menu.entry.key) && (
            <div
              onClick={() => breakLock(menu.entry)}
              style={{
                padding: '7px 12px', cursor: 'pointer',
                color: t.danger, borderRadius: 2,
                display: 'flex', alignItems: 'center', gap: 8,
                borderTop: `1px solid ${t.border}`, marginTop: 4, paddingTop: 8,
              }}
              onMouseEnter={e => (e.currentTarget.style.background = t.surface1)}
              onMouseLeave={e => (e.currentTarget.style.background = 'transparent')}
            >
              <I.lock size={12} />
              <span>Break lock</span>
            </div>
          )}
          <div style={{
            padding: '4px 12px 6px',
            color: t.textLo, fontSize: 10, fontFamily: NC_FONT_MONO,
            letterSpacing: 0.5, borderTop: `1px solid ${t.border}`,
            marginTop: 4,
          }}>
            {menu.entry.name.length > 28
              ? menu.entry.name.slice(0, 26) + '…'
              : menu.entry.name}
          </div>
        </div>
      )}
    </>
  );
};
