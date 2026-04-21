import React from 'react';
import {
  getTokens, NC_FONT_DISPLAY, NC_FONT_MONO,
  NCCard, NCEyebrow, NCBtn, NCBadge, ProviderIcon,
  PROVIDER_LIST, TopBar,
  type Theme,
} from '@nanocrew/ui';
import { I } from '@nanocrew/ui';

interface AddDrivePickerScreenProps {
  theme: Theme;
  onNext: (provider: string) => void;
  onCancel: () => void;
}

export const AddDrivePickerScreen: React.FC<AddDrivePickerScreenProps> = ({ theme, onNext, onCancel }) => {
  const t = getTokens(theme);
  return (
    <>
      <TopBar
        theme={theme}
        crumbs={['Drives', 'Add drive']}
        title={<>Choose a <span style={{ color: t.lime }}>provider</span></>}
        subtitle="Mount any S3-compatible bucket or cloud drive as a Windows drive letter."
        actions={<NCBtn theme={theme} small ghost onClick={onCancel}>Cancel</NCBtn>}
      />
      <div style={{ flex: 1, overflow: 'auto', padding: 28 }}>
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
            }}>Wasabi</div>
            <div style={{ fontSize: 13, color: t.textMd, lineHeight: 1.5, marginBottom: 8 }}>
              Hot cloud storage. S3-compatible, no egress fees. Works with NanoCrew Sync out of the box.
            </div>
            <div style={{ display: 'flex', gap: 6 }}>
              <NCBadge theme={theme} color="lime">S3-COMPATIBLE</NCBadge>
              <NCBadge theme={theme} color="lime">NO EGRESS</NCBadge>
              <NCBadge theme={theme} color="muted">13 REGIONS</NCBadge>
            </div>
          </div>
          <NCBtn theme={theme} primary iconLeft={<I.arrow size={13} />} onClick={() => onNext('wasabi')}>Connect Wasabi</NCBtn>
        </NCCard>

        <NCEyebrow theme={theme} style={{ marginBottom: 12 }}>All providers</NCEyebrow>
        <div style={{ display: 'grid', gridTemplateColumns: 'repeat(3, 1fr)', gap: 8 }}>
          {PROVIDER_LIST.filter(p => p.id !== 'wasabi').map(p => (
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
      </div>
    </>
  );
};
