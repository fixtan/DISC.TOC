// app.js — MAIN ウィンドウ。再生・音声・CDの読み込みの持ち主。
// EQ / PLAYLIST ウィンドウには、イベント（dt:state）で状態を配り、操作（dt:cmd）を受け取る。
import { AudioEngine } from './audio.js';
import { Spectrum } from './spectrum.js';
import { invoke, emit, listen, $, mmss, sleep, store, SKINS, applySkin } from './common.js';
import { makeDraggable } from './winmgr.js';

const engine = new AudioEngine();
const spectrum = new Spectrum($('dt-spectrum'), () => engine.analyser);

let toc = null, cands = [], candIdx = 0, meta = null, drives = [], curDrive = '', statusText = '';
let skin = store.get('dt_skin', 'lain');
const eqGains = new Array(10).fill(0);
const vis = { eq: true, playlist: true };

// ───────── 状態を配る ─────────
const snapshot = () => ({
  toc, meta, cands, candIdx, drives, drive: curDrive, skin, eq: eqGains, status: statusText,
  idx: P.idx, pstate: P.state === 'stop' ? 'stop' : paused() ? 'paused' : P.state,
});
let pushTimer = 0;
function pushState() {
  if (pushTimer) return;
  pushTimer = setTimeout(() => { pushTimer = 0; emit('dt:state', snapshot()); }, 0);
}
const status = (m) => { statusText = m || ''; pushState(); };

// ───────── メタデータ ─────────
function metaFromCand(c) {
  return {
    album: c?.title || '', artist: c?.artist || '', date: c?.date || '',
    tracks: toc.tracks.map((_, i) => ({ title: c?.tracks[i]?.title || '', artist: c?.tracks[i]?.artist || '' })),
  };
}
const fullMeta = () => ({ mb_id: toc.mb_id, cddb_id: toc.cddb_id, starts: toc.tracks.map((t) => t.start), ...meta });
const persist = () => invoke('save_meta', { meta: fullMeta() });
function trackLabel(i) {
  const t = meta?.tracks[i]; const title = t?.title || `Track ${toc.tracks[i].no}`;
  const a = t?.artist || meta?.artist;
  return a ? `${a} - ${title}` : title;
}
function setTitle() {
  const el = $('dt-title');
  el.textContent = (toc && P.idx !== null) ? `${P.idx + 1}. ${trackLabel(P.idx)}` : (toc ? meta?.album || 'CDが読み込まれました' : 'CDを入れてください');
  el.classList.remove('scroll');
  requestAnimationFrame(() => el.classList.toggle('scroll', el.scrollWidth > el.parentElement.clientWidth));
}

// ───────── ストリーミングプレイヤー ─────────
const P = { gen: 0, idx: null, startSec: 0, firstAt: null, schedAt: 0, srcs: [], state: 'stop', done: false };
const AHEAD = 6; // バッファしておく秒数
let history = [];
const repeatModes = ['off', 'list', 'one'];
let repeat = store.get('dt_repeat', 'off'), shuffle = store.get('dt_shuffle', 'off') === 'on';
if (!repeatModes.includes(repeat)) repeat = 'off';

function stopSources() { for (const s of P.srcs) { try { s.onended = null; s.stop(); } catch {} } P.srcs = []; }

function schedule(ab) {
  const ctx = engine.ctx;
  const i16 = new Int16Array(ab), frames = i16.length >> 1;
  const b = ctx.createBuffer(2, frames, 44100), L = b.getChannelData(0), R = b.getChannelData(1);
  for (let k = 0; k < frames; k++) { L[k] = i16[2 * k] / 32768; R[k] = i16[2 * k + 1] / 32768; }
  const s = ctx.createBufferSource(); s.buffer = b; s.connect(engine.input);
  const now = ctx.currentTime;
  if (P.firstAt === null) { P.schedAt = now + 0.1; P.firstAt = P.schedAt; P.state = 'playing'; status(''); pushState(); }
  else if (P.schedAt < now + 0.02) { const d = now + 0.05 - P.schedAt; P.firstAt += d; P.schedAt = now + 0.05; } // 途切れた分は時計から除く
  s.start(P.schedAt); P.schedAt += b.duration; P.srcs.push(s);
  s.onended = () => { P.srcs = P.srcs.filter((x) => x !== s); };
}

