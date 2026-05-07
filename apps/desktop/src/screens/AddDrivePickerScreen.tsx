import React from 'react';
import { useTranslation } from 'react-i18next';
import {
  getTokens, NC_FONT_DISPLAY, NC_FONT_MONO,
  NCCard, NCEyebrow, NCBtn, NCBadge, ProviderIcon,
  S3_PROVIDER_PRESETS, S3_PROVIDER_ORDER,
  TopBar,
  type Theme,
} from '@nanocrew/ui';
import { I } from '@nanocrew/ui';

interface AddDrivePickerScreenProps {
  theme: Theme;
  /** Called with the provider id (e.g. 'wasabi', 's3', 'b2'). */
  onNext: (providerId: string) => void;
  onCancel: () => void;
}

const RECOMMENDED_ID = 'wasabi';

export const AddDrivePickerScreen: React.FC<AddDrivePickerScreenProps> = ({ theme, onNext, onCancel }) => {
  const tok = getTokens(theme);
  const { t } = useTranslation();
  const recommended = S3_PROVIDER_PRESETS[RECOMMENDED_ID]!;
  // Every other S3-compatible provider, in the order declared by the preset file.
  const others = S3_PROVIDER_ORDER
    .filter(id => id !== RECOMMENDED_ID)
    .map(id => S3_PROVIDER_PRESETS[id])
    .filter((p): p is NonNullable<typeof p> => !!p);

  return (
    <>
      <TopBar
        theme={theme}
        crumbs={[t('picker.crumb.drives'), t('picker.crumb.addDrive')]}
        title={<>{t('picker.title')} <span style={{ color: tok.lime }}>{t('picker.titleAccent')}</span></>}
        subtitle={t('picker.subtitle')}
        actions={<NCBtn theme={theme} small ghost onClick={onCancel}>{t('common.cancel')}</NCBtn>}
      />
      <div style={{ flex: 1, overflow: 'auto', padding: 28 }}>

        {/* ── Recommended ─────────────────────────────────────────────────── */}
        <NCEyebrow theme={theme} accent style={{ marginBottom: 12 }}>{t('picker.recommended')}</NCEyebrow>
        <NCCard theme={theme} pad={24} style={{
          border: `1px solid ${tok.lime}`, background: tok.limeSoft,
          display: 'flex', alignItems: 'center', gap: 20, marginBottom: 32,
        }}>
          <div style={{
            width: 56, height: 56, borderRadius: 4,
            background: tok.lime, color: '#0A0A0A',
            display: 'flex', alignItems: 'center', justifyContent: 'center',
            fontFamily: NC_FONT_DISPLAY, fontWeight: 800, fontSize: 28,
          }}>W</div>
          <div style={{ flex: 1 }}>
            <div style={{
              fontFamily: NC_FONT_DISPLAY, fontWeight: 800, fontSize: 22,
              color: tok.textHi, letterSpacing: -0.5, marginBottom: 4,
            }}>{recommended.name}</div>
            <div style={{ fontSize: 13, color: tok.textMd, lineHeight: 1.5, marginBottom: 8 }}>
              {recommended.desc}. {t('picker.worksOutOfBox')}
            </div>
            <div style={{ display: 'flex', gap: 6 }}>
              {recommended.badges.map(b => (
                <NCBadge key={b.label} theme={theme} color={b.color}>{b.label}</NCBadge>
              ))}
            </div>
          </div>
          <NCBtn theme={theme} primary iconLeft={<I.arrow size={13} />} onClick={() => onNext(recommended.id)}>
            {t('picker.connectBtn', { name: recommended.name })}
          </NCBtn>
        </NCCard>

        {/* ── All S3-compatible providers ─────────────────────────────────── */}
        <NCEyebrow theme={theme} style={{ marginBottom: 12 }}>{t('picker.allProviders')}</NCEyebrow>
        <div style={{ display: 'grid', gridTemplateColumns: 'repeat(3, 1fr)', gap: 8, marginBottom: 32 }}>
          {others.map(p => (
            <NCCard
              key={p.id} theme={theme} pad={16}
              onClick={() => onNext(p.id)}
              style={{
                display: 'flex', alignItems: 'center', gap: 14,
                cursor: 'pointer',
              }}
            >
              <ProviderIcon id={p.id} size={28} theme={theme} />
              <div style={{ flex: 1, minWidth: 0 }}>
                <div style={{ fontSize: 13, fontWeight: 500, color: tok.textHi }}>{p.name}</div>
                <div style={{
                  fontSize: 11, color: tok.textMd, lineHeight: 1.4,
                  whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis',
                }}>{p.desc}</div>
              </div>
              <I.arrow size={13} color={tok.textMd} />
            </NCCard>
          ))}
        </div>

        {/* ── Other protocols ──────────────────────────────────────────────── */}
        <NCEyebrow theme={theme} style={{ marginBottom: 12 }}>Other protocols</NCEyebrow>
        <div style={{ display: 'grid', gridTemplateColumns: 'repeat(3, 1fr)', gap: 8, marginBottom: 32 }}>
          {[
            { id: 'sftp',    name: 'SFTP',         desc: 'SSH File Transfer Protocol' },
            { id: 'ftp',     name: 'FTP/FTPS',     desc: 'File Transfer Protocol (plain or TLS)' },
            { id: 'webdav',  name: 'WebDAV',        desc: 'Generic WebDAV servers (Nextcloud, Seafile…)' },
            { id: 'dropbox', name: 'Dropbox',       desc: 'Personal & team folders' },
            { id: 'gdrive',  name: 'Google Drive',  desc: 'Personal & workspace drives' },
            { id: 'onedrive', name: 'OneDrive',       desc: 'Personal & business' },
          ].map(p => (
            <NCCard
              key={p.id} theme={theme} pad={16}
              onClick={() => onNext(p.id)}
              style={{ display: 'flex', alignItems: 'center', gap: 14, cursor: 'pointer' }}
            >
              <ProviderIcon id={p.id} size={28} theme={theme} />
              <div style={{ flex: 1, minWidth: 0 }}>
                <div style={{ fontSize: 13, fontWeight: 500, color: tok.textHi }}>{p.name}</div>
                <div style={{
                  fontSize: 11, color: tok.textMd, lineHeight: 1.4,
                  whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis',
                }}>{p.desc}</div>
              </div>
              <I.arrow size={13} color={tok.textMd} />
            </NCCard>
          ))}
        </div>



        <div style={{
          marginTop: 24, fontSize: 11, color: tok.textMd, fontFamily: NC_FONT_MONO,
          letterSpacing: 0.5, textAlign: 'center',
        }}>
          {t('picker.providerCount', { count: others.length + 1 })}
        </div>
      </div>
    </>
  );
};
