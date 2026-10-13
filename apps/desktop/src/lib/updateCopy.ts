import type { UpdateStatus } from './updates';

type UpdateCapabilities = Pick<UpdateStatus, 'installMode' | 'restartRequired'>;

export function automaticUpdateSummary({ installMode, restartRequired }: UpdateCapabilities): string {
  if (installMode === 'manual') return 'Check automatically; approve installation when available';
  return restartRequired ? 'Check and install automatically; restart when ready' : 'Check and install automatically; app closes and reopens';
}

export function automaticUpdateDescription({ enabled, installMode, restartRequired }: UpdateCapabilities & Pick<UpdateStatus, 'enabled'>): string {
  if (!enabled) return 'Automatic updates are off. You can check and install an update here.' + (restartRequired ? '' : ' Installing an update closes and reopens DevClash.');
  if (installMode === 'manual') return 'DevClash checks at startup and every six hours. This installation requires your approval, so choose Install when a new version is available.' + (restartRequired ? '' : ' DevClash closes and reopens after you choose Install.');
  return 'DevClash checks at startup and every six hours, then downloads and installs new versions automatically. ' + (restartRequired ? 'Installation runs in the background. Restart when you’re ready.' : 'DevClash closes and reopens automatically to finish the update.');
}
