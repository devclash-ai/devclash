import { useState } from 'react';
import type { CSSProperties } from 'react';
import { BarChart3, PieChart, Box, Trophy, CircleDollarSign, TrendingUp, GripVertical, PanelsTopLeft, Sun, Pin, LayoutGrid, PanelTop, Menu, Power, ArrowUp, ArrowDown, Blend, Download } from 'lucide-react';
import type { Metric, Preferences, ProviderID, Surface, Theme, UsageSnapshot, UsageTotals, WidgetLayout, WidgetSize } from '../lib/types';
import { defaults, moveProvider, providerNames, widgetOpacityRange } from '../lib/preferences';
import { FloatingWidget } from './Widget';
import { Dashboard } from './Dashboard';
import { ProviderIcon, Toggle, WindowChrome } from './Common';
import { MenuBarPreview, MenuBarSettings } from './MenuBar';
import { native } from '../lib/desktop';
import { trayContent } from '../lib/tray';
import { automaticUpdateSummary } from '../lib/updateCopy';

interface Props {
  preferences: Preferences; snapshot: UsageSnapshot; totals: UsageTotals; theme: 'light' | 'dark';
  onChange: (prefs: Preferences) => void; onDone: () => void; onAutostart: (enabled: boolean) => void;
  initialSurface?: Surface; onWidget: () => void;
  automaticUpdates?: { enabled: boolean; pending: boolean; installMode: 'background' | 'manual'; restartRequired: boolean };
  onAutomaticChange?: (enabled: boolean) => void;
}
const metricOptions = [
  { key: 'tokens', title: 'Total Tokens', description: 'Show total tokens used for the selected period', icon: BarChart3 },
  { key: 'limits', title: 'Provider Limits', description: 'Show current session or plan limits for each provider', icon: PieChart },
  { key: 'models', title: 'Model Breakdown', description: 'Show token usage by model', icon: Box },
  { key: 'rank', title: 'Rank & Badges', description: 'Show your public UTC rank and earned badges when connected', icon: Trophy },
  { key: 'spend', title: 'Estimated Spend', description: 'Show estimated cost when pricing is available', icon: CircleDollarSign },
  { key: 'trend', title: 'Usage Trend', description: 'Show a mini chart of your usage over seven days', icon: TrendingUp },
] as const;

