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

interface AddDriveOneDriveScreenProps {
  theme: Theme;
  onBack: () => void;
  onCancel: () => void;
  onDone: () => void;
}

type DriveType = 'personal' | 'sharepoint';
type Step = 1 | 2 | 3;

const AZURE_CLIENT_ID = 'YOUR_AZURE_CLIENT_ID';

const Field: React.FC<{ label: string; children: React.ReactNode; theme: Theme; last?: boolean }> = ({
  label, children, theme, last,
}) => (
  <div style={{ marginBottom: last ? 0 : 14 }}>
    <NCLabel theme={theme}>{label}</NCLabel>
    {children}
  </div>
);

export const AddDriveOneDriveScreen: React.FC<AddDriveOneDriveScreenProps> = ({
  theme, onBack, onCancel, onDone,
}) => {
  const t = getTokens(theme);
  const { token } = useAuth();

  const [step, setStep] = React.useState<Step>(1);
  const [signingIn, setSigningIn] = React.useState(false);
  const [refreshToken, setRefreshToken] = React.useState('');
  const [driveType, setDriveType] = React.useState<DriveType>('personal');
  const [sharepointUrl, setSharepointUrl] = React.useState('');
  const [driveId, setDriveId] = React.useState('me/drive');
  const [name, setName] = React.useState('My OneDrive');
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
    setSigningIn(true);
    try {
      const rt = await invoke<string>('start_onedrive_auth', {
        token,
        clientId: AZURE_CLIENT_ID,
      });
      setRefreshToken(rt);
      setStep(2);
    } catch (e) {
      setError(String(e));
    } finally {
      setSigningIn(false);
    }
  };

  const handleDriveTypeNext = () => {
    setError(null);
    if (driveType === 'personal') {
      setDriveId('me/drive');
      setStep(3);
    } else {
      const url = sharepointUrl.trim();
      if (!url) {
        setError('Enter the SharePoint site URL.');
        return;
      }
      // Derive a drive_id placeholder from the URL; the actual resolution
      // happens server-side when the drive mounts.
      setDriveId(`sharepoint:${url}`);
      setStep(3);
    }
  };

  const handleAdd = async () => {
    setError(null);
    if (!name.trim()) { setError('Display name is required.'); return; }
    if (!letter) { setError('Select a drive letter.'); return; }

    setSaving(true);
    try {
      await invoke<number>('add_onedrive_drive', {
        token,
        clientId: AZURE_CLIENT_ID,
        refreshToken,
        driveId,
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

  const driveTypeBtnStyle = (active: boolean): React.CSSProperties => ({
    flex: 1,
    padding: '14px 16px',
    background: active ? `${t.lime}18` : t.surface1,
    border: `1px solid ${active ? t.lime : t.border}`,
    borderRadius: 4,
    cursor: 'pointer',
    textAlign: 'left',
  });

  return (
    <>
      <TopBar
        theme={theme}
        crumbs={['Drives', 'Add drive', 'OneDrive']}
        title={<>Connect <span style={{ color: t.lime }}>OneDrive</span></>}
        subtitle="Microsoft OneDrive via the Graph API. Credentials are stored in the Windows Credential Manager."
        actions={<NCBtn theme={theme} small ghost onClick={onCancel}>Cancel</NCBtn>}
      />
      <div style={{ flex: 1, overflow: 'auto', padding: 28 }}>
        <div style={{ maxWidth: 720, display: 'flex', flexDirection: 'column', gap: 24 }}>

          {/* ── Step indicator ──────────────────────────────────────────────── */}
          <div style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
            {([1, 2, 3] as Step[]).map((s, i) => (
              <React.Fragment key={s}>
                <div style={{
                  width: 28, height: 28, borderRadius: '50%',
                  background: step >= s ? t.lime : t.surface2,
                  color: step >= s ? '#0A0A0A' : t.textMd,
                  display: 'flex', alignItems: 'center', justifyContent: 'center',
                  fontSize: 12, fontWeight: 700, fontFamily: NC_FONT_MONO,
                  flexShrink: 0,
                }}>
                  {step > s ? <I.shield size={13} /> : s}
                </div>
                {i < 2 && (
                  <div style={{
                    flex: 1, height: 2,
                    background: step > s ? t.lime : t.surface2,
                    borderRadius: 1,
                  }} />
                )}
              </React.Fragment>
            ))}
          </div>

          {/* ── Step 1: sign in ─────────────────────────────────────────────── */}
          {step === 1 && (
            <NCCard theme={theme} pad={32} style={{ textAlign: 'center' }}>
              <div style={{
                width: 64, height: 64, borderRadius: 12,
                background: '#0078D4',
                display: 'flex', alignItems: 'center', justifyContent: 'center',
                margin: '0 auto 20px',
              }}>
                <svg width="36" height="36" viewBox="0 0 36 36" fill="none">
                  <rect x="2" y="2" width="15" height="15" rx="2" fill="white" opacity="0.9"/>
                  <rect x="19" y="2" width="15" height="15" rx="2" fill="white" opacity="0.7"/>
                  <rect x="2" y="19" width="15" height="15" rx="2" fill="white" opacity="0.7"/>
                  <rect x="19" y="19" width="15" height="15" rx="2" fill="white" opacity="0.5"/>
                </svg>
              </div>
              <div style={{
                fontSize: 18, fontWeight: 700, color: t.textHi,
                marginBottom: 8,
              }}>
                Sign in with Microsoft
              </div>
              <div style={{ fontSize: 13, color: t.textMd, marginBottom: 24, lineHeight: 1.6 }}>
                NanoCrew Sync will open your browser to sign in to your Microsoft account.
                Only Files.ReadWrite permission is requested.
              </div>
              <NCBtn
                theme={theme}
                primary
                icon={<I.arrow size={13} />}
                disabled={signingIn}
                onClick={handleSignIn}
              >
                {signingIn ? 'Waiting for sign-in…' : 'Sign in with Microsoft'}
              </NCBtn>
            </NCCard>
          )}

          {/* ── Step 2: drive type ──────────────────────────────────────────── */}
          {step === 2 && (
            <NCCard theme={theme} pad={24}>
              <NCEyebrow theme={theme} style={{ marginBottom: 16 }}>Drive type</NCEyebrow>
              <div style={{ display: 'flex', gap: 12, marginBottom: 20 }}>
                <button
                  style={driveTypeBtnStyle(driveType === 'personal')}
                  onClick={() => setDriveType('personal')}
                >
                  <div style={{ fontSize: 13, fontWeight: 600, color: t.textHi, marginBottom: 4 }}>
                    Personal OneDrive
                  </div>
                  <div style={{ fontSize: 11, color: t.textMd }}>
                    Your personal OneDrive storage
                  </div>
                </button>
                <button
                  style={driveTypeBtnStyle(driveType === 'sharepoint')}
                  onClick={() => setDriveType('sharepoint')}
                >
                  <div style={{ fontSize: 13, fontWeight: 600, color: t.textHi, marginBottom: 4 }}>
                    SharePoint document library
                  </div>
                  <div style={{ fontSize: 11, color: t.textMd }}>
                    OneDrive for Business or SharePoint site
                  </div>
                </button>
              </div>

              {driveType === 'sharepoint' && (
                <Field theme={theme} label="SharePoint site URL" last>
                  <NCInput
                    theme={theme} mono
                    value={sharepointUrl}
                    onChange={setSharepointUrl}
                    placeholder="https://contoso.sharepoint.com/sites/mysite"
                    prefix={<I.serverDb size={13} />}
                  />
                  <div style={{ marginTop: 6, fontSize: 11, color: t.textMd }}>
                    The root URL of your SharePoint site (not the Documents library URL).
                  </div>
                </Field>
              )}
            </NCCard>
          )}

          {/* ── Step 3: name + letter ────────────────────────────────────────── */}
          {step === 3 && (
            <NCCard theme={theme} pad={24}>
              <NCEyebrow theme={theme} style={{ marginBottom: 16 }}>Mount options</NCEyebrow>
              <Field theme={theme} label="Display name">
                <NCInput
                  theme={theme}
                  value={name}
                  onChange={setName}
                  placeholder="e.g. My OneDrive"
                />
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

          {/* ── Error banner ─────────────────────────────────────────────────── */}
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

          {/* ── Navigation row ───────────────────────────────────────────────── */}
          <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', gap: 12 }}>
            <NCBtn
              theme={theme}
              ghost
              iconLeft={<I.chevL size={14} />}
              onClick={step === 1 ? onBack : () => setStep((step - 1) as Step)}
            >
              Back
            </NCBtn>

            {step === 2 && (
              <NCBtn theme={theme} primary icon={<I.arrow size={13} />} onClick={handleDriveTypeNext}>
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
