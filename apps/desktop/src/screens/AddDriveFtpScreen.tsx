import React from 'react';
import { invoke } from '@tauri-apps/api/core';
import {
  getTokens, NC_FONT_MONO,
  NCCard, NCEyebrow, NCLabel, NCBtn, NCInput, NCToggle,
  TopBar,
  type Theme,
} from '@nanocrew/ui';
import { I } from '@nanocrew/ui';
import { useAuth } from '../context/auth.js';

interface AddDriveFtpScreenProps {
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

export const AddDriveFtpScreen: React.FC<AddDriveFtpScreenProps> = ({
  theme, onBack, onCancel, onDone,
}) => {
  const t = getTokens(theme);
  const { token } = useAuth();

  const [name, setName] = React.useState('');
  const [host, setHost] = React.useState('');
  const [port, setPort] = React.useState('21');
  const [username, setUsername] = React.useState('');
  const [password, setPassword] = React.useState('');
  const [showPassword, setShowPassword] = React.useState(false);
  const [rootPath, setRootPath] = React.useState('/');
  const [useTls, setUseTls] = React.useState(false);
  const [letter, setLetter] = React.useState('');
  const [availableLetters, setAvailableLetters] = React.useState<string[]>([]);
  const [testing, setTesting] = React.useState(false);
  const [testOk, setTestOk] = React.useState<boolean | null>(null);
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

  const buildConfig = () => ({
    host: host.trim(),
    port: parseInt(port, 10) || 21,
    username: username.trim(),
    password,
    root_path: rootPath.trim() || '/',
    use_tls: useTls,
  });

  const handleTest = async () => {
    setError(null);
    setTestOk(null);
    if (!host.trim()) { setError('Host is required.'); return; }
    if (!username.trim()) { setError('Username is required.'); return; }

    setTesting(true);
    try {
      await invoke('test_ftp_connection', { config: buildConfig() });
      setTestOk(true);
    } catch (e) {
      setTestOk(false);
      setError(String(e));
    } finally {
      setTesting(false);
    }
  };

  const handleAdd = async () => {
    setError(null);
    if (!name.trim()) { setError('Display name is required.'); return; }
    if (!host.trim()) { setError('Host is required.'); return; }
    if (!username.trim()) { setError('Username is required.'); return; }
    if (!letter) { setError('Select a drive letter.'); return; }

    setSaving(true);
    try {
      await invoke('add_ftp_drive', {
        token,
        config: buildConfig(),
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
        crumbs={['Drives', 'Add drive', 'FTP']}
        title={<>Connect <span style={{ color: t.lime }}>FTP / FTPS</span></>}
        subtitle="File Transfer Protocol. Use FTPS (FTP over TLS) for encrypted transfers."
        actions={<NCBtn theme={theme} small ghost onClick={onCancel}>Cancel</NCBtn>}
      />
      <div style={{ flex: 1, overflow: 'auto', padding: 28 }}>
        <div style={{ maxWidth: 720, display: 'flex', flexDirection: 'column', gap: 24 }}>

          <NCCard theme={theme} pad={24}>
            <NCEyebrow theme={theme} style={{ marginBottom: 16 }}>Connection</NCEyebrow>
            <Field theme={theme} label="Display name">
              <NCInput theme={theme} value={name} onChange={setName} placeholder="e.g. My FTP Server" />
            </Field>
            <div style={{ display: 'grid', gridTemplateColumns: '1fr auto', gap: 8 }}>
              <Field theme={theme} label="Host">
                <NCInput theme={theme} mono value={host} onChange={setHost} placeholder="ftp.example.com" prefix={<I.serverDb size={13} />} />
              </Field>
              <Field theme={theme} label="Port">
                <NCInput theme={theme} mono value={port} onChange={setPort} placeholder="21" style={{ width: 80 }} />
              </Field>
            </div>
            <Field theme={theme} label="Root path" last>
              <NCInput
                theme={theme} mono
                value={rootPath}
                onChange={setRootPath}
                placeholder="/public_html"
                prefix={<I.folder size={13} />}
              />
              <div style={{ marginTop: 6, fontSize: 11, color: t.textMd }}>
                Remote path to use as the drive root. Use <code style={{ fontFamily: NC_FONT_MONO }}>/</code> for the server root.
              </div>
            </Field>
          </NCCard>

          <NCCard theme={theme} pad={24}>
            <NCEyebrow theme={theme} style={{ marginBottom: 16 }}>Credentials</NCEyebrow>
            <Field theme={theme} label="Username">
              <NCInput theme={theme} mono value={username} onChange={setUsername} placeholder="user" prefix={<I.lock size={13} />} />
            </Field>
            <Field theme={theme} label="Password" last>
              <NCInput
                theme={theme} mono
                type={showPassword ? 'text' : 'password'}
                value={password}
                onChange={setPassword}
                placeholder="········"
                prefix={<I.lock size={13} />}
                suffix={
                  <span style={{ cursor: 'pointer' }} onClick={() => setShowPassword(v => !v)}>
                    {showPassword ? <I.eyeOff size={14} /> : <I.eye size={14} />}
                  </span>
                }
              />
            </Field>
          </NCCard>

          <NCCard theme={theme} pad={24}>
            <NCEyebrow theme={theme} style={{ marginBottom: 16 }}>Options</NCEyebrow>
            <div style={{ display: 'flex', alignItems: 'center', gap: 14, marginBottom: 16 }}>
              <div style={{ flex: 1 }}>
                <div style={{ fontSize: 13, color: t.textHi, fontWeight: 500 }}>Use FTPS (FTP over TLS)</div>
                <div style={{ fontSize: 11, color: t.textMd, marginTop: 2 }}>Encrypts the connection. Requires the server to support TLS.</div>
              </div>
              <NCToggle on={useTls} onChange={val => { setUseTls(val); setPort(val ? '990' : '21'); }} theme={theme} />
            </div>
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

          {testOk === true && (
            <div style={{
              padding: '10px 14px',
              background: `${t.lime}18`, border: `1px solid ${t.lime}50`,
              borderRadius: 3, fontSize: 12, color: t.lime,
              display: 'flex', gap: 8, alignItems: 'center',
            }}>
              <I.shield size={13} color={t.lime} style={{ flexShrink: 0 }} />
              Connection successful.
            </div>
          )}

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
            <div style={{ display: 'flex', gap: 8 }}>
              <NCBtn theme={theme} disabled={testing} onClick={handleTest}>
                {testing ? 'Testing…' : testOk === true ? 'Test passed' : 'Test connection'}
              </NCBtn>
              <NCBtn theme={theme} primary icon={<I.arrow size={13} />} disabled={saving} onClick={handleAdd}>
                {saving ? 'Adding…' : 'Add drive'}
              </NCBtn>
            </div>
          </div>

        </div>
      </div>
    </>
  );
};
