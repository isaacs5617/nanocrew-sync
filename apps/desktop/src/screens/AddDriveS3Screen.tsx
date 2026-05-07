// Generic S3-compatible add-drive form. One screen covers Wasabi, AWS S3,
// Backblaze B2, Cloudflare R2, MinIO, IDrive e2, DigitalOcean Spaces, Storj,
// Scaleway, Contabo, Oracle, Linode, and Vultr. Behavior is driven by the
// provider's entry in S3_PROVIDER_PRESETS:
//
//   - regions[]      → region/endpoint dropdown
//   - customEndpoint → free-form endpoint field + region hint
//   - fixedRegion    → region is auto-set and hidden
//
// The backend is identical for every S3-compatible provider: `test_connection`
// / `list_buckets` / `add_drive` just take endpoint + region + key pair.

import React from 'react';
import { useTranslation } from 'react-i18next';
import { invoke } from '@tauri-apps/api/core';
import {
  getTokens, NC_FONT_MONO,
  NCCard, NCEyebrow, NCLabel, NCBtn, NCInput, NCToggle, NCBadge,
  TopBar,
  S3_PROVIDER_PRESETS,
  type Theme, type S3ProviderPreset,
} from '@nanocrew/ui';
import { I } from '@nanocrew/ui';
import { useAuth } from '../context/auth.js';

interface AddDriveS3ScreenProps {
  theme: Theme;
  providerId: string;
  onBack: () => void;
  onCancel: () => void;
  onDone: () => void;
}

const Field: React.FC<{ label: string; children: React.ReactNode; theme: Theme; last?: boolean }> = ({
  label, children, theme, last,
}) => (
  <div style={{ marginBottom: last ? 0 : 14 }}>
    <NCLabel theme={theme}>{label}</NCLabel>
    {children}
  </div>
);

