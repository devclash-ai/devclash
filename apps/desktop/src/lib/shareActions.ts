import { invoke } from '@tauri-apps/api/core';
import { Image } from '@tauri-apps/api/image';
import { writeImage, writeText } from '@tauri-apps/plugin-clipboard-manager';
import { toBlob } from 'html-to-image';
import { native } from './desktop';
import { xDraftURL } from './share';
import { cloudPeriod } from './rank';
import type { Period, UsageSnapshot } from './types';
export async function copyPost(text: string) {
  if (native) await writeText(text); else await navigator.clipboard.writeText(text);
}
export async function cardBlob(element: HTMLElement): Promise<Blob> {
  const blob = await toBlob(element, { pixelRatio: 3, backgroundColor: getComputedStyle(element).backgroundColor });
  if (!blob) throw new Error('The share card could not be created. Try again.');
  return blob;
}
export async function copyCard(blob: Blob) {
  if (native) {
    const image = await Image.fromBytes(new Uint8Array(await blob.arrayBuffer()));
    try { await writeImage(image); } finally { await image.close(); }
  } else await navigator.clipboard.write([new ClipboardItem({ 'image/png': blob })]);
}
export async function saveCard(blob: Blob): Promise<string> {
  if (native) return invoke<string>('save_share_image', { bytes: Array.from(new Uint8Array(await blob.arrayBuffer())) });
  const url = URL.createObjectURL(blob); const link = document.createElement('a'); link.href = url; link.download = 'DevClash-Share.png'; link.click(); setTimeout(() => URL.revokeObjectURL(url), 1000);
  return 'DevClash-Share.png';
}
export async function openXDraft(text: string) {
  if (native) await invoke('open_share_draft', { text });
  else { const tab = window.open(xDraftURL(text), '_blank'); if (!tab) throw new Error('Your browser blocked the draft. Allow pop-ups and try again.'); tab.opener = null; }
}

export async function readShareRank(period: Period): Promise<UsageSnapshot['rank']> {
  const result = await invoke<{ position: number | null; tokens: number | null; updatedAt: string | null }>('cloud_share_rank', { period: cloudPeriod(period) });
  if (result.position === null) return undefined;
  if (!Number.isSafeInteger(result.position) || result.position! < 1 || !Number.isSafeInteger(result.tokens) || result.tokens! < 0 || !result.updatedAt || !Number.isFinite(Date.parse(result.updatedAt))) throw new Error('The leaderboard returned an invalid rank. Try again.');
  return { position: result.position!, tokens: result.tokens!, updatedAt: result.updatedAt, period, podiums: 0 };
}
