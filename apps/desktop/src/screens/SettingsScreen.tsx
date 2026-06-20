import React from 'react';
import { useTranslation } from 'react-i18next';
import { invoke } from '@tauri-apps/api/core';
import { getVersion } from '@tauri-apps/api/app';
import { appDataDir, join } from '@tauri-apps/api/path';
import * as Sentry from '@sentry/react';
import {
  getTokens, NC_FONT_MONO,
  NCCard, NCEyebrow, NCLabel, NCToggle, NCBtn,
  TopBar,
  type Theme,
} from '@nanocrew/ui';
import { I } from '@nanocrew/ui';
import { useAuth } from '../context/auth.js';
import { UpdateButton } from './UpdateButton.js';
import { readLockOnMinimize, writeLockOnMinimize } from '../App.js';

interface SettingsScreenProps {
  theme: Theme;
  setTheme: (t: Theme) => void;
}

const ToggleRow: React.FC<{
  label: string; sub?: string; on?: boolean; theme: Theme;
  comingSoon?: boolean;
  onChange?: (v: boolean) => void;
}> = ({
  label, sub, on, theme, comingSoon, onChange,
}) => {
  const { t } = useTranslation();
  const tok = getTokens(theme);
  const [v, setV] = React.useState(comingSoon ? false : (on ?? false));
  // Keep internal state in sync when `on` prop changes (controlled-ish).
  React.useEffect(() => { if (!comingSoon && on !== undefined) setV(on); }, [on, comingSoon]);
  const handle = (next: boolean) => {
    if (comingSoon) return;
    setV(next);
    onChange?.(next);
  };
  return (
    <div style={{ display: 'flex', alignItems: 'center', gap: 14, opacity: comingSoon ? 0.55 : 1 }}>
      <div style={{ flex: 1 }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 8, flexWrap: 'wrap' }}>
          <div style={{ fontSize: 13, color: tok.textHi, fontWeight: 500 }}>{label}</div>
          {comingSoon && (
            <span style={{
              fontFamily: NC_FONT_MONO, fontSize: 9, letterSpacing: 1.2,
              color: tok.textLo, background: tok.surface2,
              padding: '2px 6px', borderRadius: 2,
            }}>{t('settings.comingSoon')}</span>
          )}
        </div>
        {sub && <div style={{ fontSize: 11, color: tok.textMd, marginTop: 2 }}>{sub}</div>}
      </div>
      <NCToggle on={v} onChange={handle} theme={theme} />
    </div>
  );
};

const Spacer = () => <div style={{ height: 12 }} />;

/// ToggleRow backed by a string key in the SQLite `prefs` table. Reads the
/// current value on mount and persists on change. Errors fall silently back
/// to the built-in default — no toast spam for a failing toggle.
const PrefToggle: React.FC<{
  theme: Theme; token: string;
  prefKey: string; defaultOn?: boolean;
  label: string; sub?: string;
  onAfterChange?: (v: boolean) => void;
}> = ({ theme, token, prefKey, defaultOn = false, label, sub, onAfterChange }) => {
  const [on, setOn] = React.useState<boolean | null>(null);
  React.useEffect(() => {
    invoke<string | null>('get_pref', { token, key: prefKey })
      .then(v => setOn(v === null || v === undefined
        ? defaultOn
        : (v === '1' || v === 'true')))
      .catch(() => setOn(defaultOn));
  }, [token, prefKey, defaultOn]);

  if (on === null) return <ToggleRow theme={theme} label={label} sub={sub} />;
  return (
    <ToggleRow
      theme={theme} label={label} sub={sub} on={on}
      onChange={async (next) => {
        setOn(next);
        try {
          await invoke('set_pref', { token, key: prefKey, value: next ? '1' : '0' });
          onAfterChange?.(next);
        } catch {
          setOn(!next);
        }
      }}
    />
  );
};

/// Text input backed by a `prefs` key. Debounces save-on-change so we don't
/// hammer SQLite on every keystroke. Empty string saved as empty — the Rust
/// side treats empty-or-missing as "unset".
const PrefInput: React.FC<{
  theme: Theme; token: string;
  prefKey: string; label: string; sub?: string;
  placeholder?: string; mono?: boolean;
}> = ({ theme, token, prefKey, label, sub, placeholder, mono }) => {
  const tok = getTokens(theme);
  const [value, setValue] = React.useState<string>('');
  const [loaded, setLoaded] = React.useState(false);

  React.useEffect(() => {
    invoke<string | null>('get_pref', { token, key: prefKey })
      .then(v => { setValue(v ?? ''); setLoaded(true); })
      .catch(() => setLoaded(true));
  }, [token, prefKey]);

  // Debounced persistence.
  React.useEffect(() => {
    if (!loaded) return;
    const handle = window.setTimeout(() => {
      invoke('set_pref', { token, key: prefKey, value }).catch(() => {});
    }, 400);
    return () => window.clearTimeout(handle);
  }, [value, loaded, token, prefKey]);

  return (
    <div>
      <div style={{ fontSize: 13, color: tok.textHi, fontWeight: 500 }}>{label}</div>
      {sub && <div style={{ fontSize: 11, color: tok.textMd, marginTop: 2, marginBottom: 8 }}>{sub}</div>}
      <input
        type="text"
        value={value}
        placeholder={placeholder}
        onChange={e => setValue(e.target.value)}
        style={{
          width: '100%', boxSizing: 'border-box',
          padding: '10px 12px',
          background: tok.surface1,
          border: `1px solid ${tok.border}`,
          borderRadius: 3, outline: 'none',
          color: tok.textHi, fontSize: 13,
          fontFamily: mono ? NC_FONT_MONO : undefined,
        }}
      />
    </div>
  );
};