export const AddDriveS3Screen: React.FC<AddDriveS3ScreenProps> = ({
  theme, providerId, onBack, onCancel, onDone,
}) => {
  const tok = getTokens(theme);
  const { t } = useTranslation();
  const { token } = useAuth();

  const preset: S3ProviderPreset | undefined = S3_PROVIDER_PRESETS[providerId];

  const [name, setName] = React.useState('');
  // Region dropdown index (only used when preset has regions[] and !customEndpoint).
  const [regionIdx, setRegionIdx] = React.useState(0);
  // Custom-endpoint fields (used by MinIO, R2, Oracle).
  const [customEndpoint, setCustomEndpoint] = React.useState('');
  const [customRegion, setCustomRegion] = React.useState(preset?.fixedRegion ?? '');
  const [bucket, setBucket] = React.useState('');
  const [bucketPrefix, setBucketPrefix] = React.useState('');
  const [accessKeyId, setAccessKeyId] = React.useState('');
  const [secretKey, setSecretKey] = React.useState('');
  const [showSecret, setShowSecret] = React.useState(false);
  const [letter, setLetter] = React.useState('');
  const [availableLetters, setAvailableLetters] = React.useState<string[]>([]);
  const [cacheSizeGb, setCacheSizeGb] = React.useState(5);
  const [autoMount, setAutoMount] = React.useState(true);
  const [readonly, setReadonly] = React.useState(false);

  // Override the hard-coded defaults with the user's preferred defaults
  // (Settings → Drives). Runs once on mount.
  React.useEffect(() => {
    (async () => {
      try {
        const am = await invoke<string | null>('get_pref', { token, key: 'default_auto_mount' });
        if (am === '0' || am === 'false') setAutoMount(false);
        else if (am === '1' || am === 'true') setAutoMount(true);
        const ro = await invoke<string | null>('get_pref', { token, key: 'default_readonly' });
        if (ro === '1' || ro === 'true') setReadonly(true);
        else if (ro === '0' || ro === 'false') setReadonly(false);
      } catch {/* pref failures just leave the built-in defaults in place */}
    })();
  }, [token]);

  const [testing, setTesting] = React.useState(false);
  const [testOk, setTestOk] = React.useState<boolean | null>(null);
  const [saving, setSaving] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const [browsing, setBrowsing] = React.useState(false);
  const [availableBuckets, setAvailableBuckets] = React.useState<string[] | null>(null);

  // Resolve the endpoint+region the form should submit. This is the single
  // point where dropdown / custom fields / fixed region converge — every
  // downstream call (test / browse / mount) reads from here.
  const resolved = React.useMemo(() => {
    if (!preset) return { endpoint: '', region: '' };
    if (preset.customEndpoint) {
      return {
        endpoint: customEndpoint.trim(),
        region: (customRegion.trim() || preset.fixedRegion || 'us-east-1'),
      };
    }
    const r = preset.regions?.[regionIdx];
    if (!r) return { endpoint: '', region: preset.fixedRegion ?? '' };
    return { endpoint: r.endpoint, region: preset.fixedRegion ?? r.region };
  }, [preset, regionIdx, customEndpoint, customRegion]);

  React.useEffect(() => {
    invoke<string[]>('get_available_letters', { token })
      .then(letters => {
        setAvailableLetters(letters);
        if (letters.length > 0) setLetter(letters[0]!);
      })
      .catch(() => {});
  }, [token]);

  if (!preset) {
    return (
      <>
        <TopBar theme={theme} crumbs={[t('picker.crumb.drives'), t('picker.crumb.addDrive')]} title={t('addDrive.unknownProvider')} />
        <div style={{ padding: 28 }}>
          <div style={{ color: tok.danger, fontSize: 13 }}>
            {t('addDrive.noPreset', { id: providerId })}
          </div>
          <NCBtn theme={theme} ghost onClick={onBack} style={{ marginTop: 16 }}>{t('addDrive.back')}</NCBtn>
        </div>
      </>
    );
  }

  const handleTest = async () => {
    setError(null);
    setTestOk(null);
    if (!resolved.endpoint) {
      setError(t('addDrive.errors.endpointRequired'));
      return;
    }
    if (!bucket.trim() || !accessKeyId.trim() || !secretKey.trim()) {
      setError(t('addDrive.errors.fieldsRequired'));
      return;
    }
    setTesting(true);
    try {
      await invoke('test_connection', {
        token,
        input: {
          provider: preset.id,
          endpoint: resolved.endpoint,
          bucket: bucket.trim(),
          bucket_prefix: bucketPrefix.trim(),
          region: resolved.region,
          access_key_id: accessKeyId.trim(),
          secret_access_key: secretKey,
        },
      });
      setTestOk(true);
    } catch (e) {
      setTestOk(false);
      setError(prettifyError(String(e)));
    } finally {
      setTesting(false);
    }
  };

  const handleBrowse = async () => {
    setError(null);
    if (!resolved.endpoint) { setError(t('addDrive.errors.setEndpoint')); return; }
    if (!accessKeyId.trim() || !secretKey.trim()) {
      setError(t('addDrive.errors.enterKeys'));
      return;
    }
    setBrowsing(true);
    try {
      const buckets = await invoke<string[]>('list_buckets', {
        token,
        endpoint: resolved.endpoint,
        region: resolved.region,
        accessKeyId: accessKeyId.trim(),
        secretAccessKey: secretKey,
      });
      setAvailableBuckets(buckets);
      if (buckets.length > 0 && !bucket) setBucket(buckets[0]!);
    } catch (e) {
      const msg = String(e);
      const isForbidden = msg.includes('403') || msg.includes('Forbidden') || msg.includes('service error') || msg.includes('AccessDenied');
      setError(
        isForbidden
          ? t('addDrive.errors.listBucketsForbidden')
          : t('addDrive.errors.listBucketsFailed', { msg })
      );
    } finally {
      setBrowsing(false);
    }
  };

  const handleMount = async () => {
    setError(null);
    if (!name.trim()) { setError(t('addDrive.errors.nameRequired')); return; }
    if (!resolved.endpoint) { setError(t('addDrive.errors.endpointRequired')); return; }
    if (!bucket.trim()) { setError(t('addDrive.errors.bucketRequired')); return; }
    if (!accessKeyId.trim() || !secretKey.trim()) { setError(t('addDrive.errors.keysRequired')); return; }
    if (!letter) { setError(t('addDrive.errors.letterRequired')); return; }

    setSaving(true);
    try {
      await invoke('add_drive', {
        token,
        input: {
          name: name.trim(),
          provider: preset.id,
          endpoint: resolved.endpoint,
          bucket: bucket.trim(),
          bucket_prefix: bucketPrefix.trim(),
          region: resolved.region,
          letter,
          access_key_id: accessKeyId.trim(),
          secret_access_key: secretKey,
          cache_size_gb: cacheSizeGb,
          auto_mount: autoMount,
          readonly,
        },
      });
      onDone();
    } catch (e) {
      setError(prettifyError(String(e)));
    } finally {
      setSaving(false);
    }
  };

  const selectStyle: React.CSSProperties = {
    width: '100%',
    background: tok.surface1,
    border: `1px solid ${tok.border}`,
    borderRadius: 3,
    padding: '10px 12px',
    fontFamily: NC_FONT_MONO,
    fontSize: 12,
    color: tok.textHi,
    outline: 'none',
    cursor: 'pointer',
    appearance: 'none',
    WebkitAppearance: 'none',
  };

  return (
    <>
      <TopBar
        theme={theme}
        crumbs={[t('picker.crumb.drives'), t('picker.crumb.addDrive'), preset.name]}
        title={<>{t('picker.title')} <span style={{ color: tok.lime }}>{preset.name}</span></>}
        subtitle={`${preset.desc}. ${t('addDrive.credentialsSub')}`}
        actions={<NCBtn theme={theme} small ghost onClick={onCancel}>{t('common.cancel')}</NCBtn>}
      />
      <div style={{ flex: 1, overflow: 'auto', padding: 28 }}>
        <div style={{ maxWidth: 720, display: 'flex', flexDirection: 'column', gap: 24 }}>

          <div style={{ display: 'flex', alignItems: 'center', gap: 10, fontFamily: NC_FONT_MONO, fontSize: 10, letterSpacing: 1.5 }}>
            <span style={{ color: tok.lime }}>{t('addDrive.step1')}</span>
            <span style={{ color: tok.textFaint }}>—</span>
            <span style={{ color: tok.lime }}>{t('addDrive.step2')}</span>
            <span style={{ color: tok.textFaint }}>—</span>
            <span style={{ color: tok.textLo }}>{t('addDrive.step3')}</span>
          </div>

          <NCCard theme={theme} pad={24}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 16 }}>
              <NCEyebrow theme={theme}>{t('addDrive.section.connection')}</NCEyebrow>
              <div style={{ flex: 1 }} />
              {preset.badges.map(b => (
                <NCBadge key={b.label} theme={theme} color={b.color}>{b.label}</NCBadge>
              ))}
            </div>
            <Field theme={theme} label={t('addDrive.field.displayName')}>
              <NCInput theme={theme} value={name} onChange={setName} placeholder={`e.g. ${preset.name} · Main`} />
            </Field>

            {preset.customEndpoint ? (
              <>
                <Field theme={theme} label={t('addDrive.field.endpoint', { example: endpointExample(preset.id) })}>
                  <NCInput
                    theme={theme} mono
                    value={customEndpoint}
                    onChange={setCustomEndpoint}
                    placeholder={endpointExample(preset.id)}
                    prefix={<I.serverDb size={13} />}
                  />
                </Field>
                {!preset.fixedRegion && (
                  <Field theme={theme} label={t('addDrive.field.region')}>
                    <NCInput
                      theme={theme} mono
                      value={customRegion}
                      onChange={setCustomRegion}
                      placeholder="us-east-1"
                    />
                  </Field>
                )}
              </>
            ) : (
              <Field theme={theme} label={t('addDrive.field.regionEndpoint')}>
                <div style={{ position: 'relative' }}>
                  <div style={{ position: 'absolute', right: 12, top: '50%', transform: 'translateY(-50%)', pointerEvents: 'none' }}>
                    <I.chevD size={13} color={tok.textMd} />
                  </div>
                  <select
                    value={regionIdx}
                    onChange={e => setRegionIdx(Number(e.target.value))}
                    style={selectStyle}
                  >
                    {preset.regions!.map((r, i) => (
                      <option key={r.endpoint} value={i}>{r.label}</option>
                    ))}
                  </select>
                </div>
              </Field>
            )}

            <Field theme={theme} label={t('addDrive.field.bucket')}>
              <div style={{ display: 'flex', gap: 8 }}>
                {availableBuckets ? (
                  <div style={{ position: 'relative', flex: 1 }}>
                    <div style={{ position: 'absolute', right: 12, top: '50%', transform: 'translateY(-50%)', pointerEvents: 'none' }}>
                      <I.chevD size={13} color={tok.textMd} />
                    </div>
                    <select
                      value={bucket}
                      onChange={e => setBucket(e.target.value)}
                      style={{ ...selectStyle, fontFamily: NC_FONT_MONO, fontSize: 12 }}
                    >
                      {availableBuckets.map(b => <option key={b} value={b}>{b}</option>)}
                    </select>
                  </div>
                ) : (
                  <div style={{ flex: 1 }}>
                    <NCInput theme={theme} mono value={bucket} onChange={setBucket} placeholder="my-bucket-name" prefix={<I.serverDb size={13} />} />
                  </div>
                )}
                <NCBtn theme={theme} small ghost disabled={browsing} onClick={handleBrowse}>
                  {browsing ? t('addDrive.browsing') : availableBuckets ? t('addDrive.refresh') : t('addDrive.browse')}
                </NCBtn>
              </div>
            </Field>
            <Field theme={theme} label={t('addDrive.field.folderPrefix')} last>
              <NCInput
                theme={theme} mono
                value={bucketPrefix}
                onChange={setBucketPrefix}
                placeholder="e.g. users/alice  or  team/shared/projects"
                prefix={<I.folder size={13} />}
              />
              <div style={{ marginTop: 6, fontSize: 11, color: tok.textMd }}>
                {t('addDrive.field.folderPrefixHint')}
              </div>
            </Field>
          </NCCard>

          <NCCard theme={theme} pad={24}>
            <NCEyebrow theme={theme} style={{ marginBottom: 16 }}>{t('addDrive.section.credentials')}</NCEyebrow>
            <Field theme={theme} label={t('addDrive.field.accessKeyId')}>
              <NCInput theme={theme} mono value={accessKeyId} onChange={setAccessKeyId} placeholder={preset.keyIdHint ?? 'AKIAXXXXXXXXXXXXXXXX'} prefix={<I.lock size={13} />} />
            </Field>
            <Field theme={theme} label={t('addDrive.field.secretKey')} last>
              <NCInput
                theme={theme} mono
                type={showSecret ? 'text' : 'password'}
                value={secretKey}
                onChange={setSecretKey}
                placeholder="········"
                prefix={<I.lock size={13} />}
                suffix={
                  <span style={{ cursor: 'pointer' }} onClick={() => setShowSecret(v => !v)}>
                    {showSecret ? <I.eyeOff size={14} /> : <I.eye size={14} />}
                  </span>
                }
              />
            </Field>
            <div style={{
              marginTop: 14, padding: '10px 12px',
              background: tok.surface2, border: `1px solid ${tok.border}`,
              borderRadius: 3, display: 'flex', alignItems: 'flex-start', gap: 10,
            }}>
              <I.shield size={14} color={tok.lime} style={{ marginTop: 2 }} />
              <div style={{ fontSize: 11, color: tok.textMd, lineHeight: 1.6 }}>
                {t('addDrive.credentialNote')}
                {preset.docsUrl && (
                  <> · <a href={preset.docsUrl} target="_blank" rel="noreferrer" style={{ color: tok.lime }}>{preset.name} docs</a></>
                )}
              </div>
            </div>
          </NCCard>

          <NCCard theme={theme} pad={24}>
            <NCEyebrow theme={theme} style={{ marginBottom: 16 }}>{t('addDrive.section.mountOptions')}</NCEyebrow>
            <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 16 }}>
              <Field theme={theme} label={t('addDrive.field.driveLetter')}>
                <div style={{ position: 'relative' }}>
                  <div style={{ position: 'absolute', right: 12, top: '50%', transform: 'translateY(-50%)', pointerEvents: 'none' }}>
                    <I.chevD size={13} color={tok.textMd} />
                  </div>
                  <select
                    value={letter}
                    onChange={e => setLetter(e.target.value)}
                    style={{ ...selectStyle, fontSize: 16, fontWeight: 500, color: tok.lime }}
                  >
                    {availableLetters.length === 0
                      ? <option value="">{t('addDrive.noLetters')}</option>
                      : availableLetters.map(l => <option key={l} value={l}>{l}</option>)
                    }
                  </select>
                </div>
              </Field>
              <Field theme={theme} label={t('addDrive.field.cache')}>
                <div style={{ position: 'relative' }}>
                  <div style={{ position: 'absolute', right: 12, top: '50%', transform: 'translateY(-50%)', pointerEvents: 'none' }}>
                    <I.chevD size={13} color={tok.textMd} />
                  </div>
                  <select
                    value={cacheSizeGb}
                    onChange={e => setCacheSizeGb(Number(e.target.value))}
                    style={selectStyle}
                  >
                    {[1, 2, 5, 10, 20, 50].map(gb => (
                      <option key={gb} value={gb}>{t('addDrive.cacheOption', { gb })}</option>
                    ))}
                  </select>
                </div>
              </Field>
            </div>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 14, marginTop: 18 }}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 14 }}>
                <div style={{ flex: 1 }}>
                  <div style={{ fontSize: 13, color: tok.textHi, fontWeight: 500 }}>{t('addDrive.autoMount.label')}</div>
                  <div style={{ fontSize: 11, color: tok.textMd, marginTop: 2 }}>{t('addDrive.autoMount.sub')}</div>
                </div>
                <NCToggle on={autoMount} onChange={setAutoMount} theme={theme} />
              </div>
              <div style={{ display: 'flex', alignItems: 'center', gap: 14 }}>
                <div style={{ flex: 1 }}>
                  <div style={{ fontSize: 13, color: tok.textHi, fontWeight: 500 }}>{t('addDrive.readonly.label')}</div>
                  <div style={{ fontSize: 11, color: tok.textMd, marginTop: 2 }}>{t('addDrive.readonly.sub')}</div>
                </div>
                <NCToggle on={readonly} onChange={setReadonly} theme={theme} />
              </div>
            </div>
          </NCCard>

          {testOk === true && (
            <div style={{
              padding: '10px 14px',
              background: `${tok.lime}18`, border: `1px solid ${tok.lime}50`,
              borderRadius: 3, fontSize: 12, color: tok.lime,
              display: 'flex', gap: 8, alignItems: 'center',
            }}>
              <I.shield size={13} color={tok.lime} style={{ flexShrink: 0 }} />
              {t('addDrive.connectionSuccess')}
            </div>
          )}

          {error && (
            <div style={{
              padding: '10px 14px',
              background: `${tok.danger}18`, border: `1px solid ${tok.danger}50`,
              borderRadius: 3, fontSize: 12, color: tok.danger,
              display: 'flex', gap: 8, alignItems: 'center',
            }}>
              <I.warn size={13} color={tok.danger} style={{ flexShrink: 0 }} />
              {error}
            </div>
          )}

          <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', gap: 12 }}>
            <NCBtn theme={theme} ghost iconLeft={<I.chevL size={14} />} onClick={onBack}>{t('addDrive.back')}</NCBtn>
            <div style={{ display: 'flex', gap: 8 }}>
              <NCBtn theme={theme} disabled={testing} onClick={handleTest}>
                {testing ? t('addDrive.testing') : testOk === true ? t('addDrive.testPassed') : t('addDrive.testConnection')}
              </NCBtn>
              <NCBtn theme={theme} primary icon={<I.arrow size={13} />} disabled={saving} onClick={handleMount}>
                {saving ? t('addDrive.adding') : t('addDrive.mountDrive')}
              </NCBtn>
            </div>
          </div>
        </div>
      </div>
    </>
  );
};

