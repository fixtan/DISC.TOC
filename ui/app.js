import { AudioEngine } from './audio.js';
import { Spectrum } from './spectrum.js';

const { invoke } = window.__TAURI__.core;
const $ = (id) => document.getElementById(id);
const store = {
  get(k, d) { try { return localStorage.getItem(k) ?? d; } catch { return d; } },
  set(k, v) { try { localStorage.setItem(k, v); } catch {} },
};
const mmss = (s) => `${String(Math.floor(s / 60)).padStart(2, '0')}:${String(Math.floor(s % 60)).padStart(2, '0')}`;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const status = (m) => { $('dt-status').textContent = m || ''; };

const SKINS = [{ id: 'lain', label: 'LAIN' }, { id: 'aqua', label: 'AQUA' }];
const engine = new AudioEngine();
const spectrum = new Spectrum($('dt-spectrum'), () => engine.analyser);

let toc = null, cands = [], meta = null, editing = false;

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

// ───────── 表示 ─────────
function render() {
  const m = $('dt-meta'); m.innerHTML = '';
  if (toc && meta) {
    if (editing) {
      for (const [k, ph] of [['artist', 'アーティスト'], ['album', 'アルバム'], ['date', '年']]) {
        const i = document.createElement('input'); i.placeholder = ph; i.value = meta[k];
        i.oninput = () => { meta[k] = i.value; }; if (k === 'date') i.style.flex = '0 0 70px'; m.appendChild(i);
      }
    } else {
      m.textContent = (meta.artist || meta.album)
        ? `${meta.artist} — ${meta.album}${meta.date ? ` (${meta.date})` : ''}`
        : 'CD情報なし（「編集」で入力できます）';
    }
  }
  const list = $('dt-list'); list.innerHTML = '';
  toc?.tracks.forEach((t, i) => {
    const li = document.createElement('li');
    li.className = 'dt-playlist-item' + (i === P.idx ? ' active' : '');
    li.innerHTML = '<span class="no"></span><span class="t"></span><span class="d"></span>';
    li.children[0].textContent = `${t.no}.`;
    li.children[2].textContent = mmss(t.secs);
    const cell = li.children[1];
    if (editing) {
      const inp = document.createElement('input'); inp.value = meta.tracks[i].title; inp.placeholder = `Track ${t.no}`;
      inp.oninput = () => { meta.tracks[i].title = inp.value; };
      inp.onclick = (e) => e.stopPropagation(); cell.appendChild(inp);
    } else {
      const a = meta?.tracks[i]?.artist;
      cell.textContent = (meta?.tracks[i]?.title || `Track ${t.no}`) + (a && a !== meta.artist ? `  — ${a}` : '');
    }
    li.onclick = () => startTrack(i, 0);
    list.appendChild(li);
  });
  $('dt-count').textContent = toc ? `${toc.tracks.length} TRACKS` : '';
  $('dt-edit').classList.toggle('active', editing);
  $('dt-edit').textContent = editing ? '編集終了' : '編集';
  $('dt-save').hidden = !editing;
  setTitle();
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
  if (P.firstAt === null) { P.schedAt = now + 0.1; P.firstAt = P.schedAt; P.state = 'playing'; status(''); }
  else if (P.schedAt < now + 0.02) { const d = now + 0.05 - P.schedAt; P.firstAt += d; P.schedAt = now + 0.05; } // 途切れた分は時計から除く
  s.start(P.schedAt); P.schedAt += b.duration; P.srcs.push(s);
  s.onended = () => { P.srcs = P.srcs.filter((x) => x !== s); };
}

