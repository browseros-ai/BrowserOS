// biome-ignore-all lint: Injected through Runtime.evaluate; Rust fills the __BROWSEROS_POINTER_*__ placeholders.
// biome-ignore format: Keep the injected expression byte-oriented.
(async (cmd) => {
  try {
    if (document.visibilityState !== 'visible') return { status: 'skipped', reason: 'hidden' };
    if (window.top !== window) return { status: 'skipped', reason: 'not-top' };
    if (!document.documentElement || typeof document.createElement !== 'function' ||
        typeof requestAnimationFrame !== 'function' || typeof cancelAnimationFrame !== 'function') {
      return { status: 'skipped', reason: 'unsupported' };
    }
    const key = Symbol.for('browseros.agentPointer');
    let api = window[key];
    if (!api) {
      const loadCursor = () => {
        __BROWSEROS_POINTER_CURSOR__
        return typeof BrowserOSAgentCursor !== 'undefined' ? BrowserOSAgentCursor : globalThis.BrowserOSAgentCursor;
      };
      const Player = loadCursor()?.MotionPlayer;
      if (typeof Player !== 'function') return { status: 'skipped', reason: 'unsupported' };
      let host = null;
      let canvas = null;
      let player = null;
      let hideDepth = 0;
      let idleTimer = null;
      let idleMs = cmd.idleMs;
      let generation = 0;
      const remember = () => {
        if (player) api.pos = { x: player.pos.x, y: player.pos.y };
      };
      const dispose = () => {
        clearTimeout(idleTimer);
        try { remember(); } catch (_) {}
        try { if (player) player.destroy(); } catch (_) {}
        try { if (host) host.remove(); } catch (_) {}
        player = canvas = host = null;
        hideDepth = 0;
      };
      const armIdle = () => {
        clearTimeout(idleTimer);
        idleTimer = setTimeout(dispose, idleMs);
      };
      const ensurePlayer = (target) => {
        if (host && !host.isConnected) dispose();
        if (player) return true;
        host = document.createElement('browseros-agent-pointer');
        host.setAttribute('data-browseros-agent-pointer', '');
        host.setAttribute('aria-hidden', 'true');
        host.setAttribute('inert', '');
        host.inert = true;
        for (const [name, value] of Object.entries({ all: 'initial', position: 'fixed', inset: '0',
          'pointer-events': 'none', 'z-index': '2147483647', display: 'block', contain: 'strict' })) {
          host.style.setProperty(name, value, 'important');
        }
        if (typeof host.attachShadow !== 'function') { dispose(); return false; }
        const shadow = host.attachShadow({ mode: 'closed' });
        const printRule = '@media print { :host { display:none !important } }';
        if (typeof CSSStyleSheet === 'function' && 'adoptedStyleSheets' in shadow) {
          // CSSOM also works on pages whose CSP forbids inline style elements.
          const sheet = new CSSStyleSheet();
          sheet.replaceSync(printRule);
          shadow.adoptedStyleSheets = [sheet];
        } else {
          const style = document.createElement('style');
          style.textContent = printRule;
          shadow.appendChild(style);
        }
        canvas = document.createElement('canvas');
        for (const [name, value] of Object.entries({ width: '100%', height: '100%', display: 'block',
          position: 'absolute', inset: '0', 'pointer-events': 'none' })) {
          canvas.style.setProperty(name, value, 'important');
        }
        if (!canvas.getContext('2d')) { dispose(); return false; }
        shadow.appendChild(canvas);
        document.documentElement.appendChild(host);
        player = new Player(canvas, { float: false });
        player.place(api.pos || { x: target.x - 48, y: target.y - 32 });
        return true;
      };
      api = {
        pos: null,
        click(x, y) {
          try {
            if (!player || !host?.isConnected || document.visibilityState !== 'visible') return false;
            if (player.pos.x !== x || player.pos.y !== y) player.place({ x, y });
            player.click();
            remember();
            armIdle();
            return true;
          } catch (_) { return false; }
        },
        hide() {
          try {
            if (!canvas || !host?.isConnected) return false;
            canvas.style.setProperty('visibility', 'hidden', 'important');
            hideDepth += 1;
            return true;
          } catch (_) { return false; }
        },
        show() {
          try {
            if (!canvas || !host?.isConnected) return false;
            hideDepth = Math.max(0, hideDepth - 1);
            canvas.style.setProperty('visibility', hideDepth ? 'hidden' : 'visible', 'important');
            return true;
          } catch (_) { return false; }
        },
        async move(command) {
          let deadline;
          const current = ++generation;
          try {
            clearTimeout(idleTimer);
            idleMs = command.idleMs;
            const target = { x: command.x, y: command.y };
            if (!ensurePlayer(target)) return { status: 'skipped', reason: 'unsupported' };
            const arrival = player.moveTo(target, {
              params: { style: 'classic', timing: 'fixed', glideDurationMs: command.travelMs },
              reducedMotion: typeof window.matchMedia === 'function' && window.matchMedia('(prefers-reduced-motion: reduce)').matches,
            });
            const result = await Promise.race([
              Promise.resolve(arrival).then(() => ({ status: 'arrived' })),
              new Promise((resolve) => { deadline = setTimeout(() => resolve({ status: 'timeout' }), command.timeoutMs); }),
            ]);
            if (current === generation && player) {
              if (result.status === 'arrived') player.place(target);
              remember();
            }
            return result;
          } catch (_) {
            if (current === generation) dispose();
            return { status: 'skipped', reason: 'error' };
          } finally {
            clearTimeout(deadline);
            if (current === generation && player) armIdle();
          }
        },
      };
      Object.defineProperty(window, key, { value: api, enumerable: false });
    }
    return await api.move(cmd);
  } catch (_) {
    return { status: 'skipped', reason: 'error' };
  }
})(__BROWSEROS_POINTER_CMD__)
