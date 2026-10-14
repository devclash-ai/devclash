import { useCallback, useEffect, useRef, useState } from 'react';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { cancelBrowserSignIn, openCloudPage, pollBrowserSignIn, readCloud, startBrowserSignIn } from '../lib/cloud';
import type { BrowserSignIn, CloudStatus } from '../lib/cloud';
import { joinRefreshInterval, joinRefreshLimit } from '../lib/leaderboardJoin';

export function useLeaderboardJoin(enabled: boolean, status: CloudStatus, onChange: (status: CloudStatus) => void, onShow: () => void) {
  const [request, setRequest] = useState<BrowserSignIn | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [attempt, setAttempt] = useState(0);
  const current = useRef(status); current.current = status;
  const change = useRef(onChange); change.current = onChange;
  const starting = useRef(false);
  const mounted = useRef(true);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  const refresh = useCallback(async () => {
    try {
      const next = await readCloud();
      if (mounted.current) { change.current(next); setError(''); }
    } catch (reason) {
      console.error('[Leaderboard Join] STATUS_FAILED');
      if (mounted.current) setError(typeof reason === 'string' ? reason : 'Your leaderboard status could not be checked. Return here to try again.');
    }
  }, []);

  // Keep checking the secure device exchange when its dialog is closed. Native code activates the disclosed sync consent only after a matching server join receipt.
  useEffect(() => {
    if (!request) return;
    let stopped = false;
    let timer: ReturnType<typeof setTimeout>;
    async function poll() {
      try {
        if (Date.now() >= Date.parse(request!.expiresAt)) throw new Error('Connection expired. Continue joining to get a new code.');
        const next = await pollBrowserSignIn(request!.flowId);
        if (stopped) return;
        if (next) { setRequest(null); change.current(next); setAttempt(value => value + 1); return; }
        timer = setTimeout(() => void poll(), 2500);
      } catch (reason) {
        if (stopped) return;
        console.error('[Leaderboard Join] CONNECTION_FAILED');
        setError(typeof reason === 'string' ? reason : reason instanceof Error ? reason.message : 'Connection could not be checked. Continue joining to try again.');
        setRequest(null);
      }
    }
    timer = setTimeout(() => void poll(), 2500);
    return () => { stopped = true; clearTimeout(timer); };
  }, [request]);

  useEffect(() => {
    if (!enabled || !status.joinPending || !status.user) return;
    let stopped = false;
    let checks = 0;
    let timer: ReturnType<typeof setTimeout>;
    const check = async () => {
      if (stopped || !current.current.joinPending || checks >= joinRefreshLimit) return;
      checks += 1;
      await refresh();
      if (!stopped) timer = setTimeout(() => void check(), joinRefreshInterval);
    };
    void check();
    return () => { stopped = true; clearTimeout(timer); };
  }, [enabled, status.joinPending, status.user?.id, attempt, refresh]);

  useEffect(() => {
    if (!enabled) return;
    const listener = getCurrentWindow().onFocusChanged(({ payload: focused }) => {
      if (focused && current.current.joinPending && current.current.user) void refresh();
    }).catch(() => {
      console.error('[Leaderboard Join] FOCUS_LISTENER_FAILED');
      if (mounted.current) setError('Return detection is unavailable. Continue joining to check your status again.');
      return () => {};
    });
    return () => { void listener.then(off => off()); };
  }, [enabled, refresh]);

  const start = async () => {
    onShow();
    if (!enabled || starting.current || request) return;
    starting.current = true; setBusy(true); setError('');
    try {
      const options = { enableSyncOnJoin: !current.current.syncEnabled, joinReceipt: crypto.randomUUID() };
      if (current.current.user) {
        await openCloudPage('join-rank', options);
        await refresh();
        setAttempt(value => value + 1);
      } else {
        const next = await startBrowserSignIn('join-leaderboard', options);
        if (mounted.current) { setRequest(next); await refresh(); }
        else await cancelBrowserSignIn(next.flowId);
      }
    } catch (reason) {
      console.error('[Leaderboard Join] OPEN_FAILED');
      // The native store saves intent before opening the browser; keep its retry state even if opening fails.
      await refresh();
      if (mounted.current) setError(typeof reason === 'string' ? reason : 'The join page could not be opened. Continue joining to try again.');
    } finally {
      starting.current = false; if (mounted.current) setBusy(false);
    }
  };
  return { request, busy, error, start };
}