// ── Helpers ──────────────────────────────────────────────────────────────────

/** Suggest a realistic endpoint template for providers without a dropdown. */
function endpointExample(providerId: string): string {
  switch (providerId) {
    case 'r2':      return '<account-id>.r2.cloudflarestorage.com';
    case 'minio':   return 'minio.example.com:9000';
    case 'oracle':  return '<namespace>.compat.objectstorage.us-ashburn-1.oraclecloud.com';
    default:        return 's3.example.com';
  }
}

/** Map a noisy aws-sdk error string into something a human can act on. */
function prettifyError(msg: string): string {
  if (/AccessDenied|Forbidden|403/.test(msg))        return 'Access denied — check that the key has permission to access this bucket.';
  if (/InvalidAccessKeyId/.test(msg))                return 'Invalid access key ID.';
  if (/SignatureDoesNotMatch/.test(msg))             return 'Signature did not match — check the secret key.';
  if (/NoSuchBucket/.test(msg))                      return 'Bucket not found at this endpoint. Check the name and region.';
  if (/dns error|NameResolutionFailure/.test(msg))   return 'Could not resolve the endpoint hostname. Check the endpoint spelling.';
  if (/timed out|timeout/.test(msg))                 return 'Connection timed out — check your network and firewall.';
  if (/500|InternalError|internal server error/i.test(msg)) return 'The storage provider returned a server error (500). Check the bucket name, prefix, and permissions, or try again shortly.';
  if (/service.?error|ServiceError/i.test(msg))      return 'The storage provider rejected the request. Check the endpoint, bucket name, region, and key permissions.';
  return msg;
}
