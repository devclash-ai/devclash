import { test } from 'node:test';
import assert from 'node:assert/strict';
import { publicRank } from './rank.ts';
import type { CloudStatus } from './cloud';
const status = { ranked: true, rank: 4, rankPeriod: 'Week', rankTokens: 123456, badges: 2, rankGeneratedAt: '2026-10-01T12:00:00Z' } as CloudStatus;
test('rank widgets use only the matching public UTC period and eligible score', () => {
  assert.equal(publicRank(status, 'month'), undefined);
  const rank = publicRank(status, 'week')!;
  assert.equal(rank.tokens, 123456); assert.equal(rank.period, 'week'); assert.equal(rank.movement, undefined);
  assert.equal(publicRank({ ...status, ranked: false }, 'week'), undefined);
  assert.equal(publicRank({ ...status, rankTokens: null }, 'week'), undefined);
});
