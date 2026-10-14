import test from 'node:test';
import assert from 'node:assert/strict';
import { leaderboardJoinState, joinRefreshInterval, joinRefreshLimit } from './leaderboardJoin.ts';

test('joining stays actionable until confirmed, and missing ranks never become a placement', () => {
  assert.equal(leaderboardJoinState({ ranked: false, joinPending: false }, false), 'invitation');
  assert.equal(leaderboardJoinState({ ranked: false, joinPending: true }, false), 'pending');
  assert.equal(leaderboardJoinState({ ranked: false, joinPending: true }, true), 'pending');
  assert.equal(leaderboardJoinState({ ranked: true, joinPending: false }, false), 'joined');
  assert.equal(leaderboardJoinState({ ranked: true, joinPending: true }, false), 'pending');
  assert.equal(leaderboardJoinState({ ranked: true, joinPending: true }, true), 'pending');
  assert.equal(leaderboardJoinState({ ranked: true, joinPending: false }, true), 'ranked');
});
test('pending status checks have a two minute polling budget', () => {
  assert.equal(joinRefreshInterval * joinRefreshLimit, 120000);
});
