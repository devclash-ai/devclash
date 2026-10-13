import type { ModelTotal, ProjectTotal, ProviderID, UsageEvent } from './types';

interface Sum { tokens: number; cost: number | null; unpriced: number }
function add(sum: Sum, event: UsageEvent): void {
  const priced = event.costUSD != null && Number.isFinite(event.costUSD) && event.costUSD >= 0;
  sum.tokens += event.tokens;
  sum.unpriced += priced ? Number.isSafeInteger(event.unpricedTokens) && event.unpricedTokens! >= 0 ? Math.min(event.unpricedTokens!, event.tokens) : 0 : event.tokens;
  if (priced) sum.cost = (sum.cost ?? 0) + event.costUSD!;
}
const empty = (): Sum => ({ tokens: 0, cost: null, unpriced: 0 });
function modelTotals(models: Map<string, Sum>, total: number): ModelTotal[] {
  return [...models].map(([model, sum]) => ({ model, tokens: sum.tokens, share: total ? sum.tokens / total * 100 : 0,
    estimatedCost: sum.cost, unpricedTokens: sum.unpriced })).sort((a, b) => b.tokens - a.tokens || a.model.localeCompare(b.model));
}
const byUsage = (a: ProjectTotal, b: ProjectTotal): number => b.tokens - a.tokens || a.name.localeCompare(b.name) || a.id.localeCompare(b.id);

// Receives the same deduplicated, period/provider-filtered records as the dashboard total.
export function projectTotals(events: UsageEvent[], total: number): ProjectTotal[] {
  const groups = new Map<string, { project: UsageEvent['project']; sum: Sum; providers: Map<ProviderID, { sum: Sum; models: Map<string, Sum> }>; models: Map<string, Sum> }>();
  for (const event of events) {
    const key = event.project ? 'project:' + event.project.id : 'unassigned';
    const group = groups.get(key) ?? { project: event.project, sum: empty(), providers: new Map(), models: new Map() };
    add(group.sum, event);
    const modelName = event.model || 'Unknown Model';
    const provider = group.providers.get(event.provider) ?? { sum: empty(), models: new Map() }; add(provider.sum, event);
    const providerModel = provider.models.get(modelName) ?? empty(); add(providerModel, event); provider.models.set(modelName, providerModel);
    group.providers.set(event.provider, provider);
    const model = group.models.get(modelName) ?? empty(); add(model, event); group.models.set(modelName, model);
    groups.set(key, group);
  }
  return [...groups].map(([id, group]) => ({
    id, name: group.project?.name ?? 'Unassigned Usage', directory: group.project?.directory ?? null,
    tokens: group.sum.tokens, share: total ? group.sum.tokens / total * 100 : 0,
    estimatedCost: group.sum.cost, unpricedTokens: group.sum.unpriced,
    providers: [...group.providers].map(([id, provider]) => ({ id, tokens: provider.sum.tokens, estimatedCost: provider.sum.cost,
      unpricedTokens: provider.sum.unpriced, models: modelTotals(provider.models, provider.sum.tokens) })).sort((a, b) => b.tokens - a.tokens),
    models: modelTotals(group.models, group.sum.tokens),
  })).sort(byUsage);
}

// Project amounts and model rows are scoped to this provider; shares use its total.
export function providerProjects(projects: ProjectTotal[], id: ProviderID, total: number): ProjectTotal[] {
  return projects.flatMap(project => {
    const provider = project.providers.find(provider => provider.id === id);
    return provider ? [{ ...project, tokens: provider.tokens, share: total ? provider.tokens / total * 100 : 0,
      estimatedCost: provider.estimatedCost, unpricedTokens: provider.unpricedTokens, providers: [provider], models: provider.models }] : [];
  }).sort(byUsage);
}

export function matchingProjects(projects: ProjectTotal[], query: string): ProjectTotal[] {
  const search = query.trim().toLocaleLowerCase();
  return search ? projects.filter(project => (project.name + ' ' + (project.directory ?? '')).toLocaleLowerCase().includes(search)) : projects;
}
