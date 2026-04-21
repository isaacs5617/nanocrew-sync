// S3-compatible provider presets. Every entry feeds the generic
// AddDriveS3Screen — user picks a provider, gets the right endpoint/region
// dropdown (or a free-form endpoint field for self-hosted providers), and
// the app signs requests with the provider's expected path-style.
//
// Endpoints never include the `https://` scheme — the backend prepends it.
// Regions use the canonical S3 region string each provider publishes. Where
// a provider accepts any region (MinIO, Storj, etc.) we ship a default.

export interface S3Region {
  /** Human label, e.g. "US East 1 (Ashburn, VA)". */
  label: string;
  /** Hostname only — no scheme, no trailing slash. */
  endpoint: string;
  /** AWS-style region string. */
  region: string;
}

export interface S3ProviderPreset {
  id: string;
  name: string;
  /** One-line marketing description used on the picker card. */
  desc: string;
  /** Badges shown next to the provider on the picker card. */
  badges: { label: string; color: 'lime' | 'muted' }[];
  /** True when the provider requires the user to type their own endpoint
   *  (self-hosted MinIO, custom S3 gateways). When set, `regions` is ignored. */
  customEndpoint?: boolean;
  /** Region hardcoded by the provider regardless of endpoint. */
  fixedRegion?: string;
  /** Canonical region list. Omit for `customEndpoint`. */
  regions?: S3Region[];
  /** Placeholder shown in the access-key-ID field. */
  keyIdHint?: string;
  /** Support/docs URL shown inline in the form. */
  docsUrl?: string;
  /** Most providers accept path-style signing. AWS S3 prefers virtual-hosted
   *  but still accepts path-style for older regions — we default to path-style
   *  which works everywhere. */
  forcePathStyle?: boolean;
}

// ── Presets ──────────────────────────────────────────────────────────────────

