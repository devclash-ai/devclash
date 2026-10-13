import type { CloudStatus } from './cloud';
import type { Period, UsageSnapshot } from './types';
export const cloudPeriod = (period: Period): 'Today' | 'Week' | 'Month' => period === 'today' ? 'Today' : period === 'week' ? 'Week' : 'Month';
export function publicRank(status: CloudStatus, period: Period): UsageSnapshot['rank'] {
  if (!status.ranked || status.rankPeriod !== cloudPeriod(period) || !Number.isSafeInteger(status.rank) || status.rank! < 1 ||
      !Number.isSafeInteger(status.rankTokens) || status.rankTokens! < 0 || !status.rankGeneratedAt) return undefined;
  return { position: status.rank!, tokens: status.rankTokens!, period, podiums: status.badges, updatedAt: status.rankGeneratedAt };
}
