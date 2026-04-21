import type { DriveStatus } from './primitives/index.js';

export interface Drive {
  id: number;
  name: string;
  letter: string;
  provider: string;
  region: string;
  bucket: string;
  status: DriveStatus;
  used: string;
  total: string;
  readonly: boolean;
}

export interface FileEntry {
  name: string;
  type: string;
  size: string;
  modified: string;
  kind: 'folder' | 'image' | 'video' | 'doc' | 'file';
}

export interface Provider {
  id: string;
  name: string;
  desc: string;
  featured: boolean;
}

export type NavKey =
  | 'home'
  | 'drives'
  | 'files'
  | 'transfers'
  | 'activity'
  | 'account'
  | 'settings';
