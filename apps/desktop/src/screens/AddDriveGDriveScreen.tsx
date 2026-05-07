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

interface AddDriveGDriveScreenProps {
  theme: Theme;
  onBack: () => void;
  onCancel: () => void;
  onDone: () => void;
}

type Step = 1 | 2 | 3;

const Field: React.FC<{ label: string; children: React.ReactNode; theme: Theme; last?: boolean }> = ({
  label, children, theme, last,
}) => (
  <div style={{ marginBottom: last ? 0 : 14 }}>
    <NCLabel theme={theme}>{label}</NCLabel>
    {children}
  </div>
);

export const AddDriveGDriveScreen: React.FC<AddDriveGDriveScreenProps> = ({
  theme, onBack, onCancel, onDone,
}) => {
  const t = getTokens(theme);
  const { token } = useAuth();

  const [step, setStep] = React.useState<Step>(1);
  const [signing, setSigning] = React.useState(false);
  const [refreshToken, setRefreshToken] = React.useState('');
  const [rootMode, setRootMode] = React.useState<'root' | 'folder'>('root');
  const [folderId, setFolderId] = React.useState('');
  const [name, setName] = React.useState('My Google Drive');
  const [letter, setLetter] = React.useState('');
  const [availableLetters, setAvailableLetters] = React.useState<string[]>([]);
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

  const handleSignIn = async () => {
    setError(null);
    setSigning(true);
    try {
      const result = await invoke<{ access_token: string; refresh_token: string; expires_in: number }>(
        'start_gdrive_auth',
      );
      setRefreshToken(result.refresh_token);
      setStep(2);
    } catch (e) {
      setError(String(e));
    } finally {
      setSigning(false);
    }
  };

  const handleAdd = async () => {
    setError(null);
    if (!name.trim()) { setError('Display name is required.'); return; }
    if (!letter) { setError('Select a drive letter.'); return; }
    if (rootMode === 'folder' && !folderId.trim()) {
      setError('Enter a folder ID or switch to My Drive root.');
      return;
    }

    setSaving(true);
    try {
      await invoke('add_gdrive_drive', {
        token,
        refreshToken,
        rootFolderId: rootMode === 'root' ? 'root' : folderId.trim(),
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

  const stepLabel = step === 1 ? 'Sign in' : step === 2 ? 'Root folder' : 'Name & mount';

  return (
    <>
      <TopBar
        theme={theme}
        crumbs={['Drives', 'Add drive', 'Google Drive', stepLabel]}
        title={<>Connect <span style={{ color: t.lime }}>Google Drive</span></>}
        subtitle="Sign in with Google to mount your Drive as a Windows drive letter."
        actions={<NCBtn theme={theme} small ghost onClick={onCancel}>Cancel</NCBtn>}
      />
      <div style={{ flex: 1, overflow: 'auto', padding: 28 }}>
        <div style={{ maxWidth: 600, display: 'flex', flexDirection: 'column', gap: 24 }}>

          {/* ── Step indicator ──────────────────────────────────────────────── */}
          <div style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
            {([1, 2, 3] as Step[]).map(s => (
              <React.Fragment key={s}>
                <div style={{
                  width: 28, height: 28, borderRadius: '50%',
                  background: s === step ? t.lime : s < step ? `${t.lime}50` : t.surface1,
                  border: `1px solid ${s <= step ? t.lime : t.border}`,
                  display: 'flex', alignItems: 'center', justifyContent: 'center',
                  fontSize: 12, fontFamily: NC_FONT_MONO, fontWeight: 700,
                  color: s === step ? '#0A0A0A' : s < step ? t.lime : t.textMd,
                }}>
                  {s < step ? <I.shield size={11} color={t.lime} /> : s}
                </div>
                {s < 3 && (
                  <div style={{
                    flex: 1, height: 1,
                    background: s < step ? t.lime : t.border,
                  }} />
                )}
              </React.Fragment>
            ))}
          </div>

          {/* ── Step 1: Sign in ─────────────────────────────────────────────── */}
          {step === 1 && (
            <NCCard theme={theme} pad={32} style={{ textAlign: 'center' }}>
              <div style={{
                width: 64, height: 64, borderRadius: '50%',
                background: t.surface2,
                display: 'flex', alignItems: 'center', justifyContent: 'center',
                margin: '0 auto 20px',
              }}>
                <I.cloud size={28} color={t.lime} />
              </div>
              <div style={{
                fontSize: 18, fontWeight: 700, color: t.textHi,
                marginBottom: 8,
              }}>
                Sign in with Google
              </div>
              <div style={{
                fontSize: 13, color: t.textMd, lineHeight: 1.6,
                marginBottom: 28, maxWidth: 380, margin: '0 auto 28px',
              }}>
                NanoCrew Sync will open your browser to complete Google authorization.
                Your credentials are stored locally and never sent to NanoCrew servers.
              </div>
              <NCBtn
                theme={theme}
                primary
                disabled={signing}
                onClick={handleSignIn}
              >
                {signing ? 'Waiting for browser…' : 'Sign in with Google'}
              </NCBtn>
            </NCCard>
          )}

          {/* ── Step 2: Root folder ──────────────────────────────────────────── */}
          {step === 2 && (
            <NCCard theme={theme} pad={24}>
              <NCEyebrow theme={theme} style={{ marginBottom: 16 }}>Root folder</NCEyebrow>
              <div style={{ display: 'flex', flexDirection: 'column', gap: 10, marginBottom: 16 }}>
                {(['root', 'folder'] as const).map(mode => (
                  <label
                    key={mode}
                    style={{
                      display: 'flex', alignItems: 'flex-start', gap: 12,
                      padding: '12px 14px',
                      background: rootMode === mode ? `${t.lime}10` : t.surface1,
                      border: `1px solid ${rootMode === mode ? t.lime : t.border}`,
                      borderRadius: 4, cursor: 'pointer',
                    }}
                    onClick={() => setRootMode(mode)}
                  >
                    <div style={{
                      marginTop: 2, width: 16, height: 16, borderRadius: '50%',
                      border: `2px solid ${rootMode === mode ? t.lime : t.border}`,
                      background: rootMode === mode ? t.lime : 'transparent',
                      flexShrink: 0,
                    }} />
                    <div>
                      <div style={{ fontSize: 13, fontWeight: 500, color: t.textHi }}>
                        {mode === 'root' ? 'My Drive (root)' : 'Specific folder'}
                      </div>
                      <div style={{ fontSize: 11, color: t.textMd, lineHeight: 1.5 }}>
                        {mode === 'root'
                          ? 'Mount the entire My Drive as the drive root.'
                          : 'Mount only a specific folder (enter its folder ID below).'}
                      </div>
                    </div>
                  </label>
                ))}
              </div>

              {rootMode === 'folder' && (
                <Field theme={theme} label="Folder ID" last>
                  <NCInput
                    theme={theme}
                    mono
                    value={folderId}
                    onChange={setFolderId}
                    placeholder="1BxiMVs0XRA5nFMdKvBdBZjgmUUqptlbs74OgVE2upms"
                    prefix={<I.folder size={13} />}
                  />
                  <div style={{ marginTop: 6, fontSize: 11, color: t.textMd }}>
                    Find the ID in the folder URL: drive.google.com/drive/folders/<strong>{'<ID>'}</strong>
                  </div>
                </Field>
              )}
            </NCCard>
          )}

          {/* ── Step 3: Name & mount options ─────────────────────────────────── */}
          {step === 3 && (
            <NCCard theme={theme} pad={24}>
              <NCEyebrow theme={theme} style={{ marginBottom: 16 }}>Drive details</NCEyebrow>
              <Field theme={theme} label="Display name">
                <NCInput theme={theme} value={name} onChange={setName} placeholder="e.g. My Google Drive" />
              </Field>
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

          {/* ── Navigation ───────────────────────────────────────────────────── */}
          <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
            <NCBtn
              theme={theme}
              ghost
              iconLeft={<I.chevL size={14} />}
              onClick={step === 1 ? onBack : () => setStep((step - 1) as Step)}
            >
              Back
            </NCBtn>
            {step === 1 && null}
            {step === 2 && (
              <NCBtn
                theme={theme}
                primary
                icon={<I.arrow size={13} />}
                onClick={() => setStep(3)}
              >
                Next
              </NCBtn>
            )}
            {step === 3 && (
              <NCBtn
                theme={theme}
                primary
                icon={<I.arrow size={13} />}
                disabled={saving}
                onClick={handleAdd}
              >
                {saving ? 'Adding…' : 'Add drive'}
              </NCBtn>
            )}
          </div>

        </div>
      </div>
    </>
  );
};