async function startTrack(i, startSec = 0) {
  if (!toc || i == null || !toc.tracks[i]) return;
  const gen = ++P.gen; stopSources();
  try { await engine.ctx.resume(); } catch {}
  Object.assign(P, { idx: i, startSec, firstAt: null, schedAt: 0, state: 'buffering', done: false });
  setTitle(); pushState(); status(`Track ${toc.tracks[i].no} を読み込み中…`); updateBar(); spectrum.start();
  const tr = toc.tracks[i], drive = curDrive, end = tr.start + tr.sectors;
  let lba = tr.start + Math.floor(startSec * 75), first = true;
  try {
    while (gen === P.gen && lba < end) {
      if (P.firstAt !== null && P.schedAt - engine.ctx.currentTime > AHEAD) { await sleep(200); continue; }
      const n = Math.min(first ? 38 : 75, end - lba);
      const ab = await invoke('read_pcm', { drive, lba, count: n });
      if (gen !== P.gen) return;
      lba += n; schedule(ab); first = false;
    }
    if (gen === P.gen) P.done = true;
  } catch (e) {
    if (gen === P.gen) { stopSources(); P.state = 'stop'; spectrum.stop(); status('エラー: ' + e); updateBar(); pushState(); }
  }
}

function position() { return P.firstAt === null ? P.startSec : P.startSec + Math.max(0, engine.ctx.currentTime - P.firstAt); }
const paused = () => P.state !== 'stop' && engine.ctx.state !== 'running';
let dragging = false;
function updateBar() {
  const dur = (P.idx !== null && toc) ? toc.tracks[P.idx].sectors / 75 : 0;
  const pos = Math.min(position(), dur);
  $('dt-seek').max = Math.floor(dur);
  if (!dragging) $('dt-seek').value = Math.floor(pos);
  $('dt-time').textContent = P.state === 'stop' && !paused() && P.idx === null ? '00:00' : mmss(pos);
  $('dt-play').classList.toggle('active', P.state !== 'stop' && !paused());
  $('dt-pause').classList.toggle('active', paused());
}

function stopPlayback(keepIdx = false) {
  P.gen++; stopSources(); P.state = 'stop'; P.firstAt = null; P.startSec = 0; P.done = false;
  if (!keepIdx) P.idx = null;
  engine.ctx.resume?.(); spectrum.stop(); updateBar(); setTitle(); pushState();
}

function pickNext(dir) {
  const n = toc.tracks.length, cur = P.idx ?? -1;
  if (dir < 0) {
    if (shuffle && history.length) return history.pop();
    return cur <= 0 ? n - 1 : cur - 1;
  }
  if (cur >= 0) { history.push(cur); if (history.length > 50) history.shift(); }
  if (shuffle && n > 1) { let k; do { k = Math.floor(Math.random() * n); } while (k === cur); return k; }
  return cur + 1 >= n ? 0 : cur + 1;
}

setInterval(() => {
  updateBar();
  if (toc && P.state === 'playing' && P.done && engine.ctx.state === 'running' && engine.ctx.currentTime >= P.schedAt - 0.02) {
    const n = toc.tracks.length, cur = P.idx;
    if (repeat === 'one') return startTrack(cur, 0);
    if (shuffle || repeat === 'list' || cur + 1 < n) return startTrack(pickNext(1), 0);
    stopPlayback(true);   // 最後の曲(リピートなし)はここで止まる
  }
}, 250);

// ───────── 操作ボタン ─────────
$('dt-play').onclick = async () => {
  if (!toc) return;
  if (paused()) { await engine.ctx.resume(); spectrum.start(); updateBar(); pushState(); return; }
  startTrack(P.idx ?? 0, 0);
};
$('dt-pause').onclick = async () => {
  if (!toc || P.state === 'stop') return;
  if (engine.ctx.state === 'running') { await engine.ctx.suspend(); spectrum.stop(); } else { await engine.ctx.resume(); spectrum.start(); }
  updateBar(); pushState();
};
$('dt-stop').onclick = () => stopPlayback(true);
$('dt-next').onclick = () => { if (toc) startTrack(pickNext(1), 0); };
$('dt-prev').onclick = () => {
  if (!toc) return;
  if (P.idx !== null && position() > 3) return startTrack(P.idx, 0);   // 3秒超えたら頭出し
  startTrack(pickNext(-1), 0);
};
$('dt-seek').oninput = () => { dragging = true; $('dt-time').textContent = mmss(+$('dt-seek').value); };
$('dt-seek').onchange = () => { dragging = false; if (P.idx !== null) startTrack(P.idx, +$('dt-seek').value); };

const vol = +store.get('dt_vol', '1'); $('dt-vol').value = vol; engine.setVolume(vol);
$('dt-vol').oninput = () => { engine.setVolume(+$('dt-vol').value); store.set('dt_vol', $('dt-vol').value); };

