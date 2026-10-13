import { CloudSyncControl } from './CloudSyncControl';
import { cloudSyncTone } from '../lib/cloudStatus';
import { cloudSyncLabel } from '../lib/cloudStatus';
import { BrowserSignIn } from './BrowserSignIn';
import { ExternalLink, RefreshCw, Trophy, X } from 'lucide-react';
import { useState } from 'react';
import { native } from '../lib/desktop';
import { dismissProjectWarning, openCloudPage, refreshCloudProjects, setActiveCloudProject, setCloudSync, setParticipation, signOutCloud, syncCloud } from '../lib/cloud';
import type { CloudStatus } from '../lib/cloud';

export function CloudDialog({ theme = 'light', type, status, onChange, onView, onClose }: {
  theme?: 'light' | 'dark'; type: 'account' | 'rank'; status: CloudStatus; onChange: (status: CloudStatus) => void; onView: (type: 'account' | 'rank' | 'sources' | 'updates') => void; onClose: () => void;
}) {
  const [busy, setBusy] = useState(false);
  const [syncSaving, setSyncSaving] = useState(false); const [error, setError] = useState('');
  const [consent, setConsent] = useState(false);
  const [projectChoice, setProjectChoice] = useState(status.activeProjectId ?? '');
  const run = async (action: () => Promise<void>) => {
    setBusy(true); setError('');
    try { await action(); } catch (error) { console.error('Cloud action failed'); setError(typeof error === 'string' ? error : 'The cloud request could not be completed. Please try again.'); }
    finally { setBusy(false); }
  };
  const update = async (action: () => Promise<CloudStatus>) => { onChange(await action()); };
  const changeSync = (enabled: boolean) => {
    setSyncSaving(true);
    void run(() => update(() => setCloudSync(enabled))).finally(() => setSyncSaving(false));
  };
  const lastSynced = status.lastSyncedAt ? new Date(status.lastSyncedAt).toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' }) : 'Not Completed Yet';
  const selectableProjects = status.projects.filter(project => !project.archived && !project.moderated);
  const selectedProject = selectableProjects.some(project => project.id === projectChoice) ? projectChoice : '';
  const activeProject = status.projects.find(project => project.id === status.activeProjectId);

  return <div className="dialog-backdrop" onClick={onClose}><section className={'cloud-dialog cloud-account-dialog theme-' + theme} role="dialog" aria-modal="true" aria-labelledby="cloud-dialog-title" onClick={event => event.stopPropagation()}>
    <button className="icon-button dialog-close" aria-label="Close Dialog" onClick={onClose}><X size={22} /></button>
    {type === 'rank' && <div className="dialog-icon"><Trophy size={32} /></div>}
    <h1 id="cloud-dialog-title">{type === 'rank' ? 'Rank Battle' : status.user ? 'Your Account' : 'Your Usage, Yours to Keep'}</h1>
    {!native || !status.configured ? <>
      <p>{!native ? 'The browser gallery previews the desktop design. Sign in and upload real usage from the installed DevClash app.' : 'DevClash works without an account. This build has no cloud address configured; local tracking and your saved display preferences remain available.'}</p>
      <div className="cloud-status"><i className="status-dot" />Local Tracking Is Active</div>
      <button className="button primary" onClick={onClose}>Got It</button>
    </> : type === 'rank' ? <>
      <p>Compare measured tokens by UTC day, week, month, and model. Participation is optional; uploaded history stays private until you choose to join.</p>
      <div className="cloud-status"><i className="status-dot" />{status.ranked ? status.rank ? `#${status.rank} ${status.rankPeriod} · UTC` : 'Rank Battle Is Enabled' : 'Public Ranking Is Off'}</div>
      {status.user && !status.ranked && <>
        <label className="cloud-option"><input type="checkbox" checked={consent} disabled={busy} onChange={event => setConsent(event.target.checked)} /><span><strong>Join Public Rankings</strong><small>Show my ranking identity and measured token usage. Public profile visibility is controlled separately. Earlier private history does not count toward new rankings.</small></span></label>
        <button className="button primary" disabled={!consent || busy} onClick={() => void run(() => update(() => setParticipation(true, status.publicProfile)))}>Join Rank Battle</button>
      </>}
      {status.ranked && <button className="button" disabled={busy} onClick={() => void run(() => update(() => setParticipation(false, status.publicProfile)))}>Leave Public Rankings</button>}
      {!status.user && <><p className="muted">Sign in to join. You can browse the public leaderboard now.</p><button className="button" disabled={busy} onClick={() => onView('account')}>Sign In</button></>}
      {status.user && !status.syncEnabled && <p className="cloud-note">Enable Cloud Sync in your account to upload measured usage. Joining alone does not upload local records.</p>}
      <button className="button cloud-link" disabled={busy} onClick={() => void run(() => openCloudPage('rank'))}>Open Leaderboard<ExternalLink size={15} /></button>
    </> : status.user ? <>
      <p className="cloud-email">{status.user.email}</p>
      <CloudSyncControl enabled={status.syncEnabled} busy={busy} saving={syncSaving} onChange={changeSync}>
      <div className={"cloud-status sync-tone-" + cloudSyncTone(status)} role="status"><i className="status-dot" />{syncSaving ? 'Updating Cloud Sync…' : cloudSyncLabel(status)}</div>
      <div className="cloud-sync-meta"><span>Last Synced</span><span>{lastSynced}</span></div>
      {(status.pendingCount > 0 || status.nextRetryAt) && <p className="cloud-note">{status.pendingCount.toLocaleString()} records pending{status.nextRetryAt ? ` · Retry at ${new Date(status.nextRetryAt).toLocaleTimeString()}` : ''}</p>}
      {status.syncEnabled && <button className="button cloud-sync-now" disabled={busy} onClick={() => void run(() => update(syncCloud))}><RefreshCw size={15} className={busy ? 'spinning' : undefined} />Sync Now</button>}
      </CloudSyncControl>
      <details className="cloud-project-context" open={status.activeProjectId || status.projectWarning ? true : undefined}>
        <summary><span>Project Attribution<small>{status.activeProjectId ? activeProject?.title ?? 'Active Session' : 'Optional · No Active Session'}</small></span></summary>
        <p className="cloud-note">{status.activeProjectId ? `Assigning new usage to ${activeProject?.title ?? 'your selected project'} until you switch or stop.` : 'New usage is unassigned until you start a project session.'}</p>
        <label htmlFor="cloud-project-choice">Choose an Owned Project</label>
        <select id="cloud-project-choice" value={selectedProject} disabled={busy || selectableProjects.length === 0} onChange={event => setProjectChoice(event.target.value)}>
          <option value="">Select a Project</option>
          {selectableProjects.map(project => <option key={project.id} value={project.id}>{project.title}</option>)}
        </select>
        <div className="cloud-actions">
          <button className="button" disabled={busy || !selectedProject || selectedProject === status.activeProjectId} onClick={() => void run(() => update(() => setActiveCloudProject(selectedProject)))}>{status.activeProjectId ? 'Switch Project' : 'Start Project Session'}</button>
          {status.activeProjectId && <button className="button" disabled={busy} onClick={() => void run(() => update(() => setActiveCloudProject(null)))}>Stop Session</button>}
          <button className="text-button" disabled={busy} onClick={() => void run(() => update(refreshCloudProjects))}><RefreshCw size={15} className={busy ? 'spinning' : undefined} />Refresh Projects</button>
        </div>
        <p className="cloud-note">Applies to new Claude, Codex and OpenCode usage. Cursor stays unassigned. Saved projects work offline.</p>
        {status.projectWarning && <div className="cloud-project-warning" role="status"><span>{status.projectWarning}</span><button className="text-button" disabled={busy} onClick={() => void run(() => update(dismissProjectWarning))}>Dismiss</button></div>}
      </details>
      <div className="cloud-actions cloud-account-links">
        <button className="button" disabled={busy} onClick={() => void run(() => openCloudPage('profile'))}>Manage Profile<ExternalLink size={15} /></button>
        <button className="button" disabled={busy} onClick={() => onView('rank')}><Trophy size={16} />Rank Battle</button></div>
      <p className="cloud-note">{status.ranked ? 'Public rankings are enabled.' : 'Your history is private. Join Rank Battle separately.'}</p>
      <button className="text-button cloud-signout" disabled={busy} onClick={() => void run(async () => { await update(signOutCloud); setConsent(false); })}>Sign Out of This Device</button>
    </> : <>
      <BrowserSignIn onChange={onChange}/>
    </>}
    {native && <div className="cloud-actions settings-links"><button className="text-button" onClick={() => onView('sources')}>Usage Sources</button><button className="text-button" onClick={() => onView('updates')}>App Updates</button></div>}
    {(error || status.syncError) && <p className="cloud-error" role="alert">{error || status.syncError}</p>}
  </section></div>;
}
