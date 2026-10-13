import { Battery, Check, ChevronDown, SlidersHorizontal, Wifi } from 'lucide-react';
import type { MenuBarDetails, MenuBarItem, MenuBarView, Preferences, UsageSnapshot, UsageTotals } from '../lib/types';
import { enabledInOrder, providerNames } from '../lib/preferences';
import { trayContent } from '../lib/tray';
import type { TrayContent } from '../lib/tray';
import { Toggle } from './Common';

const views: { id: MenuBarView; title: string; description: string }[] = [
  { id: 'icon', title: 'Icon Only', description: 'Keep your menu bar minimal' },
  { id: 'tokens', title: 'Tokens', description: 'Total usage for your selected period' },
  { id: 'limits', title: 'Quota', description: 'Highest current quota used' },
  { id: 'tokens-limits', title: 'Tokens + Quota', description: 'Usage and limits side by side' },
  { id: 'spend', title: 'Estimated Spend', description: 'API-equivalent cost, in USD' },
  { id: 'custom', title: 'Custom', description: 'Choose your own combination' },
];
const items: { id: MenuBarItem; title: string }[] = [
  { id: 'tokens', title: 'Tokens' }, { id: 'limits', title: 'Quota' }, { id: 'spend', title: 'Estimated Spend' },
];
export function MenuBarGlyph() { return <img className="menu-bar-glyph" src="/devclash/tray-template.svg" alt="" />; }
export function MenuBarSettings({ prefs, snapshot, totals, onChange }: {
  prefs: Preferences; snapshot: UsageSnapshot; totals: UsageTotals; onChange: (prefs: Preferences) => void;
}) {
  const setDetails = (menuBarDetails: MenuBarDetails) => onChange({ ...prefs, menuBarDetails,
    surfaces: { ...prefs.surfaces, menubar: { ...prefs.surfaces.menubar, ...(menuBarDetails === 'models' ? { models: true } : {}) } } });
  const setItem = (id: MenuBarItem) => onChange({ ...prefs, menuBarItems: prefs.menuBarItems.includes(id) ? prefs.menuBarItems.filter(item => item !== id) : [...prefs.menuBarItems, id] });
  return <section className="settings-card menu-bar-settings"><h2>Menu Bar View</h2>
    <p className="settings-intro">Choose what stays visible beside the DevClash icon.</p>
    <div className="menu-bar-views" aria-label="Menu Bar View">{views.map(view => {
      const preview = trayContent({ ...prefs, menuBarView: view.id }, snapshot, totals);
      return <button key={view.id} className={'menu-bar-view' + (prefs.menuBarView === view.id ? ' selected' : '')} aria-label={view.title + ' View'} aria-pressed={prefs.menuBarView === view.id} onClick={() => onChange({ ...prefs, menuBarView: view.id })}>
        <span className="menu-bar-view-example" aria-hidden="true"><MenuBarGlyph /><span>{preview.label}</span></span>
        <span className="menu-bar-view-title">{view.title}{prefs.menuBarView === view.id && <Check size={14} />}</span>
        <span className="menu-bar-view-description">{view.description}</span>
      </button>;
    })}</div>
    {prefs.menuBarView === 'custom' && <div className="menu-bar-custom"><strong>Pinned Metrics</strong><div aria-label="Pinned Metrics">{items.map(item =>
      <button key={item.id} className={prefs.menuBarItems.includes(item.id) ? 'selected' : ''} aria-pressed={prefs.menuBarItems.includes(item.id)} onClick={() => setItem(item.id)}>{item.title}</button>)}</div>
      <span>Selected metrics appear in the order you choose them.</span></div>}
    <div className="setting-row menu-bar-source"><SlidersHorizontal size={23} /><label htmlFor="menu-bar-provider">Usage Source</label>
      <select id="menu-bar-provider" value={prefs.menuBarProvider} onChange={event => onChange({ ...prefs, menuBarProvider: event.target.value as Preferences['menuBarProvider'] })}>
        <option value="all">All Enabled Providers</option>{prefs.providers.map(id => <option key={id} value={id}>{providerNames[id]}{!prefs.enabledProviders.includes(id) ? ' (Hidden)' : ''}</option>)}
      </select></div>
    {prefs.menuBarProvider !== 'all' && !enabledInOrder(prefs).includes(prefs.menuBarProvider) && <p className="menu-bar-source-note">Enable {providerNames[prefs.menuBarProvider]} below to show its usage here.</p>}
    <div className="setting-row menu-bar-label-setting"><strong>Show Metric Labels</strong>
      <Toggle label="Show Metric Labels" enabled={prefs.menuBarLabels} onChange={() => onChange({ ...prefs, menuBarLabels: !prefs.menuBarLabels })} />
    </div>
    <div className="setting-row menu-bar-detail-setting"><label htmlFor="menu-bar-details">Dropdown View</label>
      <select id="menu-bar-details" value={prefs.menuBarDetails} onChange={event => setDetails(event.target.value as MenuBarDetails)}>
        <option value="summary">Summary</option><option value="providers">By Provider</option><option value="models">By Model</option>
      </select></div>
  </section>;
}
export function MenuBarPreview({ tray, prefs }: { tray: TrayContent; prefs: Preferences }) {
  const source = prefs.menuBarProvider === 'all' ? 'All Enabled Providers' : providerNames[prefs.menuBarProvider];
  return <div className="menu-bar-preview">
    <div className="menu-bar-preview-heading"><strong>Your Menu Bar</strong><span>{prefs.period.charAt(0).toUpperCase() + prefs.period.slice(1)} · {source}</span></div>
    <div className="menu-bar-strip" aria-label="Menu Bar Preview"><span className="menu-bar-context">File</span>
      <span className="menu-bar-status-item"><MenuBarGlyph />{tray.label && <strong>{tray.label}</strong>}</span>
      <span className="menu-bar-system-icons" aria-hidden="true"><Wifi size={15} /><Battery size={19} /></span>
    </div>
    <div className="tray-menu-preview" aria-label="Dropdown Preview">{tray.rows.map((row, index) => <span className="tray-summary-row" key={index}>{row}</span>)}
      {tray.rows.length > 0 && <hr />}<span>Open Dashboard</span><span>Floating Widget</span><span className="tray-preview-customize">Customize Menu Bar<ChevronDown size={13} /></span><span>Refresh Usage</span><hr /><span>Quit DevClash</span>
    </div>
    <p>Left click opens your dashboard. Right click opens this menu.<br />Changes appear in the menu bar immediately.</p>
    <p className="menu-bar-platform-note">macOS shows values beside the icon. Windows uses the tooltip and menu. Linux uses the menu and supported panels.</p>
  </div>;
}
