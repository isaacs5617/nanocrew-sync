import React from 'react';
import { useTranslation } from 'react-i18next';
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
  const tok = getTokens(theme);
  const { t } = useTranslation();
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
    if (!currentPw) { setPwError(t('account.password.errors.currentRequired')); return; }
    if (newPw.length < 8) { setPwError(t('account.password.errors.tooShort')); return; }
    if (newPw !== confirmPw) { setPwError(t('account.password.errors.mismatch')); return; }
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
        crumbs={[t('account.crumb')]}
        title={<>{t('account.titlePrefix')} <span style={{ color: tok.lime }}>{t('account.titleAccent')}</span></>}
        subtitle={t('account.subtitle')}
        actions={
          <div style={{ display: 'flex', gap: 8 }}>
            <NCBtn theme={theme} small ghost iconLeft={<I.lock size={13} />} onClick={lock}>
              {t('common.lock')}
            </NCBtn>
            <NCBtn theme={theme} small ghost iconLeft={<I.x size={13} />} onClick={onSignOut}>
              {t('common.signOut')}
            </NCBtn>
          </div>
        }
      />
      <div style={{ flex: 1, overflow: 'auto', padding: 28 }}>
        <div style={{ maxWidth: 720, display: 'flex', flexDirection: 'column', gap: 20 }}>

          <NCCard theme={theme} pad={24} style={{ display: 'flex', gap: 20, alignItems: 'center' }}>
            <div style={{
              width: 72, height: 72, borderRadius: 4,
              background: tok.lime, color: '#0A0A0A',
              display: 'flex', alignItems: 'center', justifyContent: 'center',
              fontFamily: NC_FONT_DISPLAY, fontWeight: 800, fontSize: 32,
            }}>{initial}</div>
            <div style={{ flex: 1 }}>
              <div style={{
                fontFamily: NC_FONT_DISPLAY, fontWeight: 800,
                fontSize: 22, color: tok.textHi, letterSpacing: -0.5, marginBottom: 4,
              }}>{account?.username ?? '…'}</div>
              <div style={{ fontSize: 13, color: tok.textMd, marginBottom: 8 }}>{t('account.localAccount')}</div>
              <div style={{ display: 'flex', gap: 6 }}>
                <NCBadge theme={theme} color="lime">{t('account.badge.free')}</NCBadge>
                <NCBadge theme={theme} color="muted">{t('account.badge.earlyAccess')}</NCBadge>
                {memberYear && <NCBadge theme={theme} color="muted">{t('account.badge.memberSince', { year: memberYear })}</NCBadge>}
              </div>
            </div>
          </NCCard>

          <NCCard theme={theme} pad={20} style={{ border: `1px solid ${tok.lime}`, background: tok.limeSoft }}>
            <div style={{ display: 'flex', gap: 14, alignItems: 'flex-start' }}>
              <I.shield size={18} color={tok.lime} style={{ marginTop: 2 }} />
              <div style={{ flex: 1 }}>
                <div style={{ fontFamily: NC_FONT_DISPLAY, fontWeight: 800, fontSize: 16, color: tok.textHi, marginBottom: 6 }}>
                  {t('account.earlyAccess.title')}
                </div>
                <div style={{ fontSize: 13, color: tok.textMd, lineHeight: 1.55, marginBottom: 10 }}>
                  {t('account.earlyAccess.body')}
                </div>
                <div style={{ fontFamily: NC_FONT_MONO, fontSize: 10, color: tok.textMd, letterSpacing: 1.5 }}>
                  {t('account.earlyAccess.footer')}
                </div>
              </div>
            </div>
          </NCCard>

          <NCCard theme={theme} pad={20}>
            <div style={{ display: 'flex', alignItems: 'center', marginBottom: 14 }}>
              <NCEyebrow theme={theme}>{t('account.security')}</NCEyebrow>
            </div>

            {/* Password row */}
            <div style={{ padding: '12px 0', borderBottom: `1px solid ${tok.border}` }}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 14, marginBottom: changingPw ? 14 : 0 }}>
                <div style={{
                  width: 32, height: 32, borderRadius: 3, background: tok.surface2,
                  border: `1px solid ${tok.border}`,
                  display: 'flex', alignItems: 'center', justifyContent: 'center',
                }}>
                  <I.lock size={14} color={tok.textMd} />
                </div>
                <div style={{ flex: 1 }}>
                  <div style={{ fontSize: 13, fontWeight: 500, color: tok.textHi }}>{t('account.password.label')}</div>
                  <div style={{ fontSize: 11, color: tok.textMd, fontFamily: NC_FONT_MONO, letterSpacing: 0.5 }}>
                    {t('account.password.sub')}
                  </div>
                </div>
                <NCBtn theme={theme} small ghost onClick={() => {
                  setChangingPw(v => !v);
                  setPwError(null); setPwSuccess(false);
                  setCurrentPw(''); setNewPw(''); setConfirmPw('');
                }}>
                  {changingPw ? t('common.cancel') : t('account.password.change')}
                </NCBtn>
              </div>

              {changingPw && (
                <div style={{ display: 'flex', flexDirection: 'column', gap: 10, paddingLeft: 46 }}>
                  <NCInput theme={theme} type="password" value={currentPw} onChange={setCurrentPw} placeholder={t('account.password.currentPlaceholder')} />
                  <NCInput theme={theme} type="password" value={newPw} onChange={setNewPw} placeholder={t('account.password.newPlaceholder')} />
                  <NCInput theme={theme} type="password" value={confirmPw} onChange={setConfirmPw} placeholder={t('account.password.confirmPlaceholder')} />
                  {pwError && (
                    <div style={{ fontSize: 12, color: tok.danger, display: 'flex', gap: 6, alignItems: 'center' }}>
                      <I.warn size={12} color={tok.danger} /> {pwError}
                    </div>
                  )}
                  <NCBtn theme={theme} small primary onClick={handleChangePassword} disabled={busy}>
                    {busy ? t('account.password.saving') : t('account.password.save')}
                  </NCBtn>
                </div>
              )}
              {pwSuccess && !changingPw && (
                <div style={{ fontSize: 12, color: tok.lime, paddingLeft: 46, paddingTop: 8 }}>
                  {t('account.password.changed')}
                </div>
              )}
            </div>

            {/* Credential storage row */}
            <div style={{ display: 'flex', alignItems: 'center', gap: 14, padding: '12px 0' }}>
              <div style={{
                width: 32, height: 32, borderRadius: 3, background: tok.surface2,
                border: `1px solid ${tok.border}`,
                display: 'flex', alignItems: 'center', justifyContent: 'center',
              }}>
                <I.shield size={14} color={tok.textMd} />
              </div>
              <div style={{ flex: 1 }}>
                <div style={{ fontSize: 13, fontWeight: 500, color: tok.textHi }}>{t('account.credentials.label')}</div>
                <div style={{ fontSize: 11, color: tok.textMd, fontFamily: NC_FONT_MONO, letterSpacing: 0.5 }}>
                  {t('account.credentials.sub')}
                </div>
              </div>
            </div>
          </NCCard>

        </div>
      </div>
    </>
  );
};
