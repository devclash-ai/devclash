import { useId } from 'react';
import { ChevronDown, RefreshCw, Settings, X } from 'lucide-react';
import type { Period, ProviderID, UsageTotals } from '../lib/types';
import { closeWindow, maximizeWindow, minimizeWindow, native } from '../lib/desktop';
export function Logo({ small = false, theme }: { small?: boolean; theme: 'light' | 'dark' }) {
  return <img className={'brand' + (small ? ' brand-small' : '')} src={'/devclash/logo-' + theme + '.svg'} alt="DevClash" />;
}
export function BrandMark({ theme, className = '', alt = '' }: { theme: 'light' | 'dark'; className?: string; alt?: string }) {
  return <img className={'brand-mark ' + className} src={'/devclash/mark-' + theme + '.svg'} alt={alt} />;
}
export function Toggle({ enabled, label, onChange, disabled = false }: { enabled: boolean; label: string; onChange: () => void; disabled?: boolean }) {
  return <button type="button" className={'toggle' + (enabled ? ' enabled' : '')} role="switch" aria-checked={enabled} aria-label={label} disabled={disabled} onClick={onChange}><span /></button>;
}
export function WindowChrome({ branded = false, theme = 'light' }: { branded?: boolean; theme?: 'light' | 'dark' }) {
  return <div className={'window-chrome' + (branded ? ' branded-chrome' : '')} data-tauri-drag-region>
    <div className="traffic-lights" data-no-share="true">
      <button className="traffic red" aria-label="Close Window" onClick={() => void closeWindow()}><X size={9} /></button>
      <button className="traffic yellow" aria-label="Minimize Window" onClick={() => void minimizeWindow()} />
      <button className="traffic green" aria-label="Maximize Window" onClick={() => void maximizeWindow()} />
    </div>{branded && <Logo small theme={theme} />}
  </div>;
}
export function ProviderIcon({ id, plain = false }: { id: ProviderID; plain?: boolean }) {
  return <span className={'provider-icon ' + id + (plain ? ' plain' : '')}><img src={'/providers/' + id + '.svg?v=1'} alt="" /></span>;
}
export function PeriodControl({ period, setPeriod, small = false }: { period: Period; setPeriod: (p: Period) => void; small?: boolean }) {
  if (small) return <label className="period-select"><select aria-label="Usage Period" value={period} onChange={event => setPeriod(event.target.value as Period)}>
    <option value="today">Today</option><option value="week">Week</option><option value="month">Month</option>
  </select><ChevronDown size={15} /></label>;
  return <div className="period-control" aria-label="Usage Period">{(['today', 'week', 'month'] as Period[]).map(p =>
    <button key={p} aria-pressed={period === p} className={p === period ? 'selected' : ''} onClick={() => setPeriod(p)}>{p.charAt(0).toUpperCase() + p.slice(1)}</button>)}</div>;
}
export function Sparkline({ trend, labels = false, filled = false }: { trend: UsageTotals['trend']; labels?: boolean; filled?: boolean }) {
  const gradient = useId().replace(/:/g, '');
  const values = trend.map(p => p.tokens); const min = filled ? 0 : Math.min(...values) * .7; const max = Math.max(1, ...values);
  const points = values.map((v, i) => [12 + i * 54, 65 - (v - min) / (max - min || 1) * 51]);
  const path = points.map(([x, y], i) => i === 0 ? 'M ' + x + ' ' + y : ' C ' + (points[i - 1][0] + 24) + ' ' + points[i - 1][1] + ', ' + (x - 24) + ' ' + y + ', ' + x + ' ' + y).join('');
  return <div className={'sparkline' + (labels ? ' labeled' : '')}><svg viewBox="0 0 350 80" preserveAspectRatio={filled ? 'none' : undefined} role="img" aria-label="Token Usage Over Seven Days">
    {filled && <><defs><linearGradient id={gradient} x1="0" y1="0" x2="0" y2="1"><stop offset="0%" stopColor="currentColor" stopOpacity=".23" /><stop offset="100%" stopColor="currentColor" stopOpacity="0" /></linearGradient></defs><path d={path + ' L 336 80 L 12 80 Z'} fill={`url(#${gradient})`} /></>}
    <path d={path} fill="none" stroke="currentColor" strokeWidth={filled ? 2 : 2.6} vectorEffect={filled ? "non-scaling-stroke" : undefined} strokeLinecap="round" />
  </svg>{labels && <div className="sparkline-labels">{trend.map(point => <span key={point.label}>{point.label}</span>)}</div>}</div>;
}
export function SettingsButton({ onClick }: { onClick: () => void }) {
  return <button className="icon-button widget-settings" aria-label="Customize Widget" onClick={onClick}><Settings size={24} /></button>;
}
export function RefreshButton({ onClick, busy = false }: { onClick: () => void; busy?: boolean }) {
  return <button className="icon-button" aria-label="Refresh Usage" onClick={onClick} disabled={busy}><RefreshCw className={busy ? 'spinning' : ''} size={21} /></button>;
}
export function EmptyState({ browser = false, reading = false }: { browser?: boolean; reading?: boolean }) {
  return <div className="empty-state"><strong>{reading ? 'Reading Your Local Usage' : browser ? 'Your Desktop Usage, at a Glance' : 'Ready When You Are'}</strong>
    <p>{reading ? 'Your usage will appear when the first scan finishes.' : browser ? 'Open the desktop app to read usage on this device, or explore the design preview.' : 'Use your providers to see activity here. Local usage and your signed-in Cursor account are read automatically.'}</p>
    {browser && <a href="/concepts">Explore the Design Preview</a>}</div>;
}
export function NativeOnly({ children }: { children: React.ReactNode }) { return native ? children : null; }
