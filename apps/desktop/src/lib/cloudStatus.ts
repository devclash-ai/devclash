import type { CloudStatus } from './cloud';

/** Consent and upload history are separate: no completed upload does not mean disabled. */
export function cloudSyncLabel(status: Pick<CloudStatus, 'syncEnabled' | 'syncError' | 'lastSyncedAt' | 'pendingCount'>): string {
  if (!status.syncEnabled) return 'Cloud Sync Is Off';
  if (status.syncError) return 'Sync Needs Attention';
  if (status.pendingCount > 0) return `Cloud Sync On · ${status.pendingCount.toLocaleString()} Pending`;
  if (!status.lastSyncedAt) return 'Waiting for First Sync';
  return 'Cloud Sync Is Active';
}

/** Off is neutral; pending and failed uploads must not look successfully synced. */
export function cloudSyncTone(status: Pick<CloudStatus, 'syncEnabled' | 'syncError' | 'lastSyncedAt' | 'pendingCount'>): 'off' | 'pending' | 'error' | 'active' {
  if (!status.syncEnabled) return 'off';
  if (status.syncError) return 'error';
  if (status.pendingCount > 0 || !status.lastSyncedAt) return 'pending';
  return 'active';
}