/// "Launch at Windows sign-in" toggle backed by the HKCU\...\Run registry
/// key. Reads the current state on mount; writes through set_autostart.
const AutostartRow: React.FC<{ theme: Theme; token: string }> = ({ theme, token }) => {
  const { t } = useTranslation();
  const [on, setOn] = React.useState<boolean | null>(null);
  React.useEffect(() => {
    invoke<boolean>('get_autostart', { token })
      .then(setOn)
      .catch(() => setOn(false));
  }, [token]);

  // Hide until the registry read completes so the toggle doesn't flicker.
  if (on === null) {
    return (
      <ToggleRow
        theme={theme}
        label={t('settings.startup.autostart.label')}
        sub={t('settings.startup.autostartLoading.sub')}
      />
    );
  }
  return (
    <ToggleRow
      theme={theme}
      label={t('settings.startup.autostart.label')}
      sub={t('settings.startup.autostart.sub')}
      on={on}
      onChange={async (next) => {
        setOn(next);
        try {
          await invoke('set_autostart', { token, enabled: next });
        } catch {
          // Roll back the UI state if the registry write failed.
          setOn(!next);
        }
      }}
    />
  );
};

/// Cache location picker. Reads the effective + default root from the backend,
/// lets the user override via a `cache_root` pref (empty = default), and
/// offers an "Open folder" shortcut. Changes apply at next mount — we don't
/// try to migrate existing cache files.
const CacheLocationCard: React.FC<{ theme: Theme; token: string }> = ({ theme, token }) => {
  const { t } = useTranslation();
  const tok = getTokens(theme);
  const [info, setInfo] = React.useState<{ effective: string; def: string; isCustom: boolean } | null>(null);
  const [reloadTick, setReloadTick] = React.useState(0);

  React.useEffect(() => {
    invoke<[string, string, boolean]>('get_cache_root_info', { token })
      .then(([effective, def, isCustom]) => setInfo({ effective, def, isCustom }))
      .catch(() => {});
  }, [token, reloadTick]);

  // Poll once more shortly after mount so the effective path tracks the pref
  // after a debounced save completes. Cheap — one SQLite read.
  React.useEffect(() => {
    const h = window.setInterval(() => setReloadTick(x => x + 1), 1500);
    return () => window.clearInterval(h);
  }, []);

  return (
    <NCCard theme={theme} pad={20}>
      <NCEyebrow theme={theme} style={{ marginBottom: 14 }}>{t('settings.cache.location')}</NCEyebrow>
      <div style={{ fontSize: 13, color: tok.textMd, lineHeight: 1.55, marginBottom: 16 }}>
        {t('settings.cache.locationDesc')}
      </div>
      <PrefInput
        theme={theme} token={token}
        prefKey="cache_root"
        label={t('settings.cache.customFolder.label')}
        sub={t('settings.cache.customFolder.sub')}
        placeholder={info?.def ?? 'C:\\Users\\…\\AppData\\Local\\NanoCrew\\Sync\\cache'}
        mono
      />
      <div style={{
        marginTop: 12, padding: '10px 12px',
        background: tok.surface1, border: `1px solid ${tok.border}`, borderRadius: 3,
        display: 'flex', alignItems: 'center', gap: 12,
      }}>
        <I.drive size={14} color={info?.isCustom ? tok.lime : tok.textLo} />
        <div style={{ flex: 1, minWidth: 0 }}>
          <div style={{ fontFamily: NC_FONT_MONO, fontSize: 10, letterSpacing: 1.2, color: tok.textLo, marginBottom: 2 }}>
            {info?.isCustom ? t('settings.cache.statusCustom') : t('settings.cache.statusDefault')}
          </div>
          <div style={{
            fontFamily: NC_FONT_MONO, fontSize: 12, color: tok.textHi,
            overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
          }}>
            {info?.effective ?? '—'}
          </div>
        </div>
        <NCBtn
          theme={theme} small ghost
          onClick={() => info && invoke('open_path', { token, path: info.effective }).catch(() => {})}
        >
          {t('settings.cache.openFolder')}
        </NCBtn>
      </div>
    </NCCard>
  );
};

