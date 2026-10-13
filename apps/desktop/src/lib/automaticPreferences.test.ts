import { test } from 'node:test';
import assert from 'node:assert/strict';
import { defaults } from './preferences.ts';
import { persistAutomaticPreference, withConfirmedAutomaticUpdates } from './automaticPreferences.ts';

test('an ordinary stale preference save preserves a confirmed opt out and the requested login change', () => {
  const stale = { ...defaults(), autoUpdates: true, launchAtLogin: true };
  const saved = withConfirmedAutomaticUpdates(stale, false);
  assert.equal(saved.autoUpdates, false); assert.equal(saved.launchAtLogin, true);
  assert.equal(stale.autoUpdates, true);
});

test('failed native writes never confirm an automatic update opt out', async () => {
  const before = defaults(); let confirmations = 0;
  await assert.rejects(persistAutomaticPreference(false, () => before, async () => { throw new Error('Disk full'); }, () => { confirmations++; }), /Disk full/);
  assert.equal(confirmations, 0); assert.equal(before.autoUpdates, true);
});
test('a saved opt out is confirmed only after persistence and preserves concurrent display edits', async () => {
  let prefs = defaults(); let release!: () => void;
  const pending = new Promise<void>(resolve => { release = resolve; });
  const write = persistAutomaticPreference(false, () => prefs, async preferences => {
    assert.equal(preferences.autoUpdates, false); await pending;
  }, preferences => { prefs = preferences; });
  assert.equal(prefs.autoUpdates, true);
  prefs = { ...prefs, theme: 'dark', widgetOpacity: 45 };
  release(); await write;
  assert.equal(prefs.autoUpdates, false); assert.equal(prefs.theme, 'dark'); assert.equal(prefs.widgetOpacity, 45);
});
