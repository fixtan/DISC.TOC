// winmgr.js — 枠なしウィンドウのドラッグ、吸着（スナップ）、グループ移動
// 座標はすべて物理ピクセル。ウィンドウの実際の移動は Rust 側（win_move）が行う。
import { invoke } from './common.js';

const SNAP_CSS = 14;   // この距離（CSS px）以内で吸着
const DOCK_TOL = 2;    // この距離（物理px）以内で接していれば「くっついている」とみなす

const overlapX = (a, b, tol = 0) => a.x < b.x + b.w + tol && a.x + a.w > b.x - tol;
const overlapY = (a, b, tol = 0) => a.y < b.y + b.h + tol && a.y + a.h > b.y - tol;

export function isDocked(a, b) {
  const touchX = Math.abs(a.x + a.w - b.x) <= DOCK_TOL || Math.abs(b.x + b.w - a.x) <= DOCK_TOL;
  const touchY = Math.abs(a.y + a.h - b.y) <= DOCK_TOL || Math.abs(b.y + b.h - a.y) <= DOCK_TOL;
  return (touchX && overlapY(a, b)) || (touchY && overlapX(a, b));
}

/** 動かす窓のグループ。
 *  メイン：くっついている窓を全部連れていく。
 *  それ以外：自分と、「自分を通らないとメインにつながっていない」窓だけ。（メインに直接つながっている窓は置いていく） */
export function groupOf(label, rects) {
  const by = Object.fromEntries(rects.map((r) => [r.label, r]));
  const flood = (start, blocked) => {
    const seen = new Set([start]), q = [start];
    while (q.length) {
      const cur = by[q.shift()];
      for (const r of rects) {
        if (seen.has(r.label) || blocked.has(r.label)) continue;
        if (isDocked(cur, r)) { seen.add(r.label); q.push(r.label); }
      }
    }
    return seen;
  };
  let seen;
  if (label === 'main' || !by.main) seen = flood(label, new Set());
  else {
    const anchored = flood('main', new Set([label]));   // 自分を通らずにメインへつながる窓
    anchored.delete(label);
    seen = flood(label, anchored);
  }
  return rects.filter((r) => seen.has(r.label));
}

export function bbox(rs) {
  const x = Math.min(...rs.map((r) => r.x)), y = Math.min(...rs.map((r) => r.y));
  return { x, y, w: Math.max(...rs.map((r) => r.x + r.w)) - x, h: Math.max(...rs.map((r) => r.y + r.h)) - y };
}

/** box を others に吸着させるための補正量 {dx, dy} */
export function snapDelta(box, others, th) {
  let bx = null, by = null;
  const pick = (cur, d) => (Math.abs(d) <= th && (cur === null || Math.abs(d) < Math.abs(cur)) ? d : cur);
  for (const o of others) {
    const xo = overlapX(box, o, th), yo = overlapY(box, o, th);
    const stackedV = Math.abs(box.y + box.h - o.y) <= th || Math.abs(box.y - (o.y + o.h)) <= th;
    const sideH = Math.abs(box.x + box.w - o.x) <= th || Math.abs(box.x - (o.x + o.w)) <= th;
    if (yo) { bx = pick(bx, o.x - (box.x + box.w)); bx = pick(bx, o.x + o.w - box.x); }          // 左右でぴったり接する
    if (xo) { by = pick(by, o.y - (box.y + box.h)); by = pick(by, o.y + o.h - box.y); }          // 上下でぴったり接する
    if (xo && stackedV) { bx = pick(bx, o.x - box.x); bx = pick(bx, o.x + o.w - (box.x + box.w)); } // 縦に並ぶとき、左右の端をそろえる
    if (yo && sideH) { by = pick(by, o.y - box.y); by = pick(by, o.y + o.h - (box.y + box.h)); }   // 横に並ぶとき、上下の端をそろえる
  }
  return { dx: bx ?? 0, dy: by ?? 0 };
}

export const saveLayout = (() => {
  let t = 0;
  return () => { clearTimeout(t); t = setTimeout(() => invoke('layout_save').catch(() => {}), 300); };
})();

/** handle（タイトルバー）をドラッグしてウィンドウを動かせるようにする */
export function makeDraggable(handle, label) {
  let st = null, busy = false, pending = null;
  handle.style.touchAction = 'none';

  const send = (moves) => {
    if (busy) { pending = moves; return; }
    busy = true;
    invoke('win_move', { moves }).catch(() => {}).finally(() => {
      busy = false;
      if (pending) { const p = pending; pending = null; send(p); }
    });
  };

  handle.addEventListener('pointerdown', async (e) => {
    if (e.button !== 0 || e.target.closest('button, input, select')) return;
    invoke('win_raise_all', { label }).catch(() => {});   // 自分のグループを最前面にする
    const my = { sx: e.screenX, sy: e.screenY, ready: false };
    st = my;
    try { handle.setPointerCapture(e.pointerId); } catch {}
    let rects;
    try { rects = (await invoke('win_rects')).filter((r) => r.visible); } catch { if (st === my) st = null; return; }
    if (st !== my) return;                               // 取得中にもう離された
    const group = groupOf(label, rects);
    if (!group.some((r) => r.label === label)) { st = null; return; }
    my.group = group; my.box = bbox(group);
    my.others = rects.filter((r) => !group.some((g) => g.label === r.label));
    my.ready = true;
  });

  handle.addEventListener('pointermove', (e) => {
    if (!st || !st.ready) return;
    const dpr = window.devicePixelRatio || 1;
    let dx = Math.round((e.screenX - st.sx) * dpr), dy = Math.round((e.screenY - st.sy) * dpr);
    if (!e.altKey) {                                      // Alt を押している間は吸着しない
      const moved = { ...st.box, x: st.box.x + dx, y: st.box.y + dy };
      const s = snapDelta(moved, st.others, Math.round(SNAP_CSS * dpr));
      dx += s.dx; dy += s.dy;
    }
    send(st.group.map((r) => ({ label: r.label, x: r.x + dx, y: r.y + dy })));
  });

  const end = () => { if (st) { const was = st.ready; st = null; if (was) saveLayout(); } };
  handle.addEventListener('pointerup', end);
  handle.addEventListener('pointercancel', end);
}
