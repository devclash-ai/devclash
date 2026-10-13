import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { pointerWindowDrag } from './widgetDrag.ts';
import type { WindowGeometry, Point } from './widgetDrag.ts';

describe('Native widget dragging', () => {
  it('retains a completed gesture while waiting for native geometry and honors display scale', async () => {
    let resolve!: (value: WindowGeometry) => void;
    const geometry = new Promise<WindowGeometry>(done => { resolve = done; });
    const positions: Point[] = [];
    const drag = pointerWindowDrag({ x: 150, y: 200 }, geometry,
      async point => { positions.push(point); }, () => { throw new Error('Unexpected failure'); });
    drag.move({ x: 170, y: 180 });
    drag.move({ x: 190, y: 160 });
    assert.deepEqual(positions, []);
    resolve({ position: { x: -600, y: 50 }, scale: 2 });
    await new Promise(done => setTimeout(done, 0));
    assert.deepEqual(positions, [{ x: -520, y: -30 }]);
  });
  it('finishes with the newest pointer position when window updates overlap', async () => {
    let release!: () => void;
    const positions: Point[] = [];
    const drag = pointerWindowDrag({ x: 0, y: 0 }, Promise.resolve({ position: { x: 10, y: 20 }, scale: 1 }),
      async point => { positions.push(point); if (positions.length === 1) await new Promise<void>(done => { release = done; }); },
      () => { throw new Error('Unexpected failure'); });
    drag.move({ x: 5, y: 5 });
    await new Promise(done => setTimeout(done, 0));
    drag.move({ x: 8, y: 9 }); release();
    await new Promise(done => setTimeout(done, 0));
    assert.deepEqual(positions, [{ x: 15, y: 25 }, { x: 18, y: 29 }]);
  });
});
