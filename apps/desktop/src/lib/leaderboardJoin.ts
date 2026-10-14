import type { CloudStatus } from './cloud.ts';

export type JoinState = 'invitation' | 'pending' | 'joined' | 'ranked';
export function leaderboardJoinState(status: Pick<CloudStatus, 'ranked' | 'joinPending'>, hasRank: boolean): JoinState {
  if (status.joinPending) return 'pending';
  return status.ranked ? hasRank ? 'ranked' : 'joined' : 'invitation';
}
export const joinRefreshInterval = 5000;
export const joinRefreshLimit = 24;
