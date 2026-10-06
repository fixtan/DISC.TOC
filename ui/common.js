// common.js — 3つのウィンドウで共有する小さな道具
const T = window.__TAURI__;
export const invoke = (...a) => T.core.invoke(...a);
export const emit = (name, payload) => T.event.emit(name, payload);
export const listen = (name, fn) => T.event.listen(name, (e) => fn(e.payload));
export const $ = (id) => document.getElementById(id);
export const mmss = (s) => `${String(Math.floor(s / 60)).padStart(2, '0')}:${String(Math.floor(s % 60)).padStart(2, '0')}`;
export const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
export const store = {
  get(k, d) { try { return localStorage.getItem(k) ?? d; } catch { return d; } },
  set(k, v) { try { localStorage.setItem(k, v); } catch {} },
};
export const SKINS = [{ id: 'lain', label: 'LAIN' }, { id: 'aqua', label: 'AQUA' }];
export const EQ_PRESETS = {
  FLAT: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0], ROCK: [5, 4, 3, 1, -1, -1, 1, 3, 4, 5], POP: [-1, 2, 4, 5, 3, 0, -1, -1, -1, -1],
  DANCE: [6, 4, 2, 0, 0, -3, -4, -4, 0, 0], VOCAL: [-2, -3, -3, 1, 4, 4, 3, 1, 0, -1], CLASSICAL: [0, 0, 0, 0, 0, 0, -3, -3, -3, -4],
  'BASS BOOST': [6, 5, 4, 2, 0, 0, 0, 0, 0, 0], 'TREBLE BOOST': [0, 0, 0, 0, 0, 1, 3, 5, 6, 7],
};
export function applySkin(id) {
  if (!SKINS.some((s) => s.id === id)) id = SKINS[0].id;
  $('app').dataset.skin = id;
  return id;
}
