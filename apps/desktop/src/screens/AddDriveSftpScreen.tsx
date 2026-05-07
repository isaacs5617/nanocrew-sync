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

interface AddDriveSftpScreenProps {
  theme: Theme;
  onBack: () => void;
  onCancel: () => void;
  onDone: () => void;
}

type AuthMethod = 'password' | 'private_key';

const Field: React.FC<{ label: string; children: React.ReactNode; theme: Theme; last?: boolean }> = ({
  label, children, theme, last,
}) => (
  <div style={{ marginBottom: last ? 0 : 14 }}>
    <NCLabel theme={theme}>{label}</NCLabel>
    {children}
  </div>
);

export const AddDriveSftpScreen: React.FC<AddDriveSftpScreenProps> = ({
  theme, onBack, onCancel, onDone,
}) => {
  const t = getTokens(theme);
  const { token } = useAuth();

  const [name, setName] = React.useState('');
  const [host, setHost] = React.useState('');
  const [port, setPort] = React.useState('22');
  const [username, setUsername] = React.useState('');
  const [authMethod, setAuthMethod] = React.useState<AuthMethod>('password');
  const [password, setPassword] = React.useState('');
  const [showPassword, setShowPassword] = React.useState(false);
  const [keyPem, setKeyPem] = React.useState('');
  const [passphrase, setPassphrase] = React.useState('');
  const [showPassphrase, setShowPassphrase] = React.useState(false);
  const [rootPath, setRootPath] = React.useState('/');
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
    port: parseInt(port, 10) || 22,
    username: username.trim(),
    auth: authMethod === 'password'
      ? { type: 'password', password }
      : { type: 'private_key', key_pem: keyPem, passphrase: passphrase || null },
    root_path: rootPath.trim() || '/',
    known_host_fingerprint: null,
  });

  const handleTest = async () => {
    setError(null);
    setTestOk(null);
    if (!host.trim()) { setError('Host is required.'); return; }
    if (!username.trim()) { setError('Username is required.'); return; }
    if (authMethod === 'password' && !password) { setError('Password is required.'); return; }
    if (authMethod === 'private_key' && !keyPem.trim()) { setError('Private key is required.'); return; }

    setTesting(true);
    try {
      await invoke('test_sftp_connection', { config: buildConfig() });
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
    if (authMethod === 'password' && !password) { setError('Password is required.'); return; }
    if (authMethod === 'private_key' && !keyPem.trim()) { setError('Private key is required.'); return; }
    if (!letter) { setError('Select a drive letter.'); return; }

    setSaving(true);
    try {
      await invoke('add_sftp_drive', {
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
        crumbs={['Drives', 'Add drive', 'SFTP']}
        title={<>Connect <span style={{ color: t.lime }}>SFTP</span></>}
        subtitle="SSH File Transfer Protocol. Credentials are stored in the Windows Credential Manager."
        actions={<NCBtn theme={theme} small ghost onClick={onCancel}>Cancel</NCBtn>}
      />
      <div style={{ flex: 1, overflow: 'auto', padding: 28 }}>
        <div style={{ maxWidth: 720, display: 'flex', flexDirection: 'column', gap: 24 }}>

          <NCCard theme={theme} pad={24}>
            <NCEyebrow theme={theme} style={{ marginBottom: 16 }}>Connection</NCEyebrow>
            <Field theme={theme} label="Display name">
              <NCInput theme={theme} value={name} onChange={setName} placeholder="e.g. My SFTP Server" />
            </Field>
            <div style={{ display: 'grid', gridTemplateColumns: '1fr auto', gap: 8 }}>
              <Field theme={theme} label="Host">
                <NCInput theme={theme} mono value={host} onChange={setHost} placeholder="sftp.example.com" prefix={<I.serverDb size={13} />} />
              </Field>
              <Field theme={theme} label="Port">
                <NCInput theme={theme} mono value={port} onChange={setPort} placeholder="22" style={{ width: 80 }} />
              </Field>
            </div>
            <Field theme={theme} label="Username">
              <NCInput theme={theme} mono value={username} onChange={setUsername} placeholder="user" prefix={<I.lock size={13} />} />
            </Field>
            <Field theme={theme} label="Root path" last>
              <NCInput
                theme={theme} mono
                value={rootPath}
                onChange={setRootPath}
                placeholder="/home/user/files"
                prefix={<I.folder size={13} />}
              />
              <div style={{ marginTop: 6, fontSize: 11, color: t.textMd }}>
                Remote path to use as the drive root. Use <code style={{ fontFamily: NC_FONT_MONO }}>/</code> for the server root.
              </div>
            </Field>
          </NCCard>

          <NCCard theme={theme} pad={24}>
            <NCEyebrow theme={theme} style={{ marginBottom: 16 }}>Authentication</NCEyebrow>
            <div style={{ display: 'flex', gap: 8, marginBottom: 16 }}>
              {(['password', 'private_key'] as AuthMethod[]).map(m => (
                <NCBtn
                  key={m}
                  theme={theme}
                  small
                  ghost={authMethod !== m}
                  onClick={() => setAuthMethod(m)}
                >
                  {m === 'password' ? 'Password' : 'Private Key'}
                </NCBtn>
              ))}
            </div>

            {authMethod === 'password' ? (
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
            ) : (
              <>
                <Field theme={theme} label="Private key (PEM)">
                  <textarea
                    value={keyPem}
                    onChange={e => setKeyPem(e.target.value)}
                    placeholder="-----BEGIN OPENSSH PRIVATE KEY-----&#10;...&#10;-----END OPENSSH PRIVATE KEY-----"
                    style={{
                      width: '100%', boxSizing: 'border-box',
                      minHeight: 120, resize: 'vertical',
                      background: t.surface1, border: `1px solid ${t.border}`,
                      borderRadius: 3, padding: '10px 12px',
                      fontFamily: NC_FONT_MONO, fontSize: 11,
                      color: t.textHi, outline: 'none',
                    }}
                  />
                </Field>
                <Field theme={theme} label="Passphrase (optional)" last>
                  <NCInput
                    theme={theme} mono
                    type={showPassphrase ? 'text' : 'password'}
                    value={passphrase}
                    onChange={setPassphrase}
                    placeholder="Leave blank if key has no passphrase"
                    prefix={<I.lock size={13} />}
                    suffix={
                      <span style={{ cursor: 'pointer' }} onClick={() => setShowPassphrase(v => !v)}>
                        {showPassphrase ? <I.eyeOff size={14} /> : <I.eye size={14} />}
                      </span>
                    }
                  />
                </Field>
              </>
            )}
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
