import React from 'react';
import {
  getTokens, NC_FONT_DISPLAY, NC_FONT_MONO,
  NCCard, NCEyebrow, NCBtn,
  TopBar,
  type Theme,
} from '@nanocrew/ui';
import { I } from '@nanocrew/ui';

interface ErrorScreenProps { theme: Theme }

export const ErrorScreen: React.FC<ErrorScreenProps> = ({ theme }) => {
  const t = getTokens(theme);
  return (
    <>
      <TopBar
        theme={theme}
        crumbs={['Drives', 'Backblaze B2 · Archive']}
        title={<>Drive <span style={{ color: t.danger }}>unreachable</span></>}
        subtitle="W:\\ Backblaze B2 · Archive — last contact 4m ago"
        actions={<>
          <NCBtn theme={theme} small>View logs</NCBtn>
          <NCBtn theme={theme} small primary iconLeft={<I.refresh size={13} />}>Reconnect</NCBtn>
        </>}
      />
      <div style={{ flex: 1, overflow: 'auto', padding: 28 }}>
        <div style={{ maxWidth: 800, display: 'flex', flexDirection: 'column', gap: 20 }}>

          <div style={{
            padding: 20, border: `1px solid ${t.danger}`,
            background: 'rgba(255,77,77,0.06)', borderRadius: 4,
            display: 'flex', gap: 16, alignItems: 'flex-start',
          }}>
            <I.warn size={20} color={t.danger} style={{ marginTop: 2 }} />
            <div style={{ flex: 1 }}>
              <div style={{ fontFamily: NC_FONT_DISPLAY, fontWeight: 800, fontSize: 18, color: t.textHi, marginBottom: 8 }}>
                Authentication failed · HTTP 403
              </div>
              <div style={{ fontSize: 13, color: t.textMd, lineHeight: 1.55, marginBottom: 12 }}>
                The application key used to mount <span style={{ color: t.textHi, fontFamily: NC_FONT_MONO }}>nc-archive-frozen</span> has
                been revoked or no longer has read permission on this bucket. The drive has been unmounted and cached
                writes are being held in queue.
              </div>
              <div style={{ fontFamily: NC_FONT_MONO, fontSize: 10, color: t.textMd, letterSpacing: 1 }}>
                ERR_AUTH_REVOKED · 2026-04-18 14:23:02 UTC · request-id 4F82-EA21
              </div>
            </div>
          </div>

          <NCCard theme={theme} pad={20}>
            <NCEyebrow theme={theme} style={{ marginBottom: 14 }}>Pending writes · 24 files · 2.1 GB</NCEyebrow>
            <div style={{ fontSize: 12, color: t.textMd, lineHeight: 1.55, marginBottom: 14 }}>
              These changes were made locally while the drive was offline. They will be pushed automatically when
              the connection is restored. You can also save them to another drive or export a backup.
            </div>
            <div style={{ display: 'flex', gap: 8 }}>
              <NCBtn theme={theme} small>Export as .zip</NCBtn>
              <NCBtn theme={theme} small>Move to another drive</NCBtn>
              <NCBtn theme={theme} small danger>Discard</NCBtn>
            </div>
          </NCCard>

          <NCCard theme={theme} pad={20}>
            <NCEyebrow theme={theme} style={{ marginBottom: 14 }}>Fix this</NCEyebrow>
            {[
              { n: '01', title: 'Update credentials', desc: 'Paste a fresh application key and reconnect.', cta: 'Update' },
              { n: '02', title: 'Verify bucket permissions', desc: 'Open the Backblaze console and check b2:ReadFile / b2:WriteFile on this key.', cta: 'Open console' },
              { n: '03', title: 'Check network & firewall', desc: 's3.us-west-004.backblazeb2.com — TCP 443 outbound must be allowed.', cta: 'Run diagnostic' },
            ].map((s, i) => (
              <div key={i} style={{
                display: 'flex', gap: 16, padding: '12px 0',
                borderTop: i === 0 ? 'none' : `1px solid ${t.border}`,
                alignItems: 'center',
              }}>
                <span style={{ fontFamily: NC_FONT_MONO, fontSize: 11, color: t.lime, letterSpacing: 1.5 }}>{s.n}</span>
                <div style={{ flex: 1 }}>
                  <div style={{ fontSize: 13, fontWeight: 500, color: t.textHi }}>{s.title}</div>
                  <div style={{ fontSize: 12, color: t.textMd, marginTop: 2 }}>{s.desc}</div>
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
