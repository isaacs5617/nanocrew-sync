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
  const t = getTokens(theme);
  const { token } = useAuth();

  const preset: S3ProviderPreset | undefined = S3_PROVIDER_PRESETS[providerId];

  const [name, setName] = React.useState('');
  // Region dropdown index (only used when preset has regions[] and !customEndpoint).
  const [regionIdx, setRegionIdx] = React.useState(0);
  // Custom-endpoint fields (used by MinIO, R2, Oracle).
  const [customEndpoint, setCustomEndpoint] = React.useState('');
  const [customRegion, setCustomRegion] = React.useState(preset?.fixedRegion ?? '');
  const [bucket, setBucket] = React.useState('');
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
        <TopBar theme={theme} crumbs={['Drives', 'Add drive']} title="Unknown provider" />
        <div style={{ padding: 28 }}>
          <div style={{ color: t.danger, fontSize: 13 }}>
            No preset for <code>{providerId}</code>. Please report this.
          </div>
          <NCBtn theme={theme} ghost onClick={onBack} style={{ marginTop: 16 }}>Back</NCBtn>
        </div>
      </>
    );
  }

  const handleTest = async () => {
    setError(null);
    setTestOk(null);
    if (!resolved.endpoint) {
      setError('Endpoint is required.');
      return;
    }
    if (!bucket.trim() || !accessKeyId.trim() || !secretKey.trim()) {
      setError('Bucket, access key, and secret are required to test.');
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
    if (!resolved.endpoint) { setError('Set an endpoint first.'); return; }
    if (!accessKeyId.trim() || !secretKey.trim()) {
      setError('Enter your access key ID and secret key first.');
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
          ? 'Bucket listing requires the s3:ListAllMyBuckets permission. Your key may be scoped to a single bucket — enter the bucket name manually instead.'
          : `Could not list buckets: ${msg}`
      );
    } finally {
      setBrowsing(false);
    }
  };

  const handleMount = async () => {
    setError(null);
    if (!name.trim()) { setError('Display name is required.'); return; }
    if (!resolved.endpoint) { setError('Endpoint is required.'); return; }
    if (!bucket.trim()) { setError('Bucket is required.'); return; }
    if (!accessKeyId.trim() || !secretKey.trim()) { setError('Access key ID and secret are required.'); return; }
    if (!letter) { setError('Select a drive letter.'); return; }

    setSaving(true);
    try {
      await invoke('add_drive', {
        token,
        input: {
          name: name.trim(),
          provider: preset.id,
          endpoint: resolved.endpoint,
          bucket: bucket.trim(),
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

  return (
    <>
      <TopBar
        theme={theme}
        crumbs={['Drives', 'Add drive', preset.name]}
        title={<>Connect <span style={{ color: t.lime }}>{preset.name}</span></>}
        subtitle={`${preset.desc}. Credentials are stored in the Windows Credential Manager — never plain text.`}
        actions={<NCBtn theme={theme} small ghost onClick={onCancel}>Cancel</NCBtn>}
      />
      <div style={{ flex: 1, overflow: 'auto', padding: 28 }}>
        <div style={{ maxWidth: 720, display: 'flex', flexDirection: 'column', gap: 24 }}>

          <div style={{ display: 'flex', alignItems: 'center', gap: 10, fontFamily: NC_FONT_MONO, fontSize: 10, letterSpacing: 1.5 }}>
            <span style={{ color: t.lime }}>01 · PROVIDER</span>
            <span style={{ color: t.textFaint }}>—</span>
            <span style={{ color: t.lime }}>02 · CREDENTIALS</span>
            <span style={{ color: t.textFaint }}>—</span>
            <span style={{ color: t.textLo }}>03 · MOUNT</span>
          </div>

          <NCCard theme={theme} pad={24}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 16 }}>
              <NCEyebrow theme={theme}>Connection</NCEyebrow>
              <div style={{ flex: 1 }} />
              {preset.badges.map(b => (
                <NCBadge key={b.label} theme={theme} color={b.color}>{b.label}</NCBadge>
              ))}
            </div>
            <Field theme={theme} label="Display name">
              <NCInput theme={theme} value={name} onChange={setName} placeholder={`e.g. ${preset.name} · Main`} />
            </Field>

            {preset.customEndpoint ? (
              <>
                <Field theme={theme} label={`Endpoint (no https:// · e.g. ${endpointExample(preset.id)})`}>
                  <NCInput
                    theme={theme} mono
                    value={customEndpoint}
                    onChange={setCustomEndpoint}
                    placeholder={endpointExample(preset.id)}
                    prefix={<I.serverDb size={13} />}
                  />
                </Field>
                {!preset.fixedRegion && (
                  <Field theme={theme} label="Region">
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
              <Field theme={theme} label="Region endpoint">
                <div style={{ position: 'relative' }}>
                  <div style={{ position: 'absolute', right: 12, top: '50%', transform: 'translateY(-50%)', pointerEvents: 'none' }}>
                    <I.chevD size={13} color={t.textMd} />
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

            <Field theme={theme} label="Bucket" last>
              <div style={{ display: 'flex', gap: 8 }}>
                {availableBuckets ? (
                  <div style={{ position: 'relative', flex: 1 }}>
                    <div style={{ position: 'absolute', right: 12, top: '50%', transform: 'translateY(-50%)', pointerEvents: 'none' }}>
                      <I.chevD size={13} color={t.textMd} />
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
                  {browsing ? '…' : availableBuckets ? 'Refresh' : 'Browse'}
                </NCBtn>
              </div>
            </Field>
          </NCCard>

          <NCCard theme={theme} pad={24}>
            <NCEyebrow theme={theme} style={{ marginBottom: 16 }}>Credentials</NCEyebrow>
            <Field theme={theme} label="Access key ID">
              <NCInput theme={theme} mono value={accessKeyId} onChange={setAccessKeyId} placeholder={preset.keyIdHint ?? 'AKIAXXXXXXXXXXXXXXXX'} prefix={<I.lock size={13} />} />
            </Field>
            <Field theme={theme} label="Secret access key" last>
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
              background: t.surface2, border: `1px solid ${t.border}`,
              borderRadius: 3, display: 'flex', alignItems: 'flex-start', gap: 10,
            }}>
              <I.shield size={14} color={t.lime} style={{ marginTop: 2 }} />
              <div style={{ fontSize: 11, color: t.textMd, lineHeight: 1.6 }}>
                Stored encrypted in the <span style={{ color: t.textHi, fontFamily: NC_FONT_MONO }}>Windows Credential Manager</span>.
                NanoCrew Sync never sees or transmits your keys.
                {preset.docsUrl && (
                  <> · <a href={preset.docsUrl} target="_blank" rel="noreferrer" style={{ color: t.lime }}>{preset.name} docs</a></>
                )}
              </div>
            </div>
          </NCCard>

          <NCCard theme={theme} pad={24}>
            <NCEyebrow theme={theme} style={{ marginBottom: 16 }}>Mount options</NCEyebrow>
            <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 16 }}>
              <Field theme={theme} label="Drive letter">
                <div style={{ position: 'relative' }}>
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
              <Field theme={theme} label="Cache">
                <div style={{ position: 'relative' }}>
                  <div style={{ position: 'absolute', right: 12, top: '50%', transform: 'translateY(-50%)', pointerEvents: 'none' }}>
                    <I.chevD size={13} color={t.textMd} />
                  </div>
                  <select
                    value={cacheSizeGb}
                    onChange={e => setCacheSizeGb(Number(e.target.value))}
                    style={selectStyle}
                  >
                    {[1, 2, 5, 10, 20, 50].map(gb => (
                      <option key={gb} value={gb}>{gb} GB · Smart</option>
                    ))}
                  </select>
                </div>
              </Field>
            </div>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 14, marginTop: 18 }}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 14 }}>
                <div style={{ flex: 1 }}>
                  <div style={{ fontSize: 13, color: t.textHi, fontWeight: 500 }}>Mount automatically at Windows sign-in</div>
                  <div style={{ fontSize: 11, color: t.textMd, marginTop: 2 }}>Reconnect the drive when you log in.</div>
                </div>
                <NCToggle on={autoMount} onChange={setAutoMount} theme={theme} />
              </div>
              <div style={{ display: 'flex', alignItems: 'center', gap: 14 }}>
                <div style={{ flex: 1 }}>
                  <div style={{ fontSize: 13, color: t.textHi, fontWeight: 500 }}>Read-only mode</div>
                  <div style={{ fontSize: 11, color: t.textMd, marginTop: 2 }}>Prevent modifications. Useful for archives.</div>
                </div>
                <NCToggle on={readonly} onChange={setReadonly} theme={theme} />
              </div>
            </div>
          </NCCard>

          {testOk === true && (
            <div style={{
              padding: '10px 14px',
              background: `${t.lime}18`, border: `1px solid ${t.lime}50`,
              borderRadius: 3, fontSize: 12, color: t.lime,
              display: 'flex', gap: 8, alignItems: 'center',
            }}>
              <I.shield size={13} color={t.lime} style={{ flexShrink: 0 }} />
              Connection successful — bucket is reachable.
            </div>
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

          <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', gap: 12 }}>
            <NCBtn theme={theme} ghost iconLeft={<I.chevL size={14} />} onClick={onBack}>Back</NCBtn>
            <div style={{ display: 'flex', gap: 8 }}>
              <NCBtn theme={theme} disabled={testing} onClick={handleTest}>
                {testing ? 'Testing…' : testOk === true ? 'Test passed' : 'Test connection'}
              </NCBtn>
              <NCBtn theme={theme} primary icon={<I.arrow size={13} />} disabled={saving} onClick={handleMount}>
                {saving ? 'Adding…' : 'Mount drive'}
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
  return msg;
}
