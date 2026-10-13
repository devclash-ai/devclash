import type { Period, ProviderID, UsageEvent, UsageSnapshot, UsageTotals } from './types';
import { projectTotals } from './projectUsage.ts';
const DAY = 86400000;
function startOfDay(date: Date): Date { const d = new Date(date); d.setHours(0, 0, 0, 0); return d; }
export function periodBounds(period: Period, now = new Date()): [Date, Date, Date] {
  const end = new Date(startOfDay(now)); end.setDate(end.getDate() + 1);
  const start = startOfDay(now);
  if (period === 'week') start.setDate(start.getDate() - (start.getDay() + 6) % 7);
  if (period === 'month') start.setDate(1);
  const previous = new Date(start);
  if (period === 'today') previous.setDate(previous.getDate() - 1);
  if (period === 'week') previous.setDate(previous.getDate() - 7);
  if (period === 'month') previous.setMonth(previous.getMonth() - 1);
  return [start, end, previous];
}
export function aggregateUsage(snapshot: UsageSnapshot, period: Period, providers: ProviderID[], now = new Date()): UsageTotals {
  const [start, end, previousStart] = periodBounds(period, now);
  const accepted = snapshot.events.filter(event => providers.includes(event.provider) && Number.isSafeInteger(event.tokens) && event.tokens >= 0);
  const dedup = new Map<string, UsageEvent>();
  for (const event of accepted) {
    const key = event.provider + ':' + event.id;
    if (!dedup.has(key) || dedup.get(key)!.tokens < event.tokens) dedup.set(key, event);
  }
  const events = [...dedup.values()];
  const current = events.filter(event => new Date(event.timestamp) >= start && new Date(event.timestamp) < end);
  const previous = events.filter(event => new Date(event.timestamp) >= previousStart && new Date(event.timestamp) < start).reduce((n, event) => n + event.tokens, 0);
  const sums: Record<ProviderID, number> = { claude: 0, codex: 0, cursor: 0, opencode: 0 };
  const models = new Map<string, { tokens: number; cost: number | null; unpricedTokens: number }>();
  const providerCosts: Record<ProviderID, number | null> = { claude: null, codex: null, cursor: null, opencode: null };
  const providerUnpricedTokens: Record<ProviderID, number> = { claude: 0, codex: 0, cursor: 0, opencode: 0 };
  const unpricedModels = new Set<string>();
  let cost = 0; let priced = 0; let unpricedTokens = 0;
  for (const event of current) {
    sums[event.provider] += event.tokens;
    const modelName = event.model || 'Unknown Model';
    const model = models.get(modelName) ?? { tokens: 0, cost: null, unpricedTokens: 0 };
    const validCost = event.costUSD != null && Number.isFinite(event.costUSD) && event.costUSD >= 0;
    const missing = validCost ? Number.isSafeInteger(event.unpricedTokens) && event.unpricedTokens! >= 0 ? Math.min(event.unpricedTokens!, event.tokens) : 0 : event.tokens;
    model.tokens += event.tokens; model.unpricedTokens += missing;
    if (validCost) {
      cost += event.costUSD!; priced++; model.cost = (model.cost ?? 0) + event.costUSD!;
      providerCosts[event.provider] = (providerCosts[event.provider] ?? 0) + event.costUSD!;
    }
    if (missing > 0) { unpricedModels.add(modelName); unpricedTokens += missing; providerUnpricedTokens[event.provider] += missing; }
    models.set(modelName, model);
  }
  const total = current.reduce((n, event) => n + event.tokens, 0);
  const trend = Array.from({ length: 7 }, (_, index) => {
    const day = startOfDay(now); day.setDate(day.getDate() - 6 + index);
    const next = new Date(day); next.setDate(next.getDate() + 1);
    return { label: day.toLocaleDateString('en-US', { month: 'short', day: 'numeric' }),
      tokens: events.filter(event => new Date(event.timestamp) >= day && new Date(event.timestamp) < next).reduce((n, event) => n + event.tokens, 0) };
  });
  return { total, previous, delta: previous ? Math.round((total - previous) / previous * 100) : null,
    providers: sums, projects: projectTotals(current, total), models: [...models].map(([model, value]) => ({ model, tokens: value.tokens, share: total ? value.tokens / total * 100 : 0,
      estimatedCost: value.cost, unpricedTokens: value.unpricedTokens })).sort((a, b) => b.tokens - a.tokens),
    trend, estimatedCost: priced > 0 ? cost : null, providerCosts, providerUnpricedTokens, unpricedTokens,
    unpricedModels: [...unpricedModels].sort(), hasData: current.length > 0 };
}
export function formatCost(cost: number | null, unpricedTokens = 0): string {
  return cost === null ? 'N/A' : cost.toLocaleString('en-US', { style: 'currency', currency: 'USD' }) + (unpricedTokens > 0 ? ' (Partial)' : '');
}
export function formatSpend(totals: UsageTotals): string {
  return totals.estimatedCost === null ? 'Estimate Unavailable' : formatCost(totals.estimatedCost) + (totals.unpricedTokens > 0 ? ' Partial Estimate' : ' Estimated');
}
export function formatTokens(value: number): string {
  if (value >= 1000000000) return (value / 1000000000).toFixed(2).replace(/0+$/, '').replace(/\.$/, '') + 'B';
  if (value >= 1000000) return (value / 1000000).toFixed(2).replace(/0+$/, '').replace(/\.$/, '') + 'M';
  if (value >= 1000) return (value / 1000).toFixed(value >= 100000 ? 0 : 1).replace(/\.0$/, '') + 'K';
  return value.toLocaleString('en-US');
}
export function formatAge(date: string | null): string {
  if (!date) return 'Waiting for Usage';
  if (!Number.isFinite(new Date(date).getTime())) return 'Waiting for Usage';
  const minutes = Math.max(0, Math.floor((Date.now() - new Date(date).getTime()) / 60000));
  return minutes < 1 ? 'Updated Just Now' : 'Updated ' + minutes + 'm Ago';
}
export function demoSnapshot(now = new Date()): UsageSnapshot {
  const today = startOfDay(now).getTime() + 12 * 3600000;
  const events: UsageEvent[] = [];
  const values: Record<ProviderID, number> = { claude: 1120000, codex: 840000, cursor: 460000, opencode: 420000 };
  const names: Record<ProviderID, string> = { claude: 'Sonnet', codex: 'GPT Codex', cursor: 'Composer', opencode: 'GLM-5.3' };
  const history = [1320000, 1670000, 1510000, 1980000, 1730000, 1950000, 2420000];
  history.forEach((total, day) => (Object.keys(values) as ProviderID[]).forEach(provider => {
    const tokens = day === 6 ? values[provider] : Math.round(total * values[provider] / 2420000);
    const project = provider === 'cursor' ? undefined : provider === 'codex'
      ? { id: 'demo-devclash', name: 'DevClash', directory: '/demo/projects/devclash' }
      : { id: 'demo-atlas', name: 'Atlas', directory: '/demo/projects/atlas' };
    events.push({ id: 'preview-' + day + '-' + provider, provider, model: names[provider],
      timestamp: new Date(today - (6 - day) * DAY).toISOString(), tokens, costUSD: tokens / 1000000 * 7.727, project });
  }));
  return { events, source: 'preview', fetchedAt: now.toISOString(), errors: [], availableProviders: ['claude', 'codex', 'cursor', 'opencode'],
    quotas: { claude: { label: 'Session', used: 64, resetLabel: 'Resets in 1h 42m' },
      codex: { label: 'Session', used: 38, resetLabel: 'Resets in 2h 18m' }, cursor: { label: 'Plan', used: 71 }, opencode: { label: 'Go Session', used: 26, resetLabel: 'Resets in 3h 10m' } },
    rank: { position: 18, movement: 3, podiums: 2, updatedAt: now.toISOString(), period: 'today', tokens: 2420000 } };
}
export function emptySnapshot(): UsageSnapshot { return { events: [], source: 'browser', fetchedAt: null, errors: [], quotas: {}, availableProviders: [] }; }
