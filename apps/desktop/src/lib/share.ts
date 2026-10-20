import type { Period, ProviderID, UsageSnapshot } from './types';
import { aggregateUsage, formatCost, periodBounds } from './usage.ts';
const tokenFormatter = new Intl.NumberFormat('en-US', { notation: 'compact', maximumFractionDigits: 0 });
export const formatShareTokens = (tokens: number) => tokenFormatter.format(tokens);

export type ShareKind = 'usage' | 'rank';
export const captionStyles = { simple: 'Simple', community: 'Community', challenge: 'Challenge' } as const;
export type CaptionStyle = keyof typeof captionStyles;
export interface ShareFields { tokens: boolean; providers: boolean; cost: boolean }
export const providerNames: Record<ProviderID, string> = { codex: 'Codex', claude: 'Claude', cursor: 'Cursor', opencode: 'OpenCode' };
export const periodNames: Record<Period, string> = { today: 'Today', week: 'This Week', month: 'This Month' };
export function shareTotals(snapshot: UsageSnapshot, period: Period, providers: ProviderID[], now = new Date()) {
  return aggregateUsage(snapshot, period, providers, now);
}
export const shareHeading = (kind: ShareKind, period: Period) => `${period === 'today' ? 'Today’s' : period === 'week' ? 'This Week’s' : 'This Month’s'} ${kind === 'rank' ? 'Global Rank' : 'AI Usage'}`;
export function sharePeriodLabel(period: Period, utc: boolean, now: Date) {
  const anchor = utc ? new Date(now.getUTCFullYear(), now.getUTCMonth(), now.getUTCDate(), 12) : now;
  const [start] = periodBounds(period, anchor);
  const format = (date: Date) => date.toLocaleDateString('en-US', { month: 'short', day: 'numeric' });
  return `${format(start)}${period === 'today' ? '' : `–${format(anchor)}`} ${anchor.getFullYear()}${utc ? ' · UTC' : ''}`;
}
export function matchingRank(snapshot: UsageSnapshot, period: Period, now: Date) {
  const rank = snapshot.rank;
  if (!rank || snapshot.rankUnavailable || rank.period !== period || !Number.isSafeInteger(rank.position) || rank.position < 1 || !Number.isSafeInteger(rank.tokens) || rank.tokens < 0) return undefined;
  const updated = new Date(rank.updatedAt);
  if (!Number.isFinite(updated.getTime()) || updated.getTime() > now.getTime() + 5 * 60_000) return undefined;
  const start = new Date(Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), now.getUTCDate()));
  if (period === 'week') start.setUTCDate(start.getUTCDate() - (start.getUTCDay() + 6) % 7);
  if (period === 'month') start.setUTCDate(1);
  return updated >= start ? rank : undefined;
}
export function suggestedCaption(kind: ShareKind, period: Period, fields: ShareFields, totals: ReturnType<typeof shareTotals>, rank: UsageSnapshot['rank'], style: CaptionStyle = 'simple') {
  const providers = Object.entries(totals.providers).filter(([, tokens]) => tokens > 0).map(([id]) => providerNames[id as ProviderID]).join(' + ');
  const when = periodNames[period].toLowerCase();
  if (kind === 'rank' && !rank) return '';
  if (kind === 'rank' && rank && style !== 'simple') {
    const score = fields.tokens ? `\n${formatShareTokens(rank.tokens)} tokens tracked.` : '';
    return style === 'community'
      ? `Building alongside the @DevClashAI community.\nCurrently #${rank.position} ${when} on the token leaderboard.${score}\nWhat are you working on?`
      : `My spot ${when}: #${rank.position} on @DevClashAI.${score}\nWhere do you rank? Share your progress.`;
  }
  if (kind === 'rank') return rank ? `Currently #${rank.position} on the @DevClashAI token leaderboard ${when}.\n${fields.tokens ? `${formatShareTokens(rank.tokens)} tokens tracked. ` : ''}Where do you rank?` : '';
  const cost = fields.cost && totals.estimatedCost !== null ? `\n${formatCost(totals.estimatedCost, totals.unpricedTokens)} estimated API-rate value.` : '';
  if (style !== 'simple') {
    const toolkit = fields.providers && providers ? `\nMy toolkit: ${providers}.` : '';
    const activity = fields.tokens ? `${formatShareTokens(totals.total)} tokens tracked ${when}` : `Tracking my AI usage ${when}`;
    return style === 'community'
      ? `${activity} with @DevClashAI.${toolkit}${cost}\nWhat are you building with AI?`
      : `My AI usage check-in ${when}.${fields.tokens ? `\n${formatShareTokens(totals.total)} tokens tracked with @DevClashAI.` : '\nTracked with @DevClashAI.'}${toolkit}${cost}\nYour turn: share your AI toolkit.`;
  }
  const intro = fields.providers && providers ? `My AI toolkit ${when}: ${providers}.` : `My AI usage ${when}.`;
  return `${intro}\n${fields.tokens ? `${formatShareTokens(totals.total)} tokens tracked` : 'Tracking my usage'} with @DevClashAI.${fields.cost && totals.estimatedCost !== null ? `\n${formatCost(totals.estimatedCost, totals.unpricedTokens)} estimated API-rate value.` : ''}\nWhat are you building?`;
}
export function shareLink(kind: ShareKind, period: Period, now: Date): string {
  if (kind !== 'rank') return 'https://devclash.ai';
  const url = new URL('https://app.devclash.ai/rank');
  url.search = new URLSearchParams({ period: period === 'today' ? 'Today' : period === 'week' ? 'Week' : 'Month', date: now.toISOString().slice(0, 10), provider: 'All Providers', model: 'All Models', region: 'Global' }).toString();
  return url.href;
}
export const xDraftURL = (text: string) => `https://x.com/intent/post?${new URLSearchParams({ text })}`;