interface LicenseStatus {
  tier: string;
  is_pro: boolean;
  expires_at: number;
  days_remaining: number;
  key_id: string | null;
  email: string | null;
  machine_fingerprint_short: string;
}

const LicenseCard: React.FC<{ theme: Theme; token: string }> = ({ theme, token }) => {
  const { t } = useTranslation();
  const tok = getTokens(theme);
  const [status, setStatus] = React.useState<LicenseStatus | null>(null);
  const [key, setKey] = React.useState('');
  const [busy, setBusy] = React.useState(false);
  const [err, setErr] = React.useState<string | null>(null);

  const refresh = React.useCallback(async () => {
    try {
      const s = await invoke<LicenseStatus>('get_license_status', { token });
      setStatus(s);
    } catch (e) {
      console.error('get_license_status failed', e);
    }
  }, [token]);

  React.useEffect(() => { refresh(); }, [refresh]);

  const activate = async () => {
    const jwt = key.trim();
    if (!jwt) return;
    setBusy(true); setErr(null);
    try {
      const s = await invoke<LicenseStatus>('activate_license', { token, licenseJwt: jwt });
      setStatus(s);
      setKey('');
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(false);
    }
  };

  const deactivate = async () => {
    setBusy(true); setErr(null);
    try {
      const s = await invoke<LicenseStatus>('deactivate_license', { token });
      setStatus(s);
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(false);
    }
  };

  if (!status) {
    return (
      <NCCard theme={theme} pad={20}>
        <NCEyebrow theme={theme}>{t('settings.license.title')}</NCEyebrow>
        <div style={{ fontSize: 12, color: tok.textMd, marginTop: 10 }}>{t('common.loading')}</div>
      </NCCard>
    );
  }

  const isActive = status.key_id !== null;
  const tierLabel = status.tier.toUpperCase();
  const badgeColor =
    status.tier === 'pro' || status.tier === 'team' ? tok.lime
    : status.tier === 'trial' ? '#f6c744'
    : tok.textLo;

  return (
    <NCCard theme={theme} pad={20}>
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 14 }}>
        <NCEyebrow theme={theme}>{t('settings.license.title')}</NCEyebrow>
        <span style={{
          fontFamily: NC_FONT_MONO, fontSize: 10, letterSpacing: 1.4,
          color: '#0A0A0A', background: badgeColor,
          padding: '3px 8px', borderRadius: 2, fontWeight: 600,
        }}>{tierLabel}</span>
      </div>

      {status.tier === 'trial' && (
        <div style={{ fontSize: 13, color: tok.textHi, marginBottom: 10 }}>
          {t('settings.license.trialDays', { count: status.days_remaining })}
        </div>
      )}
      {status.tier === 'free' && (
        <div style={{ fontSize: 13, color: tok.textHi, marginBottom: 10 }}>
          {t('settings.license.trialEnded')}
        </div>
      )}
      {isActive && (
        <div style={{ fontSize: 13, color: tok.textHi, marginBottom: 10 }}>
          {t('settings.license.thankYou')}{status.days_remaining > 0 && (
            <span style={{ color: tok.textMd }}>{' '}{t('settings.license.expiresIn', { count: status.days_remaining })}</span>
          )}
        </div>
      )}

      {isActive ? (
        <div style={{ display: 'grid', gap: 6, fontSize: 12, color: tok.textMd, marginBottom: 14 }}>
          <div><span style={{ color: tok.textLo }}>{t('settings.license.keyId')}</span> <span style={{ fontFamily: NC_FONT_MONO }}>{status.key_id}</span></div>
          {status.email && <div><span style={{ color: tok.textLo }}>{t('settings.license.email')}</span> {status.email}</div>}
          <div><span style={{ color: tok.textLo }}>{t('settings.license.thisMachine')}</span> <span style={{ fontFamily: NC_FONT_MONO }}>{status.machine_fingerprint_short}…</span></div>
        </div>
      ) : (
        <>
          <NCLabel theme={theme}>{t('settings.license.keyLabel')}</NCLabel>
          <textarea
            value={key}
            onChange={e => setKey(e.target.value)}
            placeholder={t('settings.license.keyPlaceholder')}
            rows={3}
            style={{
              width: '100%', boxSizing: 'border-box', resize: 'vertical',
              fontFamily: NC_FONT_MONO, fontSize: 11,
              background: tok.surface2, color: tok.textHi,
              border: `1px solid ${tok.border}`, borderRadius: 3,
              padding: '8px 10px', marginTop: 6, marginBottom: 10,
            }}
          />
          <div style={{ fontSize: 11, color: tok.textLo, marginBottom: 10 }}>
            {t('settings.license.thisMachine')} <span style={{ fontFamily: NC_FONT_MONO }}>{status.machine_fingerprint_short}…</span>
          </div>
        </>
      )}

      {err && (
        <div style={{
          fontSize: 12, color: '#f07f7f', background: 'rgba(240, 127, 127, 0.1)',
          border: '1px solid rgba(240, 127, 127, 0.3)', padding: '8px 10px',
          borderRadius: 3, marginBottom: 10,
        }}>{err}</div>
      )}

      <div style={{ display: 'flex', gap: 8, flexWrap: 'wrap' }}>
        {isActive ? (
          <NCBtn theme={theme} small ghost onClick={deactivate} disabled={busy}>
            {busy ? t('settings.license.deactivating') : t('settings.license.deactivate')}
          </NCBtn>
        ) : (
          <NCBtn theme={theme} small onClick={activate} disabled={busy || !key.trim()}>
            {busy ? t('settings.license.activating') : t('settings.license.activate')}
          </NCBtn>
        )}
        {!status.is_pro && (
          <NCBtn
            theme={theme} small ghost
            onClick={() => invoke('open_path', { token, path: 'https://nanocrew.dev/buy' }).catch(() => {})}
          >
            {t('settings.license.upgradePro')}
          </NCBtn>
        )}
      </div>
    </NCCard>
  );
};

