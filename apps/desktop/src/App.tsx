import { cloudSyncLabel, cloudSyncTone } from './lib/cloudStatus';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { emit, listen } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import { Download, X } from 'lucide-react';
import { enabledInOrder, loadPreferences, normalizePreferences, preferencesKey, savePreferences } from './lib/preferences';
import { aggregateUsage, demoSnapshot, emptySnapshot } from './lib/usage';
import { trayContent } from './lib/tray';
import { closeWindow, configureWidget, getAutostart, native, openCustomization, openWidget, readUsage, setAutostart, setTray, shareImage } from './lib/desktop';
import type { Preferences, Surface, UsageSnapshot } from './lib/types';
import type { ProviderID } from './lib/types';
import { retryProviderAuthorization } from './lib/providerAuthorization';
import { leaderboardJoinState } from './lib/leaderboardJoin';
import { useLeaderboardJoin } from './components/useLeaderboardJoin';
import { Dashboard } from './components/Dashboard';
import { FloatingWidget } from './components/Widget';
import { Customization } from './components/Customization';
import { Concepts } from './components/Concepts';
import { CloudDialog } from './components/CloudDialog';
import { SourceDialog } from './components/SourceDialog';
import { UpdatePanel } from './components/UpdatePanel';
import { cloudPeriod, publicRank } from './lib/rank';
import { localCloudStatus, openCloudPage, readCloud, setRankPeriod } from './lib/cloud';
import type { CloudStatus } from './lib/cloud';
import { initialUpdateStatus, readUpdateStatus } from './lib/updates';
import type { UpdateStatus } from './lib/updates';
import { persistAutomaticPreference, withConfirmedAutomaticUpdates } from './lib/automaticPreferences';

