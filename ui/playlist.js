// playlist.js — PLAYLIST ウィンドウ。曲リスト・編集・書き出し・WAV。
// 状態は MAIN から届く（dt:state）。操作は dt:cmd で MAIN に頼む。
import { invoke, emit, listen, $, mmss, applySkin } from './common.js';
import { makeDraggable, saveLayout } from './winmgr.js';

let S = null;                // MAIN から届いた最新の状態
let editing = false, draft = null, lastKey = '', lastMainStatus = '', ripping = false;
const status = (m) => { $('dt-status').textContent = m || ''; };
const cmd = (c) => emit('dt:cmd', c);
const clone = (o) => JSON.parse(JSON.stringify(o));
const fullMeta = (m) => ({ mb_id: S.toc.mb_id, cddb_id: S.toc.cddb_id, starts: S.toc.tracks.map((t) => t.start), ...m });

// ───────── 表示 ─────────
function renderList() {
  const toc = S.toc, meta = editing ? draft : S.meta;
  const m = $('dt-meta'); m.innerHTML = '';
  if (toc && meta) {
    if (editing) {
      for (const [k, ph] of [['artist', 'アーティスト'], ['album', 'アルバム'], ['date', '年']]) {
        const i = document.createElement('input'); i.placeholder = ph; i.value = meta[k];
        i.oninput = () => { draft[k] = i.value; }; if (k === 'date') i.classList.add('dt-year'); m.appendChild(i);
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
    li.className = 'dt-playlist-item';
    li.innerHTML = '<span class="no"></span><span class="t"></span><span class="d"></span>';
    li.children[0].textContent = `${t.no}.`;
    li.children[2].textContent = mmss(t.secs);
    const cell = li.children[1];
    if (editing) {
      const inp = document.createElement('input'); inp.value = draft.tracks[i].title; inp.placeholder = `Track ${t.no}`;
      inp.oninput = () => { draft.tracks[i].title = inp.value; };
      inp.onclick = (e) => e.stopPropagation(); cell.appendChild(inp);
    } else {
      const a = meta?.tracks[i]?.artist;
      cell.textContent = (meta?.tracks[i]?.title || `Track ${t.no}`) + (a && a !== meta.artist ? `  — ${a}` : '');
    }
    li.onclick = () => cmd({ type: 'play', idx: i });
    list.appendChild(li);
  });
  $('dt-count').textContent = toc ? `${toc.tracks.length} TRACKS` : '';
  $('dt-edit').classList.toggle('active', editing);
  $('dt-edit').textContent = editing ? '編集終了' : '編集';
  $('dt-save').hidden = !editing;
  markActive();
}

/** 再生中の曲の色付けだけ更新する（編集中の入力欄を壊さないため、作り直さない） */
function markActive() {
  [...$('dt-list').children].forEach((li, i) => li.classList.toggle('active', S && i === S.idx));
}

function renderTop() {
  const sel = $('dt-drive');
  if (sel.options.length !== S.drives.length || S.drives.some((d, i) => sel.options[i]?.value !== d)) {
    sel.innerHTML = S.drives.map((d) => `<option>${d}</option>`).join('');
  }
  if (S.drive) sel.value = S.drive;
  const cs = $('dt-cand');
  cs.hidden = S.cands.length <= 1;
  if (S.cands.length > 1) {
    cs.innerHTML = S.cands.map((c, i) => `<option value="${i}"></option>`).join('');
    [...cs.options].forEach((o, i) => { o.textContent = `${S.cands[i].artist} — ${S.cands[i].title} ${S.cands[i].date || ''}`; });
    cs.value = String(S.candIdx);
  }
}

function onState(s) {
  const prevId = S?.toc?.mb_id;
  S = s;
  applySkin(s.skin);
  renderTop();
  if (s.toc?.mb_id !== prevId) editing = false;                       // 別のCDになったら編集は終わり
  const key = `${s.toc?.mb_id}|${JSON.stringify(s.meta)}`;
  if (key !== lastKey) {                                              // 曲名などが変わったときだけ作り直す
    lastKey = key; if (editing) draft = clone(s.meta);
    renderList();
  } else markActive();
  if (s.status !== lastMainStatus) { lastMainStatus = s.status; status(s.status); }
}

// ───────── ボタン ─────────
$('dt-reload').onclick = () => cmd({ type: 'load', drive: $('dt-drive').value });
$('dt-drive').onchange = () => cmd({ type: 'load', drive: $('dt-drive').value });
$('dt-cand').onchange = () => cmd({ type: 'cand', idx: +$('dt-cand').value });
$('dt-edit').onclick = () => {
  if (!S?.toc || !S.meta) return;
  editing = !editing; draft = editing ? clone(S.meta) : null; renderList();
};
$('dt-save').onclick = () => { if (editing && draft) cmd({ type: 'setMeta', meta: clone(draft), msg: '保存しました' }); };

$('dt-export').onclick = async () => {
  if (!S?.toc) return;
  try {
    const m = editing ? draft : S.meta;
    const name = (m.album || S.toc.mb_id).replace(/[\\/:*?"<>|]/g, '_');
    const saved = await invoke('export_file', { meta: fullMeta(m), fmt: $('dt-fmt').value, name });
    status(saved ? '書き出しました: ' + saved : '');
  } catch (e) { status('書き出しエラー: ' + e); }
};
$('dt-import').onclick = () => $('dt-file').click();
$('dt-file').onchange = async () => {
  const f = $('dt-file').files[0]; $('dt-file').value = ''; if (!f || !S?.toc) return;
  try {
    const j = JSON.parse(await f.text());
    if (j.mb_id && j.mb_id !== S.toc.mb_id) return status('このCDのデータではありません（ディスクIDが違います）');
    if (!Array.isArray(j.tracks) || j.tracks.length !== S.toc.tracks.length) return status('トラック数が一致しません');
    const m = { album: j.album || '', artist: j.artist || '', date: j.date || '', tracks: j.tracks.map((t) => ({ title: t.title || '', artist: t.artist || '' })) };
    cmd({ type: 'setMeta', meta: m, msg: '読み込んで保存しました' });
  } catch (e) { status('読み込みエラー: ' + e); }
};
$('dt-mbreg').onclick = () => { if (S?.toc) window.__TAURI__.opener.openUrl('https://musicbrainz.org/cdtoc/attach?toc=' + S.toc.mb_toc); };

// ───────── WAV書き出し（CDリッピング） ─────────
const pad2 = (n) => String(n).padStart(2, '0');
function ripName(i) {
  const t = S.meta.tracks[i], no = pad2(S.toc.tracks[i].no), title = t.title || `Track ${no}`;
  return `${no} - ${t.artist && t.artist !== S.meta.artist ? t.artist + ' - ' : ''}${title}`;
}
async function rip(idxs) {
  if (!S?.toc || ripping) return;
  cmd({ type: 'stop' }); ripping = true;
  for (const id of ['dt-rip-one', 'dt-rip-all']) $(id).disabled = true;
  $('dt-rip-stop').hidden = false;
  const unlisten = await listen('rip-progress', (p) => {
    status(`書き出し中 ${p.index + 1}/${p.total}  ${Math.floor((p.done / p.sectors) * 100)}%  ${p.name}`);
  });
  try {
    const tracks = idxs.map((i) => ({ start: S.toc.tracks[i].start, sectors: S.toc.tracks[i].sectors, name: ripName(i) }));
    const r = await invoke('rip_wav', { drive: S.drive, tracks });
    if (!r.dir) status('');
    else if (r.cancelled) status(`中止しました（${r.written}曲 保存済み）`);
    else status(`${r.written}曲を書き出しました: ${r.dir}` + (r.bad_sectors ? `（読めなかったセクタ ${r.bad_sectors} 個は無音）` : ''));
  } catch (e) { status('書き出しエラー: ' + e); }
  finally {
    unlisten(); ripping = false;
    for (const id of ['dt-rip-one', 'dt-rip-all']) $(id).disabled = false;
    $('dt-rip-stop').hidden = true;
  }
}
$('dt-rip-one').onclick = () => { if (!S?.toc) return; if (S.idx === null) return status('曲を選んでから押してください'); rip([S.idx]); };
$('dt-rip-all').onclick = () => { if (S?.toc) rip(S.toc.tracks.map((_, i) => i)); };
$('dt-rip-stop').onclick = () => invoke('rip_cancel');

// ───────── 窓まわり ─────────
$('dt-close').onclick = () => invoke('win_set_visible', { label: 'playlist', visible: false });
$('dt-grip').addEventListener('pointerdown', (e) => {
  e.preventDefault();
  try { window.__TAURI__.window.getCurrentWindow().startResizeDragging('SouthEast'); } catch {}
});
let resizeTimer = 0;
window.addEventListener('resize', () => { clearTimeout(resizeTimer); resizeTimer = setTimeout(saveLayout, 500); });

(async () => {
  makeDraggable($('dt-drag'), 'playlist');
  await listen('dt:state', onState);          // 先に受け取る準備をしてから、MAIN に状態を頼む
  await emit('dt:hello', { from: 'playlist' });
  await invoke('win_ready', { label: 'playlist' }).catch(() => {});
})();
