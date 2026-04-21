import React from 'react';
import { invoke } from '@tauri-apps/api/core';
import {
  getTokens, NC_FONT_MONO,
  NCCard, NCEyebrow, NCLabel, NCBtn, NCInput, NCToggle,
  TopBar,
  type Theme,
} from '@nanocrew/ui';
import { I } from '@nanocrew/ui';
import { useAuth } from '../context/auth.js';

interface AddDriveWasabiScreenProps {
  theme: Theme;
  onBack: () => void;
  onCancel: () => void;
  onDone: () => void;
}

const WASABI_ENDPOINTS: { label: string; endpoint: string; region: string }[] = [
  { label: 'US East 1 (Ashburn, VA)',      endpoint: 's3.us-east-1.wasabisys.com',      region: 'us-east-1' },
  { label: 'US East 2 (Manassas, VA)',     endpoint: 's3.us-east-2.wasabisys.com',      region: 'us-east-2' },
  { label: 'US West 1 (Hillsboro, OR)',    endpoint: 's3.us-west-1.wasabisys.com',      region: 'us-west-1' },
  { label: 'EU Central 1 (Amsterdam)',     endpoint: 's3.eu-central-1.wasabisys.com',   region: 'eu-central-1' },
  { label: 'EU Central 2 (Frankfurt)',     endpoint: 's3.eu-central-2.wasabisys.com',   region: 'eu-central-2' },
  { label: 'EU West 1 (London)',           endpoint: 's3.eu-west-1.wasabisys.com',      region: 'eu-west-1' },
  { label: 'EU West 2 (Paris)',            endpoint: 's3.eu-west-2.wasabisys.com',      region: 'eu-west-2' },
  { label: 'AP Northeast 1 (Tokyo)',       endpoint: 's3.ap-northeast-1.wasabisys.com', region: 'ap-northeast-1' },
  { label: 'AP Northeast 2 (Osaka)',       endpoint: 's3.ap-northeast-2.wasabisys.com', region: 'ap-northeast-2' },
  { label: 'AP Southeast 1 (Singapore)',   endpoint: 's3.ap-southeast-1.wasabisys.com', region: 'ap-southeast-1' },
  { label: 'AP Southeast 2 (Sydney)',      endpoint: 's3.ap-southeast-2.wasabisys.com', region: 'ap-southeast-2' },
  { label: 'CA Central 1 (Toronto)',       endpoint: 's3.ca-central-1.wasabisys.com',   region: 'ca-central-1' },
];

const Field: React.FC<{ label: string; children: React.ReactNode; theme: Theme; last?: boolean }> = ({
  label, children, theme, last,
}) => (
  <div style={{ marginBottom: last ? 0 : 14 }}>
    <NCLabel theme={theme}>{label}</NCLabel>
    {children}
  </div>
);

export const AddDriveWasabiScreen: React.FC<AddDriveWasabiScreenProps> = ({
  theme, onBack, onCancel, onDone,
}) => {
  const t = getTokens(theme);
  const { token } = useAuth();

  const [name, setName] = React.useState('');
  const [endpointIdx, setEndpointIdx] = React.useState(0);
  const [bucket, setBucket] = React.useState('');
  const [accessKeyId, setAccessKeyId] = React.useState('');
  const [secretKey, setSecretKey] = React.useState('');
  const [showSecret, setShowSecret] = React.useState(false);
  const [letter, setLetter] = React.useState('');
  const [availableLetters, setAvailableLetters] = React.useState<string[]>([]);
  const [cacheSizeGb, setCacheSizeGb] = React.useState(5);
  const [autoMount, setAutoMount] = React.useState(true);
  const [readonly, setReadonly] = React.useState(false);

  const [testing, setTesting] = React.useState(false);
  const [testOk, setTestOk] = React.useState<boolean | null>(null);
  const [saving, setSaving] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const [browsing, setBrowsing] = React.useState(false);
  const [availableBuckets, setAvailableBuckets] = React.useState<string[] | null>(null);

  const ep = WASABI_ENDPOINTS[endpointIdx]!;

  React.useEffect(() => {
    invoke<string[]>('get_available_letters', { token })
      .then(letters => {
        setAvailableLetters(letters);
        if (letters.length > 0) setLetter(letters[0]!);
      })
      .catch(() => {});
  }, [token]);

  const handleTest = async () => {
    setError(null);
    setTestOk(null);
    if (!bucket.trim() || !accessKeyId.trim() || !secretKey.trim()) {
      setError('Bucket, access key, and secret are required to test.');
      return;
    }
    setTesting(true);
    try {
      await invoke('test_connection', {
        token,
        input: {
          provider: 'wasabi',
          endpoint: ep.endpoint,
          bucket: bucket.trim(),
          region: ep.region,
          access_key_id: accessKeyId.trim(),
          secret_access_key: secretKey,
        },
      });
      setTestOk(true);
    } catch (e) {
      setTestOk(false);
      setError(String(e));
    } finally {
      setTesting(false);
    }
  };

  const handleBrowse = async () => {
    setError(null);
    if (!accessKeyId.trim() || !secretKey.trim()) {
      setError('Enter your access key ID and secret key first.');
      return;
    }
    setBrowsing(true);
    try {
      const buckets = await invoke<string[]>('list_buckets', {
        token,
        endpoint: ep.endpoint,
        region: ep.region,
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
    if (!bucket.trim()) { setError('Bucket is required.'); return; }
    if (!accessKeyId.trim() || !secretKey.trim()) { setError('Access key ID and secret are required.'); return; }
    if (!letter) { setError('Select a drive letter.'); return; }

    setSaving(true);
    try {
      await invoke('add_drive', {
        token,
        input: {
          name: name.trim(),
          provider: 'wasabi',
          endpoint: ep.endpoint,
          bucket: bucket.trim(),
          region: ep.region,
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

  return (
    <>
      <TopBar
        theme={theme}
        crumbs={['Drives', 'Add drive', 'Wasabi']}
        title={<>Connect <span style={{ color: t.lime }}>Wasabi</span></>}
        subtitle="Mount a Wasabi bucket as a Windows drive. Credentials are stored in the system credential vault, never in plain text."
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
            <NCEyebrow theme={theme} style={{ marginBottom: 16 }}>Connection</NCEyebrow>
            <Field theme={theme} label="Display name">
              <NCInput theme={theme} value={name} onChange={setName} placeholder="e.g. Cortex Backups" />
            </Field>
            <Field theme={theme} label="Region endpoint">
              <div style={{ position: 'relative' }}>
                <div style={{ position: 'absolute', right: 12, top: '50%', transform: 'translateY(-50%)', pointerEvents: 'none' }}>
                  <I.chevD size={13} color={t.textMd} />
                </div>
                <select
                  value={endpointIdx}
                  onChange={e => setEndpointIdx(Number(e.target.value))}
                  style={selectStyle}
                >
                  {WASABI_ENDPOINTS.map((w, i) => (
                    <option key={w.endpoint} value={i}>{w.label}</option>
                  ))}
                </select>
              </div>
            </Field>
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
              <NCInput theme={theme} mono value={accessKeyId} onChange={setAccessKeyId} placeholder="IXXXXXXXXXXXXXXXXXXX" prefix={<I.lock size={13} />} />
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
