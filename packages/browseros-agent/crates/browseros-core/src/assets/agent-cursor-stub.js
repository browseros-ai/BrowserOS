// biome-ignore-all lint: Classic script that defines the BrowserOSAgentCursor binding agent-pointer.js loads.
// biome-ignore format: Keep the injected script compact.
var BrowserOSAgentCursor = (() => {
  class MotionPlayer {
    constructor(canvas, options = {}) {
      this.canvas = canvas;
      this.options = options;
      this.g = canvas.getContext('2d');
      if (!this.g) throw new Error('canvas 2d context unavailable');
      this.pos = { x: 0, y: 0 };
      this.motion = null;
      this.arrival = null;
      this.ripple = null;
      this.destroyed = false;
      this.frame = (now) => {
        if (this.destroyed) return;
        if (this.motion) {
          const { from, to, start, duration } = this.motion;
          const progress = Math.min(1, Math.max(0, (now - start) / duration));
          const eased = progress * progress * (3 - 2 * progress);
          this.pos = { x: from.x + (to.x - from.x) * eased, y: from.y + (to.y - from.y) * eased };
          if (progress === 1) { this.motion = null; this.finishArrival(); }
        }
        this.paint(now);
        this.raf = requestAnimationFrame(this.frame);
      };
      this.raf = requestAnimationFrame(this.frame);
    }
    finishArrival() {
      const done = this.arrival;
      this.arrival = null;
      if (done) done();
    }
    place(point) {
      this.finishArrival();
      this.motion = null;
      this.pos = { x: point.x, y: point.y };
    }
    moveTo(point, options = {}) {
      this.finishArrival();
      if (this.destroyed) return Promise.resolve();
      const requested = options.params?.glideDurationMs || 1430;
      this.motion = { from: { ...this.pos }, to: { x: point.x, y: point.y }, start: performance.now(),
        duration: Math.max(1, options.reducedMotion ? Math.min(80, requested) : requested) };
      return new Promise((resolve) => { this.arrival = resolve; });
    }
    click() {
      this.ripple = { x: this.pos.x, y: this.pos.y, start: performance.now() };
    }
    destroy() {
      this.destroyed = true;
      cancelAnimationFrame(this.raf);
      this.motion = null;
      this.finishArrival();
    }
    paint(now) {
      const { canvas, g } = this;
      const dpr = window.devicePixelRatio || 1;
      const w = canvas.clientWidth;
      const h = canvas.clientHeight;
      if (canvas.width !== Math.round(w * dpr)) canvas.width = Math.round(w * dpr);
      if (canvas.height !== Math.round(h * dpr)) canvas.height = Math.round(h * dpr);
      g.setTransform(dpr, 0, 0, dpr, 0, 0);
      g.clearRect(0, 0, w, h);
      g.save();
      g.translate(this.pos.x, this.pos.y);
      g.beginPath();
      g.moveTo(0, 0);
      g.lineTo(4, 24);
      g.lineTo(10, 17);
      g.lineTo(16, 30);
      g.lineTo(21, 27);
      g.lineTo(14, 14);
      g.lineTo(23, 13);
      g.closePath();
      g.fillStyle = this.options.fill || '#4aa8ef';
      g.strokeStyle = '#fff';
      g.lineWidth = 2;
      g.lineJoin = 'round';
      g.fill();
      g.stroke();
      g.restore();
      if (this.ripple) {
        const age = (now - this.ripple.start) / 400;
        if (age >= 1) { this.ripple = null; return; }
        g.beginPath();
        g.arc(this.ripple.x, this.ripple.y, 6 + 26 * age, 0, 2 * Math.PI);
        g.strokeStyle = `rgba(74,168,239,${1 - age})`;
        g.lineWidth = 2;
        g.stroke();
      }
    }
  }
  return { MotionPlayer };
})();
