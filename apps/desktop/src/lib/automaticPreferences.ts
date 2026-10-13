import type { Preferences } from './types';

export function withConfirmedAutomaticUpdates(preferences: Preferences, enabled: boolean): Preferences {
  return { ...preferences, autoUpdates: enabled };
}

// Confirm the updater preference only after the native store accepts the write.
export async function persistAutomaticPreference(
  enabled: boolean,
  current: () => Preferences,
  persist: (preferences: Preferences) => Promise<void>,
  confirm: (preferences: Preferences) => void,
): Promise<void> {
  await persist({ ...current(), autoUpdates: enabled });
  confirm({ ...current(), autoUpdates: enabled });
}
