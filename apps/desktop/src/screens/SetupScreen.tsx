import React from 'react';
import { invoke } from '@tauri-apps/api/core';
import {
  getTokens, NC_FONT_DISPLAY, NC_FONT_MONO, NC_FONT_UI,
  NCWordmark, NCEyebrow, NCInput, NCBtn,
  type Theme,
} from '@nanocrew/ui';
import { I } from '@nanocrew/ui';

interface SetupScreenProps {
  theme: Theme;
  onDone: () => void;
}

export const SetupScreen: React.FC<SetupScreenProps> = ({ theme, onDone }) => {
  const t = getTokens(theme);
  const [username, setUsername] = React.useState('');
  const [password, setPassword] = React.useState('');
  const [confirm, setConfirm] = React.useState('');
  const [showPw, setShowPw] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const [busy, setBusy] = React.useState(false);

  const handleCreate = async () => {
    setError(null);
    if (!username.trim()) { setError('Username is required.'); return; }
    if (password.length < 8) { setError('Password must be at least 8 characters.'); return; }
    if (password !== confirm) { setError('Passwords do not match.'); return; }

    setBusy(true);
    try {
      await invoke('create_admin', { username: username.trim(), password });
      onDone();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div
      style={{ flex: 1, display: 'flex', background: t.bg, fontFamily: NC_FONT_UI }}
      onKeyDown={e => { if (e.key === 'Enter') handleCreate(); }}
    >
      {/* Left branding panel */}
      <div style={{
        width: '46%', background: t.surface1,
        borderRight: `1px solid ${t.border}`,
        padding: 48, display: 'flex', flexDirection: 'column',
      }}>
        <div style={{ marginBottom: 48 }}>
          <NCWordmark dark={theme === 'dark'} size={22} />
        </div>
        <div style={{ flex: 1, display: 'flex', flexDirection: 'column', justifyContent: 'center', maxWidth: 420 }}>
          <NCEyebrow theme={theme} accent style={{ marginBottom: 20 }}>FIRST RUN · SETUP</NCEyebrow>
          <div style={{
            fontFamily: NC_FONT_DISPLAY, fontWeight: 800,
            fontSize: 44, lineHeight: 0.95, letterSpacing: -2,
            color: t.textHi, marginBottom: 24,
          }}>
            Create your <span style={{ color: t.lime }}>admin</span> account.
          </div>
          <div style={{ fontSize: 15, lineHeight: 1.6, color: t.textMd, marginBottom: 32 }}>
            This account is stored locally on this machine. It controls access to NanoCrew Sync and all mounted drives.
          </div>
          <div style={{ display: 'flex', flexDirection: 'column', gap: 14 }}>
            {[
              { n: '01', text: 'Credentials never leave this device' },
              { n: '02', text: 'Password hashed with Argon2id' },
              { n: '03', text: 'S3 keys stored in Windows Credential Manager' },
            ].map((l, i) => (
              <div key={i} style={{ display: 'flex', gap: 14, alignItems: 'baseline' }}>
                <span style={{ fontFamily: NC_FONT_MONO, fontSize: 11, color: t.lime, letterSpacing: 1.5 }}>{l.n}</span>
                <span style={{ fontSize: 13, color: t.textHi }}>{l.text}</span>
              </div>
            ))}
          </div>
        </div>
        <div style={{ fontFamily: NC_FONT_MONO, fontSize: 10, color: t.textLo, letterSpacing: 1.5 }}>
          NANOCREW · CAPE TOWN · CORTEX · SYNC
        </div>
      </div>

      {/* Right form panel */}
      <div style={{ flex: 1, padding: 48, display: 'flex', flexDirection: 'column', justifyContent: 'center' }}>
        <div style={{ maxWidth: 380, margin: '0 auto', width: '100%' }}>
          <div style={{
            fontFamily: NC_FONT_DISPLAY, fontWeight: 800,
            fontSize: 26, letterSpacing: -0.8, color: t.textHi, marginBottom: 8,
          }}>Set up your account</div>
          <div style={{ fontSize: 13, color: t.textMd, marginBottom: 28 }}>
            You only need to do this once. Additional users can be added later.
          </div>

          <div style={{ display: 'flex', flexDirection: 'column', gap: 14, marginBottom: 20 }}>
            <div>
              <div style={{ fontFamily: NC_FONT_MONO, fontSize: 9, fontWeight: 500, letterSpacing: 2, textTransform: 'uppercase', color: t.textMd, marginBottom: 8 }}>Username</div>
              <NCInput theme={theme} value={username} onChange={setUsername} prefix={<I.user size={13} />} />
            </div>
            <div>
              <div style={{ fontFamily: NC_FONT_MONO, fontSize: 9, fontWeight: 500, letterSpacing: 2, textTransform: 'uppercase', color: t.textMd, marginBottom: 8 }}>Password</div>
              <NCInput
                theme={theme}
                type={showPw ? 'text' : 'password'}
                value={password}
                onChange={setPassword}
                prefix={<I.lock size={13} />}
                suffix={<span style={{ cursor: 'pointer' }} onClick={() => setShowPw(v => !v)}>{showPw ? <I.eyeOff size={14} /> : <I.eye size={14} />}</span>}
              />
            </div>
            <div>
              <div style={{ fontFamily: NC_FONT_MONO, fontSize: 9, fontWeight: 500, letterSpacing: 2, textTransform: 'uppercase', color: t.textMd, marginBottom: 8 }}>Confirm password</div>
              <NCInput theme={theme} type="password" value={confirm} onChange={setConfirm} prefix={<I.lock size={13} />} />
            </div>
          </div>

          {error && (
            <div style={{
              padding: '10px 12px', marginBottom: 16,
              background: `${t.danger}18`, border: `1px solid ${t.danger}50`,
              borderRadius: 3, fontSize: 12, color: t.danger,
              display: 'flex', gap: 8, alignItems: 'flex-start',
            }}>
              <I.warn size={13} color={t.danger} style={{ marginTop: 1, flexShrink: 0 }} />
              {error}
            </div>
          )}

          <NCBtn
            theme={theme}
            primary
            style={{ width: '100%', justifyContent: 'center' }}
            onClick={handleCreate}
            disabled={busy}
          >
            {busy ? 'Creating account…' : 'Create account'}
          </NCBtn>
        </div>
      </div>
    </div>
  );
};