const params = new URLSearchParams(location.search);
const windowType = params.get('window') ?? 'main';
const gallery = location.pathname === '/concepts';
const sample = !native && (gallery || params.get('demo') === '1');
export function App() {
  const [prefs, setPrefs] = useState<Preferences>(loadPreferences);
  const [snapshot, setSnapshot] = useState<UsageSnapshot>(() => sample ? demoSnapshot() : { ...emptySnapshot(), source: native ? 'local' : 'browser' });
  const [busy, setBusy] = useState(false);
  const [authorizationPending, setAuthorizationPending] = useState<ProviderID | null>(null);
  const authorizing = useRef(false);
  const [customizing, setCustomizing] = useState(windowType === 'customize');
  const [surface, setSurface] = useState<Surface>((['dashboard', 'widget', 'menubar'].includes(params.get('surface') ?? '') ? params.get('surface') : 'widget') as Surface);
  const [dialog, setDialog] = useState<'account' | 'rank' | 'sources' | 'updates' | null>(null);
  const [cloud, setCloud] = useState<CloudStatus>(localCloudStatus);
  const join = useLeaderboardJoin(native, cloud, setCloud, () => setDialog('rank'));
  const [updates, setUpdates] = useState<UpdateStatus>(() => initialUpdateStatus(prefs.autoUpdates));
  const [updateStatusKnown, setUpdateStatusKnown] = useState(!native);
  const [automaticPending, setAutomaticPending] = useState(false);
  const [toast, setToast] = useState('');
  const [systemDark, setSystemDark] = useState(matchMedia('(prefers-color-scheme: dark)').matches);
  const latestPreferences = useRef(prefs); latestPreferences.current = prefs;
  const latestUpdates = useRef(updates); latestUpdates.current = updates;
  const preferenceWrites = useRef<Promise<void>>(Promise.resolve());
  const automaticWrite = useRef(false);
  const notify = useCallback((message: string) => { setToast(message); }, []);
  const receiveUpdateStatus = useCallback((status: UpdateStatus) => { latestUpdates.current = status; setUpdates(status); setUpdateStatusKnown(true); }, []);
  const confirmPreferences = useCallback((next: Preferences) => {
    latestPreferences.current = next;
    setPrefs(next); try { savePreferences(next); } catch (error) { console.error(error); notify('Your preferences could not be saved. Check the available disk space.'); }
    if (native) void emit('preferences-updated', next).catch(console.error);
  }, [notify]);
  const enqueuePreferenceWrite = useCallback((write: () => Promise<void>) => {
    const pending = preferenceWrites.current.then(write);
    preferenceWrites.current = pending.catch(() => {});
    return pending;
  }, []);
  const changeAutomaticUpdates = useCallback((enabled: boolean) => {
    if (!native) { confirmPreferences({ ...latestPreferences.current, autoUpdates: enabled }); return; }
    if (automaticWrite.current || !updateStatusKnown) return;
    automaticWrite.current = true; setAutomaticPending(true);
    void enqueuePreferenceWrite(() => persistAutomaticPreference(enabled, () => latestPreferences.current,
      preferences => invoke<void>('save_preferences', { preferences, automaticUpdates: enabled }), preferences => {
        receiveUpdateStatus({ ...latestUpdates.current, enabled }); confirmPreferences(preferences);
      })).catch(error => {
      console.error('Automatic update preference could not be saved', error);
      notify('Automatic updates could not be changed. Your previous setting is still active. Check the available disk space and try again.');
    }).finally(() => { automaticWrite.current = false; setAutomaticPending(false); });
  }, [confirmPreferences, enqueuePreferenceWrite, notify, receiveUpdateStatus, updateStatusKnown]);
  const changePreferences = useCallback((next: Preferences) => {
    const confirmed = native ? withConfirmedAutomaticUpdates(next, latestUpdates.current.enabled) : next;
    confirmPreferences(confirmed);
    if (native) void enqueuePreferenceWrite(() => invoke<void>('save_preferences', { preferences: withConfirmedAutomaticUpdates(confirmed, latestUpdates.current.enabled) })).catch(error => {
      console.error('Preferences could not be saved', error); notify('Your settings could not be saved on this device.');
    });
  }, [confirmPreferences, enqueuePreferenceWrite, notify]);
  const theme = prefs.theme === 'system' ? systemDark ? 'dark' : 'light' : prefs.theme;
  const totals = useMemo(() => aggregateUsage(snapshot, prefs.period, enabledInOrder(prefs)), [snapshot, prefs.period, prefs.providers, prefs.enabledProviders]);
  const displayedSnapshot = useMemo(() => native && !sample ? { ...snapshot, ranked: cloud.ranked, rankUnavailable: !!cloud.syncError,
    rank: publicRank(cloud, prefs.period),
  } : snapshot, [snapshot, cloud, prefs.period]);

  const refresh = useCallback(async (force = true) => {
    if (!native) return;
    setBusy(true);
    try { setSnapshot(await readUsage(force)); }
    catch (error) { console.error(error); notify('Usage could not be refreshed. Try again.'); }
    finally { setBusy(false); }
  }, [notify]);
  useEffect(() => {
    const media = matchMedia('(prefers-color-scheme: dark)'); const changed = () => setSystemDark(media.matches);
    media.addEventListener('change', changed); return () => media.removeEventListener('change', changed);
  }, []);
  useEffect(() => {
    const changed = (event: StorageEvent) => { if (event.key === preferencesKey) setPrefs(loadPreferences()); };
    addEventListener('storage', changed); return () => removeEventListener('storage', changed);
  }, []);
  useEffect(() => { document.documentElement.dataset.theme = theme; document.documentElement.dataset.window = gallery ? 'gallery' : windowType; }, [theme]);
  useEffect(() => {
    if (!native) return;
    const preferences = listen<Preferences>('preferences-updated', event => setPrefs(event.payload));
    const displayErrors = listen<string>('display-error', event => notify(event.payload));
    const surfaces = listen<Surface>('customize-surface', event => setSurface(event.payload));
    return () => { void preferences.then(off => off()); void surfaces.then(off => off()); void displayErrors.then(off => off()); };
  }, []);
  useEffect(() => {
    if (!native) return;
    let disposed = false; let receivedStatus = false;
    const statusListener = listen<UpdateStatus>('update-status', event => {
      receivedStatus = true; if (!disposed) receiveUpdateStatus(event.payload);
    });
    void statusListener.then(async () => {
      const status = await readUpdateStatus();
      if (!disposed && !receivedStatus) receiveUpdateStatus(status);
    }).catch(error => {
      console.error('App update status could not be read', error);
      if (!disposed && !receivedStatus) setUpdates(current => ({ ...current, phase: 'error', error: 'Update status could not be read. Try checking again.' }));
    });
    const openListener = windowType === 'main' ? listen('open-app-updates', () => { setCustomizing(false); setDialog('updates'); }) : null;
    void openListener?.catch(error => { console.error('App Updates could not be opened from the menu', error); notify('App Updates could not be opened from the menu. Open it from your account settings.'); });
    return () => { disposed = true; void statusListener.then(off => off()).catch(console.error); void openListener?.then(off => off()).catch(console.error); };
  }, [notify, receiveUpdateStatus]);
  useEffect(() => {
    if (!native) return;
    void refresh(false);
    const unlisten = listen<UsageSnapshot>('usage-updated', event => setSnapshot(event.payload));
    const timer = setInterval(() => void refresh(false), 60000);
    return () => { clearInterval(timer); void unlisten.then(off => off()); };
  }, [refresh]);
  useEffect(() => {
    if (!native) return;
    const update = () => void readCloud().then(setCloud).catch(error => {
      console.error('Cloud status could not be read'); setCloud(current => ({ ...current, syncError: typeof error === 'string' ? error : 'Your cloud account could not be read. Try again.' }));
    });
    update(); const listener = listen<CloudStatus>('cloud-updated', event => setCloud(event.payload));
    const timer = setInterval(update, 300000);
    return () => { clearInterval(timer); void listener.then(off => off()); };
  }, []);
  useEffect(() => {
    if (!native || !cloud.user || cloud.rankPeriod === cloudPeriod(prefs.period)) return;
    void setRankPeriod(cloudPeriod(prefs.period)).then(setCloud).catch(error => notify(typeof error === 'string' ? error : 'Public rank could not be refreshed.'));
  }, [prefs.period, cloud.user?.id, cloud.rankPeriod, notify]);
  useEffect(() => {
    if (!native) return;
    void (async () => {
      const saved = await invoke<unknown>('read_preferences');
      if (saved) { const restored = normalizePreferences(saved); setPrefs(restored); savePreferences(restored); }
      const desired = saved ? normalizePreferences(saved).launchAtLogin : loadPreferences().launchAtLogin;
      const current = await getAutostart();
      if (desired !== current) await setAutostart(desired);
    })().catch(error => { console.error('Display settings or autostart migration failed'); notify(typeof error === 'string' ? error : 'Your saved display settings could not be restored.'); });
  }, [notify]);
  useEffect(() => {
    if (!native || windowType !== 'widget') return;
    void configureWidget(prefs.alwaysOnTop).catch(error => { console.error(error); notify('The widget window could not be updated.'); });
    const element = document.querySelector('.floating-widget'); if (!element) return;
    const observer = new ResizeObserver(entries => {
      const size = entries[0].borderBoxSize[0]; const bounds = element.getBoundingClientRect();
      const width = Math.ceil(size?.inlineSize ?? bounds.width); const height = Math.ceil(size?.blockSize ?? bounds.height);
      void invoke('fit_widget', { width, height }).catch(error => { console.error(error); notify('The widget window could not be resized.'); });
    }); observer.observe(element); return () => observer.disconnect();
  }, [prefs.alwaysOnTop, customizing, notify]);
  useEffect(() => {
    if (!native || windowType !== 'main') return;
    const { label, rows, tooltip } = trayContent(prefs, snapshot, totals);
    void setTray(label, rows, tooltip).catch(error => { console.error(error); notify('The menu bar could not be updated. Try changing the view again.'); });
  }, [totals, prefs, snapshot]);
  useEffect(() => { if (!toast) return; const timer = setTimeout(() => setToast(''), 6000); return () => clearTimeout(timer); }, [toast]);
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => { if (event.key === 'Escape') { if (dialog) setDialog(null); else if (customizing && windowType !== 'customize') setCustomizing(false); } };
    addEventListener('keydown', onKey); return () => removeEventListener('keydown', onKey);
  }, [dialog, customizing]);
  const customize = async (selectedSurface: Surface = 'widget') => {
    setSurface(selectedSurface);
    if (native) { try { await openCustomization(selectedSurface); } catch (error) { console.error(error); notify('Customization could not be opened.'); } }
    else setCustomizing(true);
  };
  const widget = async () => { try { await openWidget(prefs.widgetSize); } catch (error) { console.error(error); notify('The floating widget could not be opened.'); } };
  const hideWidget = async () => {
    try { if (native) await closeWindow(); else window.close(); }
    catch (error) { console.error(error); notify('The widget could not be hidden. Try again.'); }
  };
  const share = async (element: HTMLElement) => { try { await shareImage(element); notify('Usage image copied. Paste it into your social post.'); } catch (error) { console.error(error); notify('The share image could not be copied. Please try again.'); } };
  const autostart = async (enabled: boolean) => {
    try { await setAutostart(enabled); changePreferences({ ...prefs, launchAtLogin: enabled }); }
    catch (error) { console.error(error); notify('Launch at login could not be changed.'); }
  };
  const retryAuthorization = async (id: ProviderID) => {
    if (authorizing.current) return;
    authorizing.current = true; setAuthorizationPending(id);
    try { await retryProviderAuthorization(id, source => invoke('authorize_provider_login', { source }), () => refresh()); }
    catch (error) {
      console.error('Provider authorization could not finish', error);
      notify(typeof error === 'string' ? error : error instanceof Error ? error.message : 'Provider authorization could not finish. Try again.');
    } finally { authorizing.current = false; setAuthorizationPending(null); }
  };
  const viewLeaderboard = () => { void openCloudPage('rank').catch(error => notify(typeof error === 'string' ? error : 'The leaderboard could not be opened. Try again.')); };
  const done = () => { if (windowType === 'customize') void closeWindow(); else setCustomizing(false); };

  return <>
    {gallery ? <Concepts preferences={prefs} snapshot={snapshot} totals={totals} onChange={changePreferences} onShare={element => void share(element)} onRank={() => setDialog('rank')} onAccount={() => setDialog('account')} onWidget={() => void widget()} /> :
      customizing ? <Customization key={surface} preferences={prefs} snapshot={displayedSnapshot} totals={totals} theme={theme} initialSurface={surface} onChange={changePreferences} onDone={done} onAutostart={enabled => void autostart(enabled)} onWidget={() => void widget()} automaticUpdates={{ enabled: native ? updates.enabled : prefs.autoUpdates, pending: automaticPending || !updateStatusKnown, installMode: updates.installMode, restartRequired: updates.restartRequired }} onAutomaticChange={changeAutomaticUpdates} /> :
      windowType === 'widget' ? <><FloatingWidget preferences={prefs} snapshot={displayedSnapshot} totals={totals} theme={theme} setPeriod={period => changePreferences({ ...prefs, period })} onCustomize={() => void customize('widget')} onShare={element => void share(element)} onRank={() => setDialog('rank')} onHide={() => void hideWidget()} windowDraggable={native} onDragError={() => notify('The widget could not be moved. Try dragging its header again.')} />{sample && <span className="standalone-sample">Sample Data · Design Preview</span>}</> :
      <Dashboard updateNotice={native && updates.phase === 'ready' && updates.restartRequired ? <div className="update-ready-notice" role="status" data-no-share="true"><Download size={17} aria-hidden="true" /><span>DevClash {updates.availableVersion} Is Ready</span><button className="text-button" onClick={() => setDialog('updates')}>Review &amp; Restart</button></div> : undefined} joinState={leaderboardJoinState(cloud, !!displayedSnapshot.rank)} joinBusy={join.busy} onJoin={() => void join.start()} onViewLeaderboard={viewLeaderboard} preferences={prefs} snapshot={displayedSnapshot} totals={totals} theme={theme} period={prefs.period} busy={busy} connected={!!cloud.user} syncEnabled={cloud.syncEnabled} syncTone={cloudSyncTone(cloud)} cloudLabel={cloudSyncLabel(cloud)} accountInitial={cloud.user?.email.charAt(0).toUpperCase()} setPeriod={period => changePreferences({ ...prefs, period })} onCustomize={() => void customize('dashboard')} onRefresh={() => void refresh()} onAccount={() => setDialog('account')} onWidget={() => void widget()} onShare={element => void share(element)} onRank={() => setDialog('rank')} onRetryAuthorization={native ? id => void retryAuthorization(id) : undefined} authorizationPending={authorizationPending} />}
    {dialog === 'sources' ? <SourceDialog snapshot={snapshot} onRefresh={() => refresh()} onClose={() => setDialog(null)} /> :
      dialog === 'updates' ? <UpdatePanel status={updates} autoUpdates={prefs.autoUpdates} automaticPending={automaticPending || !updateStatusKnown} onAutomaticChange={changeAutomaticUpdates} onStatus={receiveUpdateStatus} onClose={() => setDialog(null)} /> :
      dialog && <CloudDialog theme={theme} key={dialog} type={dialog} status={cloud} join={join} onChange={setCloud} onView={setDialog} onClose={() => setDialog(null)} />}
    {toast && <div className="toast" role="status">{toast}<button className="icon-button" aria-label="Dismiss Notification" onClick={() => setToast('')}><X size={16} /></button></div>}
  </>;
}
