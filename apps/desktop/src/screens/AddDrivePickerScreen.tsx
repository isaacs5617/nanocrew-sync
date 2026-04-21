import React from 'react';
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
  const t = getTokens(theme);
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
        crumbs={['Drives', 'Add drive']}
        title={<>Choose a <span style={{ color: t.lime }}>provider</span></>}
        subtitle="Mount any S3-compatible bucket as a Windows drive letter. One form fits every provider — pick yours below."
        actions={<NCBtn theme={theme} small ghost onClick={onCancel}>Cancel</NCBtn>}
      />
      <div style={{ flex: 1, overflow: 'auto', padding: 28 }}>

        {/* ── Recommended ─────────────────────────────────────────────────── */}
        <NCEyebrow theme={theme} accent style={{ marginBottom: 12 }}>Recommended</NCEyebrow>
        <NCCard theme={theme} pad={24} style={{
          border: `1px solid ${t.lime}`, background: t.limeSoft,
          display: 'flex', alignItems: 'center', gap: 20, marginBottom: 32,
        }}>
          <div style={{
            width: 56, height: 56, borderRadius: 4,
            background: t.lime, color: '#0A0A0A',
            display: 'flex', alignItems: 'center', justifyContent: 'center',
            fontFamily: NC_FONT_DISPLAY, fontWeight: 800, fontSize: 28,
          }}>W</div>
          <div style={{ flex: 1 }}>
            <div style={{
              fontFamily: NC_FONT_DISPLAY, fontWeight: 800, fontSize: 22,
              color: t.textHi, letterSpacing: -0.5, marginBottom: 4,
            }}>{recommended.name}</div>
            <div style={{ fontSize: 13, color: t.textMd, lineHeight: 1.5, marginBottom: 8 }}>
              {recommended.desc}. Works with NanoCrew Sync out of the box.
            </div>
            <div style={{ display: 'flex', gap: 6 }}>
              {recommended.badges.map(b => (
                <NCBadge key={b.label} theme={theme} color={b.color}>{b.label}</NCBadge>
              ))}
            </div>
          </div>
          <NCBtn theme={theme} primary iconLeft={<I.arrow size={13} />} onClick={() => onNext(recommended.id)}>
            Connect {recommended.name}
          </NCBtn>
        </NCCard>

        {/* ── All S3-compatible providers ─────────────────────────────────── */}
        <NCEyebrow theme={theme} style={{ marginBottom: 12 }}>All S3-compatible providers</NCEyebrow>
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
                <div style={{ fontSize: 13, fontWeight: 500, color: t.textHi }}>{p.name}</div>
                <div style={{
                  fontSize: 11, color: t.textMd, lineHeight: 1.4,
                  whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis',
                }}>{p.desc}</div>
              </div>
              <I.arrow size={13} color={t.textMd} />
            </NCCard>
          ))}
        </div>

        {/* ── Coming soon ─────────────────────────────────────────────────── */}
        <NCEyebrow theme={theme} style={{ marginBottom: 12 }}>Coming soon</NCEyebrow>
        <div style={{ display: 'grid', gridTemplateColumns: 'repeat(3, 1fr)', gap: 8 }}>
          {[
            { id: 'gdrive',   name: 'Google Drive', desc: 'Personal & workspace drives' },
            { id: 'onedrive', name: 'OneDrive',     desc: 'Personal & business' },
            { id: 'dropbox',  name: 'Dropbox',      desc: 'Personal & team folders' },
            { id: 'sftp',     name: 'SFTP / FTP',   desc: 'Secure file transfer protocol' },
            { id: 'webdav',   name: 'WebDAV',       desc: 'Generic WebDAV servers' },
          ].map(p => (
            <NCCard key={p.id} theme={theme} pad={16} style={{
              display: 'flex', alignItems: 'center', gap: 14,
              opacity: 0.5, cursor: 'not-allowed', position: 'relative',
            }}>
              <ProviderIcon id={p.id} size={28} theme={theme} />
              <div style={{ flex: 1, minWidth: 0 }}>
                <div style={{ fontSize: 13, fontWeight: 500, color: t.textHi }}>{p.name}</div>
                <div style={{
                  fontSize: 11, color: t.textMd, lineHeight: 1.4,
                  whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis',
                }}>{p.desc}</div>
              </div>
              <NCBadge theme={theme} color="muted">SOON</NCBadge>
            </NCCard>
          ))}
        </div>

        <div style={{
          marginTop: 24, fontSize: 11, color: t.textMd, fontFamily: NC_FONT_MONO,
          letterSpacing: 0.5, textAlign: 'center',
        }}>
          {others.length + 1} providers · more S3-compatible presets added every release
        </div>
      </div>
    </>
  );
};
