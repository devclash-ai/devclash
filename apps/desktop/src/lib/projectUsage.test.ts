import { test } from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import { matchingProjects, providerProjects } from './projectUsage.ts';
import { aggregateUsage, emptySnapshot } from './usage.ts';
import type { ProviderID, UsageEvent } from './types';

const now = new Date(2026, 9, 8, 16);
const atlas = { id: 'atlas-folder', name: 'Atlas', directory: '/projects/atlas' };
const event = (id: string, tokens: number, options: Partial<UsageEvent> = {}): UsageEvent => ({ id, provider: 'codex', model: 'gpt-test', tokens, timestamp: now.toISOString(), project: atlas, ...options });
const snapshot = (events: UsageEvent[]) => ({ ...emptySnapshot(), events });
const all: ProviderID[] = ['claude', 'codex', 'cursor', 'opencode'];

test('project usage reconciles with the total, tools, models and partial cost', () => {
  const totals = aggregateUsage(snapshot([
    event('a', 100, { costUSD: 1 }), event('b', 200, { provider: 'claude', model: 'sonnet', costUSD: 2, unpricedTokens: 50 }),
    event('c', 50, { provider: 'opencode', model: 'unknown' }), event('d', 150, { provider: 'cursor', project: undefined, costUSD: 0 }),
  ]), 'today', all, now);
  assert.equal(totals.total, 500);
  assert.equal(totals.projects.reduce((sum, project) => sum + project.tokens, 0), totals.total);
  const project = totals.projects[0];
  assert.equal(project.name, 'Atlas'); assert.equal(project.tokens, 350); assert.equal(project.share, 70);
  assert.equal(project.estimatedCost, 3); assert.equal(project.unpricedTokens, 100);
  assert.equal(project.providers.reduce((sum, tool) => sum + tool.tokens, 0), project.tokens);
  assert.equal(project.models.reduce((sum, model) => sum + model.tokens, 0), project.tokens);
  assert.equal(totals.projects[1].name, 'Unassigned Usage'); assert.equal(totals.projects[1].directory, null);
  assert.equal(totals.projects[1].estimatedCost, 0);
});

test('project rows use the same provider-qualified deduplication as the dashboard', () => {
  const first = event('shared-id', 100);
  const totals = aggregateUsage(snapshot([first, first, { ...first, tokens: 80 }, { ...first, tokens: 120 }, event('shared-id', 30, { provider: 'claude' })]), 'today', all, now);
  assert.equal(totals.total, 150); assert.equal(totals.projects[0].tokens, 150);
  assert.equal(totals.projects[0].providers.length, 2);
});

test('project usage follows local day, Monday week and calendar month boundaries', () => {
  const data = snapshot([
    event('today', 100), event('week', 50, { timestamp: new Date(2026, 9, 6, 12).toISOString() }),
    event('month', 25, { timestamp: new Date(2026, 9, 2, 12).toISOString() }),
    event('previous-month', 10, { timestamp: new Date(2026, 8, 30, 12).toISOString() }),
    event('tomorrow', 900, { timestamp: new Date(2026, 9, 9, 0).toISOString() }),
  ]);
  for (const [period, expected] of [['today', 100], ['week', 150], ['month', 175]] as const) {
    const totals = aggregateUsage(data, period, all, now);
    assert.equal(totals.total, expected); assert.equal(totals.projects[0].tokens, expected);
  }
});

test('disabled tools are excluded from project amounts, costs and unassigned usage', () => {
  const totals = aggregateUsage(snapshot([event('codex', 100, { costUSD: 1 }), event('claude', 900, { provider: 'claude', project: undefined })]), 'today', ['codex'], now);
  assert.equal(totals.projects.length, 1); assert.equal(totals.projects[0].tokens, 100);
  assert.equal(totals.projects[0].unpricedTokens, 0); assert.equal(totals.projects[0].providers[0].id, 'codex');
});

test('equal folder names stay distinct while the same folder across tools is combined', () => {
  const second = { id: 'second-atlas-folder', name: 'Atlas', directory: '/other/atlas' };
  const totals = aggregateUsage(snapshot([event('a', 100), event('b', 50, { provider: 'claude' }), event('c', 80, { project: second })]), 'today', all, now);
  assert.equal(totals.projects.length, 2); assert.equal(totals.projects[0].tokens, 150);
  assert.notEqual(totals.projects[0].id, totals.projects[1].id);
  assert.equal(matchingProjects(totals.projects, ' /OTHER/ ').length, 1);
  assert.equal(matchingProjects(totals.projects, ' atlas ').length, 2);
  assert.equal(matchingProjects(totals.projects, 'missing').length, 0);
});

test('unknown and absent project metadata never become a guessed project', () => {
  const totals = aggregateUsage(snapshot([event('unknown', 100, { project: null, costUSD: null })]), 'today', all, now);
  assert.equal(totals.projects.length, 1); assert.equal(totals.projects[0].name, 'Unassigned Usage');
  assert.equal(totals.projects[0].estimatedCost, null); assert.equal(totals.projects[0].unpricedTokens, 100);
  assert.deepEqual(aggregateUsage(emptySnapshot(), 'today', all, now).projects, []);
});

