import React from 'react';
import { invoke } from '@tauri-apps/api/core';
import { getVersion } from '@tauri-apps/api/app';
import { appDataDir, join } from '@tauri-apps/api/path';
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
  const t = getTokens(theme);
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
          <div style={{ fontSize: 13, color: t.textHi, fontWeight: 500 }}>{label}</div>
          {comingSoon && (
            <span style={{
              fontFamily: NC_FONT_MONO, fontSize: 9, letterSpacing: 1.2,
              color: t.textLo, background: t.surface2,
              padding: '2px 6px', borderRadius: 2,
            }}>COMING SOON</span>
          )}
        </div>
        {sub && <div style={{ fontSize: 11, color: t.textMd, marginTop: 2 }}>{sub}</div>}
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
  const t = getTokens(theme);
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
      <div style={{ fontSize: 13, color: t.textHi, fontWeight: 500 }}>{label}</div>
      {sub && <div style={{ fontSize: 11, color: t.textMd, marginTop: 2, marginBottom: 8 }}>{sub}</div>}
      <input
        type="text"
        value={value}
        placeholder={placeholder}
        onChange={e => setValue(e.target.value)}
        style={{
          width: '100%', boxSizing: 'border-box',
          padding: '10px 12px',
          background: t.surface1,
          border: `1px solid ${t.border}`,
          borderRadius: 3, outline: 'none',
          color: t.textHi, fontSize: 13,
          fontFamily: mono ? NC_FONT_MONO : undefined,
        }}
      />
    </div>
  );
};

