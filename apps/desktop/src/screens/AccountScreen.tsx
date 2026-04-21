import React from 'react';
import { invoke } from '@tauri-apps/api/core';
import {
  getTokens, NC_FONT_DISPLAY, NC_FONT_MONO,
  NCCard, NCEyebrow, NCBtn, NCBadge, NCInput,
  TopBar,
  type Theme,
} from '@nanocrew/ui';
import { I } from '@nanocrew/ui';
import { useAuth } from '../context/auth.js';

interface AccountScreenProps {
  theme: Theme;
  onSignOut: () => void;
}

interface AccountInfo {
  id: number;
  username: string;
  created_at: number;
}

export const AccountScreen: React.FC<AccountScreenProps> = ({ theme, onSignOut }) => {
  const t = getTokens(theme);
  const { token, lock } = useAuth();
  const [account, setAccount] = React.useState<AccountInfo | null>(null);
  const [changingPw, setChangingPw] = React.useState(false);
  const [currentPw, setCurrentPw] = React.useState('');
  const [newPw, setNewPw] = React.useState('');
  const [confirmPw, setConfirmPw] = React.useState('');
  const [pwError, setPwError] = React.useState<string | null>(null);
  const [pwSuccess, setPwSuccess] = React.useState(false);
  const [busy, setBusy] = React.useState(false);

  React.useEffect(() => {
    invoke<AccountInfo>('get_account', { token })
      .then(setAccount)
      .catch(() => {});
  }, [token]);

  const initial = account ? account.username[0]?.toUpperCase() ?? '?' : '?';
  const memberYear = account ? new Date(account.created_at * 1000).getFullYear() : null;

  const handleChangePassword = async () => {
    setPwError(null);
    setPwSuccess(false);
    if (!currentPw) { setPwError('Current password is required.'); return; }
    if (newPw.length < 8) { setPwError('New password must be at least 8 characters.'); return; }
    if (newPw !== confirmPw) { setPwError('Passwords do not match.'); return; }
    setBusy(true);
    try {
      await invoke('change_password', { token, currentPassword: currentPw, newPassword: newPw });
      setPwSuccess(true);
      setChangingPw(false);
      setCurrentPw('');
      setNewPw('');
      setConfirmPw('');
    } catch (e) {
      setPwError(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <>
      <TopBar
        theme={theme}
        crumbs={['Account']}
        title={<>Your <span style={{ color: t.lime }}>account</span></>}
        subtitle="NanoCrew Sync is currently free for everyone. Bring your own storage — we never touch your data."
        actions={
          <div style={{ display: 'flex', gap: 8 }}>
            <NCBtn theme={theme} small ghost iconLeft={<I.lock size={13} />} onClick={lock}>
              Lock
            </NCBtn>
            <NCBtn theme={theme} small ghost iconLeft={<I.x size={13} />} onClick={onSignOut}>
              Sign out
            </NCBtn>
          </div>
        }
      />
      <div style={{ flex: 1, overflow: 'auto', padding: 28 }}>
        <div style={{ maxWidth: 720, display: 'flex', flexDirection: 'column', gap: 20 }}>

          <NCCard theme={theme} pad={24} style={{ display: 'flex', gap: 20, alignItems: 'center' }}>
            <div style={{
              width: 72, height: 72, borderRadius: 4,
              background: t.lime, color: '#0A0A0A',
              display: 'flex', alignItems: 'center', justifyContent: 'center',
              fontFamily: NC_FONT_DISPLAY, fontWeight: 800, fontSize: 32,
            }}>{initial}</div>
            <div style={{ flex: 1 }}>
              <div style={{
                fontFamily: NC_FONT_DISPLAY, fontWeight: 800,
                fontSize: 22, color: t.textHi, letterSpacing: -0.5, marginBottom: 4,
              }}>{account?.username ?? '…'}</div>
              <div style={{ fontSize: 13, color: t.textMd, marginBottom: 8 }}>Local account · this device only</div>
              <div style={{ display: 'flex', gap: 6 }}>
                <NCBadge theme={theme} color="lime">FREE · UNLIMITED</NCBadge>
                <NCBadge theme={theme} color="muted">EARLY ACCESS</NCBadge>
                {memberYear && <NCBadge theme={theme} color="muted">MEMBER SINCE {memberYear}</NCBadge>}
              </div>
            </div>
          </NCCard>

          <NCCard theme={theme} pad={20} style={{ border: `1px solid ${t.lime}`, background: t.limeSoft }}>
            <div style={{ display: 'flex', gap: 14, alignItems: 'flex-start' }}>
              <I.shield size={18} color={t.lime} style={{ marginTop: 2 }} />
              <div style={{ flex: 1 }}>
                <div style={{ fontFamily: NC_FONT_DISPLAY, fontWeight: 800, fontSize: 16, color: t.textHi, marginBottom: 6 }}>
                  Free during early access
                </div>
                <div style={{ fontSize: 13, color: t.textMd, lineHeight: 1.55, marginBottom: 10 }}>
                  NanoCrew Sync has no subscription and no usage limits while we're in beta. You pay your own
                  storage provider directly — we never sit on the data path.
                </div>
                <div style={{ fontFamily: NC_FONT_MONO, fontSize: 10, color: t.textMd, letterSpacing: 1.5 }}>
                  NO CARD ON FILE · NO TRIAL EXPIRY · FEEDBACK → feedback@nanocrew.dev
                </div>
              </div>
            </div>
          </NCCard>

          <NCCard theme={theme} pad={20}>
            <div style={{ display: 'flex', alignItems: 'center', marginBottom: 14 }}>
              <NCEyebrow theme={theme}>Security</NCEyebrow>
            </div>

            {/* Password row */}
            <div style={{ padding: '12px 0', borderBottom: `1px solid ${t.border}` }}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 14, marginBottom: changingPw ? 14 : 0 }}>
                <div style={{
                  width: 32, height: 32, borderRadius: 3, background: t.surface2,
                  border: `1px solid ${t.border}`,
                  display: 'flex', alignItems: 'center', justifyContent: 'center',
                }}>
                  <I.lock size={14} color={t.textMd} />
                </div>
                <div style={{ flex: 1 }}>
                  <div style={{ fontSize: 13, fontWeight: 500, color: t.textHi }}>Password</div>
                  <div style={{ fontSize: 11, color: t.textMd, fontFamily: NC_FONT_MONO, letterSpacing: 0.5 }}>
                    Hashed with Argon2id · stored locally
                  </div>
                </div>
                <NCBtn theme={theme} small ghost onClick={() => {
                  setChangingPw(v => !v);
                  setPwError(null); setPwSuccess(false);
                  setCurrentPw(''); setNewPw(''); setConfirmPw('');
                }}>
                  {changingPw ? 'Cancel' : 'Change password'}
                </NCBtn>
              </div>

              {changingPw && (
                <div style={{ display: 'flex', flexDirection: 'column', gap: 10, paddingLeft: 46 }}>
                  <NCInput theme={theme} type="password" value={currentPw} onChange={setCurrentPw} placeholder="Current password" />
                  <NCInput theme={theme} type="password" value={newPw} onChange={setNewPw} placeholder="New password (min 8 chars)" />
                  <NCInput theme={theme} type="password" value={confirmPw} onChange={setConfirmPw} placeholder="Confirm new password" />
                  {pwError && (
                    <div style={{ fontSize: 12, color: t.danger, display: 'flex', gap: 6, alignItems: 'center' }}>
                      <I.warn size={12} color={t.danger} /> {pwError}
                    </div>
                  )}
                  <NCBtn theme={theme} small primary onClick={handleChangePassword} disabled={busy}>
                    {busy ? 'Saving…' : 'Save new password'}
                  </NCBtn>
                </div>
              )}
              {pwSuccess && !changingPw && (
                <div style={{ fontSize: 12, color: t.lime, paddingLeft: 46, paddingTop: 8 }}>
                  Password changed successfully.
                </div>
              )}
            </div>

            {/* Credential storage row */}
            <div style={{ display: 'flex', alignItems: 'center', gap: 14, padding: '12px 0' }}>
              <div style={{
                width: 32, height: 32, borderRadius: 3, background: t.surface2,
                border: `1px solid ${t.border}`,
                display: 'flex', alignItems: 'center', justifyContent: 'center',
              }}>
                <I.shield size={14} color={t.textMd} />
              </div>
              <div style={{ flex: 1 }}>
                <div style={{ fontSize: 13, fontWeight: 500, color: t.textHi }}>S3 credentials</div>
                <div style={{ fontSize: 11, color: t.textMd, fontFamily: NC_FONT_MONO, letterSpacing: 0.5 }}>
                  Windows Credential Manager · never in plaintext
                </div>
              </div>
            </div>
          </NCCard>

        </div>
      </div>
    </>
  );
};
