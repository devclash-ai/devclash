import { Cloud, Monitor, Settings, Share2, Trophy, ChevronDown, ChevronRight, Triangle } from 'lucide-react';
import { useId, useRef, useState } from 'react';
import type { Period, Preferences, ProjectTotal, ProviderID, UsageSnapshot, UsageTotals } from '../lib/types';
import { enabledInOrder, providerNames } from '../lib/preferences';
import { formatCost, formatSpend, formatTokens } from '../lib/usage';
import { canRetryAuthorization } from '../lib/providerAuthorization';
import { providerProjects } from '../lib/projectUsage';
import { ProviderProjects } from './ProjectUsage';
import { EmptyState, Logo, PeriodControl, ProviderIcon, RefreshButton, Sparkline, WindowChrome } from './Common';
interface Props {
  preferences: Preferences; snapshot: UsageSnapshot; totals: UsageTotals; period: Period;
  theme: 'light' | 'dark'; connected?: boolean; busy?: boolean; setPeriod: (p: Period) => void;
  syncEnabled?: boolean; syncTone?: 'off' | 'pending' | 'error' | 'active'; cloudLabel?: string; accountInitial?: string;
  onCustomize: () => void; onRefresh: () => void; onAccount: () => void; onWidget: () => void;
  onShare: (element: HTMLElement) => void; onRank: () => void;
  onRetryAuthorization?: (id: ProviderID) => void; authorizationPending?: ProviderID | null;
}
export function ModelRows({ totals, compact = false, spend = false }: { totals: UsageTotals; compact?: boolean; spend?: boolean }) {
  return <div className={'model-list' + (compact ? ' compact' : '')}>{totals.models.slice(0, 5).map((model, index) =>
    <div className="model-row" key={model.model}><span className="model-name">{model.model}</span><strong>{formatTokens(model.tokens)}{!compact && ' tokens'}
      {spend && !compact && <small className="row-cost muted">{formatCost(model.estimatedCost, model.unpricedTokens)}</small>}</strong>
      <span className={'meter model-meter model-color-' + index}><i style={{ width: model.share + '%' }} /></span><span>{Math.round(model.share)}%</span></div>)}</div>;
}
export function Dashboard(props: Props) {
  const { preferences, snapshot, totals, period, theme, onCustomize, onRefresh, onAccount, onWidget, onShare, onRank } = props;
  const ref = useRef<HTMLElement>(null); const metrics = preferences.surfaces.dashboard;
  const connected = !!props.connected; const providers = enabledInOrder(preferences);
  return <section ref={ref} className={'dashboard app-panel theme-' + theme} aria-label="DevClash Dashboard">
    <WindowChrome />
    <header className="dashboard-header" data-tauri-drag-region><Logo theme={theme} /><div className="header-actions" data-no-share="true">
      <RefreshButton onClick={onRefresh} busy={props.busy} /><button className="button" onClick={onCustomize}><Settings size={19} />Customize</button>
    </div></header>
    <div className="dashboard-content"><PeriodControl period={period} setPeriod={props.setPeriod} />
      {(metrics.tokens || metrics.trend || metrics.spend) && <div className="usage-summary">
        {(metrics.tokens || metrics.spend) && <div className="summary-value">{metrics.tokens && <><span className="muted summary-label">Total Tokens</span><strong className="total-tokens">{totals.hasData ? formatTokens(totals.total) : 'N/A'}</strong>
          {totals.delta !== null && <span className={totals.delta >= 0 ? 'positive' : 'muted'}>{totals.delta > 0 ? '+' : ''}{totals.delta}% vs {period === 'today' ? 'yesterday' : 'previous period'}</span>}</>}
          {!metrics.tokens && metrics.spend && <span className="muted summary-label">Estimated Spend</span>}
          {metrics.spend && <><span className="muted estimated-spend">{formatSpend(totals)}</span>
            <span className="muted spend-note">{totals.unpricedTokens > 0 ? 'Price unavailable for ' + formatTokens(totals.unpricedTokens) + ' tokens' : totals.estimatedCost !== null ? 'API-Rate Value' : ''}</span></>}
        </div>}{metrics.trend && <Sparkline trend={totals.trend} labels />}
      </div>}
      {!totals.hasData && <EmptyState browser={snapshot.source === 'browser'} reading={snapshot.source === 'local' && !snapshot.fetchedAt} />}
      {metrics.rank && connected && snapshot.rank && <button className="rank-strip" onClick={onRank}><Trophy size={33} className="trophy" /><span><span className="rank-label">Rank Battle</span><strong>#{snapshot.rank.position} {snapshot.rank.period.charAt(0).toUpperCase() + snapshot.rank.period.slice(1)} · UTC</strong></span>
        {typeof snapshot.rank.movement === 'number' && snapshot.rank.movement !== 0 && <span className="rank-movement positive"><Triangle size={14} fill="currentColor" />Up {snapshot.rank.movement} Places</span>}<span className="button rank-link">View Leaderboard<ChevronRight size={16} /></span></button>}
      {providers.length > 0 && <section className={'providers-section' + (!metrics.limits ? ' no-limits' : '')}><div className="section-head"><h2>Providers</h2><span>{period === 'today' ? "Today's Tokens" : 'Tokens'}</span>{metrics.limits && <span>Quota Usage</span>}</div>
        <div className="provider-list">{providers.map(id => <ProviderRow key={id} id={id} period={period} projects={providerProjects(totals.projects, id, totals.providers[id])} tokens={totals.providers[id]} cost={totals.providerCosts[id]} unpricedTokens={totals.providerUnpricedTokens[id]} spend={metrics.spend} snapshot={snapshot} limits={metrics.limits} onRetryAuthorization={props.onRetryAuthorization} authorizationPending={props.authorizationPending} />)}</div>
      </section>}
      {!providers.length && <div className="empty-state"><strong>Choose What You Want to Track</strong><p>Enable a provider in Customize to show its usage.</p><button className="button" onClick={onCustomize}>Choose Providers</button></div>}
      {metrics.models && totals.models.length > 0 && <section className="models-section"><div className="section-head"><h2>Models</h2><span>{metrics.spend ? 'Tokens / Cost' : 'Usage'}</span><span>Share of Total</span></div><ModelRows totals={totals} spend={metrics.spend} /></section>}
      {snapshot.errors.length > 0 && <div className="source-notice" role="status">{snapshot.errors[0]}</div>}
      {metrics.spend && totals.unpricedModels.length > 0 && <div className="source-notice" role="status">Price Unavailable: {totals.unpricedModels.join(', ')}. Their tokens are included; their cost is excluded.</div>}
    </div>
    {connected && props.syncEnabled === false && <aside className="sync-invitation" data-no-share="true" aria-label="Enable Cloud Sync">
      <Cloud size={22} aria-hidden="true" /><div><strong>Turn Usage Into Credits</strong><p>Sync privately, then convert eligible tokens into Build Credits in your dashboard.</p></div>
      <button className="button primary" onClick={onAccount}>Enable Cloud Sync<ChevronRight size={15} /></button>
    </aside>}
    <footer className="dashboard-footer"><div className="local-status"><button className="icon-button" aria-label="Open Floating Widget" onClick={onWidget}><Monitor size={24} /></button><button className={"sync-status-button sync-tone-" + (connected ? props.syncTone ?? 'pending' : 'off')} onClick={onAccount} aria-label={connected ? `Manage Cloud Sync: ${props.cloudLabel ?? 'Cloud Sync'}` : 'Sign In to Enable Cloud Sync'}><i className="status-dot" /><span>{connected ? <><Cloud size={17} />{props.cloudLabel ?? 'Cloud Sync'}</> : 'Local Only'}</span></button></div>
      <div className="footer-actions" data-no-share="true">{connected ? <button className="avatar" aria-label="Open Account" onClick={onAccount}>{props.accountInitial ?? 'A'}</button> : <button className="button account-button" onClick={onAccount}>Sign In to Save History</button>}
        <span className="footer-divider" /><button className="button share-button" disabled={!totals.hasData} onClick={() => ref.current && onShare(ref.current)}><Share2 size={20} />Share</button></div>
    </footer>
  </section>;
}
function ProviderReason({ id, reason, usage = false, onRetry, pending }: { id: ProviderID; reason: string; usage?: boolean; onRetry?: (id: ProviderID) => void; pending?: ProviderID | null }) {
  const retry = !!onRetry && canRetryAuthorization(id, reason);
  return <span className={(usage ? 'provider-error ' : '') + 'muted quota-reason'} role={usage ? 'status' : undefined}>
    {retry ? 'Keychain access is required.' : reason}
    {retry && <button type="button" className="text-button provider-retry" disabled={!!pending} aria-label={'Retry ' + providerNames[id] + ' Authorization'} onClick={() => onRetry(id)}>{pending === id ? 'Authorizing…' : 'Retry'}</button>}
  </span>;
}
function ProviderRow({ id, period, projects, tokens, cost, unpricedTokens, spend, snapshot, limits, onRetryAuthorization, authorizationPending }: { id: ProviderID; period: Period; projects: ProjectTotal[]; tokens: number; cost: number | null; unpricedTokens: number; spend: boolean; snapshot: UsageSnapshot; limits: boolean; onRetryAuthorization?: (id: ProviderID) => void; authorizationPending?: ProviderID | null }) {
  const [expanded, setExpanded] = useState(false); const contentID = useId();
  const quota = snapshot.quotas[id]; const available = snapshot.availableProviders.includes(id);
  const usageError = snapshot.providerErrors?.[id]; const quotaError = snapshot.quotaErrors?.[id];
  return <div className={'provider-row' + (!limits ? ' no-limits' : '') + (expanded ? ' expanded' : '')}><button type="button" className="provider-name provider-project-toggle" aria-label={(expanded ? 'Collapse ' : 'Expand ') + providerNames[id] + ' Project Usage'} aria-expanded={expanded} aria-controls={contentID} onClick={() => setExpanded(value => !value)}><ProviderIcon id={id} /><strong>{providerNames[id]}</strong><ChevronDown size={14} className="provider-project-chevron" /></button><div className="provider-tokens"><strong>{available ? formatTokens(tokens) + ' tokens' : 'N/A'}</strong>
    {spend && <span className="row-cost muted">{formatCost(cost, unpricedTokens)}</span>}</div>
    {limits && <div className="quota"><div><span className="muted">{quota?.label ?? 'Quota'}</span><span>{quota ? quota.used + '% used' : 'Unavailable'}</span></div>
      <span className={'meter' + (!quota ? ' unknown-meter' : '')}><i style={{ width: Math.min(100, quota?.used ?? 0) + '%' }} /></span>
      {quota?.resetLabel && <span className="muted reset-label">{quota.resetLabel}</span>}
      {!quota && quotaError && quotaError !== usageError && <ProviderReason id={id} reason={quotaError} onRetry={onRetryAuthorization} pending={authorizationPending} />}
    </div>}
    {usageError && <ProviderReason id={id} reason={usageError} usage onRetry={limits && !quota && quotaError !== usageError && canRetryAuthorization(id, quotaError) ? undefined : onRetryAuthorization} pending={authorizationPending} />}
    {expanded && <ProviderProjects key={period} contentID={contentID} provider={id} projects={projects} total={tokens} spend={spend} />}</div>;
}
