import type { MenuBarItem, Preferences, UsageSnapshot, UsageTotals } from './types';
import { enabledInOrder, providerNames } from './preferences.ts';
import { aggregateUsage, formatCost, formatSpend, formatTokens } from './usage.ts';

export interface TrayContent { label: string; rows: string[]; tooltip: string }
export function trayContent(prefs: Preferences, snapshot: UsageSnapshot, totals: UsageTotals): TrayContent {
  const metrics = prefs.surfaces.menubar;
  const ids = enabledInOrder(prefs).filter(id => prefs.menuBarProvider === 'all' || id === prefs.menuBarProvider);
  const scoped = prefs.menuBarProvider === 'all' ? totals : aggregateUsage(snapshot, prefs.period, ids);
  const quotas = ids.flatMap(id => snapshot.quotas[id] ? [snapshot.quotas[id]!.used] : []);
  const tokenLabel = scoped.hasData ? formatTokens(scoped.total) : 'N/A';
  const quotaLabel = quotas.length ? Math.max(...quotas) + '%' : 'N/A';
  const spendLabel = scoped.estimatedCost === null ? 'N/A' : formatCost(scoped.estimatedCost) + (scoped.unpricedTokens > 0 ? '*' : '');
  const values: Record<MenuBarItem, string> = { tokens: tokenLabel, limits: quotaLabel, spend: spendLabel };
  const labels: Record<MenuBarItem, string> = { tokens: 'Tokens', limits: 'Quota', spend: 'Est.' };
  const pinned: MenuBarItem[] = prefs.menuBarView === 'icon' ? [] : prefs.menuBarView === 'custom' ? prefs.menuBarItems :
    prefs.menuBarView === 'tokens-limits' ? ['tokens', 'limits'] : [prefs.menuBarView];
  const label = pinned.map(item => (prefs.menuBarLabels ? labels[item] + ' ' : item === 'spend' && scoped.estimatedCost !== null ? '~' : '') + values[item]).join(' · ');
  const period = prefs.period.charAt(0).toUpperCase() + prefs.period.slice(1);
  const source = prefs.menuBarProvider === 'all' ? 'All Providers' : providerNames[prefs.menuBarProvider];
  const rows: string[] = [];
  if (metrics.tokens) rows.push(period + ' Tokens: ' + tokenLabel);
  const providerRows = () => ids.forEach(id => {
    const parts: string[] = [];
    if (metrics.tokens) parts.push((snapshot.availableProviders.includes(id) ? formatTokens(scoped.providers[id]) : 'N/A') + ' Tokens');
    if (metrics.limits) parts.push(snapshot.quotas[id] ? snapshot.quotas[id]!.label + ' ' + snapshot.quotas[id]!.used + '% Used' : 'Quota Unavailable');
    if (metrics.spend) parts.push(formatCost(scoped.providerCosts[id], scoped.providerUnpricedTokens[id]) + ' Estimated');
    if (parts.length) rows.push(providerNames[id] + ': ' + parts.join(' · '));
  });
  const modelRows = () => { if (metrics.models) scoped.models.slice(0, prefs.menuBarDetails === 'models' ? 5 : 3).forEach(model => {
    const name = model.model.length > 48 ? model.model.slice(0, 47) + '…' : model.model;
    rows.push(name + ': ' + formatTokens(model.tokens) + (metrics.spend ? ' · ' + formatCost(model.estimatedCost, model.unpricedTokens) : ''));
  }); };
  if (prefs.menuBarDetails === 'models') modelRows();
  if (prefs.menuBarDetails === 'providers') providerRows();
  else if (metrics.limits) ids.forEach(id => rows.push(providerNames[id] + ': ' + (snapshot.quotas[id] ? snapshot.quotas[id]!.label + ' ' + snapshot.quotas[id]!.used + '% Used' : 'Quota Unavailable')));
  if (prefs.menuBarDetails !== 'models') modelRows();
  if (metrics.rank) rows.push(snapshot.rank ? 'Today’s Rank: #' + snapshot.rank.position : 'Rank Battle: Cloud Service Required');
  if (metrics.spend) rows.push('Estimated Spend: ' + formatSpend(scoped));
  if ((metrics.spend || pinned.includes('spend')) && scoped.unpricedTokens > 0) rows.push('Price Unavailable: ' + formatTokens(scoped.unpricedTokens) + ' Tokens');
  if (metrics.trend) rows.push('7-Day Tokens: ' + formatTokens(scoped.trend.reduce((sum, day) => sum + day.tokens, 0)));
  const tooltipValues = pinned.map(item => item === 'spend' ? formatSpend(scoped) : item === 'tokens' ? tokenLabel + ' Tokens' : quotaLabel + ' Quota Used');
  const tooltip = ['DevClash · ' + period + ' · ' + source, ...tooltipValues].join('\n');
  return { label, rows, tooltip };
}