test('provider expansion scopes project totals, costs and even shared model names to that tool', () => {
  const second = { id: 'second', name: 'Second', directory: '/projects/second' };
  const totals = aggregateUsage(snapshot([
    event('codex-a', 100, { model: 'shared-model', costUSD: 1 }),
    event('claude-a', 600, { provider: 'claude', model: 'shared-model', costUSD: 6, unpricedTokens: 50 }),
    event('claude-b', 100, { provider: 'claude', model: 'sonnet', costUSD: 2 }),
    event('codex-b', 80, { project: second, model: 'codex-only' }),
    event('codex-unassigned', 20, { project: undefined, model: 'unknown-folder' }),
    event('claude-c', 50, { project: second, provider: 'claude', model: 'claude-only', costUSD: 0.5 }),
  ]), 'today', all, now);
  const codex = providerProjects(totals.projects, 'codex', totals.providers.codex);
  assert.deepEqual(codex.map(project => project.tokens), [100, 80, 20]);
  assert.deepEqual(codex.map(project => project.share), [50, 40, 10]);
  assert.equal(codex[0].estimatedCost, 1); assert.equal(codex[0].unpricedTokens, 0);
  assert.deepEqual(codex[0].models, [{ model: 'shared-model', tokens: 100, share: 100, estimatedCost: 1, unpricedTokens: 0 }]);
  const claude = providerProjects(totals.projects, 'claude', totals.providers.claude);
  assert.equal(claude[0].tokens, 700); assert.equal(claude[0].estimatedCost, 8); assert.equal(claude[0].unpricedTokens, 50);
  assert.deepEqual(claude[0].models.map(model => model.model), ['shared-model', 'sonnet']);
  for (const id of all) {
    const projects = providerProjects(totals.projects, id, totals.providers[id]);
    assert.equal(projects.reduce((sum, project) => sum + project.tokens, 0), totals.providers[id]);
    assert.equal(projects.reduce((sum, project) => sum + project.unpricedTokens, 0), totals.providerUnpricedTokens[id]);
    for (const project of projects) {
      assert.equal(project.models.reduce((sum, model) => sum + model.tokens, 0), project.tokens);
      assert.deepEqual(project.providers.map(provider => provider.id), [id]);
    }
  }
  assert.deepEqual(providerProjects(totals.projects, 'cursor', 0), []);
});

test('provider project panels keep directory details local, handle costs and empty periods', async () => {
  const globals = { location: { pathname: '/', search: '' }, matchMedia: () => ({ matches: false }), isTauri: true };
  const saved = new Map(Object.keys(globals).map(key => [key, Object.getOwnPropertyDescriptor(globalThis, key)]));
  for (const [key, value] of Object.entries(globals)) Object.defineProperty(globalThis, key, { configurable: true, value });
  let server: Awaited<ReturnType<typeof createServer>> | undefined;
  try {
    server = await createServer({ root: fileURLToPath(new URL('../..', import.meta.url)), server: { middlewareMode: true, hmr: false, ws: false, watch: null } });
    const { ProviderProjects } = await server.ssrLoadModule('/src/components/ProjectUsage.tsx') as typeof import('../components/ProjectUsage');
    const totals = aggregateUsage(snapshot([
      event('a', 100, { costUSD: 1, unpricedTokens: 20 }),
      event('b', 200, { provider: 'claude', model: 'claude-only', costUSD: 2 }),
      event('cursor', 25, { provider: 'cursor', project: undefined }),
    ]), 'today', all, now);
    const render = (provider: ProviderID, spend = true) => renderToStaticMarkup(createElement(ProviderProjects, {
      contentID: 'projects-' + provider, provider, projects: providerProjects(totals.projects, provider, totals.providers[provider]), total: totals.providers[provider], spend,
    }));
    const codex = render('codex');
    assert.match(codex, /data-no-share="true"/); assert.match(codex, /aria-label="Codex Project Usage"/);
    assert.match(codex, /Atlas Usage Details/); assert.match(codex, /\/projects\/atlas/);
    assert.match(codex, /\$1\.00 \(Partial\)/); assert.doesNotMatch(codex, /claude-only|Unassigned Usage/);
    assert.match(render('cursor'), /Unassigned Usage/); assert.match(render('cursor'), /Cursor reports account-wide usage/);
    assert.doesNotMatch(render('codex', false), /\$1\.00|Tokens \/ Cost/);
    assert.match(render('opencode'), /No measured usage in this period/);
    const { Dashboard } = await server.ssrLoadModule('/src/components/Dashboard.tsx') as typeof import('../components/Dashboard');
    const { defaults } = await server.ssrLoadModule('/src/lib/preferences.ts') as typeof import('./preferences');
    const noop = () => {};
    const dashboard = renderToStaticMarkup(createElement(Dashboard, {
      preferences: defaults(), snapshot: snapshot([]), totals, period: 'today', theme: 'dark',
      setPeriod: noop, onCustomize: noop, onRefresh: noop, onAccount: noop, onWidget: noop, onShare: noop, onRank: noop,
    }));
    for (const provider of ['Claude', 'Codex', 'Cursor', 'OpenCode']) assert.match(dashboard, new RegExp('aria-label="Expand ' + provider + ' Project Usage" aria-expanded="false"'));
    assert.doesNotMatch(dashboard, /Usage by Project|\/projects\/atlas|provider-projects-heading/);
  } finally {
    await server?.close();
    for (const [key, descriptor] of saved) { if (descriptor) Object.defineProperty(globalThis, key, descriptor); else Reflect.deleteProperty(globalThis, key); }
  }
});
