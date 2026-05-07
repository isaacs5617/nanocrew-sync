import React from 'react';
import { useTranslation } from 'react-i18next';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import {
  getTokens, NC_FONT_DISPLAY, NC_FONT_MONO,
  NCCard, NCEyebrow, NCBtn, NCStatusDot, NCBadge,
  ProviderIcon, TopBar,
  type Theme, type DriveStatus,
} from '@nanocrew/ui';
import { I } from '@nanocrew/ui';
import { useAuth } from '../context/auth.js';

interface Drive {
  id: number;
  name: string;
  provider: string;
  endpoint: string;
  bucket: string;
  bucket_prefix: string;
  region: string;
  letter: string;
  access_key_id: string;
  cache_size_gb: number;
  auto_mount: boolean;
  readonly: boolean;
  created_at: number;
  status: string;
}

// ── Context menu (fixed-position portal to escape overflow:hidden) ────────────

const DriveMenu: React.FC<{
  drive: Drive;
  theme: Theme;
  anchorRect: DOMRect;
  onRemove: (id: number) => void;
  onOpen: (letter: string) => void;
  onEditPrefix: (drive: Drive) => void;
  onEditCredentials: (drive: Drive) => void;
  onCacheDrawer: (drive: Drive) => void;
  onClose: () => void;
}> = ({ drive, theme, anchorRect, onRemove, onOpen, onEditPrefix, onEditCredentials, onCacheDrawer, onClose }) => {
  const t = getTokens(theme);
  const { t: tr } = useTranslation();
  const ref = React.useRef<HTMLDivElement>(null);

  React.useEffect(() => {
    const handler = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) onClose();
    };
    document.addEventListener('mousedown', handler);
    return () => document.removeEventListener('mousedown', handler);
  }, [onClose]);

  const item = (label: string, icon: React.ReactNode, onClick: () => void, danger = false) => (
    <div
      onClick={() => { onClick(); onClose(); }}
      style={{
        display: 'flex', alignItems: 'center', gap: 10,
        padding: '8px 14px', cursor: 'pointer', fontSize: 13,
        color: danger ? t.danger : t.textHi,
      }}
      onMouseEnter={e => (e.currentTarget.style.background = t.surface2)}
      onMouseLeave={e => (e.currentTarget.style.background = 'transparent')}
    >
      {icon}
      {label}
    </div>
  );

  return (
    <div ref={ref} style={{
      position: 'fixed',
      top: anchorRect.bottom + 4,
      right: window.innerWidth - anchorRect.right,
      zIndex: 1000,
      background: t.surface1, border: `1px solid ${t.border}`,
      borderRadius: 4, boxShadow: '0 8px 24px rgba(0,0,0,0.4)',
      minWidth: 180, overflow: 'hidden',
    }}>
      {drive.status === 'mounted' && item(tr('dashboard.menu.openExplorer'), <I.folder size={13} />, () => onOpen(drive.letter))}
      {drive.status === 'mounted' && <div style={{ height: 1, background: t.border }} />}
      {item(tr('dashboard.menu.cacheAndBandwidth'), <I.settings size={13} />, () => onCacheDrawer(drive))}
      <div style={{ height: 1, background: t.border }} />
      {drive.status !== 'mounted' && item(tr('dashboard.menu.editPrefix'), <I.pencil size={13} />, () => onEditPrefix(drive))}
      {drive.status !== 'mounted' && item(tr('dashboard.menu.editCredentials'), <I.lock size={13} />, () => onEditCredentials(drive))}
      {drive.status !== 'mounted' && <div style={{ height: 1, background: t.border }} />}
      {drive.status !== 'mounted'
        ? item(tr('dashboard.menu.remove'), <I.trash size={13} />, () => onRemove(drive.id), true)
        : (
          <div style={{
            display: 'flex', alignItems: 'center', gap: 10,
            padding: '8px 14px', fontSize: 13,
            color: t.textLo, cursor: 'not-allowed',
          }}>
            <I.trash size={13} />
            {tr('dashboard.menu.remove')}
          </div>
        )}
    </div>
  );
};

// ── Drive row ─────────────────────────────────────────────────────────────────

