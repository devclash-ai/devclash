import { invoke, isTauri } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { PhysicalPosition } from '@tauri-apps/api/dpi';
import { pointerWindowDrag } from './widgetDrag';
import { Image } from '@tauri-apps/api/image';
import { writeImage } from '@tauri-apps/plugin-clipboard-manager';
import { disable, enable, isEnabled } from '@tauri-apps/plugin-autostart';
import { toBlob } from 'html-to-image';
import type { Surface, UsageSnapshot, WidgetSize } from './types';
import { widgetWidths } from './preferences';
export const native = isTauri();
export async function readUsage(refresh = false): Promise<UsageSnapshot> {
  return invoke<UsageSnapshot>(refresh ? 'refresh_usage' : 'read_usage');
}
export async function closeWindow(): Promise<void> { if (native) await getCurrentWindow().hide(); }
export async function minimizeWindow(): Promise<void> { if (native) await getCurrentWindow().minimize(); }
export async function maximizeWindow(): Promise<void> { if (native) await getCurrentWindow().toggleMaximize(); }
export async function openWidget(size: WidgetSize): Promise<void> {
  if (native) await invoke('open_widget');
  else window.open('/?window=widget&demo=1', 'devclash-widget', `width=${widgetWidths[size]},height=600`);
}
export async function openCustomization(surface: Surface): Promise<void> { if (native) await invoke('open_customization', { surface }); }
export async function configureWidget(alwaysOnTop: boolean): Promise<void> {
  if (native) await invoke('configure_widget', { alwaysOnTop });
}
export async function setAutostart(value: boolean): Promise<void> { if (native) await (value ? enable() : disable()); }
export async function getAutostart(): Promise<boolean> { return native ? isEnabled() : false; }
export async function setTray(label: string, rows: string[], tooltip: string): Promise<void> { if (native) await invoke('set_tray_summary', { label, rows, tooltip }); }
export async function shareImage(element: HTMLElement): Promise<void> {
  const blob = await toBlob(element, { pixelRatio: 2, cacheBust: false, backgroundColor: element.classList.contains('floating-widget') ? 'transparent' : getComputedStyle(element).backgroundColor,
    filter: node => !(node instanceof HTMLElement && node.dataset.noShare === 'true') });
  if (!blob) throw new Error('The share image could not be created.');
  if (native) {
    const image = await Image.fromBytes(new Uint8Array(await blob.arrayBuffer()));
    try { await writeImage(image); } finally { await image.close(); }
  } else if (navigator.clipboard && typeof ClipboardItem !== 'undefined') {
    await navigator.clipboard.write([new ClipboardItem({ 'image/png': blob })]);
  } else {
    const url = URL.createObjectURL(blob); const link = document.createElement('a');
    link.href = url; link.download = 'DevClash.png'; link.click(); URL.revokeObjectURL(url);
  }
}

export function startWidgetDrag(x: number, y: number, onError: () => void) {
  const window = getCurrentWindow();
  return pointerWindowDrag({ x, y }, Promise.all([window.outerPosition(), window.scaleFactor()])
    .then(([position, scale]) => ({ position, scale })), position => window.setPosition(new PhysicalPosition(position.x, position.y)), onError);
}
