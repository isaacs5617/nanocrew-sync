import React from 'react';
import { invoke } from '@tauri-apps/api/core';
import {
  getTokens, NC_FONT_MONO,
  NCCard, NCEyebrow, NCLabel, NCBtn, NCInput,
  TopBar,
  type Theme,
} from '@nanocrew/ui';
import { I } from '@nanocrew/ui';
import { useAuth } from '../context/auth.js';

interface AddDriveDropboxScreenProps {
  theme: Theme;
  onBack: () => void;
  onCancel: () => void;
  onDone: () => void;
}

const Field: React.FC<{ label: string; children: React.ReactNode; theme: Theme; last?: boolean }> = ({
  label, children, theme, last,
}) => (
  <div style={{ marginBottom: last ? 0 : 14 }}>
    <NCLabel theme={theme}>{label}</NCLabel>
    {children}
  </div>
);

export const AddDriveDropboxScreen: React.FC<AddDriveDropboxScreenProps> = ({
  theme, onBack, onCancel, onDone,
}) => {
  const t = getTokens(theme);
  const { token } = useAuth();

  const [name, setName] = React.useState('');
  const [rootPath, setRootPath] = React.useState('');
  const [letter, setLetter] = React.useState('');
  const [availableLetters, setAvailableLetters] = React.useState<string[]>([]);
  const [connecting, setConnecting] = React.useState(false);
  const [connected, setConnected] = React.useState(false);
  const [accessToken, setAccessToken] = React.useState('');
  const [refreshToken, setRefreshToken] = React.useState('');
  const [saving, setSaving] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);

  React.useEffect(() => {
    invoke<string[]>('get_available_letters', { token })
      .then(letters => {
        setAvailableLetters(letters);
        if (letters.length > 0) setLetter(letters[0]!);
      })
      .catch(() => {});
  }, [token]);

  const handleConnect = async () => {
    setError(null);
    setConnecting(true);
    try {
      const authUrl = await invoke<string>('start_dropbox_auth', { token });
      // Open the authorization URL in the system browser
      await invoke('open_path', { token, path: authUrl });
      // Prompt user to paste the code from the callback
      const code = window.prompt(
        'A browser window has opened. After authorizing, paste the authorization code here:'
      );
      if (!code) {
        setConnecting(false);
        return;
      }
      // Exchange the code via a direct fetch (the PKCE flow completes in the browser)
      // For now we accept the tokens entered manually — a future version will use
      // the loopback listener. Store the code as the refresh token placeholder so
      // the backend can complete the exchange on first mount.
      setAccessToken(code.trim());
      setRefreshToken(code.trim());
      setConnected(true);
    } catch (e) {
      setError(String(e));
    } finally {
      setConnecting(false);
    }
  };

  const handleAdd = async () => {
    setError(null);
    if (!name.trim()) { setError('Display name is required.'); return; }
    if (!connected || !refreshToken) { setError('Connect your Dropbox account first.'); return; }
    if (!letter) { setError('Select a drive letter.'); return; }

    setSaving(true);
    try {
      await invoke('add_dropbox_drive', {
        token,
        accessToken,
        refreshToken,
        rootPath: rootPath.trim(),
        driveLetter: letter,
        name: name.trim(),
      });
      onDone();
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
    }
  };

  const selectStyle: React.CSSProperties = {
    width: '100%',
    background: t.surface1,
    border: `1px solid ${t.border}`,
    borderRadius: 3,
    padding: '10px 12px',
    fontFamily: NC_FONT_MONO,
    fontSize: 12,
    color: t.textHi,
    outline: 'none',
    cursor: 'pointer',
    appearance: 'none',
    WebkitAppearance: 'none',
  };

  return (
    <>
      <TopBar
        theme={theme}
        crumbs={['Drives', 'Add drive', 'Dropbox']}
        title={<>Connect <span style={{ color: t.lime }}>Dropbox</span></>}
        subtitle="Mount your Dropbox as a Windows drive letter via OAuth2. Your token is stored in the Windows Credential Manager."
        actions={<NCBtn theme={theme} small ghost onClick={onCancel}>Cancel</NCBtn>}
      />
      <div style={{ flex: 1, overflow: 'auto', padding: 28 }}>
        <div style={{ maxWidth: 720, display: 'flex', flexDirection: 'column', gap: 24 }}>

          <NCCard theme={theme} pad={24}>
            <NCEyebrow theme={theme} style={{ marginBottom: 16 }}>Account</NCEyebrow>
            <Field theme={theme} label="Display name">
              <NCInput
                theme={theme}
                value={name}
                onChange={setName}
                placeholder="e.g. My Dropbox"
              />
            </Field>
            <div style={{ marginBottom: 14 }}>
              <NCLabel theme={theme}>Dropbox account</NCLabel>
              {connected ? (
                <div style={{
                  padding: '10px 14px',
                  background: `${t.lime}18`, border: `1px solid ${t.lime}50`,
                  borderRadius: 3, fontSize: 12, color: t.lime,
                  display: 'flex', gap: 8, alignItems: 'center',
                }}>
                  <I.shield size={13} color={t.lime} style={{ flexShrink: 0 }} />
                  Dropbox account connected.
                </div>
              ) : (
                <NCBtn
                  theme={theme}
                  disabled={connecting}
                  onClick={handleConnect}
                >
                  {connecting ? 'Opening browser…' : 'Connect Dropbox account'}
                </NCBtn>
              )}
            </div>
            <Field theme={theme} label="Root folder (optional)" last>
              <NCInput
                theme={theme}
                mono
                value={rootPath}
                onChange={setRootPath}
                placeholder="/Work files"
                prefix={<I.folder size={13} />}
              />
              <div style={{ marginTop: 6, fontSize: 11, color: t.textMd }}>
                Subfolder inside your Dropbox to use as the drive root. Leave blank for the Dropbox root.
              </div>
            </Field>
          </NCCard>

          <NCCard theme={theme} pad={24}>
            <NCEyebrow theme={theme} style={{ marginBottom: 16 }}>Mount options</NCEyebrow>
            <Field theme={theme} label="Drive letter" last>
              <div style={{ position: 'relative', maxWidth: 120 }}>
                <div style={{ position: 'absolute', right: 12, top: '50%', transform: 'translateY(-50%)', pointerEvents: 'none' }}>
                  <I.chevD size={13} color={t.textMd} />
                </div>
                <select
                  value={letter}
                  onChange={e => setLetter(e.target.value)}
                  style={{ ...selectStyle, fontSize: 16, fontWeight: 500, color: t.lime }}
                >
                  {availableLetters.length === 0
                    ? <option value="">No letters available</option>
                    : availableLetters.map(l => <option key={l} value={l}>{l}</option>)
                  }
                </select>
              </div>
            </Field>
          </NCCard>

          {error && (
            <div style={{
              padding: '10px 14px',
              background: `${t.danger}18`, border: `1px solid ${t.danger}50`,
              borderRadius: 3, fontSize: 12, color: t.danger,
              display: 'flex', gap: 8, alignItems: 'center',
            }}>
              <I.warn size={13} color={t.danger} style={{ flexShrink: 0 }} />
              {error}
            </div>
          )}

          <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', gap: 12 }}>
            <NCBtn theme={theme} ghost iconLeft={<I.chevL size={14} />} onClick={onBack}>Back</NCBtn>
            <NCBtn theme={theme} primary icon={<I.arrow size={13} />} disabled={saving || !connected} onClick={handleAdd}>
              {saving ? 'Adding…' : 'Add drive'}
            </NCBtn>
          </div>

        </div>
      </div>
    </>
  );
};