const DriveRow: React.FC<{
  d: Drive;
  theme: Theme;
  last: boolean;
  menuOpen: boolean;
  menuAnchor: DOMRect | null;
  onMount: (id: number) => void;
  onUnmount: (id: number) => void;
  onMenuOpen: (id: number, rect: DOMRect) => void;
  onMenuClose: () => void;
  onRemove: (id: number) => void;
  onOpen: (letter: string) => void;
  onEditPrefix: (drive: Drive) => void;
  onEditCredentials: (drive: Drive) => void;
  onCacheDrawer: (drive: Drive) => void;
}> = ({ d, theme, last, menuOpen, menuAnchor, onMount, onUnmount, onMenuOpen, onMenuClose, onRemove, onOpen, onEditPrefix, onEditCredentials, onCacheDrawer }) => {
  const t = getTokens(theme);
  const { t: tr } = useTranslation();
  const statusMap: Record<string, { label: string; color: string; dot: DriveStatus }> = {
    mounted:   { label: tr('dashboard.status.mounted'),  color: t.lime,    dot: 'mounted' },
    mounting:  { label: tr('dashboard.status.mounting'), color: t.lime,    dot: 'syncing' },
    syncing:   { label: tr('dashboard.status.syncing'),  color: t.lime,    dot: 'syncing' },
    error:     { label: tr('dashboard.status.error'),    color: t.danger,  dot: 'error' },
    offline:   { label: tr('dashboard.status.offline'),  color: t.textLo,  dot: 'offline' },
  };
  const s = statusMap[d.status] ?? statusMap['offline']!;
  const isMounted = d.status === 'mounted' || d.status === 'mounting' || d.status === 'syncing';

  return (
    <div style={{
      display: 'grid',
      gridTemplateColumns: '24px 28px 1fr 80px 110px 110px 80px',
      gap: 14, padding: '14px 16px', alignItems: 'center',
      borderBottom: last ? 'none' : `1px solid ${t.border}`,
    }}>
      <NCStatusDot state={s.dot} theme={theme} />
      <ProviderIcon id={d.provider} size={18} theme={theme} />
      <div style={{ minWidth: 0 }}>
        <div style={{
          fontSize: 13, fontWeight: 500, color: t.textHi,
          whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis',
        }}>{d.name}</div>
        <div style={{ fontFamily: NC_FONT_MONO, fontSize: 10, color: t.textMd, letterSpacing: 0.5 }}>{d.bucket}</div>
      </div>
      <div style={{
        fontFamily: NC_FONT_MONO, fontWeight: 500, fontSize: 13,
        color: isMounted ? t.lime : t.textLo,
      }}>{d.letter}</div>
      <div style={{ fontFamily: NC_FONT_MONO, fontSize: 11, color: t.textMd, letterSpacing: 0.5 }}>{d.region}</div>
      <div style={{
        fontFamily: NC_FONT_MONO, fontSize: 10, color: s.color,
        letterSpacing: 1, textTransform: 'uppercase', fontWeight: 500,
      }}>{s.label}{d.readonly ? ' · RO' : ''}</div>
      <div style={{ display: 'flex', justifyContent: 'flex-end', gap: 6, position: 'relative' }}>
        {isMounted ? (
          <NCBtn theme={theme} small ghost onClick={() => onUnmount(d.id)}>
            <I.pause size={12} />
          </NCBtn>
        ) : (
          <NCBtn theme={theme} small ghost onClick={() => onMount(d.id)}>
            <I.play size={12} />
          </NCBtn>
        )}
        <div
          onClick={e => {
            if (menuOpen) { onMenuClose(); }
            else { onMenuOpen(d.id, (e.currentTarget as HTMLElement).getBoundingClientRect()); }
          }}
          style={{ cursor: 'pointer', display: 'flex', alignItems: 'center', padding: '4px 2px' }}
        >
          <I.more size={16} color={t.textMd} />
        </div>
        {menuOpen && menuAnchor && (
          <DriveMenu
            drive={d} theme={theme} anchorRect={menuAnchor}
            onRemove={onRemove} onOpen={onOpen} onEditPrefix={onEditPrefix} onEditCredentials={onEditCredentials}
            onCacheDrawer={onCacheDrawer} onClose={onMenuClose}
          />
        )}
      </div>
    </div>
  );
};

// ── Cache & Bandwidth drawer ──────────────────────────────────────────────────

interface CacheStats {
  used_bytes: number;
  max_bytes: number;
  enabled: boolean;
}

function fmtBytes(b: number): string {
  if (b >= 1_073_741_824) return `${(b / 1_073_741_824).toFixed(1)} GB`;
  if (b >= 1_048_576)     return `${(b / 1_048_576).toFixed(0)} MB`;
  return `${(b / 1024).toFixed(0)} KB`;
}