async function startTrack(i, startSec = 0) {
  if (!toc) return;
  const gen = ++P.gen; stopSources();
  try { await engine.ctx.resume(); } catch {}
  Object.assign(P, { idx: i, startSec, firstAt: null, schedAt: 0, state: 'buffering', done: false });
  render(); status(`Track ${toc.tracks[i].no} を読み込み中…`); updateBar(); spectrum.start();
  const tr = toc.tracks[i], drive = $('dt-drive').value, end = tr.start + tr.sectors;
  let lba = tr.start + Math.floor(startSec * 75), first = true;
  try {
    while (gen === P.gen && lba < end) {
      if (P.firstAt !== null && P.schedAt - engine.ctx.currentTime > AHEAD) { await sleep(200); continue; }
      const n = Math.min(first ? 38 : 75, end - lba);
      const t0 = performance.now();
      const ab = await invoke('read_pcm', { drive, lba, count: n });
      if (gen !== P.gen) return;
      if (first) console.log('first chunk', ((performance.now() - t0) / 1000).toFixed(2) + 's');
      lba += n; schedule(ab); first = false;
    }
    if (gen === P.gen) P.done = true;
  } catch (e) {
    if (gen === P.gen) { stopSources(); P.state = 'stop'; spectrum.stop(); status('エラー: ' + e); updateBar(); }
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
  engine.ctx.resume?.(); spectrum.stop(); updateBar(); setTitle();
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
  if (paused()) { await engine.ctx.resume(); spectrum.start(); updateBar(); return; }
  startTrack(P.idx ?? 0, 0);
};
$('dt-pause').onclick = async () => {
  if (!toc || P.state === 'stop') return;
  if (engine.ctx.state === 'running') { await engine.ctx.suspend(); spectrum.stop(); } else { await engine.ctx.resume(); spectrum.start(); }
  updateBar();
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

// ───────── イコライザー ─────────
const EQ_PRESETS = {
  FLAT: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0], ROCK: [5, 4, 3, 1, -1, -1, 1, 3, 4, 5], POP: [-1, 2, 4, 5, 3, 0, -1, -1, -1, -1],
  DANCE: [6, 4, 2, 0, 0, -3, -4, -4, 0, 0], VOCAL: [-2, -3, -3, 1, 4, 4, 3, 1, 0, -1], CLASSICAL: [0, 0, 0, 0, 0, 0, -3, -3, -3, -4],
  'BASS BOOST': [6, 5, 4, 2, 0, 0, 0, 0, 0, 0], 'TREBLE BOOST': [0, 0, 0, 0, 0, 1, 3, 5, 6, 7],
};
const eqGains = new Array(10).fill(0), eqSliders = [];
const eqCanvas = $('dt-eq-canvas'), eqCtx = eqCanvas.getContext('2d'), presetSel = $('dt-eq-preset');
function drawEq() {
  eqCtx.clearRect(0, 0, eqCanvas.width, eqCanvas.height); eqCtx.beginPath();
  eqCtx.strokeStyle = getComputedStyle($('app') ?? document.body).getPropertyValue('--dt-eq-line').trim() || '#00e1ff'; eqCtx.lineWidth = 1.5;
  const step = eqCanvas.width / 9;
  eqGains.forEach((g, i) => { const x = i * step, y = 15 - (g / 20) * 12; i ? eqCtx.lineTo(x, y) : eqCtx.moveTo(x, y); });
  eqCtx.stroke();
}
function findPreset() { for (const [n, g] of Object.entries(EQ_PRESETS)) if (g.every((v, i) => v === eqGains[i])) return n; return ''; }
function setEq(gains) {
  gains.forEach((g, i) => { eqGains[i] = g; eqSliders[i].value = String(g); engine.setGain(i, g); });
  drawEq(); store.set('dt_eq', JSON.stringify(eqGains)); presetSel.value = findPreset();
}
presetSel.innerHTML = '<option value="">CUSTOM</option>' + Object.keys(EQ_PRESETS).map((n) => `<option>${n}</option>`).join('');
engine.freqs.forEach((_, i) => {
  const s = document.createElement('input'); s.type = 'range'; s.className = 'dt-eq-slider'; s.min = '-20'; s.max = '20'; s.value = '0';
  s.oninput = () => { eqGains[i] = +s.value; engine.setGain(i, +s.value); drawEq(); store.set('dt_eq', JSON.stringify(eqGains)); presetSel.value = findPreset(); };
  eqSliders.push(s); $('dt-eq-sliders').appendChild(s);
});
presetSel.onchange = () => { if (EQ_PRESETS[presetSel.value]) setEq(EQ_PRESETS[presetSel.value]); };
try { const sv = JSON.parse(store.get('dt_eq', 'null')); if (Array.isArray(sv) && sv.length === 10 && sv.every(Number.isFinite)) setEq(sv); } catch {}
const setEqOpen = (o) => { $('dt-eq').hidden = !o; $('dt-eq-toggle').classList.toggle('active', o); store.set('dt_eq_open', o ? '1' : '0'); drawEq(); };
$('dt-eq-toggle').onclick = () => setEqOpen($('dt-eq').hidden);
$('dt-eq-close').onclick = () => setEqOpen(false);
setEqOpen(store.get('dt_eq_open', '0') === '1');

// ───────── スキン ─────────
let skin = store.get('dt_skin', 'lain');
function applySkin() {
  if (!SKINS.some((s) => s.id === skin)) skin = SKINS[0].id;
  $('app').dataset.skin = skin; $('dt-skin').title = `スキン: ${SKINS.find((s) => s.id === skin).label}`;
  store.set('dt_skin', skin); drawEq(); spectrum.readColors();
}
$('dt-skin').onclick = () => { skin = SKINS[(SKINS.findIndex((s) => s.id === skin) + 1) % SKINS.length].id; applySkin(); };
applySkin();

// ───────── CD読み込み・編集・書き出し ─────────
async function load() {
  stopPlayback(); history = []; toc = null; cands = []; meta = null; editing = false; $('dt-cand').hidden = true; render();
  const drive = $('dt-drive').value; if (!drive) return status('光学ドライブが見つかりません');
  try {
    status('TOC読み込み中…'); toc = await invoke('read_toc', { drive }); meta = metaFromCand(null); render();
    status('検索中…');
    const r = await invoke('lookup', { drive }); cands = r.candidates;
    $('dt-cand').innerHTML = cands.map((c, i) => `<option value="${i}"></option>`).join('');
    [...$('dt-cand').options].forEach((o, i) => { o.textContent = `${cands[i].artist} — ${cands[i].title} ${cands[i].date || ''}`; });
    $('dt-cand').hidden = cands.length <= 1;
    meta = metaFromCand(cands[0]);
    status(r.source === 'local' ? '保存済みのデータを表示' : cands.length ? '' : '該当なし'); render();
  } catch (e) { status('エラー: ' + e); }
}
$('dt-reload').onclick = load;
$('dt-drive').onchange = load;
$('dt-cand').onchange = () => { meta = metaFromCand(cands[+$('dt-cand').value]); render(); };
$('dt-edit').onclick = () => { if (toc) { editing = !editing; render(); } };
$('dt-save').onclick = async () => { try { await persist(); status('保存しました'); } catch (e) { status('保存エラー: ' + e); } };
$('dt-export').onclick = async () => {
  if (!toc) return;
  try {
    const name = (meta.album || toc.mb_id).replace(/[\\/:*?"<>|]/g, '_');
    const saved = await invoke('export_file', { meta: fullMeta(), fmt: $('dt-fmt').value, name });
    status(saved ? '書き出しました: ' + saved : '');
  } catch (e) { status('書き出しエラー: ' + e); }
};
$('dt-import').onclick = () => $('dt-file').click();
$('dt-file').onchange = async () => {
  const f = $('dt-file').files[0]; $('dt-file').value = ''; if (!f || !toc) return;
  try {
    const j = JSON.parse(await f.text());
    if (j.mb_id && j.mb_id !== toc.mb_id) return status('このCDのデータではありません（ディスクIDが違います）');
    if (!Array.isArray(j.tracks) || j.tracks.length !== toc.tracks.length) return status('トラック数が一致しません');
    meta = { album: j.album || '', artist: j.artist || '', date: j.date || '', tracks: j.tracks.map((t) => ({ title: t.title || '', artist: t.artist || '' })) };
    await persist(); status('読み込んで保存しました'); render();
  } catch (e) { status('読み込みエラー: ' + e); }
};
$('dt-mbreg').onclick = () => { if (toc) window.__TAURI__.opener.openUrl('https://musicbrainz.org/cdtoc/attach?toc=' + toc.mb_toc); };

(async () => {
  const ds = await invoke('list_drives');
  $('dt-drive').innerHTML = ds.map((d) => `<option>${d}</option>`).join('');
  load();
})();
