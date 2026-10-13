import { test } from 'node:test';
import assert from 'node:assert/strict';
import { aggregateUsage, demoSnapshot, emptySnapshot, formatCost, formatSpend, formatTokens, periodBounds } from './usage.ts';
const now = new Date(2026, 8, 30, 16, 0);
test('demo totals reconcile across providers and models', () => {
  const totals = aggregateUsage(demoSnapshot(now), 'today', ['claude', 'codex', 'cursor'], now);
  assert.equal(totals.total, 2420000); assert.equal(totals.delta, 24);
  assert.equal(Object.values(totals.providers).reduce((a, b) => a + b), totals.total);
  assert.equal(totals.models.reduce((a, b) => a + b.tokens, 0), totals.total);
});
test('repeated and incomplete provider records do not inflate totals', () => {
  const snapshot = demoSnapshot(now); const event = snapshot.events.at(-1)!;
  snapshot.events.push(event, { ...event, tokens: 1 }, { ...event, id: 'invalid', tokens: -5 });
  assert.equal(aggregateUsage(snapshot, 'today', ['claude', 'codex', 'cursor', 'opencode'], now).total, 2840000);
});
test('unpriced models still count and priced spend is clearly partial', () => {
  const snapshot = demoSnapshot(now); snapshot.events.find(event => event.id === 'preview-6-cursor')!.costUSD = null;
  const totals = aggregateUsage(snapshot, 'today', ['claude', 'codex', 'cursor'], now);
  assert.equal(totals.total, 2420000); assert.ok(Math.abs(totals.estimatedCost! - 1960000 / 1000000 * 7.727) < 1e-9);
  assert.equal(totals.unpricedTokens, 460000); assert.deepEqual(totals.unpricedModels, ['Composer']);
  assert.equal(totals.providerCosts.cursor, null); assert.equal(totals.providerUnpricedTokens.cursor, 460000);
});
test('partial daily groups preserve their known dollars and coverage', () => {
  const snapshot = emptySnapshot(); snapshot.events = [{ id: 'daily', provider: 'codex', model: 'new-model', timestamp: now.toISOString(), tokens: 1000, costUSD: 2.5, unpricedTokens: 200 }];
  const totals = aggregateUsage(snapshot, 'today', ['codex'], now);
  assert.equal(totals.estimatedCost, 2.5); assert.equal(totals.unpricedTokens, 200);
  assert.equal(totals.models[0].estimatedCost, 2.5); assert.equal(totals.models[0].unpricedTokens, 200);
});
test('unknown pricing in a disabled provider does not mark visible spend partial', () => {
  const snapshot = demoSnapshot(now); snapshot.events.at(-1)!.costUSD = null;
  const totals = aggregateUsage(snapshot, 'today', ['codex'], now);
  assert.equal(totals.unpricedTokens, 0); assert.deepEqual(totals.unpricedModels, []);
  assert.equal(totals.estimatedCost, 840000 / 1000000 * 7.727);
});
test('provider filters affect totals, model share, and history together', () => {
  const totals = aggregateUsage(demoSnapshot(now), 'today', ['codex'], now);
  assert.equal(totals.total, 840000); assert.equal(totals.models.length, 1); assert.equal(totals.models[0].share, 100);
  assert.equal(totals.trend.at(-1)!.tokens, 840000);
  assert.equal(aggregateUsage(demoSnapshot(now), 'today', [], now).hasData, false);
});
test('week starts on local Monday and month starts on the first', () => {
  const [week, , previous] = periodBounds('week', now); assert.equal(week.getDay(), 1); assert.equal(week.getDate(), 28); assert.equal(previous.getDate(), 21);
  const [month] = periodBounds('month', now); assert.equal(month.getDate(), 1); assert.equal(month.getHours(), 0);
});
test('day boundaries exclude tomorrow and separate yesterday', () => {
  const snapshot = emptySnapshot(); const [start, end, previous] = periodBounds('today', now);
  snapshot.events = [start, end, previous].map((date, index) => ({ id: String(index), provider: 'codex', model: 'new-model', timestamp: date.toISOString(), tokens: 100 }));
  const totals = aggregateUsage(snapshot, 'today', ['codex'], now); assert.equal(totals.total, 100); assert.equal(totals.previous, 100); assert.equal(totals.delta, 0);
});
test('empty usage has no invented spend or percentage comparison', () => {
  const totals = aggregateUsage(emptySnapshot(), 'today', ['claude'], now);
  assert.equal(totals.hasData, false); assert.equal(totals.delta, null); assert.equal(totals.estimatedCost, null);
});
test('zero recorded cost is valid and partial estimates are labeled', () => {
  const snapshot = emptySnapshot(); snapshot.events = [{ id: 'zero', provider: 'claude', model: 'known', timestamp: now.toISOString(), tokens: 100, costUSD: 0 }];
  const totals = aggregateUsage(snapshot, 'today', ['claude'], now);
  assert.equal(formatSpend(totals), '$0.00 Estimated'); assert.equal(formatCost(null), 'N/A');
  snapshot.events.push({ id: 'unknown', provider: 'claude', model: 'unknown', timestamp: now.toISOString(), tokens: 200, costUSD: null });
  assert.equal(formatSpend(aggregateUsage(snapshot, 'today', ['claude'], now)), '$0.00 Partial Estimate');
});
test('compact numbers preserve useful precision', () => {
  assert.equal(formatTokens(2420000), '2.42M'); assert.equal(formatTokens(840000), '840K'); assert.equal(formatTokens(0), '0'); assert.equal(formatTokens(1050), '1.1K');
  assert.equal(formatTokens(2346630000), '2.35B');
});
test('OpenCode contributes to totals, costs, models and history and can be filtered independently', () => {
  const snapshot = demoSnapshot(now);
  const all = aggregateUsage(snapshot, 'today', ['claude', 'codex', 'cursor', 'opencode'], now);
  assert.equal(all.total, 2840000); assert.equal(all.providers.opencode, 420000);
  const only = aggregateUsage(snapshot, 'today', ['opencode'], now);
  assert.equal(only.total, 420000); assert.equal(only.models[0].model, 'GLM-5.3'); assert.equal(only.models[0].share, 100);
  assert.equal(only.trend.at(-1)!.tokens, 420000); assert.equal(only.providerCosts.opencode, 420000 / 1000000 * 7.727);
  assert.equal(aggregateUsage(snapshot, 'today', ['codex'], now).providers.opencode, 0);
});