const CacheDrawer: React.FC<{
  drive: Drive;
  theme: Theme;
  token: string;
  onClose: () => void;
}> = ({ drive, theme, token, onClose }) => {
  const t = getTokens(theme);
  const { t: tr } = useTranslation();

  // Cache state
  const [stats, setStats] = React.useState<CacheStats | null>(null);
  const [connectivity, setConnectivity] = React.useState<string>('unknown');
  const [offline, setOffline] = React.useState<{ cached_files: number; total_files: number } | null>(null);

  // Local editable values
  const [enabled, setEnabled] = React.useState(true);
  const [maxGb, setMaxGb] = React.useState(10);
  const [uploadMbps, setUploadMbps] = React.useState('0');
  const [downloadMbps, setDownloadMbps] = React.useState('0');
  const [saving, setSaving] = React.useState(false);
  const [clearing, setClearing] = React.useState(false);
  const [prefetching, setPrefetching] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);

  const loadStats = React.useCallback(async () => {
    try {
      const [s, c, o] = await Promise.all([
        invoke<CacheStats>('get_drive_cache_stats', { token, driveId: drive.id }),
        invoke<string>('get_drive_connectivity', { token, driveId: drive.id }),
        invoke<{ cached_files: number; total_files: number }>('get_drive_offline_coverage', { token, driveId: drive.id }),
      ]);
      setStats(s);
      setConnectivity(c);
      setOffline(o);
      setEnabled(s.enabled);
      setMaxGb(Math.round(s.max_bytes / 1_073_741_824) || 10);
    } catch (e) {
      setError(String(e));
    }
  }, [token, drive.id]);

  React.useEffect(() => { loadStats(); }, [loadStats]);

  const handleSaveQuota = async () => {
    setSaving(true); setError(null);
    try {
      await invoke('set_drive_cache_quota', { token, driveId: drive.id, maxBytes: maxGb * 1_073_741_824 });
      await invoke('set_drive_cache_enabled', { token, driveId: drive.id, enabled });
      const ul = parseFloat(uploadMbps) || 0;
      const dl = parseFloat(downloadMbps) || 0;
      await invoke('set_drive_bandwidth', { token, driveId: drive.id, uploadMbps: ul, downloadMbps: dl });
      await loadStats();
    } catch (e) { setError(String(e)); }
    finally { setSaving(false); }
  };

  const handleClear = async () => {
    setClearing(true); setError(null);
    try {
      await invoke('clear_drive_cache', { token, driveId: drive.id });
      await loadStats();
    } catch (e) { setError(String(e)); }
    finally { setClearing(false); }
  };

  const handlePrefetch = async () => {
    setPrefetching(true); setError(null);
    try {
      await invoke('prefetch_pinned', { token, driveId: drive.id });
    } catch (e) { setError(String(e)); }
    finally { setPrefetching(false); }
  };

  const usedPct = stats ? Math.min(100, (stats.used_bytes / stats.max_bytes) * 100) : 0;
  const offlinePct = offline && offline.total_files > 0
    ? Math.round((offline.cached_files / offline.total_files) * 100) : 0;

  const connColor = connectivity === 'online' ? t.lime : connectivity === 'offline' ? t.danger : t.textLo;
  const connLabel = connectivity === 'online' ? tr('dashboard.cache.connectivity.online') : connectivity === 'offline' ? tr('dashboard.cache.connectivity.offline') : tr('dashboard.cache.connectivity.unknown');

  return (
    <div style={{
      position: 'fixed', inset: 0, zIndex: 2000,
      display: 'flex', alignItems: 'flex-start', justifyContent: 'flex-end',
    }}
      onClick={e => { if (e.target === e.currentTarget) onClose(); }}
    >
      {/* Dim overlay */}
      <div style={{ position: 'absolute', inset: 0, background: 'rgba(0,0,0,0.45)' }} onClick={onClose} />

      {/* Drawer panel */}
      <div style={{
        position: 'relative', zIndex: 1,
        width: 400, height: '100%',
        background: t.surface1, borderLeft: `1px solid ${t.border}`,
        display: 'flex', flexDirection: 'column',
        boxShadow: '-16px 0 48px rgba(0,0,0,0.5)',
      }}>
        {/* Header */}
        <div style={{
          padding: '16px 20px', borderBottom: `1px solid ${t.border}`,
          display: 'flex', alignItems: 'center', justifyContent: 'space-between',
        }}>
          <div>
            <div style={{ fontSize: 13, fontWeight: 600, color: t.textHi }}>{drive.name}</div>
            <NCEyebrow theme={theme} style={{ marginTop: 2 }}>{tr('dashboard.cache.title')}</NCEyebrow>
          </div>
          <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
            {/* Connectivity pill */}
            <div style={{
              display: 'flex', alignItems: 'center', gap: 5,
              padding: '3px 9px', borderRadius: 20,
              background: `${connColor}18`, border: `1px solid ${connColor}40`,
              fontSize: 10, fontFamily: NC_FONT_MONO, letterSpacing: 1,
              textTransform: 'uppercase', color: connColor,
            }}>
              <div style={{ width: 5, height: 5, borderRadius: '50%', background: connColor }} />
              {connLabel}
            </div>
            <NCBtn theme={theme} ghost small onClick={onClose}><I.x size={12} /></NCBtn>
          </div>
        </div>

        {/* Body */}
        <div style={{ flex: 1, overflowY: 'auto', padding: 20, display: 'flex', flexDirection: 'column', gap: 20 }}>

          {error && (
            <div style={{
              padding: '8px 12px', background: `${t.danger}18`,
              border: `1px solid ${t.danger}40`, borderRadius: 3,
              fontSize: 11, color: t.danger,
            }}>{error}</div>
          )}

          {/* Cache section */}
          <div>
            <NCEyebrow theme={theme} style={{ marginBottom: 12 }}>{tr('dashboard.cache.diskCache')}</NCEyebrow>

            {/* Enable toggle */}
            <div style={{
              display: 'flex', alignItems: 'center', justifyContent: 'space-between',
              marginBottom: 14,
            }}>
              <div style={{ fontSize: 12, color: t.textHi }}>{tr('dashboard.cache.enableCache')}</div>
              <div
                onClick={() => setEnabled(v => !v)}
                style={{
                  width: 36, height: 20, borderRadius: 10, cursor: 'pointer',
                  background: enabled ? t.lime : t.surface2,
                  border: `1px solid ${enabled ? t.lime : t.border}`,
                  position: 'relative', transition: 'background 0.15s',
                }}
              >
                <div style={{
                  position: 'absolute', top: 2,
                  left: enabled ? 17 : 2,
                  width: 14, height: 14, borderRadius: '50%',
                  background: enabled ? '#000' : t.textMd,
                  transition: 'left 0.15s',
                }} />
              </div>
            </div>

            {/* Usage bar */}
            {stats ? (
              <div style={{ marginBottom: 14 }}>
                <div style={{
                  display: 'flex', justifyContent: 'space-between',
                  fontSize: 11, color: t.textMd, marginBottom: 5,
                }}>
                  <span>{tr('dashboard.cache.used', { value: fmtBytes(stats.used_bytes) })}</span>
                  <span>{tr('dashboard.cache.quota', { value: fmtBytes(stats.max_bytes) })}</span>
                </div>
                <div style={{
                  height: 6, background: t.surface2, borderRadius: 3, overflow: 'hidden',
                }}>
                  <div style={{
                    height: '100%', borderRadius: 3,
                    width: `${usedPct}%`,
                    background: usedPct > 90 ? t.danger : t.lime,
                    transition: 'width 0.3s',
                  }} />
                </div>
              </div>
            ) : (
              <div style={{ fontSize: 11, color: t.textLo, marginBottom: 14 }}>{tr('common.loading')}</div>
            )}

            {/* Quota slider */}
            <div style={{ marginBottom: 14 }}>
              <div style={{
                display: 'flex', justifyContent: 'space-between',
                fontSize: 11, color: t.textMd, marginBottom: 5,
              }}>
                <span>{tr('dashboard.cache.quotaLabel')}</span>
                <span style={{ fontFamily: NC_FONT_MONO }}>{maxGb} GB</span>
              </div>
              <input
                type="range" min={1} max={500} step={1}
                value={maxGb}
                onChange={e => setMaxGb(Number(e.target.value))}
                style={{ width: '100%', accentColor: t.lime }}
              />
              <div style={{
                display: 'flex', justifyContent: 'space-between',
                fontSize: 10, color: t.textLo,
              }}>
                <span>1 GB</span><span>500 GB</span>
              </div>
            </div>

            {/* Offline coverage */}
            {offline && (
              <div style={{
                padding: '8px 12px', background: t.surface2,
                borderRadius: 3, fontSize: 11, color: t.textMd,
                display: 'flex', justifyContent: 'space-between',
                alignItems: 'center', marginBottom: 14,
              }}>
                <span>{tr('dashboard.cache.offlineCoverage')}</span>
                <span style={{ fontFamily: NC_FONT_MONO, color: t.textHi }}>
                  {tr('dashboard.cache.offlineFiles', { cached: offline.cached_files, total: offline.total_files, pct: offlinePct })}
                </span>
              </div>
            )}

            {/* Actions row */}
            <div style={{ display: 'flex', gap: 8 }}>
              <NCBtn theme={theme} ghost small onClick={handleClear} disabled={clearing}>
                {clearing ? tr('dashboard.cache.clearing') : tr('dashboard.cache.clearCache')}
              </NCBtn>
              <NCBtn theme={theme} ghost small onClick={handlePrefetch} disabled={prefetching}>
                {prefetching ? tr('dashboard.cache.syncing') : tr('dashboard.cache.syncPinned')}
              </NCBtn>
            </div>
          </div>

          {/* Bandwidth section */}
          <div>
            <NCEyebrow theme={theme} style={{ marginBottom: 12 }}>{tr('dashboard.cache.bandwidthLimits')}</NCEyebrow>
            <div style={{ fontSize: 11, color: t.textLo, marginBottom: 12 }}>
              {tr('dashboard.cache.bandwidthDesc')}
            </div>

            <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 12 }}>
              <div>
                <div style={{ fontSize: 11, color: t.textMd, marginBottom: 4 }}>{tr('dashboard.cache.upload')}</div>
                <input
                  type="number" min={0} step={0.5}
                  value={uploadMbps}
                  onChange={e => setUploadMbps(e.target.value)}
                  style={{
                    width: '100%', boxSizing: 'border-box',
                    background: t.surface2, border: `1px solid ${t.border}`,
                    borderRadius: 3, padding: '6px 9px',
                    color: t.textHi, fontSize: 12, outline: 'none',
                    fontFamily: NC_FONT_MONO,
                  }}
                />
              </div>
              <div>
                <div style={{ fontSize: 11, color: t.textMd, marginBottom: 4 }}>{tr('dashboard.cache.download')}</div>
                <input
                  type="number" min={0} step={0.5}
                  value={downloadMbps}
                  onChange={e => setDownloadMbps(e.target.value)}
                  style={{
                    width: '100%', boxSizing: 'border-box',
                    background: t.surface2, border: `1px solid ${t.border}`,
                    borderRadius: 3, padding: '6px 9px',
                    color: t.textHi, fontSize: 12, outline: 'none',
                    fontFamily: NC_FONT_MONO,
                  }}
                />
              </div>
            </div>
          </div>
        </div>

        {/* Footer */}
        <div style={{
          padding: '12px 20px', borderTop: `1px solid ${t.border}`,
          display: 'flex', justifyContent: 'flex-end', gap: 8,
        }}>
          <NCBtn theme={theme} ghost small onClick={onClose}>{tr('common.cancel')}</NCBtn>
          <NCBtn theme={theme} primary small onClick={handleSaveQuota} disabled={saving}>
            {saving ? tr('dashboard.cache.saving') : tr('common.save')}
          </NCBtn>
        </div>
      </div>
    </div>
  );
};

