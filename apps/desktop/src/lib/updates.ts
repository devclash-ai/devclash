import { invoke } from '@tauri-apps/api/core';
import { native } from './desktop';

export interface UpdateStatus {
  enabled: boolean;
  installMode: 'background' | 'manual';
  restartRequired: boolean;
  currentVersion: string;
  availableVersion: string | null;
  notes: string;
  phase: 'idle' | 'checking' | 'available' | 'downloading' | 'installing' | 'ready' | 'error';
  progress: number | null;
  checkedAt: string | null;
  error: string | null;
}

export function initialUpdateStatus(enabled: boolean): UpdateStatus {
  return { enabled, installMode: 'manual', restartRequired: true, currentVersion: '', availableVersion: null, notes: '', phase: 'idle', progress: null, checkedAt: null, error: null };
}

async function updateCommand(command: string): Promise<UpdateStatus> {
  if (!native) throw new Error('App updates are available in the desktop app.');
  return invoke<UpdateStatus>(command);
}
export function readUpdateStatus(): Promise<UpdateStatus> { return updateCommand('update_status'); }
export function checkForUpdates(): Promise<UpdateStatus> { return updateCommand('check_for_updates'); }
export function installUpdate(): Promise<UpdateStatus> { return updateCommand('install_update'); }
export async function restartAfterUpdate(): Promise<void> {
  if (!native) throw new Error('Restart is available in the desktop app.');
  await invoke('restart_app');
}
