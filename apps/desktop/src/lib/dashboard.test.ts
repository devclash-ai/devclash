import { test } from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import { defaults } from './preferences.ts';
import { aggregateUsage, emptySnapshot } from './usage.ts';

test('the desktop renders collected Cursor usage, zero and history failures without manual upload controls', async () => {
  const globals = { location: { pathname: '/', search: '' }, matchMedia: () => ({ matches: false }), isTauri: true };
  const saved = new Map(Object.keys(globals).map(key => [key, Object.getOwnPropertyDescriptor(globalThis, key)]));
  for (const [key, value] of Object.entries(globals)) Object.defineProperty(globalThis, key, { configurable: true, value });
  let server: Awaited<ReturnType<typeof createServer>> | undefined;
  try {
    server = await createServer({ root: fileURLToPath(new URL('../..', import.meta.url)), server: { middlewareMode: true, hmr: false, ws: false, watch: null } });
    const { Dashboard } = await server.ssrLoadModule('/src/components/Dashboard.tsx') as typeof import('../components/Dashboard');
    const { CloudDialog } = await server.ssrLoadModule('/src/components/CloudDialog.tsx') as typeof import('../components/CloudDialog');
    const { localCloudStatus } = await server.ssrLoadModule('/src/lib/cloud.ts') as typeof import('./cloud');
    const { App } = await server.ssrLoadModule('/src/App.tsx') as typeof import('../App');
    const { SourceDialog, ProviderSourceField } = await server.ssrLoadModule('/src/components/SourceDialog.tsx') as typeof import('../components/SourceDialog');
    const app = renderToStaticMarkup(createElement(App));
    assert.doesNotMatch(app, /<input\b[^>]*\btype="file"/);
    assert.doesNotMatch(app, /Import CSV/);
    const sources = (quotaErrors: { claude?: string; cursor?: string }) => renderToStaticMarkup(createElement(SourceDialog, {
      snapshot: { ...emptySnapshot(), quotaErrors }, onRefresh: async () => {}, onClose: () => {},
    }));
    const permission = 'Keychain access is required. Open Usage Sources and authorize this provider, then Refresh.';
    assert.match(sources({ claude: permission }), /Authorize Claude Desktop/);
    assert.match(sources({ claude: permission }), /Authorize Claude Code/);
    assert.match(sources({ cursor: permission }), /Authorize Cursor CLI/);
    assert.doesNotMatch(sources({}), /Authorize Claude|Authorize Cursor/);
    const workingQuota = { ...emptySnapshot(), quotas: { claude: { label: 'Session', used: 12 } } };
    const field = (platform: string, value = '', id: 'claude' | 'cursor' | 'opencode' = 'claude') => renderToStaticMarkup(createElement(ProviderSourceField, {
      id, platform, value, detected: [], snapshot: workingQuota, busy: false, onChange: () => {}, onAuthorize: () => {},
    }));
    const macField = field('macos');
    assert.match(macField, /aria-label="Authorize Claude Desktop"/);
    assert.match(macField, /aria-label="Authorize Claude Code"/);
    assert.match(macField, /<label for="source-folder-claude"><strong>Claude<\/strong><\/label><textarea id="source-folder-claude"/);
    for (const label of macField.match(/<label\b[^>]*>[\s\S]*?<\/label>/g) ?? []) assert.doesNotMatch(label, /<button/);
    assert.match(field('macos', '', 'cursor'), /aria-label="Authorize Cursor CLI"/);
    assert.doesNotMatch(field('macos', '', 'opencode'), /Authorize/);
    assert.doesNotMatch(field('macos', '/isolated/provider'), /Authorize/);
    assert.doesNotMatch(field('windows'), /Authorize/);
    assert.doesNotMatch(field('linux'), /Authorize/);

    const now = new Date('2026-10-01T12:00:00Z');
    const prefs = { ...defaults(), enabledProviders: ['claude', 'cursor'] as const };
    const preferences = { ...prefs, enabledProviders: [...prefs.enabledProviders] };
    const snapshot = { ...emptySnapshot(), availableProviders: [...prefs.enabledProviders], source: 'local' as const, fetchedAt: now.toISOString(), events: [
      { id: 'cursor-account-record', provider: 'cursor' as const, model: 'Composer', timestamp: now.toISOString(), tokens: 420 },
      { id: 'claude-local-record', provider: 'claude' as const, model: 'Sonnet', timestamp: now.toISOString(), tokens: 1100 },
    ] };
    const noop = () => {};
    const render = () => renderToStaticMarkup(createElement(Dashboard, {
      preferences, snapshot, totals: aggregateUsage(snapshot, 'today', [...prefs.enabledProviders], now), period: 'today', theme: 'light',
      setPeriod: noop, onCustomize: noop, onRefresh: noop, onAccount: noop, onWidget: noop, onShare: noop, onRank: noop,
    }));
    const joinDashboard = (joinState: 'invitation' | 'pending' | 'joined', syncEnabled = false) => renderToStaticMarkup(createElement(Dashboard, {
      preferences, snapshot, totals: aggregateUsage(snapshot, 'today', [...prefs.enabledProviders], now), period: 'today', theme: 'dark', connected: joinState === 'joined', joinState, syncEnabled,
      setPeriod: noop, onCustomize: noop, onRefresh: noop, onAccount: noop, onWidget: noop, onShare: noop, onRank: noop,
    }));
    const invitation = joinDashboard('invitation');
    assert.match(invitation, /Join The Leaderboard/);
    assert.match(invitation, /Join &amp; Enable Sync/);
    assert.match(invitation, /Joining enables Cloud Sync/);
    assert.ok(invitation.indexOf('usage-summary') < invitation.indexOf('leaderboard-invitation'));
    assert.ok(invitation.indexOf('leaderboard-invitation') < invitation.indexOf('providers-section'));
    assert.match(invitation, /420 tokens/);
    assert.match(joinDashboard('pending'), /Continue Joining/);
    const pendingExistingMembership = renderToStaticMarkup(createElement(CloudDialog, {
      type: 'rank', status: { ...localCloudStatus, configured: true, ranked: true, joinPending: true, user: { id: 'native-account', email: 'qa@example.invalid' } },
      join: { request: null, busy: false, error: '', start: async () => {} }, onChange: noop, onView: noop, onClose: noop,
    }));
    assert.match(pendingExistingMembership, /Waiting for Join Confirmation/);
    assert.match(pendingExistingMembership, /Continue Joining/);
    assert.match(pendingExistingMembership, /Joining enables Cloud Sync for this device/);

    const joined = joinDashboard('joined');
    assert.match(joined, /Leaderboard Joined/);
    assert.match(joined, /Awaiting Your First Eligible Usage/);
    assert.match(joined, /Review Sync Settings/);
    assert.doesNotMatch(joined, /#[0-9]|Turn Usage Into Credits|Join Leaderboard|Join &amp; Enable Sync/);
    assert.doesNotMatch(joinDashboard('joined', true), /Review Sync Settings|Cloud Sync Is Off/);
    assert.match(render(), /aria-label="Expand Cursor Project Usage"[\s\S]*?<\/button><div class="provider-tokens"><strong>420 tokens<\/strong>/);
    snapshot.events = snapshot.events.filter(event => event.provider !== 'cursor');
    assert.match(render(), /aria-label="Expand Cursor Project Usage"[\s\S]*?<\/button><div class="provider-tokens"><strong>0 tokens<\/strong>/);

    const unavailable = { ...snapshot, availableProviders: ['claude' as const], providerErrors: { cursor: 'Cursor usage could not be refreshed. Check your connection and try again.' } };
    const failed = renderToStaticMarkup(createElement(Dashboard, {
      preferences, snapshot: unavailable, totals: aggregateUsage(unavailable, 'today', [...prefs.enabledProviders], now), period: 'today', theme: 'light',
      setPeriod: noop, onCustomize: noop, onRefresh: noop, onAccount: noop, onWidget: noop, onShare: noop, onRank: noop,
    }));
    assert.match(failed, /aria-label="Expand Cursor Project Usage"[\s\S]*?<\/button><div class="provider-tokens"><strong>N\/A<\/strong>/);
    assert.match(failed, /role="status">Cursor usage could not be refreshed/);
    assert.match(failed, /aria-label="Expand Claude Project Usage"[\s\S]*?<\/button><div class="provider-tokens"><strong>1\.1K tokens<\/strong>/);
    const renderFailure = (quotaErrors = {}, quotas = {}, limits = true) => renderToStaticMarkup(createElement(Dashboard, {
      preferences: { ...preferences, surfaces: { ...preferences.surfaces, dashboard: { ...preferences.surfaces.dashboard, limits } } },
      snapshot: { ...unavailable, quotaErrors, quotas }, totals: aggregateUsage(unavailable, 'today', [...prefs.enabledProviders], now), period: 'today', theme: 'dark',
      setPeriod: noop, onCustomize: noop, onRefresh: noop, onAccount: noop, onWidget: noop, onShare: noop, onRank: noop,
    }));
    const sameFailure = renderFailure({ cursor: unavailable.providerErrors.cursor });
    assert.equal(sameFailure.split(unavailable.providerErrors.cursor).length - 1, 1, 'the same usage/quota failure is shown once');
    const independentQuota = renderFailure({}, { cursor: { label: 'Plan', used: 21 } });
    assert.match(independentQuota, /21% used/);
    assert.match(independentQuota, /role="status">Cursor usage could not be refreshed/);
    assert.match(renderFailure({ cursor: 'Quota service unavailable' }), /Quota service unavailable/);
    assert.match(renderFailure({}, {}, false), /role="status">Cursor usage could not be refreshed/);

    const renderAuthorization = (value: typeof snapshot, pending: 'claude' | null = null, enabled = true) => renderToStaticMarkup(createElement(Dashboard, {
      preferences: defaults(), snapshot: value, totals: aggregateUsage(value, 'today', ['claude', 'codex', 'cursor', 'opencode'], now), period: 'today', theme: 'dark',
      setPeriod: noop, onCustomize: noop, onRefresh: noop, onAccount: noop, onWidget: noop, onShare: noop, onRank: noop,
      onRetryAuthorization: enabled ? noop : undefined, authorizationPending: pending,
    }));
    const needsPermission = { ...snapshot, quotaErrors: { claude: permission, cursor: permission, codex: 'Sign in to Codex first.', opencode: permission } };
    const retry = renderAuthorization(needsPermission);
    assert.match(retry, /aria-label="Retry Claude Authorization">Retry<\/button>/);
    assert.match(retry, /aria-label="Retry Cursor Authorization">Retry<\/button>/);
    assert.doesNotMatch(retry, /Retry Codex Authorization|Retry OpenCode Authorization/);
    assert.doesNotMatch(renderAuthorization(needsPermission, null, false), /Retry Claude Authorization/);
    const pending = renderAuthorization(needsPermission, 'claude');
    assert.match(pending, /disabled="" aria-label="Retry Claude Authorization">Authorizing…<\/button>/);
    assert.match(pending, /disabled="" aria-label="Retry Cursor Authorization">Retry<\/button>/);
    const sharedPermission = renderAuthorization({ ...snapshot, providerErrors: { cursor: permission }, quotaErrors: { cursor: permission } });
    assert.equal(sharedPermission.split('aria-label="Retry Cursor Authorization"').length - 1, 1);
    assert.doesNotMatch(renderAuthorization({ ...needsPermission, quotas: { claude: { label: 'Session', used: 12 } } }), /Retry Claude Authorization/);
  } finally {
    await server?.close();
    for (const [key, descriptor] of saved) { if (descriptor) Object.defineProperty(globalThis, key, descriptor); else Reflect.deleteProperty(globalThis, key); }
  }
});