const renderModes = () => {
  $('dt-repeat').textContent = repeat === 'one' ? 'REP 1' : 'REP';
  $('dt-repeat').title = { off: 'リピートなし', list: '全曲リピート', one: '1曲リピート' }[repeat];
  $('dt-repeat').classList.toggle('active', repeat !== 'off');
  $('dt-shuffle').classList.toggle('active', shuffle);
};
$('dt-repeat').onclick = () => { repeat = { off: 'list', list: 'one', one: 'off' }[repeat]; store.set('dt_repeat', repeat); renderModes(); };
$('dt-shuffle').onclick = () => { shuffle = !shuffle; store.set('dt_shuffle', shuffle ? 'on' : 'off'); renderModes(); };
renderModes();

// ───────── 窓の出し入れ・スキン・終了 ─────────
const renderVis = () => {
  $('dt-eq-toggle').classList.toggle('active', vis.eq);
  $('dt-pl-toggle').classList.toggle('active', vis.playlist);
};
$('dt-eq-toggle').onclick = () => invoke('win_set_visible', { label: 'eq', visible: !vis.eq });
$('dt-pl-toggle').onclick = () => invoke('win_set_visible', { label: 'playlist', visible: !vis.playlist });
$('dt-min').onclick = () => invoke('win_minimize');
$('dt-quit').onclick = () => invoke('win_quit');

function changeSkin() {
  skin = applySkin(skin); store.set('dt_skin', skin); spectrum.readColors(); pushState();
}
$('dt-skin').onclick = () => { skin = SKINS[(SKINS.findIndex((s) => s.id === skin) + 1) % SKINS.length].id; changeSkin(); };

// ───────── イコライザー（音は MAIN が持つ。窓は EQ ウィンドウ） ─────────
function applyEq(gains) {
  if (!Array.isArray(gains) || gains.length !== 10 || !gains.every(Number.isFinite)) return;
  gains.forEach((g, i) => { eqGains[i] = g; engine.setGain(i, g); });
  store.set('dt_eq', JSON.stringify(eqGains));
}
try { applyEq(JSON.parse(store.get('dt_eq', 'null'))); } catch {}

// ───────── CD読み込み ─────────
async function load(drive) {
  stopPlayback(); history = []; toc = null; cands = []; candIdx = 0; meta = null;
  if (drive) curDrive = drive;
  setTitle(); pushState();
  if (!curDrive) return status('光学ドライブが見つかりません');
  try {
    status('TOC読み込み中…'); toc = await invoke('read_toc', { drive: curDrive }); meta = metaFromCand(null); setTitle(); pushState();
    status('検索中…');
    const r = await invoke('lookup', { drive: curDrive }); cands = r.candidates; candIdx = 0;
    meta = metaFromCand(cands[0]);
    status(r.source === 'local' ? '保存済みのデータを表示' : cands.length ? '' : '該当なし');
    setTitle(); pushState();
  } catch (e) { status('エラー: ' + e); }
}

// ───────── 他のウィンドウからの操作 ─────────
const onVis = (v) => { if (v.label in vis) { vis[v.label] = !!v.visible; renderVis(); } };
const onCmd = (c) => {
  switch (c.type) {
    case 'load': load(c.drive); break;
    case 'play': startTrack(c.idx, 0); break;
    case 'stop': stopPlayback(true); break;
    case 'cand':
      if (toc && cands[c.idx]) { candIdx = c.idx; meta = metaFromCand(cands[candIdx]); setTitle(); pushState(); }
      break;
    case 'setMeta':
      if (!toc || !c.meta) break;
      meta = c.meta; setTitle(); pushState();
      persist().then(() => status(c.msg || '保存しました')).catch((e) => status('保存エラー: ' + e));
      break;
    case 'eq': applyEq(c.gains); pushState(); break;
  }
};

// ───────── 起動 ─────────
(async () => {
  makeDraggable($('dt-drag'), 'main');
  await Promise.all([listen('dt:hello', () => pushState()), listen('dt:vis', onVis), listen('dt:cmd', onCmd)]);
  skin = applySkin(skin); spectrum.readColors();
  setTitle(); renderVis(); updateBar();
  try { for (const r of await invoke('win_rects')) if (r.label in vis) vis[r.label] = r.visible; renderVis(); } catch {}
  await invoke('win_ready', { label: 'main' }).catch(() => {});
  setTimeout(() => invoke('layout_settle').catch(() => {}), 600);   // 窓が出そろってから、見えている枠を測って位置をそろえる
  try { drives = await invoke('list_drives'); } catch { drives = []; }
  curDrive = drives[0] || '';
  pushState();
  load();
})();
