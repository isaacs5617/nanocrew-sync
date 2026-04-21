// Native Windows toast bridge.
//
// Listens to backend events (drive_status_changed, transfer_progress, and
// mount_failed activity rows) and emits Windows 10/11 action-centre toasts
// via tauri-plugin-notification. Runs once at the AppShell level — hooking
// it in twice would fire duplicate notifications.
//
// Gated by three prefs so users can silence categories independently:
//   - notify_mount_events  (default: true) — mount / unmount success
//   - notify_errors        (default: true) — mount / upload failures
//   - notify_uploads       (default: false) — noisy, opt-in only
//
// All prefs flow through the existing `get_pref` / `set_pref` commands so
// the Settings screen can render them with the existing PrefToggle.

import React from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import {
  isPermissionGranted,
  requestPermission,
  sendNotification,
} from '@tauri-apps/plugin-notification';

/** Subset of the `DriveStatusPayload` emitted by the Rust backend. */
interface DriveStatusPayload {
  drive_id: number;
  status: string;
  message?: string | null;
}

/** Subset of `TransferPayload` — we only care about upload completion. */
interface TransferPayload {
  id: string;
  drive_id: number;
  filename: string;
  direction: 'upload' | 'download';
  total_bytes: number;
  done_bytes: number;
  state: 'progress' | 'done' | 'error';
  error?: string | null;
}

/** Subset of `ActivityEntry` — we watch mount_failed rows. */
interface ActivityEntry {
  id: number;
  ts: number;
  kind: string;
  action: string;
  severity: string;
  drive_id?: number | null;
  target?: string | null;
  message?: string | null;
}

interface DriveMeta { id: number; name: string; letter: string }

/** Cached drive-id → friendly label map. Refreshed on drive_status_changed
 *  rather than per-event so we don't spam `list_drives`. */
function useDriveLabels(token: string) {
  const [map, setMap] = React.useState<Map<number, DriveMeta>>(new Map());

  const refresh = React.useCallback(() => {
    invoke<DriveMeta[]>('list_drives', { token })
      .then(list => setMap(new Map(list.map(d => [d.id, d]))))
      .catch(() => {/* offline during sign-out */});
  }, [token]);

  React.useEffect(() => { refresh(); }, [refresh]);

  // Refresh when a drive is added/removed — those paths emit status changes
  // on the next mount attempt. Not perfect, but avoids polling.
  React.useEffect(() => {
    const un = listen<DriveStatusPayload>('drive_status_changed', () => {
      // Debounce to batch back-to-back events on app-launch auto-mount.
      if ((un as any)._t) clearTimeout((un as any)._t);
      (un as any)._t = setTimeout(refresh, 500);
    });
    return () => { un.then(fn => fn()); };
  }, [refresh]);

  return map;
}

/** Load a boolean pref with a default. Mirrors the Rust-side semantics. */
async function getBoolPref(token: string, key: string, fallback: boolean): Promise<boolean> {
  try {
    const v = await invoke<string | null>('get_pref', { token, key });
    if (v == null) return fallback;
    return v === '1' || v === 'true';
  } catch { return fallback; }
}

/** Resolve OS-level permission once, remembering the answer for the session. */
let permissionCache: boolean | null = null;
async function ensurePermission(): Promise<boolean> {
  if (permissionCache !== null) return permissionCache;
  try {
    const granted = await isPermissionGranted();
    if (granted) { permissionCache = true; return true; }
    const res = await requestPermission();
    permissionCache = res === 'granted';
    return permissionCache;
  } catch {
    permissionCache = false;
    return false;
  }
}

/** Fire-and-forget toast. Swallows errors — a failed toast shouldn't bring
 *  down the UI, and the activity log already records the real event. */
async function toast(title: string, body: string) {
  if (!(await ensurePermission())) return;
  try { sendNotification({ title, body }); } catch {/* ignore */}
}

export function useDriveNotifications(token: string | null) {
  // Without a signed-in token we can't read prefs or drive names — bail out.
  // The outer component will re-mount the hook when token becomes non-null.
  const labels = useDriveLabels(token ?? '');
  const prevStatus = React.useRef<Map<number, string>>(new Map());

  // Small helper: get the human-visible label for a drive id. Falls back to
  // the raw id if we don't have it cached yet (first event after launch).
  const labelFor = React.useCallback((id: number) => {
    const m = labels.get(id);
    if (!m) return `drive #${id}`;
    return m.letter ? `${m.name} (${m.letter})` : m.name;
  }, [labels]);

  // drive_status_changed — mount success, unmount, error
  React.useEffect(() => {
    if (!token) return;
    const un = listen<DriveStatusPayload>('drive_status_changed', async e => {
      const { drive_id, status, message } = e.payload;
      const prev = prevStatus.current.get(drive_id);
      prevStatus.current.set(drive_id, status);

      // Don't fire on the initial transient status we see for a drive — only
      // on real transitions. Without this, every load emits a "mounted" toast.
      if (prev === undefined) return;
      if (prev === status) return;

      if (status === 'mounted') {
        if (await getBoolPref(token, 'notify_mount_events', true)) {
          toast('Drive mounted', `${labelFor(drive_id)} is ready.`);
        }
      } else if (status === 'offline' && prev === 'mounted') {
        if (await getBoolPref(token, 'notify_mount_events', true)) {
          toast('Drive unmounted', `${labelFor(drive_id)} has been unmounted.`);
        }
      } else if (status === 'error') {
        if (await getBoolPref(token, 'notify_errors', true)) {
          toast('Mount failed', message ? `${labelFor(drive_id)}: ${message}` : `${labelFor(drive_id)} could not be mounted.`);
        }
      }
    });
    return () => { un.then(fn => fn()); prevStatus.current.clear(); };
  }, [token, labelFor]);

  // transfer_progress — upload complete (opt-in, off by default — noisy)
  React.useEffect(() => {
    if (!token) return;
    const un = listen<TransferPayload>('transfer_progress', async e => {
      const { state, direction, filename, error } = e.payload;
      if (direction !== 'upload') return;
      if (state === 'done') {
        if (await getBoolPref(token, 'notify_uploads', false)) {
          toast('Upload complete', filename);
        }
      } else if (state === 'error') {
        if (await getBoolPref(token, 'notify_errors', true)) {
          toast('Upload failed', error ? `${filename}: ${error}` : filename);
        }
      }
    });
    return () => { un.then(fn => fn()); };
  }, [token]);
}
