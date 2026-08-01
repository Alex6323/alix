/*!
 * <alix-logo> — the alix mitosis wordmark, as a dependency-free web component.
 *
 * No framework, no build step, no runtime. One file. Works in plain HTML,
 * React, Vue, Svelte — anywhere custom elements run.
 *
 *   <script src="alix-logo.js"></script>
 *   <alix-logo></alix-logo>                     <!-- plays once, rests on "alix" -->
 *   <alix-logo loop height="40"></alix-logo>    <!-- seamless loader -->
 *
 * Attributes
 *   color   any CSS color (hex / rgb / hsl / var(...) / named). Default:
 *           "currentColor" — inherits the surrounding text color, so it themes
 *           with your UI automatically. All the gradient sheen is DERIVED from
 *           this one color, so you keep the dimensional look on any theme.
 *   loop    "true"/"false" (or bare attribute). Default false. When false it
 *           plays the birth once and rests on the formed wordmark. When true it
 *           dissolves to the i-dot and reseeds forever — a calm loading state.
 *   height  px height of the mark. Width is derived (~2.31:1). Default 56.
 *   speed   playback multiplier. Default 1.
 *   paused  bare attribute: hold without animating.
 *
 * How it works (so you can trust it): the 1→2→4 "division" is NOT a path
 * topology morph — it's separate <circle>s under a gooey SVG filter
 * (feGaussianBlur → feColorMatrix threshold), so they visually neck and pinch
 * off. Each letter is ONE fixed-topology <path> whose points all start
 * collapsed on the cell center (a dot) and lerp out to the letterform while the
 * stroke tapers 44→20. So: no morph library, no D3, no topology change. Just a
 * requestAnimationFrame loop setting attributes. React-friendly, SSR-safe
 * (guards `window`), and it pauses when off-screen, when the tab is hidden, or
 * when the user prefers reduced motion.
 *
 * React:  <alix-logo loop="" ref={...} />   (or set attrs via the DOM node)
 * Public methods: el.replay(), el.play(), el.pause(), el.refresh()
 */
