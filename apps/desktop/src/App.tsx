import React from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { getVersion } from '@tauri-apps/api/app';
import { getTokens, type Theme } from '@nanocrew/ui';
import { AppShell } from '@nanocrew/ui';
import { TitleBar } from './TitleBar.js';
import { DashboardScreen } from './screens/DashboardScreen.js';
import { FileBrowserScreen } from './screens/FileBrowserScreen.js';
import { TransfersScreen } from './screens/TransfersScreen.js';
import { ActivityScreen } from './screens/ActivityScreen.js';
import { SettingsScreen } from './screens/SettingsScreen.js';
import { AccountScreen } from './screens/AccountScreen.js';
import { ErrorScreen } from './screens/ErrorScreen.js';
import { OnboardingScreen } from './screens/OnboardingScreen.js';
import { SetupScreen } from './screens/SetupScreen.js';
import { AddDrivePickerScreen } from './screens/AddDrivePickerScreen.js';
import { AddDriveS3Screen } from './screens/AddDriveS3Screen.js';
import { LockScreen } from './screens/LockScreen.js';
import { AuthContext } from './context/auth.js';
import { TransfersProvider } from './context/transfers.js';
import { useDriveNotifications } from './hooks/useDriveNotifications.js';
import type { NavKey } from '@nanocrew/ui';

type AppState = 'loading' | 'setup' | 'signin' | 'authed' | 'locked';

// localStorage key for the "lock on minimize" preference. Lives on the
// browser side because it's a UI-only setting and needs to be read before
// any backend call can happen.
const LOCK_ON_MINIMIZE_KEY = 'nanocrew.lockOnMinimize';
export const readLockOnMinimize = (): boolean =>
  window.localStorage.getItem(LOCK_ON_MINIMIZE_KEY) === '1';
export const writeLockOnMinimize = (on: boolean): void => {
  window.localStorage.setItem(LOCK_ON_MINIMIZE_KEY, on ? '1' : '0');
};

const NAV_TO_PATH: Record<NavKey, string> = {
  home:      '/drives',
  drives:    '/drives',
  files:     '/files',
  transfers: '/transfers',
  activity:  '/activity',
  account:   '/account',
  settings:  '/settings',
};

function ShellLayout({ theme, setTheme, token, onSignOut, version }: {
  theme: Theme;
  setTheme: (t: Theme) => void;
  token: string;
  onSignOut: () => void;
  version: string;
}) {
  const [activeNav, setActiveNav] = React.useState<NavKey>('drives');
  const [route, setRoute] = React.useState<string>('/drives');
  const [drives, setDrives] = React.useState<{ status: string }[]>([]);
  const t = getTokens(theme);

  // Windows action-centre toasts for mount/unmount/error + optional upload
  // complete. Runs once at the shell level so notifications fire regardless
  // of which screen the user is currently on.
  useDriveNotifications(token);

  // Load drive list and keep it in sync for sidebar counts
  React.useEffect(() => {
    invoke<{ status: string }[]>('list_drives', { token }).then(setDrives).catch(() => {});
  }, [token]);

  React.useEffect(() => {
    const unlisten = listen<{ drive_id: number; status: string }>('drive_status_changed', e => {
      setDrives(prev => prev.map((d, i) =>
        (d as any).id === e.payload.drive_id ? { ...d, status: e.payload.status } : d
      ));
    });
    return () => { unlisten.then(fn => fn()); };
  }, []);

  const navigate = (key: NavKey) => {
    setActiveNav(key);
    setRoute(NAV_TO_PATH[key]);
  };

  const renderScreen = () => {
    switch (route) {
      case '/drives':     return <DashboardScreen theme={theme} onAddDrive={() => setRoute('/add-drive')} />;
      case '/files':      return <FileBrowserScreen theme={theme} />;
      case '/transfers':  return <TransfersScreen theme={theme} />;
      case '/activity':   return <ActivityScreen theme={theme} />;
      case '/settings':   return <SettingsScreen theme={theme} setTheme={setTheme} />;
      case '/account':    return <AccountScreen theme={theme} onSignOut={onSignOut} />;
      case '/error':      return <ErrorScreen theme={theme} />;
      case '/add-drive':  return (
        <AddDrivePickerScreen
          theme={theme}
          onNext={(providerId) => setRoute(`/add-drive/${providerId}`)}
          onCancel={() => setRoute('/drives')}
        />
      );
      default:
        // Any `/add-drive/<providerId>` route renders the generic S3 screen.
        // The screen handles "unknown provider" itself, so we just pass the id.
        if (route.startsWith('/add-drive/')) {
          const providerId = route.slice('/add-drive/'.length);
          return (
            <AddDriveS3Screen
              theme={theme}
              providerId={providerId}
              onBack={() => setRoute('/add-drive')}
              onCancel={() => setRoute('/drives')}
              onDone={() => setRoute('/drives')}
            />
          );
        }
        return <DashboardScreen theme={theme} onAddDrive={() => setRoute('/add-drive')} />;
    }
  };

  return (
    <div style={{ display: 'flex', flex: 1, minHeight: 0, background: t.bg, overflow: 'hidden' }}>
      <AppShell
        theme={theme} activeNav={activeNav} onNav={navigate}
        driveCount={drives.length}
        errCount={drives.filter(d => d.status === 'error').length}
        version={version}
      >
        {renderScreen()}
      </AppShell>
    </div>
  );
}

