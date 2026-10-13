import type { ReactNode } from 'react';
import { useId } from 'react';

/** Uses drawn controls so Linux system checkbox themes cannot hide consent. */
export function CloudSyncControl({ enabled, busy, saving = false, onChange, children }: {
  enabled: boolean; busy: boolean; saving?: boolean; onChange: (enabled: boolean) => void; children?: ReactNode;
}) {
  const description = useId();
  return <section className="cloud-sync-card" aria-label="Cloud Sync Settings" aria-busy={saving}>
    <div className="cloud-sync-heading">
      <div><h2>Cloud Sync</h2><span>{enabled ? 'Saving usage from this device' : 'Usage stays on this device'}</span></div>
      <button type="button" className={'cloud-sync-switch' + (enabled ? ' is-on' : '')}
        role="switch" aria-checked={enabled} aria-label="Save Usage to Cloud" aria-describedby={description}
        disabled={busy} onClick={() => onChange(!enabled)}>
        <span className="cloud-sync-switch-label">{enabled ? 'On' : 'Off'}</span><span className="cloud-sync-track"><span /></span>
      </button>
    </div>
    <div id={description} className="cloud-sync-description">
      <p>Save usage across devices and convert eligible tokens into Build Credits.</p>
    </div>
    {saving && <p className="cloud-sync-hint" role="status">Saving your preference…</p>}
    {children}
    <details className="cloud-privacy-details"><summary>What Gets Shared?</summary><p>Usage totals, models, times and estimated costs. Prompts, local paths and provider credentials stay on this device. Public rankings require a separate opt-in.</p></details>
  </section>;
}