// ── Screen ────────────────────────────────────────────────────────────────────

interface LicenseStatus {
  tier: string;
  is_pro: boolean;
  expires_at: number;
  days_remaining: number;
  key_id: string | null;
  email: string | null;
  machine_fingerprint_short: string;
}

interface DashboardScreenProps {
  theme: Theme;
  onAddDrive?: () => void;
}

export const DashboardScreen: React.FC<DashboardScreenProps> = ({ theme, onAddDrive }) => {
  const t = getTokens(theme);
  const { t: tr } = useTranslation();
  const { token } = useAuth();
  const [drives, setDrives] = React.useState<Drive[]>([]);
  const [loading, setLoading] = React.useState(true);
  const [license, setLicense] = React.useState<LicenseStatus | null>(null);
  const [actionError, setActionError] = React.useState<string | null>(null);
  const [openMenu, setOpenMenu] = React.useState<{ id: number; rect: DOMRect } | null>(null);
  const [editPrefix, setEditPrefix] = React.useState<{ drive: Drive; value: string } | null>(null);
  const [savingPrefix, setSavingPrefix] = React.useState(false);
  const [editCreds, setEditCreds] = React.useState<{ drive: Drive; accessKeyId: string; secretAccessKey: string } | null>(null);
  const [savingCreds, setSavingCreds] = React.useState(false);
  const [cacheDrawer, setCacheDrawer] = React.useState<Drive | null>(null);

  const loadDrives = React.useCallback(async () => {
    try {
      const [result, lic] = await Promise.all([
        invoke<Drive[]>('list_drives', { token }),
        invoke<LicenseStatus>('get_license_status', { token }),
      ]);
      setDrives(result);
      setLicense(lic);
    } catch (e) {
      console.error('list_drives failed:', e);
    } finally {
      setLoading(false);
    }
  }, [token]);

  React.useEffect(() => { loadDrives(); }, [loadDrives]);

  React.useEffect(() => {
    const unlisten = listen<{ drive_id: number; status: string }>('drive_status_changed', e => {
      setDrives(prev => prev.map(d =>
        d.id === e.payload.drive_id ? { ...d, status: e.payload.status } : d
      ));
    });
    return () => { unlisten.then(fn => fn()); };
  }, []);

  const handleMount = async (driveId: number) => {
    setActionError(null);
    try { await invoke('mount_drive', { token, driveId }); }
    catch (e) { setActionError(String(e)); }
  };

  const handleUnmount = async (driveId: number) => {
    setActionError(null);
    try { await invoke('unmount_drive', { token, driveId }); }
    catch (e) { setActionError(String(e)); }
  };

  const handleRemove = async (driveId: number) => {
    setActionError(null);
    try {
      await invoke('remove_drive', { token, driveId });
      setDrives(prev => prev.filter(d => d.id !== driveId));
    } catch (e) {
      setActionError(String(e));
    }
  };

  const handleOpenInExplorer = async (letter: string) => {
    try { await invoke('open_path', { token, path: `${letter}\\` }); }
    catch (e) { setActionError(String(e)); }
  };

  const handleSavePrefix = async () => {
    if (!editPrefix) return;
    setSavingPrefix(true);
    setActionError(null);
    try {
      await invoke('set_drive_prefix', {
        token,
        driveId: editPrefix.drive.id,
        bucketPrefix: editPrefix.value,
      });
      setDrives(prev => prev.map(d =>
        d.id === editPrefix.drive.id ? { ...d, bucket_prefix: editPrefix.value } : d
      ));
      setEditPrefix(null);
    } catch (e) {
      setActionError(String(e));
    } finally {
      setSavingPrefix(false);
    }
  };

  const handleSaveCreds = async () => {
    if (!editCreds) return;
    setSavingCreds(true);
    setActionError(null);
    try {
      await invoke('set_drive_credentials', {
        token,
        driveId: editCreds.drive.id,
        accessKeyId: editCreds.accessKeyId,
        secretAccessKey: editCreds.secretAccessKey,
      });
      setDrives(prev => prev.map(d =>
        d.id === editCreds.drive.id ? { ...d, access_key_id: editCreds.accessKeyId } : d
      ));
      setEditCreds(null);
    } catch (e) {
      setActionError(String(e));
    } finally {
      setSavingCreds(false);
    }
  };

  const mounted = drives.filter(d => d.status === 'mounted' || d.status === 'syncing').length;
  const readonly = drives.filter(d => d.readonly).length;

  const addDriveLocked = license !== null && !license.is_pro && drives.length >= 2;

  const handleAddDrive = () => {
    if (addDriveLocked) {
      setActionError('Free tier is limited to 2 drives. Upgrade to Pro at nanocrew.dev/pricing');
      return;
    }
    onAddDrive?.();
  };

  return (
    <>
      <TopBar
        theme={theme}
        crumbs={[tr('dashboard.crumb')]}
        title={<>{tr('dashboard.titlePrefix')} <span style={{ color: t.lime }}>{tr('dashboard.titleAccent')}</span></>}
        subtitle={tr(drives.length === 1 ? 'dashboard.subtitleOne' : 'dashboard.subtitleOther', { count: drives.length })}
        actions={<>
          <NCBtn theme={theme} small iconLeft={<I.refresh size={13} />} onClick={loadDrives}>{tr('dashboard.refresh')}</NCBtn>
          <NCBtn theme={theme} small primary={!addDriveLocked} iconLeft={addDriveLocked ? <I.lock size={13} /> : <I.plus size={13} />} onClick={handleAddDrive}>{tr('dashboard.addDrive')}</NCBtn>
        </>}
      />

      <div style={{ flex: 1, overflow: 'auto', padding: 28 }}>
        {/* Stats strip */}
        <div style={{ display: 'grid', gridTemplateColumns: 'repeat(3, 1fr)', gap: 12, marginBottom: 24 }}>
          {[
            { label: tr('dashboard.stats.mounted'),  value: String(mounted),         foot: tr('dashboard.stats.ofConfigured', { total: drives.length }) },
            { label: tr('dashboard.stats.drives'),   value: String(drives.length),   foot: drives.length === 0 ? tr('dashboard.stats.noneYet') : tr('dashboard.stats.readonlyCount', { count: readonly }) },
            { label: tr('dashboard.stats.provider'), value: drives.length > 0 ? drives[0].provider.toUpperCase() : '—', foot: drives.length > 1 ? tr('dashboard.stats.moreCount', { count: drives.length - 1 }) : tr('dashboard.stats.tagline') },
          ].map((s, i) => (
            <NCCard key={i} theme={theme} pad={16}>
              <NCEyebrow theme={theme} style={{ marginBottom: 10 }}>{s.label}</NCEyebrow>
              <div style={{
                fontFamily: NC_FONT_DISPLAY, fontWeight: 800,
                fontSize: 36, letterSpacing: -1.5, color: t.lime,
                lineHeight: 1, marginBottom: 6,
              }}>{s.value}</div>
              <div style={{ fontSize: 11, color: t.textMd }}>{s.foot}</div>
            </NCCard>
          ))}
        </div>

        {actionError && (
          <div style={{
            padding: '10px 14px', marginBottom: 16,
            background: `${t.danger}18`, border: `1px solid ${t.danger}50`,
            borderRadius: 3, fontSize: 12, color: t.danger,
          }}>
            <div style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
              <I.warn size={13} color={t.danger} style={{ flexShrink: 0 }} />
              {actionError}
            </div>
            {actionError.toLowerCase().includes('winfsp') && (
              <div style={{ marginTop: 8, paddingLeft: 21, color: t.textMd }}>
                {tr('dashboard.winfspHint')}{' '}
                <span
                  onClick={() => invoke('open_path', { token, path: 'https://winfsp.net' })}
                  style={{ color: t.lime, fontFamily: NC_FONT_MONO, fontSize: 11, cursor: 'pointer' }}
                >winfsp.net</span>{' '}
                {tr('dashboard.winfspHintCont')}
              </div>
            )}
            {actionError.toLowerCase().includes('credential') && (
              <div style={{ marginTop: 8, paddingLeft: 21, color: t.textMd }}>
                {tr('dashboard.credentialHint')}
              </div>
            )}
            {actionError.toLowerCase().includes('upgrade to pro') && (
              <div style={{ marginTop: 8, paddingLeft: 21, color: t.textMd }}>
                <span
                  onClick={() => invoke('open_path', { token, path: 'https://nanocrew.dev/pricing' })}
                  style={{ color: t.lime, fontFamily: NC_FONT_MONO, fontSize: 11, cursor: 'pointer' }}
                >nanocrew.dev/pricing</span>
              </div>
            )}
          </div>
        )}

        {/* Drive list */}
        <NCEyebrow theme={theme} style={{ marginBottom: 12 }}>{tr('dashboard.drivesHeader')}</NCEyebrow>

        {loading ? (
          <div style={{ padding: 40, textAlign: 'center', color: t.textLo, fontFamily: NC_FONT_MONO, fontSize: 11, letterSpacing: 1.5 }}>
            {tr('dashboard.loadingUpper')}
          </div>
        ) : drives.length === 0 ? (
          <div style={{
            padding: 40, border: `1px dashed ${t.border}`, borderRadius: 4,
            display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 12,
          }}>
            <I.cloud size={32} color={t.textLo} />
            <div style={{ fontSize: 13, color: t.textMd }}>{tr('dashboard.noDrives')}</div>
            <NCBtn theme={theme} small primary iconLeft={<I.plus size={13} />} onClick={handleAddDrive}>{tr('dashboard.addFirst')}</NCBtn>
          </div>
        ) : (
          <div style={{ border: `1px solid ${t.border}`, borderRadius: 4, background: t.surface1, overflow: 'hidden' }}>
            <div style={{
              display: 'grid',
              gridTemplateColumns: '24px 28px 1fr 80px 110px 110px 80px',
              gap: 14, padding: '10px 16px',
              borderBottom: `1px solid ${t.border}`,
              fontFamily: NC_FONT_MONO, fontSize: 9, letterSpacing: 1.5,
              color: t.textMd, textTransform: 'uppercase',
            }}>
              <span /><span />
              <span>{tr('dashboard.col.name')}</span>
              <span>{tr('dashboard.col.letter')}</span>
              <span>{tr('dashboard.col.region')}</span>
              <span>{tr('dashboard.col.status')}</span>
              <span />
            </div>
            {drives.map((d, i) => (
              <DriveRow
                key={d.id} d={d} theme={theme}
                last={i === drives.length - 1}
                menuOpen={openMenu?.id === d.id}
                menuAnchor={openMenu?.id === d.id ? openMenu.rect : null}
                onMount={handleMount}
                onUnmount={handleUnmount}
                onMenuOpen={(id, rect) => setOpenMenu({ id, rect })}
                onMenuClose={() => setOpenMenu(null)}
                onRemove={handleRemove}
                onOpen={handleOpenInExplorer}
                onEditPrefix={drive => setEditPrefix({ drive, value: drive.bucket_prefix })}
                onEditCredentials={drive => setEditCreds({ drive, accessKeyId: drive.access_key_id, secretAccessKey: '' })}
                onCacheDrawer={drive => setCacheDrawer(drive)}
              />
            ))}
          </div>
        )}

        {drives.length > 0 && (
          <div style={{
            marginTop: 16, padding: 20,
            border: `1px dashed ${t.border}`, borderRadius: 4,
            display: 'flex', alignItems: 'center', gap: 14,
          }}>
            <div style={{
              width: 32, height: 32, borderRadius: 3,
              border: `1px solid ${t.border}`, background: t.surface2,
              display: 'flex', alignItems: 'center', justifyContent: 'center',
            }}>
              <I.plus size={16} color={t.textMd} />
            </div>
            <div style={{ flex: 1 }}>
              <div style={{ fontSize: 13, color: t.textHi, fontWeight: 500 }}>{tr('dashboard.connectTitle')}</div>
              <div style={{ fontSize: 12, color: t.textMd }}>{tr('dashboard.connectSub')}</div>
            </div>
            <NCBtn theme={theme} small onClick={handleAddDrive} iconLeft={addDriveLocked ? <I.lock size={13} /> : undefined}>{tr('dashboard.addDrive')}</NCBtn>
          </div>
        )}
      </div>

      {/* Edit credentials modal */}
      {editCreds && (
        <div style={{
          position: 'fixed', inset: 0, zIndex: 2000,
          background: 'rgba(0,0,0,0.55)',
          display: 'flex', alignItems: 'center', justifyContent: 'center',
        }}>
          <div style={{
            background: t.surface1, border: `1px solid ${t.border}`,
            borderRadius: 6, padding: 24, width: 480,
            boxShadow: '0 16px 48px rgba(0,0,0,0.5)',
          }}>
            <div style={{ fontSize: 14, fontWeight: 600, color: t.textHi, marginBottom: 6 }}>
              {tr('dashboard.editCreds.title', { name: editCreds.drive.name })}
            </div>
            <div style={{ fontSize: 12, color: t.textMd, marginBottom: 16 }}>
              {tr('dashboard.editCreds.desc')}
            </div>
            <div style={{ fontSize: 11, color: t.textMd, marginBottom: 4 }}>{tr('dashboard.editCreds.accessKeyId')}</div>
            <input
              autoFocus
              value={editCreds.accessKeyId}
              onChange={e => setEditCreds(p => p ? { ...p, accessKeyId: e.target.value } : p)}
              placeholder={tr('dashboard.editCreds.accessKeyId')}
              style={{
                width: '100%', boxSizing: 'border-box',
                background: t.surface2, border: `1px solid ${t.border}`,
                borderRadius: 3, padding: '7px 10px',
                color: t.textHi, fontSize: 13, outline: 'none',
                fontFamily: 'monospace', marginBottom: 12,
              }}
            />
            <div style={{ fontSize: 11, color: t.textMd, marginBottom: 4 }}>{tr('dashboard.editCreds.secretKey')}</div>
            <input
              type="password"
              value={editCreds.secretAccessKey}
              onChange={e => setEditCreds(p => p ? { ...p, secretAccessKey: e.target.value } : p)}
              onKeyDown={e => { if (e.key === 'Enter') handleSaveCreds(); if (e.key === 'Escape') setEditCreds(null); }}
              placeholder={tr('dashboard.editCreds.secretKey')}
              style={{
                width: '100%', boxSizing: 'border-box',
                background: t.surface2, border: `1px solid ${t.border}`,
                borderRadius: 3, padding: '7px 10px',
                color: t.textHi, fontSize: 13, outline: 'none',
                fontFamily: 'monospace', marginBottom: 16,
              }}
            />
            <div style={{ display: 'flex', justifyContent: 'flex-end', gap: 8 }}>
              <NCBtn theme={theme} ghost small onClick={() => setEditCreds(null)}>{tr('common.cancel')}</NCBtn>
              <NCBtn theme={theme} primary small disabled={savingCreds || !editCreds.accessKeyId || !editCreds.secretAccessKey} onClick={handleSaveCreds}>
                {savingCreds ? tr('dashboard.editCreds.saving') : tr('common.save')}
              </NCBtn>
            </div>
          </div>
        </div>
      )}

      {/* Cache & Bandwidth drawer */}
      {cacheDrawer && (
        <CacheDrawer
          drive={cacheDrawer}
          theme={theme}
          token={token!}
          onClose={() => setCacheDrawer(null)}
        />
      )}

      {/* Edit prefix modal */}
      {editPrefix && (
        <div style={{
          position: 'fixed', inset: 0, zIndex: 2000,
          background: 'rgba(0,0,0,0.55)',
          display: 'flex', alignItems: 'center', justifyContent: 'center',
        }}>
          <div style={{
            background: t.surface1, border: `1px solid ${t.border}`,
            borderRadius: 6, padding: 24, width: 480,
            boxShadow: '0 16px 48px rgba(0,0,0,0.5)',
          }}>
            <div style={{ fontSize: 14, fontWeight: 600, color: t.textHi, marginBottom: 6 }}>
              {tr('dashboard.editPrefix.title', { name: editPrefix.drive.name })}
            </div>
            <div style={{ fontSize: 12, color: t.textMd, marginBottom: 16 }}>
              {tr('dashboard.editPrefix.desc')}
            </div>
            <input
              autoFocus
              value={editPrefix.value}
              onChange={e => setEditPrefix(p => p ? { ...p, value: e.target.value } : p)}
              onKeyDown={e => { if (e.key === 'Enter') handleSavePrefix(); if (e.key === 'Escape') setEditPrefix(null); }}
              placeholder="e.g. users/alice  (leave blank for bucket root)"
              style={{
                width: '100%', boxSizing: 'border-box',
                background: t.surface2, border: `1px solid ${t.border}`,
                borderRadius: 3, padding: '7px 10px',
                color: t.textHi, fontSize: 13, outline: 'none',
                fontFamily: 'monospace', marginBottom: 16,
              }}
            />
            <div style={{ display: 'flex', justifyContent: 'flex-end', gap: 8 }}>
              <NCBtn theme={theme} ghost small onClick={() => setEditPrefix(null)}>{tr('common.cancel')}</NCBtn>
              <NCBtn theme={theme} primary small disabled={savingPrefix} onClick={handleSavePrefix}>
                {savingPrefix ? tr('dashboard.cache.saving') : tr('common.save')}
              </NCBtn>
            </div>
          </div>
        </div>
      )}
    </>
  );
};
