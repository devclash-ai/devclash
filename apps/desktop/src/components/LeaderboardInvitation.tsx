import { CheckCircle2, ChevronRight, ExternalLink, Settings, Trophy } from 'lucide-react';
import type { JoinState } from '../lib/leaderboardJoin';

export function LeaderboardInvitation({ state, syncEnabled, busy, unavailable, onJoin, onView, onSettings }: {
  state: JoinState; syncEnabled: boolean; busy?: boolean; unavailable?: boolean;
  onJoin: () => void; onView: () => void; onSettings: () => void;
}) {
  if (state === 'ranked') return null;
  const joined = state === 'joined';
  return <div className="leaderboard-invitation" data-no-share="true">
    <section className={'leaderboard-strip ' + (joined ? 'leaderboard-joined' : 'leaderboard-off')} aria-label={joined ? 'Leaderboard Joined' : 'Join The Leaderboard'}>
      <span className="leaderboard-icon">{joined ? <CheckCircle2 size={29} /> : <Trophy size={30} className="trophy" />}</span>
      <div className="leaderboard-copy"><strong>{joined ? 'Leaderboard Joined' : 'Join The Leaderboard'}</strong>
        <p>{joined ? unavailable ? 'Rank Unavailable Offline' : 'Awaiting Your First Eligible Usage' : 'See where your building takes you.'}</p>
        {!joined && <small>{syncEnabled ? 'Only new eligible usage counts.' : 'Joining enables Cloud Sync. New eligible usage needs sync.'}</small>}</div>
      <button className={'button ' + (joined ? '' : 'primary')} disabled={busy} onClick={joined ? onView : onJoin}>
        {joined ? 'View Leaderboard' : busy ? 'Opening Browser…' : state === 'pending' ? 'Continue Joining' : syncEnabled ? 'Join Leaderboard' : 'Join & Enable Sync'}
        {joined ? <ChevronRight size={15} /> : <ExternalLink size={14} />}</button>
    </section>
    {joined && !syncEnabled && <section className="leaderboard-strip leaderboard-sync" aria-label="Cloud Sync Is Off">
      <span className="leaderboard-icon"><Settings size={22} /></span><div className="leaderboard-copy"><strong>Cloud Sync Is Off</strong><p>Enable it to send new usage.</p></div>
      <button className="button" onClick={onSettings}>Review Sync Settings</button>
    </section>}
  </div>;
}