function Segmented<T extends string>({ value, options, label, onChange }: { value: T; options: readonly T[]; label: string; onChange: (value: T) => void }) {
  return <div className="segmented" aria-label={label}>{options.map(option => <button key={option} className={value === option ? 'selected' : ''} aria-pressed={value === option} onClick={() => onChange(option)}>{option.charAt(0).toUpperCase() + option.slice(1)}</button>)}</div>;
}
export function Customization(props: Props) {
  const { preferences: prefs, snapshot, totals, theme, onChange, onDone, onAutostart, onWidget } = props;
  const [surface, setSurface] = useState<Surface>(props.initialSurface ?? 'widget');
  const [dragged, setDragged] = useState<ProviderID | null>(null);
  const automaticEnabled = props.automaticUpdates?.enabled ?? prefs.autoUpdates;
  const setMetric = (key: Metric) => onChange({ ...prefs, surfaces: { ...prefs.surfaces, [surface]: { ...prefs.surfaces[surface], [key]: !prefs.surfaces[surface][key] } } });
  const setLayout = (layout: WidgetLayout) => onChange({ ...prefs, widgetLayout: layout, surfaces: { ...prefs.surfaces, widget: { ...prefs.surfaces.widget, ...(layout === 'models' ? { models: true } : {}), ...(layout === 'rank' ? { rank: true } : {}) } } });
  const toggleProvider = (id: ProviderID) => onChange({ ...prefs, enabledProviders: prefs.enabledProviders.includes(id) ? prefs.enabledProviders.filter(item => item !== id) : [...prefs.enabledProviders, id] });
  const reorder = (id: ProviderID, direction: number) => {
    const order = [...prefs.providers]; const index = order.indexOf(id); const next = index + direction;
    if (next < 0 || next >= order.length) return;
    [order[index], order[next]] = [order[next], order[index]]; onChange({ ...prefs, providers: order });
  };
  const reset = () => onChange({ ...defaults(), launchAtLogin: prefs.launchAtLogin, autoUpdates: prefs.autoUpdates });
  const tray = trayContent(prefs, snapshot, totals);
  return <section className={'customization app-panel theme-' + theme} aria-label="Customize Your View">
    <WindowChrome branded theme={theme} />
    <div className="customization-heading"><h1>Customize Your View</h1><nav className="surface-tabs" aria-label="Customize Surface">
      {([{ id: 'dashboard', label: 'Dashboard', icon: LayoutGrid }, { id: 'widget', label: 'Widget', icon: PanelTop }, { id: 'menubar', label: 'Menu Bar', icon: Menu }] as const).map(tab =>
        <button key={tab.id} className={surface === tab.id ? 'selected' : ''} aria-pressed={surface === tab.id} onClick={() => setSurface(tab.id)}><tab.icon size={20} />{tab.label}</button>)}
    </nav></div>
    <div className="customization-body"><div className="customization-options">
      {surface === 'menubar' && <MenuBarSettings prefs={prefs} snapshot={snapshot} totals={totals} onChange={onChange} />}
      <section className="settings-card metric-card"><h2>{surface === 'menubar' ? 'Dropdown Metrics' : 'Visible Metrics'}</h2>
        {metricOptions.map(option => <div className="setting-row" key={option.key}><option.icon size={25} strokeWidth={1.7} /><div className="setting-copy"><strong>{option.title}</strong><span>{surface === 'menubar' && option.key === 'trend' ? 'Show your total tokens over the last seven days' : option.description}</span></div>
          <Toggle enabled={prefs.surfaces[surface][option.key]} label={option.title} onChange={() => setMetric(option.key)} /></div>)}
      </section>
      <section className="settings-card provider-settings"><div className="settings-card-title"><h2>Providers</h2><span>Drag to Reorder</span></div><div className="provider-order-list">
        {prefs.providers.map((id, index) => <div key={id} className={'provider-setting-row' + (dragged === id ? ' dragging' : '')} draggable
          onDragStart={event => { setDragged(id); event.dataTransfer.effectAllowed = 'move'; event.dataTransfer.setData('text/plain', id); }} onDragEnd={() => setDragged(null)}
          onDragOver={event => event.preventDefault()} onDrop={event => { event.preventDefault(); if (dragged && dragged !== id) onChange(moveProvider(prefs, dragged, id)); setDragged(null); }}>
          <GripVertical size={19} className="drag-handle" /><ProviderIcon id={id} plain /><strong>{providerNames[id]}</strong>
          <div className="reorder-buttons"><button className="icon-button" aria-label={'Move ' + providerNames[id] + ' Up'} disabled={index === 0} onClick={() => reorder(id, -1)}><ArrowUp size={15} /></button>
            <button className="icon-button" aria-label={'Move ' + providerNames[id] + ' Down'} disabled={index === prefs.providers.length - 1} onClick={() => reorder(id, 1)}><ArrowDown size={15} /></button></div>
          <Toggle enabled={prefs.enabledProviders.includes(id)} label={'Show ' + providerNames[id]} onChange={() => toggleProvider(id)} /></div>)}
      </div></section>
      <section className="settings-card display-settings">
        {surface === 'widget' && <div className="setting-row"><PanelsTopLeft size={25} strokeWidth={1.7} /><strong>Widget Size</strong><Segmented<WidgetSize> value={prefs.widgetSize} options={['compact', 'standard', 'wide']} label="Widget Size" onChange={widgetSize => onChange({ ...prefs, widgetSize })} /></div>}
        <div className="setting-row"><Sun size={26} strokeWidth={1.7} /><strong>Appearance</strong><Segmented<Theme> value={prefs.theme} options={['system', 'light', 'dark']} label="Appearance" onChange={theme => onChange({ ...prefs, theme })} /></div>
        <div className="setting-row"><Download size={25} strokeWidth={1.7} /><div className="setting-copy"><strong>Automatic Updates</strong><span>{native ? props.automaticUpdates?.pending ? 'Saving or reading your update preference…' : props.automaticUpdates ? automaticUpdateSummary(props.automaticUpdates) : 'Check automatically; approve installation when available' : 'Desktop update preference; this preview does not check for updates'}</span></div><Toggle disabled={props.automaticUpdates?.pending} enabled={automaticEnabled} label="Automatic Updates" onChange={() => props.onAutomaticChange ? props.onAutomaticChange(!automaticEnabled) : onChange({ ...prefs, autoUpdates: !automaticEnabled })} /></div>
        {surface === 'widget' && <><div className="setting-row"><Blend size={26} strokeWidth={1.7} /><div className="setting-copy"><strong>Transparency</strong><span>Let your desktop show through the widget</span></div><Toggle enabled={prefs.widgetTransparency} label="Transparency" onChange={() => onChange({ ...prefs, widgetTransparency: !prefs.widgetTransparency })} /></div>
          {prefs.widgetTransparency && <div className="setting-row opacity-setting"><label htmlFor="widget-opacity">Background Opacity</label><div className="opacity-control">
            <input id="widget-opacity" type="range" min={widgetOpacityRange.min} max={widgetOpacityRange.max} step={widgetOpacityRange.step} value={prefs.widgetOpacity} style={{ '--opacity-fill': prefs.widgetOpacity + '%' } as CSSProperties} aria-valuetext={prefs.widgetOpacity + '%'} onChange={event => onChange({ ...prefs, widgetOpacity: Number(event.currentTarget.value) })} />
            <output htmlFor="widget-opacity">{prefs.widgetOpacity}%</output>
          </div></div>}</>}
        {surface === 'widget' && <div className="setting-row"><Pin size={26} strokeWidth={1.7} /><div className="setting-copy"><strong>Always On Top</strong><span>Keep the widget visible above other windows</span></div><Toggle enabled={prefs.alwaysOnTop} label="Always On Top" onChange={() => onChange({ ...prefs, alwaysOnTop: !prefs.alwaysOnTop })} /></div>}
        {surface === 'menubar' && <div className="setting-row"><Power size={25} /><div className="setting-copy"><strong>Launch at Login</strong><span>Start DevClash in your menu bar automatically</span></div><Toggle disabled={!native} enabled={prefs.launchAtLogin} label="Launch at Login" onChange={() => onAutostart(!prefs.launchAtLogin)} /></div>}
      </section>
    </div><aside className="preview-pane"><h2>Preview</h2>
      {surface === 'widget' && <div className="widget-layout-picker" aria-label="Widget Layout">{([{ id: 'minimal', label: 'Minimal' }, { id: 'usage', label: 'Usage + Limits' }, { id: 'rank', label: 'Rank + Usage' }, { id: 'models', label: 'Models' }] as const).map(layout =>
        <button key={layout.id} className={prefs.widgetLayout === layout.id ? 'selected' : ''} aria-pressed={prefs.widgetLayout === layout.id} onClick={() => setLayout(layout.id)}>{layout.label}</button>)}</div>}
      <div className={'preview-stage preview-' + surface + (surface === 'widget' && prefs.widgetTransparency ? ' transparency-preview' : '')}>
        {surface === 'widget' && <FloatingWidget preferences={prefs} snapshot={snapshot} totals={totals} theme={theme} preview setPeriod={period => onChange({ ...prefs, period })} onCustomize={() => {}} onShare={() => {}} onRank={() => {}} />}
        {surface === 'dashboard' && <div className="dashboard-preview"><Dashboard preferences={prefs} snapshot={snapshot} totals={totals} theme={theme} period={prefs.period} connected={snapshot.source === 'preview'} setPeriod={period => onChange({ ...prefs, period })} onCustomize={() => {}} onRefresh={() => {}} onAccount={() => {}} onWidget={onWidget} onShare={() => {}} onRank={() => {}} /></div>}
        {surface === 'menubar' && <MenuBarPreview tray={tray} prefs={prefs} />}
      </div>{snapshot.source === 'preview' && <span className="preview-data-note">Sample Data for the Design Preview</span>}
      {surface === 'widget' && <button className="button preview-open-widget" onClick={onWidget}><PanelTop size={17} />Open Floating Widget</button>}
    </aside></div>
    <footer className="customization-footer"><button className="text-button" onClick={reset}>Reset to Default</button><span>Changes are saved automatically.</span><button className="button primary" onClick={onDone}>Done</button></footer>
  </section>;
}
