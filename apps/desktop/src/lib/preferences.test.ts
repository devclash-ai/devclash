import { test } from 'node:test';
import assert from 'node:assert/strict';
import { defaults, enabledInOrder, moveProvider, normalizePreferences, loadPreferences } from './preferences.ts';
test('defaults are independent so one window cannot mutate another by reference', () => {
  const first = defaults(); first.providers.reverse(); first.surfaces.widget.tokens = false;
  assert.equal(defaults().providers[0], 'claude'); assert.equal(defaults().surfaces.widget.tokens, true);
});
test('existing installations gain automatic updates without changing saved display choices', () => {
  const restored = normalizePreferences({ version: 1, theme: 'dark', widgetOpacity: 45, enabledProviders: ['codex'], menuBarView: 'limits' });
  assert.equal(restored.autoUpdates, true);
  assert.equal(restored.theme, 'dark'); assert.equal(restored.widgetOpacity, 45);
  assert.deepEqual(restored.enabledProviders, ['codex']); assert.equal(restored.menuBarView, 'limits');
});
test('automatic updates preserve an explicit opt out and reject malformed saved values', () => {
  const prefs = { ...defaults(), autoUpdates: false };
  assert.equal(normalizePreferences(JSON.parse(JSON.stringify(prefs))).autoUpdates, false);
  assert.equal(normalizePreferences({ ...prefs, autoUpdates: true }).autoUpdates, true);
  for (const invalid of ['false', 'true', 0, 1, null, {}, []]) {
    assert.equal(normalizePreferences({ version: 1, autoUpdates: invalid }).autoUpdates, true);
  }
});
test('old or malformed stored preferences recover to valid defaults', () => {
  assert.deepEqual(normalizePreferences({ version: 0 }), defaults());
  const prefs = normalizePreferences({ version: 1, theme: 'invalid', enabledProviders: ['unknown'], surfaces: { widget: { tokens: 'false', spend: true } } });
  assert.equal(prefs.theme, 'system'); assert.equal(prefs.surfaces.widget.tokens, true); assert.equal(prefs.surfaces.widget.spend, true);
});
test('stored order is unique and missing known providers are restored', () => {
  const prefs = normalizePreferences({ version: 1, providers: ['cursor', 'cursor'], enabledProviders: [] });
  assert.deepEqual(prefs.providers, ['cursor', 'claude', 'codex', 'opencode']); assert.deepEqual(prefs.enabledProviders, []);
});
test('reordering a provider preserves filters and respects its own position', () => {
  const prefs = defaults(); prefs.enabledProviders = ['claude', 'cursor'];
  const next = moveProvider(prefs, 'cursor', 'claude'); assert.deepEqual(enabledInOrder(next), ['cursor', 'claude']);
  assert.equal(moveProvider(next, 'cursor', 'cursor'), next); assert.deepEqual(prefs.providers, ['claude', 'codex', 'cursor', 'opencode']);
});
test('display metrics persist independently for each surface', () => {
  const prefs = defaults(); prefs.surfaces.dashboard.models = false; prefs.surfaces.widget.spend = true;
  const restored = normalizePreferences(JSON.parse(JSON.stringify(prefs)));
  assert.equal(restored.surfaces.dashboard.models, false); assert.equal(restored.surfaces.widget.spend, true); assert.equal(restored.surfaces.menubar.spend, false);
});
test('menu bar upgrades preserve the previous pinned choice, including an intentionally hidden count', () => {
  assert.equal(normalizePreferences({ version: 1, menuBarMetric: 'limits' }).menuBarView, 'limits');
  assert.equal(normalizePreferences({ version: 1, menuBarMetric: 'tokens', surfaces: { menubar: { tokens: false } } }).menuBarView, 'icon');
  assert.equal(normalizePreferences({ version: 1, menuBarMetric: 'limits', surfaces: { menubar: { limits: false } } }).menuBarView, 'icon');
  const migrated = normalizePreferences({ version: 1, menuBarMetric: 'limits', theme: 'dark', widgetSize: 'compact', widgetTransparency: true, widgetOpacity: 95 });
  assert.equal(migrated.theme, 'dark'); assert.equal(migrated.widgetSize, 'compact'); assert.equal(migrated.widgetOpacity, 95);
});
test('menu bar views, selected provider, labels and custom metric order persist together', () => {
  const prefs = defaults(); prefs.menuBarView = 'custom'; prefs.menuBarProvider = 'opencode'; prefs.menuBarLabels = true;
  prefs.menuBarItems = ['spend', 'tokens']; prefs.menuBarDetails = 'models';
  assert.deepEqual(normalizePreferences(JSON.parse(JSON.stringify(prefs))), prefs);
  const first = defaults(); first.menuBarItems.reverse(); assert.deepEqual(defaults().menuBarItems, ['tokens', 'limits']);
});
test('malformed menu bar choices recover safely and custom selections stay unique', () => {
  const recovered = normalizePreferences({ version: 1, menuBarView: 'unknown', menuBarProvider: 'other', menuBarLabels: 'true', menuBarItems: ['bad'], menuBarDetails: 'bad' });
  assert.equal(recovered.menuBarView, 'tokens'); assert.equal(recovered.menuBarProvider, 'all'); assert.equal(recovered.menuBarLabels, false);
  assert.deepEqual(recovered.menuBarItems, ['tokens', 'limits']); assert.equal(recovered.menuBarDetails, 'summary');
  assert.deepEqual(normalizePreferences({ version: 1, menuBarItems: ['spend', 'spend', 'tokens'] }).menuBarItems, ['spend', 'tokens']);
  assert.deepEqual(normalizePreferences({ version: 1, menuBarItems: [] }).menuBarItems, []);
});
test('existing preferences gain opaque widgets without resetting their display choices', () => {
  const restored = normalizePreferences({ version: 1, theme: 'dark', widgetSize: 'compact', providers: ['codex', 'claude', 'cursor'], surfaces: { widget: { models: true } } });
  assert.equal(restored.widgetTransparency, false); assert.equal(restored.widgetOpacity, 80);
  assert.equal(restored.theme, 'dark'); assert.equal(restored.widgetSize, 'compact'); assert.equal(restored.providers[0], 'codex'); assert.equal(restored.surfaces.widget.models, true);
});
test('transparency and its selected opacity survive saved preference round trips', () => {
  const prefs = { ...defaults(), widgetTransparency: true, widgetOpacity: 45 };
  const restored = normalizePreferences(JSON.parse(JSON.stringify(prefs)));
  assert.equal(restored.widgetTransparency, true); assert.equal(restored.widgetOpacity, 45);
  const disabled = normalizePreferences({ ...restored, widgetTransparency: false });
  assert.equal(disabled.widgetOpacity, 45);
});
test('stored opacity is finite, bounded, and compatible with the slider', () => {
  for (const invalid of [NaN, Infinity, -Infinity, '60', null]) {
    assert.equal(normalizePreferences({ version: 1, widgetTransparency: 'true', widgetOpacity: invalid }).widgetOpacity, 80);
  }
  assert.equal(normalizePreferences({ version: 1, widgetTransparency: 'true' }).widgetTransparency, false);
  assert.equal(normalizePreferences({ version: 1, widgetOpacity: -20 }).widgetOpacity, 0);
  assert.equal(normalizePreferences({ version: 1, widgetOpacity: 150 }).widgetOpacity, 100);
  assert.equal(normalizePreferences({ version: 1, widgetOpacity: 42 }).widgetOpacity, 40);
});
test('older installations gain OpenCode once without changing existing provider choices', () => {
  const restored = normalizePreferences({ version: 1, providers: ['codex', 'claude', 'cursor'], enabledProviders: ['codex'],
    widgetTransparency: true, widgetOpacity: 45, theme: 'dark', widgetSize: 'compact' });
  assert.deepEqual(restored.providers, ['codex', 'claude', 'cursor', 'opencode']);
  assert.deepEqual(restored.enabledProviders, ['codex', 'opencode']);
  assert.equal(restored.widgetTransparency, true); assert.equal(restored.widgetOpacity, 45);
  const disabled = normalizePreferences({ ...restored, enabledProviders: ['codex'] });
  assert.deepEqual(disabled.enabledProviders, ['codex']);
  assert.deepEqual(normalizePreferences({ version: 1, providers: ['claude', 'codex', 'cursor'], enabledProviders: [] }).enabledProviders, []);
});

test('product rename migrates old preferences once and preserves newer settings', () => {
  const values = new Map<string, string>();
  const storage = { getItem: (key: string) => values.get(key) ?? null, setItem: (key: string, value: string) => values.set(key, value), removeItem: (key: string) => values.delete(key) };
  const before = Object.getOwnPropertyDescriptor(globalThis, 'localStorage');
  Object.defineProperty(globalThis, 'localStorage', { configurable: true, value: storage });
  try {
    values.set('usagerank.preferences.v1', JSON.stringify({ ...defaults(), theme: 'dark', widgetOpacity: 45, menuBarView: 'tokens-limits' }));
    const migrated = loadPreferences(); assert.equal(migrated.theme, 'dark'); assert.equal(migrated.widgetOpacity, 45);
    assert.ok(values.has('devclash.preferences.v1')); assert.equal(values.has('usagerank.preferences.v1'), false);
    values.set('usagerank.preferences.v1', JSON.stringify({ ...defaults(), theme: 'light' }));
    assert.equal(loadPreferences().theme, 'dark');
  } finally { if (before) Object.defineProperty(globalThis, 'localStorage', before); else Reflect.deleteProperty(globalThis, 'localStorage'); }
});
