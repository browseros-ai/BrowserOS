// Installed only in the slimviewer-input isolated world. Values stay in this closure.
(() => {
  'use strict';
  globalThis.__sv?.stop();
  const listeners = new AbortController();
  const textTypes = new Set(['text', 'search', 'url', 'tel', 'email', 'password', 'number']);
  const candidates = 'input,textarea,[contenteditable]';
  const random = crypto.getRandomValues(new Uint32Array(2));
  const doc = (random[0] & 0x1fffff) * 0x100000000 + random[1];
  let stopped = false, tap = null, snapshot = null, ranges = new WeakMap();
  let beforeInput = 0, input = 0, lastInputReport = -Infinity;
  let inputTimer = 0, boxTimer = 0, boxStarted = -Infinity, lastBoxes = '', forceBoxReport = false;
  const waits = new Set();

  function listen(target, event, fn) {
    target.addEventListener(event, fn, {capture: true, signal: listeners.signal});
  }
  function report(value) {
    if (!stopped && typeof globalThis.slimviewerAgent === 'function') {
      globalThis.slimviewerAgent(JSON.stringify(value));
    }
  }
  function activeElement() {
    let el = document.activeElement;
    while (el?.shadowRoot?.activeElement) el = el.shadowRoot.activeElement;
    return el;
  }
  function valueField(el) {
    return el?.tagName === 'TEXTAREA' || (el?.tagName === 'INPUT' && textTypes.has(el.type));
  }
  function editableOf(node) {
    let el = node?.nodeType === Node.ELEMENT_NODE ? node : node?.parentElement;
    if (!el?.isConnected) return null;
    if (valueField(el)) return el.disabled || el.readOnly || el.matches(':disabled') ? null : el;
    if (!el.isContentEditable) return null;
    while (el.parentElement?.isContentEditable) el = el.parentElement;
    return el;
  }
  function selectionFor(el) {
    const root = el.getRootNode();
    return root.getSelection ? root.getSelection() : document.getSelection();
  }
  function saveRange(el) {
    if (!el?.isContentEditable) return;
    const selection = selectionFor(el);
    if (selection?.rangeCount) {
      const range = selection.getRangeAt(0);
      if (el.contains(range.startContainer) && el.contains(range.endContainer)) {
        ranges.set(el, range.cloneRange());
      }
    }
  }
  function focus(el) {
    el.focus({preventScroll: true});
    if (editableOf(activeElement()) !== el) return false;
    const range = ranges.get(el);
    if (range && el.contains(range.startContainer) && el.contains(range.endContainer)) {
      const selection = selectionFor(el);
      selection.removeAllRanges();
      selection.addRange(range);
    }
    return true;
  }
  function focusReport() {
    const active = activeElement();
    const editable = document.hasFocus() && !!editableOf(active);
    report({k: 'focus', at: Date.now(), editable, inside: editable && active.tagName !== 'IFRAME'});
    scheduleBoxes();
  }
  listen(document, 'pointerdown', event => {
    if (!event.isTrusted) return;
    const el = editableOf(event.composedPath()[0]);
    tap = {at: Date.now(), el: el ? new WeakRef(el) : null};
    report({k: 'tap', at: tap.at, editable: !!el});
  });
  listen(document, 'focusin', focusReport);
  listen(document, 'focusout', event => {
    saveRange(editableOf(event.composedPath()[0]));
    focusReport();
    queueMicrotask(() => { if (!stopped) focusReport(); });
  });
  listen(document, 'selectionchange', () => saveRange(editableOf(activeElement())));
  function inputReport() {
    inputTimer = 0;
    lastInputReport = performance.now();
    report({k: 'input', at: Date.now()});
  }
  function flushInputReport() {
    if (!inputTimer) return;
    clearTimeout(inputTimer);
    // Transaction replies must not overtake evidence held by the normal 50 ms throttle.
    inputReport();
  }
  function countInput(event) {
    if (event.type === 'beforeinput') beforeInput++;
    else input++;
    const remaining = 50 - (performance.now() - lastInputReport);
    if (remaining <= 0) {
      clearTimeout(inputTimer);
      inputReport();
    } else if (!inputTimer) {
      inputTimer = setTimeout(inputReport, Math.ceil(remaining));
    }
  }
  listen(document, 'beforeinput', countInput);
  listen(document, 'input', countInput);

  function fieldGroup(target) {
    const fields = root => Array.from(root.querySelectorAll('input,textarea')).filter(valueField);
    if (target.form) return fields(target.form).slice(0, 32);
    let parent = target.parentElement;
    for (let depth = 0; parent && depth < 4; depth++, parent = parent.parentElement) {
      const group = fields(parent);
      if (group.length >= 2) return group.slice(0, 32);
    }
    return [];
  }
  function valueState(el) {
    return [el.value, el.selectionStart, el.selectionEnd, el.selectionDirection];
  }
  function discardSnapshot() {
    snapshot?.observer?.disconnect();
    snapshot = null;
  }
  function takeSnapshot(id, target) {
    discardSnapshot();
    const state = {
      id, target, root: document.documentElement, beforeInput, input,
      value: valueField(target) ? valueState(target) : null,
      group: fieldGroup(target).map(el => [el, el.value]),
      mutated: false, observer: null,
    };
    if (target.isContentEditable) {
      state.observer = new MutationObserver(records => { if (records.length) state.mutated = true; });
      state.observer.observe(target, {subtree: true, childList: true, characterData: true});
    }
    snapshot = state;
  }
  function evidence(state) {
    if (!state) return {changed: false, beforeInput: 0, input: 0, focusMoved: false, gone: true};
    if (state.observer?.takeRecords().length) state.mutated = true;
    const changed = state.mutated ||
      (state.value !== null && valueState(state.target).some((value, i) => value !== state.value[i])) ||
      state.group.some(([el, value]) => !el.isConnected || el.value !== value);
    return {
      changed,
      beforeInput: Math.min(0xffffffff, beforeInput - state.beforeInput),
      input: Math.min(0xffffffff, input - state.input),
      focusMoved: editableOf(activeElement()) !== state.target,
      gone: stopped || state !== snapshot || state.root !== document.documentElement || !state.target.isConnected,
    };
  }
  function wait(ms, animation = false) {
    return new Promise(resolve => {
      let timer, frame;
      const done = () => {
        clearTimeout(timer);
        if (frame) cancelAnimationFrame(frame);
        waits.delete(done);
        resolve();
      };
      waits.add(done);
      timer = setTimeout(done, ms);
      if (animation) frame = requestAnimationFrame(done);
    });
  }
  function pasteEvent(target, text) {
    const data = new DataTransfer();
    data.setData('text/plain', text);
    const event = new ClipboardEvent('paste', {
      clipboardData: data, bubbles: true, cancelable: true, composed: true,
    });
    target.dispatchEvent(event);
    return event.defaultPrevented;
  }
  function prepare({id, text, refocus, retry}) {
    flushInputReport();
    if (stopped || !document.hasFocus()) return {state: 'notfocused'};
    const active = activeElement();
    // A focused child frame owns the selection; its agent must handle the paste.
    if (active?.tagName === 'IFRAME') return {state: 'notfocused'};
    let target = editableOf(active);
    if (retry) {
      const previous = snapshot;
      if (!previous || previous.id !== id || !editableOf(previous.target) ||
          (target && target !== previous.target)) return {state: 'notfocused'};
      target = previous.target;
      if (!focus(target)) return {state: 'notfocused'};
    } else if (!target && refocus) {
      const age = tap ? Date.now() - tap.at : Infinity;
      target = age >= 0 && age <= 600000 ? editableOf(tap?.el?.deref()) : null;
      if (!target || !focus(target)) return {state: 'notfocused'};
    }
    if (!target) {
      discardSnapshot();
      return {state: 'nofield', handled: pasteEvent(active || document.body || document.documentElement, text)};
    }
    takeSnapshot(id, target);
    return {state: !retry && pasteEvent(target, text) ? 'handled' : 'ready'};
  }
  async function verify({id, windowMs}) {
    flushInputReport();
    const state = snapshot?.id === id ? snapshot : null;
    if (!state) return evidence(null);
    const duration = Math.max(0, Math.min(500, Number(windowMs) || 0));
    const deadline = performance.now() + duration;
    await Promise.resolve();
    if (duration <= 50) {
      if (!stopped && duration > 0) await wait(duration, true);
      return evidence(state);
    }
    for (;;) {
      const result = evidence(state);
      const remaining = deadline - performance.now();
      if (result.changed || result.focusMoved || result.gone || remaining <= 0) return result;
      await wait(Math.min(30, remaining));
    }
  }

  function viewport() {
    const v = window.visualViewport;
    return {
      vv: v ? [v.offsetLeft, v.offsetTop, v.width, v.height] : [0, 0, innerWidth, innerHeight],
      vv_scale: v?.scale || 1, inner: [innerWidth, innerHeight], dpr: devicePixelRatio,
      screen: [screen.width, screen.height],
      // The actor replaces this fallback with Page.getLayoutMetrics.cssVisualViewport.zoom.
      zoom: 1,
    };
  }
  function clip(rect, vv) {
    const x = Math.max(rect.left, vv[0]), y = Math.max(rect.top, vv[1]);
    return [x, y, Math.min(rect.right, vv[0] + vv[2]) - x, Math.min(rect.bottom, vv[1] + vv[3]) - y];
  }
  function hitAt(x, y) {
    let hit = document.elementFromPoint(x, y);
    for (;;) {
      const inner = hit?.shadowRoot?.elementFromPoint(x, y);
      if (!inner || inner === hit) return hit;
      hit = inner;
    }
  }
  function drawing(el, hit) {
    const parent = el.parentElement;
    return parent && parent !== el.form && parent !== document.body &&
      parent !== document.documentElement && parent.contains(hit);
  }
  function collectBoxes(reason) {
    boxTimer = 0;
    if (stopped || !document.body) return;
    boxStarted = performance.now(); // start of the last scan, for the throttle
    const vp = viewport(), boxes = [];
    let focused = null, scanned = 0;
    const active = document.hasFocus() ? editableOf(activeElement()) : null;
    const seen = new Set();
    if (document.visibilityState !== 'hidden') {
      const roots = [document];
      // Include open roots containing the active/tapped field without walking the entire DOM.
      for (const el of [active, tap?.el?.deref()]) {
        const root = el?.getRootNode();
        if (root && root !== document && !roots.includes(root)) roots.push(root);
      }
      outer: for (const root of roots) for (const candidate of root.querySelectorAll(candidates)) {
        if (boxes.length >= 128 || ++scanned > 512) break outer;
        const el = editableOf(candidate);
        if (!el || seen.has(el)) continue;
        seen.add(el);
        if (!el.getClientRects().length || !el.checkVisibility({visibilityProperty: true})) continue;
        const rect = el.getBoundingClientRect();
        const empty = rect.width === 0 || rect.height === 0;
        let box = clip(rect, vp.vv);
        const hit = empty ? hitAt(rect.left, rect.top) : box[2] > 0 && box[3] > 0 ?
          hitAt(box[0] + box[2] / 2, box[1] + box[3] / 2) : null;
        if (!hit) continue;
        if (hit !== el && !el.contains(hit)) {
          if (!drawing(el, hit)) continue;
          if (empty) box = clip((hit.contains(el) ? hit : el.parentElement).getBoundingClientRect(), vp.vv);
        }
        if (box[2] <= 0 || box[3] <= 0) continue;
        if (el === active) focused = boxes.length;
        boxes.push(box);
      }
    }
    const payload = JSON.stringify({k: 'boxes', doc, vp, boxes, focused});
    if (payload !== lastBoxes || forceBoxReport) {
      lastBoxes = payload;
      if (typeof globalThis.slimviewerAgent === 'function') {
        globalThis.slimviewerAgent(payload);
        forceBoxReport = false;
      }
    }
  }
  // Throttle layout changes, retaining the last change of a burst. Completed scrolls
  // refresh at a 40 ms minimum interval so their final positions reach the phone promptly.
  const BOX_INTERVAL = 120;
  const SCROLL_INTERVAL = 40;
  function scheduleBoxes(event) {
    if (stopped) return;
    if (event?.type === 'scrollend') {
      forceBoxReport = true;
      const remaining = SCROLL_INTERVAL - (performance.now() - boxStarted);
      if (remaining <= 0) {
        clearTimeout(boxTimer);
        collectBoxes();
      } else {
        clearTimeout(boxTimer);
        boxTimer = setTimeout(collectBoxes, remaining);
      }
      return;
    }
    if (boxTimer) return;
    boxTimer = setTimeout(collectBoxes, Math.max(0, BOX_INTERVAL - (performance.now() - boxStarted)));
  }
  for (const event of ['scroll', 'scrollend', 'load', 'transitionend', 'animationend', 'visibilitychange']) {
    listen(document, event, scheduleBoxes);
  }
  listen(window, 'resize', scheduleBoxes);
  listen(window, 'focus', focusReport);
  listen(window, 'blur', focusReport);
  if (window.visualViewport) {
    listen(visualViewport, 'resize', scheduleBoxes);
    listen(visualViewport, 'scroll', scheduleBoxes);
    listen(visualViewport, 'scrollend', scheduleBoxes);
  }
  const boxesObserver = new MutationObserver(scheduleBoxes);
  boxesObserver.observe(document, {childList: true, subtree: true});
  const keepalive = setInterval(() => {
    if (document.visibilityState !== 'hidden') scheduleBoxes();
  }, 1000);
  function frameRect(el) {
    if (!el?.isConnected || el.tagName !== 'IFRAME' ||
        !el.getClientRects().length || !el.checkVisibility({visibilityProperty: true})) return [0, 0, 0, 0];
    const rect = el.getBoundingClientRect(), style = getComputedStyle(el);
    const sx = el.offsetWidth ? rect.width / el.offsetWidth : 1;
    const sy = el.offsetHeight ? rect.height / el.offsetHeight : 1;
    const pl = parseFloat(style.paddingLeft) || 0, pr = parseFloat(style.paddingRight) || 0;
    const pt = parseFloat(style.paddingTop) || 0, pb = parseFloat(style.paddingBottom) || 0;
    const vv = viewport().vv;
    return [rect.left + (el.clientLeft + pl) * sx - vv[0],
      rect.top + (el.clientTop + pt) * sy - vv[1],
      Math.max(0, el.clientWidth - pl - pr) * sx, Math.max(0, el.clientHeight - pt - pb) * sy];
  }
  function stop() {
    stopped = true;
    listeners.abort();
    boxesObserver.disconnect();
    clearTimeout(inputTimer);
    clearTimeout(boxTimer);
    clearInterval(keepalive);
    for (const done of waits) done();
    discardSnapshot();
    tap = null;
    ranges = new WeakMap();
  }
  globalThis.__sv = {prepare, verify, frameRect, stop};
  focusReport();
})();
