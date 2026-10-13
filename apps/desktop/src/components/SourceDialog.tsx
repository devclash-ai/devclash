import { Folder, X } from 'lucide-react';
import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { providerIDs, providerNames } from '../lib/preferences';
import type { ProviderID, UsageSnapshot } from '../lib/types';
type Sources = Record<ProviderID, string[]>;
const empty: Sources = { claude: [], codex: [], cursor: [], opencode: [] };
const loginOptions: Partial<Record<ProviderID, { source: string; label: string }[]>> = {
  claude: [{ source: 'claude-desktop', label: 'Authorize Claude Desktop' }, { source: 'claude-code', label: 'Authorize Claude Code' }],
  cursor: [{ source: 'cursor-cli', label: 'Authorize Cursor CLI' }],
  codex: [{ source: 'codex', label: 'Authorize Codex' }],
};
export function ProviderSourceField({ id, value, detected, snapshot, platform, busy, onChange, onAuthorize }: {
  id: ProviderID; value: string; detected: string[]; snapshot: UsageSnapshot; platform: string; busy: boolean;
  onChange: (value: string) => void; onAuthorize: (source: string) => void;
}) {
  const canAuthorize = !value.trim() && (platform === 'macos' || (!platform && snapshot.quotaErrors?.[id]?.includes('Keychain access is required')));
  const fieldID = `source-folder-${id}`;
  return <div className="source-field">
    <label htmlFor={fieldID}><strong>{providerNames[id]}</strong></label>
    <textarea id={fieldID} rows={2} value={value} disabled={busy} onChange={event => onChange(event.target.value)} placeholder="Automatic Discovery" aria-label={providerNames[id] + ' Source Folders'} />
    <small className="muted">Reading: {detected.length ? detected.join(' · ') : 'N/A'}</small>
    {snapshot.providerErrors?.[id] && <small className="muted" role="status">Usage: {snapshot.providerErrors[id]}</small>}
    {snapshot.quotaErrors?.[id] && <small className="muted">Quota: {snapshot.quotaErrors[id]}</small>}
    {canAuthorize && loginOptions[id] && <span className="cloud-actions">{loginOptions[id].map(option =>
      <button key={option.source} type="button" className="text-button" aria-label={option.label} disabled={busy} onClick={() => onAuthorize(option.source)}>{option.label}</button>
    )}</span>}
  </div>;
}
export function SourceDialog({ snapshot, onRefresh, onClose }: { snapshot: UsageSnapshot; onRefresh: () => Promise<void>; onClose: () => void }) {
  const [sources, setSources] = useState<Record<ProviderID, string>>({ claude: '', codex: '', cursor: '', opencode: '' });
  const [detected, setDetected] = useState<Sources>(empty);
  const [platform, setPlatform] = useState('');
  const [busy, setBusy] = useState(true); const [error, setError] = useState('');
  const load = async () => {
    const settings = await invoke<{ sources: Sources; detected: Sources; platform: string }>('read_sources');
    setSources(Object.fromEntries(providerIDs.map(id => [id, settings.sources[id].join('\n')])) as Record<ProviderID, string>);
    setDetected(settings.detected);
    setPlatform(settings.platform);
  };
  useEffect(() => { void load().catch(reason => setError(typeof reason === 'string' ? reason : 'Usage sources could not be read.')).finally(() => setBusy(false)); }, []);
  const save = async () => {
    setBusy(true); setError('');
    try {
      const selected = Object.fromEntries(providerIDs.map(id => [id, sources[id].split(/\r?\n/).map(path => path.trim()).filter(Boolean)]));
      await invoke('save_sources', { sources: selected }); await onRefresh(); await load();
    } catch (reason) { setError(typeof reason === 'string' ? reason : 'Usage sources could not be saved.'); }
    finally { setBusy(false); }
  };
  const authorize = async (source: string) => {
    setBusy(true); setError('');
    try { await invoke('authorize_provider_login', { source }); await onRefresh(); }
    catch (reason) { setError(typeof reason === 'string' ? reason : 'Provider authorization could not finish.'); }
    finally { setBusy(false); }
  };
  return <div className="dialog-backdrop" onClick={onClose}><section className="cloud-dialog source-dialog" role="dialog" aria-modal="true" aria-labelledby="source-title" onClick={event => event.stopPropagation()}>
    <button className="icon-button dialog-close" aria-label="Close Dialog" onClick={onClose}><X size={22} /></button>
    <div className="dialog-icon"><Folder size={32} /></div><h1 id="source-title">Usage Sources</h1>
    <p>Choose provider folders on this computer. Use one folder per line; leave a field empty to discover its normal location.</p>
    <p className="cloud-note">For WSL, use a Windows path such as <code>{'\\\\wsl.localhost\\Ubuntu\\home\\you\\.codex'}</code>. Select a Claude config folder, a Codex home, a Cursor app-data or CLI folder, or an OpenCode data folder. Paths and credentials stay on this device.</p>
    {providerIDs.map(id => <ProviderSourceField key={id} id={id} value={sources[id]} detected={detected[id]} snapshot={snapshot} platform={platform} busy={busy}
      onChange={value => setSources(current => ({ ...current, [id]: value }))} onAuthorize={source => { void authorize(source); }} />)}
    <p className="cloud-note">Cursor usage is read automatically from your signed-in Cursor account. Provider credentials stay on this device. OpenCode quota is available for Go plans only.</p>
    {error && <p className="cloud-error" role="alert">{error}</p>}
    <button className="button primary" disabled={busy} onClick={() => void save()}>{busy ? 'Please Wait…' : 'Save and Refresh'}</button>
  </section></div>;
}
