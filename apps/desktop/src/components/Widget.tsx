import { ChevronRight, Share2, Triangle, Trophy, X } from 'lucide-react';
import { useRef } from 'react';
import { startWidgetDrag } from '../lib/desktop';
import type { CSSProperties, PointerEvent } from 'react';
import type { Period, Preferences, UsageSnapshot, UsageTotals, WidgetLayout } from '../lib/types';
import { enabledInOrder, providerNames, widgetWidths } from '../lib/preferences';
import { formatAge, formatSpend, formatTokens } from '../lib/usage';
import { BrandMark, Logo, PeriodControl, ProviderIcon, SettingsButton, Sparkline } from './Common';
import { ModelRows } from './Dashboard';
interface Props {
  preferences: Preferences; snapshot: UsageSnapshot; totals: UsageTotals; theme: 'light' | 'dark';
  layout?: WidgetLayout; setPeriod: (p: Period) => void; onCustomize: () => void;
  onShare: (element: HTMLElement) => void; onRank: () => void; preview?: boolean;
  onHide?: () => void; windowDraggable?: boolean; onDragError?: () => void;
}
export function FloatingWidget(props: Props) {
  const { preferences: prefs, snapshot, totals, theme, setPeriod, onCustomize, onShare, onRank } = props;
  const ref = useRef<HTMLElement>(null); const layout = props.layout ?? prefs.widgetLayout; const metrics = prefs.surfaces.widget;
  const gesture = useRef<ReturnType<typeof startWidgetDrag> | null>(null);
  const number = totals.hasData ? formatTokens(totals.total) : 'N/A';
  const rank = metrics.rank ? snapshot.rank : undefined;
  const style = { width: widgetWidths[prefs.widgetSize], '--widget-background-opacity': prefs.widgetTransparency ? prefs.widgetOpacity / 100 : 1 } as CSSProperties;
  const drag = (event: PointerEvent<HTMLElement>) => {
    if (!props.windowDraggable || event.button !== 0 || !(event.target instanceof Element) || event.target.closest('button, a, select, input, textarea, label, [role="button"]')) return;
    event.preventDefault();
    event.currentTarget.setPointerCapture(event.pointerId);
    gesture.current = startWidgetDrag(event.screenX, event.screenY, () => { console.error('Widget drag could not start'); props.onDragError?.(); });
  };
  const move = (event: PointerEvent<HTMLElement>) => { gesture.current?.move({ x: event.screenX, y: event.screenY }); };
  const finish = (event: PointerEvent<HTMLElement>) => { move(event); gesture.current = null; };
  return <section ref={ref} style={style} onPointerDown={drag} onPointerMove={move} onPointerUp={finish} onPointerCancel={() => { gesture.current = null; }} className={'floating-widget theme-' + theme + ' widget-' + layout + ' size-' + prefs.widgetSize + (props.preview ? ' is-preview' : '') + (props.windowDraggable ? ' is-draggable' : '')} aria-label={layout + ' Floating Widget'}>
    <header className="widget-header">{layout === 'minimal' ? <><BrandMark theme={theme} className="widget-logo" alt="DevClash" /><span className="muted">{prefs.period.charAt(0).toUpperCase() + prefs.period.slice(1)}</span></> :
      layout === 'usage' ? prefs.widgetSize === 'compact' ? <BrandMark theme={theme} className="widget-logo" alt="DevClash" /> : <Logo small theme={theme} /> : <span className="widget-title"><BrandMark theme={theme} />{layout === 'rank' ? 'Rank Battle' : 'Model Usage'}</span>}
      {layout !== 'minimal' && <PeriodControl period={prefs.period} setPeriod={setPeriod} small />}
      <span className="widget-window-actions" data-no-share="true" data-tauri-drag-region="false">
        <SettingsButton onClick={onCustomize} />
        <button className="icon-button widget-close" aria-label="Hide Widget" disabled={!props.onHide} onClick={props.onHide}><X size={14} /></button>
      </span>
    </header>
    {layout === 'rank' ? <><div className="widget-rank-stat"><Trophy size={57} strokeWidth={1.5} /><div><strong>{rank ? '#' + rank.position : snapshot.ranked ? 'N/A' : 'Join'}</strong><span>{rank ? rank.period.charAt(0).toUpperCase() + rank.period.slice(1) + ' · UTC' : snapshot.ranked ? snapshot.rankUnavailable ? 'Rank Unavailable Offline' : 'Awaiting Eligible Usage' : 'Rank Battle'}</span>
      {rank && typeof rank.movement === 'number' && rank.movement !== 0 && <p className="positive"><Triangle size={14} fill="currentColor" />Up {rank.movement} Places</p>}</div></div>
      <div className="rank-widget-tokens">{metrics.tokens && <span><strong>{rank ? formatTokens(rank.tokens) : 'N/A'}</strong><span className="muted">Ranked Tokens · UTC</span></span>}
        {rank && metrics.rank && rank.podiums > 0 && <span className="podium-chip"><Trophy size={17} />Podium ×{rank.podiums}</span>}</div>
      <button className="widget-action" onClick={onRank}>{rank || snapshot.ranked ? 'View Leaderboard' : 'Learn About Rank Battle'}<ChevronRight size={20} /></button></> :
    <><div className="widget-summary">{metrics.tokens && <div><strong>{number}</strong><span className="muted">{layout === 'minimal' ? 'Tokens' : layout === 'models' ? 'Tokens' : 'Total Tokens'}</span></div>}
      {layout === 'minimal' && totals.delta !== null && <span className={(totals.delta >= 0 ? 'positive' : 'muted') + ' widget-delta'}><Triangle size={14} fill="currentColor" style={{ transform: totals.delta < 0 ? 'rotate(180deg)' : undefined }} />{totals.delta > 0 ? '+' : ''}{totals.delta}%</span>}
      {metrics.trend && layout === 'minimal' && <Sparkline trend={totals.trend} />}
      {metrics.trend && layout === 'usage' && <Sparkline trend={totals.trend} />}
    </div>
    {layout === 'usage' && <div className="widget-providers">{enabledInOrder(prefs).map(id => <div className="widget-provider" key={id}><ProviderIcon id={id} plain /><strong>{providerNames[id]}</strong>
      <span className="widget-provider-values"><strong>{snapshot.availableProviders.includes(id) ? formatTokens(totals.providers[id]) : 'N/A'}</strong></span>
      {metrics.limits && <div className="widget-quota"><span className="muted">{snapshot.quotas[id]?.label ?? 'Quota'} <span>{snapshot.quotas[id] ? snapshot.quotas[id]!.used + '%' : 'N/A'}</span></span></div>}
    </div>)}</div>}
    {layout === 'models' && metrics.models && <ModelRows totals={totals} compact />}
    {layout === 'models' && !metrics.models && <p className="widget-hidden-metric muted">Model Breakdown Is Hidden</p>}
    {layout === 'usage' && metrics.models && totals.models.length > 0 && <ModelRows totals={totals} compact />}
    {layout === 'usage' && metrics.rank && rank && <button className="widget-inline-rank" onClick={onRank}><span className="rank-widget-icon"><BrandMark theme={theme} /></span><strong>#{rank.position} {rank.period.charAt(0).toUpperCase() + rank.period.slice(1)} · UTC</strong>{typeof rank.movement === 'number' && rank.movement !== 0 && <span className="positive"><Triangle size={12} fill="currentColor" />Up {rank.movement} Places</span>}</button>}
    {layout === 'models' ? <button className="widget-action" disabled={!totals.hasData} onClick={() => ref.current && onShare(ref.current)}><Share2 size={21} />Share</button> :
      layout === 'usage' && <footer className="widget-footer">{snapshot.source === 'preview' ? 'Updated Just Now' : formatAge(snapshot.fetchedAt)}</footer>}
    </>}
    {metrics.spend && <div className="widget-spend muted"><span>{formatSpend(totals)}</span>
      {totals.unpricedTokens > 0 && <small>Price unavailable: {formatTokens(totals.unpricedTokens)} tokens</small>}</div>}
  </section>;
}
