import { useState } from 'react';
import { ArrowLeft } from 'lucide-react';
import type { Preferences, UsageSnapshot, UsageTotals } from '../lib/types';
import { Dashboard } from './Dashboard';
import { FloatingWidget } from './Widget';
import { Customization } from './Customization';
interface Props {
  preferences: Preferences; snapshot: UsageSnapshot; totals: UsageTotals; onChange: (prefs: Preferences) => void;
  onShare: (element: HTMLElement) => void; onRank: () => void; onAccount: () => void; onWidget: () => void;
}
export function Concepts(props: Props) {
  const [page, setPage] = useState<'dashboard' | 'widgets' | 'customize'>('dashboard');
  const { preferences: prefs, snapshot, totals, onChange } = props;
  const setPeriod = (period: Preferences['period']) => onChange({ ...prefs, period });
  return <div className="concepts-page"><header className="concepts-toolbar"><a href="/" className="text-button"><ArrowLeft size={16} />Desktop App</a><nav aria-label="Design Concepts">
    <button className={page === 'dashboard' ? 'selected' : ''} onClick={() => setPage('dashboard')}>Dashboard</button><button className={page === 'widgets' ? 'selected' : ''} onClick={() => setPage('widgets')}>Widgets</button><button className={page === 'customize' ? 'selected' : ''} onClick={() => setPage('customize')}>Customization</button></nav><span className="sample-badge">Design Preview · Sample Data</span></header>
    {page === 'dashboard' && <div className="dashboard-board">{(['light', 'dark'] as const).map(theme => <div className="board-item" key={theme}><span className="board-caption">{theme === 'light' ? 'Light / Local' : 'Dark / Connected'}</span><Dashboard preferences={prefs} snapshot={snapshot} totals={totals} period={prefs.period} theme={theme} connected={theme === 'dark'} setPeriod={setPeriod} onCustomize={() => setPage('customize')} onRefresh={() => {}} onAccount={props.onAccount} onWidget={props.onWidget} onShare={props.onShare} onRank={props.onRank} /></div>)}</div>}
    {page === 'widgets' && <div className="widgets-board"><h1>DevClash Widgets</h1><div className="widget-grid">{([{ layout: 'minimal', theme: 'light', label: 'Minimal' }, { layout: 'usage', theme: 'light', label: 'Usage + Limits' }, { layout: 'rank', theme: 'dark', label: 'Rank + Usage' }, { layout: 'models', theme: 'dark', label: 'Models' }] as const).map(item =>
      <div className={'widget-board-item board-' + item.layout} key={item.layout}><span className="board-caption">{item.label}</span><FloatingWidget preferences={{ ...prefs, widgetSize: 'standard', surfaces: { ...prefs.surfaces, widget: { ...prefs.surfaces.widget, models: item.layout === 'models', rank: item.layout === 'rank', trend: item.layout === 'minimal' } } }} layout={item.layout} theme={item.theme} snapshot={snapshot} totals={totals} setPeriod={setPeriod} onCustomize={() => { onChange({ ...prefs, widgetLayout: item.layout, surfaces: { ...prefs.surfaces, widget: { ...prefs.surfaces.widget, models: item.layout === 'models' } } }); setPage('customize'); }} onShare={props.onShare} onRank={props.onRank} /></div>)}</div></div>}
    {page === 'customize' && <div className="customization-board"><Customization preferences={prefs} snapshot={snapshot} totals={totals} theme={prefs.theme === 'dark' ? 'dark' : 'light'} onChange={onChange} onDone={() => setPage('dashboard')} onAutostart={() => {}} onWidget={props.onWidget} /></div>}
  </div>;
}
