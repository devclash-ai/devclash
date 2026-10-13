import { test } from 'node:test';
import assert from 'node:assert/strict';
import { defaults } from './preferences.ts';
import { trayContent } from './tray.ts';
import { aggregateUsage, demoSnapshot, emptySnapshot } from './usage.ts';
test('tray limits only consider enabled providers', () => {
  const prefs = defaults(); prefs.menuBarView = 'limits'; prefs.enabledProviders = ['codex'];
  const snapshot = demoSnapshot(); const totals = aggregateUsage(snapshot, 'today', prefs.enabledProviders);
  const content = trayContent(prefs, snapshot, totals); assert.equal(content.label, '38%'); assert.ok(!content.rows.some(row => row.startsWith('Cursor')));
});
test('dropdown metrics can be hidden without removing the independently pinned count', () => {
  const prefs = defaults(); prefs.surfaces.menubar.tokens = false;
  const snapshot = demoSnapshot(); const content = trayContent(prefs, snapshot, aggregateUsage(snapshot, 'today', prefs.enabledProviders));
  assert.equal(content.label, '2.84M'); assert.ok(!content.rows.some(row => row.startsWith('Today Tokens')));
});
test('each menu bar preset produces its selected information with an icon-only option', () => {
  const prefs = defaults(); const snapshot = demoSnapshot(); const totals = aggregateUsage(snapshot, 'today', prefs.enabledProviders);
  for (const [view, label] of Object.entries({ icon: '', tokens: '2.84M', limits: '71%', 'tokens-limits': '2.84M · 71%', spend: '~$21.94', custom: '2.84M · 71%' })) {
    assert.equal(trayContent({ ...prefs, menuBarView: view as typeof prefs.menuBarView }, snapshot, totals).label, label);
  }
  assert.ok(trayContent({ ...prefs, menuBarView: 'icon' }, snapshot, totals).rows.length > 0);
});
test('a single provider scopes tokens, quotas, spend and models without changing dashboard totals', () => {
  const prefs = defaults(); prefs.menuBarProvider = 'codex'; prefs.menuBarView = 'custom'; prefs.menuBarItems = ['tokens', 'limits', 'spend'];
  prefs.menuBarDetails = 'models'; prefs.surfaces.menubar.models = true;
  const snapshot = demoSnapshot(); const totals = aggregateUsage(snapshot, 'today', prefs.enabledProviders);
  const content = trayContent(prefs, snapshot, totals);
  assert.equal(content.label, '840K · 38% · ~$6.49'); assert.equal(totals.total, 2840000);
  assert.ok(content.rows.includes('GPT Codex: 840K')); assert.ok(!content.rows.some(row => /Claude|Sonnet|Composer|OpenCode|GLM/.test(row)));
  assert.ok(content.tooltip.includes('Codex')); assert.ok(content.tooltip.includes('$6.49 Estimated'));
});
test('hidden or missing sources show unavailable values rather than leaking another provider', () => {
  const prefs = defaults(); prefs.menuBarProvider = 'codex'; prefs.enabledProviders = ['claude']; prefs.menuBarView = 'custom'; prefs.menuBarItems = ['tokens', 'limits', 'spend'];
  const snapshot = demoSnapshot(); const totals = aggregateUsage(snapshot, 'today', prefs.enabledProviders);
  assert.equal(trayContent(prefs, snapshot, totals).label, 'N/A · N/A · N/A');
  assert.equal(trayContent({ ...defaults(), menuBarView: 'tokens-limits' }, emptySnapshot(), aggregateUsage(emptySnapshot(), 'today', [])).label, 'N/A · N/A');
});
test('custom metric order and labels survive without affecting dropdown visibility', () => {
  const prefs = defaults(); prefs.menuBarView = 'custom'; prefs.menuBarItems = ['spend', 'tokens']; prefs.menuBarLabels = true;
  const snapshot = demoSnapshot(); const totals = aggregateUsage(snapshot, 'today', prefs.enabledProviders);
  const content = trayContent(prefs, snapshot, totals);
  assert.equal(content.label, 'Est. $21.94 · Tokens 2.84M'); assert.ok(!content.rows.some(row => row.startsWith('Estimated Spend')));
  assert.equal(trayContent({ ...prefs, menuBarItems: [] }, snapshot, totals).label, '');
});
test('partially priced spend and recorded zero cost remain honest in the pinned summary', () => {
  const prefs = defaults(); prefs.menuBarView = 'spend';
  const snapshot = demoSnapshot(); snapshot.events = [
    { id: 'free', provider: 'opencode', model: 'Free', timestamp: new Date().toISOString(), tokens: 100, costUSD: 0 },
    { id: 'unknown', provider: 'opencode', model: 'Unknown', timestamp: new Date().toISOString(), tokens: 200 },
  ];
  const content = trayContent(prefs, snapshot, aggregateUsage(snapshot, 'today', prefs.enabledProviders));
  assert.equal(content.label, '~$0.00*'); assert.ok(content.rows.includes('Price Unavailable: 200 Tokens')); assert.ok(content.tooltip.includes('Partial Estimate'));
});
test('provider and model dropdown variants keep their own detail and fit the native menu limits', () => {
  const prefs = defaults(); prefs.surfaces.menubar = { tokens: true, limits: true, models: true, rank: true, spend: true, trend: true };
  const snapshot = demoSnapshot();
  for (let i = 0; i < 8; i++) snapshot.events.push({ id: 'extra-' + i, provider: 'codex', model: '模型'.repeat(60) + i, timestamp: new Date().toISOString(), tokens: 100000, costUSD: 1 });
  const totals = aggregateUsage(snapshot, 'today', prefs.enabledProviders);
  const providers = trayContent({ ...prefs, menuBarDetails: 'providers' }, snapshot, totals);
  assert.ok(providers.rows.some(row => row.startsWith('Claude: 1.12M Tokens · Session 64% Used')));
  const models = trayContent({ ...prefs, menuBarDetails: 'models' }, snapshot, totals);
  assert.equal(models.rows.filter(row => row.includes('…')).length, 1);
  for (const content of [providers, models]) {
    assert.ok(content.rows.length <= 20); assert.ok(content.rows.every(row => Buffer.byteLength(row) <= 256));
    assert.ok(Buffer.byteLength(content.label) <= 128); assert.ok(Buffer.byteLength(content.tooltip) <= 512);
  }
});
