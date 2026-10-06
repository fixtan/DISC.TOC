// audio.js — Web Audio グラフ: input(音量) → EQ(10バンド) → アナライザー → 出力
export class AudioEngine {
  constructor() {
    this.ctx = new (window.AudioContext || window.webkitAudioContext)({ sampleRate: 44100 });
    this.freqs = [60, 170, 310, 600, 1000, 3000, 6000, 12000, 14000, 16000];
    this.input = this.ctx.createGain(); // 再生ソースはここへ。ゲイン＝音量
    let prev = this.input;
    this.eqNodes = this.freqs.map((f) => {
      const n = this.ctx.createBiquadFilter();
      n.type = 'peaking'; n.frequency.value = f; n.Q.value = 1.4; n.gain.value = 0;
      prev.connect(n); prev = n; return n;
    });
    this.analyser = this.ctx.createAnalyser();
    this.analyser.fftSize = 512;
    this.analyser.smoothingTimeConstant = 0.7;
    prev.connect(this.analyser);
    prev.connect(this.ctx.destination);
  }
  setGain(i, v) { this.eqNodes[i].gain.value = v; }
  setVolume(v) { this.input.gain.value = v; }
}
