// eq.js — EQUALIZER ウィンドウ。音は MAIN が持っていて、ここは操作と表示だけ。
import { invoke, emit, listen, $, applySkin, EQ_PRESETS } from './common.js';
import { makeDraggable } from './winmgr.js';

const gains = new Array(10).fill(0), sliders = [];
const canvas = $('dt-eq-canvas'), ctx = canvas.getContext('2d'), presetSel = $('dt-eq-preset');

function draw() {
  ctx.clearRect(0, 0, canvas.width, canvas.height); ctx.beginPath();
  ctx.strokeStyle = getComputedStyle($('app')).getPropertyValue('--dt-eq-line').trim() || '#00e1ff'; ctx.lineWidth = 1.5;
  const step = canvas.width / 9;
  gains.forEach((g, i) => { const x = i * step, y = 15 - (g / 20) * 12; i ? ctx.lineTo(x, y) : ctx.moveTo(x, y); });
  ctx.stroke();
}
const findPreset = () => { for (const [n, g] of Object.entries(EQ_PRESETS)) if (g.every((v, i) => v === gains[i])) return n; return ''; };
const send = () => emit('dt:cmd', { type: 'eq', gains: [...gains] });

presetSel.innerHTML = '<option value="">CUSTOM</option>' + Object.keys(EQ_PRESETS).map((n) => `<option>${n}</option>`).join('');
for (let i = 0; i < 10; i++) {
  const s = document.createElement('input'); s.type = 'range'; s.className = 'dt-eq-slider'; s.min = '-20'; s.max = '20'; s.value = '0';
  s.oninput = () => { gains[i] = +s.value; draw(); presetSel.value = findPreset(); send(); };
  sliders.push(s); $('dt-eq-sliders').appendChild(s);
}
presetSel.onchange = () => {
  const g = EQ_PRESETS[presetSel.value]; if (!g) return;
  g.forEach((v, i) => { gains[i] = v; sliders[i].value = String(v); }); draw(); send();
};

// MAIN から状態が来たら反映する（スキン・EQ の値）
const onState = (s) => {
  applySkin(s.skin);
  if (Array.isArray(s.eq) && !sliders.some((x) => x === document.activeElement)) {
    s.eq.forEach((v, i) => { gains[i] = v; sliders[i].value = String(v); });
    presetSel.value = findPreset();
  }
  draw();
};

$('dt-close').onclick = () => invoke('win_set_visible', { label: 'eq', visible: false });
(async () => {
  makeDraggable($('dt-drag'), 'eq');
  draw();
  await listen('dt:state', onState);          // 先に受け取る準備をしてから、MAIN に状態を頼む
  await emit('dt:hello', { from: 'eq' });
  await invoke('win_ready', { label: 'eq' }).catch(() => {});
})();
