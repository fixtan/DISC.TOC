// spectrum.js
// メインウィンドウのスペクトラムアナライザー（クラシックプレイヤー風の棒グラフ＋ピークキャップ）

const BARS = 19;

export class Spectrum {
  // getAnalyser: () => AnalyserNode | null （AudioEngine.init() 前は null なので遅延取得）
  constructor(canvas, getAnalyser) {
    this.canvas = canvas;
    this.ctx = canvas.getContext('2d');
    this.getAnalyser = getAnalyser;
    this.raf = 0;
    this.data = null;
    this.colors = null;
    this.peaks = new Array(BARS).fill(0);
    this.draw = this.draw.bind(this);
  }

  // スキンの CSS 変数から色を読む（スキン切替時にも呼ぶ）
  readColors() {
    const cs = getComputedStyle(this.canvas);
    const v = (name, fb) => cs.getPropertyValue(name).trim() || fb;
    this.colors = {
      low: v('--dt-spec-low', '#00ff66'),
      mid: v('--dt-spec-mid', '#e6ff00'),
      high: v('--dt-spec-high', '#ff3030'),
      peak: v('--dt-spec-peak', '#cceeff'),
    };
  }

  start() {
    this.readColors();
    if (this.raf) return;
    this.raf = requestAnimationFrame(this.draw);
  }

  // 停止時はバーを落として描画ループを止める
  stop() {
    if (this.raf) cancelAnimationFrame(this.raf);
    this.raf = 0;
    this.peaks.fill(0);
    this.clear();
  }

  clear() {
    this.ctx.clearRect(0, 0, this.canvas.width, this.canvas.height);
  }

  draw() {
    this.raf = requestAnimationFrame(this.draw);
    const analyser = this.getAnalyser();
    const { width: W, height: H } = this.canvas;
    const c = this.ctx;
    c.clearRect(0, 0, W, H);
    if (!analyser) return;

    if (!this.data || this.data.length !== analyser.frequencyBinCount) {
      this.data = new Uint8Array(analyser.frequencyBinCount);
    }
    analyser.getByteFrequencyData(this.data);

    // 低域〜約16kHz を対数的に BARS 本へ割り当てる
    const usable = Math.floor(this.data.length * 0.75);
    const barW = W / BARS;
    for (let i = 0; i < BARS; i++) {
      const from = Math.floor(Math.pow(usable, i / BARS));
      const to = Math.max(from + 1, Math.floor(Math.pow(usable, (i + 1) / BARS)));
      let sum = 0;
      for (let k = from; k < to; k++) sum += this.data[k];
      const v = sum / (to - from) / 255; // 0..1
      const h = Math.round(v * H);

      // 色はスキン（CSS 変数）から
      const g = c.createLinearGradient(0, H, 0, 0);
      g.addColorStop(0, this.colors.low);
      g.addColorStop(0.6, this.colors.mid);
      g.addColorStop(1, this.colors.high);
      c.fillStyle = g;
      c.fillRect(i * barW + 0.5, H - h, barW - 1, h);

      // ピークキャップ（ゆっくり落ちる）
      this.peaks[i] = Math.max(this.peaks[i] - 0.4, h);
      c.fillStyle = this.colors.peak;
      c.fillRect(i * barW + 0.5, H - Math.round(this.peaks[i]) - 1, barW - 1, 1);
    }
  }
}
