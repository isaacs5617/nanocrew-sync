// Full-screen lock overlay. Drives stay mounted while the app is locked —
// we don't drop the session token; we just require the user to re-prove
// identity before they can touch the UI again. The backend stays live, so
// Explorer can still read/write the mounted drives through WinFsp.

import React from 'react';
import { useTranslation } from 'react-i18next';
import { invoke } from '@tauri-apps/api/core';
import {
  getTokens, NC_FONT_DISPLAY, NC_FONT_MONO,
  NCCard, NCLabel, NCBtn, NCInput,
  type Theme,
} from '@nanocrew/ui';
import { I } from '@nanocrew/ui';

interface LockScreenProps {
  theme: Theme;
  token: string;
  username: string;
  onUnlock: () => void;
  onSignOut: () => void;
}

export const LockScreen: React.FC<LockScreenProps> = ({
  theme, token, username, onUnlock, onSignOut,
}) => {
  const tok = getTokens(theme);
  const { t } = useTranslation();
  const [password, setPassword] = React.useState('');
  const [busy, setBusy] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const inputRef = React.useRef<HTMLInputElement>(null);

  React.useEffect(() => {
    // Focus the password field on show — lock is keyboard-first.
    const id = window.setTimeout(() => inputRef.current?.focus(), 50);
    return () => window.clearTimeout(id);
  }, []);

  const tryUnlock = async () => {
    if (!password) { setError(t('lock.errors.required')); return; }
    setError(null);
    setBusy(true);
    try {
      await invoke('verify_password', { token, password });
      onUnlock();
    } catch {
      setError(t('lock.errors.incorrect'));
      setPassword('');
      inputRef.current?.focus();
    } finally {
      setBusy(false);
    }
  };

  return (
    <div style={{
      flex: 1, display: 'flex', alignItems: 'center', justifyContent: 'center',
      background: tok.bg, padding: 24,
    }}>
      <NCCard theme={theme} pad={40} style={{
        width: 400, display: 'flex', flexDirection: 'column', gap: 24,
      }}>
        <div style={{ textAlign: 'center' }}>
          <div style={{
            width: 64, height: 64, borderRadius: 4,
            background: tok.limeSoft, border: `1px solid ${tok.lime}`,
            display: 'inline-flex', alignItems: 'center', justifyContent: 'center',
            marginBottom: 16,
          }}>
            <I.lock size={28} color={tok.lime} />
          </div>
          <div style={{
            fontFamily: NC_FONT_DISPLAY, fontSize: 22, fontWeight: 800,
            color: tok.textHi, letterSpacing: -0.5, marginBottom: 4,
          }}>
            {t('lock.title')}
          </div>
          <div style={{ fontSize: 12, color: tok.textMd, lineHeight: 1.5 }}>
            {t('lock.subtitle')}
          </div>
        </div>

        <div>
          <NCLabel theme={theme}>{t('lock.signedInAs')}</NCLabel>
          <div style={{
            padding: '8px 12px',
            background: tok.surface2, border: `1px solid ${tok.border}`,
            borderRadius: 3, fontSize: 13, color: tok.textHi,
            fontFamily: NC_FONT_MONO,
          }}>
            {username || '—'}
          </div>
        </div>

        <div>
          <NCLabel theme={theme}>{t('lock.passwordLabel')}</NCLabel>
          <NCInput
            theme={theme}
            type="password"
            value={password}
            onChange={setPassword}
            placeholder="········"
            prefix={<I.lock size={13} />}
            inputRef={inputRef}
            onKeyDown={e => { if (e.key === 'Enter') tryUnlock(); }}
          />
        </div>

        {error && (
          <div style={{
            padding: '10px 14px',
            background: `${tok.danger}18`, border: `1px solid ${tok.danger}50`,
            borderRadius: 3, fontSize: 12, color: tok.danger,
            display: 'flex', gap: 8, alignItems: 'center',
          }}>
            <I.warn size={13} color={tok.danger} style={{ flexShrink: 0 }} />
            {error}
          </div>
        )}

        <div style={{ display: 'flex', gap: 8 }}>
          <NCBtn theme={theme} ghost onClick={onSignOut} disabled={busy}>{t('lock.signOut')}</NCBtn>
          <div style={{ flex: 1 }} />
          <NCBtn theme={theme} primary onClick={tryUnlock} disabled={busy}>
            {busy ? t('lock.unlocking') : t('lock.unlock')}
          </NCBtn>
        </div>
      </NCCard>
    </div>
  );
};
