import { useState } from 'react';
import { Download, FolderOpen } from 'lucide-react';
import { invoke } from '@tauri-apps/api/core';

type Receipt = { filename: string; eventCount: number; warningCount: number };
export function UsageDiagnostics({ disabled }: { disabled: boolean }) {
  const today = new Date().toISOString().slice(0, 10);
  const [from, setFrom] = useState(() => new Date(Date.now() - 6 * 86400000).toISOString().slice(0, 10));
  const [through, setThrough] = useState(today);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [receipt, setReceipt] = useState<Receipt | null>(null);
  const exportFile = async () => {
    setBusy(true); setError(''); setReceipt(null);
    try { setReceipt(await invoke<Receipt>('export_usage_diagnostics', { from, through })); }
    catch (reason) { setError(typeof reason === 'string' ? reason : 'Diagnostics could not be saved. Try again.'); }
    finally { setBusy(false); }
  };
  const openFolder = async () => {
    try { await invoke('open_diagnostics_folder'); }
    catch (reason) { setError(typeof reason === 'string' ? reason : 'Open Downloads from your file manager.'); }
  };
  return <section className="usage-diagnostics" aria-labelledby="diagnostics-title">
    <h2 id="diagnostics-title">Usage Diagnostics</h2>
    <p className="cloud-note">Investigate duplicate Claude and Codex usage, including Synara accounts. Choose the dates requested by support (UTC, up to 90 days).</p>
    <div className="diagnostics-dates">
      <label>From (UTC)<input type="date" value={from} max={through || today} disabled={disabled || busy} onChange={event => setFrom(event.target.value)} /></label>
      <label>Through (UTC)<input type="date" value={through} min={from} max={today} disabled={disabled || busy} onChange={event => setThrough(event.target.value)} /></label>
    </div>
    <p className="cloud-note">Includes request times, token breakdowns, costs and hashed identifiers. No prompts, replies, credentials, email addresses, project names or local paths. Only logs still on this device can be included.</p>
    <p className="cloud-note">Saves a JSON file to Downloads. Nothing is uploaded. Review the file before sharing it with support.</p>
    <button type="button" className="button secondary" disabled={disabled || busy || !from || !through} onClick={() => void exportFile()}><Download size={16} />{busy ? 'Preparing Diagnostics…' : 'Export Diagnostics'}</button>
    {receipt && <div role="status" className="diagnostics-result">
      <p>Saved {receipt.eventCount.toLocaleString()} events to Downloads: <strong>{receipt.filename}</strong></p>
      {receipt.eventCount === 0 && <p>No matching events were found. Check the dates and saved Usage Sources.</p>}
      {receipt.warningCount > 0 && <p>Some sources could not be read. This export is incomplete; check Usage Sources and folder access.</p>}
      <button type="button" className="button secondary" onClick={() => void openFolder()}><FolderOpen size={16} />Open Downloads</button>
    </div>}
    {error && <p className="cloud-error" role="alert">{error}</p>}
  </section>;
}