export function App() {
  const [theme, setTheme] = React.useState<Theme>('dark');
  const [appState, setAppState] = React.useState<AppState>('loading');
  const [token, setToken] = React.useState<string>('');
  const [username, setUsername] = React.useState<string>('');
  const [version, setVersion] = React.useState<string>('');
  const t = getTokens(theme);

  React.useEffect(() => { getVersion().then(setVersion).catch(() => {}); }, []);

  // On mount: check whether an admin account exists yet.
  React.useEffect(() => {
    invoke<boolean>('has_account')
      .then(exists => setAppState(exists ? 'signin' : 'setup'))
      .catch(() => setAppState('signin'));
  }, []);

  // Session lock — listen for the Tauri window's "resize" (minimize fires
  // this with a zeroed inner size on Windows). We only lock when signed in
  // and when the user opted in via Settings. Drives stay mounted throughout;
  // the lock is purely UI-level.
  React.useEffect(() => {
    if (appState !== 'authed') return;
    if (!readLockOnMinimize()) return;

    let cancelled = false;
    (async () => {
      const { getCurrentWindow } = await import('@tauri-apps/api/window');
      const win = getCurrentWindow();
      const unlisten = await win.onResized(async () => {
        if (cancelled) return;
        try {
          if (await win.isMinimized()) {
            setAppState('locked');
            invoke('record_lock_event', { token, locked: true, reason: 'minimize' }).catch(() => {});
          }
        } catch { /* ignore */ }
      });
      return unlisten;
    })().catch(() => {});

    return () => { cancelled = true; };
  }, [appState]);

  const handleSetupDone = () => setAppState('signin');

  const handleSignIn = (tok: string) => {
    setToken(tok);
    setAppState('authed');
    invoke<{ username: string }>('get_account', { token: tok })
      .then(a => setUsername(a.username))
      .catch(() => {});
  };

  const handleSignOut = async () => {
    if (token) await invoke('sign_out', { token }).catch(() => {});
    setToken('');
    setUsername('');
    setAppState('signin');
  };

  const handleLock = () => {
    if (appState !== 'authed') return;
    setAppState('locked');
    invoke('record_lock_event', { token, locked: true, reason: 'manual' }).catch(() => {});
  };
  const handleUnlock = () => {
    if (appState !== 'locked') return;
    setAppState('authed');
    invoke('record_lock_event', { token, locked: false, reason: null }).catch(() => {});
  };

  const renderContent = () => {
    switch (appState) {
      case 'loading':
        return (
          <div style={{ flex: 1, display: 'flex', alignItems: 'center', justifyContent: 'center', background: t.bg }}>
            <div style={{ fontFamily: 'monospace', fontSize: 12, color: t.textLo, letterSpacing: 2 }}>LOADING…</div>
          </div>
        );
      case 'setup':
        return <SetupScreen theme={theme} onDone={handleSetupDone} />;
      case 'signin':
        return <OnboardingScreen theme={theme} onSignIn={handleSignIn} />;
      case 'authed':
        return (
          <AuthContext.Provider value={{ token, signOut: handleSignOut, lock: handleLock }}>
            <TransfersProvider>
              <ShellLayout theme={theme} setTheme={setTheme} token={token} onSignOut={handleSignOut} version={version} />
            </TransfersProvider>
          </AuthContext.Provider>
        );
      case 'locked':
        return (
          <LockScreen
            theme={theme}
            token={token}
            username={username}
            onUnlock={handleUnlock}
            onSignOut={handleSignOut}
          />
        );
    }
  };

  return (
    <div style={{
      display: 'flex', flexDirection: 'column',
      height: '100vh', overflow: 'hidden',
      background: t.bg,
    }}>
      <TitleBar theme={theme} />
      <div style={{ flex: 1, display: 'flex', minHeight: 0, overflow: 'hidden' }}>
        {renderContent()}
      </div>
    </div>
  );
}
