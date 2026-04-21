import React from 'react';
import { invoke } from '@tauri-apps/api/core';
import {
  getTokens, NC_FONT_DISPLAY, NC_FONT_MONO, NC_FONT_UI,
  NCWordmark, NCEyebrow, NCInput, NCBtn,
  type Theme,
} from '@nanocrew/ui';
import { I } from '@nanocrew/ui';

interface OnboardingScreenProps {
  theme: Theme;
  onSignIn: (token: string) => void;
}

export const OnboardingScreen: React.FC<OnboardingScreenProps> = ({ theme, onSignIn }) => {
  const t = getTokens(theme);
  const [username, setUsername] = React.useState('');
  const [password, setPassword] = React.useState('');
  const [showPw, setShowPw] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const [busy, setBusy] = React.useState(false);

  const handleSignIn = async () => {
    setError(null);
    if (!username.trim() || !password) { setError('Username and password are required.'); return; }

    setBusy(true);
    try {
      const token = await invoke<string>('sign_in', { username: username.trim(), password });
      onSignIn(token);
    } catch (e) {
      setError('Invalid username or password.');
    } finally {
      setBusy(false);
    }
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'Enter') handleSignIn();
  };

  return (
    <div style={{ flex: 1, display: 'flex', background: t.bg, fontFamily: NC_FONT_UI }}>
      {/* Left panel */}
      <div style={{
        width: '46%', background: t.surface1,
        borderRight: `1px solid ${t.border}`,
        padding: 48, display: 'flex', flexDirection: 'column',
      }}>
        <div style={{ marginBottom: 48 }}>
          <NCWordmark dark={theme === 'dark'} size={22} />
        </div>
        <div style={{ flex: 1, display: 'flex', flexDirection: 'column', justifyContent: 'center', maxWidth: 420 }}>
          <NCEyebrow theme={theme} accent style={{ marginBottom: 20 }}>WELCOME · EARLY ACCESS</NCEyebrow>
          <div style={{
            fontFamily: NC_FONT_DISPLAY, fontWeight: 800,
            fontSize: 52, lineHeight: 0.95, letterSpacing: -2,
            color: t.textHi, marginBottom: 24,
          }}>
            Mount any <span style={{ color: t.lime }}>bucket</span> as a Windows drive.
          </div>
          <div style={{ fontSize: 15, lineHeight: 1.6, color: t.textMd, marginBottom: 32 }}>
            NanoCrew Sync turns Wasabi, S3 and other S3-compatible stores into native drive letters.
            Open files in any app, stream large media, and keep your credentials in the OS keychain.
          </div>
          <div style={{ display: 'flex', flexDirection: 'column', gap: 14 }}>
            {[
              { n: '01', text: 'Mounts like Z:\\ — every Windows app sees it as local' },
              { n: '02', text: 'Bring-your-own storage — we never touch your data' },
              { n: '03', text: 'Free for everyone, forever during beta' },
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

      {/* Right panel */}
      <div style={{ flex: 1, padding: 48, display: 'flex', flexDirection: 'column', justifyContent: 'center' }}>
        <div style={{ maxWidth: 380, margin: '0 auto', width: '100%' }} onKeyDown={handleKeyDown}>
          <div style={{
            fontFamily: NC_FONT_DISPLAY, fontWeight: 800,
            fontSize: 26, letterSpacing: -0.8, color: t.textHi, marginBottom: 8,
          }}>Sign in to continue</div>
          <div style={{ fontSize: 13, color: t.textMd, marginBottom: 28 }}>
            Enter your local NanoCrew Sync credentials.
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
          </div>

          {error && (
            <div style={{
              padding: '10px 12px', marginBottom: 16,
              background: `${t.danger}18`, border: `1px solid ${t.danger}50`,
              borderRadius: 3, fontSize: 12, color: t.danger,
              display: 'flex', gap: 8, alignItems: 'center',
            }}>
              <I.warn size={13} color={t.danger} style={{ flexShrink: 0 }} />
              {error}
            </div>
          )}

          <NCBtn
            theme={theme}
            primary
            style={{ width: '100%', justifyContent: 'center' }}
            onClick={handleSignIn}
            disabled={busy}
          >
            {busy ? 'Signing in…' : 'Sign in'}
          </NCBtn>
        </div>
      </div>
    </div>
  );
};