export const S3_PROVIDER_PRESETS: Record<string, S3ProviderPreset> = {
  wasabi: {
    id: 'wasabi',
    name: 'Wasabi',
    desc: 'Hot cloud storage · no egress fees',
    badges: [
      { label: 'S3-COMPATIBLE', color: 'lime' },
      { label: 'NO EGRESS', color: 'lime' },
      { label: '13 REGIONS', color: 'muted' },
    ],
    keyIdHint: 'IXXXXXXXXXXXXXXXXXXX',
    docsUrl: 'https://docs.wasabi.com/',
    forcePathStyle: true,
    regions: [
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
    ],
  },

  s3: {
    id: 's3',
    name: 'Amazon S3',
    desc: 'AWS S3 buckets — every region, every storage class',
    badges: [
      { label: 'S3-NATIVE', color: 'lime' },
      { label: 'ALL REGIONS', color: 'muted' },
    ],
    keyIdHint: 'AKIAXXXXXXXXXXXXXXXX',
    docsUrl: 'https://docs.aws.amazon.com/s3/',
    regions: [
      { label: 'US East 1 (N. Virginia)',      endpoint: 's3.us-east-1.amazonaws.com',      region: 'us-east-1' },
      { label: 'US East 2 (Ohio)',             endpoint: 's3.us-east-2.amazonaws.com',      region: 'us-east-2' },
      { label: 'US West 1 (N. California)',    endpoint: 's3.us-west-1.amazonaws.com',      region: 'us-west-1' },
      { label: 'US West 2 (Oregon)',           endpoint: 's3.us-west-2.amazonaws.com',      region: 'us-west-2' },
      { label: 'CA Central 1 (Canada)',        endpoint: 's3.ca-central-1.amazonaws.com',   region: 'ca-central-1' },
      { label: 'EU West 1 (Ireland)',          endpoint: 's3.eu-west-1.amazonaws.com',      region: 'eu-west-1' },
      { label: 'EU West 2 (London)',           endpoint: 's3.eu-west-2.amazonaws.com',      region: 'eu-west-2' },
      { label: 'EU West 3 (Paris)',            endpoint: 's3.eu-west-3.amazonaws.com',      region: 'eu-west-3' },
      { label: 'EU Central 1 (Frankfurt)',     endpoint: 's3.eu-central-1.amazonaws.com',   region: 'eu-central-1' },
      { label: 'EU North 1 (Stockholm)',       endpoint: 's3.eu-north-1.amazonaws.com',     region: 'eu-north-1' },
      { label: 'EU South 1 (Milan)',           endpoint: 's3.eu-south-1.amazonaws.com',     region: 'eu-south-1' },
      { label: 'AP South 1 (Mumbai)',          endpoint: 's3.ap-south-1.amazonaws.com',     region: 'ap-south-1' },
      { label: 'AP Northeast 1 (Tokyo)',       endpoint: 's3.ap-northeast-1.amazonaws.com', region: 'ap-northeast-1' },
      { label: 'AP Northeast 2 (Seoul)',       endpoint: 's3.ap-northeast-2.amazonaws.com', region: 'ap-northeast-2' },
      { label: 'AP Northeast 3 (Osaka)',       endpoint: 's3.ap-northeast-3.amazonaws.com', region: 'ap-northeast-3' },
      { label: 'AP Southeast 1 (Singapore)',   endpoint: 's3.ap-southeast-1.amazonaws.com', region: 'ap-southeast-1' },
      { label: 'AP Southeast 2 (Sydney)',      endpoint: 's3.ap-southeast-2.amazonaws.com', region: 'ap-southeast-2' },
      { label: 'AP Southeast 3 (Jakarta)',     endpoint: 's3.ap-southeast-3.amazonaws.com', region: 'ap-southeast-3' },
      { label: 'AP East 1 (Hong Kong)',        endpoint: 's3.ap-east-1.amazonaws.com',      region: 'ap-east-1' },
      { label: 'AF South 1 (Cape Town)',       endpoint: 's3.af-south-1.amazonaws.com',     region: 'af-south-1' },
      { label: 'ME South 1 (Bahrain)',         endpoint: 's3.me-south-1.amazonaws.com',     region: 'me-south-1' },
      { label: 'SA East 1 (São Paulo)',        endpoint: 's3.sa-east-1.amazonaws.com',      region: 'sa-east-1' },
    ],
  },

  b2: {
    id: 'b2',
    name: 'Backblaze B2',
    desc: 'Low-cost cloud object storage · free Cloudflare egress',
    badges: [
      { label: 'S3-COMPATIBLE', color: 'lime' },
      { label: 'LOW COST', color: 'lime' },
    ],
    keyIdHint: '004xxxxxxxxxxxxxxxxxxxxxx',
    docsUrl: 'https://www.backblaze.com/docs/cloud-storage-s3-compatible-api',
    forcePathStyle: true,
    regions: [
      { label: 'US West 000 (Sacramento)',   endpoint: 's3.us-west-000.backblazeb2.com',   region: 'us-west-000' },
      { label: 'US West 001 (Phoenix)',      endpoint: 's3.us-west-001.backblazeb2.com',   region: 'us-west-001' },
      { label: 'US West 002 (Phoenix)',      endpoint: 's3.us-west-002.backblazeb2.com',   region: 'us-west-002' },
      { label: 'US West 004 (Sacramento)',   endpoint: 's3.us-west-004.backblazeb2.com',   region: 'us-west-004' },
      { label: 'US East 005 (Reston, VA)',   endpoint: 's3.us-east-005.backblazeb2.com',   region: 'us-east-005' },
      { label: 'EU Central 003 (Amsterdam)', endpoint: 's3.eu-central-003.backblazeb2.com', region: 'eu-central-003' },
    ],
  },

  r2: {
    id: 'r2',
    name: 'Cloudflare R2',
    desc: 'Zero-egress S3-compatible object storage',
    badges: [
      { label: 'S3-COMPATIBLE', color: 'lime' },
      { label: 'ZERO EGRESS', color: 'lime' },
    ],
    keyIdHint: 'Your R2 Access Key ID',
    docsUrl: 'https://developers.cloudflare.com/r2/api/s3/api/',
    forcePathStyle: true,
    // R2 uses the account-scoped endpoint; user supplies their account ID as
    // part of the endpoint. We expose this via a "customEndpoint" flow but
    // show the template as a tip.
    customEndpoint: true,
    fixedRegion: 'auto',
  },

  minio: {
    id: 'minio',
    name: 'MinIO',
    desc: 'Self-hosted S3-compatible server',
    badges: [
      { label: 'S3-COMPATIBLE', color: 'lime' },
      { label: 'SELF-HOSTED', color: 'muted' },
    ],
    keyIdHint: 'minioadmin',
    docsUrl: 'https://min.io/docs/minio/linux/index.html',
    forcePathStyle: true,
    customEndpoint: true,
    fixedRegion: 'us-east-1',
  },

  idrive: {
    id: 'idrive',
    name: 'IDrive e2',
    desc: 'S3-compatible cloud storage · no egress fees',
    badges: [
      { label: 'S3-COMPATIBLE', color: 'lime' },
      { label: 'NO EGRESS', color: 'lime' },
    ],
    keyIdHint: 'Your IDrive e2 Access Key',
    docsUrl: 'https://www.idrive.com/e2/developers',
    forcePathStyle: true,
    regions: [
      { label: 'Los Angeles',         endpoint: 'v2j5.la.idrivee2-39.com',  region: 'LA' },
      { label: 'Dallas',              endpoint: 'v1j2.dal.idrivee2-42.com', region: 'DAL' },
      { label: 'Chicago',             endpoint: 'b2z8.chi.idrivee2-51.com', region: 'CHI' },
      { label: 'Phoenix',             endpoint: 'i5a7.phx.idrivee2-66.com', region: 'PHX' },
      { label: 'Washington, DC',      endpoint: 'j1x0.va.idrivee2-22.com',  region: 'VA' },
      { label: 'Ireland',             endpoint: 'z9r2.ie.idrivee2-52.com',  region: 'IE' },
      { label: 'Frankfurt',           endpoint: 'a5b7.fra.idrivee2-29.com', region: 'FRA' },
      { label: 'Singapore',           endpoint: 'x8v9.sg.idrivee2-47.com',  region: 'SG' },
    ],
  },

  digitalocean: {
    id: 'digitalocean',
    name: 'DigitalOcean Spaces',
    desc: 'Simple object storage with built-in CDN',
    badges: [
      { label: 'S3-COMPATIBLE', color: 'lime' },
      { label: 'CDN', color: 'muted' },
    ],
    keyIdHint: 'DO00XXXXXXXXXXXXXXXXX',
    docsUrl: 'https://docs.digitalocean.com/products/spaces/',
    regions: [
      { label: 'New York 3 (NYC3)',     endpoint: 'nyc3.digitaloceanspaces.com', region: 'nyc3' },
      { label: 'San Francisco 2 (SFO2)', endpoint: 'sfo2.digitaloceanspaces.com', region: 'sfo2' },
      { label: 'San Francisco 3 (SFO3)', endpoint: 'sfo3.digitaloceanspaces.com', region: 'sfo3' },
      { label: 'Amsterdam 3 (AMS3)',    endpoint: 'ams3.digitaloceanspaces.com', region: 'ams3' },
      { label: 'Singapore 1 (SGP1)',    endpoint: 'sgp1.digitaloceanspaces.com', region: 'sgp1' },
      { label: 'Frankfurt 1 (FRA1)',    endpoint: 'fra1.digitaloceanspaces.com', region: 'fra1' },
      { label: 'Sydney 1 (SYD1)',       endpoint: 'syd1.digitaloceanspaces.com', region: 'syd1' },
      { label: 'Bangalore 1 (BLR1)',    endpoint: 'blr1.digitaloceanspaces.com', region: 'blr1' },
    ],
  },

  storj: {
    id: 'storj',
    name: 'Storj',
    desc: 'Decentralized cloud object storage',
    badges: [
      { label: 'S3-COMPATIBLE', color: 'lime' },
      { label: 'DECENTRALIZED', color: 'muted' },
    ],
    keyIdHint: 'Your Storj Access Key',
    docsUrl: 'https://docs.storj.io/dcs/api/s3',
    forcePathStyle: true,
    fixedRegion: 'global',
    regions: [
      { label: 'Global (gateway.storjshare.io)', endpoint: 'gateway.storjshare.io', region: 'global' },
    ],
  },

  scaleway: {
    id: 'scaleway',
    name: 'Scaleway',
    desc: 'European cloud object storage',
    badges: [
      { label: 'S3-COMPATIBLE', color: 'lime' },
      { label: 'EU', color: 'muted' },
    ],
    keyIdHint: 'SCWXXXXXXXXXXXXXXXXX',
    docsUrl: 'https://www.scaleway.com/en/docs/object-storage/',
    forcePathStyle: true,
    regions: [
      { label: 'Paris (fr-par)',       endpoint: 's3.fr-par.scw.cloud',  region: 'fr-par' },
      { label: 'Amsterdam (nl-ams)',   endpoint: 's3.nl-ams.scw.cloud',  region: 'nl-ams' },
      { label: 'Warsaw (pl-waw)',      endpoint: 's3.pl-waw.scw.cloud',  region: 'pl-waw' },
    ],
  },

  contabo: {
    id: 'contabo',
    name: 'Contabo',
    desc: 'Budget S3-compatible object storage',
    badges: [
      { label: 'S3-COMPATIBLE', color: 'lime' },
      { label: 'LOW COST', color: 'muted' },
    ],
    keyIdHint: 'Your Contabo Access Key',
    docsUrl: 'https://contabo.com/en/object-storage/',
    forcePathStyle: true,
    regions: [
      { label: 'Germany (EU)',         endpoint: 'eu2.contabostorage.com',      region: 'default' },
      { label: 'USA (Central)',        endpoint: 'usc1.contabostorage.com',     region: 'default' },
      { label: 'Singapore (Asia)',     endpoint: 'sin1.contabostorage.com',     region: 'default' },
    ],
  },

  oracle: {
    id: 'oracle',
    name: 'Oracle Object Storage',
    desc: 'Oracle Cloud object storage with S3 API',
    badges: [
      { label: 'S3-COMPATIBLE', color: 'lime' },
      { label: 'ENTERPRISE', color: 'muted' },
    ],
    keyIdHint: 'Your OCI Access Key',
    docsUrl: 'https://docs.oracle.com/en-us/iaas/Content/Object/Tasks/s3compatibleapi.htm',
    forcePathStyle: true,
    customEndpoint: true,
    fixedRegion: 'us-ashburn-1',
  },

  linode: {
    id: 'linode',
    name: 'Linode Object Storage',
    desc: 'Akamai-backed S3-compatible storage',
    badges: [
      { label: 'S3-COMPATIBLE', color: 'lime' },
      { label: 'AKAMAI', color: 'muted' },
    ],
    keyIdHint: 'Your Linode Access Key',
    docsUrl: 'https://www.linode.com/docs/products/storage/object-storage/',
    forcePathStyle: true,
    regions: [
      { label: 'Newark, NJ (us-east-1)',     endpoint: 'us-east-1.linodeobjects.com', region: 'us-east-1' },
      { label: 'Atlanta, GA (us-southeast)', endpoint: 'us-southeast-1.linodeobjects.com', region: 'us-southeast-1' },
      { label: 'Fremont, CA (us-west)',      endpoint: 'us-west-1.linodeobjects.com', region: 'us-west-1' },
      { label: 'Frankfurt (eu-central)',     endpoint: 'eu-central-1.linodeobjects.com', region: 'eu-central-1' },
      { label: 'Amsterdam (nl-ams)',         endpoint: 'nl-ams-1.linodeobjects.com', region: 'nl-ams-1' },
      { label: 'Paris (fr-par)',             endpoint: 'fr-par-1.linodeobjects.com', region: 'fr-par-1' },
      { label: 'Milan (it-mil)',             endpoint: 'it-mil-1.linodeobjects.com', region: 'it-mil-1' },
      { label: 'Singapore (ap-south)',       endpoint: 'ap-south-1.linodeobjects.com', region: 'ap-south-1' },
      { label: 'Sydney (ap-southeast)',      endpoint: 'ap-southeast-1.linodeobjects.com', region: 'ap-southeast-1' },
    ],
  },

  vultr: {
    id: 'vultr',
    name: 'Vultr Object Storage',
    desc: 'S3-compatible storage with global regions',
    badges: [
      { label: 'S3-COMPATIBLE', color: 'lime' },
      { label: 'GLOBAL', color: 'muted' },
    ],
    keyIdHint: 'Your Vultr Access Key',
    docsUrl: 'https://www.vultr.com/docs/vultr-object-storage/',
    forcePathStyle: true,
    regions: [
      { label: 'New Jersey (ewr1)',   endpoint: 'ewr1.vultrobjects.com', region: 'ewr1' },
      { label: 'Amsterdam (ams1)',    endpoint: 'ams1.vultrobjects.com', region: 'ams1' },
      { label: 'Silicon Valley (sjc1)', endpoint: 'sjc1.vultrobjects.com', region: 'sjc1' },
      { label: 'Singapore (sgp1)',    endpoint: 'sgp1.vultrobjects.com', region: 'sgp1' },
      { label: 'Bangalore (blr1)',    endpoint: 'blr1.vultrobjects.com', region: 'blr1' },
      { label: 'Tokyo (nrt1)',        endpoint: 'nrt1.vultrobjects.com', region: 'nrt1' },
      { label: 'Paris (cdg1)',        endpoint: 'cdg1.vultrobjects.com', region: 'cdg1' },
    ],
  },
};

/** Ordered list the picker iterates. Wasabi stays at the top as the
 *  "recommended" default. AWS S3 second for prestige. Alphabetical thereafter. */
export const S3_PROVIDER_ORDER: string[] = [
  'wasabi',
  's3',
  'b2',
  'r2',
  'digitalocean',
  'idrive',
  'linode',
  'minio',
  'oracle',
  'scaleway',
  'storj',
  'contabo',
  'vultr',
];
