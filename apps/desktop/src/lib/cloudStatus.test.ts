import { test } from 'node:test';
import assert from 'node:assert/strict';
import { cloudSyncLabel, cloudSyncTone } from './cloudStatus.ts';

test('enabled sync without a successful upload is waiting, not off', () => {
  assert.equal(cloudSyncLabel({pendingCount: 0, syncEnabled: true, lastSyncedAt: null, syncError: null}), 'Waiting for First Sync');
});
test('enabled sync reports failures and completed uploads distinctly', () => {
  assert.equal(cloudSyncLabel({pendingCount: 0, syncEnabled: true, lastSyncedAt: null, syncError: 'Offline'}), 'Sync Needs Attention');
  assert.equal(cloudSyncLabel({pendingCount: 0, syncEnabled: true, lastSyncedAt: '2026-10-08T00:00:00Z', syncError: null}), 'Cloud Sync Is Active');
});
test('disabled consent stays off regardless of old upload history or error', () => {
  assert.equal(cloudSyncLabel({pendingCount: 0, syncEnabled: false, lastSyncedAt: '2026-10-08T00:00:00Z', syncError: 'Offline'}), 'Cloud Sync Is Off');
});

test('initial and later backlogs show enabled sync and pending count', () => {
  for (const lastSyncedAt of [null, '2026-10-08T00:00:00Z']) {
    assert.equal(cloudSyncLabel({syncEnabled: true, lastSyncedAt, syncError: null, pendingCount: 365017}), `Cloud Sync On · ${(365017).toLocaleString()} Pending`);
  }
});
test('backlog does not hide paused sync or errors', () => {
  assert.equal(cloudSyncLabel({syncEnabled: false, lastSyncedAt: null, syncError: null, pendingCount: 900}), 'Cloud Sync Is Off');
  assert.equal(cloudSyncLabel({syncEnabled: true, lastSyncedAt: null, syncError: 'Offline', pendingCount: 900}), 'Sync Needs Attention');
});

test('off and incomplete uploads never show the successful green state', () => {
  const base = {syncEnabled: true, syncError: null, pendingCount: 0, lastSyncedAt: '2026-10-09T10:00:00Z'};
  assert.equal(cloudSyncTone(base), 'active');
  assert.equal(cloudSyncTone({...base, syncEnabled: false}), 'off');
  assert.equal(cloudSyncTone({...base, syncEnabled: false, syncError: 'Offline', pendingCount: 5}), 'off');
  assert.equal(cloudSyncTone({...base, lastSyncedAt: null}), 'pending');
  assert.equal(cloudSyncTone({...base, pendingCount: 5}), 'pending');
  assert.equal(cloudSyncTone({...base, syncError: 'Offline'}), 'error');
});