/// "Launch at Windows sign-in" toggle backed by the HKCU\...\Run registry
/// key. Reads the current state on mount; writes through set_autostart.
const AutostartRow: React.FC<{ theme: Theme; token: string }> = ({ theme, token }) => {
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
        label="Launch NanoCrew Sync at Windows sign-in"
        sub="Reconnect mounted drives automatically."
      />
    );
  }
  return (
    <ToggleRow
      theme={theme}
      label="Launch NanoCrew Sync at Windows sign-in"
      sub="Reconnect mounted drives automatically. Current-user only — no admin rights needed."
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

const PlaceholderSection: React.FC<{ title: string; body: string; theme: Theme }> = ({ title, body, theme }) => {
  const t = getTokens(theme);
  return (
    <NCCard theme={theme} pad={24} style={{ display: 'flex', gap: 16, alignItems: 'flex-start' }}>
      <I.settings size={20} color={t.textLo} style={{ marginTop: 2, flexShrink: 0 }} />
      <div>
        <div style={{ fontSize: 14, fontWeight: 600, color: t.textHi, marginBottom: 6 }}>{title}</div>
        <div style={{ fontSize: 13, color: t.textMd, lineHeight: 1.55 }}>{body}</div>
        <div style={{
          marginTop: 12, display: 'inline-block',
          fontFamily: NC_FONT_MONO, fontSize: 10, letterSpacing: 1.5,
          color: t.textLo, background: t.surface2,
          padding: '4px 8px', borderRadius: 2,
        }}>COMING IN A FUTURE RELEASE</div>
      </div>
    </NCCard>
  );
};

const AdvancedSection: React.FC<{ theme: Theme; token: string }> = ({ theme, token }) => {
  const t = getTokens(theme);
  const [winfspStatus, setWinfspStatus] = React.useState<'checking' | 'installed' | 'missing'>('checking');

  React.useEffect(() => {
    invoke<boolean>('check_winfsp', { token })
      .then(ok => setWinfspStatus(ok ? 'installed' : 'missing'))
      .catch(() => setWinfspStatus('missing'));
  }, [token]);

  return <>
    <NCCard theme={theme} pad={20}>
      <NCEyebrow theme={theme} style={{ marginBottom: 14 }}>Logging</NCEyebrow>
      <PrefToggle
        theme={theme} token={token}
        prefKey="verbose_logging"
        label="Enable verbose logging"
        sub="Writes detailed (debug-level) logs to %APPDATA%\NanoCrew\Sync\logs. Takes effect on next launch."
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
          Open log folder
        </NCBtn>
      </div>
    </NCCard>
    <NCCard theme={theme} pad={20}>
      <NCEyebrow theme={theme} style={{ marginBottom: 14 }}>WinFsp</NCEyebrow>
      <div style={{ fontSize: 13, color: t.textMd, lineHeight: 1.55, marginBottom: 14 }}>
        NanoCrew Sync uses WinFsp to mount cloud buckets as Windows drive letters. WinFsp is installed automatically alongside NanoCrew Sync.
      </div>
      <div style={{
        display: 'flex', alignItems: 'center', gap: 12,
        padding: '10px 14px', background: t.surface1,
        border: `1px solid ${winfspStatus === 'missing' ? t.danger : t.border}`,
        borderRadius: 3, marginBottom: 10,
      }}>
        <I.drive size={16} color={winfspStatus === 'installed' ? t.lime : winfspStatus === 'missing' ? t.danger : t.textMd} />
        <div style={{ flex: 1 }}>
          <div style={{ fontFamily: NC_FONT_MONO, fontSize: 12, color: t.textHi }}>WinFsp</div>
          <div style={{ fontFamily: NC_FONT_MONO, fontSize: 10, color: winfspStatus === 'installed' ? t.lime : winfspStatus === 'missing' ? t.danger : t.textLo, letterSpacing: 1, marginTop: 2 }}>
            {winfspStatus === 'checking' ? 'CHECKING…' : winfspStatus === 'installed' ? 'INSTALLED · READY' : 'NOT DETECTED'}
          </div>
        </div>
        {winfspStatus !== 'installed' && (
          <NCBtn
            theme={theme} small ghost
            onClick={() => invoke('open_path', { token, path: 'https://winfsp.dev/rel/' })}
          >
            Download
          </NCBtn>
        )}
      </div>
      {winfspStatus === 'missing' && (
        <div style={{ fontSize: 12, color: t.textMd, lineHeight: 1.55 }}>
          WinFsp was not found on this machine. Download and install it, then restart NanoCrew Sync. Drive mounting will not work until WinFsp is installed.
        </div>
      )}
    </NCCard>
  </>;
};

export const SettingsScreen: React.FC<SettingsScreenProps> = ({ theme, setTheme }) => {
  const t = getTokens(theme);
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
  const sections = ['General', 'Drives', 'Network', 'Cache & storage', 'Security', 'Notifications', 'Advanced', 'About'];

  const renderContent = () => {
    switch (activeSection) {
      case 'General':
        return <>
          <NCCard theme={theme} pad={20}>
            <NCEyebrow theme={theme} style={{ marginBottom: 14 }}>Startup</NCEyebrow>
            <AutostartRow theme={theme} token={token} />
            <Spacer />
            <PrefToggle
              theme={theme} token={token}
              prefKey="start_minimized"
              label="Start minimized to system tray"
              sub="Launch into the tray on startup — the window stays hidden until you click the tray icon."
            />
            <Spacer />
            <PrefToggle
              theme={theme} token={token}
              prefKey="auto_update_check"
              defaultOn
              label="Check for updates automatically"
              sub="Check Cortex Labs for new releases at launch. Installs still require your confirmation."
            />
          </NCCard>

          <NCCard theme={theme} pad={20}>
            <NCEyebrow theme={theme} style={{ marginBottom: 14 }}>Appearance</NCEyebrow>
            <div style={{ marginBottom: 14 }}>
              <NCLabel theme={theme}>Theme</NCLabel>
              <div style={{ display: 'flex', gap: 8 }}>
                {(['Dark', 'Light', 'Match system'] as const).map((v, i) => {
                  const active = (theme === 'dark' && i === 0) || (theme === 'light' && i === 1);
                  return (
                    <div key={v}
                      onClick={() => i < 2 && setTheme(i === 0 ? 'dark' : 'light')}
                      style={{
                        flex: 1, padding: '10px 12px', textAlign: 'center',
                        background: active ? t.limeSoft : t.surface1,
                        border: `1px solid ${active ? t.lime : t.border}`,
                        borderRadius: 3, color: t.textHi, fontSize: 13, fontWeight: 500, cursor: 'pointer',
                      }}>{v}</div>
                  );
                })}
              </div>
            </div>
            <div>
              <NCLabel theme={theme}>Accent color</NCLabel>
              <div style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
                <div style={{ width: 28, height: 28, borderRadius: 3, background: t.lime, border: `1px solid ${t.lime}` }} />
                <span style={{ fontFamily: NC_FONT_MONO, fontSize: 12, color: t.textHi }}>
                  {theme === 'dark' ? '#C8FF00 · CORTEX LIME' : '#3A5200 · OLIVE'}
                </span>
              </div>
            </div>
          </NCCard>

          <NCCard theme={theme} pad={20}>
            <NCEyebrow theme={theme} style={{ marginBottom: 14 }}>Language & region</NCEyebrow>
            <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 16 }}>
              {[
                { label: 'Language', value: 'English (South Africa)' },
                { label: 'Default region', value: 'af-south-1', mono: true },
              ].map((f, i) => (
                <div key={i}>
                  <NCLabel theme={theme}>{f.label}</NCLabel>
                  <div style={{
                    display: 'flex', alignItems: 'center', padding: '10px 12px',
                    background: t.surface1, border: `1px solid ${t.border}`, borderRadius: 3,
                    fontSize: 13, color: t.textHi,
                    fontFamily: f.mono ? NC_FONT_MONO : undefined,
                  }}>
                    <span style={{ flex: 1 }}>{f.value}</span>
                    <I.chevD size={13} color={t.textMd} />
                  </div>
                </div>
              ))}
            </div>
          </NCCard>
        </>;

      case 'Drives':
        return <>
          <NCCard theme={theme} pad={20}>
            <NCEyebrow theme={theme} style={{ marginBottom: 14 }}>Defaults for new drives</NCEyebrow>
            <PrefToggle
              theme={theme} token={token}
              prefKey="default_auto_mount" defaultOn
              label="Auto-mount on startup"
              sub="Pre-selects the auto-mount toggle for new drives added via Add Drive."
            />
            <Spacer />
            <PrefToggle
              theme={theme} token={token}
              prefKey="default_readonly"
              label="Read-only by default"
              sub="Pre-selects read-only for new drives. Prevents accidental writes."
            />
          </NCCard>
          <PlaceholderSection
            theme={theme}
            title="Per-drive overrides"
            body="Set cache quotas, bandwidth limits, and sync schedules per drive. Individual drive settings are configured from the drive's context menu on the Drives screen."
          />
        </>;

      case 'Network':
        return <>
          <NCCard theme={theme} pad={20}>
            <NCEyebrow theme={theme} style={{ marginBottom: 14 }}>Bandwidth</NCEyebrow>
            <ToggleRow theme={theme} label="Limit upload speed" sub="Throttle uploads so NanoCrew Sync doesn't saturate your connection." comingSoon />
            <Spacer />
            <ToggleRow theme={theme} label="Limit download speed" comingSoon />
          </NCCard>
          <NCCard theme={theme} pad={20}>
            <NCEyebrow theme={theme} style={{ marginBottom: 14 }}>Proxy & TLS</NCEyebrow>
            <div style={{ fontSize: 12, color: t.textMd, lineHeight: 1.55, marginBottom: 16 }}>
              Route all S3 traffic through a corporate HTTPS proxy and/or trust an extra root CA certificate. Changes apply to new mounts and test connections — remount a drive to pick them up.
            </div>
            <PrefInput
              theme={theme} token={token}
              prefKey="proxy_url"
              label="HTTPS proxy"
              sub="e.g. http://proxy.corp:8080 or http://user:pass@proxy.corp:8080. Leave blank to connect directly."
              placeholder="http://proxy.example.com:8080"
              mono
            />
            <Spacer />
            <PrefInput
              theme={theme} token={token}
              prefKey="custom_ca_pem_path"
              label="Custom CA certificate (PEM)"
              sub="Absolute path to a .pem file containing one or more trusted root certificates. Added alongside OS roots and SSL_CERT_FILE."
              placeholder="C:\\path\\to\\corp-root-ca.pem"
              mono
            />
          </NCCard>
        </>;

      case 'Cache & storage':
        return <>
          <NCCard theme={theme} pad={20}>
            <NCEyebrow theme={theme} style={{ marginBottom: 14 }}>Local cache</NCEyebrow>
            <div style={{ fontSize: 13, color: t.textMd, marginBottom: 14, lineHeight: 1.55 }}>
              NanoCrew Sync caches S3 metadata in memory with a 5-second TTL. Temp files used during uploads are deleted automatically after each write.
            </div>
            <ToggleRow theme={theme} label="Automatically evict stale cache entries" comingSoon />
            <Spacer />
            <div style={{ display: 'flex', alignItems: 'center', gap: 14, marginTop: 4 }}>
              <div style={{ flex: 1 }}>
                <div style={{ fontSize: 13, color: t.textHi, fontWeight: 500 }}>Clear all cached data</div>
                <div style={{ fontSize: 11, color: t.textMd, marginTop: 2 }}>
                  {cacheCleared ? <span style={{ color: t.lime }}>Cache cleared.</span> : 'Mounted drives will re-fetch metadata on next access.'}
                </div>
              </div>
              <NCBtn theme={theme} small ghost onClick={handleClearCache}>Clear cache</NCBtn>
            </div>
          </NCCard>
          <PlaceholderSection
            theme={theme}
            title="Cache location"
            body="Choose where NanoCrew Sync stores its local cache. By default this is %LOCALAPPDATA%\NanoCrew\Sync\cache."
          />
        </>;

      case 'Security':
        return <>
          <NCCard theme={theme} pad={20}>
            <NCEyebrow theme={theme} style={{ marginBottom: 14 }}>Session</NCEyebrow>
            <PrefToggle
              theme={theme} token={token}
              prefKey="lock_on_session_lock"
              label="Require password after Windows lock"
              sub="Re-authenticate when Windows resumes from sleep or the Win+L lock screen."
            />
            <Spacer />
            <ToggleRow
              theme={theme}
              label="Lock app when minimized"
              sub="Require your password to unlock. Drives stay mounted — files stay accessible in Explorer."
              on={readLockOnMinimize()}
              onChange={writeLockOnMinimize}
            />
          </NCCard>
          <NCCard theme={theme} pad={20}>
            <NCEyebrow theme={theme} style={{ marginBottom: 14 }}>Credential storage</NCEyebrow>
            <div style={{ fontSize: 13, color: t.textMd, lineHeight: 1.55 }}>
              S3 secret keys are stored in the <strong style={{ color: t.textHi }}>local SQLite database</strong> in your user app-data directory, protected by Windows file-system permissions. Your admin password is hashed with <strong style={{ color: t.textHi }}>Argon2id</strong> and never stored in plaintext.
            </div>
          </NCCard>
        </>;

      case 'Notifications':
        return <>
          <NCCard theme={theme} pad={20}>
            <NCEyebrow theme={theme} style={{ marginBottom: 14 }}>System notifications</NCEyebrow>
            <PrefToggle
              theme={theme} token={token}
              prefKey="notify_mount_events" defaultOn
              label="Drive mounted / unmounted"
              sub="Native Windows toast when a drive becomes available or is unmounted."
            />
            <Spacer />
            <PrefToggle
              theme={theme} token={token}
              prefKey="notify_errors" defaultOn
              label="Errors (mount + upload)"
              sub="Surface WinFsp mount failures and upload errors as toasts."
            />
            <Spacer />
            <PrefToggle
              theme={theme} token={token}
              prefKey="notify_uploads"
              label="Upload completed"
              sub="Off by default — large file transfers can get noisy."
            />
            <Spacer />
            <ToggleRow theme={theme} label="Low disk space warning" comingSoon />
          </NCCard>
        </>;

      case 'Advanced':
        return <AdvancedSection theme={theme} token={token} />;

      case 'About':
        return <>
          <NCCard theme={theme} pad={24}>
            <div style={{ display: 'flex', gap: 20, alignItems: 'center', marginBottom: 20 }}>
              <div style={{
                width: 56, height: 56, borderRadius: 6,
                background: t.lime, display: 'flex', alignItems: 'center', justifyContent: 'center',
              }}>
                <I.cloud size={28} color="#0A0A0A" />
              </div>
              <div>
                <div style={{ fontSize: 18, fontWeight: 700, color: t.textHi, letterSpacing: -0.5 }}>NanoCrew Sync</div>
                <div style={{ fontFamily: NC_FONT_MONO, fontSize: 11, color: t.textMd, letterSpacing: 1, marginTop: 4 }}>
                  VERSION {appVersion || '0.1.0'} · EARLY ACCESS
                </div>
              </div>
            </div>
            <div style={{ fontSize: 13, color: t.textMd, lineHeight: 1.6 }}>
              Mount S3-compatible cloud storage (Wasabi, Amazon S3, Backblaze B2) as local Windows drive letters. No subscriptions. No data routing. Your credentials stay on your machine.
            </div>
            <UpdateButton theme={theme} />
          </NCCard>
          <NCCard theme={theme} pad={20}>
            <NCEyebrow theme={theme} style={{ marginBottom: 14 }}>Built with</NCEyebrow>
            {[
              ['Tauri 2', 'Rust + WebView2 desktop shell'],
              ['WinFsp', 'User-mode Windows filesystem driver'],
              ['AWS SDK for Rust', 'S3-compatible object storage client'],
              ['React 18', 'Frontend UI framework'],
            ].map(([name, desc]) => (
              <div key={name} style={{
                display: 'flex', justifyContent: 'space-between', alignItems: 'baseline',
                padding: '8px 0', borderBottom: `1px solid ${t.border}`,
              }}>
                <span style={{ fontSize: 13, color: t.textHi, fontWeight: 500 }}>{name}</span>
                <span style={{ fontSize: 12, color: t.textMd, fontFamily: NC_FONT_MONO }}>{desc}</span>
              </div>
            ))}
          </NCCard>
        </>;

      default:
        return null;
    }
  };

  return (
    <>
      <TopBar
        theme={theme}
        crumbs={['Settings']}
        title="Preferences"
        subtitle="Local settings · stored in Windows Credential Manager and app data directory"
      />
      <div style={{ flex: 1, display: 'flex', overflow: 'hidden' }}>
        <div style={{ width: 200, borderRight: `1px solid ${t.border}`, padding: '20px 0', flexShrink: 0 }}>
          {sections.map((n) => (
            <div key={n} onClick={() => setActiveSection(n)} style={{
              padding: '8px 20px', fontSize: 13,
              color: n === activeSection ? t.textHi : t.textMd,
              background: n === activeSection ? t.surface2 : 'transparent',
              borderLeft: `2px solid ${n === activeSection ? t.lime : 'transparent'}`,
              fontWeight: n === activeSection ? 500 : 400, cursor: 'pointer',
            }}>{n}</div>
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
