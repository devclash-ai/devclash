import type { MenuBarDetails, MenuBarItem, MenuBarView, MetricPreferences, Preferences, ProviderID, Surface, WidgetSize } from './types';
const KEY = 'devclash.preferences.v1';
const LEGACY_KEY = 'usagerank.preferences.v1';
export const providerNames: Record<ProviderID, string> = { claude: 'Claude', codex: 'Codex', cursor: 'Cursor', opencode: 'OpenCode' };
export const providerIDs: ProviderID[] = ['claude', 'codex', 'cursor', 'opencode'];
export const widgetWidths: Record<WidgetSize, number> = { compact: 275, standard: 310, wide: 370 };
export const widgetOpacityRange = { min: 0, max: 100, step: 5 };
export const menuBarViews: MenuBarView[] = ['icon', 'tokens', 'limits', 'tokens-limits', 'spend', 'custom'];
export const menuBarItems: MenuBarItem[] = ['tokens', 'limits', 'spend'];
export const menuBarDetails: MenuBarDetails[] = ['summary', 'providers', 'models'];
const metrics: MetricPreferences = { tokens: true, limits: true, models: false, rank: true, spend: false, trend: true };
export function defaults(): Preferences {
  return { version: 1, theme: 'system', period: 'today', providers: [...providerIDs], enabledProviders: [...providerIDs],
    surfaces: { dashboard: { ...metrics, models: true }, widget: { ...metrics }, menubar: { ...metrics, rank: false, models: false } },
    widgetLayout: 'usage', widgetSize: 'standard', widgetTransparency: false, widgetOpacity: 80,
    alwaysOnTop: true, launchAtLogin: false, autoUpdates: true, menuBarView: 'tokens', menuBarProvider: 'all', menuBarLabels: false,
    menuBarItems: ['tokens', 'limits'], menuBarDetails: 'summary' };
}
export function normalizePreferences(raw: unknown): Preferences {
  const base = defaults();
  if (!raw || typeof raw !== 'object') return base;
  const value = raw as Partial<Preferences> & { menuBarMetric?: 'tokens' | 'limits' };
  if (value.version !== 1) return base;
  const validIDs = (ids: unknown): ids is ProviderID[] => Array.isArray(ids) && ids.every(id => providerIDs.includes(id));
  if (validIDs(value.providers)) base.providers = [...new Set([...value.providers, ...providerIDs])];
  if (validIDs(value.enabledProviders)) {
    // Enable the new source once for older installations, preserving an explicit all-hidden view.
    const newProvider = validIDs(value.providers) && !value.providers.includes('opencode') && value.enabledProviders.length > 0 ? ['opencode' as const] : [];
    base.enabledProviders = [...new Set([...value.enabledProviders, ...newProvider])];
  }
  if (['system', 'light', 'dark'].includes(value.theme ?? '')) base.theme = value.theme!;
  if (['today', 'week', 'month'].includes(value.period ?? '')) base.period = value.period!;
  if (['minimal', 'usage', 'rank', 'models'].includes(value.widgetLayout ?? '')) base.widgetLayout = value.widgetLayout!;
  if (['compact', 'standard', 'wide'].includes(value.widgetSize ?? '')) base.widgetSize = value.widgetSize!;
  if (typeof value.widgetTransparency === 'boolean') base.widgetTransparency = value.widgetTransparency;
  if (typeof value.widgetOpacity === 'number' && Number.isFinite(value.widgetOpacity)) {
    const { min, max, step } = widgetOpacityRange;
    base.widgetOpacity = Math.min(max, Math.max(min, Math.round(value.widgetOpacity / step) * step));
  }
  if (typeof value.alwaysOnTop === 'boolean') base.alwaysOnTop = value.alwaysOnTop;
  if (typeof value.launchAtLogin === 'boolean') base.launchAtLogin = value.launchAtLogin;
  if (typeof value.autoUpdates === 'boolean') base.autoUpdates = value.autoUpdates;
  // Preserve the original token/quota selection when upgrading saved preferences.
  if (value.menuBarMetric === 'tokens' || value.menuBarMetric === 'limits') {
    base.menuBarView = value.surfaces?.menubar?.[value.menuBarMetric] === false ? 'icon' : value.menuBarMetric;
  } else if (!value.menuBarView && value.surfaces?.menubar?.tokens === false) base.menuBarView = 'icon';
  if (menuBarViews.includes(value.menuBarView!)) base.menuBarView = value.menuBarView!;
  if (value.menuBarProvider === 'all' || providerIDs.includes(value.menuBarProvider!)) base.menuBarProvider = value.menuBarProvider!;
  if (typeof value.menuBarLabels === 'boolean') base.menuBarLabels = value.menuBarLabels;
  if (Array.isArray(value.menuBarItems) && value.menuBarItems.every(item => menuBarItems.includes(item))) base.menuBarItems = [...new Set(value.menuBarItems)];
  if (menuBarDetails.includes(value.menuBarDetails!)) base.menuBarDetails = value.menuBarDetails!;
  for (const surface of ['dashboard', 'widget', 'menubar'] as Surface[]) {
    const saved = value.surfaces?.[surface];
    if (!saved) continue;
    for (const metric of Object.keys(metrics) as (keyof MetricPreferences)[]) {
      if (typeof saved[metric] === 'boolean') base.surfaces[surface][metric] = saved[metric];
    }
  }
  return base;
}
export function loadPreferences(): Preferences {
  try {
    const saved = localStorage.getItem(KEY);
    if (saved !== null) return normalizePreferences(JSON.parse(saved));
    const legacy = localStorage.getItem(LEGACY_KEY);
    const migrated = normalizePreferences(JSON.parse(legacy ?? 'null'));
    if (legacy !== null) { localStorage.setItem(KEY, JSON.stringify(migrated)); localStorage.removeItem(LEGACY_KEY); }
    return migrated;
  }
  catch { return defaults(); }
}
export function savePreferences(prefs: Preferences): void { localStorage.setItem(KEY, JSON.stringify(prefs)); }
export function moveProvider(prefs: Preferences, id: ProviderID, target: ProviderID): Preferences {
  if (id === target) return prefs;
  const providers = prefs.providers.filter(item => item !== id);
  providers.splice(providers.indexOf(target), 0, id);
  return { ...prefs, providers };
}
export function enabledInOrder(prefs: Preferences): ProviderID[] {
  return prefs.providers.filter(id => prefs.enabledProviders.includes(id));
}
export const preferencesKey = KEY;