/// "Contact Support" card. Builds a mailto: URL pre-filled with app version,
/// OS string, and the path to today's log file, then hands it to the shell
/// plugin which routes it to the user's default mail client.
const SupportCard: React.FC<{ theme: Theme; token: string; appVersion: string }> = ({ theme, token, appVersion }) => {
  const { t } = useTranslation();
  const tok = getTokens(theme);

  const handleClick = async () => {
    const today = new Date().toISOString().slice(0, 10);
    const osHint = (navigator.userAgent.match(/Windows NT [\d.]+/) ?? ['Windows'])[0];
    const body =
      `App: NanoCrew Sync v${appVersion || 'unknown'}\n` +
      `OS: ${osHint}\n` +
      `Drive ID: <none unless selected>\n\n` +
      `--- describe your issue below ---\n\n` +
      `--- log file path: %APPDATA%\\dev.nanocrew.sync\\logs\\nanocrew-sync.log.${today} ---`;
    const url =
      `mailto:nanosync@nanocrew.ai` +
      `?subject=${encodeURIComponent(`Support: NanoCrew Sync v${appVersion || 'unknown'}`)}` +
      `&body=${encodeURIComponent(body)}`;
    try {
      await invoke('open_path', { token, path: url });
    } catch (e) {
      console.error('open mailto failed', e);
    }
  };

  return (
    <NCCard theme={theme} pad={20}>
      <NCEyebrow theme={theme} style={{ marginBottom: 14 }}>{t('settings.about.support')}</NCEyebrow>
      <div style={{ display: 'flex', alignItems: 'center', gap: 14 }}>
        <div style={{
          width: 32, height: 32, borderRadius: 3, background: tok.surface2,
          border: `1px solid ${tok.border}`,
          display: 'flex', alignItems: 'center', justifyContent: 'center',
        }}>
          <I.cloud size={14} color={tok.textMd} />
        </div>
        <div style={{ flex: 1 }}>
          <div style={{ fontSize: 13, fontWeight: 500, color: tok.textHi }}>
            {t('settings.about.contactSupport.label')}
          </div>
          <div style={{ fontSize: 11, color: tok.textMd, marginTop: 2 }}>
            {t('settings.about.contactSupport.sub')}
          </div>
        </div>
        <NCBtn theme={theme} small ghost onClick={handleClick}>
          {t('settings.about.contactSupport.button')}
        </NCBtn>
      </div>
    </NCCard>
  );
};

const PlaceholderSection: React.FC<{ title: string; body: string; theme: Theme }> = ({ title, body, theme }) => {
  const { t } = useTranslation();
  const tok = getTokens(theme);
  return (
    <NCCard theme={theme} pad={24} style={{ display: 'flex', gap: 16, alignItems: 'flex-start' }}>
      <I.settings size={20} color={tok.textLo} style={{ marginTop: 2, flexShrink: 0 }} />
      <div>
        <div style={{ fontSize: 14, fontWeight: 600, color: tok.textHi, marginBottom: 6 }}>{title}</div>
        <div style={{ fontSize: 13, color: tok.textMd, lineHeight: 1.55 }}>{body}</div>
        <div style={{
          marginTop: 12, display: 'inline-block',
          fontFamily: NC_FONT_MONO, fontSize: 10, letterSpacing: 1.5,
          color: tok.textLo, background: tok.surface2,
          padding: '4px 8px', borderRadius: 2,
        }}>{t('settings.comingInFutureRelease')}</div>
      </div>
    </NCCard>
  );
};

