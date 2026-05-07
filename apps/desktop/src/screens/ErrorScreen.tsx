import React from 'react';
import { useTranslation } from 'react-i18next';
import {
  getTokens, NC_FONT_DISPLAY, NC_FONT_MONO,
  NCCard, NCEyebrow, NCBtn,
  TopBar,
  type Theme,
} from '@nanocrew/ui';
import { I } from '@nanocrew/ui';

interface ErrorScreenProps { theme: Theme }

export const ErrorScreen: React.FC<ErrorScreenProps> = ({ theme }) => {
  const { t } = useTranslation();
  const tok = getTokens(theme);
  return (
    <>
      <TopBar
        theme={theme}
        crumbs={['Drives', 'Backblaze B2 · Archive']}
        title={<>Drive <span style={{ color: tok.danger }}>{t('error.title')}</span></>}
        subtitle="W:\\ Backblaze B2 · Archive — last contact 4m ago"
        actions={<>
          <NCBtn theme={theme} small>{t('error.viewLogs')}</NCBtn>
          <NCBtn theme={theme} small primary iconLeft={<I.refresh size={13} />}>{t('error.reconnect')}</NCBtn>
        </>}
      />
      <div style={{ flex: 1, overflow: 'auto', padding: 28 }}>
        <div style={{ maxWidth: 800, display: 'flex', flexDirection: 'column', gap: 20 }}>

          <div style={{
            padding: 20, border: `1px solid ${tok.danger}`,
            background: 'rgba(255,77,77,0.06)', borderRadius: 4,
            display: 'flex', gap: 16, alignItems: 'flex-start',
          }}>
            <I.warn size={20} color={tok.danger} style={{ marginTop: 2 }} />
            <div style={{ flex: 1 }}>
              <div style={{ fontFamily: NC_FONT_DISPLAY, fontWeight: 800, fontSize: 18, color: tok.textHi, marginBottom: 8 }}>
                Authentication failed · HTTP 403
              </div>
              <div style={{ fontSize: 13, color: tok.textMd, lineHeight: 1.55, marginBottom: 12 }}>
                The application key used to mount <span style={{ color: tok.textHi, fontFamily: NC_FONT_MONO }}>nc-archive-frozen</span> has
                been revoked or no longer has read permission on this bucket. The drive has been unmounted and cached
                writes are being held in queue.
              </div>
              <div style={{ fontFamily: NC_FONT_MONO, fontSize: 10, color: tok.textMd, letterSpacing: 1 }}>
                ERR_AUTH_REVOKED · 2026-04-18 14:23:02 UTC · request-id 4F82-EA21
              </div>
            </div>
          </div>

          <NCCard theme={theme} pad={20}>
            <NCEyebrow theme={theme} style={{ marginBottom: 14 }}>{t('error.pendingWrites', { count: 24, size: '2.1 GB' })}</NCEyebrow>
            <div style={{ fontSize: 12, color: tok.textMd, lineHeight: 1.55, marginBottom: 14 }}>
              {t('error.pendingDesc')}
            </div>
            <div style={{ display: 'flex', gap: 8 }}>
              <NCBtn theme={theme} small>{t('error.exportZip')}</NCBtn>
              <NCBtn theme={theme} small>{t('error.moveToAnotherDrive')}</NCBtn>
              <NCBtn theme={theme} small danger>{t('error.discard')}</NCBtn>
            </div>
          </NCCard>

          <NCCard theme={theme} pad={20}>
            <NCEyebrow theme={theme} style={{ marginBottom: 14 }}>{t('error.fixThis')}</NCEyebrow>
            {[
              { n: '01', title: 'Update credentials', desc: 'Paste a fresh application key and reconnect.', cta: 'Update' },
              { n: '02', title: 'Verify bucket permissions', desc: 'Open the Backblaze console and check b2:ReadFile / b2:WriteFile on this key.', cta: 'Open console' },
              { n: '03', title: 'Check network & firewall', desc: 's3.us-west-004.backblazeb2.com — TCP 443 outbound must be allowed.', cta: 'Run diagnostic' },
            ].map((s, i) => (
              <div key={i} style={{
                display: 'flex', gap: 16, padding: '12px 0',
                borderTop: i === 0 ? 'none' : `1px solid ${tok.border}`,
                alignItems: 'center',
              }}>
                <span style={{ fontFamily: NC_FONT_MONO, fontSize: 11, color: tok.lime, letterSpacing: 1.5 }}>{s.n}</span>
                <div style={{ flex: 1 }}>
                  <div style={{ fontSize: 13, fontWeight: 500, color: tok.textHi }}>{s.title}</div>
                  <div style={{ fontSize: 12, color: tok.textMd, marginTop: 2 }}>{s.desc}</div>
                </div>
                <NCBtn theme={theme} small>{s.cta}</NCBtn>
              </div>
            ))}
          </NCCard>
        </div>
      </div>
    </>
  );
};
