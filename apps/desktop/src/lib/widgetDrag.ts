export interface Point { x: number; y: number }
export interface WindowGeometry { position: Point; scale: number }

// Native geometry is asynchronous. Keep the latest pointer coordinates even if
// the whole gesture finishes before the window replies, then apply its final move.
export function pointerWindowDrag(start: Point, geometry: Promise<WindowGeometry>,
  moveWindow: (position: Point) => Promise<void>, onError: () => void) {
  let latest = start; let revision = 0; let applied = 0; let moving = false;
  let ready: WindowGeometry | undefined; let failed = false;
  const apply = async () => {
    if (!ready || moving || failed) return;
    moving = true;
    try {
      while (revision !== applied) {
        const version = revision;
        await moveWindow({ x: Math.round(ready.position.x + (latest.x - start.x) * ready.scale),
          y: Math.round(ready.position.y + (latest.y - start.y) * ready.scale) });
        applied = version;
      }
    } catch { failed = true; onError(); }
    finally { moving = false; }
  };
  void geometry.then(value => { ready = value; void apply(); }).catch(() => { failed = true; onError(); });
  return { move(point: Point) { latest = point; revision++; void apply(); } };
}
