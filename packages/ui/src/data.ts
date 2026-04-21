import type { Drive, FileEntry, Provider } from './types.js';

export const SAMPLE_DRIVES: Drive[] = [
  { id: 1, name: 'Wasabi · Cortex Backups', letter: 'Z:', provider: 'wasabi', region: 'eu-central-1', bucket: 'nanocrew-cortex-backups', status: 'mounted', used: '248.7 GB', total: '—', readonly: false },
  { id: 2, name: 'Wasabi · Client Deliverables', letter: 'Y:', provider: 'wasabi', region: 'eu-west-1', bucket: 'nc-client-deliverables', status: 'syncing', used: '84.2 GB', total: '—', readonly: false },
  { id: 3, name: 'S3 · Prod Media', letter: 'X:', provider: 's3', region: 'af-south-1', bucket: 'cortex-prod-media', status: 'mounted', used: '1.2 TB', total: '—', readonly: true },
  { id: 4, name: 'Backblaze B2 · Archive', letter: 'W:', provider: 'b2', region: 'us-west-004', bucket: 'nc-archive-frozen', status: 'error', used: '—', total: '—', readonly: false },
];

export const SAMPLE_FILES: FileEntry[] = [
  { name: '2026-Q1-cortex-models', type: 'folder', size: '—', modified: '2d ago', kind: 'folder' },
  { name: 'client-deliverables', type: 'folder', size: '—', modified: '5h ago', kind: 'folder' },
  { name: 'finance', type: 'folder', size: '—', modified: '1w ago', kind: 'folder' },
  { name: 'kickoff-recording.mp4', type: 'video', size: '1.24 GB', modified: '3h ago', kind: 'video' },
  { name: 'cortex-architecture-v0.10.pdf', type: 'doc', size: '8.4 MB', modified: '6h ago', kind: 'doc' },
  { name: 'cortex-logo-sheet.png', type: 'image', size: '2.1 MB', modified: '1d ago', kind: 'image' },
  { name: 'NDA-Cortex-v3-signed.pdf', type: 'doc', size: '640 KB', modified: '2d ago', kind: 'doc' },
  { name: 'model-weights-shard-01.safetensors', type: 'file', size: '4.7 GB', modified: '3d ago', kind: 'file' },
  { name: 'model-weights-shard-02.safetensors', type: 'file', size: '4.7 GB', modified: '3d ago', kind: 'file' },
  { name: 'deployment-runbook.md', type: 'doc', size: '48 KB', modified: '5d ago', kind: 'doc' },
  { name: 'team-all-hands-2026-Q1.mp4', type: 'video', size: '2.8 GB', modified: '1w ago', kind: 'video' },
  { name: 'brand-refresh.fig', type: 'file', size: '184 MB', modified: '1w ago', kind: 'file' },
];

export const PROVIDER_LIST: Provider[] = [
  { id: 'wasabi', name: 'Wasabi', desc: 'Hot cloud storage · S3-compatible', featured: true },
  { id: 's3', name: 'Amazon S3', desc: 'AWS S3 buckets', featured: false },
  { id: 'b2', name: 'Backblaze B2', desc: 'Low-cost cloud object storage', featured: false },
  { id: 'gdrive', name: 'Google Drive', desc: 'Personal & workspace drives', featured: false },
  { id: 'onedrive', name: 'OneDrive', desc: 'Personal & business', featured: false },
  { id: 'dropbox', name: 'Dropbox', desc: 'Personal & team folders', featured: false },
  { id: 'sftp', name: 'SFTP / FTP', desc: 'Secure file transfer protocol', featured: false },
  { id: 'webdav', name: 'WebDAV', desc: 'Generic WebDAV servers', featured: false },
];