(function () {
  if (typeof window === 'undefined' || !window.customElements) return;
  if (customElements.get('alix-logo')) return;

  var ASPECT = 300 / 130; // viewBox is 300 wide × 130 tall

  var MARKUP =
    '<svg part="svg" viewBox="190 92 300 130" style="display:block;width:100%;height:100%;overflow:visible">' +
      '<defs>' +
        '<radialGradient id="cellFill" cx="0.42" cy="0.38" r="0.7">' +
          '<stop id="cf0" offset="0"></stop>' +
          '<stop id="cf1" offset="0.5"></stop>' +
          '<stop id="cf2" offset="1"></stop>' +
        '</radialGradient>' +
        '<filter id="goo" x="-30%" y="-60%" width="160%" height="220%">' +
          '<feGaussianBlur in="SourceGraphic" stdDeviation="8" result="blur"></feGaussianBlur>' +
          '<feColorMatrix in="blur" mode="matrix" values="1 0 0 0 0  0 1 0 0 0  0 0 1 0 0  0 0 0 20 -9" result="goo"></feColorMatrix>' +
          '<feComposite in="SourceGraphic" in2="goo" operator="atop"></feComposite>' +
        '</filter>' +
        '<linearGradient id="lettersGrad" gradientUnits="userSpaceOnUse" x1="210" y1="110" x2="470" y2="245">' +
          '<stop id="lg0" offset="0"></stop>' +
          '<stop id="lg1" offset="1"></stop>' +
        '</linearGradient>' +
      '</defs>' +
      '<g filter="url(#goo)">' +
        '<circle id="cell0" cx="340" cy="170" r="0" fill="url(#cellFill)"></circle>' +
        '<circle id="cell1" cx="340" cy="170" r="0" fill="url(#cellFill)"></circle>' +
        '<circle id="cell2" cx="340" cy="170" r="0" fill="url(#cellFill)"></circle>' +
        '<circle id="cell3" cx="340" cy="170" r="0" fill="url(#cellFill)"></circle>' +
        '<circle id="bridge0" cx="340" cy="170" r="0" fill="url(#cellFill)"></circle>' +
        '<circle id="bridge1" cx="340" cy="170" r="0" fill="url(#cellFill)"></circle>' +
        '<circle id="bridge2" cx="340" cy="170" r="0" fill="url(#cellFill)"></circle>' +
      '</g>' +
      '<g id="letters" fill="none" stroke="url(#lettersGrad)" stroke-linecap="round" stroke-linejoin="round">' +
        '<path id="letter0" stroke-width="0"></path>' +
        '<path id="letter1" stroke-width="0"></path>' +
        '<path id="letter2" stroke-width="0"></path>' +
        '<path id="letter3" stroke-width="0"></path>' +
      '</g>' +
      '<circle id="dotCell" cx="340" cy="112" r="0" fill="url(#cellFill)"></circle>' +
    '</svg>';

  class AlixLogo extends HTMLElement {
    static get observedAttributes() { return ['color', 'loop', 'height', 'speed', 'paused']; }

    constructor() {
      super();
      // ---- geometry (identical to the source design) ----
      this.W = 680; this.H = 340; this.MIDY = 170;
      this.D = 90;
      this.DOTUP = 58; this.DOTR = 9; this.SW = 20; this.BLOBW = 44;
      this.LX = [252, 322, 368, 438];
      // ---- timeline (seconds) ----
      this.TL = {
        div2: [0.5, 1.25], div4: [1.25, 2.0],
        bud: [2.0, 2.62], bloomStart: 2.0, bloomDur: 0.62, stagger: 0,
        hold: [2.62, 3.5], diss: [3.5, 4.1], reseed: [4.1, 4.85],
        dotMove: [3.5, 4.8],
      };
      this._raf = 0; this._last = null; this._elapsed = 0; this._tc = 0;
      this._onscreen = true; this._visible = true;
      this._sync = this._sync.bind(this);
    }

    get CYCLE() { return this.TL.reseed[1]; }
    get restAt() { return this.TL.bud[1]; }

    // ---- attribute-backed options ----
    get _loop() { var v = this.getAttribute('loop'); return v !== null && v !== 'false'; }
    get _height() { var h = parseFloat(this.getAttribute('height')); return isFinite(h) && h > 0 ? h : 56; }
    get _speed() { var s = parseFloat(this.getAttribute('speed')); return isFinite(s) && s > 0 ? s : 1; }
    get _paused() { return this.hasAttribute('paused'); }
    get _reduced() {
      return typeof matchMedia === 'function' && matchMedia('(prefers-reduced-motion: reduce)').matches;
    }

    connectedCallback() {
      if (!this._sr) {
        this._sr = this.attachShadow({ mode: 'open' });
        var host = document.createElement('style');
        host.textContent = ':host{display:inline-block;line-height:0}:host([hidden]){display:none}';
        this._sr.appendChild(host);
        var holder = document.createElement('div');
        holder.style.cssText = 'width:100%;height:100%';
        holder.innerHTML = MARKUP;
        this._sr.appendChild(holder);
      }
      this._applySize();
      this._applyColors();

      // repaint on theme changes (class / data-theme / style on <html> or <body>)
      if (!this._mo) {
        this._mo = new MutationObserver(() => this.refresh());
        var opt = { attributes: true, attributeFilter: ['class', 'style', 'data-theme'] };
        this._mo.observe(document.documentElement, opt);
        if (document.body) this._mo.observe(document.body, opt);
      }
      // pause when scrolled off-screen
      if (!this._io && 'IntersectionObserver' in window) {
        this._io = new IntersectionObserver((es) => { this._onscreen = es[0].isIntersecting; this._sync(); });
        this._io.observe(this);
      }
      document.addEventListener('visibilitychange', this._onVis = () => { this._visible = !document.hidden; this._sync(); });

      // The one-time reveal plays for everyone; reduced motion only suppresses the
      // continuous loop (handled in _sync / _play).
      this.draw(0);
      this._sync();
    }

    disconnectedCallback() {
      this._pause();
      if (this._mo) { this._mo.disconnect(); this._mo = null; }
      if (this._io) { this._io.disconnect(); this._io = null; }
      if (this._onVis) { document.removeEventListener('visibilitychange', this._onVis); this._onVis = null; }
    }

    attributeChangedCallback(name, oldV, newV) {
      if (!this._sr || oldV === newV) return;
      if (name === 'height') this._applySize();
      else if (name === 'color') this._applyColors();
      else if (name === 'loop' || name === 'speed') { this._elapsed = 0; this._last = null; this._sync(); }
      else if (name === 'paused') this._sync();
    }

    // ---- public API ----
    replay() { this._elapsed = 0; this._last = null; this.draw(0); this._sync(); }
    play() { this.removeAttribute('paused'); }
    pause() { this.setAttribute('paused', ''); }
    refresh() { this._applyColors(); if (!this._raf) this.draw(this._tc); }

    _applySize() {
      var h = this._height;
      this.style.height = h + 'px';
      this.style.width = (h * ASPECT).toFixed(1) + 'px';
    }

    // whether the loop should currently be running
    _sync() {
      var run = !this._paused && this._onscreen && this._visible && this.isConnected && !(this._reduced && this._loop);
      if (run) this._play(); else this._pause();
    }
    _play() {
      if (this._raf || (this._reduced && this._loop)) return;
      var self = this;
      var step = function (now) {
        if (self._last == null) self._last = now;
        var dt = (now - self._last) / 1000; self._last = now;
        if (dt > 0.1) dt = 0.1; // clamp long gaps (tab was away) so we never leap
        self._elapsed += dt * self._speed;
        var tc = self._loop ? (self._elapsed % self.CYCLE) : Math.min(self._elapsed, self.restAt);
        self.draw(tc);
        if (!self._loop && self._elapsed >= self.restAt) { self._raf = 0; return; }
        self._raf = requestAnimationFrame(step);
      };
      this._raf = requestAnimationFrame(step);
    }
    _pause() { if (this._raf) cancelAnimationFrame(this._raf); this._raf = 0; this._last = null; }

    // ---------- math ----------
    clamp01(v) { return v < 0 ? 0 : v > 1 ? 1 : v; }
    lerp(a, b, t) { return a + (b - a) * t; }
    easeInOut(p) { return 0.5 - 0.5 * Math.cos(Math.PI * this.clamp01(p)); }
    smoother(p) { p = this.clamp01(p); return p * p * p * (p * (p * 6 - 15) + 10); }
    backOut(p) { p = this.clamp01(p); var c1 = 1.70158, c3 = c1 + 1; return 1 + c3 * Math.pow(p - 1, 3) + c1 * Math.pow(p - 1, 2); }
    hermite(p0, p1, m0, m1, s) { var s2 = s * s, s3 = s2 * s; return (2 * s3 - 3 * s2 + 1) * p0 + (s3 - 2 * s2 + s) * m0 + (-2 * s3 + 3 * s2) * p1 + (s3 - s2) * m1; }
    drift(seed, t, freq) { return (Math.sin(t * freq + seed) + 0.6 * Math.sin(t * freq * 1.73 + seed * 2.1) + 0.34 * Math.sin(t * freq * 2.94 + seed * 3.7)) * 0.45; }
    driftEnv(tc) { return this.smoother(this.clamp01((tc - 0.5) / 0.3)) * (1 - this.smoother(this.clamp01((tc - 3.3) / 0.8))); }
    sdiv(t, w) { return this.clamp01((t - w[0]) / (w[1] - w[0])); }
    gauss(t, c, w) { var z = (t - c) / w; return Math.exp(-z * z); }
    get pinchT2() { var a = this.TL.div2[0], b = this.TL.div2[1]; return a + 0.7 * (b - a); }
    get pinchT4() { var a = this.TL.div4[0], b = this.TL.div4[1]; return a + 0.7 * (b - a); }

    // ---------- color: one base color → derived sheen ----------
    _toRGB(c) {
      if (!c) return [157, 140, 240];
      c = ('' + c).trim();
      if (c[0] === '#') { var h = c.slice(1); if (h.length === 3) h = h.split('').map(function (x) { return x + x; }).join(''); var n = parseInt(h, 16); return [(n >> 16) & 255, (n >> 8) & 255, n & 255]; }
      var m = c.match(/rgba?\(([^)]+)\)/i);
      if (m) { var p = m[1].split(/[ ,\/]+/).map(function (s) { return parseFloat(s); }); return [p[0] || 0, p[1] || 0, p[2] || 0]; }
      return [157, 140, 240];
    }
    mix(rgb, t, a) { return rgb.map(function (v, i) { return Math.round(v + (t[i] - v) * a); }); }
    rgbStr(a) { return 'rgb(' + a[0] + ',' + a[1] + ',' + a[2] + ')'; }
    _resolveBase() {
      var attr = this.getAttribute('color');
      if (attr && attr !== 'currentColor' && attr !== 'auto') this.style.color = attr; else this.style.color = '';
      return this._toRGB(getComputedStyle(this).color);
    }
    _applyColors() {
      var sr = this._sr; if (!sr) return;
      var rgb = this._resolveBase();
      var self = this;
      var set = function (id, a) { var s = sr.getElementById(id); if (s) s.setAttribute('stop-color', self.rgbStr(a)); };
      // Flat brand color: every gradient stop is the base color, so the cells and
      // letters render as one solid fill (no derived sheen).
      set('cf0', rgb); set('cf1', rgb); set('cf2', rgb);
      set('lg0', rgb); set('lg1', rgb);
    }

    // ---------- letter skeletons + blob→letter path builder ----------
    skel(i, cx) {
      var MY = this.MIDY, xh = MY - 32, base = MY + 32;
      if (i === 1) return [{ t: 'M', p: [[cx, MY - this.DOTUP - this.DOTR + this.SW / 2]] }, { t: 'L', p: [[cx, base]] }];
      if (i === 2) return [{ t: 'M', p: [[cx, xh]] }, { t: 'L', p: [[cx, base]] }];
      if (i === 3) return [
        { t: 'M', p: [[cx - 24, xh]] }, { t: 'L', p: [[cx + 24, base]] },
        { t: 'M', p: [[cx + 24, xh]] }, { t: 'L', p: [[cx - 24, base]] },
      ];
      var bx = cx - 6, by = MY - 2, rx = 28, ry = 30, k = 0.5523, sx = cx + 24;
      return [
        { t: 'M', p: [[bx, by - ry]] },
        { t: 'C', p: [[bx + rx * k, by - ry], [bx + rx, by - ry * k], [bx + rx, by]] },
        { t: 'C', p: [[bx + rx, by + ry * k], [bx + rx * k, by + ry], [bx, by + ry]] },
        { t: 'C', p: [[bx - rx * k, by + ry], [bx - rx, by + ry * k], [bx - rx, by]] },
        { t: 'C', p: [[bx - rx, by - ry * k], [bx - rx * k, by - ry], [bx, by - ry]] },
        { t: 'Z', p: [] },
        { t: 'M', p: [[sx, xh]] },
        { t: 'L', p: [[sx, base]] },
      ];
    }
    buildD(segs, cx, e) {
      var self = this;
      var lp = function (pt) { return self.lerp(cx, pt[0], e).toFixed(1) + ' ' + self.lerp(self.MIDY, pt[1], e).toFixed(1); };
      var d = '';
      for (var i = 0; i < segs.length; i++) { var s = segs[i]; if (s.t === 'Z') { d += 'Z '; continue; } d += s.t + ' ' + s.p.map(lp).join(' ') + ' '; }
      return d.trim();
    }

    cellXY(i, t) {
      var C = { x: this.W / 2, y: this.MIDY };
      var pair = i < 2 ? { x: C.x - this.D, y: C.y } : { x: C.x + this.D, y: C.y };
      var slot = { x: this.LX[i], y: this.MIDY };
      var t0 = this.TL.div2[0], t1 = this.TL.div2[1], t2 = this.TL.div4[1];
      var mx = (slot.x - C.x) * 0.5, my = (slot.y - C.y) * 0.5;
      var x, y;
      if (t <= t0) { x = C.x; y = C.y; }
      else if (t < t1) { var s = this.clamp01((t - t0) / (t1 - t0)); x = this.hermite(C.x, pair.x, 0, mx, s); y = this.hermite(C.y, pair.y, 0, my, s); }
      else if (t < t2) { var s2 = this.clamp01((t - t1) / (t2 - t1)); x = this.hermite(pair.x, slot.x, mx, 0, s2); y = this.hermite(pair.y, slot.y, my, 0, s2); }
      else { x = slot.x; y = slot.y; }
      var env = this.driftEnv(t);
      x += this.drift(i * 1.7 + 0.3, t, 1.05) * 3.2 * env;
      y += this.drift(i * 2.3 + 1.4, t, 0.92) * 3.6 * env;
      return { x: x, y: y };
    }
    genR(t) {
      var R1 = 44, R2 = 32, R3 = 22, self = this;
      var shrink = function (s) { return self.smoother(self.clamp01((s - 0.58) / 0.42)); };
      var d2a = this.TL.div2[0], d2b = this.TL.div2[1], d4a = this.TL.div4[0], d4b = this.TL.div4[1];
      if (t <= d2a) return R1;
      if (t < d2b) return this.lerp(R1, R2, shrink(this.sdiv(t, this.TL.div2)));
      if (t < d4a) return R2;
      if (t < d4b) return this.lerp(R2, R3, shrink(this.sdiv(t, this.TL.div4)));
      return R3;
    }
    cellR(i, t) {
      var r = this.genR(t);
      r *= 1 + 0.11 * this.gauss(t, this.pinchT2, 0.08) + 0.11 * this.gauss(t, this.pinchT4, 0.08);
      var env = this.driftEnv(t);
      r *= 1 + env * (0.055 * Math.sin(t * 2.5 + i * 1.3) + 0.03 * Math.sin(t * 4.1 + i * 2.7));
      r *= (1 - this.clamp01(this.bloomP(i, t) * 2.5));
      return Math.max(0, r);
    }
    bloomP(i, t) { return this.clamp01((t - (this.TL.bloomStart + i * this.TL.stagger)) / this.TL.bloomDur); }

    draw(tc) {
      var sr = this._sr; if (!sr) return;
      this._tc = tc;
      var R1 = 44, cx = this.W / 2, cyc = this.MIDY;
      var rs0 = this.TL.reseed[0];
      var inReseed = tc >= rs0;

      var P = [], Rd = [], i;
      for (i = 0; i < 4; i++) {
        var x, y, r;
        if (inReseed) { x = cx; y = cyc; r = 0; }
        else { var pp = this.cellXY(i, tc); x = pp.x; y = pp.y; r = this.cellR(i, tc); }
        P[i] = { x: x, y: y }; Rd[i] = r;
        var c = sr.getElementById('cell' + i);
        if (c) { c.setAttribute('cx', x.toFixed(1)); c.setAttribute('cy', y.toFixed(1)); c.setAttribute('r', r.toFixed(1)); }
      }
      var self = this;
      var neck = function (s, r) { return r * 0.9 * (1 - self.smoother(self.clamp01(s / 0.7))); };
      var setB = function (id, a, b, r) {
        var el = sr.getElementById(id); if (!el) return;
        el.setAttribute('cx', ((a.x + b.x) / 2).toFixed(1));
        el.setAttribute('cy', ((a.y + b.y) / 2).toFixed(1));
        el.setAttribute('r', Math.max(0, r).toFixed(1));
      };
      var inDiv2 = !inReseed && tc >= this.TL.div2[0] && tc < this.TL.div2[1];
      var inDiv4 = !inReseed && tc >= this.TL.div4[0] && tc < this.TL.div4[1];
      setB('bridge0', P[0], P[2], inDiv2 ? neck(this.sdiv(tc, this.TL.div2), (Rd[0] + Rd[2]) / 2) : 0);
      setB('bridge1', P[0], P[1], inDiv4 ? neck(this.sdiv(tc, this.TL.div4), (Rd[0] + Rd[1]) / 2) : 0);
      setB('bridge2', P[2], P[3], inDiv4 ? neck(this.sdiv(tc, this.TL.div4), (Rd[2] + Rd[3]) / 2) : 0);

      var dot = sr.getElementById('dotCell');
      if (dot) {
        var dx, dy, dr;
        var dm0 = this.TL.dotMove[0], dm1 = this.TL.dotMove[1];
        if (tc >= dm0) {
          var raw = this.clamp01((tc - dm0) / (dm1 - dm0));
          var mp = this.easeInOut(raw), rp = raw * raw;
          dx = this.lerp(this.LX[2], cx, mp); dy = this.lerp(this.MIDY - this.DOTUP, cyc, mp); dr = this.lerp(this.DOTR, R1, rp);
        } else {
          var ba = this.TL.bud[0], bb = this.TL.bud[1];
          var src = this.cellXY(2, tc), tgt = { x: this.LX[2], y: this.MIDY - this.DOTUP };
          var bp = this.easeInOut((tc - ba) / (bb - ba));
          dx = this.lerp(src.x, tgt.x, bp); dy = this.lerp(src.y, tgt.y, bp);
          dr = this.DOTR * this.backOut(this.clamp01((tc - ba) / (bb - ba)));
        }
        dot.setAttribute('cx', dx.toFixed(1)); dot.setAttribute('cy', dy.toFixed(1)); dot.setAttribute('r', Math.max(0, dr).toFixed(1));
      }

      for (i = 0; i < 4; i++) {
        var p = sr.getElementById('letter' + i); if (!p) continue;
        var lbp = this.bloomP(i, tc);
        if (lbp <= 0 || inReseed) { p.setAttribute('d', ''); p.setAttribute('stroke-width', '0'); continue; }
        var e = this.smoother(lbp);
        p.setAttribute('d', this.buildD(this.skel(i, this.LX[i]), this.LX[i], e));
        p.setAttribute('stroke-width', this.lerp(this.BLOBW, this.SW, e).toFixed(1));
      }
      var letters = sr.getElementById('letters');
      if (letters) {
        var a = 1, da = this.TL.diss[0], db = this.TL.diss[1];
        if (tc >= da && tc < db) a = 1 - this.smoother((tc - da) / (db - da));
        else if (tc >= db) a = 0;
        letters.style.opacity = a.toFixed(3);
      }
    }
  }

  customElements.define('alix-logo', AlixLogo);
})();
