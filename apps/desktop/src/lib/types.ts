export type ProviderID = 'claude' | 'codex' | 'cursor' | 'opencode';
export type Period = 'today' | 'week' | 'month';
export type Theme = 'system' | 'light' | 'dark';
export type Surface = 'dashboard' | 'widget' | 'menubar';
export type WidgetLayout = 'minimal' | 'usage' | 'rank' | 'models';
export type WidgetSize = 'compact' | 'standard' | 'wide';
export type MenuBarView = 'icon' | 'tokens' | 'limits' | 'tokens-limits' | 'spend' | 'custom';
export type MenuBarItem = 'tokens' | 'limits' | 'spend';
export type MenuBarDetails = 'summary' | 'providers' | 'models';
export type Metric = 'tokens' | 'limits' | 'models' | 'rank' | 'spend' | 'trend';
export interface MetricPreferences { tokens: boolean; limits: boolean; models: boolean; rank: boolean; spend: boolean; trend: boolean }
export interface Preferences {
  version: 1;
  theme: Theme;
  period: Period;
  providers: ProviderID[];
  enabledProviders: ProviderID[];
  surfaces: Record<Surface, MetricPreferences>;
  widgetLayout: WidgetLayout;
  widgetSize: WidgetSize;
  widgetTransparency: boolean;
  widgetOpacity: number;
  alwaysOnTop: boolean;
  launchAtLogin: boolean;
  autoUpdates: boolean;
  menuBarView: MenuBarView;
  menuBarProvider: 'all' | ProviderID;
  menuBarLabels: boolean;
  menuBarItems: MenuBarItem[];
  menuBarDetails: MenuBarDetails;
}
export interface UsageEvent {
  id: string;
  provider: ProviderID;
  timestamp: string;
  model: string;
  tokens: number;
  costUSD?: number | null;
  unpricedTokens?: number;
  costSource?: 'recorded' | 'estimated' | 'mixed' | null;
  project?: { id: string; name: string; directory: string } | null;
}
export interface Quota { label: string; used: number; resetsAt?: string; resetLabel?: string }
export interface UsageSnapshot {
  events: UsageEvent[];
  fetchedAt: string | null;
  source: 'local' | 'preview' | 'browser';
  errors: string[];
  availableProviders: ProviderID[];
  quotas: Partial<Record<ProviderID, Quota>>;
  quotaErrors?: Partial<Record<ProviderID, string>>;
  providerErrors?: Partial<Record<ProviderID, string>>;
  ranked?: boolean;
  rankUnavailable?: boolean;
  rank?: { position: number; movement?: number; podiums: number; updatedAt: string; period: Period; tokens: number };
}
export interface ModelTotal { model: string; tokens: number; share: number; estimatedCost: number | null; unpricedTokens: number }
export interface ProjectTotal {
  id: string;
  name: string;
  directory: string | null;
  tokens: number;
  share: number;
  estimatedCost: number | null;
  unpricedTokens: number;
  providers: { id: ProviderID; tokens: number; estimatedCost: number | null; unpricedTokens: number; models: ModelTotal[] }[];
  models: ModelTotal[];
}
export interface UsageTotals {
  total: number;
  previous: number;
  delta: number | null;
  providers: Record<ProviderID, number>;
  models: ModelTotal[];
  projects: ProjectTotal[];
  trend: { label: string; tokens: number }[];
  estimatedCost: number | null;
  providerCosts: Record<ProviderID, number | null>;
  providerUnpricedTokens: Record<ProviderID, number>;
  unpricedTokens: number;
  unpricedModels: string[];
  hasData: boolean;
}
