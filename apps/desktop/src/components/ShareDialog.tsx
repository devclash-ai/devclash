import { CalendarDays, Check, ChevronDown, Copy, Download, ExternalLink, X } from 'lucide-react';
import { useEffect, useMemo, useRef, useState } from 'react';
import { native } from '../lib/desktop';
import { ShareCard } from './ShareCard';
import type { Period, ProviderID, UsageSnapshot } from '../lib/types';
import { captionStyles, type CaptionStyle, matchingRank, periodNames, sharePeriodLabel, shareTotals, suggestedCaption, type ShareFields, type ShareKind } from '../lib/share';
import { cardBlob, copyCard, copyPost, openXDraft, saveCard, readShareRank } from '../lib/shareActions';

export function ShareDialog({ snapshot, initialPeriod, providers, theme, onClose }: {
  snapshot: UsageSnapshot; initialPeriod: Period; providers: ProviderID[]; theme: 'light' | 'dark'; onClose: () => void;
}) {
  // Freeze the source while editing so background refreshes cannot change a prepared post.
  const [source] = useState(snapshot); const [now] = useState(() => new Date());
  const [kind, setKind] = useState<ShareKind>('usage'); const [period, setPeriod] = useState(initialPeriod);
  const [fields, setFields] = useState<ShareFields>({ tokens: true, providers: true, cost: false });
  const [captionStyle, setCaptionStyle] = useState<CaptionStyle>('simple');
  const [caption, setCaption] = useState<string | null>(null); const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState(''); const [error, setError] = useState('');
  const panel = useRef<HTMLElement>(null); const card = useRef<HTMLDivElement>(null);
  const totals = useMemo(() => shareTotals(source, period, providers, now), [source, period, providers, now]);
  const [loadedRank, setLoadedRank] = useState<UsageSnapshot['rank']>();
  const [rankLoading, setRankLoading] = useState(false);
  const [rankError, setRankError] = useState('');
  const [rankRetry, setRankRetry] = useState(0);
  useEffect(() => {
    if (kind !== 'rank' || !native || !snapshot.ranked) return;
    let active = true; setLoadedRank(undefined); setRankLoading(true); setRankError('');
    readShareRank(period).then(value => { if (active) setLoadedRank(value); }).catch(reason => {
      console.error('Share rank could not be loaded', reason);
      if (active) setRankError(typeof reason === 'string' ? reason : reason instanceof Error ? reason.message : 'Your rank could not be loaded. Try again.');
    }).finally(() => { if (active) setRankLoading(false); });
    return () => { active = false; };
  }, [kind, period, snapshot.ranked, rankRetry]);
  const rank = matchingRank(native ? { ...snapshot, rank: snapshot.ranked ? loadedRank : undefined, rankUnavailable: !!rankError } : snapshot, period, new Date());
  const unavailable = kind === 'rank' ? rankLoading || !rank : !totals.hasData;
  const text = caption ?? suggestedCaption(kind, period, fields, totals, rank, captionStyle);
  const post = text.trim();
  const pasteShortcut = navigator.platform.toLowerCase().includes('mac') ? '⌘V' : 'Ctrl+V';
  const date = sharePeriodLabel(period, kind === 'rank', now);
  const ready = !unavailable && !!text.trim() && text.length <= 2000 && !busy;
  useEffect(() => {
    const previous = document.activeElement as HTMLElement | null;
    panel.current?.querySelector<HTMLButtonElement>('button')?.focus();
    const root = document.getElementById('root'); const siblings = [...(root?.children ?? [])].filter(node => node instanceof HTMLElement && !node.contains(panel.current)) as HTMLElement[];
    const states = siblings.map(node => node.inert); siblings.forEach(node => { node.inert = true; });
    return () => { siblings.forEach((node, index) => { node.inert = states[index]; }); previous?.focus(); };
  }, []);
  const reset = () => { setCaption(null); setMessage(''); setError(''); };
  const action = async (type: 'x' | 'text' | 'save' | 'image') => {
    if (!ready) return; setBusy(true); setError(''); setMessage('');
    try {
      if (type === 'text') { await copyPost(post); setMessage('Post text copied.'); }
      else if (type === 'x') {
        if (!card.current) throw new Error('The share preview is unavailable. Try again.');
        try { await copyCard(await cardBlob(card.current)); }
        catch (reason) { console.error('Share card copy failed', reason); throw new Error('The card could not be copied. Save the image and attach it in X, or try Copy Card again.'); }
        await openXDraft(post);
        setMessage(`Card copied. In X, press ${pasteShortcut} in the post field to attach it, then review and post.`);
      } else {
        if (!card.current) throw new Error('The share preview is unavailable. Try again.');
        const blob = await cardBlob(card.current);
        if (type === 'save') setMessage(`Saved to Downloads: ${await saveCard(blob)}`);
        else { await copyCard(blob); setMessage('Card copied. Paste it into your post.'); }
      }
    } catch (reason) { console.error('Sharing failed', reason); setError(typeof reason === 'string' ? reason : reason instanceof Error ? reason.message : 'Sharing could not be completed. Try again.'); }
    finally { setBusy(false); }
  };
  return <div className="dialog-backdrop" onClick={() => !busy && onClose()}>
    <section ref={panel} className={`cloud-dialog cloud-account-dialog share-dialog theme-${theme}`} role="dialog" aria-modal="true" aria-labelledby="share-title" onClick={event => event.stopPropagation()} onKeyDown={event => {
      if (event.key !== 'Tab') return;
      const items = [...(panel.current?.querySelectorAll<HTMLElement>('button:not(:disabled), select:not(:disabled), input:not(:disabled), textarea:not(:disabled)') ?? [])];
      const first = items[0], last = items.at(-1);
      if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last?.focus(); }
      else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first?.focus(); }
    }}>
      <button className="icon-button dialog-close" aria-label="Close Share Dialog" onClick={onClose}><X size={22} /></button>
      <h1 id="share-title">Share Your Build</h1><p className="share-subtitle">Choose what you share.</p>
      <div className="share-tabs" aria-label="Share Content">{(['usage', 'rank'] as ShareKind[]).map(value => <button key={value} aria-pressed={kind === value} onClick={() => { setKind(value); reset(); }}>{value === 'usage' ? 'Usage' : 'Rank'}</button>)}</div>
      <label className="share-field">Period<span className="share-select-wrap share-period-select"><CalendarDays className="share-calendar" size={17} aria-hidden="true" /><select value={period} onChange={event => { const next = event.target.value as Period; setPeriod(next); reset(); }}>{Object.entries(periodNames).map(([value, name]) => <option key={value} value={value}>{name} · {now.toLocaleDateString('en-US', { month: 'short', day: 'numeric', year: 'numeric' })}</option>)}</select><ChevronDown size={15} aria-hidden="true" /></span></label>
      {kind === 'rank' && rankError && <p className="cloud-error" role="alert">{rankError} <button className="text-button" onClick={() => setRankRetry(value => value + 1)}>Retry</button></p>}
      {unavailable ? <p className="share-empty" role="status">{kind === 'rank' ? rankLoading ? 'Loading Your Rank…' : 'No current public rank is available for this period. Join the leaderboard and sync, or share your usage instead.' : 'No measured usage is available for this period.'}</p> : <>
      <fieldset className="share-caption-options"><legend>Post Caption</legend><div role="group" aria-label="Caption Style">{Object.entries(captionStyles).map(([value, label]) => <button key={value} aria-pressed={captionStyle === value} onClick={() => { setCaptionStyle(value as CaptionStyle); reset(); }}>{label}</button>)}</div></fieldset>
      <label className="share-field"><span className="share-caption-label">Edit Caption</span><textarea aria-label="Post Caption" rows={3} maxLength={2000} value={text} onChange={event => setCaption(event.target.value)} /></label>
      {caption !== null && <button className="text-button share-reset" onClick={() => setCaption(null)}>Reset Caption</button>}
      <fieldset className="share-fields"><legend>Show On Card</legend>{(['tokens', ...(kind === 'rank' ? [] : ['providers', 'cost'])] as (keyof ShareFields)[]).map(key => <label key={key}><input type="checkbox" checked={fields[key]} onChange={() => { setFields({ ...fields, [key]: !fields[key] }); reset(); }} /><span className="share-check" aria-hidden="true">{fields[key] && <Check size={13} />}</span>{key === 'tokens' ? 'Tokens' : key === 'providers' ? 'Providers' : 'Cost'}</label>)}</fieldset>
      <div className="share-preview-label"><span>Card Preview</span>{source.source === 'preview' && <span>Sample Data</span>}</div>
      <div className="share-card-container"><ShareCard cardRef={card} kind={kind} period={period} date={date} fields={fields} totals={totals} rank={rank} providers={providers} /></div></>}

      <div className="share-footer"><button className="button primary" disabled={!ready} onClick={() => void action('x')}>{busy ? 'Preparing…' : 'Copy Card & Open X'}<ExternalLink size={16} /></button>
        <div className="share-secondary"><button className="button" disabled={!ready} onClick={() => void action('text')}><Copy size={15} />Copy Text</button><button className="button" disabled={!ready} onClick={() => void action('save')}><Download size={15} />Save Image</button></div>
        <p className="share-paste-step">In X, press <kbd>{pasteShortcut}</kbd> to attach this card.</p><p className="share-hint">Your caption opens in X. The image is copied, not attached automatically. Review both before posting.</p>
        <button className="text-button share-copy-card" disabled={!ready} onClick={() => void action('image')}>Copy Card</button>
        {message && <p className="share-message" role="status">{message}</p>}{error && <p className="cloud-error" role="alert">{error}</p>}
      </div>
    </section>
  </div>;
}
