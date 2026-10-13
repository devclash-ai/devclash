import { invoke } from '@tauri-apps/api/core';
import { native } from './desktop';
export interface CloudProject { id: string; title: string; archived: boolean; moderated: boolean }
export interface CloudStatus {
  configured: boolean; origin: string | null; appOrigin: string | null; user: { id: string; email: string } | null;
  expiresAt: string | null; syncEnabled: boolean; lastSyncedAt: string | null; syncError: string | null;
  ranked: boolean; publicProfile: boolean; rank: number | null; badges: number;
  pendingCount: number; retryAttempts: number; nextRetryAt: string | null;
  rankPeriod: 'Today' | 'Week' | 'Month'; rankTokens: number | null; rankGeneratedAt: string | null;
  projects: CloudProject[]; activeProjectId: string | null; projectWarning: string | null;
}
export interface SignInChallenge { challengeId: string; expiresAt: string; resendAfterSeconds: number; message: string }
export const localCloudStatus: CloudStatus = {
  configured: false, origin: null, appOrigin: null, user: null, expiresAt: null, syncEnabled: false, lastSyncedAt: null,
  syncError: null, ranked: false, publicProfile: false, rank: null, badges: 0,
  pendingCount: 0, retryAttempts: 0, nextRetryAt: null, rankPeriod: 'Today', rankTokens: null, rankGeneratedAt: null,
  projects: [], activeProjectId: null, projectWarning: null,
};
export async function readCloud(): Promise<CloudStatus> { return native ? invoke('cloud_status') : localCloudStatus; }
export async function refreshCloudProjects(): Promise<CloudStatus> { return invoke('cloud_refresh_projects'); }
export async function setActiveCloudProject(projectId: string | null): Promise<CloudStatus> { return invoke('cloud_set_active_project', { projectId }); }
export async function dismissProjectWarning(): Promise<CloudStatus> { return invoke('cloud_dismiss_project_warning'); }
export async function requestSignIn(email: string): Promise<SignInChallenge> { return invoke('cloud_request_code', { email }); }
export async function verifySignIn(challengeId: string, code: string): Promise<CloudStatus> { return invoke('cloud_verify_code', { challengeId, code }); }
export async function setCloudSync(enabled: boolean): Promise<CloudStatus> { return invoke('cloud_set_sync', { enabled }); }
export async function syncCloud(): Promise<CloudStatus> { return invoke('cloud_sync_now'); }
export async function setParticipation(ranked: boolean, publicProfile: boolean): Promise<CloudStatus> { return invoke('cloud_participation', { ranked, publicProfile }); }
export async function signOutCloud(): Promise<CloudStatus> { return invoke('cloud_logout'); }
export async function openCloudPage(page: 'rank' | 'profile'): Promise<void> { return invoke('cloud_open_page', { page }); }

export async function setRankPeriod(period: 'Today' | 'Week' | 'Month'): Promise<CloudStatus> { return invoke('cloud_set_rank_period', { period }); }

export interface BrowserSignIn { flowId: string; displayCode: string; expiresAt: string }
export const startBrowserSignIn = (): Promise<BrowserSignIn> => invoke('cloud_start_browser_login');
export const pollBrowserSignIn = (flowId: string): Promise<CloudStatus | null> => invoke('cloud_poll_browser_login', {flowId});
export const cancelBrowserSignIn = (flowId: string): Promise<void> => invoke('cloud_cancel_browser_login', {flowId});