const AdvancedSection: React.FC<{ theme: Theme; token: string }> = ({ theme, token }) => {
  const { t } = useTranslation();
  const tok = getTokens(theme);
  const [winfspStatus, setWinfspStatus] = React.useState<'checking' | 'installed' | 'missing'>('checking');

  React.useEffect(() => {
    invoke<boolean>('check_winfsp', { token })
      .then(ok => setWinfspStatus(ok ? 'installed' : 'missing'))
      .catch(() => setWinfspStatus('missing'));
  }, [token]);

  return <>
    <NCCard theme={theme} pad={20}>
      <NCEyebrow theme={theme} style={{ marginBottom: 14 }}>{t('settings.advanced.logging')}</NCEyebrow>
      <PrefToggle
        theme={theme} token={token}
        prefKey="verbose_logging"
        label={t('settings.advanced.verboseLogging.label')}
        sub={t('settings.advanced.verboseLogging.sub')}
      />
      <div style={{ marginTop: 14, display: 'flex', gap: 10 }}>
        <NCBtn
          theme={theme} small ghost
          onClick={async () => {
            try {
              const base = await appDataDir();
              const logs = await join(base, 'logs');
              await invoke('open_path', { token, path: logs });
            } catch (e) {
              console.error('open logs failed', e);
            }
          }}
        >
          {t('settings.advanced.openLogFolder')}
        </NCBtn>
      </div>
    </NCCard>
    <NCCard theme={theme} pad={20}>
      <NCEyebrow theme={theme} style={{ marginBottom: 14 }}>{t('settings.advanced.winfsp')}</NCEyebrow>
      <div style={{ fontSize: 13, color: tok.textMd, lineHeight: 1.55, marginBottom: 14 }}>
        {t('settings.advanced.winfspDesc')}
      </div>
      <div style={{
        display: 'flex', alignItems: 'center', gap: 12,
        padding: '10px 14px', background: tok.surface1,
        border: `1px solid ${winfspStatus === 'missing' ? tok.danger : tok.border}`,
        borderRadius: 3, marginBottom: 10,
      }}>
        <I.drive size={16} color={winfspStatus === 'installed' ? tok.lime : winfspStatus === 'missing' ? tok.danger : tok.textMd} />
        <div style={{ flex: 1 }}>
          <div style={{ fontFamily: NC_FONT_MONO, fontSize: 12, color: tok.textHi }}>WinFsp</div>
          <div style={{ fontFamily: NC_FONT_MONO, fontSize: 10, color: winfspStatus === 'installed' ? tok.lime : winfspStatus === 'missing' ? tok.danger : tok.textLo, letterSpacing: 1, marginTop: 2 }}>
            {winfspStatus === 'checking' ? t('settings.advanced.winfspChecking') : winfspStatus === 'installed' ? t('settings.advanced.winfspInstalled') : t('settings.advanced.winfspMissing')}
          </div>
        </div>
        {winfspStatus !== 'installed' && (
          <NCBtn
            theme={theme} small ghost
            onClick={() => invoke('open_path', { token, path: 'https://winfsp.dev/rel/' })}
          >
            {t('settings.advanced.winfspDownload')}
          </NCBtn>
        )}
      </div>
      {winfspStatus === 'missing' && (
        <div style={{ fontSize: 12, color: tok.textMd, lineHeight: 1.55 }}>
          {t('settings.advanced.winfspMissingDesc')}
        </div>
      )}
    </NCCard>
    <NCCard theme={theme} pad={20}>
      <NCEyebrow theme={theme} style={{ marginBottom: 14 }}>{t('settings.advanced.crashReporting')}</NCEyebrow>
      <PrefToggle
        theme={theme} token={token}
        prefKey="telemetry_enabled"
        defaultOn
        label={t('settings.advanced.crashReporting.label')}
        sub={t('settings.advanced.crashReporting.sub')}
        onAfterChange={(enabled) => {
          if (!enabled) Sentry.close().catch(() => {});
        }}
      />
    </NCCard>
  </>;
};

