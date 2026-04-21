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
  onClose: () => void;
}> = ({ drive, theme, anchorRect, onRemove, onOpen, onClose }) => {
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
}> = ({ d, theme, last, menuOpen, menuAnchor, onMount, onUnmount, onMenuOpen, onMenuClose, onRemove, onOpen }) => {
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
            onRemove={onRemove} onOpen={onOpen} onClose={onMenuClose}
          />
        )}
      </div>
    </div>
  );
};

// ── Screen ────────────────────────────────────────────────────────────────────

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
  const [actionError, setActionError] = React.useState<string | null>(null);
  const [openMenu, setOpenMenu] = React.useState<{ id: number; rect: DOMRect } | null>(null);

  const loadDrives = React.useCallback(async () => {
    try {
      const result = await invoke<Drive[]>('list_drives', { token });
      setDrives(result);
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

  const mounted = drives.filter(d => d.status === 'mounted' || d.status === 'syncing').length;
  const readonly = drives.filter(d => d.readonly).length;

  return (
    <>
      <TopBar
        theme={theme}
        crumbs={[tr('dashboard.crumb')]}
        title={<>{tr('dashboard.titlePrefix')} <span style={{ color: t.lime }}>{tr('dashboard.titleAccent')}</span></>}
        subtitle={tr(drives.length === 1 ? 'dashboard.subtitleOne' : 'dashboard.subtitleOther', { count: drives.length })}
        actions={<>
          <NCBtn theme={theme} small iconLeft={<I.refresh size={13} />} onClick={loadDrives}>{tr('dashboard.refresh')}</NCBtn>
          <NCBtn theme={theme} small primary iconLeft={<I.plus size={13} />} onClick={onAddDrive}>{tr('dashboard.addDrive')}</NCBtn>
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
            <NCBtn theme={theme} small primary iconLeft={<I.plus size={13} />} onClick={onAddDrive}>{tr('dashboard.addFirst')}</NCBtn>
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
            <NCBtn theme={theme} small onClick={onAddDrive}>{tr('dashboard.addDrive')}</NCBtn>
          </div>
        )}
      </div>
    </>
  );
};
