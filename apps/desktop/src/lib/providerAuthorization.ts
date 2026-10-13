import type { ProviderID } from './types';

const sources: Partial<Record<ProviderID, string>> = {
  claude: 'claude-retry', codex: 'codex', cursor: 'cursor-cli',
};

export function canRetryAuthorization(id: ProviderID, reason?: string): boolean {
  return !!sources[id] && !!reason?.includes('Keychain access is required');
}

export async function retryProviderAuthorization(
  id: ProviderID, authorize: (source: string) => Promise<unknown>, refresh: () => Promise<void>,
): Promise<void> {
  const source = sources[id];
  if (!source) throw new Error('This provider has no local authorization action.');
  await authorize(source);
  await refresh();
}