export const SettingsScreen: React.FC<SettingsScreenProps> = ({ theme, setTheme }) => {
  const { t } = useTranslation();
  const tok = getTokens(theme);
  const { token } = useAuth();
  const [activeSection, setActiveSection] = React.useState('General');
  const [cacheCleared, setCacheCleared] = React.useState(false);
  const [appVersion, setAppVersion] = React.useState('');
  React.useEffect(() => { getVersion().then(setAppVersion).catch(() => {}); }, []);

  const handleClearCache = async () => {
    try {
      await invoke('clear_cache', { token });
      setCacheCleared(true);
      setTimeout(() => setCacheCleared(false), 3000);
    } catch {}
  };

  const sections = [
    { key: 'General',         label: t('settings.section.general') },
    { key: 'Drives',          label: t('settings.section.drives') },
    { key: 'Network',         label: t('settings.section.network') },
    { key: 'Cache & storage', label: t('settings.section.cacheStorage') },
    { key: 'Security',        label: t('settings.section.security') },
    { key: 'Notifications',   label: t('settings.section.notifications') },
    { key: 'Advanced',        label: t('settings.section.advanced') },
    { key: 'About',           label: t('settings.section.about') },
  ];

  const renderContent = () => {
    switch (activeSection) {
      case 'General':
        return <>
          <NCCard theme={theme} pad={20}>
            <NCEyebrow theme={theme} style={{ marginBottom: 14 }}>{t('settings.startup.title')}</NCEyebrow>
            <AutostartRow theme={theme} token={token} />
            <Spacer />
            <PrefToggle
              theme={theme} token={token}
              prefKey="start_minimized"
              label={t('settings.startup.startMinimized.label')}
              sub={t('settings.startup.startMinimized.sub')}
            />
            <Spacer />
            <PrefToggle
              theme={theme} token={token}
              prefKey="auto_update_check"
              defaultOn
              label={t('settings.startup.autoUpdateCheck.label')}
              sub={t('settings.startup.autoUpdateCheck.sub')}
            />
          </NCCard>

          <NCCard theme={theme} pad={20}>
            <NCEyebrow theme={theme} style={{ marginBottom: 14 }}>{t('settings.appearance.title')}</NCEyebrow>
            <div style={{ marginBottom: 14 }}>
              <NCLabel theme={theme}>{t('settings.appearance.theme')}</NCLabel>
              <div style={{ display: 'flex', gap: 8 }}>
                {([
                  t('settings.appearance.themeDark'),
                  t('settings.appearance.themeLight'),
                  t('settings.appearance.themeSystem'),
                ] as const).map((v, i) => {
                  const active = (theme === 'dark' && i === 0) || (theme === 'light' && i === 1);
                  return (
                    <div key={v}
                      onClick={() => i < 2 && setTheme(i === 0 ? 'dark' : 'light')}
                      style={{
                        flex: 1, padding: '10px 12px', textAlign: 'center',
                        background: active ? tok.limeSoft : tok.surface1,
                        border: `1px solid ${active ? tok.lime : tok.border}`,
                        borderRadius: 3, color: tok.textHi, fontSize: 13, fontWeight: 500, cursor: 'pointer',
                      }}>{v}</div>
                  );
                })}
              </div>
            </div>
            <div>
              <NCLabel theme={theme}>{t('settings.appearance.accentColor')}</NCLabel>
              <div style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
                <div style={{ width: 28, height: 28, borderRadius: 3, background: tok.lime, border: `1px solid ${tok.lime}` }} />
                <span style={{ fontFamily: NC_FONT_MONO, fontSize: 12, color: tok.textHi }}>
                  {theme === 'dark' ? '#C8FF00 · CORTEX LIME' : '#3A5200 · OLIVE'}
                </span>
              </div>
            </div>
          </NCCard>

          <NCCard theme={theme} pad={20}>
            <NCEyebrow theme={theme} style={{ marginBottom: 14 }}>{t('settings.language.title')}</NCEyebrow>
            <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 16 }}>
              {[
                { label: t('settings.language.language'), value: 'English (South Africa)' },
                { label: t('settings.language.defaultRegion'), value: 'af-south-1', mono: true },
              ].map((f, i) => (
                <div key={i}>
                  <NCLabel theme={theme}>{f.label}</NCLabel>
                  <div style={{
                    display: 'flex', alignItems: 'center', padding: '10px 12px',
                    background: tok.surface1, border: `1px solid ${tok.border}`, borderRadius: 3,
                    fontSize: 13, color: tok.textHi,
                    fontFamily: f.mono ? NC_FONT_MONO : undefined,
                  }}>
                    <span style={{ flex: 1 }}>{f.value}</span>
                    <I.chevD size={13} color={tok.textMd} />
                  </div>
                </div>
              ))}
            </div>
          </NCCard>
        </>;

      case 'Drives':
        return <>
          <NCCard theme={theme} pad={20}>
            <NCEyebrow theme={theme} style={{ marginBottom: 14 }}>{t('settings.drives.defaults')}</NCEyebrow>
            <PrefToggle
              theme={theme} token={token}
              prefKey="default_auto_mount" defaultOn
              label={t('settings.drives.autoMount.label')}
              sub={t('settings.drives.autoMount.sub')}
            />
            <Spacer />
            <PrefToggle
              theme={theme} token={token}
              prefKey="default_readonly"
              label={t('settings.drives.readonly.label')}
              sub={t('settings.drives.readonly.sub')}
            />
          </NCCard>
          <PlaceholderSection
            theme={theme}
            title={t('settings.drives.perDriveOverrides.title')}
            body={t('settings.drives.perDriveOverrides.body')}
          />
        </>;

      case 'Network':
        return <>
          <NCCard theme={theme} pad={20}>
            <NCEyebrow theme={theme} style={{ marginBottom: 14 }}>{t('settings.network.bandwidth')}</NCEyebrow>
            <div style={{ fontSize: 12, color: tok.textMd, lineHeight: 1.55, marginBottom: 16 }}>
              {t('settings.network.bandwidthDesc')}
            </div>
            <PrefInput
              theme={theme} token={token}
              prefKey="upload_rate_mbps"
              label={t('settings.network.uploadLimit.label')}
              sub={t('settings.network.uploadLimit.sub')}
              placeholder={t('settings.network.uploadLimit.placeholder')}
              mono
            />
            <Spacer />
            <PrefInput
              theme={theme} token={token}
              prefKey="download_rate_mbps"
              label={t('settings.network.downloadLimit.label')}
              sub={t('settings.network.downloadLimit.sub')}
              placeholder={t('settings.network.downloadLimit.placeholder')}
              mono
            />
          </NCCard>
          <NCCard theme={theme} pad={20}>
            <NCEyebrow theme={theme} style={{ marginBottom: 14 }}>{t('settings.network.proxyTls')}</NCEyebrow>
            <div style={{ fontSize: 12, color: tok.textMd, lineHeight: 1.55, marginBottom: 16 }}>
              {t('settings.network.proxyTlsDesc')}
            </div>
            <PrefInput
              theme={theme} token={token}
              prefKey="proxy_url"
              label={t('settings.network.proxy.label')}
              sub={t('settings.network.proxy.sub')}
              placeholder={t('settings.network.proxy.placeholder')}
              mono
            />
            <Spacer />
            <PrefInput
              theme={theme} token={token}
              prefKey="custom_ca_pem_path"
              label={t('settings.network.ca.label')}
              sub={t('settings.network.ca.sub')}
              placeholder={t('settings.network.ca.placeholder')}
              mono
            />
          </NCCard>
        </>;

      case 'Cache & storage':
        return <>
          <NCCard theme={theme} pad={20}>
            <NCEyebrow theme={theme} style={{ marginBottom: 14 }}>{t('settings.cache.localCache')}</NCEyebrow>
            <div style={{ fontSize: 13, color: tok.textMd, marginBottom: 14, lineHeight: 1.55 }}>
              {t('settings.cache.localCacheDesc')}
            </div>
            <PrefToggle
              theme={theme} token={token}
              prefKey="cache_enabled" defaultOn
              label={t('settings.cache.enable.label')}
              sub={t('settings.cache.enable.sub')}
            />
            <Spacer />
            <div style={{ display: 'flex', alignItems: 'center', gap: 14, marginTop: 4 }}>
              <div style={{ flex: 1 }}>
                <div style={{ fontSize: 13, color: tok.textHi, fontWeight: 500 }}>{t('settings.cache.clearAll.label')}</div>
                <div style={{ fontSize: 11, color: tok.textMd, marginTop: 2 }}>
                  {cacheCleared ? <span style={{ color: tok.lime }}>{t('settings.cache.cleared')}</span> : t('settings.cache.clearAll.sub')}
                </div>
              </div>
              <NCBtn theme={theme} small ghost onClick={handleClearCache}>{t('settings.cache.clearBtn')}</NCBtn>
            </div>
          </NCCard>
          <CacheLocationCard theme={theme} token={token} />
        </>;

      case 'Security':
        return <>
          <NCCard theme={theme} pad={20}>
            <NCEyebrow theme={theme} style={{ marginBottom: 14 }}>{t('settings.security.session')}</NCEyebrow>
            <PrefToggle
              theme={theme} token={token}
              prefKey="lock_on_session_lock"
              label={t('settings.security.lockOnWindowsLock.label')}
              sub={t('settings.security.lockOnWindowsLock.sub')}
            />
            <Spacer />
            <ToggleRow
              theme={theme}
              label={t('settings.security.lockOnMinimize.label')}
              sub={t('settings.security.lockOnMinimize.sub')}
              on={readLockOnMinimize()}
              onChange={writeLockOnMinimize}
            />
          </NCCard>
          <NCCard theme={theme} pad={20}>
            <NCEyebrow theme={theme} style={{ marginBottom: 14 }}>{t('settings.security.credentialStorage')}</NCEyebrow>
            <div style={{ fontSize: 13, color: tok.textMd, lineHeight: 1.55 }}>
              {t('settings.security.credentialStorageDesc')}
            </div>
          </NCCard>
        </>;

      case 'Notifications':
        return <>
          <NCCard theme={theme} pad={20}>
            <NCEyebrow theme={theme} style={{ marginBottom: 14 }}>{t('settings.notifications.system')}</NCEyebrow>
            <PrefToggle
              theme={theme} token={token}
              prefKey="notify_mount_events" defaultOn
              label={t('settings.notifications.mountEvents.label')}
              sub={t('settings.notifications.mountEvents.sub')}
            />
            <Spacer />
            <PrefToggle
              theme={theme} token={token}
              prefKey="notify_errors" defaultOn
              label={t('settings.notifications.errors.label')}
              sub={t('settings.notifications.errors.sub')}
            />
            <Spacer />
            <PrefToggle
              theme={theme} token={token}
              prefKey="notify_uploads"
              label={t('settings.notifications.uploads.label')}
              sub={t('settings.notifications.uploads.sub')}
            />
            <Spacer />
            <ToggleRow theme={theme} label={t('settings.notifications.lowDisk.label')} comingSoon />
          </NCCard>
        </>;

      case 'Advanced':
        return <AdvancedSection theme={theme} token={token} />;

      case 'About':
        return <>
          <LicenseCard theme={theme} token={token} />
          <NCCard theme={theme} pad={24}>
            <div style={{ display: 'flex', gap: 20, alignItems: 'center', marginBottom: 20 }}>
              <div style={{
                width: 56, height: 56, borderRadius: 6,
                background: tok.lime, display: 'flex', alignItems: 'center', justifyContent: 'center',
              }}>
                <I.cloud size={28} color="#0A0A0A" />
              </div>
              <div>
                <div style={{ fontSize: 18, fontWeight: 700, color: tok.textHi, letterSpacing: -0.5 }}>{t('settings.about.title')}</div>
                <div style={{ fontFamily: NC_FONT_MONO, fontSize: 11, color: tok.textMd, letterSpacing: 1, marginTop: 4 }}>
                  VERSION {appVersion || '0.1.0'} · EARLY ACCESS
                </div>
              </div>
            </div>
            <div style={{ fontSize: 13, color: tok.textMd, lineHeight: 1.6 }}>
              Mount S3-compatible cloud storage (Wasabi, Amazon S3, Backblaze B2) as local Windows drive letters. No subscriptions. No data routing. Your credentials stay on your machine.
            </div>
            <UpdateButton theme={theme} />
          </NCCard>
          <NCCard theme={theme} pad={20}>
            <NCEyebrow theme={theme} style={{ marginBottom: 14 }}>{t('settings.about.builtWith')}</NCEyebrow>
            {[
              ['Tauri 2', 'Rust + WebView2 desktop shell'],
              ['WinFsp', 'User-mode Windows filesystem driver'],
              ['AWS SDK for Rust', 'S3-compatible object storage client'],
              ['React 18', 'Frontend UI framework'],
            ].map(([name, desc]) => (
              <div key={name} style={{
                display: 'flex', justifyContent: 'space-between', alignItems: 'baseline',
                padding: '8px 0', borderBottom: `1px solid ${tok.border}`,
              }}>
                <span style={{ fontSize: 13, color: tok.textHi, fontWeight: 500 }}>{name}</span>
                <span style={{ fontSize: 12, color: tok.textMd, fontFamily: NC_FONT_MONO }}>{desc}</span>
              </div>
            ))}
          </NCCard>
          <SupportCard theme={theme} token={token} appVersion={appVersion} />
        </>;

      default:
        return null;
    }
  };

  return (
    <>
      <TopBar
        theme={theme}
        crumbs={[t('settings.title')]}
        title={t('settings.title')}
        subtitle={t('settings.subtitle')}
      />
      <div style={{ flex: 1, display: 'flex', overflow: 'hidden' }}>
        <div style={{ width: 200, borderRight: `1px solid ${tok.border}`, padding: '20px 0', flexShrink: 0 }}>
          {sections.map(({ key: sKey, label }) => (
            <div key={sKey} onClick={() => setActiveSection(sKey)} style={{
              padding: '8px 20px', fontSize: 13,
              color: sKey === activeSection ? tok.textHi : tok.textMd,
              background: sKey === activeSection ? tok.surface2 : 'transparent',
              borderLeft: `2px solid ${sKey === activeSection ? tok.lime : 'transparent'}`,
              fontWeight: sKey === activeSection ? 500 : 400, cursor: 'pointer',
            }}>{label}</div>
          ))}
        </div>

        <div style={{ flex: 1, overflow: 'auto', padding: 28 }}>
          <div style={{ maxWidth: 640, display: 'flex', flexDirection: 'column', gap: 20 }}>
            {renderContent()}
          </div>
        </div>
      </div>
    </>
  );
};
