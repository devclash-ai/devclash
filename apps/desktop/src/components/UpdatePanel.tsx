import { Download, RefreshCw, X } from 'lucide-react';
import { useState } from 'react';
import { native } from '../lib/desktop';
import { checkForUpdates, installUpdate, restartAfterUpdate } from '../lib/updates';
import type { UpdateStatus } from '../lib/updates';
import { automaticUpdateDescription, automaticUpdateSummary } from '../lib/updateCopy';
import { Toggle } from './Common';

interface Props {
  status: UpdateStatus;
  autoUpdates: boolean;
  automaticPending: boolean;
  onAutomaticChange: (enabled: boolean) => void;
  onStatus: (status: UpdateStatus) => void;
  onClose: () => void;
}

export function UpdatePanel({ status, autoUpdates, automaticPending, onAutomaticChange, onStatus, onClose }: Props) {
  const [pending, setPending] = useState(false);
  const [actionError, setActionError] = useState('');
  const busy = pending || ['checking', 'downloading', 'installing'].includes(status.phase);
  const automaticEnabled = native ? status.enabled : autoUpdates;
  const progress = status.progress === null ? null : Math.max(0, Math.min(100, Math.round(status.progress)));
  const message = !native ? 'Update checks and installation are available in the desktop app.' :
    status.phase === 'checking' ? 'Checking for Updates…' :
    status.phase === 'downloading' ? `Downloading ${status.availableVersion ?? 'Update'}${progress === null ? '…' : `: ${progress}%`}` :
    status.phase === 'installing' ? status.restartRequired ? 'Installing Update…' : 'Installing Update… The Installer Will Close and Reopen DevClash.' :
    status.phase === 'ready' ? `Version ${status.availableVersion ?? ''} ${status.restartRequired ? 'Is Ready. Restart to Finish.' : 'Is Installed.'}` :
    status.phase === 'available' ? `Version ${status.availableVersion ?? ''} Is Available.` :
    status.phase === 'idle' && status.checkedAt ? 'You’re Up to Date.' :
    status.phase === 'idle' ? automaticEnabled ? 'New versions are checked automatically while DevClash is running.' : 'Check for an update whenever you’re ready.' : '';
  const action = async (run: () => Promise<UpdateStatus | void>, fallback: string) => {
    setPending(true); setActionError('');
    try { const next = await run(); if (next) onStatus(next); }
    catch (error) { console.error('App update action failed', error); setActionError(typeof error === 'string' ? error : fallback); }
    finally { setPending(false); }
  };
  const error = actionError || status.error;
  return <div className="dialog-backdrop" onClick={onClose}><section className="cloud-dialog update-dialog" role="dialog" aria-modal="true" aria-labelledby="update-title" onClick={event => event.stopPropagation()}>
    <button className="icon-button dialog-close" aria-label="Close Dialog" onClick={onClose}><X size={22} /></button>
    <div className="dialog-icon"><Download size={32} /></div><h1 id="update-title">App Updates</h1>
    <p>{native && status.currentVersion ? `DevClash ${status.currentVersion}. ` : ''}Keep DevClash up to date.{status.restartRequired && ' Restart when you’re ready.'}</p>
    <div className="update-setting"><div className="setting-copy"><strong>Automatic Updates</strong><span>{automaticPending ? 'Reading or saving your update preference…' : native ? automaticUpdateSummary(status) : 'Check automatically; install in the background when supported'}</span></div><Toggle enabled={automaticEnabled} disabled={automaticPending} label="Automatic Updates" onChange={() => onAutomaticChange(!automaticEnabled)} /></div>
    <p className="cloud-note">{!native ? 'This preview saves your preference. Open the desktop app to check and install updates.' : automaticUpdateDescription(status)}</p>
    {native && status.installMode === 'manual' && status.availableVersion && status.phase !== 'ready' && <p className="cloud-note">{status.restartRequired ? 'Your system may ask for permission to install the update. Restart DevClash when it is ready.' : 'Installing this update will close DevClash and may ask for system permission. The installer will reopen the app.'}</p>}
    {status.phase === 'installing' && <p className="cloud-note">Turning this off applies to future updates. The current installation is already finishing.</p>}
    {message && <p className="update-status" role="status">{message}</p>}
    {status.phase === 'downloading' && <progress className="update-progress" aria-label="Update Download" max={100} value={progress ?? undefined} />}
    {status.checkedAt && native && <p className="cloud-note">Last checked {new Date(status.checkedAt).toLocaleString()}.</p>}
    {error && <p className="cloud-error" role="alert">{error}</p>}
    {status.notes && <details className="update-release-notes"><summary>Release Notes</summary><p className="update-notes">{status.notes}</p></details>}
    <div className="update-actions">
      {status.phase === 'ready' && status.restartRequired ? <button className="button primary" disabled={!native || pending} onClick={() => void action(restartAfterUpdate, 'Restart could not be started. Quit and reopen DevClash.')}>Restart DevClash</button> : <>
        <button className={'button' + (status.availableVersion ? '' : ' primary')} disabled={!native || busy} onClick={() => void action(checkForUpdates, 'Updates could not be checked. Try again.')}><RefreshCw size={16} className={status.phase === 'checking' ? 'spinning' : undefined} />Check for Updates</button>
        {status.availableVersion && status.phase !== 'ready' && <button className="button primary" disabled={!native || busy} onClick={() => void action(installUpdate, 'The update could not be installed. Try again.')}><Download size={16} />Install {status.availableVersion}</button>}
      </>}
    </div>
  </section></div>;
}
