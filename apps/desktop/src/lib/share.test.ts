import { test } from 'node:test';
import assert from 'node:assert/strict';
import { formatShareTokens, matchingRank, shareTotals, suggestedCaption, shareLink, xDraftURL, sharePeriodLabel } from './share.ts';
import { emptySnapshot } from './usage.ts';
const now = new Date('2026-10-20T16:00:00Z');
test('usage shares deduplicated actual totals without project names or paths', () => {
  const source = emptySnapshot(); const event = { id: '1', provider: 'claude' as const, timestamp: now.toISOString(), model: 'claude', tokens: 100, project: { id: 'a', name: 'private-client', directory: '/secret/client' } };
  source.events = [event, event, { ...event, id: '2', tokens: 700 }];
  const totals = shareTotals(source, 'today', ['claude'], now); assert.equal(totals.total, 800);
  const caption = suggestedCaption('usage', 'today', { tokens: true, providers: true, cost: false }, totals, undefined);
  assert.match(caption, /800 tokens/); assert.doesNotMatch(caption, /secret|private-client/);
});
test('rank uses public score and rejects stale or mismatched period', () => {
  const source = emptySnapshot(); source.rank = { position: 3, tokens: 55, period: 'today', updatedAt: now.toISOString(), podiums: 0 };
  assert.equal(matchingRank(source, 'today', now)?.tokens, 55); assert.equal(matchingRank(source, 'week', now), undefined);
  source.rank.updatedAt = '2026-10-19T12:00:00Z'; assert.equal(matchingRank(source, 'today', now), undefined);
  source.rank.updatedAt = now.toISOString(); source.rankUnavailable = true; assert.equal(matchingRank(source, 'today', now), undefined);
});
test('hidden metrics stay out of generated caption and intent safely encodes text', () => {
  const source = emptySnapshot(); source.events = [{ id: 'a', provider: 'codex', model: 'x', timestamp: now.toISOString(), tokens: 123456, costUSD: 12 }];
  const totals = shareTotals(source, 'today', ['codex'], now);
  const text = suggestedCaption('usage', 'today', { tokens: false, providers: false, cost: false }, totals, undefined);
  assert.doesNotMatch(text, /Codex|123|\$12/); assert.match(text, /@DevClashAI/); assert.doesNotMatch(text, /https?:\/\//);
  const url = new URL(xDraftURL(text + ' & #test')); assert.equal(url.host, 'x.com'); assert.equal(url.searchParams.get('text'), text + ' & #test');
});
test('local usage has no fabricated public snapshot and rank link names UTC period', () => {
  assert.equal(shareLink('usage', 'today', now), 'https://devclash.ai');
  const url = new URL(shareLink('rank', 'week', now)); assert.equal(url.searchParams.get('period'), 'Week'); assert.equal(url.searchParams.get('date'), '2026-10-20');
  assert.match(sharePeriodLabel('week', true, now), /Oct 19–Oct 20 2026 · UTC/);
});

test('rank caption uses the actual public position and score, never local usage or a podium default', () => {
  const source = emptySnapshot(); source.events = [{id:'local', provider:'codex', model:'x', timestamp:now.toISOString(), tokens:999999}];
  const totals = shareTotals(source, 'today', ['codex'], now);
  for (const position of [1, 42, 1284, 1234567]) {
    const rank = {position, tokens:12345, period:'today' as const, updatedAt:now.toISOString(), podiums:0};
    const caption = suggestedCaption('rank', 'today', {tokens:true, providers:true, cost:false}, totals, rank);
    assert.ok(caption.includes(`#${position} `)); assert.match(caption, /12K/); assert.doesNotMatch(caption, /999|Codex|winner|trophy/);
  }
  assert.equal(suggestedCaption('rank', 'today', {tokens:true, providers:true, cost:false}, totals, undefined), '');
});

test('share token labels have no decimals and carry rounding across compact units', () => {
  for (const [tokens, expected] of [[0, '0'], [999, '999'], [1000, '1K'], [12345, '12K'], [999500, '1M'], [63800000, '64M'], [285140000, '285M'], [285900000, '286M'], [999500000, '1B'], [1120000000, '1B']] as const) {
    assert.equal(formatShareTokens(tokens), expected);
  }
});

test('all three caption styles use selected period and actual metrics for usage and rank', () => {
  const source = emptySnapshot();
  source.events = [{ id: 'a', provider: 'codex', model: 'x', timestamp: now.toISOString(), tokens: 285140000, costUSD: 12 }];
  const totals = shareTotals(source, 'month', ['codex'], now);
  const rank = { position: 42, tokens: 12345000, period: 'month' as const, updatedAt: now.toISOString(), podiums: 0 };
  for (const kind of ['usage', 'rank'] as const) {
    const captions = (['simple', 'community', 'challenge'] as const).map(style => {
      const text = suggestedCaption(kind, 'month', { tokens: true, providers: true, cost: true }, totals, rank, style);
      assert.match(text, /this month/); assert.match(text, /@DevClashAI/);
      if (kind === 'usage') { assert.match(text, /285M/); assert.match(text, /Codex/); assert.match(text, /\$12/); }
      else { assert.match(text, /#42/); assert.match(text, /12M/); assert.doesNotMatch(text, /285M|Codex|\$12|winner|trophy/); }
      return text;
    });
    assert.equal(new Set(captions).size, 3);
  }
});

test('every caption style respects hidden metrics and missing rank', () => {
  const source = emptySnapshot();
  source.events = [{ id: 'a', provider: 'codex', model: 'x', timestamp: now.toISOString(), tokens: 285140000, costUSD: 12 }];
  const totals = shareTotals(source, 'today', ['codex'], now);
  const fields = { tokens: false, providers: false, cost: false };
  const rank = { position: 42, tokens: 12345000, period: 'today' as const, updatedAt: now.toISOString(), podiums: 0 };
  for (const style of ['simple', 'community', 'challenge'] as const) {
    for (const kind of ['usage', 'rank'] as const) {
      const text = suggestedCaption(kind, 'today', fields, totals, rank, style);
      assert.doesNotMatch(text, /285M|12M|Codex|\$12/);
      assert.match(text, /@DevClashAI/);
    }
    assert.equal(suggestedCaption('rank', 'today', fields, totals, undefined, style), '');
  }
});


test('fresh server ranks tolerate small device clock skew but reject far-future timestamps', () => {
  const source = emptySnapshot();
  source.rank = { position: 42, tokens: 12345, period: 'today', updatedAt: new Date(now.getTime() + 60_000).toISOString(), podiums: 0 };
  assert.equal(matchingRank(source, 'today', now)?.position, 42);
  source.rank.updatedAt = new Date(now.getTime() + 5 * 60_000).toISOString();
  assert.equal(matchingRank(source, 'today', now)?.position, 42);
  source.rank.updatedAt = new Date(now.getTime() + 5 * 60_000 + 1).toISOString();
  assert.equal(matchingRank(source, 'today', now), undefined);
  source.rank.updatedAt = 'not-a-date';
  assert.equal(matchingRank(source, 'today', now), undefined);
});
