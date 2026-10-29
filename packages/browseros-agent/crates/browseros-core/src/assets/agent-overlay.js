// biome-ignore-all lint: Injected through Runtime.evaluate; Rust fills the __BROWSEROS_OVERLAY_*__ placeholders.
// biome-ignore format: Keep the injected expression byte-oriented.
// Icons: Phosphor (MIT License, Copyright (c) 2020 Phosphor Icons), path data inlined.
((cmd) => {
  try {
    if (window.top !== window) return { status: 'skipped', reason: 'not-top' };
    if (!document.documentElement || typeof document.createElement !== 'function') {
      return { status: 'skipped', reason: 'unsupported' };
    }
    const key = Symbol.for('browseros.agentOverlay');
    let api = window[key];
    if (!api) {
      const ICON = {
        grab: 'M188,76a31.85,31.85,0,0,0-11.21,2,32,32,0,0,0-48.79-11A32,32,0,0,0,76,92v16H68a32,32,0,0,0-32,32v12a92,92,0,0,0,184,0V108A32,32,0,0,0,188,76Zm8,76a68,68,0,0,1-136,0V140a8,8,0,0,1,8-8h8v20a12,12,0,0,0,24,0V92a8,8,0,0,1,16,0v28a12,12,0,0,0,24,0V92a8,8,0,0,1,16,0v28a12,12,0,0,0,24,0V108a8,8,0,0,1,16,0Z',
        palm: 'M216,104v48a88,88,0,0,1-176,0V64a16,16,0,0,1,32,0v56a8,8,0,0,0,16,0V32a16,16,0,0,1,32,0v80a8,8,0,0,0,16,0V48a16,16,0,0,1,32,0v80.67A48.08,48.08,0,0,0,128,176a8,8,0,0,0,16,0,32,32,0,0,1,32-32,8,8,0,0,0,8-8V104a16,16,0,0,1,32,0Z',
        check: 'M232.49,80.49l-128,128a12,12,0,0,1-17,0l-56-56a12,12,0,1,1,17-17L96,183,215.51,63.51a12,12,0,0,1,17,17Z',
        back: 'M236,144a68.07,68.07,0,0,1-68,68H80a12,12,0,0,1,0-24h88a44,44,0,0,0,0-88H61l27.52,27.51a12,12,0,0,1-17,17l-48-48a12,12,0,0,1,0-17l48-48a12,12,0,1,1,17,17L61,76H168A68.08,68.08,0,0,1,236,144Z',
        close: 'M208.49,191.51a12,12,0,0,1-17,17L128,145,64.49,208.49a12,12,0,0,1-17-17L111,128,47.51,64.49a12,12,0,0,1,17-17L128,111l63.51-63.52a12,12,0,0,1,17,17L145,128Z',
      };
      const icon = (name) => `<svg viewBox="0 0 256 256" aria-hidden="true"><path fill="currentColor" d="${ICON[name]}"/></svg>`;
      // The neo blue tokens match claw-app's newtab styles. @property does not register inside a
      // shadow tree, so the travelling band rotates a conic layer instead of animating an angle.
      const CSS = `
:host { all: initial !important; }
@media print { :host { display: none !important; } }
* { box-sizing: border-box; }
.layer { position: fixed; inset: 0; pointer-events: none; font: 400 14px/1.3 -apple-system, BlinkMacSystemFont, "Segoe UI", system-ui, Roboto, "Helvetica Neue", Arial, sans-serif;
  color: #fff; -webkit-font-smoothing: antialiased; --out: cubic-bezier(0.16, 1, 0.3, 1); --spring: cubic-bezier(0.34, 1.56, 0.64, 1); }
.layer[data-capture="hidden"] { visibility: hidden !important; }
.shield { position: absolute; inset: 0; pointer-events: none; }
.layer[data-held="on"] .shield { pointer-events: auto; cursor: default; touch-action: none; }
.layer[data-yield="on"], .layer[data-yield="on"] * { pointer-events: none !important; }
.tint { position: absolute; inset: 0; opacity: 0; transition: opacity 0.7s var(--out);
  background: radial-gradient(120% 90% at 50% 50%, transparent 62%, rgba(2,84,236,0.12) 100%), linear-gradient(rgba(2,84,236,0.035), rgba(2,84,236,0.035)); }
.layer[data-state="working"] .tint { opacity: 1; }
.layer[data-state="between"] .tint { opacity: 0.6; }
.layer[data-touch="on"] .tint { opacity: 1; background: radial-gradient(120% 90% at 50% 50%, transparent 40%, rgba(2,84,236,0.24) 100%), linear-gradient(rgba(12,39,66,0.10), rgba(12,39,66,0.10)); }
.edge { position: absolute; inset: 0; padding: 3px; overflow: hidden; opacity: 0; transform: scale(1.02);
  -webkit-mask: linear-gradient(#000 0 0) content-box, linear-gradient(#000 0 0); -webkit-mask-composite: xor; mask-composite: exclude;
  transition: opacity 0.6s var(--out), transform 0.8s var(--out); }
.edge i { position: absolute; left: 50%; top: 50%; width: 200vmax; height: 200vmax; margin: -100vmax 0 0 -100vmax;
  background: conic-gradient(rgba(2,84,236,0) 0deg, #0254ec 30deg, #7fb4ff 62deg, #fff 72deg, #3d7bff 92deg, rgba(2,84,236,0) 150deg, rgba(2,84,236,0) 180deg, #0254ec 210deg, #7fb4ff 242deg, #3d7bff 270deg, rgba(2,84,236,0) 330deg);
  animation: spin 4.8s linear infinite; }
.edge-base { position: absolute; inset: 0; opacity: 0; transition: opacity 0.5s, box-shadow 0.5s;
  box-shadow: inset 0 0 0 1.5px rgba(2,84,236,0.45), inset 0 0 22px rgba(2,84,236,0.16); }
.layer[data-state="working"] .edge { opacity: 1; transform: none; }
.layer[data-state="between"] .edge { opacity: 0.6; transform: none; }
.layer[data-state="between"] .edge i { animation-duration: 8s; }
.layer[data-touch="on"] .edge i { animation-duration: 1.6s; }
.layer[data-state="working"] .edge-base, .layer[data-state="between"] .edge-base { opacity: 1; }
.layer[data-state="finished"] .edge-base { opacity: 1; box-shadow: inset 0 0 0 2px rgba(47,224,140,0.9), inset 0 0 22px rgba(47,224,140,0.18); }
.layer[data-fade="on"] .edge-base, .layer[data-fade="on"] .island { opacity: 0 !important; transition: opacity 1.2s; }
.bloom { position: absolute; inset: 0; opacity: 0; background: radial-gradient(70% 60% at 50% 100%, rgba(47,224,140,0.25), transparent 70%); }
.layer[data-state="finished"] .bloom { animation: bloom 1.8s var(--out) forwards; }
.ring { position: absolute; left: 0; top: 0; width: 10px; height: 10px; border-radius: 12px; opacity: 0;
  box-shadow: 0 0 0 2px #0254ec, 0 0 0 7px rgba(2,84,236,0.16), 0 0 28px 4px rgba(2,84,236,0.25);
  transition: transform 0.6s cubic-bezier(0.22,1,0.36,1), width 0.6s cubic-bezier(0.22,1,0.36,1), height 0.6s cubic-bezier(0.22,1,0.36,1), opacity 0.4s; }
.layer[data-state="working"] .ring[data-show="on"] { opacity: 1; }
.lens { position: absolute; left: 0; top: 0; width: 64px; height: 64px; margin: -32px 0 0 -32px; border-radius: 50%; opacity: 0; scale: 0.6;
  display: grid; place-items: center; color: #fff; background: rgba(2,84,236,0.55); -webkit-backdrop-filter: blur(6px); backdrop-filter: blur(6px);
  box-shadow: 0 0 0 2px rgba(255,255,255,0.95), 0 0 0 8px rgba(2,84,236,0.22), 0 10px 30px -6px rgba(2,22,51,0.5);
  transition: opacity 0.25s, scale 0.35s var(--spring); }
.lens svg { width: 24px; height: 24px; }
.layer[data-touch="on"] .lens { opacity: 1; scale: 1; animation: breathe 1.6s ease-in-out infinite; }
.ripple { position: absolute; width: 18px; height: 18px; margin: -9px 0 0 -9px; border-radius: 50%; border: 2px solid rgba(255,255,255,0.95);
  box-shadow: 0 0 0 2px rgba(2,84,236,0.6); animation: ripple 0.7s var(--out) forwards; }
.island { position: absolute; bottom: 18px; left: 50%; width: min(460px, calc(100% - 32px)); transform: translate(-50%, 150%); opacity: 0;
  pointer-events: auto; color: #fff; background: rgba(12,39,66,0.9); -webkit-backdrop-filter: blur(18px) saturate(160%); backdrop-filter: blur(18px) saturate(160%);
  border: 1px solid rgba(255,255,255,0.12); border-radius: 18px;
  box-shadow: 0 1px 0 rgba(255,255,255,0.14) inset, 0 18px 50px -12px rgba(2,22,51,0.55), 0 0 0 6px rgba(2,84,236,0.10);
  transition: transform 0.7s var(--spring), opacity 0.4s var(--out), background 0.4s, color 0.4s; }
.layer[data-state] .island { transform: translate(-50%, 0); opacity: 1; }
.layer[data-state="person"] .island, .layer[data-state="handing"] .island { background: rgba(255,255,255,0.97); color: #0c2742; border-color: rgba(2,84,236,0.2); box-shadow: 0 18px 50px -12px rgba(2,22,51,0.35); }
.island[data-nudge="on"] { animation: nudge 0.42s var(--out); }
.row { display: flex; align-items: center; gap: 12px; padding: 10px; }
.mark { position: relative; width: 36px; height: 36px; flex: none; display: grid; place-items: center; }
.mark > svg { position: relative; z-index: 1; width: 30px; height: 30px; border-radius: 8px; display: block; }
.orbit { position: absolute; inset: -3px; border-radius: 11px; overflow: hidden; opacity: 0; transition: opacity 0.3s; }
.orbit i { position: absolute; left: 50%; top: 50%; width: 64px; height: 64px; margin: -32px 0 0 -32px;
  background: conic-gradient(transparent 0deg, #7fb4ff 90deg, #fff 120deg, transparent 200deg); animation: spin 1.4s linear infinite; }
.layer[data-state="working"] .orbit, .layer[data-state="between"] .orbit { opacity: 1; }
.ok { position: absolute; right: -4px; bottom: -4px; z-index: 2; width: 17px; height: 17px; border-radius: 50%; display: grid; place-items: center;
  background: #2fe08c; color: #0c2742; box-shadow: 0 0 0 2px #0c2742; transform: scale(0); transition: transform 0.45s var(--spring); }
.ok svg { width: 11px; height: 11px; }
.layer[data-state="finished"] .ok { transform: scale(1); }
.copy { flex: 1; min-width: 0; }
.who { font-size: 12px; font-weight: 600; letter-spacing: 0.01em; opacity: 0.7; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.what { position: relative; height: 19px; overflow: hidden; font-size: 15px; font-weight: 700; letter-spacing: -0.01em; line-height: 1.25; }
.what span { position: absolute; left: 0; top: 0; max-width: 100%; white-space: nowrap; overflow: hidden; text-overflow: ellipsis;
  transition: transform 0.45s var(--out), opacity 0.35s, filter 0.35s; }
.what span[data-at="out"] { transform: translateY(-110%); opacity: 0; filter: blur(3px); }
.what span[data-at="in"] { transform: translateY(110%); opacity: 0; filter: blur(3px); }
button { font: inherit; font-size: 13.5px; font-weight: 700; line-height: 1; border-radius: 11px; border: 0; margin: 0; cursor: pointer; white-space: nowrap;
  padding: 9px 13px; display: inline-flex; align-items: center; gap: 7px; transition: transform 0.15s, background 0.2s; }
button:active { transform: scale(0.97); }
button svg { width: 16px; height: 16px; flex: none; }
button:focus-visible { outline: 2px solid #7fb4ff; outline-offset: 2px; }
.ghost { background: rgba(255,255,255,0.10); color: #fff; }
.ghost:hover { background: rgba(255,255,255,0.18); }
.primary { background: #0254ec; color: #fff; }
.primary:hover { background: #0246c6; }
.layer[data-state="person"] .ghost, .layer[data-state="handing"] .ghost, .layer[data-state="finished"] .ghost { background: #eaf1ff; color: #0254ec; }
.layer[data-state="finished"] .ghost { background: rgba(255,255,255,0.10); color: #fff; }
.layer[data-state="handing"] .main { opacity: 0.6; pointer-events: none; }
.layer[data-touch="on"] .main { display: none; }
.ask { display: grid; grid-template-rows: 0fr; transition: grid-template-rows 0.5s var(--out); }
.ask > div { overflow: hidden; }
.layer[data-touch="on"] .ask { grid-template-rows: 1fr; }
.ask-inner { padding: 2px 14px 14px 58px; display: grid; gap: 10px; }
.ask p { margin: 0; font-size: 13.5px; line-height: 1.45; color: rgba(255,255,255,0.78); }
.actions { display: flex; gap: 8px; flex-wrap: wrap; }
@keyframes spin { to { transform: rotate(360deg); } }
@keyframes bloom { 0% { opacity: 0; } 25% { opacity: 1; } 100% { opacity: 0; } }
@keyframes ripple { from { transform: scale(0.4); opacity: 1; } to { transform: scale(3.4); opacity: 0; } }
@keyframes breathe { 50% { box-shadow: 0 0 0 2px rgba(255,255,255,0.95), 0 0 0 14px rgba(2,84,236,0.10), 0 10px 30px -6px rgba(2,22,51,0.5); } }
@keyframes nudge { 0%, 100% { translate: 0 0; } 20% { translate: -7px 0; } 45% { translate: 6px 0; } 70% { translate: -3px 0; } }
@media (max-width: 560px) { .ask-inner { padding-left: 14px; } .main span { display: none; } }
@media (prefers-reduced-motion: reduce) {
  .edge i, .orbit i, .lens, .ripple, .island[data-nudge="on"], .bloom { animation: none !important; }
  .ring, .island, .edge { transition: opacity 0.2s !important; }
}
@media (prefers-reduced-transparency: reduce) {
  .island { background: #0c2742; -webkit-backdrop-filter: none; backdrop-filter: none; }
  .lens { background: #0254ec; -webkit-backdrop-filter: none; backdrop-filter: none; }
}`;
      const COPY = {
        working: 'BrowserOS neo is working on this page',
        person: 'You have control',
        finished: 'BrowserOS neo',
        paused: 'neo is paused on this page',
        handing: 'Handing back to neo',
        done: 'Finished. The page is yours.',
        between: 'Working out the next step',
      };
      const HELD = new Set(['working', 'between']);
      const SWALLOW = ['click', 'dblclick', 'auxclick', 'contextmenu', 'mousedown', 'mouseup', 'pointerdown', 'pointerup',
        'pointermove', 'mousemove', 'touchstart', 'touchmove', 'touchend', 'dragstart', 'dragover', 'drop', 'wheel'];
      const KEYS = ['keydown', 'keypress', 'keyup', 'beforeinput'];
      let host = null;
      let layer = null;
      let el = {};
      let state = null;
      let label = '';
      let yielded = false;
      let captureDepth = 0;
      let queue = [];
      let lastReachIn = 0;
      let timers = {};
      const clear = (name) => { clearTimeout(timers[name]); timers[name] = null; };
      const later = (name, ms, fn) => { clear(name); timers[name] = setTimeout(fn, ms); };
      const emit = (event) => {
        const entry = { ...event, at: Date.now() };
        queue.push(entry);
        if (queue.length > 20) queue = queue.slice(-20);
        try {
          if (typeof globalThis.__browserosOverlay === 'function') globalThis.__browserosOverlay(JSON.stringify(entry));
        } catch (_) {}
      };
      const held = () => !!layer && HELD.has(state) && !yielded;
      const sync = () => {
        if (!layer) return;
        layer.dataset.state = state;
        layer.dataset.held = held() ? 'on' : 'off';
        layer.dataset.yield = yielded ? 'on' : 'off';
        if (!held()) layer.dataset.touch = 'off';
      };
      const swapText = (text) => {
        const current = el.what.querySelector('span:not([data-at="out"])');
        if (current && current.textContent === text) return;
        const next = document.createElement('span');
        next.dataset.at = 'in';
        next.textContent = text;
        el.what.append(next);
        requestAnimationFrame(() => requestAnimationFrame(() => {
          delete next.dataset.at;
          if (current) current.dataset.at = 'out';
        }));
        setTimeout(() => { if (current) current.remove(); }, 500);
      };
      const setMain = (text, iconName, handler) => {
        el.main.innerHTML = `${icon(iconName)}<span></span>`;
        el.main.querySelector('span').textContent = text;
        el.main.onclick = (event) => { if (event.isTrusted) handler(); };
      };
      const setRing = (rect) => {
        if (!rect || !(rect.width > 0) || !(rect.height > 0)) { el.ring.dataset.show = 'off'; return; }
        const pad = 6;
        el.ring.style.width = `${rect.width + pad * 2}px`;
        el.ring.style.height = `${rect.height + pad * 2}px`;
        el.ring.style.transform = `translate(${rect.x - pad}px, ${rect.y - pad}px)`;
        el.ring.dataset.show = 'on';
      };
      const releaseTouch = () => { if (layer) layer.dataset.touch = 'off'; };
      const takeOver = (via) => {
        if (!held() && !(yielded && HELD.has(state))) return;
        yielded = false;
        clear('yield');
        enter('person', { label: COPY.paused });
        emit({ type: 'take-over', via });
      };
      const handBack = () => {
        if (state !== 'person') return;
        enter('handing', { label: COPY.handing });
        emit({ type: 'hand-back' });
      };
      const enter = (next, command) => {
        const previous = state;
        state = next;
        if (layer) delete layer.dataset.fade;
        if (typeof command.label === 'string' && command.label) label = command.label;
        clear('finish');
        if (next === 'working' || next === 'between') {
          el.who.textContent = COPY.working;
          swapText(next === 'between' ? (command.label || COPY.between) : label);
          if (next === 'working') setRing(command.rect); else setRing(null);
          if (previous !== 'working' && previous !== 'between') setMain('Take over', 'grab', () => takeOver('button'));
        } else if (next === 'person' || next === 'handing') {
          setRing(null);
          el.who.textContent = COPY.person;
          swapText(next === 'handing' ? COPY.handing : COPY.paused);
          setMain('Hand back to neo', 'back', handBack);
        } else if (next === 'finished') {
          setRing(null);
          el.who.textContent = COPY.finished;
          swapText(COPY.done);
          setMain('Close', 'close', dispose);
          later('finish', command.finishMs || 3000, () => {
            if (layer) layer.dataset.fade = 'on';
            later('finish', 1400, dispose);
          });
        }
        sync();
      };
      const spawnRipple = (x, y) => {
        const ripple = document.createElement('span');
        ripple.className = 'ripple';
        ripple.style.left = `${x}px`;
        ripple.style.top = `${y}px`;
        layer.append(ripple);
        setTimeout(() => ripple.remove(), 750);
      };
      const onShield = (event) => {
        if (!event.isTrusted || !layer) return;
        event.stopPropagation();
        if (!held()) return;
        if (event.cancelable) event.preventDefault();
        const point = event.touches && event.touches[0] ? event.touches[0] : event;
        if (event.type === 'pointermove' || event.type === 'pointerdown' || event.type === 'touchstart') {
          el.lens.style.transform = `translate(${point.clientX}px, ${point.clientY}px)`;
          if (layer.dataset.touch !== 'on') {
            layer.dataset.touch = 'on';
            if (Date.now() - lastReachIn > 10000) { lastReachIn = Date.now(); emit({ type: 'reach-in', phase: state }); }
          }
          later('touch', 2200, releaseTouch);
        }
        if (event.type === 'pointerdown' || event.type === 'touchstart') {
          spawnRipple(point.clientX, point.clientY);
          delete el.island.dataset.nudge;
          void el.island.offsetWidth;
          el.island.dataset.nudge = 'on';
        }
      };
      const onKey = (event) => {
        if (!event.isTrusted || !held()) return;
        if (event.type === 'keydown' && event.key === 'Escape') {
          event.preventDefault();
          event.stopImmediatePropagation();
          takeOver('esc');
          return;
        }
        event.preventDefault();
        event.stopImmediatePropagation();
      };
      const keep = (event) => { event.stopPropagation(); };
      const mount = () => {
        host = document.createElement('browseros-agent-overlay');
        host.setAttribute('data-browseros-agent-overlay', '');
        host.setAttribute('aria-hidden', 'true');
        for (const [name, value] of Object.entries({ all: 'initial', position: 'fixed', inset: '0', 'pointer-events': 'none',
          'z-index': '2147483646', display: 'block', contain: 'strict' })) {
          host.style.setProperty(name, value, 'important');
        }
        if (typeof host.attachShadow !== 'function') { host = null; return false; }
        const shadow = host.attachShadow({ mode: 'closed' });
        if (typeof CSSStyleSheet === 'function' && 'adoptedStyleSheets' in shadow) {
          // CSSOM also works on pages whose CSP forbids inline style elements.
          const sheet = new CSSStyleSheet();
          sheet.replaceSync(CSS);
          shadow.adoptedStyleSheets = [sheet];
        } else {
          const style = document.createElement('style');
          style.textContent = CSS;
          shadow.appendChild(style);
        }
        layer = document.createElement('div');
        layer.className = 'layer';
        layer.innerHTML = `<div class="tint"></div><div class="shield"></div><div class="edge"><i></i></div><div class="edge-base"></div>
<div class="bloom"></div><div class="ring"></div><div class="lens">${icon('palm')}</div>
<div class="island"><div class="row"><div class="mark"><span class="orbit"><i></i></span>__BROWSEROS_OVERLAY_MARK__<span class="ok">${icon('check')}</span></div>
<div class="copy"><div class="who"></div><div class="what"></div></div><button class="ghost main" type="button"></button></div>
<div class="ask"><div><div class="ask-inner"><p>Clicks and typing here are held while neo works, so you don't bump into each other. Take over to use the page yourself.</p>
<div class="actions"><button class="primary take" type="button">${icon('grab')}Take over</button><button class="ghost let" type="button">Let neo finish</button></div></div></div></div></div>`;
        shadow.appendChild(layer);
        el = {
          shield: layer.querySelector('.shield'), ring: layer.querySelector('.ring'), lens: layer.querySelector('.lens'),
          island: layer.querySelector('.island'), who: layer.querySelector('.who'), what: layer.querySelector('.what'),
          main: layer.querySelector('.main'),
        };
        for (const type of SWALLOW) el.shield.addEventListener(type, onShield, { passive: false });
        for (const type of SWALLOW) el.island.addEventListener(type, keep);
        el.shield.addEventListener('pointerleave', (event) => { if (event.isTrusted) later('touch', 900, releaseTouch); });
        layer.querySelector('.take').addEventListener('click', (event) => { if (event.isTrusted) takeOver('button'); });
        layer.querySelector('.let').addEventListener('click', (event) => {
          if (!event.isTrusted) return;
          releaseTouch();
          emit({ type: 'let-finish' });
        });
        for (const type of KEYS) window.addEventListener(type, onKey, true);
        document.documentElement.appendChild(host);
        return true;
      };
      const dispose = () => {
        for (const name of Object.keys(timers)) clear(name);
        for (const type of KEYS) window.removeEventListener(type, onKey, true);
        try { if (host) host.remove(); } catch (_) {}
        host = layer = null;
        el = {};
        state = null;
        yielded = false;
        captureDepth = 0;
      };
      const arm = (expiryMs) => later('expiry', expiryMs || 35000, dispose);
      const drain = () => { const events = queue; queue = []; return events; };
      api = {
        show(command) {
          if (command.phase === 'quiet') { dispose(); return { status: 'disposed', events: drain() }; }
          if (host && !host.isConnected) { dispose(); }
          if (!host && !mount()) return { status: 'skipped', reason: 'unsupported', events: drain() };
          // A local take over wins until the server has heard it and answers with a later phase.
          if ((state === 'person' || state === 'handing') && HELD.has(command.phase) && !command.resume) {
            arm(command.expiryMs);
            return { status: 'person', events: drain() };
          }
          enter(command.phase, command);
          arm(command.expiryMs);
          return { status: 'shown', state, events: drain() };
        },
        beat(expiryMs) {
          if (!host || !host.isConnected) return { status: 'absent', events: drain() };
          arm(expiryMs);
          return { status: state, events: drain() };
        },
        yieldInput() {
          if (!layer) return 'absent';
          if (state === 'person' || state === 'handing') return 'person';
          yielded = true;
          // A yield that is never resumed must not leave the page unheld for good.
          later('yield', 3000, () => { yielded = false; sync(); });
          sync();
          return 'yielded';
        },
        resumeInput() {
          if (!layer) return false;
          yielded = false;
          clear('yield');
          sync();
          return true;
        },
        hide() {
          if (!layer) return false;
          captureDepth += 1;
          layer.dataset.capture = 'hidden';
          return true;
        },
        reveal() {
          if (!layer) return false;
          captureDepth = Math.max(0, captureDepth - 1);
          if (!captureDepth) delete layer.dataset.capture;
          return true;
        },
        dispose() { dispose(); return true; },
      };
      Object.defineProperty(window, key, { value: api, enumerable: false, configurable: true, writable: true });
    }
    if (cmd.op === 'beat') return api.beat(cmd.expiryMs);
    if (cmd.op === 'dispose') return { status: api.dispose() ? 'disposed' : 'absent' };
    return api.show(cmd);
  } catch (_) {
    return { status: 'skipped', reason: 'error', events: [] };
  }
})(__BROWSEROS_OVERLAY_CMD__)
