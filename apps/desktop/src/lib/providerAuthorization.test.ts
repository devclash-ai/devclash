import { test } from 'node:test';
import assert from 'node:assert/strict';
import { canRetryAuthorization, retryProviderAuthorization } from './providerAuthorization.ts';

test('permission retry is limited to supported local logins and permission errors', () => {
  const reason = 'Keychain access is required. Open Usage Sources and authorize this provider, then Refresh.';
  for (const id of ['claude', 'codex', 'cursor'] as const) assert.equal(canRetryAuthorization(id, reason), true);
  assert.equal(canRetryAuthorization('opencode', reason), false);
  assert.equal(canRetryAuthorization('claude', 'Sign in to Claude first.'), false);
  assert.equal(canRetryAuthorization('cursor', 'Network unavailable'), false);
  assert.equal(canRetryAuthorization('codex'), false);
});

test('retry authorizes the provider before refreshing measured usage and quotas', async () => {
  for (const [id, source] of [['claude', 'claude-retry'], ['cursor', 'cursor-cli'], ['codex', 'codex']] as const) {
    const calls: string[] = [];
    let release!: () => void;
    const permission = new Promise<void>(resolve => { release = resolve; });
    const retry = retryProviderAuthorization(id, async selected => { calls.push(selected); await permission; }, async () => { calls.push('refresh'); });
    assert.deepEqual(calls, [source]);
    release(); await retry;
    assert.deepEqual(calls, [source, 'refresh']);
  }
});

test('denied authorization remains an error and never runs the refresh as if authorized', async () => {
  let refreshed = false;
  await assert.rejects(retryProviderAuthorization('claude', async () => { throw 'Authorization was denied. Try again.'; }, async () => { refreshed = true; }),
    reason => reason === 'Authorization was denied. Try again.');
  assert.equal(refreshed, false);
  await assert.rejects(retryProviderAuthorization('opencode', async () => {}, async () => {}), /no local authorization action/);
});
