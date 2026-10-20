import type { Ref } from 'react';
import { Logo, ProviderIcon, Sparkline } from './Common';
import type { Period, ProviderID, UsageSnapshot, UsageTotals } from '../lib/types';
import { formatCost } from '../lib/usage';
import { formatShareTokens, periodNames, providerNames, shareHeading, type ShareFields, type ShareKind } from '../lib/share';

export function ShareCard({ cardRef, kind, period, date, fields, totals, rank, providers }: {
  cardRef?: Ref<HTMLDivElement>; kind: ShareKind; period: Period; date: string;
  fields: ShareFields; totals: UsageTotals; rank: UsageSnapshot['rank']; providers: ProviderID[];
}) {
  const visibleProviders = providers.filter(id => totals.providers[id] > 0);
  const position = rank?.position.toLocaleString('en-US') ?? '';
  return <div ref={cardRef} className={`share-card share-card-${kind}${kind === 'usage' && fields.cost ? ' share-card-with-cost' : ''}${fields.providers && visibleProviders.length > 2 ? ' share-card-dense' : ''}`}>
    <header className="share-card-header"><Logo small theme="dark" /><span>{date.replace(' · UTC', '')}</span></header>
    {kind === 'usage' ? <>
      <div className="share-usage-main">
        <div className="share-hero"><span className="share-eyebrow">{shareHeading(kind, period)}</span>
          {fields.tokens ? <><strong style={{ fontSize: `${Math.min(12, 60 / formatShareTokens(totals.total).length)}cqw` }}>{formatShareTokens(totals.total)}</strong><span className="share-hero-label">Tokens Used</span></> : <strong className="share-without-tokens">Building With AI</strong>}
        </div>
        {fields.tokens && <div className="share-trend"><Sparkline trend={totals.trend} filled labels /></div>}
      </div>
      <footer className="share-card-bottom">
        <div className="share-card-providers" style={{ gridTemplateColumns: `repeat(${fields.providers ? Math.max(1, visibleProviders.length) : 1}, minmax(0, 1fr))` }}>{fields.providers && visibleProviders.map(id => <div key={id} className="share-provider"><ProviderIcon id={id} /><div><span>{providerNames[id]}</span>{fields.tokens && <b>{formatShareTokens(totals.providers[id])}</b>}</div></div>)}
        </div>
        {fields.cost && <div className="share-cost"><span>Estimated Cost</span><b>{formatCost(totals.estimatedCost, totals.unpricedTokens)}</b></div>}
      </footer>
    </> : <>
      <div className="share-rank-main"><div className="share-rank-hero"><span className="share-eyebrow">{shareHeading(kind, period)}</span><strong style={{ fontSize: `${Math.min(19, 80 / (position.length + 1))}cqw` }}>#{position}</strong><span className="share-hero-label">On The Leaderboard</span></div>
        <div className="share-rank-details">{fields.tokens && <div><span className="share-eyebrow">Ranked Tokens</span><b>{formatShareTokens(rank?.tokens ?? 0)}</b></div>}<div><span className="share-eyebrow">Period</span><strong>{periodNames[period]} · UTC</strong></div></div>
      </div>
      <footer className="share-card-bottom"><span className="share-motto">Build. Track. Climb.</span></footer>
    </>}
  </div>;
}
