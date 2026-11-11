/*
 * Playwright v1.64.1 (cfe490d941f992655948eee8fd4fd94e1c4234bf), packages/isomorphic helpers and packages/protocol value
 * serializers, for Neo's QuickJS prelude.
 * Playwright: Copyright (c) Microsoft Corporation. Licensed under the Apache License, Version 2.0.
 * mime: Copyright (c) 2023 Robert Kieffer. Licensed under the MIT License.
 * Built from crates/neo-sdk/neo-playwright/third_party/playwright and
 * crates/neo-sdk/neo-playwright/third_party/mime, with crates/neo-sdk/neo-playwright-js/src/prelude,
 * by crates/neo-sdk/neo-playwright-js/scripts/build.ts.
 * Changed by Neo: isomorphic/utilityScriptSerializers.ts and protocol/src/serializers.ts import
 * Neo's Buffer view, which QuickJS lacks; URL, atob and btoa use the realm globals.
 * Every other upstream module, and mime, is built unmodified.
 * Do not edit. Notices: crates/neo-sdk/neo-playwright/THIRD_PARTY_NOTICES.md.
 */
(() => {
  // crates/neo-sdk/neo-playwright-js/src/prelude/ops.generated.ts
  var OPS = {
    "locator.click": {
      effect: "write",
      target: "target",
      lifetime: "operation"
    },
    "locator.dblclick": {
      effect: "write",
      target: "target",
      lifetime: "operation"
    },
    "locator.hover": {
      effect: "write",
      target: "target",
      lifetime: "operation"
    },
    "locator.check": {
      effect: "write",
      target: "target",
      lifetime: "operation"
    },
    "locator.uncheck": {
      effect: "write",
      target: "target",
      lifetime: "operation"
    },
    "locator.dragTo": {
      effect: "write",
      target: "target",
      lifetime: "operation"
    },
    "locator.drop": {
      effect: "write",
      target: "target",
      lifetime: "operation"
    },
    "locator.fill": {
      effect: "write",
      target: "target",
      lifetime: "operation"
    },
    "locator.press": {
      effect: "write",
      target: "target",
      lifetime: "operation"
    },
    "locator.pressSequentially": {
      effect: "write",
      target: "target",
      lifetime: "operation"
    },
    "locator.selectOption": {
      effect: "write",
      target: "target",
      lifetime: "operation"
    },
    "locator.selectText": {
      effect: "write",
      target: "target",
      lifetime: "operation"
    },
    "locator.focus": {
      effect: "write",
      target: "target",
      lifetime: "operation"
    },
    "locator.blur": {
      effect: "write",
      target: "target",
      lifetime: "operation"
    },
    "locator.setInputFiles": {
      effect: "write",
      target: "target",
      lifetime: "operation"
    },
    "locator.dispatchEvent": {
      effect: "write",
      target: "target",
      lifetime: "operation"
    },
    "locator.scrollIntoViewIfNeeded": {
      effect: "read",
      target: "target",
      lifetime: "operation"
    },
    "locator.textContent": {
      effect: "read",
      target: "target",
      lifetime: "operation"
    },
    "locator.innerText": {
      effect: "read",
      target: "target",
      lifetime: "operation"
    },
    "locator.innerHTML": {
      effect: "read",
      target: "target",
      lifetime: "operation"
    },
    "locator.getAttribute": {
      effect: "read",
      target: "target",
      lifetime: "operation"
    },
    "locator.inputValue": {
      effect: "read",
      target: "target",
      lifetime: "operation"
    },
    "locator.isEnabled": {
      effect: "read",
      target: "target",
      lifetime: "operation"
    },
    "locator.isDisabled": {
      effect: "read",
      target: "target",
      lifetime: "operation"
    },
    "locator.isEditable": {
      effect: "read",
      target: "target",
      lifetime: "operation"
    },
    "locator.isChecked": {
      effect: "read",
      target: "target",
      lifetime: "operation"
    },
    "locator.isVisible": {
      effect: "read",
      target: "target",
      lifetime: "operation"
    },
    "locator.isHidden": {
      effect: "read",
      target: "target",
      lifetime: "operation"
    },
    "locator.count": {
      effect: "read",
      target: "target",
      lifetime: "operation"
    },
    "locator.allTextContents": {
      effect: "read",
      target: "target",
      lifetime: "operation"
    },
    "locator.allInnerTexts": {
      effect: "read",
      target: "target",
      lifetime: "operation"
    },
    "locator.boundingBox": {
      effect: "read",
      target: "target",
      lifetime: "operation"
    },
    "locator.waitFor": {
      effect: "read",
      target: "target",
      lifetime: "operation"
    },
    "locator.elementHandle": {
      effect: "read",
      target: "target",
      lifetime: "operation"
    },
    "locator.elementHandles": {
      effect: "read",
      target: "target",
      lifetime: "operation"
    },
    "locator.normalize": {
      effect: "read",
      target: "target",
      lifetime: "operation"
    },
    "page.ariaSnapshot": {
      effect: "read",
      target: "page",
      lifetime: "operation"
    },
    "locator.ariaSnapshot": {
      effect: "read",
      target: "target",
      lifetime: "operation"
    },
    "expect.locator": {
      effect: "read",
      target: "target",
      lifetime: "operation"
    },
    "expect.page": {
      effect: "read",
      target: "page",
      lifetime: "operation"
    },
    "keyboard.down": {
      effect: "write",
      target: "page",
      lifetime: "operation"
    },
    "keyboard.up": {
      effect: "write",
      target: "page",
      lifetime: "operation"
    },
    "keyboard.press": {
      effect: "write",
      target: "page",
      lifetime: "operation"
    },
    "keyboard.type": {
      effect: "write",
      target: "page",
      lifetime: "operation"
    },
    "keyboard.insertText": {
      effect: "write",
      target: "page",
      lifetime: "operation"
    },
    "mouse.move": {
      effect: "write",
      target: "page",
      lifetime: "operation"
    },
    "mouse.down": {
      effect: "write",
      target: "page",
      lifetime: "operation"
    },
    "mouse.up": {
      effect: "write",
      target: "page",
      lifetime: "operation"
    },
    "mouse.click": {
      effect: "write",
      target: "page",
      lifetime: "operation"
    },
    "mouse.wheel": {
      effect: "write",
      target: "page",
      lifetime: "operation"
    },
    "page.goto": {
      effect: "write",
      target: "page",
      lifetime: "operation"
    },
    "page.reload": {
      effect: "write",
      target: "page",
      lifetime: "operation"
    },
    "page.goBack": {
      effect: "write",
      target: "page",
      lifetime: "operation"
    },
    "page.goForward": {
      effect: "write",
      target: "page",
      lifetime: "operation"
    },
    "page.waitForNavigation": {
      effect: "read",
      target: "page",
      lifetime: "operation"
    },
    "page.waitForURL": {
      effect: "read",
      target: "page",
      lifetime: "operation"
    },
    "page.waitForLoadState": {
      effect: "read",
      target: "page",
      lifetime: "operation"
    },
    "page.title": {
      effect: "read",
      target: "page",
      lifetime: "operation"
    },
    "page.content": {
      effect: "read",
      target: "page",
      lifetime: "operation"
    },
    "page.setContent": {
      effect: "write",
      target: "page",
      lifetime: "operation"
    },
    "page.addScriptTag": {
      effect: "write",
      target: "page",
      lifetime: "operation"
    },
    "page.addStyleTag": {
      effect: "write",
      target: "page",
      lifetime: "operation"
    },
    "page.emulateMedia": {
      effect: "write",
      target: "page",
      lifetime: "operation"
    },
    "page.close": {
      effect: "write",
      target: "page",
      lifetime: "operation"
    },
    "page.opener": {
      effect: "read",
      target: "page",
      lifetime: "operation"
    },
    "page.frames": {
      effect: "read",
      target: "page",
      lifetime: "operation"
    },
    "page.evaluate": {
      effect: "write",
      target: "page",
      lifetime: "operation"
    },
    "locator.evaluate": {
      effect: "write",
      target: "target",
      lifetime: "operation"
    },
    "locator.evaluateAll": {
      effect: "write",
      target: "target",
      lifetime: "operation"
    },
    "page.waitForFunction": {
      effect: "write",
      target: "page",
      lifetime: "operation"
    },
    "locator.waitForFunction": {
      effect: "write",
      target: "target",
      lifetime: "operation"
    },
    "page.screenshot": {
      effect: "read",
      target: "page",
      lifetime: "operation"
    },
    "page.saveScreenshot": {
      effect: "write",
      target: null,
      lifetime: "operation"
    },
    "page.pdf": {
      effect: "read",
      target: "page",
      lifetime: "operation"
    },
    "page.setViewportSize": {
      effect: "write",
      target: "page",
      lifetime: "operation"
    },
    "page.addInitScript": {
      effect: "write",
      target: "page",
      lifetime: "operation"
    },
    "page.exposeBinding": {
      effect: "write",
      target: "page",
      lifetime: "operation"
    },
    "bindingCall.resolve": {
      effect: "write",
      target: "page",
      lifetime: "operation"
    },
    "context.pages": {
      effect: "read",
      target: null,
      lifetime: "operation"
    },
    "context.newPage": {
      effect: "write",
      target: null,
      lifetime: "operation"
    },
    "context.addInitScript": {
      effect: "write",
      target: null,
      lifetime: "operation"
    },
    "context.exposeBinding": {
      effect: "write",
      target: null,
      lifetime: "operation"
    },
    "disposable.dispose": {
      effect: "write",
      target: null,
      lifetime: "operation"
    },
    "events.listen": {
      effect: "read",
      target: null,
      lifetime: "operation"
    },
    "events.next": {
      effect: "read",
      target: null,
      lifetime: "background"
    },
    "events.wait": {
      effect: "read",
      target: null,
      lifetime: "operation"
    },
    "events.interceptFileChooser": {
      effect: "read",
      target: null,
      lifetime: "operation"
    },
    "dialog.accept": {
      effect: "write",
      target: "page",
      lifetime: "operation"
    },
    "dialog.dismiss": {
      effect: "write",
      target: "page",
      lifetime: "operation"
    },
    "download.path": {
      effect: "read",
      target: null,
      lifetime: "operation"
    },
    "download.saveAs": {
      effect: "write",
      target: null,
      lifetime: "operation"
    }
  };

  // crates/neo-sdk/neo-playwright-js/src/prelude/chooser-setup.ts
  var pending = new Set;
  function trackChooserSetup(setup) {
    pending.add(setup);
    const remove = () => {
      pending.delete(setup);
    };
    setup.then(remove, remove);
  }
  function readyChoosers(op) {
    if (OPS[op].effect === "read")
      return null;
    return pending.size ? Promise.all([...pending]) : null;
  }

  // crates/neo-sdk/neo-playwright-js/src/prelude/host.ts
  function localTimeout(message) {
    const neo = Reflect.get(globalThis, "__neo");
    if (typeof neo !== "object" || neo === null)
      return null;
    const make = Reflect.get(neo, "timeoutError");
    return typeof make === "function" ? Reflect.apply(make, neo, [message]) : null;
  }
  function host() {
    const neo = Reflect.get(globalThis, "__neo");
    if (typeof neo !== "object" || neo === null)
      throw new Error("The Playwright prelude needs claw-sdk's bootstrap (globalThis.__neo), which was not installed");
    return neo;
  }
  function refuser() {
    const neo = Reflect.get(globalThis, "__neo");
    if (typeof neo !== "object" || neo === null)
      return null;
    const refuse = Reflect.get(neo, "refuse");
    return typeof refuse === "function" ? (api, code, reason) => Reflect.apply(refuse, neo, [api, code, reason]) : null;
  }

  // crates/neo-sdk/neo-playwright-js/src/prelude/errors.ts
  var FIELDS = [
    "code",
    "owner",
    "delivery",
    "retryable",
    "operationId",
    "timeout"
  ];

  class PlaywrightError extends Error {
  }

  class TimeoutError extends PlaywrightError {
    constructor(message) {
      super(message);
      this.name = "TimeoutError";
    }
  }

  class AbortError extends PlaywrightError {
    constructor(message, cause) {
      super(message, { cause });
      this.name = "AbortError";
    }
  }
  function withFields(error, fields) {
    for (const field of FIELDS)
      if (fields[field] !== undefined)
        Reflect.set(error, field, fields[field]);
    return error;
  }
  function readFields(thrown) {
    const fields = {};
    if (typeof thrown !== "object" || thrown === null)
      return fields;
    for (const field of FIELDS) {
      const value = Reflect.get(thrown, field);
      if (value !== undefined)
        Reflect.set(fields, field, value);
    }
    return fields;
  }
  function apiMessage(api, op, message) {
    if (message.startsWith(`${api}: `))
      return message;
    if (op !== api && message.startsWith(`${op}: `))
      return `${api}: ${message.slice(op.length + 2)}`;
    return `${api}: ${message}`;
  }
  function shapeBridgeError(op, api, thrown) {
    const read = readFields(thrown);
    const timedOut = read.timeout === true || thrown instanceof Error && thrown.name === "TimeoutError";
    const assertion = op === "expect.locator" || op === "expect.page";
    const named = assertion ? "Error" : read.code === "aborted" ? "AbortError" : timedOut ? "TimeoutError" : "Error";
    if (thrown instanceof Error) {
      if (!assertion)
        thrown.message = apiMessage(api, op, thrown.message);
      thrown.name = named;
      return withFields(thrown, { timeout: timedOut });
    }
    const message = typeof thrown === "object" && thrown !== null ? String(Reflect.get(thrown, "message") ?? thrown) : String(thrown);
    const shaped = assertion ? message : apiMessage(api, op, message);
    return withFields(named === "TimeoutError" ? new TimeoutError(shaped) : named === "AbortError" ? new AbortError(shaped, undefined) : new PlaywrightError(shaped), { ...read, timeout: timedOut });
  }
  function withAbortCause(error, signal) {
    if (signal?.aborted && error.name === "AbortError")
      Object.defineProperty(error, "cause", {
        value: signal.reason,
        writable: true,
        configurable: true
      });
    return error;
  }
  function abortedError(api, signal) {
    return withFields(new AbortError(`${api}: The operation was aborted`, signal.reason), {
      code: "aborted",
      owner: "script",
      delivery: "not_sent",
      retryable: true,
      timeout: false
    });
  }
  var REFUSED_ARGUMENTS = {
    code: "invalid_input",
    owner: "script",
    delivery: "not_sent",
    retryable: false,
    timeout: false
  };
  var UNSUPPORTED = {
    code: "unsupported",
    owner: "neo",
    delivery: "not_sent",
    retryable: false,
    timeout: false
  };
  function refusal(api, code, message, fallback, name = "Error") {
    const refuse = refuser();
    if (refuse === null)
      return withFields(name === "TypeError" ? new TypeError(message) : new PlaywrightError(message), fallback);
    const error = refuse(api, code, message);
    error.name = name;
    return withFields(error, { timeout: false });
  }
  function clientError(api, message) {
    return refusal(api, "invalid_input", `${api}: ${message}`, REFUSED_ARGUMENTS);
  }
  function unprefixedClientError(api, message, kind = "Error") {
    return refusal(api, "invalid_input", message, REFUSED_ARGUMENTS, kind);
  }
  function plainError(api, message) {
    return refusal(api, "invalid_input", message, REFUSED_ARGUMENTS);
  }
  function internalError(api, message) {
    return new PlaywrightError(`${api}: ${message}`);
  }
  function waitTimeoutError(api, message) {
    const text = `${api}: ${message}`;
    return withFields(localTimeout(text) ?? new TimeoutError(text), {
      code: "wait_timeout",
      owner: "website",
      delivery: "not_sent",
      retryable: true,
      timeout: true
    });
  }
  function unsupportedError(api, hint) {
    return refusal(api, "method_unavailable", `${api}: ${hint}`, UNSUPPORTED);
  }
  function assertionError(message) {
    return new PlaywrightError(message);
  }
  function answerError(api, thrown) {
    const message = thrown instanceof Error ? thrown.message : String(thrown);
    return new PlaywrightError(`${api}: ${message}`);
  }
  function prefixed(api, error) {
    if (error instanceof Error && !error.message.startsWith(`${api}: `))
      error.message = `${api}: ${error.message}`;
    return error;
  }

  // crates/neo-sdk/neo-playwright-js/src/prelude/bridge.ts
  async function invoke(op, call, args, signal) {
    try {
      const ready = readyChoosers(op);
      if (ready)
        await ready;
      countPumps();
      const value = await host().invoke(op, { call, ...args }, signal === undefined ? undefined : { signal });
      const held = pumpsCaughtUp(op);
      if (held)
        await held;
      return value;
    } catch (thrown) {
      const held = pumpsCaughtUp(op);
      if (held)
        await held;
      throw withAbortCause(shapeBridgeError(op, call.api, thrown), signal);
    }
  }
  async function invokeObserved(op, call, args, onResult, signal) {
    try {
      const ready = readyChoosers(op);
      if (ready)
        await ready;
      countPumps();
      const reply = await host().invoke(op, { call, ...args }, {
        observed: true,
        signal,
        onResult: onResult && ((value, observed) => onResult(value, readObservation(observed)))
      });
      const held = pumpsCaughtUp(op);
      if (held)
        await held;
      return {
        value: field(reply, "value"),
        observed: readObservation(field(reply, "observed"))
      };
    } catch (thrown) {
      const held = pumpsCaughtUp(op);
      if (held)
        await held;
      throw withAbortCause(shapeBridgeError(op, call.api, thrown), signal);
    }
  }
  var PUMP_HOLD_MS = 1000;
  var pumpsNeeded = 0;
  var pumpsDispatched = 0;
  var countingPumps = false;
  var pumpHolds = [];
  function countPumps() {
    if (countingPumps)
      return;
    countingPumps = true;
    host().onReply((envelope) => {
      const pumped = field(envelope, "pumped");
      if (typeof pumped === "number" && pumped > pumpsNeeded)
        pumpsNeeded = pumped;
    });
  }
  function pumpDispatched() {
    pumpsDispatched = Math.max(pumpsDispatched, pumpsNeeded);
    for (const hold of pumpHolds.filter((hold) => hold.needed <= pumpsDispatched))
      hold.release();
  }
  function pumpsCaughtUp(op) {
    const needed = pumpsNeeded;
    if (op.startsWith("events.") || pumpsDispatched >= needed)
      return;
    return new Promise((resolve) => {
      let cancel = () => {};
      const release = () => {
        cancel();
        const index = pumpHolds.indexOf(hold);
        if (index >= 0)
          pumpHolds.splice(index, 1);
        resolve();
      };
      const hold = { needed, release };
      pumpHolds.push(hold);
      cancel = startTimer(PUMP_HOLD_MS, release);
    });
  }
  function registerBrand(brand, fromJson) {
    host().brands.register(brand, fromJson);
  }
  function onReply(listener) {
    host().onReply((envelope) => {
      const observed = readObservation(field(envelope, "observed"));
      if (observed && field(envelope, "err") !== undefined)
        observed.closes = [];
      listener({
        observed,
        pages: readPageFacts(field(envelope, "pages")),
        defaultPage: readTab(field(envelope, "defaultPage"))
      });
    });
  }
  function provide(name, build) {
    host().provide(name, (init) => {
      const page = readTab(field(init, "page"));
      return build({
        page: page ? { $neo: "page", ...page } : null,
        pages: readPageFacts(field(init, "pages")),
        observed: readObservation(field(init, "observed"))
      });
    });
  }
  function awaitsProviders() {
    return refuser() !== null;
  }
  function field(value, name) {
    return typeof value === "object" && value !== null ? Reflect.get(value, name) : undefined;
  }
  function readTab(value) {
    const tab = field(value, "tab");
    return typeof tab === "number" ? { tab } : null;
  }
  function readObservation(value) {
    const seq = field(value, "seq");
    const events = field(value, "events");
    if (typeof seq !== "number" || !Number.isFinite(seq) || typeof events !== "number" || !Number.isFinite(events))
      return null;
    const result = { seq, events };
    const payloadCurrent = field(value, "payloadCurrent");
    if (typeof payloadCurrent === "boolean")
      result.payloadCurrent = payloadCurrent;
    const closes = field(value, "closes");
    if (Array.isArray(closes)) {
      result.closes = [];
      for (const close of closes) {
        const tab = field(close, "tab");
        const at = field(close, "seq");
        if (typeof tab === "number" && Number.isFinite(tab) && typeof at === "number" && Number.isFinite(at))
          result.closes.push({ tab, seq: at });
      }
    }
    return result;
  }
  function readPageFacts(pages) {
    if (!Array.isArray(pages))
      return [];
    const facts = [];
    for (const page of pages) {
      if (typeof page !== "object" || page === null)
        continue;
      const tab = Reflect.get(page, "tab");
      if (typeof tab !== "number")
        continue;
      const fact = { tab };
      const url = Reflect.get(page, "url");
      const title = Reflect.get(page, "title");
      const closed = Reflect.get(page, "closed");
      const viewport = Reflect.get(page, "viewportSize");
      if (typeof url === "string")
        fact.url = url;
      if (typeof title === "string")
        fact.title = title;
      if (typeof closed === "boolean")
        fact.closed = closed;
      if (viewport === null)
        fact.viewportSize = null;
      else if (typeof viewport === "object" && viewport !== null) {
        const width = Reflect.get(viewport, "width");
        const height = Reflect.get(viewport, "height");
        if (typeof width === "number" && typeof height === "number")
          fact.viewportSize = { width, height };
      }
      facts.push(fact);
    }
    return facts;
  }
  function clock() {
    const sleep = Reflect.get(globalThis, "__neo_sleep");
    return typeof sleep === "function" ? (ms) => Promise.resolve(Reflect.apply(sleep, globalThis, [ms])) : null;
  }
  function sleep(api, ms) {
    const after = clock();
    if (!after)
      return Promise.reject(unsupportedError(api, "this runtime has no timers; wait on the page instead"));
    return after(ms).then(() => {
      return;
    });
  }
  function startTimer(ms, fire) {
    const after = clock();
    if (!after)
      return () => {};
    let live = true;
    after(ms).then(() => {
      if (live)
        fire();
    });
    return () => {
      live = false;
    };
  }

  // crates/neo-sdk/neo-playwright-js/src/prelude/emitter.ts
  class EventEmitter {
    #registered = new Map;
    #maxListeners = 10;
    _checkEvent(_event) {}
    _listenersChanged(_event, _listening) {}
    addListener(event, listener) {
      return this.#add(event, listener, false, false);
    }
    on(event, listener) {
      return this.#add(event, listener, false, false);
    }
    once(event, listener) {
      return this.#add(event, listener, true, false);
    }
    prependListener(event, listener) {
      return this.#add(event, listener, false, true);
    }
    prependOnceListener(event, listener) {
      return this.#add(event, listener, true, true);
    }
    removeListener(event, listener) {
      const list = this.#registered.get(event);
      if (!list)
        return this;
      const index = list.findLastIndex((entry) => entry.listener === listener);
      if (index === -1)
        return this;
      list.splice(index, 1);
      if (list.length === 0) {
        this.#registered.delete(event);
        this._listenersChanged(event, false);
      }
      return this;
    }
    off(event, listener) {
      return this.removeListener(event, listener);
    }
    removeAllListeners(event) {
      const events = event === undefined ? [...this.#registered.keys()] : [event];
      for (const name of events) {
        if (!this.#registered.delete(name))
          continue;
        this._listenersChanged(name, false);
      }
      return this;
    }
    emit(event, ...args) {
      const list = this.#registered.get(event);
      if (!list || list.length === 0)
        return false;
      for (const entry of [...list]) {
        if (entry.once)
          this.removeListener(event, entry.listener);
        try {
          Reflect.apply(entry.listener, this, args);
        } catch (error) {
          Promise.reject(error);
        }
      }
      return true;
    }
    listeners(event) {
      return (this.#registered.get(event) ?? []).map((entry) => entry.listener);
    }
    rawListeners(event) {
      return this.listeners(event);
    }
    listenerCount(event) {
      return this.#registered.get(event)?.length ?? 0;
    }
    eventNames() {
      return [...this.#registered.keys()];
    }
    setMaxListeners(count) {
      this.#maxListeners = count;
      return this;
    }
    getMaxListeners() {
      return this.#maxListeners;
    }
    #add(event, listener, once, prepend) {
      this._checkEvent(event);
      const list = this.#registered.get(event) ?? [];
      const first = list.length === 0;
      if (prepend)
        list.unshift({ listener, once });
      else
        list.push({ listener, once });
      this.#registered.set(event, list);
      if (first)
        this._listenersChanged(event, true);
      return this;
    }
  }

  // crates/neo-sdk/neo-playwright-js/src/prelude/options.ts
  function pick(source, keys) {
    const picked = {};
    if (!source)
      return picked;
    for (const key of keys)
      if (source[key] !== undefined)
        picked[key] = source[key];
    return picked;
  }
  var POINTER_KEYS = [
    "button",
    "clickCount",
    "delay",
    "force",
    "modifiers",
    "noWaitAfter",
    "position",
    "scroll",
    "steps",
    "trial"
  ];
  var DBLCLICK_KEYS = POINTER_KEYS.filter((key) => key !== "clickCount");
  var HOVER_KEYS = [
    "force",
    "modifiers",
    "noWaitAfter",
    "position",
    "scroll",
    "trial"
  ];
  var CHECK_KEYS = [
    "force",
    "noWaitAfter",
    "position",
    "scroll",
    "trial"
  ];
  var DRAG_KEYS = [
    "force",
    "noWaitAfter",
    "scroll",
    "sourcePosition",
    "targetPosition",
    "steps",
    "trial"
  ];
  var FILL_KEYS = ["force", "noWaitAfter"];
  var KEY_KEYS = ["delay", "noWaitAfter"];
  var SCREENSHOT_KEYS = [
    "type",
    "quality",
    "omitBackground",
    "fullPage",
    "clip",
    "animations",
    "caret",
    "style",
    "scale",
    "maskColor"
  ];
  var handles = new WeakSet;
  function markHandle(object) {
    handles.add(object);
  }
  var inspectCustom = Symbol.for("nodejs.util.inspect.custom");
  function isHandle(value) {
    return typeof value === "object" && value !== null && handles.has(value);
  }
  function selectOption(api, path, item) {
    if (typeof item !== "object" || item === null)
      throw clientError(api, `${path}: expected object, got ${item === null ? "null" : typeof item}`);
    const option = item;
    const picked = {};
    for (const key of ["valueOrLabel", "value", "label"]) {
      const given = option[key];
      if (given === undefined)
        continue;
      if (given instanceof String)
        picked[key] = given.valueOf();
      else if (typeof given === "string")
        picked[key] = given;
      else
        throw clientError(api, `${path}.${key}: expected string, got ${typeof given}`);
    }
    const index = option.index;
    if (index !== undefined) {
      const value = index instanceof Number ? index.valueOf() : typeof index === "number" ? index : null;
      if (value === null)
        throw clientError(api, `${path}.index: expected integer, got ${typeof index}`);
      if (!Number.isInteger(value))
        throw clientError(api, `${path}.index: expected integer, got float ${value}`);
      picked.index = value;
    }
    return picked;
  }
  function selectOptionValues(api, values) {
    if (values === null || values === undefined)
      return { options: [], elements: [] };
    const list = Array.isArray(values) ? values : [values];
    const options = [];
    const elements = [];
    for (const [index, item] of list.entries()) {
      if (item === null)
        throw clientError(api, `options[${index}]: expected object, got null`);
      if (typeof item === "string") {
        options.push({ valueOrLabel: item });
        continue;
      }
      const brand = typeof item === "object" ? Reflect.get(item, "_brand") : undefined;
      if (handles.has(Object(item)) && typeof brand === "object" && brand !== null) {
        elements.push(brand);
        continue;
      }
      options.push(selectOption(api, `options[${index}]`, item));
    }
    return { options, elements };
  }

  // crates/neo-sdk/neo-playwright-js/src/prelude/browser.ts
  class Browser extends EventEmitter {
    #context;
    constructor(context) {
      super();
      this.#context = context;
      context._browser = this;
      markHandle(this);
    }
    contexts() {
      return [this.#context];
    }
    version() {
      return "";
    }
    isConnected() {
      return true;
    }
    toJSON() {
      return { version: this.version() };
    }
  }

  // crates/neo-sdk/neo-playwright-js/src/prelude/encodings.ts
  var ENCODINGS = new Map([
    ["utf8", "utf8"],
    ["utf-8", "utf8"],
    ["hex", "hex"],
    ["base64", "base64"],
    ["base64url", "base64url"],
    ["latin1", "latin1"],
    ["binary", "latin1"],
    ["ascii", "ascii"],
    ["utf16le", "utf16le"],
    ["utf-16le", "utf16le"],
    ["ucs2", "utf16le"],
    ["ucs-2", "utf16le"]
  ]);
  var BASE64 = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
  var BASE64URL = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
  var BASE64_UNITS = Uint16Array.from(BASE64, (char) => char.charCodeAt(0));
  var BASE64URL_UNITS = Uint16Array.from(BASE64URL, (char) => char.charCodeAt(0));
  var SEXTETS = (() => {
    const table = new Int16Array(128).fill(-1);
    for (let index = 0;index < 64; index++) {
      table[BASE64.charCodeAt(index)] = index;
      table[BASE64URL.charCodeAt(index)] = index;
    }
    return table;
  })();
  var CHUNK = 8192;
  var HEX_DIGITS = "0123456789abcdef";
  function encodingNamed(name) {
    return ENCODINGS.get(name.toLowerCase());
  }

  class TextWriter {
    #units = new Uint16Array(CHUNK);
    #parts = [];
    #filled = 0;
    unit(code) {
      this.#units[this.#filled++] = code;
      if (this.#filled === CHUNK)
        this.#flush();
    }
    units(units) {
      this.#flush();
      if (units.length > 0)
        this.#parts.push(fromUnits(units));
    }
    text() {
      this.#flush();
      return this.#parts.join("");
    }
    #flush() {
      if (this.#filled === 0)
        return;
      this.#parts.push(fromUnits(this.#units.subarray(0, this.#filled)));
      this.#filled = 0;
    }
  }
  function fromUnits(units) {
    return String(Reflect.apply(String.fromCharCode, null, units));
  }
  function utf8Length(text) {
    let length = 0;
    for (let index = 0;index < text.length; index++) {
      const code = text.charCodeAt(index);
      if (code < 128)
        length += 1;
      else if (code < 2048)
        length += 2;
      else if (code >= 55296 && code <= 56319 && isLowSurrogate(text, index + 1)) {
        length += 4;
        index++;
      } else
        length += 3;
    }
    return length;
  }
  function isLowSurrogate(text, index) {
    const code = text.charCodeAt(index);
    return code >= 56320 && code <= 57343;
  }
  function encodeUtf8(text) {
    const out = new Uint8Array(utf8Length(text));
    let at = 0;
    for (let index = 0;index < text.length; index++) {
      let point = text.charCodeAt(index);
      if (point >= 55296 && point <= 56319) {
        if (isLowSurrogate(text, index + 1)) {
          point = 65536 + (point - 55296 << 10) + (text.charCodeAt(index + 1) - 56320);
          index++;
        } else
          point = 65533;
      } else if (point >= 56320 && point <= 57343)
        point = 65533;
      if (point < 128)
        out[at++] = point;
      else if (point < 2048) {
        out[at++] = 192 | point >> 6;
        out[at++] = 128 | point & 63;
      } else if (point < 65536) {
        out[at++] = 224 | point >> 12;
        out[at++] = 128 | point >> 6 & 63;
        out[at++] = 128 | point & 63;
      } else {
        out[at++] = 240 | point >> 18;
        out[at++] = 128 | point >> 12 & 63;
        out[at++] = 128 | point >> 6 & 63;
        out[at++] = 128 | point & 63;
      }
    }
    return out;
  }
  function leadOf(byte) {
    if (byte >= 194 && byte <= 223)
      return { needed: 1, point: byte & 31, lower: 128, upper: 191 };
    if (byte >= 224 && byte <= 239)
      return {
        needed: 2,
        point: byte & 15,
        lower: byte === 224 ? 160 : 128,
        upper: byte === 237 ? 159 : 191
      };
    if (byte >= 240 && byte <= 244)
      return {
        needed: 3,
        point: byte & 7,
        lower: byte === 240 ? 144 : 128,
        upper: byte === 244 ? 143 : 191
      };
    return null;
  }
  function sequenceAt(bytes, index) {
    const lead = leadOf(bytes[index] ?? 0);
    if (lead === null)
      return [65533, index + 1];
    let { point, lower, upper } = lead;
    let next = index + 1;
    for (let seen = 0;seen < lead.needed; seen++) {
      const byte = bytes[next];
      if (byte === undefined || byte < lower || byte > upper)
        return [65533, next];
      point = point << 6 | byte & 63;
      lower = 128;
      upper = 191;
      next++;
    }
    return [point, next];
  }
  function decodeUtf8(bytes) {
    const text = new TextWriter;
    let index = 0;
    while (index < bytes.length) {
      const byte = bytes[index] ?? 0;
      if (byte < 128) {
        text.unit(byte);
        index++;
        continue;
      }
      const [point, next] = sequenceAt(bytes, index);
      index = next;
      if (point < 65536)
        text.unit(point);
      else {
        text.unit(55296 + (point - 65536 >> 10));
        text.unit(56320 + (point - 65536 & 1023));
      }
    }
    return text.text();
  }
  function decodeBase64(text) {
    const out = new Uint8Array(Math.floor(text.length * 3 / 4));
    let length = 0;
    let bits = 0;
    let held = 0;
    for (let index = 0;index < text.length; index++) {
      const code = text.charCodeAt(index);
      if (code === 61)
        break;
      const sextet = code < 128 ? SEXTETS[code] ?? -1 : -1;
      if (sextet < 0)
        continue;
      held = (held << 6 | sextet) & 16777215;
      bits += 6;
      if (bits >= 8) {
        bits -= 8;
        out[length++] = held >> bits & 255;
      }
    }
    return out.subarray(0, length);
  }
  function encodeBase64(bytes, alphabet, pad) {
    const text = new TextWriter;
    const block = new Uint16Array(CHUNK);
    let filled = 0;
    let index = 0;
    for (;index + 2 < bytes.length; index += 3) {
      const n = (bytes[index] ?? 0) << 16 | (bytes[index + 1] ?? 0) << 8 | (bytes[index + 2] ?? 0);
      block[filled] = alphabet[n >> 18 & 63] ?? 0;
      block[filled + 1] = alphabet[n >> 12 & 63] ?? 0;
      block[filled + 2] = alphabet[n >> 6 & 63] ?? 0;
      block[filled + 3] = alphabet[n & 63] ?? 0;
      filled += 4;
      if (filled === CHUNK) {
        text.units(block);
        filled = 0;
      }
    }
    text.units(block.subarray(0, filled));
    const left = bytes.length - index;
    if (left > 0) {
      const n = (bytes[index] ?? 0) << 16 | (bytes[index + 1] ?? 0) << 8;
      text.unit(alphabet[n >> 18 & 63] ?? 0);
      text.unit(alphabet[n >> 12 & 63] ?? 0);
      if (left === 2)
        text.unit(alphabet[n >> 6 & 63] ?? 0);
      else if (pad)
        text.unit(61);
      if (pad)
        text.unit(61);
    }
    return text.text();
  }
  function decodeHex(text) {
    const out = new Uint8Array(text.length >> 1);
    let length = 0;
    for (let index = 0;index + 1 < text.length; index += 2) {
      const pair = text.slice(index, index + 2);
      if (!/^[0-9a-fA-F]{2}$/.test(pair))
        break;
      out[length++] = Number.parseInt(pair, 16);
    }
    return out.subarray(0, length);
  }
  function encode(text, encoding) {
    switch (encoding) {
      case "utf8":
        return encodeUtf8(text);
      case "hex":
        return decodeHex(text);
      case "base64":
      case "base64url":
        return decodeBase64(text);
      case "latin1":
      case "ascii": {
        const out = new Uint8Array(text.length);
        for (let index = 0;index < text.length; index++)
          out[index] = text.charCodeAt(index) & 255;
        return out;
      }
      case "utf16le": {
        const out = new Uint8Array(text.length * 2);
        for (let index = 0;index < text.length; index++) {
          const code = text.charCodeAt(index);
          out[index * 2] = code & 255;
          out[index * 2 + 1] = code >> 8;
        }
        return out;
      }
    }
  }
  function decode(bytes, encoding) {
    switch (encoding) {
      case "utf8":
        return decodeUtf8(bytes);
      case "hex": {
        const text = new TextWriter;
        for (const byte of bytes) {
          text.unit(HEX_DIGITS.charCodeAt(byte >> 4));
          text.unit(HEX_DIGITS.charCodeAt(byte & 15));
        }
        return text.text();
      }
      case "base64":
        return encodeBase64(bytes, BASE64_UNITS, true);
      case "base64url":
        return encodeBase64(bytes, BASE64URL_UNITS, false);
      case "latin1":
      case "ascii": {
        const mask = encoding === "ascii" ? 127 : 255;
        const text = new TextWriter;
        for (const byte of bytes)
          text.unit(byte & mask);
        return text.text();
      }
      case "utf16le": {
        const text = new TextWriter;
        for (let index = 0;index + 1 < bytes.length; index += 2)
          text.unit((bytes[index] ?? 0) | (bytes[index + 1] ?? 0) << 8);
        return text.text();
      }
    }
  }

  // crates/neo-sdk/neo-playwright-js/src/prelude/node-errors.ts
  function nodeError(Kind, code, message) {
    const error = new Kind(message);
    Reflect.set(error, "code", code);
    return error;
  }
  function received(value) {
    if (value === null)
      return "Received null";
    if (value === undefined)
      return "Received undefined";
    switch (typeof value) {
      case "function":
        return `Received function ${value.name}`;
      case "object": {
        const named = Reflect.get(value, "constructor");
        return typeof named === "function" ? `Received an instance of ${named.name}` : "Received [Object: null prototype] {}";
      }
      case "number":
        return `Received type number (${Object.is(value, -0) ? "-0" : String(value)})`;
      case "bigint":
        return `Received type bigint (${value}n)`;
      case "string": {
        const shown = value.length > 28 ? `${value.slice(0, 25)}...` : value;
        return shown.includes("'") ? `Received type string (${JSON.stringify(shown)})` : `Received type string ('${shown}')`;
      }
      default:
        return `Received type ${typeof value} (${String(value)})`;
    }
  }
  function outOfRange(name, range, value) {
    const shown = Number.isInteger(value) && Math.abs(value) > 2 ** 32 ? withSeparators(String(value)) : Object.is(value, -0) ? "-0" : String(value);
    return nodeError(RangeError, "ERR_OUT_OF_RANGE", `The value of "${name}" is out of range. It must be ${range}. Received ${shown}`);
  }
  function withSeparators(digits) {
    let grouped = "";
    let end = digits.length;
    const start = digits.startsWith("-") ? 1 : 0;
    for (;end >= start + 4; end -= 3)
      grouped = `_${digits.slice(end - 3, end)}${grouped}`;
    return `${digits.slice(0, end)}${grouped}`;
  }
  function invalidValue(value) {
    let shown = inspected(value);
    if (shown.length > 128)
      shown = `${shown.slice(0, 128)}...`;
    return nodeError(TypeError, "ERR_INVALID_ARG_VALUE", `The argument 'value' is invalid. Received ${shown}`);
  }
  var ESCAPES = new Map([
    [8, "\\b"],
    [9, "\\t"],
    [10, "\\n"],
    [12, "\\f"],
    [13, "\\r"],
    [92, "\\\\"]
  ]);
  function inspected(value) {
    if (typeof value !== "string") {
      const kind = Reflect.get(Object(value), "constructor");
      const name = typeof kind === "function" ? kind.name : "Object";
      return name === "Buffer" ? "<Buffer >" : `${name}(0) []`;
    }
    const quote = !value.includes("'") ? "'" : !value.includes('"') ? '"' : !value.includes("`") && !value.includes("${") ? "`" : "'";
    const source = value.slice(0, 129);
    let text = "";
    for (let index = 0;index < source.length; index++) {
      const code = source.charCodeAt(index);
      const pair = code >= 55296 && code <= 56319 && (source.charCodeAt(index + 1) & 64512) === 56320;
      if (pair) {
        text += source.slice(index, index + 2);
        index++;
      } else if (code === 39 && quote === "'")
        text += "\\'";
      else if (ESCAPES.has(code))
        text += ESCAPES.get(code);
      else if (code < 32 || code >= 127 && code < 160)
        text += `\\x${code.toString(16).toUpperCase().padStart(2, "0")}`;
      else if (code >= 55296 && code <= 57343)
        text += `\\u${code.toString(16)}`;
      else
        text += source[index];
    }
    return `${quote}${text}${quote}`;
  }

  // crates/neo-sdk/neo-playwright-js/src/prelude/buffer.ts
  var MAX_LENGTH = Number.MAX_SAFE_INTEGER;
  function encodingOf(name) {
    const encoding = encodingNamed(String(name));
    if (encoding === undefined)
      throw nodeError(TypeError, "ERR_UNKNOWN_ENCODING", `Unknown encoding: ${String(name)}`);
    return encoding;
  }
  function fillEncoding(name) {
    if (name === undefined || name === null || name === "")
      return "utf8";
    if (typeof name !== "string")
      throw nodeError(TypeError, "ERR_INVALID_ARG_TYPE", `The "encoding" argument must be of type string. ${received(name)}`);
    return encodingOf(name);
  }
  function fillBound(value, name, max) {
    if (typeof value !== "number")
      throw nodeError(TypeError, "ERR_INVALID_ARG_TYPE", `The "${name}" argument must be of type number. ${received(value)}`);
    if (!Number.isInteger(value))
      throw outOfRange(name, "an integer", value);
    if (value < 0 || value > max)
      throw outOfRange(name, `>= 0 && <= ${max}`, value);
    return value;
  }
  function searchStart(length, offset, size, forward) {
    if (offset < 0) {
      if (offset + length >= 0)
        return length + offset;
      return forward || size === 0 ? 0 : -1;
    }
    if (offset + size <= length)
      return offset;
    if (size === 0)
      return length;
    return forward ? -1 : length - 1;
  }
  function search(haystack, length, needle, offset, end, forward, wide) {
    const size = needle.length;
    let start = searchStart(length, offset, size, forward);
    let stop = Math.min(Math.max(end, 0), length);
    if (wide)
      stop -= stop % 2;
    if (size === 0)
      return Math.min(start, stop);
    if (length === 0 || start < 0)
      return -1;
    if (start >= stop) {
      if (forward || stop === 0)
        return -1;
      start = stop - 1;
    }
    if (forward && size + start > stop || size > stop)
      return -1;
    if (wide && (stop < 2 || size < 2))
      return -1;
    const step = wide ? 2 : 1;
    const span = size - size % step;
    const last = Math.floor(stop / step) - span / step;
    const first = Math.floor(start / step);
    const matches = (unit) => {
      for (let index = 0;index < span; index++)
        if (haystack[unit * step + index] !== needle[index])
          return false;
      return true;
    };
    if (forward) {
      for (let unit = first;unit <= last; unit++)
        if (matches(unit))
          return unit * step;
    } else
      for (let unit = Math.min(first, last);unit >= 0; unit--)
        if (matches(unit))
          return unit * step;
    return -1;
  }
  function stringBytes(text, encoding) {
    return typeof encoding === "string" && encoding.length > 0 ? encode(text, encodingOf(encoding)) : encode(text, "utf8");
  }
  function isArrayBuffer(value) {
    return value instanceof ArrayBuffer || typeof SharedArrayBuffer === "function" && value instanceof SharedArrayBuffer;
  }
  var FROM_TYPES = "The first argument must be of type string or an instance of Buffer, ArrayBuffer, or Array or an Array-like Object.";

  class Buffer extends Uint8Array {
    constructor(...args) {
      const [value, encoding] = args;
      const form = typeof value === "string" ? [stringBytes(value, encoding)] : args;
      super(...form);
    }
    static from(value, encodingOrOffset, length) {
      if (typeof value === "string")
        return Buffer.#copy(stringBytes(value, encodingOrOffset));
      if (typeof value === "object" && value !== null) {
        if (isArrayBuffer(value))
          return Buffer.#view(value, encodingOrOffset, length);
        const unwrap = Reflect.get(value, "valueOf");
        const primitive = typeof unwrap === "function" ? Reflect.apply(unwrap, value, []) : undefined;
        if (primitive !== null && primitive !== undefined && primitive !== value && (typeof primitive === "string" || typeof primitive === "object"))
          return Buffer.from(primitive, encodingOrOffset, length);
        const count = Reflect.get(value, "length");
        if (count !== undefined || isArrayBuffer(Reflect.get(value, "buffer")))
          return typeof count === "number" ? Buffer.#arrayLike(value, count) : new Buffer(0);
        const data = Reflect.get(value, "data");
        if (Reflect.get(value, "type") === "Buffer" && Array.isArray(data))
          return Buffer.#arrayLike(data, data.length);
        const toPrimitive = Reflect.get(value, Symbol.toPrimitive);
        if (typeof toPrimitive === "function") {
          const text = Reflect.apply(toPrimitive, value, ["string"]);
          if (typeof text === "string")
            return Buffer.#copy(stringBytes(text, encodingOrOffset));
        }
      }
      throw nodeError(TypeError, "ERR_INVALID_ARG_TYPE", `${FROM_TYPES} ${received(value)}`);
    }
    static alloc(size, fill, encoding) {
      if (typeof size !== "number")
        throw nodeError(TypeError, "ERR_INVALID_ARG_TYPE", `The "size" argument must be of type number. ${received(size)}`);
      if (!(size >= 0 && size <= MAX_LENGTH))
        throw outOfRange("size", `>= 0 && <= ${MAX_LENGTH}`, size);
      const buffer = new Buffer(Math.floor(size));
      if (fill === undefined || fill === 0 || !(size > 0))
        return buffer;
      return buffer.fill(fill, 0, buffer.length, encoding);
    }
    static allocUnsafe(size) {
      return Buffer.alloc(size);
    }
    static isBuffer(value) {
      return value instanceof Buffer;
    }
    static isEncoding(encoding) {
      return typeof encoding === "string" && encodingNamed(encoding) !== undefined;
    }
    static byteLength(value, encoding) {
      if (typeof value === "string")
        return stringBytes(value, encoding).length;
      if (ArrayBuffer.isView(value) || isArrayBuffer(value))
        return value.byteLength;
      throw nodeError(TypeError, "ERR_INVALID_ARG_TYPE", `The "string" argument must be of type string or an instance of Buffer or ArrayBuffer. ${received(value)}`);
    }
    static concat(list, totalLength) {
      if (!Array.isArray(list))
        throw nodeError(TypeError, "ERR_INVALID_ARG_TYPE", `The "list" argument must be an instance of Array. ${received(list)}`);
      const parts = list.map((part, index) => {
        if (part instanceof Uint8Array)
          return part;
        throw nodeError(TypeError, "ERR_INVALID_ARG_TYPE", `The "list[${index}]" argument must be an instance of Buffer or Uint8Array. ${received(part)}`);
      });
      const size = totalLength === undefined ? parts.reduce((sum, part) => sum + part.length, 0) : Number(totalLength);
      const out = Buffer.alloc(size);
      let at = 0;
      for (const part of parts) {
        if (at >= out.length)
          break;
        out.set(part.subarray(0, out.length - at), at);
        at += part.length;
      }
      return out;
    }
    toString(encoding, start, end) {
      const length = this.length;
      let from = Number(start ?? 0);
      if (!(from > 0))
        from = 0;
      else if (from >= length)
        return "";
      else
        from = Math.trunc(from);
      let to = end === undefined ? length : Number(end);
      to = to > length ? length : Math.trunc(to) || 0;
      if (to <= from)
        return "";
      const bytes = this.subarray(from, to);
      return encoding === undefined ? decode(bytes, "utf8") : decode(bytes, encodingOf(encoding));
    }
    fill(value, offset, end, encoding) {
      let [from, to, named] = [offset, end, encoding];
      if (typeof value === "string") {
        if (from === undefined || typeof from === "string")
          [from, to, named] = [0, this.length, from];
        else if (typeof to === "string")
          [to, named] = [this.length, to];
      }
      const text = typeof value === "string" ? fillEncoding(named) : undefined;
      let start = 0;
      let stop = this.length;
      if (from !== undefined) {
        start = fillBound(from, "offset", MAX_LENGTH);
        stop = to === undefined ? this.length : fillBound(to, "end", this.length);
        if (start >= stop)
          return this;
      }
      if (text === undefined || value === "") {
        if (ArrayBuffer.isView(value))
          this.#repeat(new Uint8Array(value.buffer, value.byteOffset, value.byteLength), value, start, stop);
        else
          Reflect.apply(Uint8Array.prototype.fill, this, [
            value === "" ? 0 : value,
            start,
            stop
          ]);
        return this;
      }
      this.#repeat(encode(String(value), text), value, start, stop);
      return this;
    }
    #repeat(pattern, value, start, end) {
      if (pattern.length === 0)
        throw invalidValue(value);
      let filled = Math.min(pattern.length, end - start);
      this.set(pattern.subarray(0, filled), start);
      while (start + filled < end) {
        const copied = Math.min(filled, end - start - filled);
        this.copyWithin(start + filled, start, start + copied);
        filled += copied;
      }
    }
    indexOf(value, byteOffset, end, encoding) {
      return this.#search(value, byteOffset, end, encoding, true);
    }
    lastIndexOf(value, byteOffset, end, encoding) {
      return this.#search(value, byteOffset, end, encoding, false);
    }
    includes(value, byteOffset, end, encoding) {
      return this.#search(value, byteOffset, end, encoding, true) !== -1;
    }
    #search(value, byteOffset, end, encoding, forward) {
      let [offset, stop, named] = [byteOffset, end, encoding];
      if (typeof stop === "string")
        [stop, named] = [this.length, stop];
      else if (stop === undefined)
        stop = this.length;
      if (typeof offset === "string")
        [offset, named] = [undefined, offset];
      let from = Math.min(Math.max(Number(offset), -2147483648), 2147483647);
      if (Number.isNaN(from))
        from = forward ? 0 : this.length;
      const until = Math.trunc(Number(stop)) || 0;
      const at = Math.trunc(from);
      if (typeof value === "number") {
        const byte = Uint8Array.of(value >>> 0 & 255);
        return search(this, this.length, byte, at, until, forward, false);
      }
      const known = named === undefined ? "utf8" : encodingNamed(String(named));
      if (typeof value === "string") {
        const text = known ?? encodingOf(named);
        const wide = text === "utf16le";
        const length = wide ? this.length - this.length % 2 : this.length;
        const needle = encode(value, text);
        return search(this, length, needle, at, until, forward, wide);
      }
      if (value instanceof Uint8Array)
        return search(this, this.length, value, at, until, forward, known === "utf16le");
      throw nodeError(TypeError, "ERR_INVALID_ARG_TYPE", `The "value" argument must be one of type number or string or an instance of Buffer or Uint8Array. ${received(value)}`);
    }
    toLocaleString(encoding, start, end) {
      return this.toString(encoding, start, end);
    }
    equals(other) {
      if (!(other instanceof Uint8Array))
        throw nodeError(TypeError, "ERR_INVALID_ARG_TYPE", `The "otherBuffer" argument must be an instance of Buffer or Uint8Array. ${received(other)}`);
      if (other.length !== this.length)
        return false;
      return this.every((byte, index) => byte === other[index]);
    }
    readUInt8(offset = 0) {
      return this.#readUnsigned(offset, 1, "be");
    }
    readUInt16BE(offset = 0) {
      return this.#readUnsigned(offset, 2, "be");
    }
    readUInt16LE(offset = 0) {
      return this.#readUnsigned(offset, 2, "le");
    }
    readUInt32BE(offset = 0) {
      return this.#readUnsigned(offset, 4, "be");
    }
    readUInt32LE(offset = 0) {
      return this.#readUnsigned(offset, 4, "le");
    }
    readInt32BE(offset = 0) {
      return this.readUInt32BE(offset) | 0;
    }
    #readUnsigned(offset, width, order) {
      if (typeof offset !== "number")
        throw nodeError(TypeError, "ERR_INVALID_ARG_TYPE", `The "offset" argument must be of type number. ${received(offset)}`);
      if (Math.floor(offset) !== offset)
        throw outOfRange("offset", "an integer", offset);
      const last = this.length - width;
      if (last < 0)
        throw nodeError(RangeError, "ERR_BUFFER_OUT_OF_BOUNDS", "Attempt to access memory outside buffer bounds");
      if (offset < 0 || offset > last)
        throw outOfRange("offset", `>= 0 and <= ${last}`, offset);
      let value = 0;
      for (let index = 0;index < width; index++) {
        const at = order === "be" ? offset + index : offset + width - index - 1;
        value = value * 256 + (this[at] ?? 0);
      }
      return value;
    }
    slice(start, end) {
      return this.subarray(start, end);
    }
    subarray(start, end) {
      const view = super.subarray(start, end);
      return new Buffer(view.buffer, view.byteOffset, view.length);
    }
    static #copy(bytes) {
      const buffer = new Buffer(bytes.length);
      buffer.set(bytes);
      return buffer;
    }
    static #view(memory, offset, length) {
      const start = offset === undefined ? 0 : Number(offset) || 0;
      const room = memory.byteLength - start;
      if (room < 0)
        throw Buffer.#outOfBounds("offset");
      let size = room;
      if (length !== undefined) {
        size = Number(length) > 0 ? Number(length) : 0;
        if (size > room)
          throw Buffer.#outOfBounds("length");
      }
      const view = Reflect.construct(Buffer, [memory, start, size]);
      return view;
    }
    static #outOfBounds(name) {
      return nodeError(RangeError, "ERR_BUFFER_OUT_OF_BOUNDS", `"${name}" is outside of buffer bounds`);
    }
    static #arrayLike(items, count) {
      const size = Math.max(0, Math.trunc(count) || 0);
      const buffer = new Buffer(size);
      for (let index = 0;index < size; index++)
        buffer[index] = Number(Reflect.get(items, index));
      return buffer;
    }
  }

  // crates/neo-sdk/neo-playwright/third_party/playwright/packages/isomorphic/utilityScriptSerializers.ts
  var kFunctionBindingPrefix = "__pw_fn_";
  var kBindingsControllerProperty = "__playwright__binding__controller__";
  function isRegExp(obj) {
    try {
      return obj instanceof RegExp || Object.prototype.toString.call(obj) === "[object RegExp]";
    } catch (error) {
      return false;
    }
  }
  function isDate(obj) {
    try {
      return obj instanceof Date || Object.prototype.toString.call(obj) === "[object Date]";
    } catch (error) {
      return false;
    }
  }
  function isURL(obj) {
    try {
      return obj instanceof URL || Object.prototype.toString.call(obj) === "[object URL]";
    } catch (error) {
      return false;
    }
  }
  function isError(obj) {
    try {
      return obj instanceof Error || obj && Object.getPrototypeOf(obj)?.name === "Error";
    } catch (error) {
      return false;
    }
  }
  function isTypedArray(obj, constructor) {
    try {
      return obj instanceof constructor || Object.prototype.toString.call(obj) === `[object ${constructor.name}]`;
    } catch (error) {
      return false;
    }
  }
  function isArrayBuffer2(obj) {
    try {
      return obj instanceof ArrayBuffer || Object.prototype.toString.call(obj) === "[object ArrayBuffer]";
    } catch (error) {
      return false;
    }
  }
  function isMap(obj) {
    try {
      return obj instanceof Map || Object.prototype.toString.call(obj) === "[object Map]";
    } catch (error) {
      return false;
    }
  }
  function isSet(obj) {
    try {
      return obj instanceof Set || Object.prototype.toString.call(obj) === "[object Set]";
    } catch (error) {
      return false;
    }
  }
  var typedArrayConstructors = {
    i8: Int8Array,
    ui8: Uint8Array,
    ui8c: Uint8ClampedArray,
    i16: Int16Array,
    ui16: Uint16Array,
    i32: Int32Array,
    ui32: Uint32Array,
    f32: Float32Array,
    f64: Float64Array,
    bi64: BigInt64Array,
    bui64: BigUint64Array
  };
  function typedArrayToBase64(array) {
    if ("toBase64" in array)
      return array.toBase64();
    const binary = Array.from(new Uint8Array(array.buffer, array.byteOffset, array.byteLength)).map((b) => String.fromCharCode(b)).join("");
    return btoa(binary);
  }
  function base64ToTypedArray(base64, TypedArrayConstructor) {
    const binary = atob(base64);
    const bytes = new Uint8Array(binary.length);
    for (let i = 0;i < binary.length; i++)
      bytes[i] = binary.charCodeAt(i);
    return new TypedArrayConstructor(bytes.buffer);
  }
  function parseEvaluationResultValue(value, handles = [], refs = new Map) {
    if (Object.is(value, undefined))
      return;
    if (typeof value === "object" && value) {
      if ("ref" in value)
        return refs.get(value.ref);
      if ("v" in value) {
        if (value.v === "undefined")
          return;
        if (value.v === "null")
          return null;
        if (value.v === "NaN")
          return NaN;
        if (value.v === "Infinity")
          return Infinity;
        if (value.v === "-Infinity")
          return -Infinity;
        if (value.v === "-0")
          return -0;
        return;
      }
      if ("d" in value) {
        return new Date(value.d);
      }
      if ("u" in value)
        return new URL(value.u);
      if ("bi" in value)
        return BigInt(value.bi);
      if ("e" in value) {
        const error = new Error(value.e.m);
        error.name = value.e.n;
        error.stack = value.e.s;
        return error;
      }
      if ("r" in value)
        return new RegExp(value.r.p, value.r.f);
      if ("a" in value) {
        const result = [];
        refs.set(value.id, result);
        for (const a of value.a)
          result.push(parseEvaluationResultValue(a, handles, refs));
        return result;
      }
      if ("o" in value) {
        const result = {};
        refs.set(value.id, result);
        for (const { k, v } of value.o) {
          if (k === "__proto__")
            continue;
          result[k] = parseEvaluationResultValue(v, handles, refs);
        }
        return result;
      }
      if ("m" in value) {
        const result = new Map;
        refs.set(value.id, result);
        for (const { k, v } of value.m)
          result.set(parseEvaluationResultValue(k, handles, refs), parseEvaluationResultValue(v, handles, refs));
        return result;
      }
      if ("s" in value) {
        const result = new Set;
        refs.set(value.id, result);
        for (const item of value.s)
          result.add(parseEvaluationResultValue(item, handles, refs));
        return result;
      }
      if ("h" in value)
        return handles[value.h];
      if ("fn" in value) {
        const name = value.fn;
        return (...args) => globalThis[kBindingsControllerProperty].callBinding(name, ...args);
      }
      if ("ta" in value)
        return base64ToTypedArray(value.ta.b, typedArrayConstructors[value.ta.k]);
      if ("ab" in value)
        return base64ToTypedArray(value.ab.b, Uint8Array).buffer;
    }
    return value;
  }
  function serializeAsCallArgument(value, handleSerializer, options = {}) {
    return serialize(value, handleSerializer, { visited: new Map, lastId: 0, extendedSerialization: options.extendedSerialization });
  }
  function serialize(value, handleSerializer, visitorInfo) {
    if (value && typeof value === "object") {
      if (typeof globalThis.Window === "function" && value instanceof globalThis.Window)
        return "ref: <Window>";
      if (typeof globalThis.Document === "function" && value instanceof globalThis.Document)
        return "ref: <Document>";
      if (typeof globalThis.Node === "function" && value instanceof globalThis.Node)
        return "ref: <Node>";
    }
    return innerSerialize(value, handleSerializer, visitorInfo);
  }
  function innerSerialize(value, handleSerializer, visitorInfo) {
    const result = handleSerializer(value);
    if ("fallThrough" in result)
      value = result.fallThrough;
    else
      return result;
    if (typeof value === "symbol")
      return { v: "undefined" };
    if (Object.is(value, undefined))
      return { v: "undefined" };
    if (Object.is(value, null))
      return { v: "null" };
    if (Object.is(value, NaN))
      return { v: "NaN" };
    if (Object.is(value, Infinity))
      return { v: "Infinity" };
    if (Object.is(value, -Infinity))
      return { v: "-Infinity" };
    if (Object.is(value, -0))
      return { v: "-0" };
    if (typeof value === "boolean")
      return value;
    if (typeof value === "number")
      return value;
    if (typeof value === "string")
      return value;
    if (typeof value === "bigint")
      return { bi: value.toString() };
    if (isError(value)) {
      let stack;
      if (value.stack?.startsWith(value.name + ": " + value.message)) {
        stack = value.stack;
      } else {
        stack = `${value.name}: ${value.message}
${value.stack}`;
      }
      return { e: { n: value.name, m: value.message, s: stack } };
    }
    if (isDate(value))
      return { d: value.toJSON() };
    if (isURL(value))
      return { u: value.toJSON() };
    if (isRegExp(value))
      return { r: { p: value.source, f: value.flags } };
    for (const [k, ctor] of Object.entries(typedArrayConstructors)) {
      if (isTypedArray(value, ctor))
        return { ta: { b: typedArrayToBase64(value), k } };
    }
    if (isArrayBuffer2(value))
      return { ab: { b: typedArrayToBase64(new Uint8Array(value)) } };
    const id = visitorInfo.visited.get(value);
    if (id)
      return { ref: id };
    if (visitorInfo.extendedSerialization) {
      if (isMap(value)) {
        const m = [];
        const id = ++visitorInfo.lastId;
        visitorInfo.visited.set(value, id);
        const iterator = value.entries();
        const next = new Map().entries().next;
        while (true) {
          const entry = next.call(iterator);
          if (entry.done)
            break;
          m.push({ k: serialize(entry.value[0], handleSerializer, visitorInfo), v: serialize(entry.value[1], handleSerializer, visitorInfo) });
        }
        return { m, id };
      }
      if (isSet(value)) {
        const s = [];
        const id = ++visitorInfo.lastId;
        visitorInfo.visited.set(value, id);
        const iterator = value.values();
        const next = new Set().values().next;
        while (true) {
          const entry = next.call(iterator);
          if (entry.done)
            break;
          s.push(serialize(entry.value, handleSerializer, visitorInfo));
        }
        return { s, id };
      }
    }
    if (Array.isArray(value)) {
      const a = [];
      const id = ++visitorInfo.lastId;
      visitorInfo.visited.set(value, id);
      for (let i = 0;i < value.length; ++i)
        a.push(serialize(value[i], handleSerializer, visitorInfo));
      return { a, id };
    }
    if (typeof value === "object") {
      const o = [];
      const id = ++visitorInfo.lastId;
      visitorInfo.visited.set(value, id);
      for (const name of Object.keys(value)) {
        let item;
        try {
          item = value[name];
        } catch (e) {
          continue;
        }
        if (name === "toJSON" && typeof item === "function")
          o.push({ k: name, v: { o: [], id: 0 } });
        else
          o.push({ k: name, v: serialize(item, handleSerializer, visitorInfo) });
      }
      let jsonWrapper;
      try {
        if (o.length === 0 && value.toJSON && typeof value.toJSON === "function")
          jsonWrapper = { value: value.toJSON() };
      } catch (e) {}
      if (jsonWrapper)
        return innerSerialize(jsonWrapper.value, handleSerializer, visitorInfo);
      return { o, id };
    }
    if (typeof value === "function" && value.name.startsWith(kFunctionBindingPrefix))
      return { fn: value.name };
  }

  // crates/neo-sdk/neo-playwright-js/src/prelude/vendored-globals.ts
  var Buffer2 = {
    from: (buffer, byteOffset, length) => new Uint8Array(buffer, byteOffset, length)
  };

  // crates/neo-sdk/neo-playwright/third_party/playwright/packages/protocol/src/serializers.ts
  function parseSerializedValue(value, handles) {
    return innerParseSerializedValue(value, handles, new Map, []);
  }
  function innerParseSerializedValue(value, handles, refs, accessChain) {
    if (value.ref !== undefined)
      return refs.get(value.ref);
    if (value.n !== undefined)
      return value.n;
    if (value.s !== undefined)
      return value.s;
    if (value.b !== undefined)
      return value.b;
    if (value.v !== undefined) {
      if (value.v === "undefined")
        return;
      if (value.v === "null")
        return null;
      if (value.v === "NaN")
        return NaN;
      if (value.v === "Infinity")
        return Infinity;
      if (value.v === "-Infinity")
        return -Infinity;
      if (value.v === "-0")
        return -0;
    }
    if (value.d !== undefined)
      return new Date(value.d);
    if (value.u !== undefined)
      return new URL(value.u);
    if (value.bi !== undefined)
      return BigInt(value.bi);
    if (value.e !== undefined) {
      const error = new Error(value.e.m);
      error.name = value.e.n;
      error.stack = value.e.s;
      return error;
    }
    if (value.r !== undefined)
      return new RegExp(value.r.p, value.r.f);
    if (value.ta !== undefined) {
      const ctor = typedArrayKindToConstructor[value.ta.k];
      return new ctor(value.ta.b.buffer, value.ta.b.byteOffset, value.ta.b.length / ctor.BYTES_PER_ELEMENT);
    }
    if (value.a !== undefined) {
      const result = [];
      refs.set(value.id, result);
      for (let i = 0;i < value.a.length; i++)
        result.push(innerParseSerializedValue(value.a[i], handles, refs, [...accessChain, i]));
      return result;
    }
    if (value.o !== undefined) {
      const result = {};
      refs.set(value.id, result);
      for (const { k, v } of value.o)
        result[k] = innerParseSerializedValue(v, handles, refs, [...accessChain, k]);
      return result;
    }
    if (value.h !== undefined) {
      if (handles === undefined)
        throw new Error("Unexpected handle");
      return handles[value.h];
    }
    if (value.fn !== undefined) {
      const dummy = () => {};
      Object.defineProperty(dummy, "name", { value: value.fn });
      return dummy;
    }
    throw new Error(`Attempting to deserialize unexpected value${accessChainToDisplayString(accessChain)}: ${value}`);
  }
  function serializeValue(value, handleSerializer) {
    return innerSerializeValue(value, handleSerializer, { lastId: 0, visited: new Map }, []);
  }
  function innerSerializeValue(value, handleSerializer, visitorInfo, accessChain) {
    const handle = handleSerializer(value);
    if ("fallThrough" in handle)
      value = handle.fallThrough;
    else
      return handle;
    if (typeof value === "symbol")
      return { v: "undefined" };
    if (Object.is(value, undefined))
      return { v: "undefined" };
    if (Object.is(value, null))
      return { v: "null" };
    if (Object.is(value, NaN))
      return { v: "NaN" };
    if (Object.is(value, Infinity))
      return { v: "Infinity" };
    if (Object.is(value, -Infinity))
      return { v: "-Infinity" };
    if (Object.is(value, -0))
      return { v: "-0" };
    if (typeof value === "boolean")
      return { b: value };
    if (typeof value === "number")
      return { n: value };
    if (typeof value === "string")
      return { s: value };
    if (typeof value === "bigint")
      return { bi: value.toString() };
    if (isError2(value))
      return { e: { n: value.name, m: value.message, s: value.stack || "" } };
    if (isDate2(value))
      return { d: value.toJSON() };
    if (isURL2(value))
      return { u: value.toJSON() };
    if (isRegExp2(value))
      return { r: { p: value.source, f: value.flags } };
    const typedArrayKind = constructorToTypedArrayKind.get(value.constructor);
    if (typedArrayKind)
      return { ta: { b: Buffer2.from(value.buffer, value.byteOffset, value.byteLength), k: typedArrayKind } };
    const id = visitorInfo.visited.get(value);
    if (id)
      return { ref: id };
    if (Array.isArray(value)) {
      const a = [];
      const id = ++visitorInfo.lastId;
      visitorInfo.visited.set(value, id);
      for (let i = 0;i < value.length; ++i)
        a.push(innerSerializeValue(value[i], handleSerializer, visitorInfo, [...accessChain, i]));
      return { a, id };
    }
    if (typeof value === "object") {
      const o = [];
      const id = ++visitorInfo.lastId;
      visitorInfo.visited.set(value, id);
      for (const name of Object.keys(value))
        o.push({ k: name, v: innerSerializeValue(value[name], handleSerializer, visitorInfo, [...accessChain, name]) });
      return { o, id };
    }
    throw new Error(`Attempting to serialize unexpected value${accessChainToDisplayString(accessChain)}: ${value}`);
  }
  function accessChainToDisplayString(accessChain) {
    const chainString = accessChain.map((accessor, i) => {
      if (typeof accessor === "string")
        return i ? `.${accessor}` : accessor;
      return `[${accessor}]`;
    }).join("");
    return chainString.length > 0 ? ` at position "${chainString}"` : "";
  }
  function isRegExp2(obj) {
    return obj instanceof RegExp || Object.prototype.toString.call(obj) === "[object RegExp]";
  }
  function isDate2(obj) {
    return obj instanceof Date || Object.prototype.toString.call(obj) === "[object Date]";
  }
  function isURL2(obj) {
    return obj instanceof URL || Object.prototype.toString.call(obj) === "[object URL]";
  }
  function isError2(obj) {
    const proto = obj ? Object.getPrototypeOf(obj) : null;
    return obj instanceof Error || proto?.name === "Error" || proto && isError2(proto);
  }
  var typedArrayKindToConstructor = {
    i8: Int8Array,
    ui8: Uint8Array,
    ui8c: Uint8ClampedArray,
    i16: Int16Array,
    ui16: Uint16Array,
    i32: Int32Array,
    ui32: Uint32Array,
    f32: Float32Array,
    f64: Float64Array,
    bi64: BigInt64Array,
    bui64: BigUint64Array
  };
  var constructorToTypedArrayKind = new Map(Object.entries(typedArrayKindToConstructor).map(([k, v]) => [v, k]));

  // crates/neo-sdk/neo-playwright/third_party/mime/types/other.ts
  var types = {
    "application/prs.cww": ["cww"],
    "application/prs.xsf+xml": ["xsf"],
    "application/vnd.1000minds.decision-model+xml": ["1km"],
    "application/vnd.3gpp.pic-bw-large": ["plb"],
    "application/vnd.3gpp.pic-bw-small": ["psb"],
    "application/vnd.3gpp.pic-bw-var": ["pvb"],
    "application/vnd.3gpp2.tcap": ["tcap"],
    "application/vnd.3m.post-it-notes": ["pwn"],
    "application/vnd.accpac.simply.aso": ["aso"],
    "application/vnd.accpac.simply.imp": ["imp"],
    "application/vnd.acucobol": ["acu"],
    "application/vnd.acucorp": ["atc", "acutc"],
    "application/vnd.adobe.air-application-installer-package+zip": ["air"],
    "application/vnd.adobe.formscentral.fcdt": ["fcdt"],
    "application/vnd.adobe.fxp": ["fxp", "fxpl"],
    "application/vnd.adobe.xdp+xml": ["xdp"],
    "application/vnd.adobe.xfdf": ["*xfdf"],
    "application/vnd.age": ["age"],
    "application/vnd.ahead.space": ["ahead"],
    "application/vnd.airzip.filesecure.azf": ["azf"],
    "application/vnd.airzip.filesecure.azs": ["azs"],
    "application/vnd.amazon.ebook": ["azw"],
    "application/vnd.americandynamics.acc": ["acc"],
    "application/vnd.amiga.ami": ["ami"],
    "application/vnd.android.package-archive": ["apk"],
    "application/vnd.anser-web-certificate-issue-initiation": ["cii"],
    "application/vnd.anser-web-funds-transfer-initiation": ["fti"],
    "application/vnd.antix.game-component": ["atx"],
    "application/vnd.apple.installer+xml": ["mpkg"],
    "application/vnd.apple.keynote": ["key"],
    "application/vnd.apple.mpegurl": ["m3u8"],
    "application/vnd.apple.numbers": ["numbers"],
    "application/vnd.apple.pages": ["pages"],
    "application/vnd.apple.pkpass": ["pkpass"],
    "application/vnd.aristanetworks.swi": ["swi"],
    "application/vnd.astraea-software.iota": ["iota"],
    "application/vnd.audiograph": ["aep"],
    "application/vnd.autodesk.fbx": ["fbx"],
    "application/vnd.balsamiq.bmml+xml": ["bmml"],
    "application/vnd.blueice.multipass": ["mpm"],
    "application/vnd.bmi": ["bmi"],
    "application/vnd.businessobjects": ["rep"],
    "application/vnd.chemdraw+xml": ["cdxml"],
    "application/vnd.chipnuts.karaoke-mmd": ["mmd"],
    "application/vnd.cinderella": ["cdy"],
    "application/vnd.citationstyles.style+xml": ["csl"],
    "application/vnd.claymore": ["cla"],
    "application/vnd.cloanto.rp9": ["rp9"],
    "application/vnd.clonk.c4group": ["c4g", "c4d", "c4f", "c4p", "c4u"],
    "application/vnd.cluetrust.cartomobile-config": ["c11amc"],
    "application/vnd.cluetrust.cartomobile-config-pkg": ["c11amz"],
    "application/vnd.commonspace": ["csp"],
    "application/vnd.contact.cmsg": ["cdbcmsg"],
    "application/vnd.cosmocaller": ["cmc"],
    "application/vnd.crick.clicker": ["clkx"],
    "application/vnd.crick.clicker.keyboard": ["clkk"],
    "application/vnd.crick.clicker.palette": ["clkp"],
    "application/vnd.crick.clicker.template": ["clkt"],
    "application/vnd.crick.clicker.wordbank": ["clkw"],
    "application/vnd.criticaltools.wbs+xml": ["wbs"],
    "application/vnd.ctc-posml": ["pml"],
    "application/vnd.cups-ppd": ["ppd"],
    "application/vnd.curl.car": ["car"],
    "application/vnd.curl.pcurl": ["pcurl"],
    "application/vnd.dart": ["dart"],
    "application/vnd.data-vision.rdz": ["rdz"],
    "application/vnd.dbf": ["dbf"],
    "application/vnd.dcmp+xml": ["dcmp"],
    "application/vnd.dece.data": ["uvf", "uvvf", "uvd", "uvvd"],
    "application/vnd.dece.ttml+xml": ["uvt", "uvvt"],
    "application/vnd.dece.unspecified": ["uvx", "uvvx"],
    "application/vnd.dece.zip": ["uvz", "uvvz"],
    "application/vnd.denovo.fcselayout-link": ["fe_launch"],
    "application/vnd.dna": ["dna"],
    "application/vnd.dolby.mlp": ["mlp"],
    "application/vnd.dpgraph": ["dpg"],
    "application/vnd.dreamfactory": ["dfac"],
    "application/vnd.ds-keypoint": ["kpxx"],
    "application/vnd.dvb.ait": ["ait"],
    "application/vnd.dvb.service": ["svc"],
    "application/vnd.dynageo": ["geo"],
    "application/vnd.ecowin.chart": ["mag"],
    "application/vnd.enliven": ["nml"],
    "application/vnd.epson.esf": ["esf"],
    "application/vnd.epson.msf": ["msf"],
    "application/vnd.epson.quickanime": ["qam"],
    "application/vnd.epson.salt": ["slt"],
    "application/vnd.epson.ssf": ["ssf"],
    "application/vnd.eszigno3+xml": ["es3", "et3"],
    "application/vnd.ezpix-album": ["ez2"],
    "application/vnd.ezpix-package": ["ez3"],
    "application/vnd.fdf": ["*fdf"],
    "application/vnd.fdsn.mseed": ["mseed"],
    "application/vnd.fdsn.seed": ["seed", "dataless"],
    "application/vnd.flographit": ["gph"],
    "application/vnd.fluxtime.clip": ["ftc"],
    "application/vnd.framemaker": ["fm", "frame", "maker", "book"],
    "application/vnd.frogans.fnc": ["fnc"],
    "application/vnd.frogans.ltf": ["ltf"],
    "application/vnd.fsc.weblaunch": ["fsc"],
    "application/vnd.fujitsu.oasys": ["oas"],
    "application/vnd.fujitsu.oasys2": ["oa2"],
    "application/vnd.fujitsu.oasys3": ["oa3"],
    "application/vnd.fujitsu.oasysgp": ["fg5"],
    "application/vnd.fujitsu.oasysprs": ["bh2"],
    "application/vnd.fujixerox.ddd": ["ddd"],
    "application/vnd.fujixerox.docuworks": ["xdw"],
    "application/vnd.fujixerox.docuworks.binder": ["xbd"],
    "application/vnd.fuzzysheet": ["fzs"],
    "application/vnd.genomatix.tuxedo": ["txd"],
    "application/vnd.geogebra.file": ["ggb"],
    "application/vnd.geogebra.slides": ["ggs"],
    "application/vnd.geogebra.tool": ["ggt"],
    "application/vnd.geometry-explorer": ["gex", "gre"],
    "application/vnd.geonext": ["gxt"],
    "application/vnd.geoplan": ["g2w"],
    "application/vnd.geospace": ["g3w"],
    "application/vnd.gmx": ["gmx"],
    "application/vnd.google-apps.document": ["gdoc"],
    "application/vnd.google-apps.drawing": ["gdraw"],
    "application/vnd.google-apps.form": ["gform"],
    "application/vnd.google-apps.jam": ["gjam"],
    "application/vnd.google-apps.map": ["gmap"],
    "application/vnd.google-apps.presentation": ["gslides"],
    "application/vnd.google-apps.script": ["gscript"],
    "application/vnd.google-apps.site": ["gsite"],
    "application/vnd.google-apps.spreadsheet": ["gsheet"],
    "application/vnd.google-earth.kml+xml": ["kml"],
    "application/vnd.google-earth.kmz": ["kmz"],
    "application/vnd.gov.sk.xmldatacontainer+xml": ["xdcf"],
    "application/vnd.grafeq": ["gqf", "gqs"],
    "application/vnd.groove-account": ["gac"],
    "application/vnd.groove-help": ["ghf"],
    "application/vnd.groove-identity-message": ["gim"],
    "application/vnd.groove-injector": ["grv"],
    "application/vnd.groove-tool-message": ["gtm"],
    "application/vnd.groove-tool-template": ["tpl"],
    "application/vnd.groove-vcard": ["vcg"],
    "application/vnd.hal+xml": ["hal"],
    "application/vnd.handheld-entertainment+xml": ["zmm"],
    "application/vnd.hbci": ["hbci"],
    "application/vnd.hhe.lesson-player": ["les"],
    "application/vnd.hp-hpgl": ["hpgl"],
    "application/vnd.hp-hpid": ["hpid"],
    "application/vnd.hp-hps": ["hps"],
    "application/vnd.hp-jlyt": ["jlt"],
    "application/vnd.hp-pcl": ["pcl"],
    "application/vnd.hp-pclxl": ["pclxl"],
    "application/vnd.hydrostatix.sof-data": ["sfd-hdstx"],
    "application/vnd.ibm.minipay": ["mpy"],
    "application/vnd.ibm.modcap": ["afp", "listafp", "list3820"],
    "application/vnd.ibm.rights-management": ["irm"],
    "application/vnd.ibm.secure-container": ["sc"],
    "application/vnd.iccprofile": ["icc", "icm"],
    "application/vnd.igloader": ["igl"],
    "application/vnd.immervision-ivp": ["ivp"],
    "application/vnd.immervision-ivu": ["ivu"],
    "application/vnd.insors.igm": ["igm"],
    "application/vnd.intercon.formnet": ["xpw", "xpx"],
    "application/vnd.intergeo": ["i2g"],
    "application/vnd.intu.qbo": ["qbo"],
    "application/vnd.intu.qfx": ["qfx"],
    "application/vnd.ipunplugged.rcprofile": ["rcprofile"],
    "application/vnd.irepository.package+xml": ["irp"],
    "application/vnd.is-xpr": ["xpr"],
    "application/vnd.isac.fcs": ["fcs"],
    "application/vnd.jam": ["jam"],
    "application/vnd.jcp.javame.midlet-rms": ["rms"],
    "application/vnd.jisp": ["jisp"],
    "application/vnd.joost.joda-archive": ["joda"],
    "application/vnd.kahootz": ["ktz", "ktr"],
    "application/vnd.kde.karbon": ["karbon"],
    "application/vnd.kde.kchart": ["chrt"],
    "application/vnd.kde.kformula": ["kfo"],
    "application/vnd.kde.kivio": ["flw"],
    "application/vnd.kde.kontour": ["kon"],
    "application/vnd.kde.kpresenter": ["kpr", "kpt"],
    "application/vnd.kde.kspread": ["ksp"],
    "application/vnd.kde.kword": ["kwd", "kwt"],
    "application/vnd.kenameaapp": ["htke"],
    "application/vnd.kidspiration": ["kia"],
    "application/vnd.kinar": ["kne", "knp"],
    "application/vnd.koan": ["skp", "skd", "skt", "skm"],
    "application/vnd.kodak-descriptor": ["sse"],
    "application/vnd.las.las+xml": ["lasxml"],
    "application/vnd.llamagraphics.life-balance.desktop": ["lbd"],
    "application/vnd.llamagraphics.life-balance.exchange+xml": ["lbe"],
    "application/vnd.lotus-1-2-3": ["123"],
    "application/vnd.lotus-approach": ["apr"],
    "application/vnd.lotus-freelance": ["pre"],
    "application/vnd.lotus-notes": ["nsf"],
    "application/vnd.lotus-organizer": ["org"],
    "application/vnd.lotus-screencam": ["scm"],
    "application/vnd.lotus-wordpro": ["lwp"],
    "application/vnd.macports.portpkg": ["portpkg"],
    "application/vnd.mapbox-vector-tile": ["mvt"],
    "application/vnd.mcd": ["mcd"],
    "application/vnd.medcalcdata": ["mc1"],
    "application/vnd.mediastation.cdkey": ["cdkey"],
    "application/vnd.mfer": ["mwf"],
    "application/vnd.mfmp": ["mfm"],
    "application/vnd.micrografx.flo": ["flo"],
    "application/vnd.micrografx.igx": ["igx"],
    "application/vnd.mif": ["mif"],
    "application/vnd.mobius.daf": ["daf"],
    "application/vnd.mobius.dis": ["dis"],
    "application/vnd.mobius.mbk": ["mbk"],
    "application/vnd.mobius.mqy": ["mqy"],
    "application/vnd.mobius.msl": ["msl"],
    "application/vnd.mobius.plc": ["plc"],
    "application/vnd.mobius.txf": ["txf"],
    "application/vnd.mophun.application": ["mpn"],
    "application/vnd.mophun.certificate": ["mpc"],
    "application/vnd.mozilla.xul+xml": ["xul"],
    "application/vnd.ms-artgalry": ["cil"],
    "application/vnd.ms-cab-compressed": ["cab"],
    "application/vnd.ms-excel": ["xls", "xlm", "xla", "xlc", "xlt", "xlw"],
    "application/vnd.ms-excel.addin.macroenabled.12": ["xlam"],
    "application/vnd.ms-excel.sheet.binary.macroenabled.12": ["xlsb"],
    "application/vnd.ms-excel.sheet.macroenabled.12": ["xlsm"],
    "application/vnd.ms-excel.template.macroenabled.12": ["xltm"],
    "application/vnd.ms-fontobject": ["eot"],
    "application/vnd.ms-htmlhelp": ["chm"],
    "application/vnd.ms-ims": ["ims"],
    "application/vnd.ms-lrm": ["lrm"],
    "application/vnd.ms-officetheme": ["thmx"],
    "application/vnd.ms-outlook": ["msg"],
    "application/vnd.ms-pki.seccat": ["cat"],
    "application/vnd.ms-pki.stl": ["*stl"],
    "application/vnd.ms-powerpoint": ["ppt", "pps", "pot"],
    "application/vnd.ms-powerpoint.addin.macroenabled.12": ["ppam"],
    "application/vnd.ms-powerpoint.presentation.macroenabled.12": ["pptm"],
    "application/vnd.ms-powerpoint.slide.macroenabled.12": ["sldm"],
    "application/vnd.ms-powerpoint.slideshow.macroenabled.12": ["ppsm"],
    "application/vnd.ms-powerpoint.template.macroenabled.12": ["potm"],
    "application/vnd.ms-project": ["*mpp", "mpt"],
    "application/vnd.ms-visio.viewer": ["vdx"],
    "application/vnd.ms-word.document.macroenabled.12": ["docm"],
    "application/vnd.ms-word.template.macroenabled.12": ["dotm"],
    "application/vnd.ms-works": ["wps", "wks", "wcm", "wdb"],
    "application/vnd.ms-wpl": ["wpl"],
    "application/vnd.ms-xpsdocument": ["xps"],
    "application/vnd.mseq": ["mseq"],
    "application/vnd.musician": ["mus"],
    "application/vnd.muvee.style": ["msty"],
    "application/vnd.mynfc": ["taglet"],
    "application/vnd.nato.bindingdataobject+xml": ["bdo"],
    "application/vnd.neurolanguage.nlu": ["nlu"],
    "application/vnd.nitf": ["ntf", "nitf"],
    "application/vnd.noblenet-directory": ["nnd"],
    "application/vnd.noblenet-sealer": ["nns"],
    "application/vnd.noblenet-web": ["nnw"],
    "application/vnd.nokia.n-gage.ac+xml": ["*ac"],
    "application/vnd.nokia.n-gage.data": ["ngdat"],
    "application/vnd.nokia.n-gage.symbian.install": ["n-gage"],
    "application/vnd.nokia.radio-preset": ["rpst"],
    "application/vnd.nokia.radio-presets": ["rpss"],
    "application/vnd.novadigm.edm": ["edm"],
    "application/vnd.novadigm.edx": ["edx"],
    "application/vnd.novadigm.ext": ["ext"],
    "application/vnd.oasis.opendocument.chart": ["odc"],
    "application/vnd.oasis.opendocument.chart-template": ["otc"],
    "application/vnd.oasis.opendocument.database": ["odb"],
    "application/vnd.oasis.opendocument.formula": ["odf"],
    "application/vnd.oasis.opendocument.formula-template": ["odft"],
    "application/vnd.oasis.opendocument.graphics": ["odg"],
    "application/vnd.oasis.opendocument.graphics-template": ["otg"],
    "application/vnd.oasis.opendocument.image": ["odi"],
    "application/vnd.oasis.opendocument.image-template": ["oti"],
    "application/vnd.oasis.opendocument.presentation": ["odp"],
    "application/vnd.oasis.opendocument.presentation-template": ["otp"],
    "application/vnd.oasis.opendocument.spreadsheet": ["ods"],
    "application/vnd.oasis.opendocument.spreadsheet-template": ["ots"],
    "application/vnd.oasis.opendocument.text": ["odt"],
    "application/vnd.oasis.opendocument.text-master": ["odm"],
    "application/vnd.oasis.opendocument.text-template": ["ott"],
    "application/vnd.oasis.opendocument.text-web": ["oth"],
    "application/vnd.olpc-sugar": ["xo"],
    "application/vnd.oma.dd2+xml": ["dd2"],
    "application/vnd.openblox.game+xml": ["obgx"],
    "application/vnd.openofficeorg.extension": ["oxt"],
    "application/vnd.openstreetmap.data+xml": ["osm"],
    "application/vnd.openxmlformats-officedocument.presentationml.presentation": [
      "pptx"
    ],
    "application/vnd.openxmlformats-officedocument.presentationml.slide": [
      "sldx"
    ],
    "application/vnd.openxmlformats-officedocument.presentationml.slideshow": [
      "ppsx"
    ],
    "application/vnd.openxmlformats-officedocument.presentationml.template": [
      "potx"
    ],
    "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet": ["xlsx"],
    "application/vnd.openxmlformats-officedocument.spreadsheetml.template": [
      "xltx"
    ],
    "application/vnd.openxmlformats-officedocument.wordprocessingml.document": [
      "docx"
    ],
    "application/vnd.openxmlformats-officedocument.wordprocessingml.template": [
      "dotx"
    ],
    "application/vnd.osgeo.mapguide.package": ["mgp"],
    "application/vnd.osgi.dp": ["dp"],
    "application/vnd.osgi.subsystem": ["esa"],
    "application/vnd.palm": ["pdb", "pqa", "oprc"],
    "application/vnd.pawaafile": ["paw"],
    "application/vnd.pg.format": ["str"],
    "application/vnd.pg.osasli": ["ei6"],
    "application/vnd.picsel": ["efif"],
    "application/vnd.pmi.widget": ["wg"],
    "application/vnd.pocketlearn": ["plf"],
    "application/vnd.powerbuilder6": ["pbd"],
    "application/vnd.previewsystems.box": ["box"],
    "application/vnd.procrate.brushset": ["brushset"],
    "application/vnd.procreate.brush": ["brush"],
    "application/vnd.procreate.dream": ["drm"],
    "application/vnd.proteus.magazine": ["mgz"],
    "application/vnd.publishare-delta-tree": ["qps"],
    "application/vnd.pvi.ptid1": ["ptid"],
    "application/vnd.pwg-xhtml-print+xml": ["xhtm"],
    "application/vnd.quark.quarkxpress": [
      "qxd",
      "qxt",
      "qwd",
      "qwt",
      "qxl",
      "qxb"
    ],
    "application/vnd.rar": ["rar"],
    "application/vnd.realvnc.bed": ["bed"],
    "application/vnd.recordare.musicxml": ["mxl"],
    "application/vnd.recordare.musicxml+xml": ["musicxml"],
    "application/vnd.rig.cryptonote": ["cryptonote"],
    "application/vnd.rim.cod": ["cod"],
    "application/vnd.rn-realmedia": ["rm"],
    "application/vnd.rn-realmedia-vbr": ["rmvb"],
    "application/vnd.route66.link66+xml": ["link66"],
    "application/vnd.sailingtracker.track": ["st"],
    "application/vnd.seemail": ["see"],
    "application/vnd.sema": ["sema"],
    "application/vnd.semd": ["semd"],
    "application/vnd.semf": ["semf"],
    "application/vnd.shana.informed.formdata": ["ifm"],
    "application/vnd.shana.informed.formtemplate": ["itp"],
    "application/vnd.shana.informed.interchange": ["iif"],
    "application/vnd.shana.informed.package": ["ipk"],
    "application/vnd.simtech-mindmapper": ["twd", "twds"],
    "application/vnd.smaf": ["mmf"],
    "application/vnd.smart.teacher": ["teacher"],
    "application/vnd.software602.filler.form+xml": ["fo"],
    "application/vnd.solent.sdkm+xml": ["sdkm", "sdkd"],
    "application/vnd.spotfire.dxp": ["dxp"],
    "application/vnd.spotfire.sfs": ["sfs"],
    "application/vnd.stardivision.calc": ["sdc"],
    "application/vnd.stardivision.draw": ["sda"],
    "application/vnd.stardivision.impress": ["sdd"],
    "application/vnd.stardivision.math": ["smf"],
    "application/vnd.stardivision.writer": ["sdw", "vor"],
    "application/vnd.stardivision.writer-global": ["sgl"],
    "application/vnd.stepmania.package": ["smzip"],
    "application/vnd.stepmania.stepchart": ["sm"],
    "application/vnd.sun.wadl+xml": ["wadl"],
    "application/vnd.sun.xml.calc": ["sxc"],
    "application/vnd.sun.xml.calc.template": ["stc"],
    "application/vnd.sun.xml.draw": ["sxd"],
    "application/vnd.sun.xml.draw.template": ["std"],
    "application/vnd.sun.xml.impress": ["sxi"],
    "application/vnd.sun.xml.impress.template": ["sti"],
    "application/vnd.sun.xml.math": ["sxm"],
    "application/vnd.sun.xml.writer": ["sxw"],
    "application/vnd.sun.xml.writer.global": ["sxg"],
    "application/vnd.sun.xml.writer.template": ["stw"],
    "application/vnd.sus-calendar": ["sus", "susp"],
    "application/vnd.svd": ["svd"],
    "application/vnd.symbian.install": ["sis", "sisx"],
    "application/vnd.syncml+xml": ["xsm"],
    "application/vnd.syncml.dm+wbxml": ["bdm"],
    "application/vnd.syncml.dm+xml": ["xdm"],
    "application/vnd.syncml.dmddf+xml": ["ddf"],
    "application/vnd.tao.intent-module-archive": ["tao"],
    "application/vnd.tcpdump.pcap": ["pcap", "cap", "dmp"],
    "application/vnd.tmobile-livetv": ["tmo"],
    "application/vnd.trid.tpt": ["tpt"],
    "application/vnd.triscape.mxs": ["mxs"],
    "application/vnd.trueapp": ["tra"],
    "application/vnd.ufdl": ["ufd", "ufdl"],
    "application/vnd.uiq.theme": ["utz"],
    "application/vnd.umajin": ["umj"],
    "application/vnd.unity": ["unityweb"],
    "application/vnd.uoml+xml": ["uoml", "uo"],
    "application/vnd.vcx": ["vcx"],
    "application/vnd.visio": ["vsd", "vst", "vss", "vsw", "vsdx", "vtx"],
    "application/vnd.visionary": ["vis"],
    "application/vnd.vsf": ["vsf"],
    "application/vnd.wap.wbxml": ["wbxml"],
    "application/vnd.wap.wmlc": ["wmlc"],
    "application/vnd.wap.wmlscriptc": ["wmlsc"],
    "application/vnd.webturbo": ["wtb"],
    "application/vnd.wolfram.player": ["nbp"],
    "application/vnd.wordperfect": ["wpd"],
    "application/vnd.wqd": ["wqd"],
    "application/vnd.wt.stf": ["stf"],
    "application/vnd.xara": ["xar"],
    "application/vnd.xfdl": ["xfdl"],
    "application/vnd.yamaha.hv-dic": ["hvd"],
    "application/vnd.yamaha.hv-script": ["hvs"],
    "application/vnd.yamaha.hv-voice": ["hvp"],
    "application/vnd.yamaha.openscoreformat": ["osf"],
    "application/vnd.yamaha.openscoreformat.osfpvg+xml": ["osfpvg"],
    "application/vnd.yamaha.smaf-audio": ["saf"],
    "application/vnd.yamaha.smaf-phrase": ["spf"],
    "application/vnd.yellowriver-custom-menu": ["cmp"],
    "application/vnd.zul": ["zir", "zirz"],
    "application/vnd.zzazz.deck+xml": ["zaz"],
    "application/x-7z-compressed": ["7z"],
    "application/x-abiword": ["abw"],
    "application/x-ace-compressed": ["ace"],
    "application/x-apple-diskimage": ["*dmg"],
    "application/x-arj": ["arj"],
    "application/x-authorware-bin": ["aab", "x32", "u32", "vox"],
    "application/x-authorware-map": ["aam"],
    "application/x-authorware-seg": ["aas"],
    "application/x-bcpio": ["bcpio"],
    "application/x-bdoc": ["*bdoc"],
    "application/x-bittorrent": ["torrent"],
    "application/x-blender": ["blend"],
    "application/x-blorb": ["blb", "blorb"],
    "application/x-bzip": ["bz"],
    "application/x-bzip2": ["bz2", "boz"],
    "application/x-cbr": ["cbr", "cba", "cbt", "cbz", "cb7"],
    "application/x-cdlink": ["vcd"],
    "application/x-cfs-compressed": ["cfs"],
    "application/x-chat": ["chat"],
    "application/x-chess-pgn": ["pgn"],
    "application/x-chrome-extension": ["crx"],
    "application/x-cocoa": ["cco"],
    "application/x-compressed": ["*rar"],
    "application/x-conference": ["nsc"],
    "application/x-cpio": ["cpio"],
    "application/x-csh": ["csh"],
    "application/x-debian-package": ["*deb", "udeb"],
    "application/x-dgc-compressed": ["dgc"],
    "application/x-director": [
      "dir",
      "dcr",
      "dxr",
      "cst",
      "cct",
      "cxt",
      "w3d",
      "fgd",
      "swa"
    ],
    "application/x-doom": ["wad"],
    "application/x-dtbncx+xml": ["ncx"],
    "application/x-dtbook+xml": ["dtb"],
    "application/x-dtbresource+xml": ["res"],
    "application/x-dvi": ["dvi"],
    "application/x-envoy": ["evy"],
    "application/x-eva": ["eva"],
    "application/x-font-bdf": ["bdf"],
    "application/x-font-ghostscript": ["gsf"],
    "application/x-font-linux-psf": ["psf"],
    "application/x-font-pcf": ["pcf"],
    "application/x-font-snf": ["snf"],
    "application/x-font-type1": ["pfa", "pfb", "pfm", "afm"],
    "application/x-freearc": ["arc"],
    "application/x-futuresplash": ["spl"],
    "application/x-gca-compressed": ["gca"],
    "application/x-glulx": ["ulx"],
    "application/x-gnumeric": ["gnumeric"],
    "application/x-gramps-xml": ["gramps"],
    "application/x-gtar": ["gtar"],
    "application/x-hdf": ["hdf"],
    "application/x-httpd-php": ["php"],
    "application/x-install-instructions": ["install"],
    "application/x-ipynb+json": ["ipynb"],
    "application/x-iso9660-image": ["*iso"],
    "application/x-iwork-keynote-sffkey": ["*key"],
    "application/x-iwork-numbers-sffnumbers": ["*numbers"],
    "application/x-iwork-pages-sffpages": ["*pages"],
    "application/x-java-archive-diff": ["jardiff"],
    "application/x-java-jnlp-file": ["jnlp"],
    "application/x-keepass2": ["kdbx"],
    "application/x-latex": ["latex"],
    "application/x-lua-bytecode": ["luac"],
    "application/x-lzh-compressed": ["lzh", "lha"],
    "application/x-makeself": ["run"],
    "application/x-mie": ["mie"],
    "application/x-mobipocket-ebook": ["*prc", "mobi"],
    "application/x-ms-application": ["application"],
    "application/x-ms-shortcut": ["lnk"],
    "application/x-ms-wmd": ["wmd"],
    "application/x-ms-wmz": ["wmz"],
    "application/x-ms-xbap": ["xbap"],
    "application/x-msaccess": ["mdb"],
    "application/x-msbinder": ["obd"],
    "application/x-mscardfile": ["crd"],
    "application/x-msclip": ["clp"],
    "application/x-msdos-program": ["*exe"],
    "application/x-msdownload": ["*exe", "*dll", "com", "bat", "*msi"],
    "application/x-msmediaview": ["mvb", "m13", "m14"],
    "application/x-msmetafile": ["*wmf", "*wmz", "*emf", "emz"],
    "application/x-msmoney": ["mny"],
    "application/x-mspublisher": ["pub"],
    "application/x-msschedule": ["scd"],
    "application/x-msterminal": ["trm"],
    "application/x-mswrite": ["wri"],
    "application/x-netcdf": ["nc", "cdf"],
    "application/x-ns-proxy-autoconfig": ["pac"],
    "application/x-nzb": ["nzb"],
    "application/x-perl": ["pl", "pm"],
    "application/x-pilot": ["*prc", "*pdb"],
    "application/x-pkcs12": ["p12", "pfx"],
    "application/x-pkcs7-certificates": ["p7b", "spc"],
    "application/x-pkcs7-certreqresp": ["p7r"],
    "application/x-rar-compressed": ["*rar"],
    "application/x-redhat-package-manager": ["rpm"],
    "application/x-research-info-systems": ["ris"],
    "application/x-sea": ["sea"],
    "application/x-sh": ["sh"],
    "application/x-shar": ["shar"],
    "application/x-shockwave-flash": ["swf"],
    "application/x-silverlight-app": ["xap"],
    "application/x-sql": ["*sql"],
    "application/x-stuffit": ["sit"],
    "application/x-stuffitx": ["sitx"],
    "application/x-subrip": ["srt"],
    "application/x-sv4cpio": ["sv4cpio"],
    "application/x-sv4crc": ["sv4crc"],
    "application/x-t3vm-image": ["t3"],
    "application/x-tads": ["gam"],
    "application/x-tar": ["tar"],
    "application/x-tcl": ["tcl", "tk"],
    "application/x-tex": ["tex"],
    "application/x-tex-tfm": ["tfm"],
    "application/x-texinfo": ["texinfo", "texi"],
    "application/x-tgif": ["*obj"],
    "application/x-ustar": ["ustar"],
    "application/x-virtualbox-hdd": ["hdd"],
    "application/x-virtualbox-ova": ["ova"],
    "application/x-virtualbox-ovf": ["ovf"],
    "application/x-virtualbox-vbox": ["vbox"],
    "application/x-virtualbox-vbox-extpack": ["vbox-extpack"],
    "application/x-virtualbox-vdi": ["vdi"],
    "application/x-virtualbox-vhd": ["vhd"],
    "application/x-virtualbox-vmdk": ["vmdk"],
    "application/x-wais-source": ["src"],
    "application/x-web-app-manifest+json": ["webapp"],
    "application/x-x509-ca-cert": ["der", "crt", "pem"],
    "application/x-xfig": ["fig"],
    "application/x-xliff+xml": ["*xlf"],
    "application/x-xpinstall": ["xpi"],
    "application/x-xz": ["xz"],
    "application/x-zip-compressed": ["*zip"],
    "application/x-zmachine": ["z1", "z2", "z3", "z4", "z5", "z6", "z7", "z8"],
    "audio/vnd.dece.audio": ["uva", "uvva"],
    "audio/vnd.digital-winds": ["eol"],
    "audio/vnd.dra": ["dra"],
    "audio/vnd.dts": ["dts"],
    "audio/vnd.dts.hd": ["dtshd"],
    "audio/vnd.lucent.voice": ["lvp"],
    "audio/vnd.ms-playready.media.pya": ["pya"],
    "audio/vnd.nuera.ecelp4800": ["ecelp4800"],
    "audio/vnd.nuera.ecelp7470": ["ecelp7470"],
    "audio/vnd.nuera.ecelp9600": ["ecelp9600"],
    "audio/vnd.rip": ["rip"],
    "audio/x-aac": ["*aac"],
    "audio/x-aiff": ["aif", "aiff", "aifc"],
    "audio/x-caf": ["caf"],
    "audio/x-flac": ["flac"],
    "audio/x-m4a": ["*m4a"],
    "audio/x-matroska": ["mka"],
    "audio/x-mpegurl": ["m3u"],
    "audio/x-ms-wax": ["wax"],
    "audio/x-ms-wma": ["wma"],
    "audio/x-pn-realaudio": ["ram", "ra"],
    "audio/x-pn-realaudio-plugin": ["rmp"],
    "audio/x-realaudio": ["*ra"],
    "audio/x-wav": ["*wav"],
    "chemical/x-cdx": ["cdx"],
    "chemical/x-cif": ["cif"],
    "chemical/x-cmdf": ["cmdf"],
    "chemical/x-cml": ["cml"],
    "chemical/x-csml": ["csml"],
    "chemical/x-xyz": ["xyz"],
    "image/prs.btif": ["btif", "btf"],
    "image/prs.pti": ["pti"],
    "image/vnd.adobe.photoshop": ["psd"],
    "image/vnd.airzip.accelerator.azv": ["azv"],
    "image/vnd.blockfact.facti": ["facti"],
    "image/vnd.dece.graphic": ["uvi", "uvvi", "uvg", "uvvg"],
    "image/vnd.djvu": ["djvu", "djv"],
    "image/vnd.dvb.subtitle": ["*sub"],
    "image/vnd.dwg": ["dwg"],
    "image/vnd.dxf": ["dxf"],
    "image/vnd.fastbidsheet": ["fbs"],
    "image/vnd.fpx": ["fpx"],
    "image/vnd.fst": ["fst"],
    "image/vnd.fujixerox.edmics-mmr": ["mmr"],
    "image/vnd.fujixerox.edmics-rlc": ["rlc"],
    "image/vnd.microsoft.icon": ["ico"],
    "image/vnd.ms-dds": ["dds"],
    "image/vnd.ms-modi": ["mdi"],
    "image/vnd.ms-photo": ["wdp"],
    "image/vnd.net-fpx": ["npx"],
    "image/vnd.pco.b16": ["b16"],
    "image/vnd.tencent.tap": ["tap"],
    "image/vnd.valve.source.texture": ["vtf"],
    "image/vnd.wap.wbmp": ["wbmp"],
    "image/vnd.xiff": ["xif"],
    "image/vnd.zbrush.pcx": ["pcx"],
    "image/x-3ds": ["3ds"],
    "image/x-adobe-dng": ["dng"],
    "image/x-cmu-raster": ["ras"],
    "image/x-cmx": ["cmx"],
    "image/x-freehand": ["fh", "fhc", "fh4", "fh5", "fh7"],
    "image/x-icon": ["*ico"],
    "image/x-jng": ["jng"],
    "image/x-mrsid-image": ["sid"],
    "image/x-ms-bmp": ["*bmp"],
    "image/x-pcx": ["*pcx"],
    "image/x-pict": ["pic", "pct"],
    "image/x-portable-anymap": ["pnm"],
    "image/x-portable-bitmap": ["pbm"],
    "image/x-portable-graymap": ["pgm"],
    "image/x-portable-pixmap": ["ppm"],
    "image/x-rgb": ["rgb"],
    "image/x-tga": ["tga"],
    "image/x-xbitmap": ["xbm"],
    "image/x-xpixmap": ["xpm"],
    "image/x-xwindowdump": ["xwd"],
    "message/vnd.wfa.wsc": ["wsc"],
    "model/vnd.bary": ["bary"],
    "model/vnd.cld": ["cld"],
    "model/vnd.collada+xml": ["dae"],
    "model/vnd.dwf": ["dwf"],
    "model/vnd.gdl": ["gdl"],
    "model/vnd.gtw": ["gtw"],
    "model/vnd.mts": ["*mts"],
    "model/vnd.opengex": ["ogex"],
    "model/vnd.parasolid.transmit.binary": ["x_b"],
    "model/vnd.parasolid.transmit.text": ["x_t"],
    "model/vnd.pytha.pyox": ["pyo", "pyox"],
    "model/vnd.sap.vds": ["vds"],
    "model/vnd.usda": ["usda"],
    "model/vnd.usdz+zip": ["usdz"],
    "model/vnd.valve.source.compiled-map": ["bsp"],
    "model/vnd.vtu": ["vtu"],
    "text/prs.lines.tag": ["dsc"],
    "text/vnd.curl": ["curl"],
    "text/vnd.curl.dcurl": ["dcurl"],
    "text/vnd.curl.mcurl": ["mcurl"],
    "text/vnd.curl.scurl": ["scurl"],
    "text/vnd.dvb.subtitle": ["sub"],
    "text/vnd.familysearch.gedcom": ["ged"],
    "text/vnd.fly": ["fly"],
    "text/vnd.fmi.flexstor": ["flx"],
    "text/vnd.graphviz": ["gv"],
    "text/vnd.in3d.3dml": ["3dml"],
    "text/vnd.in3d.spot": ["spot"],
    "text/vnd.sun.j2me.app-descriptor": ["jad"],
    "text/vnd.wap.wml": ["wml"],
    "text/vnd.wap.wmlscript": ["wmls"],
    "text/x-asm": ["s", "asm"],
    "text/x-c": ["c", "cc", "cxx", "cpp", "h", "hh", "dic"],
    "text/x-component": ["htc"],
    "text/x-fortran": ["f", "for", "f77", "f90"],
    "text/x-handlebars-template": ["hbs"],
    "text/x-java-source": ["java"],
    "text/x-lua": ["lua"],
    "text/x-markdown": ["mkd"],
    "text/x-nfo": ["nfo"],
    "text/x-opml": ["opml"],
    "text/x-org": ["*org"],
    "text/x-pascal": ["p", "pas"],
    "text/x-processing": ["pde"],
    "text/x-sass": ["sass"],
    "text/x-scss": ["scss"],
    "text/x-setext": ["etx"],
    "text/x-sfv": ["sfv"],
    "text/x-suse-ymp": ["ymp"],
    "text/x-uuencode": ["uu"],
    "text/x-vcalendar": ["vcs"],
    "text/x-vcard": ["vcf"],
    "video/vnd.dece.hd": ["uvh", "uvvh"],
    "video/vnd.dece.mobile": ["uvm", "uvvm"],
    "video/vnd.dece.pd": ["uvp", "uvvp"],
    "video/vnd.dece.sd": ["uvs", "uvvs"],
    "video/vnd.dece.video": ["uvv", "uvvv"],
    "video/vnd.dvb.file": ["dvb"],
    "video/vnd.fvt": ["fvt"],
    "video/vnd.mpegurl": ["mxu", "m4u"],
    "video/vnd.ms-playready.media.pyv": ["pyv"],
    "video/vnd.uvvu.mp4": ["uvu", "uvvu"],
    "video/vnd.vivo": ["viv"],
    "video/x-f4v": ["f4v"],
    "video/x-fli": ["fli"],
    "video/x-flv": ["flv"],
    "video/x-m4v": ["m4v"],
    "video/x-matroska": ["mkv", "mk3d", "mks"],
    "video/x-mng": ["mng"],
    "video/x-ms-asf": ["asf", "asx"],
    "video/x-ms-vob": ["vob"],
    "video/x-ms-wm": ["wm"],
    "video/x-ms-wmv": ["wmv"],
    "video/x-ms-wmx": ["wmx"],
    "video/x-ms-wvx": ["wvx"],
    "video/x-msvideo": ["avi"],
    "video/x-sgi-movie": ["movie"],
    "video/x-smv": ["smv"],
    "x-conference/x-cooltalk": ["ice"]
  };
  Object.freeze(types);
  var other_default = types;

  // crates/neo-sdk/neo-playwright/third_party/mime/types/standard.ts
  var types2 = {
    "application/andrew-inset": ["ez"],
    "application/appinstaller": ["appinstaller"],
    "application/applixware": ["aw"],
    "application/appx": ["appx"],
    "application/appxbundle": ["appxbundle"],
    "application/atom+xml": ["atom"],
    "application/atomcat+xml": ["atomcat"],
    "application/atomdeleted+xml": ["atomdeleted"],
    "application/atomsvc+xml": ["atomsvc"],
    "application/atsc-dwd+xml": ["dwd"],
    "application/atsc-held+xml": ["held"],
    "application/atsc-rsat+xml": ["rsat"],
    "application/automationml-aml+xml": ["aml"],
    "application/automationml-amlx+zip": ["amlx"],
    "application/bdoc": ["bdoc"],
    "application/calendar+xml": ["xcs"],
    "application/ccxml+xml": ["ccxml"],
    "application/cdfx+xml": ["cdfx"],
    "application/cdmi-capability": ["cdmia"],
    "application/cdmi-container": ["cdmic"],
    "application/cdmi-domain": ["cdmid"],
    "application/cdmi-object": ["cdmio"],
    "application/cdmi-queue": ["cdmiq"],
    "application/cpl+xml": ["cpl"],
    "application/cu-seeme": ["cu"],
    "application/cwl": ["cwl"],
    "application/dash+xml": ["mpd"],
    "application/dash-patch+xml": ["mpp"],
    "application/davmount+xml": ["davmount"],
    "application/dicom": ["dcm"],
    "application/docbook+xml": ["dbk"],
    "application/dssc+der": ["dssc"],
    "application/dssc+xml": ["xdssc"],
    "application/ecmascript": ["ecma"],
    "application/emma+xml": ["emma"],
    "application/emotionml+xml": ["emotionml"],
    "application/epub+zip": ["epub"],
    "application/exi": ["exi"],
    "application/express": ["exp"],
    "application/fdf": ["fdf"],
    "application/fdt+xml": ["fdt"],
    "application/font-tdpfr": ["pfr"],
    "application/geo+json": ["geojson"],
    "application/gml+xml": ["gml"],
    "application/gpx+xml": ["gpx"],
    "application/gxf": ["gxf"],
    "application/gzip": ["gz"],
    "application/hjson": ["hjson"],
    "application/hyperstudio": ["stk"],
    "application/inkml+xml": ["ink", "inkml"],
    "application/ipfix": ["ipfix"],
    "application/its+xml": ["its"],
    "application/java-archive": ["jar", "war", "ear"],
    "application/java-serialized-object": ["ser"],
    "application/java-vm": ["class"],
    "application/javascript": ["*js"],
    "application/json": ["json", "map"],
    "application/json5": ["json5"],
    "application/jsonml+json": ["jsonml"],
    "application/ld+json": ["jsonld"],
    "application/lgr+xml": ["lgr"],
    "application/lost+xml": ["lostxml"],
    "application/mac-binhex40": ["hqx"],
    "application/mac-compactpro": ["cpt"],
    "application/mads+xml": ["mads"],
    "application/manifest+json": ["webmanifest"],
    "application/marc": ["mrc"],
    "application/marcxml+xml": ["mrcx"],
    "application/mathematica": ["ma", "nb", "mb"],
    "application/mathml+xml": ["mathml"],
    "application/mbox": ["mbox"],
    "application/media-policy-dataset+xml": ["mpf"],
    "application/mediaservercontrol+xml": ["mscml"],
    "application/metalink+xml": ["metalink"],
    "application/metalink4+xml": ["meta4"],
    "application/mets+xml": ["mets"],
    "application/mmt-aei+xml": ["maei"],
    "application/mmt-usd+xml": ["musd"],
    "application/mods+xml": ["mods"],
    "application/mp21": ["m21", "mp21"],
    "application/mp4": ["*mp4", "*mpg4", "mp4s", "m4p"],
    "application/msix": ["msix"],
    "application/msixbundle": ["msixbundle"],
    "application/msword": ["doc", "dot"],
    "application/mxf": ["mxf"],
    "application/n-quads": ["nq"],
    "application/n-triples": ["nt"],
    "application/node": ["cjs"],
    "application/octet-stream": [
      "bin",
      "dms",
      "lrf",
      "mar",
      "so",
      "dist",
      "distz",
      "pkg",
      "bpk",
      "dump",
      "elc",
      "deploy",
      "exe",
      "dll",
      "deb",
      "dmg",
      "iso",
      "img",
      "msi",
      "msp",
      "msm",
      "buffer"
    ],
    "application/oda": ["oda"],
    "application/oebps-package+xml": ["opf"],
    "application/ogg": ["ogx"],
    "application/omdoc+xml": ["omdoc"],
    "application/onenote": [
      "onetoc",
      "onetoc2",
      "onetmp",
      "onepkg",
      "one",
      "onea"
    ],
    "application/oxps": ["oxps"],
    "application/p2p-overlay+xml": ["relo"],
    "application/patch-ops-error+xml": ["xer"],
    "application/pdf": ["pdf"],
    "application/pgp-encrypted": ["pgp"],
    "application/pgp-keys": ["asc"],
    "application/pgp-signature": ["sig", "*asc"],
    "application/pics-rules": ["prf"],
    "application/pkcs10": ["p10"],
    "application/pkcs7-mime": ["p7m", "p7c"],
    "application/pkcs7-signature": ["p7s"],
    "application/pkcs8": ["p8"],
    "application/pkix-attr-cert": ["ac"],
    "application/pkix-cert": ["cer"],
    "application/pkix-crl": ["crl"],
    "application/pkix-pkipath": ["pkipath"],
    "application/pkixcmp": ["pki"],
    "application/pls+xml": ["pls"],
    "application/postscript": ["ai", "eps", "ps"],
    "application/provenance+xml": ["provx"],
    "application/pskc+xml": ["pskcxml"],
    "application/raml+yaml": ["raml"],
    "application/rdf+xml": ["rdf", "owl"],
    "application/reginfo+xml": ["rif"],
    "application/relax-ng-compact-syntax": ["rnc"],
    "application/resource-lists+xml": ["rl"],
    "application/resource-lists-diff+xml": ["rld"],
    "application/rls-services+xml": ["rs"],
    "application/route-apd+xml": ["rapd"],
    "application/route-s-tsid+xml": ["sls"],
    "application/route-usd+xml": ["rusd"],
    "application/rpki-ghostbusters": ["gbr"],
    "application/rpki-manifest": ["mft"],
    "application/rpki-roa": ["roa"],
    "application/rsd+xml": ["rsd"],
    "application/rss+xml": ["rss"],
    "application/rtf": ["rtf"],
    "application/sbml+xml": ["sbml"],
    "application/scvp-cv-request": ["scq"],
    "application/scvp-cv-response": ["scs"],
    "application/scvp-vp-request": ["spq"],
    "application/scvp-vp-response": ["spp"],
    "application/sdp": ["sdp"],
    "application/senml+xml": ["senmlx"],
    "application/sensml+xml": ["sensmlx"],
    "application/set-payment-initiation": ["setpay"],
    "application/set-registration-initiation": ["setreg"],
    "application/shf+xml": ["shf"],
    "application/sieve": ["siv", "sieve"],
    "application/smil+xml": ["smi", "smil"],
    "application/sparql-query": ["rq"],
    "application/sparql-results+xml": ["srx"],
    "application/sql": ["sql"],
    "application/srgs": ["gram"],
    "application/srgs+xml": ["grxml"],
    "application/sru+xml": ["sru"],
    "application/ssdl+xml": ["ssdl"],
    "application/ssml+xml": ["ssml"],
    "application/swid+xml": ["swidtag"],
    "application/tei+xml": ["tei", "teicorpus"],
    "application/thraud+xml": ["tfi"],
    "application/timestamped-data": ["tsd"],
    "application/toml": ["toml"],
    "application/trig": ["trig"],
    "application/ttml+xml": ["ttml"],
    "application/ubjson": ["ubj"],
    "application/urc-ressheet+xml": ["rsheet"],
    "application/urc-targetdesc+xml": ["td"],
    "application/voicexml+xml": ["vxml"],
    "application/wasm": ["wasm"],
    "application/watcherinfo+xml": ["wif"],
    "application/widget": ["wgt"],
    "application/winhlp": ["hlp"],
    "application/wsdl+xml": ["wsdl"],
    "application/wspolicy+xml": ["wspolicy"],
    "application/xaml+xml": ["xaml"],
    "application/xcap-att+xml": ["xav"],
    "application/xcap-caps+xml": ["xca"],
    "application/xcap-diff+xml": ["xdf"],
    "application/xcap-el+xml": ["xel"],
    "application/xcap-ns+xml": ["xns"],
    "application/xenc+xml": ["xenc"],
    "application/xfdf": ["xfdf"],
    "application/xhtml+xml": ["xhtml", "xht"],
    "application/xliff+xml": ["xlf"],
    "application/xml": ["xml", "xsl", "xsd", "rng"],
    "application/xml-dtd": ["dtd"],
    "application/xop+xml": ["xop"],
    "application/xproc+xml": ["xpl"],
    "application/xslt+xml": ["*xsl", "xslt"],
    "application/xspf+xml": ["xspf"],
    "application/xv+xml": ["mxml", "xhvml", "xvml", "xvm"],
    "application/yang": ["yang"],
    "application/yin+xml": ["yin"],
    "application/zip": ["zip"],
    "application/zip+dotlottie": ["lottie"],
    "audio/3gpp": ["*3gpp"],
    "audio/aac": ["adts", "aac"],
    "audio/adpcm": ["adp"],
    "audio/amr": ["amr"],
    "audio/basic": ["au", "snd"],
    "audio/midi": ["mid", "midi", "kar", "rmi"],
    "audio/mobile-xmf": ["mxmf"],
    "audio/mp3": ["*mp3"],
    "audio/mp4": ["m4a", "mp4a", "m4b"],
    "audio/mpeg": ["mpga", "mp2", "mp2a", "mp3", "m2a", "m3a"],
    "audio/ogg": ["oga", "ogg", "spx", "opus"],
    "audio/s3m": ["s3m"],
    "audio/silk": ["sil"],
    "audio/wav": ["wav"],
    "audio/wave": ["*wav"],
    "audio/webm": ["weba"],
    "audio/xm": ["xm"],
    "font/collection": ["ttc"],
    "font/otf": ["otf"],
    "font/ttf": ["ttf"],
    "font/woff": ["woff"],
    "font/woff2": ["woff2"],
    "image/aces": ["exr"],
    "image/apng": ["apng"],
    "image/avci": ["avci"],
    "image/avcs": ["avcs"],
    "image/avif": ["avif"],
    "image/bmp": ["bmp", "dib"],
    "image/cgm": ["cgm"],
    "image/dicom-rle": ["drle"],
    "image/dpx": ["dpx"],
    "image/emf": ["emf"],
    "image/fits": ["fits"],
    "image/g3fax": ["g3"],
    "image/gif": ["gif"],
    "image/heic": ["heic"],
    "image/heic-sequence": ["heics"],
    "image/heif": ["heif"],
    "image/heif-sequence": ["heifs"],
    "image/hej2k": ["hej2"],
    "image/ief": ["ief"],
    "image/jaii": ["jaii"],
    "image/jais": ["jais"],
    "image/jls": ["jls"],
    "image/jp2": ["jp2", "jpg2"],
    "image/jpeg": ["jpg", "jpeg", "jpe"],
    "image/jph": ["jph"],
    "image/jphc": ["jhc"],
    "image/jpm": ["jpm", "jpgm"],
    "image/jpx": ["jpx", "jpf"],
    "image/jxl": ["jxl"],
    "image/jxr": ["jxr"],
    "image/jxra": ["jxra"],
    "image/jxrs": ["jxrs"],
    "image/jxs": ["jxs"],
    "image/jxsc": ["jxsc"],
    "image/jxsi": ["jxsi"],
    "image/jxss": ["jxss"],
    "image/ktx": ["ktx"],
    "image/ktx2": ["ktx2"],
    "image/pjpeg": ["jfif"],
    "image/png": ["png"],
    "image/sgi": ["sgi"],
    "image/svg+xml": ["svg", "svgz"],
    "image/t38": ["t38"],
    "image/tiff": ["tif", "tiff"],
    "image/tiff-fx": ["tfx"],
    "image/webp": ["webp"],
    "image/wmf": ["wmf"],
    "message/disposition-notification": ["disposition-notification"],
    "message/global": ["u8msg"],
    "message/global-delivery-status": ["u8dsn"],
    "message/global-disposition-notification": ["u8mdn"],
    "message/global-headers": ["u8hdr"],
    "message/rfc822": ["eml", "mime", "mht", "mhtml"],
    "model/3mf": ["3mf"],
    "model/gltf+json": ["gltf"],
    "model/gltf-binary": ["glb"],
    "model/iges": ["igs", "iges"],
    "model/jt": ["jt"],
    "model/mesh": ["msh", "mesh", "silo"],
    "model/mtl": ["mtl"],
    "model/obj": ["obj"],
    "model/prc": ["prc"],
    "model/step": ["step", "stp", "stpnc", "p21", "210"],
    "model/step+xml": ["stpx"],
    "model/step+zip": ["stpz"],
    "model/step-xml+zip": ["stpxz"],
    "model/stl": ["stl"],
    "model/u3d": ["u3d"],
    "model/vrml": ["wrl", "vrml"],
    "model/x3d+binary": ["*x3db", "x3dbz"],
    "model/x3d+fastinfoset": ["x3db"],
    "model/x3d+vrml": ["*x3dv", "x3dvz"],
    "model/x3d+xml": ["x3d", "x3dz"],
    "model/x3d-vrml": ["x3dv"],
    "text/cache-manifest": ["appcache", "manifest"],
    "text/calendar": ["ics", "ifb"],
    "text/coffeescript": ["coffee", "litcoffee"],
    "text/css": ["css"],
    "text/csv": ["csv"],
    "text/html": ["html", "htm", "shtml"],
    "text/jade": ["jade"],
    "text/javascript": ["js", "mjs"],
    "text/jsx": ["jsx"],
    "text/less": ["less"],
    "text/markdown": ["md", "markdown"],
    "text/mathml": ["mml"],
    "text/mdx": ["mdx"],
    "text/n3": ["n3"],
    "text/plain": ["txt", "text", "conf", "def", "list", "log", "in", "ini"],
    "text/richtext": ["rtx"],
    "text/rtf": ["*rtf"],
    "text/sgml": ["sgml", "sgm"],
    "text/shex": ["shex"],
    "text/slim": ["slim", "slm"],
    "text/spdx": ["spdx"],
    "text/stylus": ["stylus", "styl"],
    "text/tab-separated-values": ["tsv"],
    "text/troff": ["t", "tr", "roff", "man", "me", "ms"],
    "text/turtle": ["ttl"],
    "text/uri-list": ["uri", "uris", "urls"],
    "text/vcard": ["vcard"],
    "text/vtt": ["vtt"],
    "text/wgsl": ["wgsl"],
    "text/xml": ["*xml"],
    "text/yaml": ["yaml", "yml"],
    "video/3gpp": ["3gp", "3gpp"],
    "video/3gpp2": ["3g2"],
    "video/h261": ["h261"],
    "video/h263": ["h263"],
    "video/h264": ["h264"],
    "video/iso.segment": ["m4s"],
    "video/jpeg": ["jpgv"],
    "video/jpm": ["*jpm", "*jpgm"],
    "video/mj2": ["mj2", "mjp2"],
    "video/mp2t": ["ts", "m2t", "m2ts", "mts"],
    "video/mp4": ["mp4", "mp4v", "mpg4"],
    "video/mpeg": ["mpeg", "mpg", "mpe", "m1v", "m2v"],
    "video/ogg": ["ogv"],
    "video/quicktime": ["qt", "mov"],
    "video/webm": ["webm"]
  };
  Object.freeze(types2);
  var standard_default = types2;

  // crates/neo-sdk/neo-playwright/third_party/mime/src/Mime.ts
  class Mime {
    #extensionToType = new Map;
    #typeToExtension = new Map;
    #typeToExtensions = new Map;
    constructor(...args) {
      for (const arg of args) {
        this.define(arg);
      }
    }
    define(typeMap, force = false) {
      for (let [type, extensions] of Object.entries(typeMap)) {
        type = type.toLowerCase();
        extensions = extensions.map((ext) => ext.toLowerCase());
        if (!this.#typeToExtensions.has(type)) {
          this.#typeToExtensions.set(type, new Set);
        }
        const allExtensions = this.#typeToExtensions.get(type);
        let first = true;
        for (let extension of extensions) {
          const starred = extension.startsWith("*");
          extension = starred ? extension.slice(1) : extension;
          allExtensions?.add(extension);
          if (first) {
            this.#typeToExtension.set(type, extension);
          }
          first = false;
          if (starred)
            continue;
          const currentType = this.#extensionToType.get(extension);
          if (currentType && currentType != type && !force) {
            throw new Error(`"${type} -> ${extension}" conflicts with "${currentType} -> ${extension}". Pass \`force=true\` to override this definition.`);
          }
          this.#extensionToType.set(extension, type);
        }
      }
      return this;
    }
    getType(path) {
      if (typeof path !== "string")
        return null;
      const last = path.replace(/^.*[/\\]/s, "").toLowerCase();
      const ext = last.replace(/^.*\./s, "").toLowerCase();
      const hasPath = last.length < path.length;
      const hasDot = ext.length < last.length - 1;
      if (!hasDot && hasPath)
        return null;
      return this.#extensionToType.get(ext) ?? null;
    }
    getExtension(type) {
      if (typeof type !== "string")
        return null;
      type = type?.split?.(";")[0];
      return (type && this.#typeToExtension.get(type.trim().toLowerCase())) ?? null;
    }
    getAllExtensions(type) {
      if (typeof type !== "string")
        return null;
      return this.#typeToExtensions.get(type.toLowerCase()) ?? null;
    }
    _freeze() {
      this.define = () => {
        throw new Error("define() not allowed for built-in Mime objects. See https://github.com/broofa/mime/blob/main/README.md#custom-mime-instances");
      };
      Object.freeze(this);
      for (const extensions of this.#typeToExtensions.values()) {
        Object.freeze(extensions);
      }
      return this;
    }
    _getTestState() {
      return {
        types: this.#extensionToType,
        extensions: this.#typeToExtension
      };
    }
  }

  // crates/neo-sdk/neo-playwright/third_party/mime/src/index.ts
  var src_default = new Mime(standard_default, other_default)._freeze();

  // crates/neo-sdk/neo-playwright-js/src/prelude/files.ts
  var UPLOAD_LIMIT = 50 * 1024 * 1024;
  var NEO_UPLOAD_LIMIT = 10 * 1024 * 1024;
  function inputFiles(api, files) {
    const items = Array.isArray(files) ? files.slice() : [files];
    if (items.some((item) => typeof item === "string")) {
      const paths = items.filter((item) => typeof item === "string");
      if (paths.length !== items.length)
        throw unprefixedClientError(api, "File paths cannot be mixed with buffers");
      return { paths };
    }
    const total = payloadBytes(api, items);
    if (total >= UPLOAD_LIMIT)
      throw unprefixedClientError(api, "Cannot set buffer larger than 50Mb, please write it to a file and pass its path instead.");
    const checked = items.map((item, index) => checkedPayload(api, item, index));
    if (total >= NEO_UPLOAD_LIMIT)
      throw unprefixedClientError(api, "Cannot set buffer larger than 10Mb in Neo, please write it to a file and pass its path instead.");
    return {
      payloads: checked.map(({ name, mimeType, buffer }) => ({
        name,
        mimeType: mimeType || src_default.getType(name) || "application/octet-stream",
        buffer: String(Reflect.apply(Reflect.get(buffer, "toString"), buffer, ["base64"]))
      }))
    };
  }
  function dropFiles(api, files) {
    const input = inputFiles(api, files);
    if ("payloads" in input)
      return input;
    return {
      paths: input.paths.map((path) => ({
        path,
        mimeType: src_default.getType(path) || "application/octet-stream"
      }))
    };
  }
  function payloadBytes(api, items) {
    let total = 0;
    for (const item of items) {
      if (item === null || item === undefined)
        throw unprefixedClientError(api, `Cannot read properties of ${item} (reading 'buffer')`, "TypeError");
      const buffer = Reflect.get(Object(item), "buffer");
      if (!buffer)
        continue;
      const length = Reflect.get(Object(buffer), "byteLength");
      total += length;
    }
    return total;
  }
  function checkedPayload(api, item, index) {
    const at = `payloads[${index}]`;
    if (item === null)
      throw clientError(api, `${at}: expected object, got null`);
    if (typeof item !== "object")
      throw clientError(api, `${at}: expected object, got ${typeof item}`);
    const name = validString(api, `${at}.name`, Reflect.get(item, "name"));
    const rawType = Reflect.get(item, "mimeType");
    const mimeType = rawType === undefined ? undefined : validString(api, `${at}.mimeType`, rawType);
    const buffer = Reflect.get(item, "buffer");
    if (!(buffer instanceof Buffer) && !(buffer instanceof Object))
      throw clientError(api, `${at}.buffer: expected Buffer, got ${typeof buffer}`);
    return mimeType === undefined ? { name, buffer } : { name, mimeType, buffer };
  }
  function validString(api, path, value) {
    if (value instanceof String)
      return value.valueOf();
    if (typeof value === "string")
      return value;
    throw clientError(api, `${path}: expected string, got ${typeof value}`);
  }
  var CLIENT_MESSAGES = [
    /^Multiple directories are not supported$/,
    /^Dropping a directory is not supported/,
    /^File paths must be all files or a single directory$/,
    /^(?:E[A-Z0-9]+|UNKNOWN): [^,\n]+, stat '/
  ];
  var NUL_PATH = /^The argument 'path' must be a string, Uint8Array, or URL without null bytes\. Received /;
  async function withClientErrors(api, call) {
    try {
      return await call();
    } catch (error) {
      if (!(error instanceof Error) || !error.message.startsWith(`${api}: `))
        throw error;
      const bare = error.message.slice(api.length + 2);
      if (NUL_PATH.test(bare)) {
        error.message = bare;
        error.name = "TypeError";
      } else if (CLIENT_MESSAGES.some((message) => message.test(bare)))
        error.message = bare;
      throw error;
    }
  }

  // crates/neo-sdk/neo-playwright/third_party/playwright/packages/isomorphic/stringUtils.ts
  function escapeWithQuotes(text, char = "'") {
    const stringified = JSON.stringify(text);
    const escapedText = stringified.substring(1, stringified.length - 1).replace(/\\"/g, '"');
    if (char === "'")
      return char + escapedText.replace(/[']/g, "\\'") + char;
    if (char === '"')
      return char + escapedText.replace(/["]/g, "\\\"") + char;
    if (char === "`")
      return char + escapedText.replace(/[`]/g, "\\`") + char;
    throw new Error("Invalid escape char");
  }
  function toTitleCase(name) {
    return name.charAt(0).toUpperCase() + name.substring(1);
  }
  function toSnakeCase(name) {
    return name.replace(/([a-z0-9])([A-Z])/g, "$1_$2").replace(/([A-Z])([A-Z][a-z])/g, "$1_$2").toLowerCase();
  }
  function normalizeEscapedRegexQuotes(source) {
    return source.replace(/(^|[^\\])(\\\\)*\\(['"`])/g, "$1$2$3");
  }
  function escapeRegexForSelector(re) {
    if (re.unicode || re.unicodeSets)
      return String(re);
    return String(re).replace(/(^|[^\\])(\\\\)*(["'`])/g, "$1$2\\$3").replace(/>>/g, "\\>\\>");
  }
  function escapeForTextSelector(text, exact) {
    if (typeof text !== "string")
      return escapeRegexForSelector(text);
    return `${JSON.stringify(text)}${exact ? "s" : "i"}`;
  }
  function escapeForAttributeSelector(value, exact) {
    if (typeof value !== "string")
      return escapeRegexForSelector(value);
    return `"${value.replace(/\\/g, "\\\\").replace(/["]/g, "\\\"")}"${exact ? "s" : "i"}`;
  }
  function escapeRegExp(s) {
    return s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  }
  function longestCommonSubstring(s1, s2) {
    const n = s1.length;
    const m = s2.length;
    let maxLen = 0;
    let endingIndex = 0;
    const dp = Array(n + 1).fill(null).map(() => Array(m + 1).fill(0));
    for (let i = 1;i <= n; i++) {
      for (let j = 1;j <= m; j++) {
        if (s1[i - 1] === s2[j - 1]) {
          dp[i][j] = dp[i - 1][j - 1] + 1;
          if (dp[i][j] > maxLen) {
            maxLen = dp[i][j];
            endingIndex = i;
          }
        }
      }
    }
    return s1.slice(endingIndex - maxLen, endingIndex);
  }
  var ansiRegex = new RegExp("([\\u001B\\u009B][[\\]()#?]*(?:(?:(?:[a-zA-Z\\d]*(?:;[-a-zA-Z\\d\\/#&.:=?%@~_]*)*)?\\u0007)|(?:(?:\\d{0,4}(?:;\\d{0,4})*)?[\\dA-PR-TZcf-ntqry=><~])))", "g");

  // crates/neo-sdk/neo-playwright/third_party/playwright/packages/isomorphic/yaml.ts
  function yamlEscapeKeyIfNeeded(str) {
    if (!yamlStringNeedsQuotes(str))
      return str;
    return `'` + str.replace(/'/g, `''`) + `'`;
  }
  function yamlEscapeValueIfNeeded(str) {
    if (!yamlStringNeedsQuotes(str))
      return str;
    return '"' + str.replace(/[\\"\x00-\x1f\x7f-\x9f]/g, (c) => {
      switch (c) {
        case "\\":
          return "\\\\";
        case '"':
          return "\\\"";
        case "\b":
          return "\\b";
        case "\f":
          return "\\f";
        case `
`:
          return "\\n";
        case "\r":
          return "\\r";
        case "\t":
          return "\\t";
        default:
          const code = c.charCodeAt(0);
          return "\\x" + code.toString(16).padStart(2, "0");
      }
    }) + '"';
  }
  function yamlStringNeedsQuotes(str) {
    if (str.length === 0)
      return true;
    if (/^\s|\s$/.test(str))
      return true;
    if (/[\x00-\x08\x0b\x0c\x0e-\x1f\x7f-\x9f]/.test(str))
      return true;
    if (/^-/.test(str))
      return true;
    if (/[\n:](\s|$)/.test(str))
      return true;
    if (/\s#/.test(str))
      return true;
    if (/[\n\r]/.test(str))
      return true;
    if (/^[&*\],?!>|@"'#%]/.test(str))
      return true;
    if (/[{}`]/.test(str))
      return true;
    if (/^\[/.test(str))
      return true;
    if (!isNaN(Number(str)) || ["y", "n", "yes", "no", "true", "false", "on", "off", "null"].includes(str.toLowerCase()))
      return true;
    return false;
  }

  // crates/neo-sdk/neo-playwright/third_party/playwright/packages/isomorphic/ariaSnapshotRenderer.ts
  function renderAriaSnapshotAsYaml(snapshot, options = {}) {
    const lines = [];
    const includeText = options.convertStringsToRegex ? textContributesInfo : () => true;
    const renderString = options.convertStringsToRegex ? convertToBestGuessRegex : (str) => str;
    const visitText = (text, depth) => {
      const escaped = yamlEscapeValueIfNeeded(renderString(text));
      if (escaped)
        lines.push(indent(depth) + "- text: " + escaped);
    };
    const createKey = (node) => {
      let key = node.role;
      if (node.name && node.name.length <= 900) {
        const name = renderString(node.name);
        if (name) {
          const isRegex = options.convertStringsToRegex && name !== node.name;
          key += " " + (isRegex ? name : JSON.stringify(name));
        }
      }
      if (node.checked === "mixed")
        key += ` [checked=mixed]`;
      if (node.checked === true)
        key += ` [checked]`;
      if (node.disabled)
        key += ` [disabled]`;
      if (node.expanded)
        key += ` [expanded]`;
      if (node.active)
        key += ` [active]`;
      if (node.invalid === "grammar" || node.invalid === "spelling")
        key += ` [invalid=${node.invalid}]`;
      if (node.invalid === true)
        key += ` [invalid]`;
      if (node.level)
        key += ` [level=${node.level}]`;
      if (node.pressed === "mixed")
        key += ` [pressed=mixed]`;
      if (node.pressed === true)
        key += ` [pressed]`;
      if (node.selected === true)
        key += ` [selected]`;
      if (node.ariaHidden)
        key += ` [aria-hidden]`;
      if (node.ref) {
        key += ` [ref=${node.ref}]`;
        if (node.cursor === "pointer")
          key += " [cursor=pointer]";
      }
      if (node.box)
        key += ` [box=${node.box.x},${node.box.y},${node.box.width},${node.box.height}]`;
      return key;
    };
    const visit = (node, depth) => {
      if (node.role === "text") {
        visitText(node.text || "", depth);
        return;
      }
      options.lineToNode?.set(lines.length, node);
      const escapedKey = indent(depth) + "- " + yamlEscapeKeyIfNeeded(createKey(node));
      const props = [];
      if (node.url !== undefined)
        props.push(["url", node.url]);
      if (node.placeholder !== undefined)
        props.push(["placeholder", node.placeholder]);
      if (node.text === undefined && !props.length && !node.children?.length) {
        lines.push(escapedKey);
      } else if (node.text !== undefined && !props.length) {
        if (includeText(node, node.text))
          lines.push(escapedKey + ": " + yamlEscapeValueIfNeeded(renderString(node.text)));
        else
          lines.push(escapedKey);
      } else {
        lines.push(escapedKey + ":");
        for (const [name, value] of props)
          lines.push(indent(depth + 1) + "- /" + name + ": " + yamlEscapeValueIfNeeded(value));
        if (node.text !== undefined) {
          visitText(includeText(node, node.text) ? node.text : "", depth + 1);
        } else {
          for (const child of node.children || []) {
            if (typeof child === "string")
              visitText(includeText(node, child) ? child : "", depth + 1);
            else
              visit(child, depth + 1);
          }
        }
      }
    };
    for (const node of snapshot)
      visit(node, 0);
    return lines.join(`
`);
  }
  function indent(depth) {
    return "  ".repeat(depth);
  }
  function convertToBestGuessRegex(text) {
    const dynamicContent = [
      { regex: /\b[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}\b/, replacement: "[0-9a-fA-F-]+" },
      { regex: /\b[\d,.]+[bkmBKM]+\b/, replacement: "[\\d,.]+[bkmBKM]+" },
      { regex: /\b\d+[hmsp]+\b/, replacement: "\\d+[hmsp]+" },
      { regex: /\b[\d,.]+[hmsp]+\b/, replacement: "[\\d,.]+[hmsp]+" },
      { regex: /\b\d+,\d+\b/, replacement: "\\d+,\\d+" },
      { regex: /\b\d+\.\d{2,}\b/, replacement: "\\d+\\.\\d+" },
      { regex: /\b\d{2,}\.\d+\b/, replacement: "\\d+\\.\\d+" },
      { regex: /\b\d{2,}\b/, replacement: "\\d+" }
    ];
    let pattern = "";
    let lastIndex = 0;
    const combinedRegex = new RegExp(dynamicContent.map((r) => "(" + r.regex.source + ")").join("|"), "g");
    text.replace(combinedRegex, (match, ...args) => {
      const offset = args[args.length - 2];
      const groups = args.slice(0, -2);
      pattern += escapeRegExp(text.slice(lastIndex, offset));
      for (let i = 0;i < groups.length; i++) {
        if (groups[i]) {
          const { replacement } = dynamicContent[i];
          pattern += replacement;
          break;
        }
      }
      lastIndex = offset + match.length;
      return match;
    });
    if (!pattern)
      return text;
    pattern += escapeRegExp(text.slice(lastIndex));
    return String(new RegExp(pattern));
  }
  function textContributesInfo(node, text) {
    if (!text.length)
      return false;
    if (!node.name)
      return true;
    const substr = text.length <= 200 && node.name.length <= 200 ? longestCommonSubstring(text, node.name) : "";
    let filtered = text;
    while (substr && filtered.includes(substr))
      filtered = filtered.replace(substr, "");
    return filtered.trim().length / text.length > 0.1;
  }
  // crates/neo-sdk/neo-playwright/third_party/playwright/packages/isomorphic/cssTokenizer.ts
  var between = function(num, first, last) {
    return num >= first && num <= last;
  };
  function digit(code) {
    return between(code, 48, 57);
  }
  function hexdigit(code) {
    return digit(code) || between(code, 65, 70) || between(code, 97, 102);
  }
  function uppercaseletter(code) {
    return between(code, 65, 90);
  }
  function lowercaseletter(code) {
    return between(code, 97, 122);
  }
  function letter(code) {
    return uppercaseletter(code) || lowercaseletter(code);
  }
  function nonascii(code) {
    return code >= 128;
  }
  function namestartchar(code) {
    return letter(code) || nonascii(code) || code === 95;
  }
  function namechar(code) {
    return namestartchar(code) || digit(code) || code === 45;
  }
  function nonprintable(code) {
    return between(code, 0, 8) || code === 11 || between(code, 14, 31) || code === 127;
  }
  function newline(code) {
    return code === 10;
  }
  function whitespace(code) {
    return newline(code) || code === 9 || code === 32;
  }
  var maximumallowedcodepoint = 1114111;

  class InvalidCharacterError extends Error {
    constructor(message) {
      super(message);
      this.name = "InvalidCharacterError";
    }
  }
  function preprocess(str) {
    const codepoints = [];
    for (let i = 0;i < str.length; i++) {
      let code = str.charCodeAt(i);
      if (code === 13 && str.charCodeAt(i + 1) === 10) {
        code = 10;
        i++;
      }
      if (code === 13 || code === 12)
        code = 10;
      if (code === 0)
        code = 65533;
      if (between(code, 55296, 56319) && between(str.charCodeAt(i + 1), 56320, 57343)) {
        const lead = code - 55296;
        const trail = str.charCodeAt(i + 1) - 56320;
        code = Math.pow(2, 16) + lead * Math.pow(2, 10) + trail;
        i++;
      }
      codepoints.push(code);
    }
    return codepoints;
  }
  function stringFromCode(code) {
    if (code <= 65535)
      return String.fromCharCode(code);
    code -= Math.pow(2, 16);
    const lead = Math.floor(code / Math.pow(2, 10)) + 55296;
    const trail = code % Math.pow(2, 10) + 56320;
    return String.fromCharCode(lead) + String.fromCharCode(trail);
  }
  function tokenize(str1) {
    const str = preprocess(str1);
    let i = -1;
    const tokens = [];
    let code;
    let line = 0;
    let column = 0;
    let lastLineLength = 0;
    const incrLineno = function() {
      line += 1;
      lastLineLength = column;
      column = 0;
    };
    const locStart = { line, column };
    const codepoint = function(i) {
      if (i >= str.length)
        return -1;
      return str[i];
    };
    const next = function(num) {
      if (num === undefined)
        num = 1;
      if (num > 3)
        throw "Spec Error: no more than three codepoints of lookahead.";
      return codepoint(i + num);
    };
    const consume = function(num) {
      if (num === undefined)
        num = 1;
      i += num;
      code = codepoint(i);
      if (newline(code))
        incrLineno();
      else
        column += num;
      return true;
    };
    const reconsume = function() {
      i -= 1;
      if (newline(code)) {
        line -= 1;
        column = lastLineLength;
      } else {
        column -= 1;
      }
      locStart.line = line;
      locStart.column = column;
      return true;
    };
    const eof = function(codepoint) {
      if (codepoint === undefined)
        codepoint = code;
      return codepoint === -1;
    };
    const donothing = function() {};
    const parseerror = function() {};
    const consumeAToken = function() {
      consumeComments();
      consume();
      if (whitespace(code)) {
        while (whitespace(next()))
          consume();
        return new WhitespaceToken;
      } else if (code === 34) {
        return consumeAStringToken();
      } else if (code === 35) {
        if (namechar(next()) || areAValidEscape(next(1), next(2))) {
          const token = new HashToken("");
          if (wouldStartAnIdentifier(next(1), next(2), next(3)))
            token.type = "id";
          token.value = consumeAName();
          return token;
        } else {
          return new DelimToken(code);
        }
      } else if (code === 36) {
        if (next() === 61) {
          consume();
          return new SuffixMatchToken;
        } else {
          return new DelimToken(code);
        }
      } else if (code === 39) {
        return consumeAStringToken();
      } else if (code === 40) {
        return new OpenParenToken;
      } else if (code === 41) {
        return new CloseParenToken;
      } else if (code === 42) {
        if (next() === 61) {
          consume();
          return new SubstringMatchToken;
        } else {
          return new DelimToken(code);
        }
      } else if (code === 43) {
        if (startsWithANumber()) {
          reconsume();
          return consumeANumericToken();
        } else {
          return new DelimToken(code);
        }
      } else if (code === 44) {
        return new CommaToken;
      } else if (code === 45) {
        if (startsWithANumber()) {
          reconsume();
          return consumeANumericToken();
        } else if (next(1) === 45 && next(2) === 62) {
          consume(2);
          return new CDCToken;
        } else if (startsWithAnIdentifier()) {
          reconsume();
          return consumeAnIdentlikeToken();
        } else {
          return new DelimToken(code);
        }
      } else if (code === 46) {
        if (startsWithANumber()) {
          reconsume();
          return consumeANumericToken();
        } else {
          return new DelimToken(code);
        }
      } else if (code === 58) {
        return new ColonToken;
      } else if (code === 59) {
        return new SemicolonToken;
      } else if (code === 60) {
        if (next(1) === 33 && next(2) === 45 && next(3) === 45) {
          consume(3);
          return new CDOToken;
        } else {
          return new DelimToken(code);
        }
      } else if (code === 64) {
        if (wouldStartAnIdentifier(next(1), next(2), next(3)))
          return new AtKeywordToken(consumeAName());
        else
          return new DelimToken(code);
      } else if (code === 91) {
        return new OpenSquareToken;
      } else if (code === 92) {
        if (startsWithAValidEscape()) {
          reconsume();
          return consumeAnIdentlikeToken();
        } else {
          parseerror();
          return new DelimToken(code);
        }
      } else if (code === 93) {
        return new CloseSquareToken;
      } else if (code === 94) {
        if (next() === 61) {
          consume();
          return new PrefixMatchToken;
        } else {
          return new DelimToken(code);
        }
      } else if (code === 123) {
        return new OpenCurlyToken;
      } else if (code === 124) {
        if (next() === 61) {
          consume();
          return new DashMatchToken;
        } else if (next() === 124) {
          consume();
          return new ColumnToken;
        } else {
          return new DelimToken(code);
        }
      } else if (code === 125) {
        return new CloseCurlyToken;
      } else if (code === 126) {
        if (next() === 61) {
          consume();
          return new IncludeMatchToken;
        } else {
          return new DelimToken(code);
        }
      } else if (digit(code)) {
        reconsume();
        return consumeANumericToken();
      } else if (namestartchar(code)) {
        reconsume();
        return consumeAnIdentlikeToken();
      } else if (eof()) {
        return new EOFToken;
      } else {
        return new DelimToken(code);
      }
    };
    const consumeComments = function() {
      while (next(1) === 47 && next(2) === 42) {
        consume(2);
        while (true) {
          consume();
          if (code === 42 && next() === 47) {
            consume();
            break;
          } else if (eof()) {
            parseerror();
            return;
          }
        }
      }
    };
    const consumeANumericToken = function() {
      const num = consumeANumber();
      if (wouldStartAnIdentifier(next(1), next(2), next(3))) {
        const token = new DimensionToken;
        token.value = num.value;
        token.repr = num.repr;
        token.type = num.type;
        token.unit = consumeAName();
        return token;
      } else if (next() === 37) {
        consume();
        const token = new PercentageToken;
        token.value = num.value;
        token.repr = num.repr;
        return token;
      } else {
        const token = new NumberToken;
        token.value = num.value;
        token.repr = num.repr;
        token.type = num.type;
        return token;
      }
    };
    const consumeAnIdentlikeToken = function() {
      const str = consumeAName();
      if (str.toLowerCase() === "url" && next() === 40) {
        consume();
        while (whitespace(next(1)) && whitespace(next(2)))
          consume();
        if (next() === 34 || next() === 39)
          return new FunctionToken(str);
        else if (whitespace(next()) && (next(2) === 34 || next(2) === 39))
          return new FunctionToken(str);
        else
          return consumeAURLToken();
      } else if (next() === 40) {
        consume();
        return new FunctionToken(str);
      } else {
        return new IdentToken(str);
      }
    };
    const consumeAStringToken = function(endingCodePoint) {
      if (endingCodePoint === undefined)
        endingCodePoint = code;
      let string = "";
      while (consume()) {
        if (code === endingCodePoint || eof()) {
          return new StringToken(string);
        } else if (newline(code)) {
          parseerror();
          reconsume();
          return new BadStringToken;
        } else if (code === 92) {
          if (eof(next()))
            donothing();
          else if (newline(next()))
            consume();
          else
            string += stringFromCode(consumeEscape());
        } else {
          string += stringFromCode(code);
        }
      }
      throw new Error("Internal error");
    };
    const consumeAURLToken = function() {
      const token = new URLToken("");
      while (whitespace(next()))
        consume();
      if (eof(next()))
        return token;
      while (consume()) {
        if (code === 41 || eof()) {
          return token;
        } else if (whitespace(code)) {
          while (whitespace(next()))
            consume();
          if (next() === 41 || eof(next())) {
            consume();
            return token;
          } else {
            consumeTheRemnantsOfABadURL();
            return new BadURLToken;
          }
        } else if (code === 34 || code === 39 || code === 40 || nonprintable(code)) {
          parseerror();
          consumeTheRemnantsOfABadURL();
          return new BadURLToken;
        } else if (code === 92) {
          if (startsWithAValidEscape()) {
            token.value += stringFromCode(consumeEscape());
          } else {
            parseerror();
            consumeTheRemnantsOfABadURL();
            return new BadURLToken;
          }
        } else {
          token.value += stringFromCode(code);
        }
      }
      throw new Error("Internal error");
    };
    const consumeEscape = function() {
      consume();
      if (hexdigit(code)) {
        const digits = [code];
        for (let total = 0;total < 5; total++) {
          if (hexdigit(next())) {
            consume();
            digits.push(code);
          } else {
            break;
          }
        }
        if (whitespace(next()))
          consume();
        let value = parseInt(digits.map(function(x) {
          return String.fromCharCode(x);
        }).join(""), 16);
        if (value > maximumallowedcodepoint)
          value = 65533;
        return value;
      } else if (eof()) {
        return 65533;
      } else {
        return code;
      }
    };
    const areAValidEscape = function(c1, c2) {
      if (c1 !== 92)
        return false;
      if (newline(c2))
        return false;
      return true;
    };
    const startsWithAValidEscape = function() {
      return areAValidEscape(code, next());
    };
    const wouldStartAnIdentifier = function(c1, c2, c3) {
      if (c1 === 45)
        return namestartchar(c2) || c2 === 45 || areAValidEscape(c2, c3);
      else if (namestartchar(c1))
        return true;
      else if (c1 === 92)
        return areAValidEscape(c1, c2);
      else
        return false;
    };
    const startsWithAnIdentifier = function() {
      return wouldStartAnIdentifier(code, next(1), next(2));
    };
    const wouldStartANumber = function(c1, c2, c3) {
      if (c1 === 43 || c1 === 45) {
        if (digit(c2))
          return true;
        if (c2 === 46 && digit(c3))
          return true;
        return false;
      } else if (c1 === 46) {
        if (digit(c2))
          return true;
        return false;
      } else if (digit(c1)) {
        return true;
      } else {
        return false;
      }
    };
    const startsWithANumber = function() {
      return wouldStartANumber(code, next(1), next(2));
    };
    const consumeAName = function() {
      let result = "";
      while (consume()) {
        if (namechar(code)) {
          result += stringFromCode(code);
        } else if (startsWithAValidEscape()) {
          result += stringFromCode(consumeEscape());
        } else {
          reconsume();
          return result;
        }
      }
      throw new Error("Internal parse error");
    };
    const consumeANumber = function() {
      let repr = "";
      let type = "integer";
      if (next() === 43 || next() === 45) {
        consume();
        repr += stringFromCode(code);
      }
      while (digit(next())) {
        consume();
        repr += stringFromCode(code);
      }
      if (next(1) === 46 && digit(next(2))) {
        consume();
        repr += stringFromCode(code);
        consume();
        repr += stringFromCode(code);
        type = "number";
        while (digit(next())) {
          consume();
          repr += stringFromCode(code);
        }
      }
      const c1 = next(1);
      const c2 = next(2);
      const c3 = next(3);
      if ((c1 === 69 || c1 === 101) && digit(c2)) {
        consume();
        repr += stringFromCode(code);
        consume();
        repr += stringFromCode(code);
        type = "number";
        while (digit(next())) {
          consume();
          repr += stringFromCode(code);
        }
      } else if ((c1 === 69 || c1 === 101) && (c2 === 43 || c2 === 45) && digit(c3)) {
        consume();
        repr += stringFromCode(code);
        consume();
        repr += stringFromCode(code);
        consume();
        repr += stringFromCode(code);
        type = "number";
        while (digit(next())) {
          consume();
          repr += stringFromCode(code);
        }
      }
      const value = convertAStringToANumber(repr);
      return { type, value, repr };
    };
    const convertAStringToANumber = function(string) {
      return +string;
    };
    const consumeTheRemnantsOfABadURL = function() {
      while (consume()) {
        if (code === 41 || eof()) {
          return;
        } else if (startsWithAValidEscape()) {
          consumeEscape();
          donothing();
        } else {
          donothing();
        }
      }
    };
    let iterationCount = 0;
    while (!eof(next())) {
      tokens.push(consumeAToken());
      iterationCount++;
      if (iterationCount > str.length * 2)
        throw new Error("I'm infinite-looping!");
    }
    return tokens;
  }

  class CSSParserToken {
    tokenType = "";
    value;
    toJSON() {
      return { token: this.tokenType };
    }
    toString() {
      return this.tokenType;
    }
    toSource() {
      return "" + this;
    }
  }

  class BadStringToken extends CSSParserToken {
    tokenType = "BADSTRING";
  }

  class BadURLToken extends CSSParserToken {
    tokenType = "BADURL";
  }

  class WhitespaceToken extends CSSParserToken {
    tokenType = "WHITESPACE";
    toString() {
      return "WS";
    }
    toSource() {
      return " ";
    }
  }

  class CDOToken extends CSSParserToken {
    tokenType = "CDO";
    toSource() {
      return "<!--";
    }
  }

  class CDCToken extends CSSParserToken {
    tokenType = "CDC";
    toSource() {
      return "-->";
    }
  }

  class ColonToken extends CSSParserToken {
    tokenType = ":";
  }

  class SemicolonToken extends CSSParserToken {
    tokenType = ";";
  }

  class CommaToken extends CSSParserToken {
    tokenType = ",";
  }

  class GroupingToken extends CSSParserToken {
    value = "";
    mirror = "";
  }

  class OpenCurlyToken extends GroupingToken {
    tokenType = "{";
    constructor() {
      super();
      this.value = "{";
      this.mirror = "}";
    }
  }

  class CloseCurlyToken extends GroupingToken {
    tokenType = "}";
    constructor() {
      super();
      this.value = "}";
      this.mirror = "{";
    }
  }

  class OpenSquareToken extends GroupingToken {
    tokenType = "[";
    constructor() {
      super();
      this.value = "[";
      this.mirror = "]";
    }
  }

  class CloseSquareToken extends GroupingToken {
    tokenType = "]";
    constructor() {
      super();
      this.value = "]";
      this.mirror = "[";
    }
  }

  class OpenParenToken extends GroupingToken {
    tokenType = "(";
    constructor() {
      super();
      this.value = "(";
      this.mirror = ")";
    }
  }

  class CloseParenToken extends GroupingToken {
    tokenType = ")";
    constructor() {
      super();
      this.value = ")";
      this.mirror = "(";
    }
  }

  class IncludeMatchToken extends CSSParserToken {
    tokenType = "~=";
  }

  class DashMatchToken extends CSSParserToken {
    tokenType = "|=";
  }

  class PrefixMatchToken extends CSSParserToken {
    tokenType = "^=";
  }

  class SuffixMatchToken extends CSSParserToken {
    tokenType = "$=";
  }

  class SubstringMatchToken extends CSSParserToken {
    tokenType = "*=";
  }

  class ColumnToken extends CSSParserToken {
    tokenType = "||";
  }

  class EOFToken extends CSSParserToken {
    tokenType = "EOF";
    toSource() {
      return "";
    }
  }

  class DelimToken extends CSSParserToken {
    tokenType = "DELIM";
    value = "";
    constructor(code) {
      super();
      this.value = stringFromCode(code);
    }
    toString() {
      return "DELIM(" + this.value + ")";
    }
    toJSON() {
      const json = this.constructor.prototype.constructor.prototype.toJSON.call(this);
      json.value = this.value;
      return json;
    }
    toSource() {
      if (this.value === "\\")
        return "\\\n";
      else
        return this.value;
    }
  }

  class StringValuedToken extends CSSParserToken {
    value = "";
    ASCIIMatch(str) {
      return this.value.toLowerCase() === str.toLowerCase();
    }
    toJSON() {
      const json = this.constructor.prototype.constructor.prototype.toJSON.call(this);
      json.value = this.value;
      return json;
    }
  }

  class IdentToken extends StringValuedToken {
    constructor(val) {
      super();
      this.value = val;
    }
    tokenType = "IDENT";
    toString() {
      return "IDENT(" + this.value + ")";
    }
    toSource() {
      return escapeIdent(this.value);
    }
  }

  class FunctionToken extends StringValuedToken {
    tokenType = "FUNCTION";
    mirror;
    constructor(val) {
      super();
      this.value = val;
      this.mirror = ")";
    }
    toString() {
      return "FUNCTION(" + this.value + ")";
    }
    toSource() {
      return escapeIdent(this.value) + "(";
    }
  }

  class AtKeywordToken extends StringValuedToken {
    tokenType = "AT-KEYWORD";
    constructor(val) {
      super();
      this.value = val;
    }
    toString() {
      return "AT(" + this.value + ")";
    }
    toSource() {
      return "@" + escapeIdent(this.value);
    }
  }

  class HashToken extends StringValuedToken {
    tokenType = "HASH";
    type;
    constructor(val) {
      super();
      this.value = val;
      this.type = "unrestricted";
    }
    toString() {
      return "HASH(" + this.value + ")";
    }
    toJSON() {
      const json = this.constructor.prototype.constructor.prototype.toJSON.call(this);
      json.value = this.value;
      json.type = this.type;
      return json;
    }
    toSource() {
      if (this.type === "id")
        return "#" + escapeIdent(this.value);
      else
        return "#" + escapeHash(this.value);
    }
  }

  class StringToken extends StringValuedToken {
    tokenType = "STRING";
    constructor(val) {
      super();
      this.value = val;
    }
    toString() {
      return '"' + escapeString(this.value) + '"';
    }
  }

  class URLToken extends StringValuedToken {
    tokenType = "URL";
    constructor(val) {
      super();
      this.value = val;
    }
    toString() {
      return "URL(" + this.value + ")";
    }
    toSource() {
      return 'url("' + escapeString(this.value) + '")';
    }
  }

  class NumberToken extends CSSParserToken {
    tokenType = "NUMBER";
    type;
    repr;
    constructor() {
      super();
      this.type = "integer";
      this.repr = "";
    }
    toString() {
      if (this.type === "integer")
        return "INT(" + this.value + ")";
      return "NUMBER(" + this.value + ")";
    }
    toJSON() {
      const json = super.toJSON();
      json.value = this.value;
      json.type = this.type;
      json.repr = this.repr;
      return json;
    }
    toSource() {
      return this.repr;
    }
  }

  class PercentageToken extends CSSParserToken {
    tokenType = "PERCENTAGE";
    repr;
    constructor() {
      super();
      this.repr = "";
    }
    toString() {
      return "PERCENTAGE(" + this.value + ")";
    }
    toJSON() {
      const json = this.constructor.prototype.constructor.prototype.toJSON.call(this);
      json.value = this.value;
      json.repr = this.repr;
      return json;
    }
    toSource() {
      return this.repr + "%";
    }
  }

  class DimensionToken extends CSSParserToken {
    tokenType = "DIMENSION";
    type;
    repr;
    unit;
    constructor() {
      super();
      this.type = "integer";
      this.repr = "";
      this.unit = "";
    }
    toString() {
      return "DIM(" + this.value + "," + this.unit + ")";
    }
    toJSON() {
      const json = this.constructor.prototype.constructor.prototype.toJSON.call(this);
      json.value = this.value;
      json.type = this.type;
      json.repr = this.repr;
      json.unit = this.unit;
      return json;
    }
    toSource() {
      const source = this.repr;
      let unit = escapeIdent(this.unit);
      if (unit[0].toLowerCase() === "e" && (unit[1] === "-" || between(unit.charCodeAt(1), 48, 57))) {
        unit = "\\65 " + unit.slice(1, unit.length);
      }
      return source + unit;
    }
  }
  function escapeIdent(string) {
    string = "" + string;
    let result = "";
    const firstcode = string.charCodeAt(0);
    for (let i = 0;i < string.length; i++) {
      const code = string.charCodeAt(i);
      if (code === 0)
        throw new InvalidCharacterError("Invalid character: the input contains U+0000.");
      if (between(code, 1, 31) || code === 127 || i === 0 && between(code, 48, 57) || i === 1 && between(code, 48, 57) && firstcode === 45)
        result += "\\" + code.toString(16) + " ";
      else if (code >= 128 || code === 45 || code === 95 || between(code, 48, 57) || between(code, 65, 90) || between(code, 97, 122))
        result += string[i];
      else
        result += "\\" + string[i];
    }
    return result;
  }
  function escapeHash(string) {
    string = "" + string;
    let result = "";
    for (let i = 0;i < string.length; i++) {
      const code = string.charCodeAt(i);
      if (code === 0)
        throw new InvalidCharacterError("Invalid character: the input contains U+0000.");
      if (code >= 128 || code === 45 || code === 95 || between(code, 48, 57) || between(code, 65, 90) || between(code, 97, 122))
        result += string[i];
      else
        result += "\\" + code.toString(16) + " ";
    }
    return result;
  }
  function escapeString(string) {
    string = "" + string;
    let result = "";
    for (let i = 0;i < string.length; i++) {
      const code = string.charCodeAt(i);
      if (code === 0)
        throw new InvalidCharacterError("Invalid character: the input contains U+0000.");
      if (between(code, 1, 31) || code === 127)
        result += "\\" + code.toString(16) + " ";
      else if (code === 34 || code === 92)
        result += "\\" + string[i];
      else
        result += string[i];
    }
    return result;
  }

  // crates/neo-sdk/neo-playwright/third_party/playwright/packages/isomorphic/cssParser.ts
  class InvalidSelectorError extends Error {
  }
  function parseCSS(selector, customNames) {
    let tokens;
    try {
      tokens = tokenize(selector);
      if (!(tokens[tokens.length - 1] instanceof EOFToken))
        tokens.push(new EOFToken);
    } catch (e) {
      const newMessage = e.message + ` while parsing css selector "${selector}". Did you mean to CSS.escape it?`;
      const index = (e.stack || "").indexOf(e.message);
      if (index !== -1)
        e.stack = e.stack.substring(0, index) + newMessage + e.stack.substring(index + e.message.length);
      e.message = newMessage;
      throw e;
    }
    const unsupportedToken = tokens.find((token) => {
      return token instanceof AtKeywordToken || token instanceof BadStringToken || token instanceof BadURLToken || token instanceof ColumnToken || token instanceof CDOToken || token instanceof CDCToken || token instanceof SemicolonToken || token instanceof OpenCurlyToken || token instanceof CloseCurlyToken || token instanceof URLToken || token instanceof PercentageToken;
    });
    if (unsupportedToken)
      throw new InvalidSelectorError(`Unsupported token "${unsupportedToken.toSource()}" while parsing css selector "${selector}". Did you mean to CSS.escape it?`);
    let pos = 0;
    const names = new Set;
    function unexpected() {
      return new InvalidSelectorError(`Unexpected token "${tokens[pos].toSource()}" while parsing css selector "${selector}". Did you mean to CSS.escape it?`);
    }
    function skipWhitespace() {
      while (tokens[pos] instanceof WhitespaceToken)
        pos++;
    }
    function isIdent(p = pos) {
      return tokens[p] instanceof IdentToken;
    }
    function isString(p = pos) {
      return tokens[p] instanceof StringToken;
    }
    function isNumber(p = pos) {
      return tokens[p] instanceof NumberToken;
    }
    function isComma(p = pos) {
      return tokens[p] instanceof CommaToken;
    }
    function isOpenParen(p = pos) {
      return tokens[p] instanceof OpenParenToken;
    }
    function isCloseParen(p = pos) {
      return tokens[p] instanceof CloseParenToken;
    }
    function isFunction(p = pos) {
      return tokens[p] instanceof FunctionToken;
    }
    function isStar(p = pos) {
      return tokens[p] instanceof DelimToken && tokens[p].value === "*";
    }
    function isEOF(p = pos) {
      return tokens[p] instanceof EOFToken;
    }
    function isClauseCombinator(p = pos) {
      return tokens[p] instanceof DelimToken && [">", "+", "~"].includes(tokens[p].value);
    }
    function isSelectorClauseEnd(p = pos) {
      return isComma(p) || isCloseParen(p) || isEOF(p) || isClauseCombinator(p) || tokens[p] instanceof WhitespaceToken;
    }
    function consumeFunctionArguments() {
      const result = [consumeArgument()];
      while (true) {
        skipWhitespace();
        if (!isComma())
          break;
        pos++;
        result.push(consumeArgument());
      }
      return result;
    }
    function consumeArgument() {
      skipWhitespace();
      if (isNumber())
        return tokens[pos++].value;
      if (isString())
        return tokens[pos++].value;
      return consumeComplexSelector();
    }
    function consumeComplexSelector() {
      const result = { simples: [] };
      skipWhitespace();
      if (isClauseCombinator()) {
        result.simples.push({ selector: { functions: [{ name: "scope", args: [] }] }, combinator: "" });
      } else {
        result.simples.push({ selector: consumeSimpleSelector(), combinator: "" });
      }
      while (true) {
        skipWhitespace();
        if (isClauseCombinator()) {
          result.simples[result.simples.length - 1].combinator = tokens[pos++].value;
          skipWhitespace();
        } else if (isSelectorClauseEnd()) {
          break;
        }
        result.simples.push({ combinator: "", selector: consumeSimpleSelector() });
      }
      return result;
    }
    function consumeSimpleSelector() {
      let rawCSSString = "";
      const functions = [];
      while (!isSelectorClauseEnd()) {
        if (isIdent() || isStar()) {
          rawCSSString += tokens[pos++].toSource();
        } else if (tokens[pos] instanceof HashToken) {
          rawCSSString += tokens[pos++].toSource();
        } else if (tokens[pos] instanceof DelimToken && tokens[pos].value === ".") {
          pos++;
          if (isIdent())
            rawCSSString += "." + tokens[pos++].toSource();
          else
            throw unexpected();
        } else if (tokens[pos] instanceof ColonToken) {
          pos++;
          if (isIdent()) {
            if (!customNames.has(tokens[pos].value.toLowerCase())) {
              rawCSSString += ":" + tokens[pos++].toSource();
            } else {
              const name = tokens[pos++].value.toLowerCase();
              functions.push({ name, args: [] });
              names.add(name);
            }
          } else if (isFunction()) {
            const name = tokens[pos++].value.toLowerCase();
            if (!customNames.has(name)) {
              rawCSSString += `:${name}(${consumeBuiltinFunctionArguments()})`;
            } else {
              functions.push({ name, args: consumeFunctionArguments() });
              names.add(name);
            }
            skipWhitespace();
            if (!isCloseParen())
              throw unexpected();
            pos++;
          } else {
            throw unexpected();
          }
        } else if (tokens[pos] instanceof OpenSquareToken) {
          rawCSSString += "[";
          pos++;
          while (!(tokens[pos] instanceof CloseSquareToken) && !isEOF())
            rawCSSString += tokens[pos++].toSource();
          if (!(tokens[pos] instanceof CloseSquareToken))
            throw unexpected();
          rawCSSString += "]";
          pos++;
        } else {
          throw unexpected();
        }
      }
      if (!rawCSSString && !functions.length)
        throw unexpected();
      return { css: rawCSSString || undefined, functions };
    }
    function consumeBuiltinFunctionArguments() {
      let s = "";
      let balance = 1;
      while (!isEOF()) {
        if (isOpenParen() || isFunction())
          balance++;
        if (isCloseParen())
          balance--;
        if (!balance)
          break;
        s += tokens[pos++].toSource();
      }
      return s;
    }
    const result = consumeFunctionArguments();
    if (!isEOF())
      throw unexpected();
    if (result.some((arg) => typeof arg !== "object" || !("simples" in arg)))
      throw new InvalidSelectorError(`Error while parsing css selector "${selector}". Did you mean to CSS.escape it?`);
    return { selector: result, names: Array.from(names) };
  }

  // crates/neo-sdk/neo-playwright/third_party/playwright/packages/isomorphic/selectorParser.ts
  var kNestedSelectorNames = new Set(["internal:has", "internal:has-not", "internal:and", "internal:or", "internal:chain", "left-of", "right-of", "above", "below", "near"]);
  var kNestedSelectorNamesWithDistance = new Set(["left-of", "right-of", "above", "below", "near"]);
  var customCSSNames = new Set(["not", "is", "where", "has", "scope", "light", "visible", "text", "text-matches", "text-is", "has-text", "above", "below", "right-of", "left-of", "near", "nth-match"]);
  function parseSelector(selector) {
    const parsedStrings = parseSelectorString(selector);
    const parts = [];
    for (const part of parsedStrings.parts) {
      if (part.name === "css" || part.name === "css:light") {
        if (part.name === "css:light")
          part.body = ":light(" + part.body + ")";
        const parsedCSS = parseCSS(part.body, customCSSNames);
        parts.push({
          name: "css",
          body: parsedCSS.selector,
          source: part.body
        });
        continue;
      }
      if (kNestedSelectorNames.has(part.name)) {
        let innerSelector;
        let distance;
        try {
          const unescaped = JSON.parse("[" + part.body + "]");
          if (!Array.isArray(unescaped) || unescaped.length < 1 || unescaped.length > 2 || typeof unescaped[0] !== "string")
            throw new InvalidSelectorError(`Malformed selector: ${part.name}=` + part.body);
          innerSelector = unescaped[0];
          if (unescaped.length === 2) {
            if (typeof unescaped[1] !== "number" || !kNestedSelectorNamesWithDistance.has(part.name))
              throw new InvalidSelectorError(`Malformed selector: ${part.name}=` + part.body);
            distance = unescaped[1];
          }
        } catch (e) {
          throw new InvalidSelectorError(`Malformed selector: ${part.name}=` + part.body);
        }
        const nested = { name: part.name, source: part.body, body: { parsed: parseSelector(innerSelector), distance } };
        const lastFrame = [...nested.body.parsed.parts].reverse().find((part) => part.name === "internal:control" && part.body === "enter-frame");
        const lastFrameIndex = lastFrame ? nested.body.parsed.parts.indexOf(lastFrame) : -1;
        const outerParts = parts[0]?.name === "internal:control" && parts[0].body === "any-frame" ? parts.slice(1) : parts;
        if (lastFrameIndex !== -1 && selectorPartsEqual(nested.body.parsed.parts.slice(0, lastFrameIndex + 1), outerParts.slice(0, lastFrameIndex + 1))) {
          nested.body.parsed.parts.splice(0, lastFrameIndex + 1);
          if (nested.body.parsed.capture !== undefined) {
            if (nested.body.parsed.capture <= lastFrameIndex)
              throw new InvalidSelectorError(`Can not capture the selector before diving into the frame. Only use * after the last frame has been selected`);
            nested.body.parsed.capture -= lastFrameIndex + 1;
          }
        }
        parts.push(nested);
        continue;
      }
      parts.push({ ...part, source: part.body });
    }
    if (kNestedSelectorNames.has(parts[0].name))
      throw new InvalidSelectorError(`"${parts[0].name}" selector cannot be first`);
    return {
      capture: parsedStrings.capture,
      parts
    };
  }
  var kAnyFrameSelector = "internal:control=any-frame";
  function splitSelectorByFrame(selectorText) {
    const selector = parseSelector(selectorText);
    const chunks = [];
    let chunk = {
      parts: []
    };
    let anyFrame = false;
    let chunkStartIndex = 0;
    for (let i = 0;i < selector.parts.length; ++i) {
      const part = selector.parts[i];
      if (part.name === "internal:control" && part.body === "any-frame") {
        if (i !== 0)
          throw new InvalidSelectorError(`"${part.body}" is only allowed as the first selector token, while parsing selector ${selectorText}`);
        anyFrame = true;
        chunkStartIndex = i + 1;
        continue;
      }
      if (part.name === "internal:control" && part.body === "enter-frame") {
        const lastPart = chunk.parts[chunk.parts.length - 1];
        if (!lastPart || lastPart.name === "internal:control" && lastPart.body === "enter-frame")
          throw new InvalidSelectorError("Selector cannot start with entering frame, select the iframe first");
        chunks.push(chunk);
        chunk = { parts: [] };
        chunkStartIndex = i + 1;
        continue;
      }
      if (selector.capture === i)
        chunk.capture = i - chunkStartIndex;
      chunk.parts.push(part);
    }
    if (!chunk.parts.length) {
      if (anyFrame && !chunks.length)
        throw new InvalidSelectorError(`Selector cannot be empty after frameLocator(), while parsing selector ${selectorText}`);
      throw new InvalidSelectorError(`Selector cannot end with entering frame, while parsing selector ${selectorText}`);
    }
    const lastPart = chunk.parts[chunk.parts.length - 1];
    if (lastPart.name === "internal:control" && lastPart.body === "enter-frame")
      throw new InvalidSelectorError(`Selector cannot end with entering frame, while parsing selector ${selectorText}`);
    chunks.push(chunk);
    if (typeof selector.capture === "number" && typeof chunks[chunks.length - 1].capture !== "number")
      throw new InvalidSelectorError(`Can not capture the selector before diving into the frame. Only use * after the last frame has been selected`);
    return { anyFrame, chunks };
  }
  function selectorPartsEqual(list1, list2) {
    return stringifySelector({ parts: list1 }) === stringifySelector({ parts: list2 });
  }
  function stringifySelector(selector, forceEngineName) {
    if (typeof selector === "string")
      return selector;
    return selector.parts.map((p, i) => {
      let includeEngine = true;
      if (!forceEngineName && i !== selector.capture) {
        if (p.name === "css")
          includeEngine = false;
        else if (p.name === "xpath" && (p.source.startsWith("//") || p.source.startsWith("..")))
          includeEngine = false;
      }
      const prefix = includeEngine ? p.name + "=" : "";
      return `${i === selector.capture ? "*" : ""}${prefix}${p.source}`;
    }).join(" >> ");
  }
  function parseSelectorString(selector) {
    let index = 0;
    let quote;
    let start = 0;
    const result = { parts: [] };
    const append = () => {
      const part = selector.substring(start, index).trim();
      const eqIndex = part.indexOf("=");
      let name;
      let body;
      if (eqIndex !== -1 && part.substring(0, eqIndex).trim().match(/^[a-zA-Z_0-9-+:*]+$/)) {
        name = part.substring(0, eqIndex).trim();
        body = part.substring(eqIndex + 1);
      } else if (part.length > 1 && part[0] === '"' && part[part.length - 1] === '"') {
        name = "text";
        body = part;
      } else if (part.length > 1 && part[0] === "'" && part[part.length - 1] === "'") {
        name = "text";
        body = part;
      } else if (/^\(*\/\//.test(part) || part.startsWith("..")) {
        name = "xpath";
        body = part;
      } else {
        name = "css";
        body = part;
      }
      let capture = false;
      if (name[0] === "*") {
        capture = true;
        name = name.substring(1);
      }
      result.parts.push({ name, body });
      if (capture) {
        if (result.capture !== undefined)
          throw new InvalidSelectorError(`Only one of the selectors can capture using * modifier`);
        result.capture = result.parts.length - 1;
      }
    };
    if (!selector.includes(">>")) {
      index = selector.length;
      append();
      return result;
    }
    const shouldIgnoreTextSelectorQuote = () => {
      const prefix = selector.substring(start, index);
      const match = prefix.match(/^\s*text\s*=(.*)$/);
      return !!match && !!match[1];
    };
    while (index < selector.length) {
      const c = selector[index];
      if (c === "\\" && index + 1 < selector.length) {
        index += 2;
      } else if (c === quote) {
        quote = undefined;
        index++;
      } else if (!quote && (c === '"' || c === "'" || c === "`") && !shouldIgnoreTextSelectorQuote()) {
        quote = c;
        index++;
      } else if (!quote && c === ">" && selector[index + 1] === ">") {
        append();
        index += 2;
        start = index;
      } else {
        index++;
      }
    }
    append();
    return result;
  }
  function parseAttributeSelector(selector, allowUnquotedStrings) {
    let wp = 0;
    let EOL = selector.length === 0;
    const next = () => selector[wp] || "";
    const eat1 = () => {
      const result = next();
      ++wp;
      EOL = wp >= selector.length;
      return result;
    };
    const syntaxError = (stage) => {
      if (EOL)
        throw new InvalidSelectorError(`Unexpected end of selector while parsing selector \`${selector}\``);
      throw new InvalidSelectorError(`Error while parsing selector \`${selector}\` - unexpected symbol "${next()}" at position ${wp}` + (stage ? " during " + stage : ""));
    };
    function skipSpaces() {
      while (!EOL && /\s/.test(next()))
        eat1();
    }
    function isCSSNameChar(char) {
      return char >= "" || char >= "0" && char <= "9" || char >= "A" && char <= "Z" || char >= "a" && char <= "z" || char >= "0" && char <= "9" || char === "_" || char === "-";
    }
    function readIdentifier() {
      let result = "";
      skipSpaces();
      while (!EOL && isCSSNameChar(next()))
        result += eat1();
      return result;
    }
    function readQuotedString(quote) {
      let result = eat1();
      if (result !== quote)
        syntaxError("parsing quoted string");
      while (!EOL && next() !== quote) {
        if (next() === "\\")
          eat1();
        result += eat1();
      }
      if (next() !== quote)
        syntaxError("parsing quoted string");
      result += eat1();
      return result;
    }
    function readRegularExpression() {
      if (eat1() !== "/")
        syntaxError("parsing regular expression");
      let source = "";
      let inClass = false;
      while (!EOL) {
        if (next() === "\\") {
          source += eat1();
          if (EOL)
            syntaxError("parsing regular expression");
        } else if (inClass && next() === "]") {
          inClass = false;
        } else if (!inClass && next() === "[") {
          inClass = true;
        } else if (!inClass && next() === "/") {
          break;
        }
        source += eat1();
      }
      if (eat1() !== "/")
        syntaxError("parsing regular expression");
      let flags = "";
      while (!EOL && next().match(/[dgimsuvy]/))
        flags += eat1();
      try {
        return new RegExp(source, flags);
      } catch (e) {
        throw new InvalidSelectorError(`Error while parsing selector \`${selector}\`: ${e.message}`);
      }
    }
    function readAttributeToken() {
      let token = "";
      skipSpaces();
      if (next() === `'` || next() === `"`)
        token = readQuotedString(next()).slice(1, -1);
      else
        token = readIdentifier();
      if (!token)
        syntaxError("parsing property path");
      return token;
    }
    function readOperator() {
      skipSpaces();
      let op = "";
      if (!EOL)
        op += eat1();
      if (!EOL && op !== "=")
        op += eat1();
      if (!["=", "*=", "^=", "$=", "|=", "~="].includes(op))
        syntaxError("parsing operator");
      return op;
    }
    function readAttribute() {
      eat1();
      const jsonPath = [];
      jsonPath.push(readAttributeToken());
      skipSpaces();
      while (next() === ".") {
        eat1();
        jsonPath.push(readAttributeToken());
        skipSpaces();
      }
      if (next() === "]") {
        eat1();
        return { name: jsonPath.join("."), jsonPath, op: "<truthy>", value: null, caseSensitive: false };
      }
      const operator = readOperator();
      let value = undefined;
      let caseSensitive = true;
      skipSpaces();
      if (next() === "/") {
        if (operator !== "=")
          throw new InvalidSelectorError(`Error while parsing selector \`${selector}\` - cannot use ${operator} in attribute with regular expression`);
        value = readRegularExpression();
      } else if (next() === `'` || next() === `"`) {
        value = readQuotedString(next()).slice(1, -1);
        skipSpaces();
        if (next() === "i" || next() === "I") {
          caseSensitive = false;
          eat1();
        } else if (next() === "s" || next() === "S") {
          caseSensitive = true;
          eat1();
        }
      } else {
        value = "";
        while (!EOL && (isCSSNameChar(next()) || next() === "+" || next() === "."))
          value += eat1();
        if (value === "true") {
          value = true;
        } else if (value === "false") {
          value = false;
        } else {
          if (!allowUnquotedStrings) {
            value = +value;
            if (Number.isNaN(value))
              syntaxError("parsing attribute value");
          }
        }
      }
      skipSpaces();
      if (next() !== "]")
        syntaxError("parsing attribute value");
      eat1();
      if (operator !== "=" && typeof value !== "string")
        throw new InvalidSelectorError(`Error while parsing selector \`${selector}\` - cannot use ${operator} in attribute with non-string matching value - ${value}`);
      return { name: jsonPath.join("."), jsonPath, op: operator, value, caseSensitive };
    }
    const result = {
      name: "",
      attributes: []
    };
    result.name = readIdentifier();
    skipSpaces();
    while (next() === "[") {
      result.attributes.push(readAttribute());
      skipSpaces();
    }
    if (!EOL)
      syntaxError(undefined);
    if (!result.name && !result.attributes.length)
      throw new InvalidSelectorError(`Error while parsing selector \`${selector}\` - selector cannot be empty`);
    return result;
  }

  // crates/neo-sdk/neo-playwright/third_party/playwright/packages/isomorphic/locatorGenerators.ts
  function asLocatorDescription(lang, selector) {
    try {
      const parsed = parseSelector(selector);
      const customDescription = parseCustomDescription(parsed);
      if (customDescription)
        return customDescription;
      return innerAsLocators(new generators[lang], parsed, false, 1)[0];
    } catch (e) {
      return selector;
    }
  }
  function locatorCustomDescription(selector) {
    try {
      const parsed = parseSelector(selector);
      return parseCustomDescription(parsed);
    } catch (e) {
      return;
    }
  }
  function parseCustomDescription(parsed) {
    const lastPart = parsed.parts[parsed.parts.length - 1];
    if (lastPart?.name === "internal:describe") {
      const description = JSON.parse(lastPart.body);
      if (typeof description === "string")
        return description;
    }
    return;
  }
  function asLocator(lang, selector, isFrameLocator = false) {
    return asLocators(lang, selector, isFrameLocator, 1)[0];
  }
  function asLocators(lang, selector, isFrameLocator = false, maxOutputSize = 20, preferredQuote) {
    try {
      return innerAsLocators(new generators[lang](preferredQuote), parseSelector(selector), isFrameLocator, maxOutputSize);
    } catch (e) {
      return [selector];
    }
  }
  function innerAsLocators(factory, parsed, isFrameLocator = false, maxOutputSize = 20) {
    const parts = [...parsed.parts];
    const tokens = [];
    let nextBase = isFrameLocator ? "frame-locator" : "page";
    for (let index = 0;index < parts.length; index++) {
      const part = parts[index];
      const base = nextBase;
      nextBase = "locator";
      if (part.name === "internal:describe")
        continue;
      if (part.name === "nth") {
        if (part.body === "0")
          tokens.push([factory.generateLocator(base, "first", ""), factory.generateLocator(base, "nth", "0")]);
        else if (part.body === "-1")
          tokens.push([factory.generateLocator(base, "last", ""), factory.generateLocator(base, "nth", "-1")]);
        else
          tokens.push([factory.generateLocator(base, "nth", part.body)]);
        continue;
      }
      if (part.name === "visible") {
        const tokenList = [];
        if (part.body === "true")
          tokenList.push(factory.generateLocator(base, "visible", ""));
        tokenList.push(factory.generateLocator(base, "filter-visible", part.body), factory.generateLocator(base, "default", `visible=${part.body}`));
        tokens.push(tokenList);
        continue;
      }
      if (part.name === "internal:text") {
        const { exact, text } = detectExact(part.body);
        tokens.push([factory.generateLocator(base, "text", text, { exact })]);
        continue;
      }
      if (part.name === "internal:has-text") {
        const { exact, text } = detectExact(part.body);
        if (!exact) {
          tokens.push([factory.generateLocator(base, "has-text", text, { exact })]);
          continue;
        }
      }
      if (part.name === "internal:has-not-text") {
        const { exact, text } = detectExact(part.body);
        if (!exact) {
          tokens.push([factory.generateLocator(base, "has-not-text", text, { exact })]);
          continue;
        }
      }
      if (part.name === "internal:has") {
        const inners = innerAsLocators(factory, part.body.parsed, false, maxOutputSize);
        tokens.push(inners.map((inner) => factory.generateLocator(base, "has", inner)));
        continue;
      }
      if (part.name === "internal:has-not") {
        const inners = innerAsLocators(factory, part.body.parsed, false, maxOutputSize);
        tokens.push(inners.map((inner) => factory.generateLocator(base, "hasNot", inner)));
        continue;
      }
      if (part.name === "internal:and") {
        const inners = innerAsLocators(factory, part.body.parsed, false, maxOutputSize);
        tokens.push(inners.map((inner) => factory.generateLocator(base, "and", inner)));
        continue;
      }
      if (part.name === "internal:or") {
        const inners = innerAsLocators(factory, part.body.parsed, false, maxOutputSize);
        tokens.push(inners.map((inner) => factory.generateLocator(base, "or", inner)));
        continue;
      }
      if (part.name === "internal:chain") {
        const inners = innerAsLocators(factory, part.body.parsed, false, maxOutputSize);
        tokens.push(inners.map((inner) => factory.generateLocator(base, "chain", inner)));
        continue;
      }
      if (part.name === "internal:label") {
        const { exact, text } = detectExact(part.body);
        tokens.push([factory.generateLocator(base, "label", text, { exact })]);
        continue;
      }
      if (part.name === "internal:role") {
        const attrSelector = parseAttributeSelector(part.body, true);
        const options = { attrs: [] };
        for (const attr of attrSelector.attributes) {
          if (attr.name === "name" || attr.name === "description") {
            if (typeof attr.value === "string") {
              if (options.exact !== undefined && options.exact !== attr.caseSensitive)
                throw new Error(`Conflicting exactness in internal:role selector: ${stringifySelector({ parts: [part] })}`);
              options.exact = attr.caseSensitive;
            }
            options[attr.name] = attr.value;
          } else {
            if (attr.name === "level" && typeof attr.value === "string")
              attr.value = +attr.value;
            options.attrs.push({ name: attr.name === "include-hidden" ? "includeHidden" : attr.name, value: attr.value });
          }
        }
        tokens.push([factory.generateLocator(base, "role", attrSelector.name, options)]);
        continue;
      }
      if (part.name === "internal:testid") {
        const attrSelector = parseAttributeSelector(part.body, true);
        const { value } = attrSelector.attributes[0];
        tokens.push([factory.generateLocator(base, "test-id", value)]);
        continue;
      }
      if (part.name === "internal:attr") {
        const attrSelector = parseAttributeSelector(part.body, true);
        const { name, value, caseSensitive } = attrSelector.attributes[0];
        const text = value;
        const exact = !!caseSensitive;
        if (name === "placeholder") {
          tokens.push([factory.generateLocator(base, "placeholder", text, { exact })]);
          continue;
        }
        if (name === "alt") {
          tokens.push([factory.generateLocator(base, "alt", text, { exact })]);
          continue;
        }
        if (name === "title") {
          tokens.push([factory.generateLocator(base, "title", text, { exact })]);
          continue;
        }
      }
      if (part.name === "internal:control" && part.body === "any-frame") {
        tokens.push([factory.generateLocator(base, "any-frame", "")]);
        nextBase = "frame-locator";
        continue;
      }
      if (part.name === "internal:control" && part.body === "enter-frame") {
        const lastTokens = tokens[tokens.length - 1];
        const lastPart = parts[index - 1];
        const transformed = lastTokens.map((token) => factory.chainLocators([token, factory.generateLocator(base, "frame", "")]));
        if (["xpath", "css"].includes(lastPart.name)) {
          transformed.push(factory.generateLocator(base, "frame-locator", stringifySelector({ parts: [lastPart] })), factory.generateLocator(base, "frame-locator", stringifySelector({ parts: [lastPart] }, true)));
        }
        lastTokens.splice(0, lastTokens.length, ...transformed);
        nextBase = "frame-locator";
        continue;
      }
      const nextPart = parts[index + 1];
      const selectorPart = stringifySelector({ parts: [part] });
      const locatorPart = factory.generateLocator(base, "default", selectorPart);
      if (nextPart && ["internal:has-text", "internal:has-not-text"].includes(nextPart.name)) {
        const { exact, text } = detectExact(nextPart.body);
        if (!exact) {
          const nextLocatorPart = factory.generateLocator("locator", nextPart.name === "internal:has-text" ? "has-text" : "has-not-text", text, { exact });
          const options = {};
          if (nextPart.name === "internal:has-text")
            options.hasText = text;
          else
            options.hasNotText = text;
          const combinedPart = factory.generateLocator(base, "default", selectorPart, options);
          tokens.push([factory.chainLocators([locatorPart, nextLocatorPart]), combinedPart]);
          index++;
          continue;
        }
      }
      let locatorPartWithEngine;
      if (["xpath", "css"].includes(part.name)) {
        const selectorPart = stringifySelector({ parts: [part] }, true);
        locatorPartWithEngine = factory.generateLocator(base, "default", selectorPart);
      }
      tokens.push([locatorPart, locatorPartWithEngine].filter(Boolean));
    }
    return combineTokens(factory, tokens, maxOutputSize);
  }
  function combineTokens(factory, tokens, maxOutputSize) {
    const currentTokens = tokens.map(() => "");
    const result = [];
    const visit = (index) => {
      if (index === tokens.length) {
        result.push(factory.chainLocators(currentTokens));
        return result.length < maxOutputSize;
      }
      for (const taken of tokens[index]) {
        currentTokens[index] = taken;
        if (!visit(index + 1))
          return false;
      }
      return true;
    };
    visit(0);
    return result;
  }
  function detectExact(text) {
    let exact = false;
    const match = text.match(/^\/(.*)\/([dgimsuvy]*)$/);
    if (match)
      return { text: new RegExp(match[1], match[2]) };
    if (text.endsWith('"')) {
      text = JSON.parse(text);
      exact = true;
    } else if (text.endsWith('"s')) {
      text = JSON.parse(text.substring(0, text.length - 1));
      exact = true;
    } else if (text.endsWith('"i')) {
      text = JSON.parse(text.substring(0, text.length - 1));
      exact = false;
    }
    return { exact, text };
  }

  class JavaScriptLocatorFactory {
    preferredQuote;
    constructor(preferredQuote) {
      this.preferredQuote = preferredQuote;
    }
    generateLocator(base, kind, body, options = {}) {
      switch (kind) {
        case "default":
          if (options.hasText !== undefined)
            return `locator(${this.quote(body)}, { hasText: ${this.toHasText(options.hasText)} })`;
          if (options.hasNotText !== undefined)
            return `locator(${this.quote(body)}, { hasNotText: ${this.toHasText(options.hasNotText)} })`;
          return `locator(${this.quote(body)})`;
        case "frame-locator":
          return `frameLocator(${this.quote(body)})`;
        case "frame":
          return `contentFrame()`;
        case "any-frame":
          return `frameLocator()`;
        case "nth":
          return `nth(${body})`;
        case "first":
          return `first()`;
        case "last":
          return `last()`;
        case "visible":
          return `visible()`;
        case "filter-visible":
          return `filter({ visible: ${body === "true" ? "true" : "false"} })`;
        case "role":
          const attrs = [];
          if (isRegExp3(options.name))
            attrs.push(`name: ${this.regexToSourceString(options.name)}`);
          else if (typeof options.name === "string")
            attrs.push(`name: ${this.quote(options.name)}`);
          if (isRegExp3(options.description))
            attrs.push(`description: ${this.regexToSourceString(options.description)}`);
          else if (typeof options.description === "string")
            attrs.push(`description: ${this.quote(options.description)}`);
          if (options.exact && (typeof options.name === "string" || typeof options.description === "string"))
            attrs.push(`exact: true`);
          for (const { name, value } of options.attrs)
            attrs.push(`${name}: ${typeof value === "string" ? this.quote(value) : value}`);
          const attrString = attrs.length ? `, { ${attrs.join(", ")} }` : "";
          return `getByRole(${this.quote(body)}${attrString})`;
        case "has-text":
          return `filter({ hasText: ${this.toHasText(body)} })`;
        case "has-not-text":
          return `filter({ hasNotText: ${this.toHasText(body)} })`;
        case "has":
          return `filter({ has: ${body} })`;
        case "hasNot":
          return `filter({ hasNot: ${body} })`;
        case "and":
          return `and(${body})`;
        case "or":
          return `or(${body})`;
        case "chain":
          return `locator(${body})`;
        case "test-id":
          return `getByTestId(${this.toTestIdValue(body)})`;
        case "text":
          return this.toCallWithExact("getByText", body, !!options.exact);
        case "alt":
          return this.toCallWithExact("getByAltText", body, !!options.exact);
        case "placeholder":
          return this.toCallWithExact("getByPlaceholder", body, !!options.exact);
        case "label":
          return this.toCallWithExact("getByLabel", body, !!options.exact);
        case "title":
          return this.toCallWithExact("getByTitle", body, !!options.exact);
        default:
          throw new Error("Unknown selector kind " + kind);
      }
    }
    chainLocators(locators) {
      return locators.join(".");
    }
    regexToSourceString(re) {
      return normalizeEscapedRegexQuotes(String(re));
    }
    toCallWithExact(method, body, exact) {
      if (isRegExp3(body))
        return `${method}(${this.regexToSourceString(body)})`;
      return exact ? `${method}(${this.quote(body)}, { exact: true })` : `${method}(${this.quote(body)})`;
    }
    toHasText(body) {
      if (isRegExp3(body))
        return this.regexToSourceString(body);
      return this.quote(body);
    }
    toTestIdValue(value) {
      if (isRegExp3(value))
        return this.regexToSourceString(value);
      return this.quote(value);
    }
    quote(text) {
      return escapeWithQuotes(text, this.preferredQuote ?? "'");
    }
  }

  class PythonLocatorFactory {
    generateLocator(base, kind, body, options = {}) {
      switch (kind) {
        case "default":
          if (options.hasText !== undefined)
            return `locator(${this.quote(body)}, has_text=${this.toHasText(options.hasText)})`;
          if (options.hasNotText !== undefined)
            return `locator(${this.quote(body)}, has_not_text=${this.toHasText(options.hasNotText)})`;
          return `locator(${this.quote(body)})`;
        case "frame-locator":
          return `frame_locator(${this.quote(body)})`;
        case "frame":
          return `content_frame`;
        case "any-frame":
          return `frame_locator()`;
        case "nth":
          return `nth(${body})`;
        case "first":
          return `first`;
        case "last":
          return `last`;
        case "visible":
          return `visible`;
        case "filter-visible":
          return `filter(visible=${body === "true" ? "True" : "False"})`;
        case "role":
          const attrs = [];
          if (isRegExp3(options.name))
            attrs.push(`name=${this.regexToString(options.name)}`);
          else if (typeof options.name === "string")
            attrs.push(`name=${this.quote(options.name)}`);
          if (isRegExp3(options.description))
            attrs.push(`description=${this.regexToString(options.description)}`);
          else if (typeof options.description === "string")
            attrs.push(`description=${this.quote(options.description)}`);
          if (options.exact && (typeof options.name === "string" || typeof options.description === "string"))
            attrs.push(`exact=True`);
          for (const { name, value } of options.attrs) {
            let valueString = typeof value === "string" ? this.quote(value) : value;
            if (typeof value === "boolean")
              valueString = value ? "True" : "False";
            attrs.push(`${toSnakeCase(name)}=${valueString}`);
          }
          const attrString = attrs.length ? `, ${attrs.join(", ")}` : "";
          return `get_by_role(${this.quote(body)}${attrString})`;
        case "has-text":
          return `filter(has_text=${this.toHasText(body)})`;
        case "has-not-text":
          return `filter(has_not_text=${this.toHasText(body)})`;
        case "has":
          return `filter(has=${body})`;
        case "hasNot":
          return `filter(has_not=${body})`;
        case "and":
          return `and_(${body})`;
        case "or":
          return `or_(${body})`;
        case "chain":
          return `locator(${body})`;
        case "test-id":
          return `get_by_test_id(${this.toTestIdValue(body)})`;
        case "text":
          return this.toCallWithExact("get_by_text", body, !!options.exact);
        case "alt":
          return this.toCallWithExact("get_by_alt_text", body, !!options.exact);
        case "placeholder":
          return this.toCallWithExact("get_by_placeholder", body, !!options.exact);
        case "label":
          return this.toCallWithExact("get_by_label", body, !!options.exact);
        case "title":
          return this.toCallWithExact("get_by_title", body, !!options.exact);
        default:
          throw new Error("Unknown selector kind " + kind);
      }
    }
    chainLocators(locators) {
      return locators.join(".");
    }
    regexToString(body) {
      const suffix = regexFlagsSuffix(body, "re", regexFlagNames.python);
      return `re.compile(r"${normalizeEscapedRegexQuotes(body.source).replace(/\\\//, "/").replace(/"/g, "\\\"")}"${suffix})`;
    }
    toCallWithExact(method, body, exact) {
      if (isRegExp3(body))
        return `${method}(${this.regexToString(body)})`;
      if (exact)
        return `${method}(${this.quote(body)}, exact=True)`;
      return `${method}(${this.quote(body)})`;
    }
    toHasText(body) {
      if (isRegExp3(body))
        return this.regexToString(body);
      return `${this.quote(body)}`;
    }
    toTestIdValue(value) {
      if (isRegExp3(value))
        return this.regexToString(value);
      return this.quote(value);
    }
    quote(text) {
      return escapeWithQuotes(text, '"');
    }
  }

  class JavaLocatorFactory {
    generateLocator(base, kind, body, options = {}) {
      let clazz;
      switch (base) {
        case "page":
          clazz = "Page";
          break;
        case "frame-locator":
          clazz = "FrameLocator";
          break;
        case "locator":
          clazz = "Locator";
          break;
      }
      switch (kind) {
        case "default":
          if (options.hasText !== undefined)
            return `locator(${this.quote(body)}, new ${clazz}.LocatorOptions().setHasText(${this.toHasText(options.hasText)}))`;
          if (options.hasNotText !== undefined)
            return `locator(${this.quote(body)}, new ${clazz}.LocatorOptions().setHasNotText(${this.toHasText(options.hasNotText)}))`;
          return `locator(${this.quote(body)})`;
        case "frame-locator":
          return `frameLocator(${this.quote(body)})`;
        case "frame":
          return `contentFrame()`;
        case "any-frame":
          return `frameLocator()`;
        case "nth":
          return `nth(${body})`;
        case "first":
          return `first()`;
        case "last":
          return `last()`;
        case "visible":
          return `visible()`;
        case "filter-visible":
          return `filter(new ${clazz}.FilterOptions().setVisible(${body === "true" ? "true" : "false"}))`;
        case "role":
          const attrs = [];
          if (isRegExp3(options.name))
            attrs.push(`.setName(${this.regexToString(options.name)})`);
          else if (typeof options.name === "string")
            attrs.push(`.setName(${this.quote(options.name)})`);
          if (isRegExp3(options.description))
            attrs.push(`.setDescription(${this.regexToString(options.description)})`);
          else if (typeof options.description === "string")
            attrs.push(`.setDescription(${this.quote(options.description)})`);
          if (options.exact && (typeof options.name === "string" || typeof options.description === "string"))
            attrs.push(`.setExact(true)`);
          for (const { name, value } of options.attrs)
            attrs.push(`.set${toTitleCase(name)}(${typeof value === "string" ? this.quote(value) : value})`);
          const attrString = attrs.length ? `, new ${clazz}.GetByRoleOptions()${attrs.join("")}` : "";
          return `getByRole(AriaRole.${toSnakeCase(body).toUpperCase()}${attrString})`;
        case "has-text":
          return `filter(new ${clazz}.FilterOptions().setHasText(${this.toHasText(body)}))`;
        case "has-not-text":
          return `filter(new ${clazz}.FilterOptions().setHasNotText(${this.toHasText(body)}))`;
        case "has":
          return `filter(new ${clazz}.FilterOptions().setHas(${body}))`;
        case "hasNot":
          return `filter(new ${clazz}.FilterOptions().setHasNot(${body}))`;
        case "and":
          return `and(${body})`;
        case "or":
          return `or(${body})`;
        case "chain":
          return `locator(${body})`;
        case "test-id":
          return `getByTestId(${this.toTestIdValue(body)})`;
        case "text":
          return this.toCallWithExact(clazz, "getByText", body, !!options.exact);
        case "alt":
          return this.toCallWithExact(clazz, "getByAltText", body, !!options.exact);
        case "placeholder":
          return this.toCallWithExact(clazz, "getByPlaceholder", body, !!options.exact);
        case "label":
          return this.toCallWithExact(clazz, "getByLabel", body, !!options.exact);
        case "title":
          return this.toCallWithExact(clazz, "getByTitle", body, !!options.exact);
        default:
          throw new Error("Unknown selector kind " + kind);
      }
    }
    chainLocators(locators) {
      return locators.join(".");
    }
    regexToString(body) {
      const suffix = regexFlagsSuffix(body, "Pattern", regexFlagNames.java);
      return `Pattern.compile(${this.quote(normalizeEscapedRegexQuotes(body.source))}${suffix})`;
    }
    toCallWithExact(clazz, method, body, exact) {
      if (isRegExp3(body))
        return `${method}(${this.regexToString(body)})`;
      if (exact)
        return `${method}(${this.quote(body)}, new ${clazz}.${toTitleCase(method)}Options().setExact(true))`;
      return `${method}(${this.quote(body)})`;
    }
    toHasText(body) {
      if (isRegExp3(body))
        return this.regexToString(body);
      return this.quote(body);
    }
    toTestIdValue(value) {
      if (isRegExp3(value))
        return this.regexToString(value);
      return this.quote(value);
    }
    quote(text) {
      return escapeWithQuotes(text, '"');
    }
  }

  class CSharpLocatorFactory {
    generateLocator(base, kind, body, options = {}) {
      switch (kind) {
        case "default":
          if (options.hasText !== undefined)
            return `Locator(${this.quote(body)}, new() { ${this.toHasText(options.hasText)} })`;
          if (options.hasNotText !== undefined)
            return `Locator(${this.quote(body)}, new() { ${this.toHasNotText(options.hasNotText)} })`;
          return `Locator(${this.quote(body)})`;
        case "frame-locator":
          return `FrameLocator(${this.quote(body)})`;
        case "frame":
          return `ContentFrame`;
        case "any-frame":
          return `FrameLocator()`;
        case "nth":
          return `Nth(${body})`;
        case "first":
          return `First`;
        case "last":
          return `Last`;
        case "visible":
          return `Visible`;
        case "filter-visible":
          return `Filter(new() { Visible = ${body === "true" ? "true" : "false"} })`;
        case "role":
          const attrs = [];
          if (isRegExp3(options.name))
            attrs.push(`NameRegex = ${this.regexToString(options.name)}`);
          else if (typeof options.name === "string")
            attrs.push(`Name = ${this.quote(options.name)}`);
          if (isRegExp3(options.description))
            attrs.push(`DescriptionRegex = ${this.regexToString(options.description)}`);
          else if (typeof options.description === "string")
            attrs.push(`Description = ${this.quote(options.description)}`);
          if (options.exact && (typeof options.name === "string" || typeof options.description === "string"))
            attrs.push(`Exact = true`);
          for (const { name, value } of options.attrs)
            attrs.push(`${toTitleCase(name)} = ${typeof value === "string" ? this.quote(value) : value}`);
          const attrString = attrs.length ? `, new() { ${attrs.join(", ")} }` : "";
          return `GetByRole(AriaRole.${toTitleCase(body)}${attrString})`;
        case "has-text":
          return `Filter(new() { ${this.toHasText(body)} })`;
        case "has-not-text":
          return `Filter(new() { ${this.toHasNotText(body)} })`;
        case "has":
          return `Filter(new() { Has = ${body} })`;
        case "hasNot":
          return `Filter(new() { HasNot = ${body} })`;
        case "and":
          return `And(${body})`;
        case "or":
          return `Or(${body})`;
        case "chain":
          return `Locator(${body})`;
        case "test-id":
          return `GetByTestId(${this.toTestIdValue(body)})`;
        case "text":
          return this.toCallWithExact("GetByText", body, !!options.exact);
        case "alt":
          return this.toCallWithExact("GetByAltText", body, !!options.exact);
        case "placeholder":
          return this.toCallWithExact("GetByPlaceholder", body, !!options.exact);
        case "label":
          return this.toCallWithExact("GetByLabel", body, !!options.exact);
        case "title":
          return this.toCallWithExact("GetByTitle", body, !!options.exact);
        default:
          throw new Error("Unknown selector kind " + kind);
      }
    }
    chainLocators(locators) {
      return locators.join(".");
    }
    regexToString(body) {
      const suffix = regexFlagsSuffix(body, "RegexOptions", regexFlagNames.csharp);
      return `new Regex(${this.quote(normalizeEscapedRegexQuotes(body.source))}${suffix})`;
    }
    toCallWithExact(method, body, exact) {
      if (isRegExp3(body))
        return `${method}(${this.regexToString(body)})`;
      if (exact)
        return `${method}(${this.quote(body)}, new() { Exact = true })`;
      return `${method}(${this.quote(body)})`;
    }
    toHasText(body) {
      if (isRegExp3(body))
        return `HasTextRegex = ${this.regexToString(body)}`;
      return `HasText = ${this.quote(body)}`;
    }
    toTestIdValue(value) {
      if (isRegExp3(value))
        return this.regexToString(value);
      return this.quote(value);
    }
    toHasNotText(body) {
      if (isRegExp3(body))
        return `HasNotTextRegex = ${this.regexToString(body)}`;
      return `HasNotText = ${this.quote(body)}`;
    }
    quote(text) {
      return escapeWithQuotes(text, '"');
    }
  }

  class JsonlLocatorFactory {
    generateLocator(base, kind, body, options = {}) {
      return JSON.stringify({
        kind,
        body,
        options
      });
    }
    chainLocators(locators) {
      const objects = locators.map((l) => JSON.parse(l));
      for (let i = 0;i < objects.length - 1; ++i) {
        let tail = objects[i];
        while (tail.next)
          tail = tail.next;
        tail.next = objects[i + 1];
      }
      return JSON.stringify(objects[0]);
    }
  }
  var generators = {
    javascript: JavaScriptLocatorFactory,
    python: PythonLocatorFactory,
    java: JavaLocatorFactory,
    csharp: CSharpLocatorFactory,
    jsonl: JsonlLocatorFactory
  };
  var regexFlagNames = {
    python: { i: "IGNORECASE", m: "MULTILINE", s: "DOTALL" },
    java: { i: "CASE_INSENSITIVE", m: "MULTILINE", s: "DOTALL" },
    csharp: { i: "IgnoreCase", m: "Multiline", s: "Singleline" }
  };
  function regexFlagsSuffix(body, flagsEnum, names) {
    const flags = [...body.flags].filter((flag) => (flag in names)).map((flag) => `${flagsEnum}.${names[flag]}`);
    return flags.length ? `, ${flags.join(" | ")}` : "";
  }
  function isRegExp3(obj) {
    return obj instanceof RegExp;
  }
  // crates/neo-sdk/neo-playwright/third_party/playwright/packages/isomorphic/locatorUtils.ts
  function getByAttributeTextSelector(attrName, text, options) {
    return `internal:attr=[${attrName}=${escapeForAttributeSelector(text, options?.exact || false)}]`;
  }
  function encodeTestIdAttributeName(testIdAttributeName) {
    return testIdAttributeName.includes(",") ? JSON.stringify(testIdAttributeName) : testIdAttributeName;
  }
  function getByTestIdSelector(testIdAttributeName, testId) {
    return `internal:testid=[${encodeTestIdAttributeName(testIdAttributeName)}=${escapeForAttributeSelector(testId, true)}]`;
  }
  function getByLabelSelector(text, options) {
    return "internal:label=" + escapeForTextSelector(text, !!options?.exact);
  }
  function getByAltTextSelector(text, options) {
    return getByAttributeTextSelector("alt", text, options);
  }
  function getByTitleSelector(text, options) {
    return getByAttributeTextSelector("title", text, options);
  }
  function getByPlaceholderSelector(text, options) {
    return getByAttributeTextSelector("placeholder", text, options);
  }
  function getByTextSelector(text, options) {
    return "internal:text=" + escapeForTextSelector(text, !!options?.exact);
  }
  function getByRoleSelector(role, options = {}) {
    const props = [];
    if (options.checked !== undefined)
      props.push(["checked", String(options.checked)]);
    if (options.disabled !== undefined)
      props.push(["disabled", String(options.disabled)]);
    if (options.selected !== undefined)
      props.push(["selected", String(options.selected)]);
    if (options.expanded !== undefined)
      props.push(["expanded", String(options.expanded)]);
    if (options.includeHidden !== undefined)
      props.push(["include-hidden", String(options.includeHidden)]);
    if (options.level !== undefined)
      props.push(["level", String(options.level)]);
    if (options.name !== undefined)
      props.push(["name", escapeForAttributeSelector(options.name, !!options.exact)]);
    if (options.description !== undefined)
      props.push(["description", escapeForAttributeSelector(options.description, !!options.exact)]);
    if (options.pressed !== undefined)
      props.push(["pressed", String(options.pressed)]);
    return `internal:role=${role}${props.map(([n, v]) => `[${n}=${v}]`).join("")}`;
  }
  // crates/neo-sdk/neo-playwright/third_party/playwright/packages/isomorphic/mimeType.ts
  function getMimeTypeForPath(path) {
    const dotIndex = path.lastIndexOf(".");
    if (dotIndex === -1)
      return null;
    const extension = path.substring(dotIndex + 1);
    return types3.get(extension) || null;
  }
  var types3 = new Map([
    ["ez", "application/andrew-inset"],
    ["aw", "application/applixware"],
    ["atom", "application/atom+xml"],
    ["atomcat", "application/atomcat+xml"],
    ["atomdeleted", "application/atomdeleted+xml"],
    ["atomsvc", "application/atomsvc+xml"],
    ["dwd", "application/atsc-dwd+xml"],
    ["held", "application/atsc-held+xml"],
    ["rsat", "application/atsc-rsat+xml"],
    ["bdoc", "application/bdoc"],
    ["xcs", "application/calendar+xml"],
    ["ccxml", "application/ccxml+xml"],
    ["cdfx", "application/cdfx+xml"],
    ["cdmia", "application/cdmi-capability"],
    ["cdmic", "application/cdmi-container"],
    ["cdmid", "application/cdmi-domain"],
    ["cdmio", "application/cdmi-object"],
    ["cdmiq", "application/cdmi-queue"],
    ["cu", "application/cu-seeme"],
    ["mpd", "application/dash+xml"],
    ["davmount", "application/davmount+xml"],
    ["dbk", "application/docbook+xml"],
    ["dssc", "application/dssc+der"],
    ["xdssc", "application/dssc+xml"],
    ["ecma", "application/ecmascript"],
    ["es", "application/ecmascript"],
    ["emma", "application/emma+xml"],
    ["emotionml", "application/emotionml+xml"],
    ["epub", "application/epub+zip"],
    ["exi", "application/exi"],
    ["exp", "application/express"],
    ["fdt", "application/fdt+xml"],
    ["pfr", "application/font-tdpfr"],
    ["geojson", "application/geo+json"],
    ["gml", "application/gml+xml"],
    ["gpx", "application/gpx+xml"],
    ["gxf", "application/gxf"],
    ["gz", "application/gzip"],
    ["hjson", "application/hjson"],
    ["stk", "application/hyperstudio"],
    ["ink", "application/inkml+xml"],
    ["inkml", "application/inkml+xml"],
    ["ipfix", "application/ipfix"],
    ["its", "application/its+xml"],
    ["ear", "application/java-archive"],
    ["jar", "application/java-archive"],
    ["war", "application/java-archive"],
    ["ser", "application/java-serialized-object"],
    ["class", "application/java-vm"],
    ["js", "application/javascript"],
    ["mjs", "application/javascript"],
    ["json", "application/json"],
    ["map", "application/json"],
    ["json5", "application/json5"],
    ["jsonml", "application/jsonml+json"],
    ["jsonld", "application/ld+json"],
    ["lgr", "application/lgr+xml"],
    ["lostxml", "application/lost+xml"],
    ["hqx", "application/mac-binhex40"],
    ["cpt", "application/mac-compactpro"],
    ["mads", "application/mads+xml"],
    ["webmanifest", "application/manifest+json"],
    ["mrc", "application/marc"],
    ["mrcx", "application/marcxml+xml"],
    ["ma", "application/mathematica"],
    ["mb", "application/mathematica"],
    ["nb", "application/mathematica"],
    ["mathml", "application/mathml+xml"],
    ["mbox", "application/mbox"],
    ["mscml", "application/mediaservercontrol+xml"],
    ["metalink", "application/metalink+xml"],
    ["meta4", "application/metalink4+xml"],
    ["mets", "application/mets+xml"],
    ["maei", "application/mmt-aei+xml"],
    ["musd", "application/mmt-usd+xml"],
    ["mods", "application/mods+xml"],
    ["m21", "application/mp21"],
    ["mp21", "application/mp21"],
    ["m4p", "application/mp4"],
    ["mp4s", "application/mp4"],
    ["doc", "application/msword"],
    ["dot", "application/msword"],
    ["mxf", "application/mxf"],
    ["nq", "application/n-quads"],
    ["nt", "application/n-triples"],
    ["cjs", "application/node"],
    ["bin", "application/octet-stream"],
    ["bpk", "application/octet-stream"],
    ["buffer", "application/octet-stream"],
    ["deb", "application/octet-stream"],
    ["deploy", "application/octet-stream"],
    ["dist", "application/octet-stream"],
    ["distz", "application/octet-stream"],
    ["dll", "application/octet-stream"],
    ["dmg", "application/octet-stream"],
    ["dms", "application/octet-stream"],
    ["dump", "application/octet-stream"],
    ["elc", "application/octet-stream"],
    ["exe", "application/octet-stream"],
    ["img", "application/octet-stream"],
    ["iso", "application/octet-stream"],
    ["lrf", "application/octet-stream"],
    ["mar", "application/octet-stream"],
    ["msi", "application/octet-stream"],
    ["msm", "application/octet-stream"],
    ["msp", "application/octet-stream"],
    ["pkg", "application/octet-stream"],
    ["so", "application/octet-stream"],
    ["oda", "application/oda"],
    ["opf", "application/oebps-package+xml"],
    ["ogx", "application/ogg"],
    ["omdoc", "application/omdoc+xml"],
    ["onepkg", "application/onenote"],
    ["onetmp", "application/onenote"],
    ["onetoc", "application/onenote"],
    ["onetoc2", "application/onenote"],
    ["oxps", "application/oxps"],
    ["relo", "application/p2p-overlay+xml"],
    ["xer", "application/patch-ops-error+xml"],
    ["pdf", "application/pdf"],
    ["pgp", "application/pgp-encrypted"],
    ["asc", "application/pgp-signature"],
    ["sig", "application/pgp-signature"],
    ["prf", "application/pics-rules"],
    ["p10", "application/pkcs10"],
    ["p7c", "application/pkcs7-mime"],
    ["p7m", "application/pkcs7-mime"],
    ["p7s", "application/pkcs7-signature"],
    ["p8", "application/pkcs8"],
    ["ac", "application/pkix-attr-cert"],
    ["cer", "application/pkix-cert"],
    ["crl", "application/pkix-crl"],
    ["pkipath", "application/pkix-pkipath"],
    ["pki", "application/pkixcmp"],
    ["pls", "application/pls+xml"],
    ["ai", "application/postscript"],
    ["eps", "application/postscript"],
    ["ps", "application/postscript"],
    ["provx", "application/provenance+xml"],
    ["pskcxml", "application/pskc+xml"],
    ["raml", "application/raml+yaml"],
    ["owl", "application/rdf+xml"],
    ["rdf", "application/rdf+xml"],
    ["rif", "application/reginfo+xml"],
    ["rnc", "application/relax-ng-compact-syntax"],
    ["rl", "application/resource-lists+xml"],
    ["rld", "application/resource-lists-diff+xml"],
    ["rs", "application/rls-services+xml"],
    ["rapd", "application/route-apd+xml"],
    ["sls", "application/route-s-tsid+xml"],
    ["rusd", "application/route-usd+xml"],
    ["gbr", "application/rpki-ghostbusters"],
    ["mft", "application/rpki-manifest"],
    ["roa", "application/rpki-roa"],
    ["rsd", "application/rsd+xml"],
    ["rss", "application/rss+xml"],
    ["rtf", "application/rtf"],
    ["sbml", "application/sbml+xml"],
    ["scq", "application/scvp-cv-request"],
    ["scs", "application/scvp-cv-response"],
    ["spq", "application/scvp-vp-request"],
    ["spp", "application/scvp-vp-response"],
    ["sdp", "application/sdp"],
    ["senmlx", "application/senml+xml"],
    ["sensmlx", "application/sensml+xml"],
    ["setpay", "application/set-payment-initiation"],
    ["setreg", "application/set-registration-initiation"],
    ["shf", "application/shf+xml"],
    ["sieve", "application/sieve"],
    ["siv", "application/sieve"],
    ["smi", "application/smil+xml"],
    ["smil", "application/smil+xml"],
    ["rq", "application/sparql-query"],
    ["srx", "application/sparql-results+xml"],
    ["gram", "application/srgs"],
    ["grxml", "application/srgs+xml"],
    ["sru", "application/sru+xml"],
    ["ssdl", "application/ssdl+xml"],
    ["ssml", "application/ssml+xml"],
    ["swidtag", "application/swid+xml"],
    ["tei", "application/tei+xml"],
    ["teicorpus", "application/tei+xml"],
    ["tfi", "application/thraud+xml"],
    ["tsd", "application/timestamped-data"],
    ["toml", "application/toml"],
    ["trig", "application/trig"],
    ["ttml", "application/ttml+xml"],
    ["ubj", "application/ubjson"],
    ["rsheet", "application/urc-ressheet+xml"],
    ["td", "application/urc-targetdesc+xml"],
    ["vxml", "application/voicexml+xml"],
    ["wasm", "application/wasm"],
    ["wgt", "application/widget"],
    ["hlp", "application/winhlp"],
    ["wsdl", "application/wsdl+xml"],
    ["wspolicy", "application/wspolicy+xml"],
    ["xaml", "application/xaml+xml"],
    ["xav", "application/xcap-att+xml"],
    ["xca", "application/xcap-caps+xml"],
    ["xdf", "application/xcap-diff+xml"],
    ["xel", "application/xcap-el+xml"],
    ["xns", "application/xcap-ns+xml"],
    ["xenc", "application/xenc+xml"],
    ["xht", "application/xhtml+xml"],
    ["xhtml", "application/xhtml+xml"],
    ["xlf", "application/xliff+xml"],
    ["rng", "application/xml"],
    ["xml", "application/xml"],
    ["xsd", "application/xml"],
    ["xsl", "application/xml"],
    ["dtd", "application/xml-dtd"],
    ["xop", "application/xop+xml"],
    ["xpl", "application/xproc+xml"],
    ["*xsl", "application/xslt+xml"],
    ["xslt", "application/xslt+xml"],
    ["xspf", "application/xspf+xml"],
    ["mxml", "application/xv+xml"],
    ["xhvml", "application/xv+xml"],
    ["xvm", "application/xv+xml"],
    ["xvml", "application/xv+xml"],
    ["yang", "application/yang"],
    ["yin", "application/yin+xml"],
    ["zip", "application/zip"],
    ["*3gpp", "audio/3gpp"],
    ["adp", "audio/adpcm"],
    ["amr", "audio/amr"],
    ["au", "audio/basic"],
    ["snd", "audio/basic"],
    ["kar", "audio/midi"],
    ["mid", "audio/midi"],
    ["midi", "audio/midi"],
    ["rmi", "audio/midi"],
    ["mxmf", "audio/mobile-xmf"],
    ["*mp3", "audio/mp3"],
    ["m4a", "audio/mp4"],
    ["mp4a", "audio/mp4"],
    ["m2a", "audio/mpeg"],
    ["m3a", "audio/mpeg"],
    ["mp2", "audio/mpeg"],
    ["mp2a", "audio/mpeg"],
    ["mp3", "audio/mpeg"],
    ["mpga", "audio/mpeg"],
    ["oga", "audio/ogg"],
    ["ogg", "audio/ogg"],
    ["opus", "audio/ogg"],
    ["spx", "audio/ogg"],
    ["s3m", "audio/s3m"],
    ["sil", "audio/silk"],
    ["wav", "audio/wav"],
    ["*wav", "audio/wave"],
    ["weba", "audio/webm"],
    ["xm", "audio/xm"],
    ["ttc", "font/collection"],
    ["otf", "font/otf"],
    ["ttf", "font/ttf"],
    ["woff", "font/woff"],
    ["woff2", "font/woff2"],
    ["exr", "image/aces"],
    ["apng", "image/apng"],
    ["avif", "image/avif"],
    ["bmp", "image/bmp"],
    ["cgm", "image/cgm"],
    ["drle", "image/dicom-rle"],
    ["emf", "image/emf"],
    ["fits", "image/fits"],
    ["g3", "image/g3fax"],
    ["gif", "image/gif"],
    ["heic", "image/heic"],
    ["heics", "image/heic-sequence"],
    ["heif", "image/heif"],
    ["heifs", "image/heif-sequence"],
    ["hej2", "image/hej2k"],
    ["hsj2", "image/hsj2"],
    ["ief", "image/ief"],
    ["jls", "image/jls"],
    ["jp2", "image/jp2"],
    ["jpg2", "image/jp2"],
    ["jpe", "image/jpeg"],
    ["jpeg", "image/jpeg"],
    ["jpg", "image/jpeg"],
    ["jph", "image/jph"],
    ["jhc", "image/jphc"],
    ["jpm", "image/jpm"],
    ["jpf", "image/jpx"],
    ["jpx", "image/jpx"],
    ["jxr", "image/jxr"],
    ["jxra", "image/jxra"],
    ["jxrs", "image/jxrs"],
    ["jxs", "image/jxs"],
    ["jxsc", "image/jxsc"],
    ["jxsi", "image/jxsi"],
    ["jxss", "image/jxss"],
    ["ktx", "image/ktx"],
    ["ktx2", "image/ktx2"],
    ["png", "image/png"],
    ["sgi", "image/sgi"],
    ["svg", "image/svg+xml"],
    ["svgz", "image/svg+xml"],
    ["t38", "image/t38"],
    ["tif", "image/tiff"],
    ["tiff", "image/tiff"],
    ["tfx", "image/tiff-fx"],
    ["webp", "image/webp"],
    ["wmf", "image/wmf"],
    ["disposition-notification", "message/disposition-notification"],
    ["u8msg", "message/global"],
    ["u8dsn", "message/global-delivery-status"],
    ["u8mdn", "message/global-disposition-notification"],
    ["u8hdr", "message/global-headers"],
    ["eml", "message/rfc822"],
    ["mime", "message/rfc822"],
    ["3mf", "model/3mf"],
    ["gltf", "model/gltf+json"],
    ["glb", "model/gltf-binary"],
    ["iges", "model/iges"],
    ["igs", "model/iges"],
    ["mesh", "model/mesh"],
    ["msh", "model/mesh"],
    ["silo", "model/mesh"],
    ["mtl", "model/mtl"],
    ["obj", "model/obj"],
    ["stpx", "model/step+xml"],
    ["stpz", "model/step+zip"],
    ["stpxz", "model/step-xml+zip"],
    ["stl", "model/stl"],
    ["vrml", "model/vrml"],
    ["wrl", "model/vrml"],
    ["*x3db", "model/x3d+binary"],
    ["x3dbz", "model/x3d+binary"],
    ["x3db", "model/x3d+fastinfoset"],
    ["*x3dv", "model/x3d+vrml"],
    ["x3dvz", "model/x3d+vrml"],
    ["x3d", "model/x3d+xml"],
    ["x3dz", "model/x3d+xml"],
    ["x3dv", "model/x3d-vrml"],
    ["appcache", "text/cache-manifest"],
    ["manifest", "text/cache-manifest"],
    ["ics", "text/calendar"],
    ["ifb", "text/calendar"],
    ["coffee", "text/coffeescript"],
    ["litcoffee", "text/coffeescript"],
    ["css", "text/css"],
    ["csv", "text/csv"],
    ["htm", "text/html"],
    ["html", "text/html"],
    ["shtml", "text/html"],
    ["jade", "text/jade"],
    ["jsx", "text/jsx"],
    ["less", "text/less"],
    ["markdown", "text/markdown"],
    ["md", "text/markdown"],
    ["mml", "text/mathml"],
    ["mdx", "text/mdx"],
    ["n3", "text/n3"],
    ["conf", "text/plain"],
    ["def", "text/plain"],
    ["in", "text/plain"],
    ["ini", "text/plain"],
    ["list", "text/plain"],
    ["log", "text/plain"],
    ["text", "text/plain"],
    ["txt", "text/plain"],
    ["rtx", "text/richtext"],
    ["*rtf", "text/rtf"],
    ["sgm", "text/sgml"],
    ["sgml", "text/sgml"],
    ["shex", "text/shex"],
    ["slim", "text/slim"],
    ["slm", "text/slim"],
    ["spdx", "text/spdx"],
    ["styl", "text/stylus"],
    ["stylus", "text/stylus"],
    ["tsv", "text/tab-separated-values"],
    ["man", "text/troff"],
    ["me", "text/troff"],
    ["ms", "text/troff"],
    ["roff", "text/troff"],
    ["t", "text/troff"],
    ["tr", "text/troff"],
    ["ttl", "text/turtle"],
    ["uri", "text/uri-list"],
    ["uris", "text/uri-list"],
    ["urls", "text/uri-list"],
    ["vcard", "text/vcard"],
    ["vtt", "text/vtt"],
    ["*xml", "text/xml"],
    ["yaml", "text/yaml"],
    ["yml", "text/yaml"],
    ["3gp", "video/3gpp"],
    ["3gpp", "video/3gpp"],
    ["3g2", "video/3gpp2"],
    ["h261", "video/h261"],
    ["h263", "video/h263"],
    ["h264", "video/h264"],
    ["m4s", "video/iso.segment"],
    ["jpgv", "video/jpeg"],
    ["jpm", "video/jpm"],
    ["jpgm", "video/jpm"],
    ["mj2", "video/mj2"],
    ["mjp2", "video/mj2"],
    ["ts", "application/typescript"],
    ["mp4", "video/mp4"],
    ["mp4v", "video/mp4"],
    ["mpg4", "video/mp4"],
    ["m1v", "video/mpeg"],
    ["m2v", "video/mpeg"],
    ["mpe", "video/mpeg"],
    ["mpeg", "video/mpeg"],
    ["mpg", "video/mpeg"],
    ["ogv", "video/ogg"],
    ["mov", "video/quicktime"],
    ["qt", "video/quicktime"],
    ["webm", "video/webm"]
  ]);
  // crates/neo-sdk/neo-playwright/third_party/playwright/packages/isomorphic/urlMatch.ts
  var escapedChars = new Set(["$", "^", "+", ".", "*", "(", ")", "|", "\\", "?", "{", "}", "[", "]"]);
  function globToRegexPattern(glob) {
    const tokens = ["^"];
    let inGroup = false;
    for (let i = 0;i < glob.length; ++i) {
      const c = glob[i];
      if (c === "\\" && i + 1 < glob.length) {
        const char = glob[++i];
        tokens.push(escapedChars.has(char) ? "\\" + char : char);
        continue;
      }
      if (c === "*") {
        const charBefore = glob[i - 1];
        let starCount = 1;
        while (glob[i + 1] === "*") {
          starCount++;
          i++;
        }
        if (starCount > 1) {
          const charAfter = glob[i + 1];
          if (charAfter === "/") {
            if (charBefore === "/")
              tokens.push("((.+/)|)");
            else
              tokens.push("(.*/)");
            ++i;
          } else {
            tokens.push("(.*)");
          }
        } else {
          tokens.push("([^/]*)");
        }
        continue;
      }
      switch (c) {
        case "{":
          if (inGroup)
            throw new Error(`Invalid glob pattern ${JSON.stringify(glob)}: nested '{' is not supported`);
          inGroup = true;
          tokens.push("(");
          break;
        case "}":
          if (!inGroup)
            throw new Error(`Invalid glob pattern ${JSON.stringify(glob)}: unmatched '}'`);
          inGroup = false;
          tokens.push(")");
          break;
        case ",":
          if (inGroup) {
            tokens.push("|");
            break;
          }
          tokens.push("\\" + c);
          break;
        default:
          tokens.push(escapedChars.has(c) ? "\\" + c : c);
      }
    }
    if (inGroup)
      throw new Error(`Invalid glob pattern ${JSON.stringify(glob)}: unmatched '{'`);
    tokens.push("$");
    return tokens.join("");
  }
  // crates/neo-sdk/neo-playwright-js/src/prelude/selectors.ts
  function withLocatorOptions(api, frame, selector, options) {
    let result = selector;
    if (options?.hasText)
      result += ` >> internal:has-text=${escapeForTextSelector(options.hasText, false)}`;
    if (options?.hasNotText)
      result += ` >> internal:has-not-text=${escapeForTextSelector(options.hasNotText, false)}`;
    if (options?.has) {
      if (options.has._frame !== frame)
        throw plainError(api, 'Inner "has" locator must belong to the same frame.');
      result += ` >> internal:has=${JSON.stringify(options.has._selector)}`;
    }
    if (options?.hasNot) {
      if (options.hasNot._frame !== frame)
        throw plainError(api, 'Inner "hasNot" locator must belong to the same frame.');
      result += ` >> internal:has-not=${JSON.stringify(options.hasNot._selector)}`;
    }
    if (options?.visible !== undefined)
      result += ` >> visible=${options.visible ? "true" : "false"}`;
    return result;
  }
  function locatorBrand(api, page, frame, selector) {
    let split;
    try {
      split = splitSelectorByFrame(selector);
    } catch (error) {
      throw clientError(api, error instanceof Error ? error.message : String(error));
    }
    if (split.anyFrame)
      throw unsupportedError(api, "frameLocator() needs a selector in Neo, such as frameLocator('iframe') or locator('#pay').contentFrame()");
    return {
      $neo: "locator",
      page,
      frame,
      chunks: JSON.parse(JSON.stringify(split.chunks)),
      display: asLocator("javascript", selector)
    };
  }

  // crates/neo-sdk/neo-playwright-js/src/prelude/signal.ts
  function signalOf(api, options) {
    if (typeof options !== "object" || options === null)
      return;
    const signal = Reflect.get(options, "signal");
    if (signal === undefined)
      return;
    if (typeof signal !== "object" || signal === null || typeof Reflect.get(signal, "aborted") !== "boolean" || typeof Reflect.get(signal, "addEventListener") !== "function" || typeof Reflect.get(signal, "removeEventListener") !== "function")
      throw clientError(api, "signal: expected an AbortSignal");
    if (signal.aborted)
      throw abortedError(api, signal);
    return signal;
  }
  async function untilAborted(signal, aborted, work) {
    if (signal === undefined)
      return await work();
    if (signal.aborted)
      throw aborted();
    let detach = () => {};
    const abort = new Promise((_, reject) => {
      const listener = () => reject(aborted());
      signal.addEventListener("abort", listener, { once: true });
      detach = () => signal.removeEventListener("abort", listener);
    });
    try {
      return await Promise.race([work(), abort]);
    } finally {
      detach();
    }
  }

  // crates/neo-sdk/neo-playwright-js/src/prelude/locator.ts
  function testIdAttributeName() {
    return "data-testid";
  }

  class Locator {
    _frame;
    _selector;
    constructor(frame, selector) {
      this._frame = frame;
      this._selector = selector;
      markHandle(this);
    }
    page() {
      return this._frame.page();
    }
    toString() {
      return asLocatorDescription("javascript", this._selector);
    }
    [inspectCustom]() {
      return this.toString();
    }
    description() {
      return locatorCustomDescription(this._selector) ?? null;
    }
    locator(selectorOrLocator, options) {
      const api = "locator.locator";
      if (typeof selectorOrLocator === "string")
        return new Locator(this._frame, withLocatorOptions(api, this._frame, `${this._selector} >> ${selectorOrLocator}`, options));
      if (selectorOrLocator._frame !== this._frame)
        throw plainError(api, "Locators must belong to the same frame.");
      return new Locator(this._frame, withLocatorOptions(api, this._frame, `${this._selector} >> internal:chain=${JSON.stringify(selectorOrLocator._selector)}`, options));
    }
    within(locator) {
      return locator.locator(this);
    }
    getByTestId(testId) {
      return this.locator(getByTestIdSelector(testIdAttributeName(), testId));
    }
    getByAltText(text, options) {
      return this.locator(getByAltTextSelector(text, options));
    }
    getByLabel(text, options) {
      return this.locator(getByLabelSelector(text, options));
    }
    getByPlaceholder(text, options) {
      return this.locator(getByPlaceholderSelector(text, options));
    }
    getByText(text, options) {
      return this.locator(getByTextSelector(text, options));
    }
    getByTitle(text, options) {
      return this.locator(getByTitleSelector(text, options));
    }
    getByRole(role, options = {}) {
      return this.locator(getByRoleSelector(role, options));
    }
    frameLocator(selector) {
      return new FrameLocator(this._frame, `${this._selector} >> ${selector}`);
    }
    contentFrame() {
      return new FrameLocator(this._frame, this._selector);
    }
    filter(options) {
      return new Locator(this._frame, withLocatorOptions("locator.filter", this._frame, this._selector, options));
    }
    visible() {
      return new Locator(this._frame, withLocatorOptions("locator.visible", this._frame, this._selector, {
        visible: true
      }));
    }
    describe(description) {
      return new Locator(this._frame, `${this._selector} >> internal:describe=${JSON.stringify(description)}`);
    }
    first() {
      return new Locator(this._frame, `${this._selector} >> nth=0`);
    }
    last() {
      return new Locator(this._frame, `${this._selector} >> nth=-1`);
    }
    nth(index) {
      return new Locator(this._frame, `${this._selector} >> nth=${index}`);
    }
    and(locator) {
      if (locator._frame !== this._frame)
        throw plainError("locator.and", "Locators must belong to the same frame.");
      return new Locator(this._frame, `${this._selector} >> internal:and=${JSON.stringify(locator._selector)}`);
    }
    or(locator) {
      if (locator._frame !== this._frame)
        throw plainError("locator.or", "Locators must belong to the same frame.");
      return new Locator(this._frame, `${this._selector} >> internal:or=${JSON.stringify(locator._selector)}`);
    }
    click(options = {}) {
      return this._frame._click("locator.click", this._selector, {
        ...options,
        strict: true
      });
    }
    dblclick(options = {}) {
      return this._frame._dblclick("locator.dblclick", this._selector, {
        ...options,
        strict: true
      });
    }
    hover(options = {}) {
      return this._frame._hover("locator.hover", this._selector, {
        ...options,
        strict: true
      });
    }
    check(options = {}) {
      return this._frame._check("locator.check", this._selector, true, {
        ...options,
        strict: true
      });
    }
    uncheck(options = {}) {
      return this._frame._check("locator.uncheck", this._selector, false, {
        ...options,
        strict: true
      });
    }
    setChecked(checked, options = {}) {
      return this._frame._check("locator.setChecked", this._selector, checked, {
        ...options,
        strict: true
      });
    }
    dragTo(target, options = {}) {
      return this._frame._dragAndDrop("locator.dragTo", this._selector, target._selector, {
        ...options,
        strict: true
      });
    }
    fill(value, options = {}) {
      return this._frame._fill("locator.fill", this._selector, value, {
        ...options,
        strict: true
      });
    }
    clear(options = {}) {
      return this._frame._fill("locator.clear", this._selector, "", {
        ...options,
        strict: true
      });
    }
    press(key, options = {}) {
      return this._frame._press("locator.press", this._selector, key, {
        ...options,
        strict: true
      });
    }
    pressSequentially(text, options = {}) {
      return this._frame._type("locator.pressSequentially", this._selector, text, {
        ...options,
        strict: true
      });
    }
    type(text, options = {}) {
      return this._frame._type("locator.type", this._selector, text, {
        ...options,
        strict: true
      });
    }
    selectOption(values, options = {}) {
      return this._frame._selectOption("locator.selectOption", this._selector, values, {
        ...options,
        strict: true
      });
    }
    selectText(options = {}) {
      return this._frame._selectText("locator.selectText", this._selector, {
        ...options,
        strict: true
      });
    }
    focus(options = {}) {
      return this._frame._focus("locator.focus", this._selector, {
        ...options,
        strict: true
      });
    }
    blur(options = {}) {
      return this._frame._blur("locator.blur", this._selector, {
        ...options,
        strict: true
      });
    }
    drop(payload, options = {}) {
      return this._frame._drop(this._selector, payload, options);
    }
    setInputFiles(files, options = {}) {
      return this._frame._setInputFiles("locator.setInputFiles", this._selector, files, {
        ...options,
        strict: true
      });
    }
    dispatchEvent(type, eventInit = {}, options = {}) {
      return this._frame._dispatchEvent("locator.dispatchEvent", this._selector, type, eventInit, {
        ...options,
        strict: true
      });
    }
    scrollIntoViewIfNeeded(options = {}) {
      return this._frame._scrollIntoViewIfNeeded("locator.scrollIntoViewIfNeeded", this._selector, {
        ...options,
        strict: true
      });
    }
    textContent(options = {}) {
      return this._frame._read("locator.textContent", "locator.textContent", this._selector, {
        ...options,
        strict: true
      });
    }
    innerText(options = {}) {
      return this._frame._read("locator.innerText", "locator.innerText", this._selector, {
        ...options,
        strict: true
      });
    }
    innerHTML(options = {}) {
      return this._frame._read("locator.innerHTML", "locator.innerHTML", this._selector, {
        ...options,
        strict: true
      });
    }
    inputValue(options = {}) {
      return this._frame._read("locator.inputValue", "locator.inputValue", this._selector, {
        ...options,
        strict: true
      });
    }
    getAttribute(name, options = {}) {
      return this._frame._getAttribute("locator.getAttribute", this._selector, name, {
        ...options,
        strict: true
      });
    }
    isChecked(options = {}) {
      return this._frame._read("locator.isChecked", "locator.isChecked", this._selector, {
        ...options,
        strict: true
      });
    }
    isDisabled(options = {}) {
      return this._frame._read("locator.isDisabled", "locator.isDisabled", this._selector, {
        ...options,
        strict: true
      });
    }
    isEditable(options = {}) {
      return this._frame._read("locator.isEditable", "locator.isEditable", this._selector, {
        ...options,
        strict: true
      });
    }
    isEnabled(options = {}) {
      return this._frame._read("locator.isEnabled", "locator.isEnabled", this._selector, {
        ...options,
        strict: true
      });
    }
    isHidden(_options) {
      return this._frame._isVisible("locator.isHidden", "locator.isHidden", this._selector, {
        strict: true
      });
    }
    isVisible(_options) {
      return this._frame._isVisible("locator.isVisible", "locator.isVisible", this._selector, {
        strict: true
      });
    }
    count() {
      return this._frame._everyMatch("locator.count", "locator.count", this._selector);
    }
    async normalize() {
      const resolved = await this._frame._normalize("locator.normalize", this._selector);
      return new Locator(this._frame, resolved);
    }
    async all() {
      const count = await this._frame._everyMatch("locator.count", "locator.all", this._selector);
      return Array.from({ length: count }, (_, index) => this.nth(index));
    }
    allTextContents() {
      return this._frame._everyMatch("locator.allTextContents", "locator.allTextContents", this._selector);
    }
    allInnerTexts() {
      return this._frame._everyMatch("locator.allInnerTexts", "locator.allInnerTexts", this._selector);
    }
    boundingBox(options = {}) {
      return this._frame._boundingBox("locator.boundingBox", this._selector, options);
    }
    async waitFor(options = {}) {
      await this._frame._waitForSelector("locator.waitFor", this._selector, { ...options, strict: true }, true);
    }
    async elementHandle(options = {}) {
      const handle = await this._frame._elementHandle("locator.elementHandle", this._selector, true, false, options);
      if (!handle)
        throw internalError("locator.elementHandle", `Could not resolve ${this} to DOM Element`);
      return handle;
    }
    elementHandles() {
      return this._frame._elementHandles("locator.elementHandles", this._selector);
    }
    async evaluate(pageFunction, arg, options) {
      const api = "locator.evaluate";
      signalOf(api, options);
      const script = await this._frame._page._evaluationScript(api, "locator", pageFunction, arg, options);
      return await this._frame._evaluateOn(api, this._selector, true, script, false, typeof options === "object" && options !== null ? options : {});
    }
    async waitForFunction(pageFunction, arg, options) {
      const api = "locator.waitForFunction";
      const script = pageCall(api, "query", pageFunction, arg);
      await this._frame._selectorOp("locator.waitForFunction", api, this._selector, true, this._frame._wait(api, options ?? {}), { script });
    }
    async evaluateAll(pageFunction, arg) {
      const api = "locator.evaluateAll";
      return await this._frame._evaluateAll(api, this._selector, pageCall(api, "query", pageFunction, arg));
    }
    ariaSnapshot(options) {
      return this._frame._ariaSnapshot("locator.ariaSnapshot", this._selector, options);
    }
    ariaSnapshotJSON(options) {
      return this._frame._ariaSnapshotJSON("locator.ariaSnapshotJSON", this._selector, options);
    }
    screenshot(options) {
      return this._frame._screenshot("locator.screenshot", this._selector, options);
    }
  }

  class FrameLocator {
    _frame;
    _frameSelector;
    constructor(frame, selector) {
      this._frame = frame;
      this._frameSelector = selector;
      markHandle(this);
    }
    #childSelector(selector) {
      if (this._frameSelector === kAnyFrameSelector)
        return `${this._frameSelector} >> ${selector}`;
      return `${this._frameSelector} >> internal:control=enter-frame >> ${selector}`;
    }
    locator(selectorOrLocator, options) {
      const api = "frameLocator.locator";
      if (typeof selectorOrLocator === "string")
        return new Locator(this._frame, withLocatorOptions(api, this._frame, this.#childSelector(selectorOrLocator), options));
      if (selectorOrLocator._frame !== this._frame)
        throw plainError(api, "Locators must belong to the same frame.");
      return new Locator(this._frame, withLocatorOptions(api, this._frame, this.#childSelector(selectorOrLocator._selector), options));
    }
    getByTestId(testId) {
      return this.locator(getByTestIdSelector(testIdAttributeName(), testId));
    }
    getByAltText(text, options) {
      return this.locator(getByAltTextSelector(text, options));
    }
    getByLabel(text, options) {
      return this.locator(getByLabelSelector(text, options));
    }
    getByPlaceholder(text, options) {
      return this.locator(getByPlaceholderSelector(text, options));
    }
    getByText(text, options) {
      return this.locator(getByTextSelector(text, options));
    }
    getByTitle(text, options) {
      return this.locator(getByTitleSelector(text, options));
    }
    getByRole(role, options = {}) {
      return this.locator(getByRoleSelector(role, options));
    }
    owner() {
      return new Locator(this._frame, this._frameSelector);
    }
    frameLocator(selector) {
      return new FrameLocator(this._frame, this.#childSelector(selector));
    }
    #nthSelector(api, nth) {
      if (this._frameSelector === kAnyFrameSelector)
        throw plainError(api, "Selecting the nth frame is not allowed on frameLocator().");
      return `${this._frameSelector} >> nth=${nth}`;
    }
    first() {
      return new FrameLocator(this._frame, this.#nthSelector("frameLocator.first", "0"));
    }
    last() {
      return new FrameLocator(this._frame, this.#nthSelector("frameLocator.last", "-1"));
    }
    nth(index) {
      return new FrameLocator(this._frame, this.#nthSelector("frameLocator.nth", String(index)));
    }
  }

  // crates/neo-sdk/neo-playwright-js/src/prelude/screenshot.ts
  function screenshotType({
    api,
    path
  }) {
    const mime = getMimeTypeForPath(path.toLowerCase());
    if (mime === "image/png")
      return "png";
    if (mime === "image/jpeg")
      return "jpeg";
    if (mime === "image/webp")
      return "webp";
    throw plainError(api, `path: unsupported mime type "${mime}"`);
  }
  async function saveScreenshot({
    api,
    bytes,
    path,
    signal
  }) {
    if (!path)
      return;
    await invoke("page.saveScreenshot", { api, timeoutMs: 0 }, { path, base64: decode(bytes, "base64") }, signal);
  }

  // crates/neo-sdk/neo-playwright-js/src/prelude/snapshot.ts
  function comment(partial) {
    return `# partial: ${partial.frame} not read (${partial.reason})`;
  }
  function iframeLine(lines, lineToNode, ref) {
    const tag = `[ref=${ref}]`;
    for (const [index, node] of lineToNode)
      if (node.ref === ref && lines[index]?.includes(tag))
        return index;
    return lines.findIndex((line) => /^\s*- /.test(line) && line.includes(tag));
  }
  function renderSnapshot(result) {
    const json = result.json;
    const lineToNode = new Map;
    const yaml = renderAriaSnapshotAsYaml(json, { lineToNode });
    if (result.partial.length === 0)
      return yaml;
    const lines = yaml === "" ? [] : yaml.split(`
`);
    const placed = result.partial.map((partial) => ({
      partial,
      line: iframeLine(lines, lineToNode, partial.ref)
    }));
    const unplaced = [];
    for (const { partial, line } of placed.sort((a, b) => b.line - a.line)) {
      if (line === -1) {
        unplaced.unshift(comment(partial));
        continue;
      }
      const indent = /^\s*/.exec(lines[line] ?? "")?.[0] ?? "";
      lines.splice(line + 1, 0, `${indent}  ${comment(partial)}`);
    }
    return [...lines, ...unplaced].join(`
`);
  }

  // crates/neo-sdk/neo-playwright-js/src/prelude/timeouts.ts
  var DEFAULT_TIMEOUT = 5000;
  var DEFAULT_NAVIGATION_TIMEOUT = 30000;
  function timeoutMs(api, timeout) {
    if (!Number.isFinite(timeout) || timeout < 0)
      throw clientError(api, `timeout: expected a non-negative number, got ${timeout}`);
    return Math.ceil(timeout);
  }

  class TimeoutSettings {
    parent;
    defaultTimeout;
    defaultNavigationTimeout;
    constructor(parent) {
      this.parent = parent;
    }
    setDefaultTimeout(timeout) {
      this.defaultTimeout = timeout;
    }
    setDefaultNavigationTimeout(timeout) {
      this.defaultNavigationTimeout = timeout;
    }
    timeout(options) {
      if (typeof options?.timeout === "number")
        return options.timeout;
      return this.inherited();
    }
    navigationTimeout(options) {
      if (typeof options?.timeout === "number")
        return options.timeout;
      return this.inheritedNavigation();
    }
    inherited() {
      return this.defaultTimeout ?? this.parent?.inherited() ?? DEFAULT_TIMEOUT;
    }
    inheritedNavigation() {
      return this.defaultNavigationTimeout ?? this.defaultTimeout ?? this.parent?.inheritedNavigation() ?? DEFAULT_NAVIGATION_TIMEOUT;
    }
  }

  // crates/neo-sdk/neo-playwright-js/src/prelude/url.ts
  function normalizeUrl(text) {
    return URL.parse(text)?.href ?? text;
  }
  function globMatches(glob, url) {
    if (glob === "")
      return true;
    return new RegExp(globToRegexPattern(glob)).test(url);
  }

  // crates/neo-sdk/neo-playwright-js/src/prelude/frame.ts
  function knownLoadState(state) {
    if (state === "networkidle0")
      return "networkidle";
    if (state === "load" || state === "domcontentloaded" || state === "networkidle" || state === "commit")
      return state;
    return null;
  }
  function loadStateError(name) {
    return `${name}: expected one of (load|domcontentloaded|networkidle|commit)`;
  }
  function loadState(api, name, state) {
    const known = knownLoadState(state);
    if (known === null)
      throw plainError(api, loadStateError(name));
    return known;
  }

  class Frame extends EventEmitter {
    _page;
    _id;
    _name = "";
    _url = "";
    _parentFrame;
    _detached = false;
    #eventView = null;
    constructor(page, id, parent) {
      super();
      this._page = page;
      this._id = id;
      this._parentFrame = parent;
      markHandle(this);
    }
    page() {
      return this._page;
    }
    name() {
      return this.#eventView?.info.name ?? this._name;
    }
    url() {
      return this.#eventView?.info.url ?? (this._id === null ? this._page._url : this._url);
    }
    _duringEvent(info, event, read) {
      const previous = this.#eventView;
      this.#eventView = { info, detached: event === "framedetached" };
      try {
        return read();
      } finally {
        this.#eventView = previous;
      }
    }
    parentFrame() {
      return this._parentFrame;
    }
    childFrames() {
      return this._page.frames().filter((frame) => frame._parentFrame === this);
    }
    isDetached() {
      return this.#eventView?.detached ?? this._detached;
    }
    toJSON() {
      return { name: this.name(), url: this.url() };
    }
    locator(selector, options) {
      return this._locator("frame.locator", selector, options);
    }
    _locator(api, selector, options) {
      return new Locator(this, withLocatorOptions(api, this, selector, options));
    }
    getByTestId(testId) {
      return this.locator(getByTestIdSelector(testIdAttributeName(), testId));
    }
    getByAltText(text, options) {
      return this.locator(getByAltTextSelector(text, options));
    }
    getByLabel(text, options) {
      return this.locator(getByLabelSelector(text, options));
    }
    getByPlaceholder(text, options) {
      return this.locator(getByPlaceholderSelector(text, options));
    }
    getByText(text, options) {
      return this.locator(getByTextSelector(text, options));
    }
    getByTitle(text, options) {
      return this.locator(getByTitleSelector(text, options));
    }
    getByRole(role, options = {}) {
      return this.locator(getByRoleSelector(role, options));
    }
    frameLocator(selector) {
      return new FrameLocator(this, selector === undefined ? kAnyFrameSelector : selector);
    }
    click(selector, options) {
      return this._click("frame.click", selector, options);
    }
    dblclick(selector, options) {
      return this._dblclick("frame.dblclick", selector, options);
    }
    hover(selector, options) {
      return this._hover("frame.hover", selector, options);
    }
    check(selector, options) {
      return this._check("frame.check", selector, true, options);
    }
    uncheck(selector, options) {
      return this._check("frame.uncheck", selector, false, options);
    }
    setChecked(selector, checked, options) {
      return this._check("frame.setChecked", selector, checked, options);
    }
    dragAndDrop(source, target, options) {
      return this._dragAndDrop("frame.dragAndDrop", source, target, options);
    }
    fill(selector, value, options) {
      return this._fill("frame.fill", selector, value, options);
    }
    press(selector, key, options) {
      return this._press("frame.press", selector, key, options);
    }
    type(selector, text, options) {
      return this._type("frame.type", selector, text, options);
    }
    focus(selector, options) {
      return this._focus("frame.focus", selector, options);
    }
    selectOption(selector, values, options) {
      return this._selectOption("frame.selectOption", selector, values, options);
    }
    setInputFiles(selector, files, options) {
      return this._setInputFiles("frame.setInputFiles", selector, files, options);
    }
    dispatchEvent(selector, type, eventInit, options) {
      return this._dispatchEvent("frame.dispatchEvent", selector, type, eventInit, options);
    }
    textContent(selector, options) {
      return this._read("locator.textContent", "frame.textContent", selector, options);
    }
    innerText(selector, options) {
      return this._read("locator.innerText", "frame.innerText", selector, options);
    }
    innerHTML(selector, options) {
      return this._read("locator.innerHTML", "frame.innerHTML", selector, options);
    }
    getAttribute(selector, name, options) {
      return this._getAttribute("frame.getAttribute", selector, name, options);
    }
    inputValue(selector, options) {
      return this._read("locator.inputValue", "frame.inputValue", selector, options);
    }
    isChecked(selector, options) {
      return this._read("locator.isChecked", "frame.isChecked", selector, options);
    }
    isDisabled(selector, options) {
      return this._read("locator.isDisabled", "frame.isDisabled", selector, options);
    }
    isEditable(selector, options) {
      return this._read("locator.isEditable", "frame.isEditable", selector, options);
    }
    isEnabled(selector, options) {
      return this._read("locator.isEnabled", "frame.isEnabled", selector, options);
    }
    isHidden(selector, options) {
      return this._isVisible("locator.isHidden", "frame.isHidden", selector, options);
    }
    isVisible(selector, options) {
      return this._isVisible("locator.isVisible", "frame.isVisible", selector, options);
    }
    $(selector, options) {
      return this._elementHandle("frame.$", selector, options?.strict === true, true);
    }
    $$(selector) {
      return this._elementHandles("frame.$$", selector);
    }
    async $eval(selector, pageFunction, arg) {
      assertMaxArguments("frame.$eval", arguments.length, 3);
      const api = "frame.$eval";
      return await this._evaluateOn(api, selector, false, pageCall(api, "query", pageFunction, arg), true);
    }
    async $$eval(selector, pageFunction, arg) {
      assertMaxArguments("frame.$$eval", arguments.length, 3);
      const api = "frame.$$eval";
      return await this._evaluateAll(api, selector, pageCall(api, "query", pageFunction, arg));
    }
    waitForSelector(selector, options) {
      return this._waitForSelector("frame.waitForSelector", selector, options, false);
    }
    goto(url, options) {
      return this._goto("frame.goto", url, options);
    }
    waitForLoadState(state, options) {
      return this._waitForLoadState("frame.waitForLoadState", state, options);
    }
    waitForNavigation(options) {
      return this._page._waitForNavigation("frame.waitForNavigation", this, options);
    }
    waitForURL(url, options) {
      return this._page._waitForURL("frame.waitForURL", this, url, options);
    }
    async title() {
      const api = "frame.title";
      return await invoke("page.title", this._call(api, 0), await this._frameArgs(api));
    }
    content() {
      return this._content("frame.content");
    }
    addScriptTag(options = {}) {
      return this._addTag("frame.addScriptTag", "page.addScriptTag", options);
    }
    addStyleTag(options = {}) {
      return this._addTag("frame.addStyleTag", "page.addStyleTag", options);
    }
    setContent(html, options) {
      return this._setContent("frame.setContent", html, options);
    }
    async evaluate(pageFunction, arg, options) {
      assertMaxArguments("frame.evaluate", arguments.length, 3);
      return await this._evaluate("frame.evaluate", pageFunction, arg, options);
    }
    waitForFunction(pageFunction, arg, options) {
      return this._waitForFunction("frame.waitForFunction", pageFunction, arg, options);
    }
    async waitForTimeout(timeout) {
      await sleep("frame.waitForTimeout", timeout);
      await this._page._context._refreshWorkingFrames("frame.waitForTimeout");
    }
    _call(api, timeout) {
      return { api, timeoutMs: timeoutMs(api, timeout) };
    }
    _timeout(options) {
      return this._page._timeoutSettings.timeout(options);
    }
    _wait(api, options) {
      return { timeout: this._timeout(options), signal: signalOf(api, options) };
    }
    _navigationTimeout(options) {
      return this._page._timeoutSettings.navigationTimeout(options);
    }
    async _frameArgs(api) {
      const page = await this._page._brand(api);
      return this._id === null ? { page } : { page, frame: this._id };
    }
    async _locatorBrand(api, selector) {
      if (typeof selector !== "string")
        throw clientError(api, `selector: expected string, got ${typeof selector}`);
      return locatorBrand(api, await this._page._brand(api), this._id, selector);
    }
    async _selectorOp(op, api, selector, strict, wait, args) {
      const target = await this._locatorBrand(api, selector);
      const input = {
        target,
        strict: strict === true ? "strict" : "firstMatch",
        ...args
      };
      return await invoke(op, this._call(api, wait.timeout), input, wait.signal);
    }
    async _click(api, selector, options = {}) {
      await this._selectorOp("locator.click", api, selector, options.strict, this._wait(api, options), {
        options: pick(options, POINTER_KEYS)
      });
    }
    async _dblclick(api, selector, options = {}) {
      await this._selectorOp("locator.dblclick", api, selector, options.strict, this._wait(api, options), {
        options: pick(options, DBLCLICK_KEYS)
      });
    }
    async _hover(api, selector, options = {}) {
      await this._selectorOp("locator.hover", api, selector, options.strict, this._wait(api, options), {
        options: pick(options, HOVER_KEYS)
      });
    }
    async _check(api, selector, checked, options = {}) {
      const op = checked ? "locator.check" : "locator.uncheck";
      await this._selectorOp(op, api, selector, options.strict, this._wait(api, options), {
        options: pick(options, CHECK_KEYS)
      });
    }
    async _dragAndDrop(api, source, target, options = {}) {
      const wait = this._wait(api, options);
      const to = await this._locatorBrand(api, target);
      await this._selectorOp("locator.dragTo", api, source, options.strict, wait, {
        to,
        options: pick(options, DRAG_KEYS)
      });
    }
    async _fill(api, selector, value, options = {}) {
      if (typeof value !== "string")
        throw clientError(api, `value: expected string, got ${typeof value}`);
      await this._selectorOp("locator.fill", api, selector, options.strict, this._wait(api, options), {
        value,
        options: pick(options, FILL_KEYS)
      });
    }
    async _press(api, selector, key, options = {}) {
      await this._selectorOp("locator.press", api, selector, options.strict, this._wait(api, options), {
        key,
        options: pick(options, KEY_KEYS)
      });
    }
    async _type(api, selector, text, options = {}) {
      await this._selectorOp("locator.pressSequentially", api, selector, options.strict, this._wait(api, options), { text, options: pick(options, KEY_KEYS) });
    }
    async _focus(api, selector, options = {}) {
      await this._selectorOp("locator.focus", api, selector, options.strict, this._wait(api, options), {});
    }
    async _blur(api, selector, options = {}) {
      await this._selectorOp("locator.blur", api, selector, options.strict, this._wait(api, options), {});
    }
    async _selectOption(api, selector, values, options = {}) {
      return await this._selectorOp("locator.selectOption", api, selector, options.strict, this._wait(api, options), { ...selectOptionValues(api, values), ...pick(options, ["force"]) });
    }
    async _selectText(api, selector, options = {}) {
      await this._selectorOp("locator.selectText", api, selector, options.strict, this._wait(api, options), pick(options, ["force"]));
    }
    async _drop(selector, payload, options) {
      const api = "locator.drop";
      const files = payload.files === undefined ? undefined : dropFiles(api, payload.files);
      await withClientErrors(api, () => this._selectorOp("locator.drop", api, selector, true, this._wait(api, options), {
        files,
        data: Object.entries(payload.data ?? {}).map(([mimeType, value]) => ({
          mimeType,
          value
        })),
        position: options.position
      }));
    }
    async _setInputFiles(api, selector, files, options = {}) {
      const input = inputFiles(api, files);
      await withClientErrors(api, () => this._selectorOp("locator.setInputFiles", api, selector, options.strict, this._wait(api, options), { files: input }));
    }
    async _dispatchEvent(api, selector, type, eventInit, options = {}) {
      await this._selectorOp("locator.dispatchEvent", api, selector, options.strict, this._wait(api, options), { type, eventInit: eventInit ?? {} });
    }
    async _scrollIntoViewIfNeeded(api, selector, options = {}) {
      await this._selectorOp("locator.scrollIntoViewIfNeeded", api, selector, options.strict, this._wait(api, options), {});
    }
    async _read(op, api, selector, options = {}) {
      const args = {};
      return await this._selectorOp(op, api, selector, options.strict, this._wait(api, options), args);
    }
    async _getAttribute(api, selector, name, options = {}) {
      return await this._selectorOp("locator.getAttribute", api, selector, options.strict, this._wait(api, options), { name });
    }
    async _isVisible(op, api, selector, options = {}) {
      return await this._selectorOp(op, api, selector, options.strict, { timeout: 0 }, {});
    }
    async _everyMatch(op, api, selector) {
      const target = await this._locatorBrand(api, selector);
      const input = { target };
      return await invoke(op, this._call(api, 0), input);
    }
    async _normalize(api, selector) {
      const target = await this._locatorBrand(api, selector);
      const input = { target, selector };
      const resolved = await invoke("locator.normalize", this._call(api, 0), input);
      return resolved.selector;
    }
    async _boundingBox(api, selector, options = {}) {
      return await this._selectorOp("locator.boundingBox", api, selector, true, this._wait(api, options), {});
    }
    async _waitForSelector(api, selector, options = {}, omitReturnValue) {
      const raw = options;
      if (typeof raw === "object" && raw !== null && "visibility" in raw)
        throw clientError(api, "options.visibility is not supported, did you mean options.state?");
      const state = options.state ?? "visible";
      const handle = await this._selectorOp("locator.waitFor", api, selector, options.strict, this._wait(api, options), { state, omitReturnValue });
      return this._page._handle(handle);
    }
    async _elementHandle(api, selector, strict, noWait, options = {}) {
      const handle = await this._selectorOp("locator.elementHandle", api, selector, strict, noWait ? { timeout: 0 } : this._wait(api, options), noWait ? { noWait: true } : {});
      return this._page._handle(handle);
    }
    async _elementHandles(api, selector) {
      const target = await this._locatorBrand(api, selector);
      const handles = await invoke("locator.elementHandles", this._call(api, 0), {
        target
      });
      return handles.flatMap((handle) => {
        const element = this._page._handle(handle);
        return element ? [element] : [];
      });
    }
    async _evaluateOn(api, selector, strict, script, noWait, options = {}) {
      const value = await this._selectorOp("locator.evaluate", api, selector, strict, noWait ? { timeout: 0 } : this._wait(api, options), noWait ? { script, noWait: true, selector } : { script });
      return fromPageValue(api, value);
    }
    async _evaluateAll(api, selector, script) {
      const target = await this._locatorBrand(api, selector);
      const value = await invoke("locator.evaluateAll", this._call(api, 0), {
        target,
        script
      });
      return fromPageValue(api, value);
    }
    async _snapshot(api, selector, options = {}) {
      const mode = options.mode ?? "default";
      const shape = pick({ mode, depth: options.depth, boxes: options.boxes }, [
        "mode",
        "depth",
        "boxes"
      ]);
      const wait = this._wait(api, options);
      if (selector !== null)
        return await this._selectorOp("locator.ariaSnapshot", api, selector, true, wait, shape);
      if (this._id !== null)
        throw unsupportedError(api, "snapshot a child frame through its iframe: page.locator('iframe').contentFrame().locator('body').ariaSnapshot()");
      return await invoke("page.ariaSnapshot", this._call(api, wait.timeout), { page: await this._page._brand(api), ...shape }, wait.signal);
    }
    async _ariaSnapshot(api, selector, options) {
      return renderSnapshot(await this._snapshot(api, selector, options));
    }
    async _ariaSnapshotJSON(api, selector, options) {
      return (await this._snapshot(api, selector, options)).json;
    }
    async _screenshot(api, selector, options = {}) {
      const wire = await screenshotOptions(api, options);
      if (!wire.type && options.path)
        wire.type = screenshotType({ api, path: options.path });
      const { timeout, signal } = this._wait(api, options);
      const page = await this._page._brand(api);
      const input = selector === null ? { page, ...wire } : {
        page,
        target: await this._locatorBrand(api, selector),
        strict: "strict",
        ...wire
      };
      const image = await invoke("page.screenshot", this._call(api, timeout), input, signal);
      await saveScreenshot({ api, bytes: image, path: options.path, signal });
      return image;
    }
    async _goto(api, url, options = {}) {
      if (typeof url !== "string")
        throw clientError(api, `url: expected string, got ${typeof url}`);
      const waitUntil = loadState(api, "waitUntil", options.waitUntil ?? "load");
      const signal = signalOf(api, options);
      return await this._page._navigating(async () => {
        const args = await this._frameArgs(api);
        const { value: result, observed } = await invokeObserved("page.goto", this._call(api, this._navigationTimeout(options)), {
          ...args,
          url,
          waitUntil,
          ...pick(options, ["referer"])
        }, undefined, signal);
        return await this._page._navigated(api, this, result, observed);
      });
    }
    async _waitForLoadState(api, state, options = {}) {
      const verified = loadState(api, "state", state ?? "load");
      const signal = signalOf(api, options);
      await untilAborted(signal, () => abortedError(api, signal), async () => {
        const { value: result, observed } = await invokeObserved("page.waitForLoadState", this._call(api, this._navigationTimeout(options)), { ...await this._frameArgs(api), state: verified }, undefined, signal);
        await this._page._navigated(api, this, result, observed);
      });
    }
    async _content(api) {
      return await invoke("page.content", this._call(api, 0), await this._frameArgs(api));
    }
    async _addTag(api, op, options) {
      const keys = op === "page.addScriptTag" ? ["url", "path", "content", "type"] : ["url", "path", "content"];
      const input = pick(options, keys);
      for (const key of keys) {
        const value = input[key];
        if (value !== undefined && typeof value !== "string")
          throw clientError(api, `${key}: expected string, got ${value === null ? "object" : typeof value}`);
      }
      let handle;
      try {
        const { value: result, observed } = await invokeObserved(op, this._call(api, 0), {
          ...await this._frameArgs(api),
          ...input
        });
        this._page._evaluated(result.frames, observed);
        handle = this._page._handle(result.element);
      } catch (error) {
        if (error instanceof Error && error.message.startsWith(`${api}: `) && /^(?:E[A-Z0-9]+|UNKNOWN): .*?, (?:open|read) '/.test(error.message.slice(api.length + 2)))
          error.message = error.message.slice(api.length + 2);
        throw error;
      }
      if (!handle)
        throw plainError(api, "The inserted tag did not return an element");
      return handle;
    }
    async _setContent(api, html, options = {}) {
      const waitUntil = loadState(api, "waitUntil", options.waitUntil ?? "load");
      const signal = signalOf(api, options);
      const { value: result, observed } = await invokeObserved("page.setContent", this._call(api, this._navigationTimeout(options)), { ...await this._frameArgs(api), html, waitUntil }, undefined, signal);
      await this._page._navigated(api, this, result, observed);
    }
    async _evaluate(api, pageFunction, arg, options) {
      const script = await this._page._evaluationScript(api, "frame", pageFunction, arg, options);
      const { value: result, observed } = await invokeObserved("page.evaluate", this._call(api, 0), {
        ...await this._frameArgs(api),
        script
      });
      this._page._evaluated(result.frames, observed);
      return fromPageValue(api, result.value);
    }
    async _waitForFunction(api, pageFunction, arg, options = {}) {
      if (typeof options.polling === "string" && options.polling !== "raf")
        throw plainError(api, `Unknown polling option: ${options.polling}`);
      const script = pageCall(api, "frame", pageFunction, arg);
      const wait = this._wait(api, options);
      const { value, preview } = await invoke("page.waitForFunction", this._call(api, wait.timeout), {
        ...await this._frameArgs(api),
        script,
        ...typeof options.polling === "number" ? { polling: options.polling } : {}
      }, wait.signal);
      return new WaitHandle(value, preview);
    }
    _update(info) {
      this._name = info.name;
      this._url = info.url;
    }
  }
  function frameMatches(frame, selector) {
    const name = typeof selector === "string" ? selector : selector.name;
    if (name)
      return frame.name() === name;
    const url = typeof selector === "string" ? undefined : selector.url;
    if (url === undefined)
      return false;
    if (typeof url === "string")
      return globMatches(url, frame.url());
    if (url instanceof RegExp) {
      url.lastIndex = 0;
      return url.test(frame.url());
    }
    return Boolean(url(frame.url()));
  }
  async function screenshotOptions(api, options) {
    const wire = pick(options, SCREENSHOT_KEYS);
    if (options.mask !== undefined) {
      wire.mask = await Promise.all(options.mask.map((locator) => {
        if (!(locator instanceof Locator))
          throw clientError(api, "mask: expected a Locator");
        return locator._frame._locatorBrand(api, locator._selector);
      }));
    }
    return wire;
  }
  var HELD_VALUE = "Neo's waitForFunction handle holds the value, not the page's object; read it with jsonValue()";

  class WaitHandle {
    #value;
    #preview;
    constructor(value, preview) {
      this.#value = value;
      this.#preview = preview;
      markHandle(this);
    }
    async jsonValue() {
      return fromPageValue("jsHandle.jsonValue", this.#value);
    }
    _value(api) {
      return fromPageValue(api, this.#value);
    }
    asElement() {
      return null;
    }
    async dispose() {}
    toString() {
      return this.#preview;
    }
    evaluate() {
      return Promise.reject(unsupportedError("jsHandle.evaluate", HELD_VALUE));
    }
    evaluateHandle() {
      return Promise.reject(unsupportedError("jsHandle.evaluateHandle", HELD_VALUE));
    }
    getProperty() {
      return Promise.reject(unsupportedError("jsHandle.getProperty", HELD_VALUE));
    }
    getProperties() {
      return Promise.reject(unsupportedError("jsHandle.getProperties", HELD_VALUE));
    }
  }

  // crates/neo-sdk/neo-playwright-js/src/prelude/element-handle.ts
  function elementBrand(value) {
    if (value instanceof ElementHandle)
      return value._brand;
    if (typeof value !== "object" || value === null)
      return null;
    if (Reflect.get(value, "$neo") !== "element")
      return null;
    return value;
  }
  function handleGuid() {
    let digits = "";
    for (let i = 0;i < 32; i++)
      digits += Math.floor(Math.random() * 16).toString(16);
    return `handle@${digits}`;
  }

  class ElementHandle extends EventEmitter {
    _page;
    _brand;
    _guid = handleGuid();
    _disposed = false;
    constructor(page, brand) {
      super();
      this._page = page;
      this._brand = brand;
      markHandle(this);
    }
    asElement() {
      return this;
    }
    async dispose() {
      this._disposed = true;
    }
    #live(api) {
      if (this._disposed)
        throw clientError(api, "Target page, context or browser has been closed");
    }
    toString() {
      return this._brand.display;
    }
    toJSON() {
      return this._brand;
    }
    async#op(op, api, timeout, args) {
      this.#live(api);
      const input = {
        target: this._brand,
        strict: "strict",
        ...args
      };
      return await invoke(op, { api, timeoutMs: timeoutMs(api, timeout) }, input);
    }
    #timeout(options) {
      return this._page._timeoutSettings.timeout(options);
    }
    #rooted(api, selector) {
      this.#live(api);
      if (typeof selector !== "string")
        throw clientError(api, `selector: expected string, got ${typeof selector}`);
      return {
        ...locatorBrand(api, this._brand.page, null, selector),
        root: this._brand
      };
    }
    async $(selector) {
      const api = "elementHandle.$";
      const handle = await invoke("locator.elementHandle", { api, timeoutMs: timeoutMs(api, 0) }, {
        target: this.#rooted(api, selector),
        strict: "firstMatch",
        noWait: true
      });
      return this._page._handle(handle);
    }
    async $$(selector) {
      const api = "elementHandle.$$";
      const handles = await invoke("locator.elementHandles", { api, timeoutMs: timeoutMs(api, 0) }, { target: this.#rooted(api, selector) });
      return handles.flatMap((handle) => {
        const element = this._page._handle(handle);
        return element ? [element] : [];
      });
    }
    async click(options = {}) {
      await this.#op("locator.click", "elementHandle.click", this.#timeout(options), {
        options: pick(options, POINTER_KEYS)
      });
    }
    async dblclick(options = {}) {
      await this.#op("locator.dblclick", "elementHandle.dblclick", this.#timeout(options), {
        options: pick(options, DBLCLICK_KEYS)
      });
    }
    async hover(options = {}) {
      await this.#op("locator.hover", "elementHandle.hover", this.#timeout(options), {
        options: pick(options, HOVER_KEYS)
      });
    }
    async check(options = {}) {
      await this.#op("locator.check", "elementHandle.check", this.#timeout(options), {
        options: pick(options, CHECK_KEYS)
      });
    }
    async uncheck(options = {}) {
      await this.#op("locator.uncheck", "elementHandle.uncheck", this.#timeout(options), {
        options: pick(options, CHECK_KEYS)
      });
    }
    async setChecked(checked, options = {}) {
      await this.#op(checked ? "locator.check" : "locator.uncheck", "elementHandle.setChecked", this.#timeout(options), { options: pick(options, CHECK_KEYS) });
    }
    async fill(value, options = {}) {
      await this.#op("locator.fill", "elementHandle.fill", this.#timeout(options), {
        value,
        options: pick(options, FILL_KEYS)
      });
    }
    async press(key, options = {}) {
      await this.#op("locator.press", "elementHandle.press", this.#timeout(options), {
        key,
        options: pick(options, KEY_KEYS)
      });
    }
    async type(text, options = {}) {
      await this.#op("locator.pressSequentially", "elementHandle.type", this.#timeout(options), {
        text,
        options: pick(options, KEY_KEYS)
      });
    }
    async selectOption(values, options = {}) {
      const api = "elementHandle.selectOption";
      return await this.#op("locator.selectOption", api, this.#timeout(options), {
        ...selectOptionValues(api, values),
        ...pick(options, ["force"])
      });
    }
    async selectText(options = {}) {
      await this.#op("locator.selectText", "elementHandle.selectText", this.#timeout(options), pick(options, ["force"]));
    }
    async setInputFiles(files, options = {}) {
      const api = "elementHandle.setInputFiles";
      const input = inputFiles(api, files);
      await withClientErrors(api, () => this.#op("locator.setInputFiles", api, this.#timeout(options), {
        files: input
      }));
    }
    async focus() {
      await this.#op("locator.focus", "elementHandle.focus", 0, {});
    }
    async dispatchEvent(type, eventInit = {}) {
      await this.#op("locator.dispatchEvent", "elementHandle.dispatchEvent", 0, {
        type,
        eventInit
      });
    }
    async scrollIntoViewIfNeeded(options = {}) {
      await this.#op("locator.scrollIntoViewIfNeeded", "elementHandle.scrollIntoViewIfNeeded", this.#timeout(options), {});
    }
    async waitForElementState(state, options = {}) {
      await this.#op("locator.waitFor", "elementHandle.waitForElementState", this.#timeout(options), {
        state,
        omitReturnValue: true
      });
    }
    textContent() {
      return this.#op("locator.textContent", "elementHandle.textContent", 0, {});
    }
    innerText() {
      return this.#op("locator.innerText", "elementHandle.innerText", 0, {});
    }
    innerHTML() {
      return this.#op("locator.innerHTML", "elementHandle.innerHTML", 0, {});
    }
    inputValue() {
      return this.#op("locator.inputValue", "elementHandle.inputValue", 0, {});
    }
    getAttribute(name) {
      return this.#op("locator.getAttribute", "elementHandle.getAttribute", 0, {
        name
      });
    }
    isChecked() {
      return this.#op("locator.isChecked", "elementHandle.isChecked", 0, {});
    }
    isDisabled() {
      return this.#op("locator.isDisabled", "elementHandle.isDisabled", 0, {});
    }
    isEditable() {
      return this.#op("locator.isEditable", "elementHandle.isEditable", 0, {});
    }
    isEnabled() {
      return this.#op("locator.isEnabled", "elementHandle.isEnabled", 0, {});
    }
    isHidden() {
      return this.#op("locator.isHidden", "elementHandle.isHidden", 0, {});
    }
    isVisible() {
      return this.#op("locator.isVisible", "elementHandle.isVisible", 0, {});
    }
    boundingBox() {
      return this.#op("locator.boundingBox", "elementHandle.boundingBox", 0, {});
    }
    async evaluate(pageFunction, arg, options) {
      const api = "elementHandle.evaluate";
      this.#live(api);
      const script = await this._page._evaluationScript(api, "handle", pageFunction, arg, options);
      return fromPageValue(api, await this.#op("locator.evaluate", api, 0, { script }));
    }
    async $eval(selector, pageFunction, arg) {
      const api = "elementHandle.$eval";
      const script = pageCall(api, "query", pageFunction, arg);
      const value = await invoke("locator.evaluate", { api, timeoutMs: timeoutMs(api, 0) }, {
        target: this.#rooted(api, selector),
        strict: "firstMatch",
        noWait: true,
        selector,
        script
      });
      return fromPageValue(api, value);
    }
    async $$eval(selector, pageFunction, arg) {
      const api = "elementHandle.$$eval";
      const script = pageCall(api, "query", pageFunction, arg);
      const value = await invoke("locator.evaluateAll", { api, timeoutMs: timeoutMs(api, 0) }, { target: this.#rooted(api, selector), script });
      return fromPageValue(api, value);
    }
    async screenshot(options = {}) {
      const api = "elementHandle.screenshot";
      const wire = await screenshotOptions(api, options);
      if (!wire.type && options.path)
        wire.type = screenshotType({ api, path: options.path });
      const signal = signalOf(api, options);
      const image = await invoke("page.screenshot", { api, timeoutMs: timeoutMs(api, this.#timeout(options)) }, {
        page: await this._page._brand(api),
        target: this._brand,
        strict: "strict",
        ...wire
      }, signal);
      await saveScreenshot({ api, bytes: image, path: options.path, signal });
      return image;
    }
  }

  // crates/neo-sdk/neo-playwright-js/src/prelude/init-script.ts
  var TOO_MANY_ARGUMENTS = "Too many arguments. If you need to pass more than 1 argument to the function wrap them in an object.";
  async function addInitScript(api, owner, script, arg, options) {
    if (options !== undefined && (typeof options !== "object" || options === null || Array.isArray(options)))
      throw plainError(api, TOO_MANY_ARGUMENTS);
    if (options && Reflect.get(options, "exposeFunctions")) {
      if (typeof script !== "function")
        throw plainError(api, "Passing functions requires the init script to be a function");
      return await withExposedFunctions(owner, script, arg);
    }
    return registration(await owner.register(initSource(api, script, arg)));
  }
  async function withExposedFunctions(owner, script, arg) {
    const exposing = [];
    const undo = async (error) => {
      const settled = await Promise.allSettled(exposing);
      await Promise.all(settled.map((outcome) => outcome.status === "fulfilled" ? outcome.value.dispose().catch(() => {}) : undefined));
      throw error;
    };
    let serialized;
    try {
      serialized = serializeAsCallArgument(arg, (value) => {
        if (typeof value === "function") {
          const name = callbackName();
          exposing.push(owner.expose(name, (_source, ...args) => Reflect.apply(value, undefined, args)));
          return { fn: name };
        }
        return { fallThrough: value };
      });
    } catch (error) {
      return await undo(error);
    }
    let bindings;
    let init;
    try {
      bindings = await Promise.all(exposing);
      const source = `(${String(script)})(globalThis['${kBindingsControllerProperty}'].parseInitScriptArg(${JSON.stringify(serialized)}))`;
      init = registration(await owner.register({ source }));
    } catch (error) {
      return await undo(error);
    }
    let disposed = false;
    return new Disposable(async () => {
      if (disposed)
        return;
      disposed = true;
      await Promise.all(bindings.map((binding) => binding.dispose().catch(() => {})));
      await init.dispose();
    });
  }
  function callbackName() {
    let text = kFunctionBindingPrefix;
    for (let index = 0;index < 16; index++)
      text += Math.floor(Math.random() * 256).toString(16).padStart(2, "0");
    return text;
  }
  function initSource(api, script, arg) {
    if (typeof script === "function") {
      const argText = Object.is(arg, undefined) ? "undefined" : JSON.stringify(arg);
      return { source: `(${String(script)})(${argText})` };
    }
    if (arg !== undefined)
      throw plainError(api, "Cannot evaluate a string with arguments");
    if (typeof script === "string")
      return { source: script };
    if (typeof script === "object" && script !== null) {
      const content = Reflect.get(script, "content");
      if (content !== undefined)
        return { source: String(content) };
      const path = Reflect.get(script, "path");
      if (path !== undefined)
        return { path: String(path) };
    }
    throw plainError(api, "Either path or content property must be present");
  }

  class Disposable {
    #dispose;
    constructor(dispose) {
      this.#dispose = dispose;
    }
    async dispose() {
      await this.#dispose();
    }
  }
  function registration(id, forget) {
    return new Disposable(async () => {
      try {
        await invoke("disposable.dispose", { api: "disposable.dispose", timeoutMs: 0 }, { id });
      } finally {
        forget?.();
      }
    });
  }
  var asyncDispose = Reflect.get(Symbol, "asyncDispose");
  if (typeof asyncDispose === "symbol")
    Object.defineProperty(Disposable.prototype, asyncDispose, {
      configurable: true,
      writable: true,
      value() {
        return this.dispose();
      }
    });

  // crates/neo-sdk/neo-playwright-js/src/prelude/evaluation.ts
  function exposesFunctions(options) {
    return typeof options === "object" && options !== null && !!Reflect.get(options, "exposeFunctions");
  }
  var TOO_MANY_ARGUMENTS2 = "Too many arguments. If you need to pass more than 1 argument to the function wrap them in an object.";
  var REFUSALS = {
    unsupported: unsupportedError,
    client: clientError,
    plain: plainError
  };

  class FailedCheck extends Error {
    kind;
    constructor(kind, message) {
      super(message);
      this.kind = kind;
    }
  }
  function assertMaxArguments(api, count, max) {
    if (count > max)
      throw plainError(api, TOO_MANY_ARGUMENTS2);
  }
  function assertEvaluateOptions(options) {
    if (options !== undefined && (typeof options !== "object" || options === null || Array.isArray(options)))
      throw new FailedCheck("plain", TOO_MANY_ARGUMENTS2);
  }
  var fallThrough = (value) => ({ fallThrough: value });
  var FORMS = {
    frame: { options: true, afterWait: false },
    locator: { options: true, afterWait: true },
    handle: { options: true, afterWait: false },
    query: { options: false, afterWait: false }
  };
  function pageCall(api, form, pageFunction, arg, options, callbacks) {
    const { options: checksOptions, afterWait } = FORMS[form];
    try {
      if (checksOptions)
        assertEvaluateOptions(options);
      return script(api, pageFunction, arg, callbacks);
    } catch (error) {
      if (afterWait) {
        const message = error instanceof Error ? error.message : String(error);
        return {
          refused: message.startsWith(`${api}: `) ? message.slice(api.length + 2) : message
        };
      }
      if (error instanceof FailedCheck)
        throw REFUSALS[error.kind](api, error.message);
      throw error;
    }
  }
  function script(api, pageFunction, arg, callbacks) {
    const isFunction = typeof pageFunction === "function";
    const { serialized, clientHandles } = serializeArgument(api, arg, callbacks);
    let expression;
    try {
      expression = normalizeEvaluationExpression(String(pageFunction), isFunction);
    } catch (error) {
      return { refused: error instanceof Error ? error.message : String(error) };
    }
    const onServer = parseSerializedValue(serialized, clientHandles);
    const handles = [];
    const text = JSON.stringify(serializeAsCallArgument(onServer, (value) => {
      if (!(value instanceof ElementHandle))
        return fallThrough(value);
      handles.push(value._brand);
      return { h: handles.length - 1 };
    }));
    return {
      run: {
        expression,
        isFunction,
        arg: text,
        ...handles.length > 0 ? { handles } : {}
      }
    };
  }
  function serializeArgument(api, arg, callbacks) {
    let serialized;
    const clientHandles = [];
    const copies = new Map;
    try {
      serialized = serializeValue(arg, (value) => {
        if (typeof value === "function" && callbacks) {
          const name = callbackName();
          callbacks.push([name, value]);
          return { fn: name };
        }
        if (value instanceof ElementHandle) {
          clientHandles.push(value);
          return { h: clientHandles.length - 1 };
        }
        if (value instanceof WaitHandle) {
          if (!copies.has(value))
            copies.set(value, value._value(api));
          return fallThrough(copies.get(value));
        }
        if (!isHandle(value))
          return fallThrough(value);
        throw new FailedCheck("unsupported", "Neo passes element handles as evaluate arguments, not pages, frames, locators or JSHandles; pass the value itself, or an element handle");
      });
    } catch (error) {
      if (error instanceof FailedCheck)
        throw error;
      throw new FailedCheck("plain", error instanceof Error ? error.message : String(error));
    }
    validateValue(serialized, "arg.value");
    clientHandles.forEach((handle, index) => {
      if (handle._disposed)
        throw new FailedCheck("client", `arg.handles[${index}]: no object with guid ${handle._guid}`);
    });
    return { serialized, clientHandles };
  }
  function validateValue(value, path) {
    const expectString = (field, at) => {
      if (typeof field !== "string" && !(field instanceof String))
        throw new FailedCheck("client", `${at}: expected string, got ${typeof field}`);
    };
    if (value.d !== undefined)
      expectString(value.d, `${path}.d`);
    if (value.e !== undefined)
      for (const key of ["m", "n", "s"])
        expectString(value.e[key], `${path}.e.${key}`);
    value.a?.forEach((item, index) => {
      validateValue(item, `${path}.a[${index}]`);
    });
    value.o?.forEach((entry, index) => {
      validateValue(entry.v, `${path}.o[${index}].v`);
    });
  }
  function inPageArgument(value) {
    const serialized = serializeValue(value, (item) => {
      if (isHandle(item) || item instanceof WaitHandle)
        throw new Error("Neo answers a binding with values, not handles; return the value itself");
      return fallThrough(item);
    });
    return JSON.stringify(serializeAsCallArgument(parseSerializedValue(serialized, []), fallThrough));
  }
  function fromInPageArgument(value) {
    return parseEvaluationResultValue(value);
  }
  function fromPageValue(api, value) {
    try {
      return parseEvaluationResultValue(value);
    } catch (error) {
      throw answerError(api, error);
    }
  }
  function normalizeEvaluationExpression(expression, isFunction) {
    expression = expression.trim();
    if (isFunction) {
      try {
        new Function(`(${expression})`);
      } catch {
        if (expression.startsWith("async "))
          expression = `async function ${expression.substring("async ".length)}`;
        else
          expression = `function ${expression}`;
        try {
          new Function(`(${expression})`);
        } catch {
          throw new Error("Passed function is not well-serializable!");
        }
      }
    }
    if (/^(async)?\s*function(\s|\()/.test(expression))
      expression = `(${expression})`;
    return expression;
  }

  // crates/neo-sdk/neo-playwright-js/src/prelude/bindings.ts
  async function exposeOnPage(page, api, name, callback, noGlobal = false) {
    if (page._exposed.has(name))
      throw clientError(api, `Function "${name}" has been already registered`);
    if (page._context._exposed.has(name))
      throw clientError(api, `Function "${name}" has been already registered in the browser context`);
    page._exposed.set(name, (source, ...args) => callback(source, ...args));
    const events = page._context._events;
    events.bindingAdded();
    const forget = once(() => {
      page._exposed.delete(name);
      events.bindingRemoved();
    });
    try {
      const { id } = await invoke("page.exposeBinding", { api, timeoutMs: 0 }, {
        page: await page._brand(api),
        name,
        ...noGlobal ? { noGlobal: true } : {}
      });
      return registration(id, forget);
    } catch (error) {
      forget();
      throw error;
    }
  }
  async function exposeOnContext(context, api, name, callback, noGlobal = false) {
    if (context._exposed.has(name))
      throw clientError(api, `Function "${name}" has been already registered`);
    if (context.pages().some((page) => page._exposed.has(name)))
      throw clientError(api, `Function "${name}" has been already registered in one of the pages`);
    context._exposed.set(name, (source, ...args) => callback(source, ...args));
    context._events.bindingAdded();
    const forget = once(() => {
      context._exposed.delete(name);
      context._events.bindingRemoved();
    });
    try {
      const { id } = await invoke("context.exposeBinding", { api, timeoutMs: 0 }, { name, ...noGlobal ? { noGlobal: true } : {} });
      return registration(id, forget);
    } catch (error) {
      forget();
      throw error;
    }
  }
  function once(forget) {
    let done = false;
    return () => {
      if (done)
        return;
      done = true;
      forget();
    };
  }
  async function answerBindingCall(page, record) {
    let payload;
    try {
      payload = JSON.parse(record.payload);
    } catch {
      return;
    }
    const { name, seq } = payload;
    if (typeof name !== "string")
      return;
    const callback = page._exposed.get(name) ?? page._context._exposed.get(name);
    if (!callback)
      return;
    let answer;
    try {
      if (!Array.isArray(payload.serializedArgs))
        throw new Error("serializedArgs is not an array. This can happen when Array.prototype.toJSON is defined incorrectly");
      const args = payload.serializedArgs.map(fromInPageArgument);
      const frame = await page._bindingFrame(record.frame);
      const result = await callback({ context: page._context, page, frame }, ...args);
      answer = { name, seq, result };
    } catch (error) {
      answer = { name, seq, error };
    }
    if ((page._exposed.get(name) ?? page._context._exposed.get(name)) !== callback)
      return;
    let arg;
    try {
      arg = inPageArgument(answer);
    } catch (error) {
      arg = inPageArgument({ name, seq, error });
    }
    const api = "bindingCall.resolve";
    await invoke("bindingCall.resolve", { api, timeoutMs: 0 }, { page: await page._brand(api), context: record.executionContext, arg }).catch(() => {});
  }

  // crates/neo-sdk/neo-playwright-js/src/prelude/event-deadline.ts
  class EventDeadline {
    api;
    event;
    timeout;
    endsAt;
    constructor(api, event, since, timeout) {
      this.api = api;
      this.event = event;
      this.timeout = timeout;
      this.endsAt = timeout === 0 ? null : since + timeout;
    }
    left() {
      if (this.endsAt === null)
        return 0;
      const left = this.endsAt - Date.now();
      if (left <= 0)
        throw this.expired();
      return left;
    }
    named(error) {
      if (!(error instanceof Error) || Reflect.get(error, "code") !== "wait_timeout")
        return error;
      const [, ...log] = error.message.split(`
`);
      error.message = [`${this.api}: ${this.message()}`, ...log].join(`
`);
      return error;
    }
    race(verdict) {
      const endsAt = this.endsAt;
      if (endsAt === null || !isThenable(verdict))
        return Promise.resolve(verdict);
      let cancel = () => {};
      const expired = new Promise((_, reject) => {
        cancel = startTimer(Math.max(0, endsAt - Date.now()), () => reject(this.expired()));
      });
      return Promise.race([verdict, expired]).finally(cancel);
    }
    expired() {
      return waitTimeoutError(this.api, this.message());
    }
    message() {
      return this.event === null ? `Timeout ${this.timeout}ms exceeded.` : `Timeout ${this.timeout}ms exceeded while waiting for event "${this.event}"`;
    }
  }
  function isThenable(value) {
    return (typeof value === "object" || typeof value === "function") && value !== null && typeof Reflect.get(value, "then") === "function";
  }

  // crates/neo-sdk/neo-playwright-js/src/prelude/event-values.ts
  class FileChooser {
    #pageValue;
    #element;
    #multiple;
    constructor(page, info) {
      this.#pageValue = page;
      const element = page._handle(info.element);
      if (!element)
        throw new Error("Rust returned a chooser without its input");
      this.#element = element;
      this.#multiple = info.isMultiple;
      markHandle(this);
    }
    element() {
      return this.#element;
    }
    isMultiple() {
      return this.#multiple;
    }
    page() {
      return this.#pageValue;
    }
    async setFiles(files, options) {
      await this.#element.setInputFiles(files, options);
    }
  }

  class Dialog {
    #pageValue;
    #info;
    constructor(page, info) {
      this.#pageValue = page;
      this.#info = info;
      markHandle(this);
    }
    page() {
      return this.#pageValue;
    }
    type() {
      return this.#info.type;
    }
    message() {
      return this.#info.message;
    }
    defaultValue() {
      return this.#info.defaultValue;
    }
    async accept(promptText) {
      const api = "dialog.accept";
      await invoke("dialog.accept", { api, timeoutMs: 0 }, {
        page: await this.#pageValue._brand(api),
        dialog: this.#info.id,
        ...promptText === undefined ? {} : { promptText }
      });
    }
    async dismiss() {
      const api = "dialog.dismiss";
      const page = await this.#pageValue._brand(api);
      try {
        await invoke("dialog.dismiss", { api, timeoutMs: 0 }, { page, dialog: this.#info.id });
      } catch (error) {
        if (Reflect.get(Object(error), "code") === "target_closed")
          return;
        throw error;
      }
    }
  }

  class Download {
    #pageValue;
    #info;
    constructor(page, info) {
      this.#pageValue = page;
      this.#info = info;
      markHandle(this);
    }
    page() {
      return this.#pageValue;
    }
    url() {
      return this.#info.url;
    }
    suggestedFilename() {
      return this.#info.suggestedFilename;
    }
    async path() {
      const api = "download.path";
      const done = await invoke("download.path", { api, timeoutMs: 0 }, { download: this.#info.id });
      if (done.path === null)
        throw new Error(`${api}: ${done.failure ?? "download failed"}`);
      return done.path;
    }
    async failure() {
      const api = "download.failure";
      const done = await invoke("download.path", { api, timeoutMs: 0 }, { download: this.#info.id });
      return done.failure;
    }
    async saveAs(path) {
      const api = "download.saveAs";
      await invoke("download.saveAs", { api, timeoutMs: 0 }, { download: this.#info.id, path });
    }
  }

  class ConsoleMessage {
    #pageValue;
    #info;
    constructor(page, info) {
      this.#pageValue = page;
      this.#info = info;
      markHandle(this);
    }
    page() {
      return this.#pageValue;
    }
    worker() {
      return null;
    }
    type() {
      return this.#info.type;
    }
    text() {
      return this.#info.text;
    }
    location() {
      const { url, lineNumber, columnNumber } = this.#info.location;
      return {
        url,
        line: lineNumber,
        column: columnNumber,
        lineNumber,
        columnNumber
      };
    }
    timestamp() {
      return this.#info.timestamp;
    }
    toString() {
      return this.#info.text;
    }
    [inspectCustom]() {
      return this.text();
    }
  }
  function pageError(info) {
    const error = new Error(info.message);
    error.name = info.name;
    error.stack = info.stack;
    return error;
  }

  // crates/neo-sdk/neo-playwright-js/src/prelude/events.ts
  var PAGE_EVENTS = new Set([
    "popup",
    "close",
    "dialog",
    "dialogclosed",
    "console",
    "pageerror",
    "download",
    "filechooser",
    "load",
    "domcontentloaded",
    "frameattached",
    "framenavigated",
    "framedetached"
  ]);
  var CONTEXT_EVENTS = new Set([
    "page",
    "pageload",
    "dialogclosed",
    "frameattached",
    "framenavigated",
    "framedetached"
  ]);
  var UNREPORTED = {
    request: "Neo does not report network events; wait for what the page shows instead",
    response: "Neo does not report network events; wait for what the page shows instead",
    requestfailed: "Neo does not report network events; wait for what the page shows instead",
    requestfinished: "Neo does not report network events; wait for what the page shows instead",
    websocket: "Neo does not report WebSockets",
    worker: "Neo does not report workers",
    serviceworker: "Neo does not report service workers",
    backgroundpage: "Neo does not report extension background pages",
    crash: "Neo does not report crashes; a crashed page fails its next call",
    pageclose: 'listen for close on the page itself: page.on("close")',
    weberror: "the context reports no weberror events; pageerror listeners are silent in hosted BrowserOS"
  };
  function checkReported(api, event, reported) {
    if (typeof event !== "string")
      return;
    const ours = reported === "page" ? PAGE_EVENTS : CONTEXT_EVENTS;
    if (ours.has(event))
      return;
    const other = reported === "page" ? CONTEXT_EVENTS : PAGE_EVENTS;
    const hint = UNREPORTED[event] ?? (other.has(event) ? reported === "page" ? `"${event}" is a browser context event: context.on("${event}")` : `Neo reports "${event}" on pages: page.on("${event}")` : undefined);
    if (hint !== undefined)
      throw unsupportedError(`${api}('${event}')`, hint);
  }
  function isReported(event, on) {
    return typeof event === "string" && (on === "page" ? PAGE_EVENTS : CONTEXT_EVENTS).has(event);
  }

  class EventHub {
    owner;
    listening = 0;
    pumping = false;
    batch = null;
    dialogs = new Map;
    downloads = new Map;
    choosers = new Map;
    nextChooserLease = 0;
    chooserListeners = new Map;
    chooserChanges = new Map;
    contextChanges = new Map;
    contextSince = new Map;
    constructor(owner) {
      this.owner = owner;
    }
    listen(page, event, on) {
      const since = Date.now();
      this.listening += on ? 1 : -1;
      if (page === null) {
        if (on)
          this.contextSince.set(event, since);
        else
          this.contextSince.delete(event);
        const previous = this.contextChanges.get(event) ?? Promise.resolve();
        const changed = previous.then(() => this.sendListen(null, event, on, since));
        this.contextChanges.set(event, changed);
      } else if (event === "filechooser") {
        const lease = on ? this.intercept(page) : this.chooserListeners.get(page);
        if (on && lease)
          this.chooserListeners.set(page, lease);
        else
          this.chooserListeners.delete(page);
        const previous = this.chooserChanges.get(page) ?? Promise.resolve();
        const changed = previous.then(() => this.sendListen(page, event, on, since, lease));
        this.chooserChanges.set(page, changed);
        trackChooserSetup(changed);
      } else
        this.sendListen(page, event, on, since);
      if (on && !this.pumping) {
        this.pumping = true;
        this.pump();
      }
    }
    bindingAdded() {
      this.listening++;
      if (!this.pumping) {
        this.pumping = true;
        this.pump();
      }
    }
    bindingRemoved() {
      this.listening--;
    }
    async wait(api, page, event, optionsOrPredicate, since, settings) {
      const kind = page === null ? "context" : "page";
      checkReported(api, event, kind);
      if (!isReported(event, kind))
        throw unsupportedError(api, `Neo reports only Playwright's events, not "${event}"`);
      const options = typeof optionsOrPredicate === "function" ? { predicate: optionsOrPredicate } : optionsOrPredicate ?? {};
      const deadline = new EventDeadline(api, event, since, timeoutMs(api, settings.timeout(options)));
      const signal = signalOf(api, options);
      const lease = event === "filechooser" && page ? this.intercept(page) : null;
      try {
        return await untilAborted(signal, () => abortedError(api, signal), async () => {
          if (lease)
            await deadline.race(lease.ready);
          return this.waitFor(api, page, event, options, since, deadline, signal);
        });
      } finally {
        if (lease)
          lease.release().catch(() => {});
      }
    }
    async waitFor(api, page, event, options, since, deadline, signal) {
      const target = page === null ? null : await page._openedBrand(api);
      let after = null;
      for (;; ) {
        const left = deadline.left();
        let record;
        let observed = null;
        try {
          ({ value: record, observed } = await invokeObserved("events.wait", { api, timeoutMs: left }, { page: target, event, since, after }, undefined, signal));
        } catch (error) {
          throw deadline.named(error);
        }
        if (record.event === "close" && this.batch?.closes.has(record.seq))
          await this.batch.done;
        let value;
        try {
          value = this.value(record, observed);
        } finally {
          this.settleCloses(observed);
        }
        const predicate = options.predicate;
        if (!predicate)
          return value;
        let accepted;
        try {
          accepted = await deadline.race(this.duringEvent(record, () => Reflect.apply(predicate, undefined, [value])));
        } catch (error) {
          throw prefixed(api, error);
        }
        if (accepted)
          return value;
        after = record.seq;
      }
    }
    async sendListen(page, event, on, since, lease) {
      const api = `${page === null ? "browserContext" : "page"}.${on ? "on" : "off"}`;
      try {
        if (on && lease)
          await lease.ready;
        await invoke("events.listen", { api, timeoutMs: 0 }, {
          page: page === null ? null : await page._openedBrand(api),
          event,
          on,
          since
        });
      } catch {
        if (on && lease)
          lease.release().catch(() => {});
      } finally {
        if (!on && lease)
          await lease.release().catch(() => {});
      }
    }
    intercept(page) {
      const id = ++this.nextChooserLease;
      const api = "page.filechooser";
      const target = page._openedBrand(api);
      const ready = target.then(async (page) => {
        await invoke("events.interceptFileChooser", { api, timeoutMs: 0 }, { page, lease: id, on: true });
      });
      trackChooserSetup(ready);
      let released = null;
      return {
        ready,
        release: () => {
          if (!released) {
            released = ready.then(async () => {
              await invoke("events.interceptFileChooser", { api, timeoutMs: 0 }, { page: await target, lease: id, on: false });
            });
            trackChooserSetup(released);
          }
          return released;
        }
      };
    }
    async pump() {
      try {
        while (this.listening > 0) {
          try {
            await invokeObserved("events.next", { api: "events.next", timeoutMs: 0 }, {}, ({ events }, observed) => this.deliverBatch(events, observed));
          } catch (error) {
            pumpDispatched();
            throw error;
          }
        }
      } catch {} finally {
        this.pumping = false;
      }
    }
    deliverBatch(events, observed) {
      this.stageBatch(events);
      try {
        for (const event of events)
          this.deliver(event, observed);
      } finally {
        try {
          this.settleCloses(observed);
        } finally {
          const batch = this.batch;
          this.batch = null;
          batch?.finish();
          pumpDispatched();
        }
      }
    }
    stageBatch(events) {
      const closes = new Set(events.filter((event) => event.event === "close").map((event) => event.seq));
      if (closes.size === 0)
        return;
      let finish;
      const done = new Promise((resolve) => {
        finish = resolve;
      });
      this.batch = { closes, done, finish };
    }
    page(value, url) {
      const page = this.owner._pageFromResult(value, url);
      if (!page)
        throw new Error(`Rust named no page in an event: ${String(value)}`);
      return page;
    }
    settleCloses(observed) {
      for (const close of observed?.closes ?? [])
        this.page({ $neo: "page", tab: close.tab })._settleClose(observed);
    }
    deliver(record, observed) {
      if (record.event === "binding") {
        answerBindingCall(this.page(record.page), record).catch(() => {});
        return;
      }
      const context = record.event === "page" || record.event === "pageload" || "context" in record && record.context;
      if (context && typeof record.at === "number" && record.at < (this.contextSince.get(record.event) ?? Infinity))
        return;
      switch (record.event) {
        case "popup":
          this.page(record.page).emit("popup", this.page(record.popup.page, record.popup.url));
          return;
        case "page":
          this.owner.emit("page", this.page(record.page, record.url));
          return;
        case "load":
        case "domcontentloaded": {
          const page = this.page(record.page);
          page._eventFrame(record.seq, "framenavigated", record.frame);
          this.duringEvent(record, () => page.emit(record.event, page));
          return;
        }
        case "pageload": {
          const page = this.page(record.page);
          page._eventFrame(record.seq, "framenavigated", record.frame);
          this.duringEvent(record, () => this.owner.emit("pageload", page));
          return;
        }
        case "frameattached":
        case "framenavigated":
        case "framedetached": {
          const page = this.page(record.page);
          const frame = page._eventFrame(record.seq, record.event, record.frame);
          const emitter = record.context ? this.owner : page;
          frame._duringEvent(record.frame, record.event, () => emitter.emit(record.event, frame));
          return;
        }
        case "close":
          this.page(record.page)._closeEvent(record.seq, observed);
          return;
        case "dialogclosed": {
          const page = this.page(record.page);
          const emitter = record.context ? this.owner : page;
          emitter.emit("dialogclosed", this.dialog(page, record.dialog));
          return;
        }
        case "dialog": {
          const page = this.page(record.page);
          const dialog = this.dialog(page, record.dialog);
          if (!page.emit("dialog", dialog))
            (dialog.type() === "beforeunload" ? dialog.accept() : dialog.dismiss()).catch(() => {});
          return;
        }
        case "console": {
          const page = this.page(record.page);
          page.emit("console", new ConsoleMessage(page, record.message));
          return;
        }
        case "pageerror":
          this.page(record.page).emit("pageerror", pageError(record.error));
          return;
        case "download": {
          const page = this.page(record.page);
          page.emit("download", this.download(page, record.download));
          return;
        }
        case "filechooser": {
          const page = this.page(record.page);
          page.emit("filechooser", this.chooser(record.seq, page, record.chooser));
          return;
        }
      }
    }
    duringEvent(record, read) {
      switch (record.event) {
        case "frameattached":
        case "framenavigated":
        case "framedetached": {
          const frame = this.page(record.page)._eventFrame(record.seq, record.event, record.frame);
          return frame._duringEvent(record.frame, record.event, read);
        }
        case "load":
        case "domcontentloaded":
        case "pageload":
          return this.page(record.page).mainFrame()._duringEvent(record.frame, "framenavigated", read);
        default:
          return read();
      }
    }
    value(record, observed) {
      switch (record.event) {
        case "popup":
          return this.page(record.popup.page, record.popup.url);
        case "page":
          return this.page(record.page, record.url);
        case "load":
        case "domcontentloaded":
        case "pageload": {
          const page = this.page(record.page);
          page._eventFrame(record.seq, "framenavigated", record.frame);
          return page;
        }
        case "frameattached":
        case "framenavigated":
        case "framedetached":
          return this.page(record.page)._eventFrame(record.seq, record.event, record.frame);
        case "close": {
          const page = this.page(record.page);
          page._closeEvent(record.seq, observed);
          return page;
        }
        case "dialog":
        case "dialogclosed":
          return this.dialog(this.page(record.page), record.dialog);
        case "console":
          return new ConsoleMessage(this.page(record.page), record.message);
        case "pageerror":
          return pageError(record.error);
        case "download":
          return this.download(this.page(record.page), record.download);
        case "filechooser":
          return this.chooser(record.seq, this.page(record.page), record.chooser);
        case "binding":
          return;
      }
    }
    dialog(page, info) {
      const known = this.dialogs.get(info.id);
      if (known)
        return known;
      const dialog = new Dialog(page, info);
      this.dialogs.set(info.id, dialog);
      return dialog;
    }
    download(page, info) {
      const known = this.downloads.get(info.id);
      if (known)
        return known;
      const download = new Download(page, info);
      this.downloads.set(info.id, download);
      return download;
    }
    chooser(seq, page, info) {
      const known = this.choosers.get(seq);
      if (known)
        return known;
      const chooser = new FileChooser(page, info);
      this.choosers.set(seq, chooser);
      return chooser;
    }
  }

  // crates/neo-sdk/neo-playwright-js/src/prelude/input.ts
  class Keyboard {
    #page;
    constructor(page) {
      this.#page = page;
      markHandle(this);
    }
    async down(key) {
      const api = "keyboard.down";
      await invoke("keyboard.down", { api, timeoutMs: 0 }, { page: await this.#page._brand(api), key });
    }
    async up(key) {
      const api = "keyboard.up";
      await invoke("keyboard.up", { api, timeoutMs: 0 }, { page: await this.#page._brand(api), key });
    }
    async press(key, options = {}) {
      const api = "keyboard.press";
      await invoke("keyboard.press", { api, timeoutMs: 0 }, {
        page: await this.#page._brand(api),
        key,
        options: pick(options, ["delay"])
      });
    }
    async type(text, options = {}) {
      const api = "keyboard.type";
      await invoke("keyboard.type", { api, timeoutMs: 0 }, {
        page: await this.#page._brand(api),
        text,
        options: pick(options, ["delay"])
      });
    }
    async insertText(text) {
      const api = "keyboard.insertText";
      await invoke("keyboard.insertText", { api, timeoutMs: 0 }, { page: await this.#page._brand(api), text });
    }
  }

  class Mouse {
    #page;
    constructor(page) {
      this.#page = page;
      markHandle(this);
    }
    async move(x, y, options = {}) {
      const api = "mouse.move";
      await invoke("mouse.move", { api, timeoutMs: 0 }, {
        page: await this.#page._brand(api),
        x,
        y,
        options: pick(options, ["steps"])
      });
    }
    async down(options = {}) {
      const api = "mouse.down";
      await invoke("mouse.down", { api, timeoutMs: 0 }, {
        page: await this.#page._brand(api),
        options: pick(options, ["button", "clickCount"])
      });
    }
    async up(options = {}) {
      const api = "mouse.up";
      await invoke("mouse.up", { api, timeoutMs: 0 }, {
        page: await this.#page._brand(api),
        options: pick(options, ["button", "clickCount"])
      });
    }
    click(x, y, options = {}) {
      return this.#clickAt("mouse.click", x, y, options);
    }
    dblclick(x, y, options = {}) {
      return this.#clickAt("mouse.dblclick", x, y, { ...options, clickCount: 2 });
    }
    async wheel(deltaX, deltaY) {
      const api = "mouse.wheel";
      await invoke("mouse.wheel", { api, timeoutMs: 0 }, { page: await this.#page._brand(api), deltaX, deltaY });
    }
    async#clickAt(api, x, y, options) {
      await invoke("mouse.click", { api, timeoutMs: 0 }, {
        page: await this.#page._brand(api),
        x,
        y,
        options: pick(options, ["button", "clickCount", "delay"])
      });
    }
  }

  // crates/neo-sdk/neo-playwright-js/src/prelude/response.ts
  class Response {
    #frameValue;
    #info;
    constructor(frame, info) {
      this.#frameValue = frame;
      this.#info = info;
      markHandle(this);
    }
    url() {
      return this.#info.url;
    }
    status() {
      return this.#info.status;
    }
    statusText() {
      return this.#info.statusText;
    }
    ok() {
      return this.#info.ok;
    }
    frame() {
      return this.#frameValue;
    }
    headers() {
      return {};
    }
    async allHeaders() {
      return {};
    }
    async headersArray() {
      return [];
    }
    async headerValue(_name) {
      return null;
    }
    async headerValues(_name) {
      return [];
    }
    async finished() {
      return null;
    }
    fromServiceWorker() {
      return false;
    }
    toJSON() {
      return this.#info;
    }
  }

  // crates/neo-sdk/neo-playwright-js/src/prelude/page.ts
  var PDF_KEYS = [
    "scale",
    "displayHeaderFooter",
    "headerTemplate",
    "footerTemplate",
    "printBackground",
    "landscape",
    "pageRanges",
    "format",
    "width",
    "height",
    "preferCSSPageSize",
    "margin",
    "tagged",
    "outline"
  ];

  class Page extends EventEmitter {
    _tab;
    _context;
    _timeoutSettings;
    _mainFrame;
    keyboard;
    mouse;
    _url;
    #urlKnown;
    _title = "";
    _closed = false;
    #viewportSize = null;
    #viewportSet = false;
    #observedSeq = 0;
    #observedHead = 0;
    #liveHead = 0;
    #closeEvent = 0;
    #approvedClose = null;
    _working = false;
    _navigationsInFlight = 0;
    #childFrames = new Map;
    #knownFrames = new Map;
    #frameEvents = new Map;
    #frameSequences = new Map;
    _exposed = new Map;
    constructor(context, tab, url) {
      super();
      this._context = context;
      this._tab = tab;
      this._url = url ?? "about:blank";
      this.#urlKnown = url !== undefined;
      this._timeoutSettings = new TimeoutSettings(context._timeoutSettings);
      this._mainFrame = new Frame(this, null, null);
      this.keyboard = new Keyboard(this);
      this.mouse = new Mouse(this);
      markHandle(this);
    }
    context() {
      return this._context;
    }
    mainFrame() {
      return this._mainFrame;
    }
    frames() {
      return [this._mainFrame, ...this.#childFrames.values()];
    }
    frame(selector) {
      const name = typeof selector === "string" ? selector : selector.name;
      const url = typeof selector === "string" ? undefined : selector.url;
      if (!name && url === undefined)
        throw plainError("page.frame", "Either name or url matcher should be specified");
      return this.frames().find((frame) => frameMatches(frame, selector)) ?? null;
    }
    url() {
      return this._mainFrame.url();
    }
    isClosed() {
      return this._closed;
    }
    setDefaultTimeout(timeout) {
      this._timeoutSettings.setDefaultTimeout(timeout);
    }
    setDefaultNavigationTimeout(timeout) {
      this._timeoutSettings.setDefaultNavigationTimeout(timeout);
    }
    video() {
      return null;
    }
    viewportSize() {
      return this.#viewportSize || null;
    }
    async setViewportSize(viewportSize) {
      this.#viewportSize = viewportSize;
      this.#viewportSet = true;
      for (const side of ["width", "height"]) {
        if (!Number.isInteger(viewportSize?.[side]))
          throw clientError("page.setViewportSize", `viewportSize.${side}: expected integer`);
      }
      const size = { width: viewportSize.width, height: viewportSize.height };
      await invoke("page.setViewportSize", { api: "page.setViewportSize", timeoutMs: 0 }, { page: await this._brand("page.setViewportSize"), viewportSize: size });
    }
    workers() {
      return [];
    }
    toJSON() {
      return this._tab === null ? { $neo: "page", tab: null } : { $neo: "page", tab: this._tab };
    }
    locator(selector, options) {
      return this._mainFrame._locator("page.locator", selector, options);
    }
    getByRef(ref) {
      return this.locator(`aria-ref=${ref}`);
    }
    getByTestId(testId) {
      return this._mainFrame.getByTestId(testId);
    }
    getByAltText(text, options) {
      return this._mainFrame.getByAltText(text, options);
    }
    getByLabel(text, options) {
      return this._mainFrame.getByLabel(text, options);
    }
    getByPlaceholder(text, options) {
      return this._mainFrame.getByPlaceholder(text, options);
    }
    getByText(text, options) {
      return this._mainFrame.getByText(text, options);
    }
    getByTitle(text, options) {
      return this._mainFrame.getByTitle(text, options);
    }
    getByRole(role, options = {}) {
      return this._mainFrame.getByRole(role, options);
    }
    frameLocator(selector) {
      return this._mainFrame.frameLocator(selector);
    }
    click(selector, options) {
      return this._mainFrame._click("page.click", selector, options);
    }
    dblclick(selector, options) {
      return this._mainFrame._dblclick("page.dblclick", selector, options);
    }
    hover(selector, options) {
      return this._mainFrame._hover("page.hover", selector, options);
    }
    check(selector, options) {
      return this._mainFrame._check("page.check", selector, true, options);
    }
    uncheck(selector, options) {
      return this._mainFrame._check("page.uncheck", selector, false, options);
    }
    setChecked(selector, checked, options) {
      return this._mainFrame._check("page.setChecked", selector, checked, options);
    }
    dragAndDrop(source, target, options) {
      return this._mainFrame._dragAndDrop("page.dragAndDrop", source, target, options);
    }
    fill(selector, value, options) {
      return this._mainFrame._fill("page.fill", selector, value, options);
    }
    press(selector, key, options) {
      return this._mainFrame._press("page.press", selector, key, options);
    }
    type(selector, text, options) {
      return this._mainFrame._type("page.type", selector, text, options);
    }
    focus(selector, options) {
      return this._mainFrame._focus("page.focus", selector, options);
    }
    selectOption(selector, values, options) {
      return this._mainFrame._selectOption("page.selectOption", selector, values, options);
    }
    setInputFiles(selector, files, options) {
      return this._mainFrame._setInputFiles("page.setInputFiles", selector, files, options);
    }
    dispatchEvent(selector, type, eventInit, options) {
      return this._mainFrame._dispatchEvent("page.dispatchEvent", selector, type, eventInit, options);
    }
    textContent(selector, options) {
      return this._mainFrame._read("locator.textContent", "page.textContent", selector, options);
    }
    innerText(selector, options) {
      return this._mainFrame._read("locator.innerText", "page.innerText", selector, options);
    }
    innerHTML(selector, options) {
      return this._mainFrame._read("locator.innerHTML", "page.innerHTML", selector, options);
    }
    getAttribute(selector, name, options) {
      return this._mainFrame._getAttribute("page.getAttribute", selector, name, options);
    }
    inputValue(selector, options) {
      return this._mainFrame._read("locator.inputValue", "page.inputValue", selector, options);
    }
    isChecked(selector, options) {
      return this._mainFrame._read("locator.isChecked", "page.isChecked", selector, options);
    }
    isDisabled(selector, options) {
      return this._mainFrame._read("locator.isDisabled", "page.isDisabled", selector, options);
    }
    isEditable(selector, options) {
      return this._mainFrame._read("locator.isEditable", "page.isEditable", selector, options);
    }
    isEnabled(selector, options) {
      return this._mainFrame._read("locator.isEnabled", "page.isEnabled", selector, options);
    }
    isHidden(selector, options) {
      return this._mainFrame._isVisible("locator.isHidden", "page.isHidden", selector, options);
    }
    isVisible(selector, options) {
      return this._mainFrame._isVisible("locator.isVisible", "page.isVisible", selector, options);
    }
    $(selector, options) {
      return this._mainFrame._elementHandle("page.$", selector, options?.strict === true, true);
    }
    $$(selector) {
      return this._mainFrame._elementHandles("page.$$", selector);
    }
    async $eval(selector, pageFunction, arg) {
      assertMaxArguments("page.$eval", arguments.length, 3);
      const api = "page.$eval";
      return await this._mainFrame._evaluateOn(api, selector, false, pageCall(api, "query", pageFunction, arg), true);
    }
    async $$eval(selector, pageFunction, arg) {
      assertMaxArguments("page.$$eval", arguments.length, 3);
      const api = "page.$$eval";
      return await this._mainFrame._evaluateAll(api, selector, pageCall(api, "query", pageFunction, arg));
    }
    waitForSelector(selector, options) {
      return this._mainFrame._waitForSelector("page.waitForSelector", selector, options, false);
    }
    goto(url, options) {
      return this._mainFrame._goto("page.goto", url, options);
    }
    reload(options) {
      return this.#history("page.reload", options);
    }
    goBack(options) {
      return this.#history("page.goBack", options);
    }
    goForward(options) {
      return this.#history("page.goForward", options);
    }
    waitForLoadState(state, options) {
      return this._mainFrame._waitForLoadState("page.waitForLoadState", state, options);
    }
    waitForNavigation(options) {
      return this._waitForNavigation("page.waitForNavigation", this._mainFrame, options);
    }
    waitForURL(url, options) {
      return this._waitForURL("page.waitForURL", this._mainFrame, url, options);
    }
    async title() {
      const api = "page.title";
      const title = await invoke("page.title", { api, timeoutMs: 0 }, { page: await this._brand(api) });
      return title;
    }
    content() {
      return this._mainFrame._content("page.content");
    }
    async emulateMedia(options = {}) {
      const api = "page.emulateMedia";
      const allowed = {
        media: ["screen", "print", "no-override"],
        colorScheme: ["dark", "light", "no-preference", "no-override"],
        reducedMotion: ["reduce", "no-preference", "no-override"],
        forcedColors: ["active", "none", "no-override"],
        contrast: ["no-preference", "more", "no-override"]
      };
      const input = {};
      for (const key of Object.keys(allowed)) {
        const value = options[key] === null ? "no-override" : options[key];
        if (value === undefined)
          continue;
        if (typeof value !== "string" || !allowed[key].includes(value))
          throw clientError(api, `${key}: expected one of (${allowed[key].join("|")})`);
        input[key] = value;
      }
      await invoke("page.emulateMedia", this._mainFrame._call(api, 0), {
        page: await this._brand(api),
        ...input
      });
    }
    addScriptTag(options = {}) {
      return this._mainFrame._addTag("page.addScriptTag", "page.addScriptTag", options);
    }
    addStyleTag(options = {}) {
      return this._mainFrame._addTag("page.addStyleTag", "page.addStyleTag", options);
    }
    setContent(html, options) {
      return this._mainFrame._setContent("page.setContent", html, options);
    }
    async evaluate(pageFunction, arg, options) {
      assertMaxArguments("page.evaluate", arguments.length, 3);
      return await this._mainFrame._evaluate("page.evaluate", pageFunction, arg, options);
    }
    addInitScript(script, arg, options) {
      const api = "page.addInitScript";
      return addInitScript(api, {
        register: async (source) => (await invoke("page.addInitScript", { api, timeoutMs: 0 }, { page: await this._brand(api), script: source })).id,
        expose: (name, callback) => exposeOnPage(this, api, name, callback, true)
      }, script, arg, options);
    }
    exposeBinding(name, callback) {
      return exposeOnPage(this, "page.exposeBinding", name, callback);
    }
    exposeFunction(name, callback) {
      return exposeOnPage(this, "page.exposeFunction", name, (_source, ...args) => Reflect.apply(callback, undefined, args));
    }
    async _evaluationScript(api, form, pageFunction, arg, options) {
      const callbacks = exposesFunctions(options) ? [] : undefined;
      const script = pageCall(api, form, pageFunction, arg, options, callbacks);
      if (!callbacks?.length || !("run" in script))
        return script;
      const settled = await Promise.allSettled(callbacks.map(([name, callback]) => exposeOnPage(this, api, name, (_source, ...args) => Reflect.apply(callback, undefined, args), true)));
      const failed = settled.find((outcome) => outcome.status === "rejected");
      if (failed) {
        await Promise.all(settled.map((outcome) => outcome.status === "fulfilled" ? outcome.value.dispose().catch(() => {}) : undefined));
        throw failed.reason;
      }
      return script;
    }
    async _bindingFrame(id) {
      if (id === null)
        return this._mainFrame;
      const known = this.#childFrames.get(id);
      if (known)
        return known;
      await this._refreshFrames("page.exposeBinding").catch(() => {});
      return this.#childFrames.get(id) ?? this._mainFrame;
    }
    waitForFunction(pageFunction, arg, options) {
      return this._mainFrame._waitForFunction("page.waitForFunction", pageFunction, arg, options);
    }
    async waitForTimeout(timeout) {
      await sleep("page.waitForTimeout", timeout);
      await this._context._refreshWorkingFrames("page.waitForTimeout");
    }
    ariaSnapshot(options) {
      return this._mainFrame._ariaSnapshot("page.ariaSnapshot", null, options);
    }
    ariaSnapshotJSON(options) {
      return this._mainFrame._ariaSnapshotJSON("page.ariaSnapshotJSON", null, options);
    }
    screenshot(options) {
      return this._mainFrame._screenshot("page.screenshot", null, options);
    }
    async pdf(options = {}) {
      const api = "page.pdf";
      if (options.path !== undefined)
        throw unsupportedError(api, "Neo returns the PDF instead of writing a file; omit path");
      const wire = pick(options, PDF_KEYS);
      for (const size of ["width", "height"])
        if (typeof wire[size] === "number")
          wire[size] = `${wire[size]}px`;
      if (typeof wire.margin === "object" && wire.margin !== null) {
        const margin = { ...wire.margin };
        for (const side of ["top", "right", "bottom", "left"])
          if (typeof margin[side] === "number")
            margin[side] = `${margin[side]}px`;
        wire.margin = margin;
      }
      return await invoke("page.pdf", { api, timeoutMs: 0 }, { page: await this._brand(api), ...wire });
    }
    async opener() {
      const api = "page.opener";
      const target = await this._brand(api);
      const { opener } = await invoke("page.opener", { api, timeoutMs: 0 }, { page: target });
      const page = this._context._pageFromResult(opener);
      return page && !page.isClosed() ? page : null;
    }
    async close(options = {}) {
      const api = "page.close";
      if (this._closed || this._tab === null) {
        this.#close();
        return;
      }
      const page = await this._brand(api);
      try {
        await invoke("page.close", { api, timeoutMs: 0 }, {
          page,
          ...options.runBeforeUnload ? { runBeforeUnload: true } : {}
        });
      } catch (error) {
        if (!["target_closed", "page_closed"].includes(String(Reflect.get(Object(error), "code"))) || options.runBeforeUnload)
          throw error;
      }
    }
    waitForEvent(event, optionsOrPredicate = {}) {
      const since = Date.now();
      return this._context._events.wait("page.waitForEvent", this, event, optionsOrPredicate, since, this._timeoutSettings);
    }
    _checkEvent(event) {
      checkReported("page.on", event, "page");
    }
    _listenersChanged(event, listening) {
      if (isReported(event, "page"))
        this._context._events.listen(this, event, listening);
    }
    async _brand(api) {
      if (this._tab === null) {
        if (this._closed)
          throw clientError(api, "Target page, context or browser has been closed");
        return { $neo: "page", tab: null };
      }
      this._working = true;
      return { $neo: "page", tab: this._tab };
    }
    async _openedBrand(api) {
      if (this._tab === null)
        await this._refreshFrames(api);
      const brand = await this._brand(api);
      if (brand.tab === null)
        throw new Error(`${api}: Neo opened no tab for the run's page`);
      return brand;
    }
    _handle(value) {
      if (value instanceof ElementHandle)
        return value;
      const brand = elementBrand(value);
      return brand ? new ElementHandle(this, brand) : null;
    }
    async _navigating(work) {
      this._navigationsInFlight++;
      try {
        return await work();
      } finally {
        this._navigationsInFlight--;
      }
    }
    _evaluated(frames, at) {
      if (this.#payloadCurrent(at) && Array.isArray(frames))
        this._applyFrames(frames, at);
    }
    async _navigated(api, frame, result, at) {
      if (this.#payloadCurrent(at)) {
        if (typeof result?.url === "string") {
          if (frame === this._mainFrame) {
            this._url = result.url;
            this.#urlKnown = true;
          } else
            frame._url = result.url;
        }
        if (Array.isArray(result?.frames))
          this._applyFrames(result.frames, at);
        else
          await this._refreshFrames(api);
      } else if (!this._closed) {
        await this._refreshFrames(api).catch(() => {
          return;
        });
      }
      return typeof result?.response?.status === "number" ? new Response(frame, result.response) : null;
    }
    #payloadCurrent(at) {
      return !this._closed && at !== null && at.payloadCurrent === true && at.seq === this.#observedSeq;
    }
    async _readInitialFrames() {
      if (this._tab === null)
        return;
      await this._refreshFrames("page.frames").catch(() => {
        return;
      });
    }
    async _refreshFrames(api, timeoutMs = 0) {
      const { value: tree, observed } = await invokeObserved("page.frames", { api, timeoutMs }, { page: await this._brand(api) });
      if (this.#payloadCurrent(observed) && Array.isArray(tree?.frames))
        this._applyFrames(tree.frames, observed);
    }
    _applyFrames(infos, at = null) {
      const current = (id) => !at || (this.#frameSequences.get(id) ?? 0) <= at.events;
      const remember = (id) => {
        if (at)
          this.#frameSequences.set(id, Math.max(at.events, this.#frameSequences.get(id) ?? 0));
      };
      const main = infos.find((info) => info.parentId === null);
      if (main && current(null)) {
        remember(null);
        this._url = main.url;
        this.#urlKnown = true;
        this._mainFrame._name = main.name;
      }
      const present = new Map;
      for (const info of infos) {
        if (info.parentId === null || !current(info.id))
          continue;
        present.set(info.id, info);
        remember(info.id);
        let frame = this.#knownFrames.get(info.id);
        if (!frame) {
          frame = new Frame(this, info.id, this._mainFrame);
          this.#knownFrames.set(info.id, frame);
        }
        frame._detached = false;
        this.#childFrames.set(info.id, frame);
        frame._update(info);
      }
      for (const [id, frame] of this.#childFrames) {
        if (!current(id))
          continue;
        const info = present.get(id);
        if (!info) {
          remember(id);
          frame._detached = true;
          this.#childFrames.delete(id);
          continue;
        }
        frame._parentFrame = info.parentId === main?.id ? this._mainFrame : this.#childFrames.get(info.parentId ?? "") ?? this._mainFrame;
      }
      this.#pruneFrames();
    }
    _eventFrame(seq, event, info) {
      const delivered = this.#frameEvents.get(seq);
      if (delivered)
        return delivered;
      const id = info.parentId === null ? null : info.id;
      const current = id === null ? this._mainFrame : this.#knownFrames.get(id);
      if (current && seq <= (this.#frameSequences.get(id) ?? 0)) {
        return this.#rememberFrameEvent(seq, current);
      }
      this.#frameSequences.set(id, seq);
      let frame;
      if (info.parentId === null) {
        frame = this._mainFrame;
        this._url = info.url;
        this.#urlKnown = true;
      } else {
        frame = this.#knownFrames.get(info.id) ?? new Frame(this, info.id, this._mainFrame);
        this.#knownFrames.set(info.id, frame);
        frame._parentFrame = this.#knownFrames.get(info.parentId) ?? this._mainFrame;
        if (event === "framedetached") {
          frame._detached = true;
          this.#childFrames.delete(info.id);
        } else {
          frame._detached = false;
          this.#childFrames.set(info.id, frame);
        }
      }
      if (event !== "frameattached" || frame.url() === "")
        frame._update(info);
      this.#pruneFrames();
      return this.#rememberFrameEvent(seq, frame);
    }
    #rememberFrameEvent(seq, frame) {
      this.#frameEvents.set(seq, frame);
      if (this.#frameEvents.size > 1024) {
        const oldest = this.#frameEvents.keys().next().value;
        if (oldest !== undefined)
          this.#frameEvents.delete(oldest);
      }
      return frame;
    }
    #pruneFrames() {
      let retained = this.#knownFrames.size - this.#childFrames.size;
      for (const [id, frame] of this.#knownFrames) {
        if (retained <= 1024)
          break;
        if (frame._detached) {
          this.#knownFrames.delete(id);
          this.#frameSequences.delete(id);
          retained--;
        }
      }
    }
    #detachFrame(frame) {
      const pending = [frame];
      for (const removed of pending) {
        pending.push(...removed.childFrames());
        if (removed._id === null)
          continue;
        removed._detached = true;
        this.#childFrames.delete(removed._id);
        this.#frameSequences.set(removed._id, Math.max(this.#observedHead, this.#frameSequences.get(removed._id) ?? 0));
      }
      this.#pruneFrames();
    }
    async#frameWaitFailed(api, frame, error) {
      if (frame._id === null || !(error instanceof Error) || Reflect.get(error, "code") !== "target_closed")
        return;
      if (this._closed || error.message.startsWith(`${api}: Navigating frame was detached!`)) {
        this.#detachFrame(frame);
        return;
      }
      if (!error.message.startsWith(`${api}: Frame was detached`))
        return;
      await this._refreshFrames(api, 1000).catch(() => {
        return;
      });
      if (this._closed)
        this.#detachFrame(frame);
    }
    #applyViewport(size) {
      if (this.#viewportSet || size === undefined)
        return;
      if (this.#viewportSize === null || size === null || this.#viewportSize.width !== size.width || this.#viewportSize.height !== size.height)
        this.#viewportSize = size;
    }
    _applyFact(fact, at, approved = null) {
      if (fact.tab !== undefined)
        this._tab = fact.tab;
      if (at === null) {
        this.#applyViewport(fact.viewportSize);
        if (fact.url !== undefined && !this.#urlKnown) {
          this._url = fact.url;
          this.#urlKnown = true;
        }
        return;
      }
      if (at.seq < this.#observedSeq)
        return;
      this.#observedSeq = at.seq;
      this.#observedHead = Math.max(this.#observedHead, at.events);
      this.#applyViewport(fact.viewportSize);
      if (fact.url !== undefined) {
        this._url = fact.url;
        this.#urlKnown = true;
      }
      if (fact.title !== undefined)
        this._title = fact.title;
      if (fact.closed === true) {
        this.#approvedClose = approved === null ? null : { seq: approved, observed: at.seq };
        if (approved === null)
          this.#close();
        return;
      }
      if (fact.closed !== false)
        return;
      this.#approvedClose = null;
      if (this._closed && at.events < this.#closeEvent)
        return;
      this.#liveHead = at.events;
      this._closed = false;
    }
    _closeEvent(seq, at = null) {
      if (this.#approvedClose?.seq === seq) {
        this.#closeEvent = Math.max(this.#closeEvent, seq);
        this.#approvedClose = null;
        this.#close();
        return;
      }
      if (at !== null && at.seq < this.#observedSeq)
        return;
      if (this.#approvedClose !== null) {
        this.#closeEvent = Math.max(this.#closeEvent, seq);
        return;
      }
      if (seq <= this.#closeEvent)
        return;
      this.#closeEvent = seq;
      if (seq <= this.#liveHead)
        return;
      this.#close();
    }
    _settleClose(at) {
      if (at === null || this.#approvedClose?.observed !== at.seq)
        return;
      this.#closeEvent = Math.max(this.#closeEvent, this.#approvedClose.seq);
      this.#approvedClose = null;
      this.#close();
    }
    #close() {
      if (this._closed)
        return;
      this._closed = true;
      this.emit("close", this);
    }
    async _waitForURL(api, frame, url, options = {}) {
      const waitUntil = knownLoadState(options.waitUntil ?? "load");
      if (waitUntil === null)
        throw clientError(api, loadStateError("waitUntil"));
      const timeout = frame._navigationTimeout(options);
      const signal = signalOf(api, options);
      await untilAborted(signal, () => abortedError(api, signal), () => this.#waitForURL(api, frame, url, waitUntil, timeout, signal));
    }
    async#waitForURL(api, frame, url, waitUntil, timeout, signal) {
      const args = await frame._frameArgs(api);
      const nativeWait = async (work) => {
        try {
          return await work();
        } catch (error) {
          await this.#frameWaitFailed(api, frame, error);
          throw error;
        }
      };
      const wait = (pattern, until, budget) => nativeWait(() => invokeObserved("page.waitForURL", frame._call(api, budget), { ...args, url: pattern, waitUntil: until }, undefined, signal));
      if (typeof url !== "function") {
        const pattern = typeof url === "string" ? { glob: url } : url instanceof RegExp ? { regexSource: url.source, regexFlags: url.flags } : undefined;
        if (!pattern)
          throw clientError(api, "url: expected a string, a RegExp or a function");
        const { value: result, observed } = await wait(pattern, waitUntil, timeout);
        await this._navigated(api, frame, result, observed);
        return;
      }
      const started = Date.now();
      for (;; ) {
        const current = frame.url();
        let matched;
        try {
          matched = await url(URL.parse(current) ?? current);
        } catch (error) {
          throw prefixed(api, error);
        }
        const elapsed = Date.now() - started;
        const remaining = timeout === 0 ? 0 : Math.max(1, timeout - elapsed);
        if (matched) {
          await nativeWait(() => frame._waitForLoadState(api, waitUntil, {
            timeout: remaining,
            signal
          }));
          return;
        }
        const { value: result, observed } = await wait({ regexSource: `^(?!${escapeRegExp(current)}$)`, regexFlags: "" }, "commit", remaining);
        await this._navigated(api, frame, result, observed);
      }
    }
    async _waitForNavigation(api, frame, options = {}) {
      const signal = signalOf(api, options);
      return await untilAborted(signal, () => abortedError(api, signal), () => this.#waitForNavigation(api, frame, options, signal));
    }
    async#waitForNavigation(api, frame, options, signal) {
      const since = Date.now();
      let after = this.#liveHead;
      const waitUntil = loadState(api, "waitUntil", options.waitUntil ?? "load");
      const timeout = frame._navigationTimeout(options);
      const matcher = options.url;
      const predicate = typeof matcher === "function" ? matcher : undefined;
      const url = typeof matcher === "string" ? { glob: matcher } : matcher instanceof RegExp ? { regexSource: matcher.source, regexFlags: matcher.flags } : undefined;
      if (matcher !== undefined && !url && !predicate)
        throw clientError(api, "url: expected a string, a RegExp or a function");
      const args = await frame._frameArgs(api);
      const deadline = new EventDeadline(api, null, since, timeout);
      const rethrow = (error) => {
        throw deadline.named(error);
      };
      for (;; ) {
        const { value: result, observed } = await deadline.race(invokeObserved("page.waitForNavigation", frame._call(api, deadline.left()), {
          ...args,
          since,
          after,
          ...url ? { url } : {},
          waitUntil: predicate ? "commit" : waitUntil
        }, undefined, signal)).catch(rethrow);
        const response = await deadline.race(this._navigated(api, frame, result, observed));
        if (!predicate) {
          deadline.left();
          return response;
        }
        after = result.navigationSeq;
        let matched;
        try {
          matched = await deadline.race(predicate(URL.parse(result.url ?? "") ?? result.url));
        } catch (error) {
          throw prefixed(api, error);
        }
        if (matched) {
          await deadline.race(frame._waitForLoadState(api, waitUntil, {
            timeout: deadline.left(),
            signal
          }).catch(rethrow));
          deadline.left();
          return response;
        }
      }
    }
    async#history(api, options = {}) {
      const waitUntil = loadState(api, "waitUntil", options.waitUntil ?? "load");
      const signal = signalOf(api, options);
      return await this._navigating(async () => {
        const { value: result, observed } = await invokeObserved(api, this._mainFrame._call(api, this._mainFrame._navigationTimeout(options)), { page: await this._brand(api), waitUntil }, undefined, signal);
        return await this._navigated(api, this._mainFrame, result, observed);
      });
    }
  }

  // crates/neo-sdk/neo-playwright-js/src/prelude/context.ts
  function pageFact(value) {
    if (value instanceof Page)
      return value._tab === null ? null : { tab: value._tab };
    if (typeof value !== "object" || value === null)
      return null;
    const tab = Reflect.get(value, "tab");
    return typeof tab === "number" ? { tab } : null;
  }

  class BrowserContext extends EventEmitter {
    _timeoutSettings = new TimeoutSettings;
    _events;
    _exposed = new Map;
    _browser = null;
    #byTab = new Map;
    #unopened = null;
    #started = false;
    constructor() {
      super();
      this._events = new EventHub(this);
      markHandle(this);
    }
    pages() {
      return [...this.#byTab.values()].filter((page) => !page.isClosed());
    }
    browser() {
      return this._browser;
    }
    isClosed() {
      return false;
    }
    setDefaultTimeout(timeout) {
      this._timeoutSettings.setDefaultTimeout(timeout);
    }
    setDefaultNavigationTimeout(timeout) {
      this._timeoutSettings.setDefaultNavigationTimeout(timeout);
    }
    backgroundPages() {
      return [];
    }
    serviceWorkers() {
      return [];
    }
    addInitScript(script, arg, options) {
      const api = "browserContext.addInitScript";
      return addInitScript(api, {
        register: async (source) => (await invoke("context.addInitScript", { api, timeoutMs: 0 }, { script: source })).id,
        expose: (name, callback) => exposeOnContext(this, api, name, callback, true)
      }, script, arg, options);
    }
    exposeBinding(name, callback) {
      return exposeOnContext(this, "browserContext.exposeBinding", name, callback);
    }
    exposeFunction(name, callback) {
      return exposeOnContext(this, "browserContext.exposeFunction", name, (_source, ...args) => Reflect.apply(callback, undefined, args));
    }
    async newPage() {
      return await this.#openPage("browserContext.newPage");
    }
    waitForEvent(event, optionsOrPredicate = {}) {
      const since = Date.now();
      return this._events.wait("browserContext.waitForEvent", null, event, optionsOrPredicate, since, this._timeoutSettings);
    }
    toJSON() {
      return { pages: this.pages().length };
    }
    _checkEvent(event) {
      checkReported("browserContext.on", event, "context");
    }
    _listenersChanged(event, listening) {
      if (isReported(event, "context"))
        this._events.listen(null, event, listening);
    }
    _pageFor(fact, at = null, approved = null) {
      let page = this.#byTab.get(fact.tab);
      if (!page) {
        page = new Page(this, fact.tab, fact.url);
        page._working = this.#started;
        this.#byTab.set(fact.tab, page);
      }
      page._applyFact(fact, at, approved);
      return page;
    }
    _pageFromResult(value, url) {
      const fact = pageFact(value);
      if (!fact)
        return null;
      if (url !== undefined)
        fact.url = url;
      return this._pageFor(fact);
    }
    _runPage(init) {
      for (const fact of init.pages)
        this._pageFor(fact, init.observed);
      const page = init.page ? this._pageFor(init.page, init.observed) : new Page(this, null);
      if (!init.page)
        this.#unopened = page;
      page._working = true;
      this.#started = true;
      return page;
    }
    async _refreshWorkingFrames(api) {
      const working = this.pages().filter((page) => page._working && page._tab !== null && page._navigationsInFlight === 0);
      await Promise.all(working.map((page) => page._refreshFrames(api).catch(() => {
        return;
      })));
    }
    _applyReply({ pages, defaultPage, observed }) {
      const page = this.#unopened;
      if (defaultPage && page) {
        this.#unopened = null;
        page._tab = defaultPage.tab;
        this.#byTab.set(defaultPage.tab, page);
      }
      const approved = new Map;
      for (const close of observed?.closes ?? [])
        approved.set(close.tab, Math.max(approved.get(close.tab) ?? 0, close.seq));
      for (const fact of pages)
        this._pageFor(fact, observed, approved.get(fact.tab) ?? null);
    }
    async#openPage(api) {
      const opened = await invoke("context.newPage", { api, timeoutMs: 0 }, {});
      const page = this._pageFromResult(opened.page, opened.url);
      if (!page)
        throw internalError(api, "context.newPage returned no page");
      return page;
    }
  }

  // crates/neo-sdk/neo-playwright-js/src/prelude/expect-equality.ts
  function stringify(value, seen = new Set) {
    if (typeof value === "string")
      return `"${value.replace(/["\\]/g, "\\$&")}"`;
    if (typeof value === "number")
      return Object.is(value, -0) ? "-0" : String(value);
    if (typeof value === "bigint")
      return `${value}n`;
    if (typeof value === "function")
      return `[Function ${value.name || "anonymous"}]`;
    if (typeof value !== "object" || value === null)
      return String(value);
    if (value instanceof AsymmetricMatcher)
      return value.toString();
    if (value instanceof RegExp)
      return String(value);
    if (value instanceof Date)
      return Number.isNaN(+value) ? "Date { NaN }" : value.toISOString();
    if (value instanceof Error)
      return `[${value.name}: ${value.message}]`;
    if (value instanceof Locator)
      return `Locator ${value.toString()}`;
    if (seen.has(value))
      return "[Circular]";
    seen.add(value);
    try {
      if (Array.isArray(value))
        return `[${value.map((item) => stringify(item, seen)).join(", ")}]`;
      if (value instanceof Map)
        return `Map {${[...value].map(([key, item]) => `${stringify(key, seen)} => ${stringify(item, seen)}`).join(", ")}}`;
      if (value instanceof Set)
        return `Set {${[...value].map((item) => stringify(item, seen)).join(", ")}}`;
      const name = Object.getPrototypeOf(value)?.constructor?.name;
      const prefix = name && name !== "Object" ? `${name} ` : "";
      const entries = Object.entries(value).map(([key, item]) => `${JSON.stringify(key)}: ${stringify(item, seen)}`);
      return `${prefix}{${entries.join(", ")}}`;
    } finally {
      seen.delete(value);
    }
  }
  function receiverText(receiver) {
    if (typeof receiver === "object" && receiver !== null)
      return `${Object.getPrototypeOf(receiver)?.constructor?.name ?? "Object"} ${stringify(receiver)}`;
    return String(receiver);
  }

  class AsymmetricMatcher {
    inverse;
    constructor(inverse) {
      this.inverse = inverse;
    }
    toString() {
      return `${this.inverse ? "Not" : ""}${this.describe()}`;
    }
  }

  class Any extends AsymmetricMatcher {
    expected;
    constructor(expected) {
      super(false);
      this.expected = expected;
      if (expected === undefined)
        throw unprefixedClientError("expect.any", "any() expects to be passed a constructor function. Please pass one or use anything() to match any object.", "TypeError");
    }
    asymmetricMatch(other) {
      if (this.expected === String)
        return typeof other === "string" || other instanceof String;
      if (this.expected === Number)
        return typeof other === "number" || other instanceof Number;
      if (this.expected === Boolean)
        return typeof other === "boolean" || other instanceof Boolean;
      if (this.expected === Function)
        return typeof other === "function";
      if (this.expected === Symbol)
        return typeof other === "symbol";
      if (this.expected === BigInt)
        return typeof other === "bigint";
      if (this.expected === Object)
        return typeof other === "object" && other !== null;
      return typeof this.expected === "function" && other instanceof this.expected;
    }
    describe() {
      return `Any<${typeof this.expected === "function" ? this.expected.name : String(this.expected)}>`;
    }
  }

  class Anything extends AsymmetricMatcher {
    asymmetricMatch(other) {
      return other !== null && other !== undefined;
    }
    describe() {
      return "Anything";
    }
  }

  class ArrayContaining extends AsymmetricMatcher {
    expected;
    constructor(expected, inverse) {
      super(inverse);
      this.expected = expected;
    }
    asymmetricMatch(other) {
      const matches = Array.isArray(other) && this.expected.every((item) => other.some((candidate) => equals(candidate, item)));
      return this.inverse ? !matches : matches;
    }
    describe() {
      return `ArrayContaining ${stringify(this.expected)}`;
    }
  }

  class ArrayOf extends AsymmetricMatcher {
    expected;
    constructor(expected, inverse) {
      super(inverse);
      this.expected = expected;
    }
    asymmetricMatch(other) {
      const matches = Array.isArray(other) && other.every((item) => equals(item, this.expected));
      return this.inverse ? !matches : matches;
    }
    describe() {
      return `ArrayOf ${stringify(this.expected)}`;
    }
  }

  class ObjectContaining extends AsymmetricMatcher {
    expected;
    constructor(expected, inverse) {
      super(inverse);
      this.expected = expected;
    }
    asymmetricMatch(other) {
      const matches = typeof other === "object" && other !== null && Object.entries(this.expected).every(([key, item]) => (key in other) && equals(Reflect.get(other, key), item));
      return this.inverse ? !matches : matches;
    }
    describe() {
      return `ObjectContaining ${stringify(this.expected)}`;
    }
  }

  class StringContaining extends AsymmetricMatcher {
    expected;
    constructor(expected, inverse) {
      super(inverse);
      this.expected = expected;
    }
    asymmetricMatch(other) {
      const matches = typeof other === "string" && other.includes(this.expected);
      return this.inverse ? !matches : matches;
    }
    describe() {
      return `StringContaining ${stringify(this.expected)}`;
    }
  }

  class StringMatching extends AsymmetricMatcher {
    expected;
    constructor(expected, inverse) {
      super(inverse);
      this.expected = typeof expected === "string" ? new RegExp(expected) : expected;
    }
    asymmetricMatch(other) {
      const matches = typeof other === "string" && this.expected.test(other);
      return this.inverse ? !matches : matches;
    }
    describe() {
      return `StringMatching ${String(this.expected)}`;
    }
  }

  class CloseTo extends AsymmetricMatcher {
    expected;
    precision;
    constructor(expected, precision, inverse) {
      super(inverse);
      this.expected = expected;
      this.precision = precision;
    }
    asymmetricMatch(other) {
      const matches = typeof other === "number" && closeTo(other, this.expected, this.precision);
      return this.inverse ? !matches : matches;
    }
    describe() {
      return `NumberCloseTo ${this.expected} (${this.precision} digits)`;
    }
  }
  function closeTo(received, expected, precision) {
    if (received === Number.POSITIVE_INFINITY && expected === Number.POSITIVE_INFINITY)
      return true;
    if (received === Number.NEGATIVE_INFINITY && expected === Number.NEGATIVE_INFINITY)
      return true;
    return Math.abs(expected - received) < 10 ** -precision / 2;
  }
  function equals(a, b, strict = false, stack = []) {
    if (a instanceof AsymmetricMatcher)
      return a.asymmetricMatch(b);
    if (b instanceof AsymmetricMatcher)
      return b.asymmetricMatch(a);
    if (a instanceof Error && b instanceof Error)
      return a.message === b.message;
    if (Object.is(a, b))
      return true;
    if (typeof a !== "object" || typeof b !== "object" || !a || !b)
      return false;
    if (Object.prototype.toString.call(a) !== Object.prototype.toString.call(b))
      return false;
    if (a instanceof Date || a instanceof RegExp || isBoxed(a))
      return valueEquals(a, b);
    if (strict && Object.getPrototypeOf(a) !== Object.getPrototypeOf(b))
      return false;
    if (stack.includes(a))
      return true;
    return structureEquals(a, b, strict, [...stack, a]);
  }
  function isBoxed(value) {
    return value instanceof Number || value instanceof String || value instanceof Boolean;
  }
  function valueEquals(a, b) {
    if (a instanceof Date && b instanceof Date)
      return +a === +b || Number.isNaN(+a) && Number.isNaN(+b);
    if (a instanceof RegExp && b instanceof RegExp)
      return a.source === b.source && a.flags === b.flags;
    return Object.is(a.valueOf(), b.valueOf());
  }
  function structureEquals(a, b, strict, stack) {
    if (a instanceof Map && b instanceof Map)
      return a.size === b.size && [...a].every(([key, value]) => b.has(key) && equals(value, b.get(key), strict, stack));
    if (a instanceof Set && b instanceof Set)
      return a.size === b.size && [...a].every((value) => b.has(value) || [...b].some((other) => equals(value, other, strict, stack)));
    if (Array.isArray(a) && Array.isArray(b))
      return a.length === b.length && a.every((_, index) => (!strict || (index in a) === (index in b)) && equals(a[index], b[index], strict, stack));
    const keys = (value) => Object.keys(value).filter((key) => strict || Reflect.get(value, key) !== undefined);
    const aKeys = keys(a);
    return aKeys.length === keys(b).length && aKeys.every((key) => Object.hasOwn(b, key) && equals(Reflect.get(a, key), Reflect.get(b, key), strict, stack));
  }
  function matchesObject(received, expected) {
    if (expected instanceof AsymmetricMatcher)
      return expected.asymmetricMatch(received);
    if (typeof expected !== "object" || expected === null || expected instanceof Date || expected instanceof RegExp)
      return equals(received, expected);
    if (typeof received !== "object" || received === null)
      return false;
    if (Array.isArray(expected))
      return Array.isArray(received) && received.length === expected.length && expected.every((item, index) => matchesObject(received[index], item));
    return Object.entries(expected).every(([key, item]) => (key in received) && matchesObject(Reflect.get(received, key), item));
  }

  // crates/neo-sdk/neo-playwright-js/src/prelude/expect-values.ts
  function header(name, state, receiver = "received", args = "expected") {
    const modifier = state.promise ? `.${state.promise}` : "";
    return `expect(${receiver})${modifier}${state.isNot ? ".not" : ""}.${name}(${args})`;
  }
  function fail(state, message) {
    throw assertionError(state.message ? `${state.message}

${message}` : message);
  }
  function check(state, pass, message) {
    if (pass !== state.isNot)
      return;
    fail(state, message());
  }
  function expectedLine(state, value) {
    return `Expected: ${state.isNot ? "not " : ""}${stringify(value)}`;
  }
  function valueMatchers(actual, state) {
    const plain = (name, args = "expected") => header(name, state, "received", args);
    return {
      toBe: (expected) => check(state, Object.is(actual, expected), () => [
        `${plain("toBe")} // Object.is equality`,
        "",
        expectedLine(state, expected),
        ...state.isNot ? [] : [`Received: ${stringify(actual)}`]
      ].join(`
`)),
      toEqual: (expected) => check(state, equals(actual, expected), () => [
        `${plain("toEqual")} // deep equality`,
        "",
        expectedLine(state, expected),
        ...state.isNot ? [] : [`Received: ${stringify(actual)}`]
      ].join(`
`)),
      toStrictEqual: (expected) => check(state, equals(actual, expected, true), () => [
        `${plain("toStrictEqual")} // deep equality`,
        "",
        expectedLine(state, expected),
        ...state.isNot ? [] : [`Received: ${stringify(actual)}`]
      ].join(`
`)),
      toBeTruthy: () => check(state, Boolean(actual), () => `${plain("toBeTruthy", "")}

Received: ${stringify(actual)}`),
      toBeFalsy: () => check(state, !actual, () => `${plain("toBeFalsy", "")}

Received: ${stringify(actual)}`),
      toBeNull: () => check(state, actual === null, () => `${plain("toBeNull", "")}

Received: ${stringify(actual)}`),
      toBeUndefined: () => check(state, actual === undefined, () => `${plain("toBeUndefined", "")}

Received: ${stringify(actual)}`),
      toBeDefined: () => check(state, actual !== undefined, () => `${plain("toBeDefined", "")}

Received: ${stringify(actual)}`),
      toBeNaN: () => check(state, Number.isNaN(actual), () => `${plain("toBeNaN", "")}

Received: ${stringify(actual)}`),
      toBeGreaterThan: (expected) => compare(state, actual, expected, ">", "toBeGreaterThan"),
      toBeGreaterThanOrEqual: (expected) => compare(state, actual, expected, ">=", "toBeGreaterThanOrEqual"),
      toBeLessThan: (expected) => compare(state, actual, expected, "<", "toBeLessThan"),
      toBeLessThanOrEqual: (expected) => compare(state, actual, expected, "<=", "toBeLessThanOrEqual"),
      toBeCloseTo: (expected, precision = 2) => check(state, typeof actual === "number" && closeTo(actual, expected, precision), () => `${plain("toBeCloseTo", "expected, precision")}

Expected: ${state.isNot ? "not " : ""}${stringify(expected)}
Received: ${stringify(actual)}

Expected precision:    ${precision}`),
      toBeInstanceOf: (expected) => check(state, typeof expected === "function" && actual instanceof expected, () => `${plain("toBeInstanceOf")}

Expected constructor: ${state.isNot ? "not " : ""}${typeof expected === "function" ? expected.name : stringify(expected)}
Received value: ${stringify(actual)}`),
      toContain: (expected) => {
        const pass = typeof actual === "string" ? actual.includes(String(expected)) : actual !== null && actual !== undefined && typeof Reflect.get(Object(actual), Symbol.iterator) === "function" && [...actual].includes(expected);
        check(state, pass, () => `${plain("toContain")} // indexOf

Expected value: ${state.isNot ? "not " : ""}${stringify(expected)}
Received: ${stringify(actual)}`);
      },
      toContainEqual: (expected) => {
        const pass = actual !== null && actual !== undefined && typeof Reflect.get(Object(actual), Symbol.iterator) === "function" && [...actual].some((item) => equals(item, expected));
        check(state, pass, () => `${plain("toContainEqual")} // deep equality

Expected value: ${state.isNot ? "not " : ""}${stringify(expected)}
Received: ${stringify(actual)}`);
      },
      toHaveLength: (expected) => {
        const length = actual === null || actual === undefined ? undefined : Reflect.get(Object(actual), "length");
        check(state, length === expected, () => `${plain("toHaveLength")}

Expected length: ${state.isNot ? "not " : ""}${expected}
Received length: ${String(length)}
Received: ${stringify(actual)}`);
      },
      toHaveProperty: (path, ...value) => {
        const keys = Array.isArray(path) ? path : path.split(".");
        let current = actual;
        let found = true;
        for (const key of keys) {
          if (current === null || current === undefined || !(String(key) in Object(current))) {
            found = false;
            break;
          }
          current = Reflect.get(Object(current), key);
        }
        const pass = found && (value.length === 0 || equals(current, value[0]));
        check(state, pass, () => `${plain("toHaveProperty", value.length ? "path, value" : "path")}

Expected path: ${stringify(path)}${value.length ? `
Expected value: ${state.isNot ? "not " : ""}${stringify(value[0])}` : ""}
Received: ${found ? stringify(current) : stringify(actual)}`);
      },
      toMatch: (expected) => {
        if (typeof actual !== "string")
          fail(state, `${plain("toMatch")}

received value must be a string
Received: ${stringify(actual)}`);
        const pass = typeof expected === "string" ? actual.includes(expected) : new RegExp(expected).test(actual);
        check(state, pass, () => `${plain("toMatch")}

Expected ${typeof expected === "string" ? "substring" : "pattern"}: ${state.isNot ? "not " : ""}${stringify(expected)}
Received string: ${stringify(actual)}`);
      },
      toMatchObject: (expected) => check(state, matchesObject(actual, expected), () => `${plain("toMatchObject")}

${expectedLine(state, expected)}
Received: ${stringify(actual)}`),
      toThrow: (expected) => thrown(actual, expected, state, "toThrow"),
      toThrowError: (expected) => thrown(actual, expected, state, "toThrowError")
    };
  }
  function compare(state, actual, expected, operator, name) {
    const pass = typeof actual === "number" || typeof actual === "bigint" ? operator === ">" ? actual > expected : operator === ">=" ? actual >= expected : operator === "<" ? actual < expected : actual <= expected : false;
    check(state, pass, () => `${header(name, state)}

Expected: ${state.isNot ? "not " : ""}${operator} ${stringify(expected)}
Received:${" ".repeat(operator.length + 1)}${stringify(actual)}`);
  }
  function thrown(actual, expected, state, name) {
    let error;
    let didThrow = false;
    if (state.promise === "rejects") {
      error = actual;
      didThrow = true;
    } else {
      if (typeof actual !== "function")
        fail(state, `${header(name, state)}

Received value must be a function
Received: ${stringify(actual)}`);
      try {
        Reflect.apply(actual, undefined, []);
      } catch (caught) {
        error = caught;
        didThrow = true;
      }
    }
    const message = error instanceof Error ? error.message : typeof error === "string" ? error : stringify(error);
    let pass = didThrow;
    if (didThrow && expected !== undefined) {
      if (typeof expected === "string")
        pass = message.includes(expected);
      else if (expected instanceof RegExp)
        pass = expected.test(message);
      else if (typeof expected === "function")
        pass = error instanceof expected;
      else if (expected instanceof AsymmetricMatcher)
        pass = expected.asymmetricMatch(error);
      else if (expected instanceof Error)
        pass = message === expected.message;
    }
    check(state, pass, () => `${header(name, state)}

${expected === undefined ? "" : `Expected: ${state.isNot ? "not " : ""}${stringify(expected)}
`}${didThrow ? `Received message: ${stringify(message)}` : "Received function did not throw"}`);
  }

  // crates/neo-sdk/neo-playwright-js/src/prelude/expect-retrying.ts
  function textValues(items, options = {}) {
    return items.map((item) => {
      const value = {};
      if (typeof item === "string")
        value.string = item;
      else {
        value.regexSource = item.source;
        value.regexFlags = item.flags;
      }
      if (options.matchSubstring !== undefined)
        value.matchSubstring = options.matchSubstring;
      if (options.ignoreCase !== undefined)
        value.ignoreCase = options.ignoreCase;
      if (options.normalizeWhiteSpace !== undefined)
        value.normalizeWhiteSpace = options.normalizeWhiteSpace;
      return value;
    });
  }
  function truthy(expression, word, arg, options, extra = {}) {
    return {
      receiver: "Locator",
      expectation: arg,
      expected: (isNot) => `Expected: ${isNot ? "not " : ""}${word}`,
      params: { expression, ...extra },
      options
    };
  }
  function argumentError(matcher, message) {
    return plainError(`expect.${matcher}`, message);
  }
  function text(matcher, receiver, expression, expected, values, options, extra = {}) {
    if (typeof expected !== "string" && !(expected instanceof RegExp))
      throw argumentError(matcher, "expected value must be a string or regular expression");
    const substring = values[0]?.matchSubstring === true;
    const suffix = typeof expected === "string" ? substring ? " substring" : "" : " pattern";
    const params = {
      expression,
      expectedText: values,
      ...extra
    };
    if (options?.useInnerText !== undefined)
      params.useInnerText = options.useInnerText;
    return {
      receiver,
      expectation: "expected",
      expected: (isNot) => `Expected${suffix}: ${isNot ? "not " : ""}${stringify(expected)}`,
      params,
      options
    };
  }
  function equal(expression, expected, options, extra = {}) {
    const params = { expression, ...extra };
    if (options?.useInnerText !== undefined)
      params.useInnerText = options.useInnerText;
    return {
      receiver: "Locator",
      expectation: "expected",
      expected: (isNot) => `Expected: ${isNot ? "not " : ""}${stringify(expected)}`,
      params,
      options
    };
  }
  var RETRYING = {
    toBeAttached: (options) => {
      const attached = options?.attached !== false;
      return truthy(attached ? "to.be.attached" : "to.be.detached", attached ? "attached" : "detached", attached ? "" : "{ attached: false }", options);
    },
    toBeChecked: (options) => {
      const expectedValue = {};
      if (options?.checked !== undefined)
        expectedValue.checked = options.checked;
      if (options?.indeterminate !== undefined)
        expectedValue.indeterminate = options.indeterminate;
      const word = options?.indeterminate ? "indeterminate" : options?.checked === false ? "unchecked" : "checked";
      const arg = options?.indeterminate ? "{ indeterminate: true }" : options?.checked === false ? "{ checked: false }" : "";
      return truthy("to.be.checked", word, arg, options, { expectedValue });
    },
    toBeDisabled: (options) => truthy("to.be.disabled", "disabled", "", options),
    toBeEditable: (options) => {
      const editable = options?.editable !== false;
      return truthy(editable ? "to.be.editable" : "to.be.readonly", editable ? "editable" : "readOnly", editable ? "" : "{ editable: false }", options);
    },
    toBeEmpty: (options) => truthy("to.be.empty", "empty", "", options),
    toBeEnabled: (options) => {
      const enabled = options?.enabled !== false;
      return truthy(enabled ? "to.be.enabled" : "to.be.disabled", enabled ? "enabled" : "disabled", enabled ? "" : "{ enabled: false }", options);
    },
    toBeFocused: (options) => truthy("to.be.focused", "focused", "", options),
    toBeHidden: (options) => truthy("to.be.hidden", "hidden", "", options),
    toBeVisible: (options) => {
      const visible = options?.visible !== false;
      return truthy(visible ? "to.be.visible" : "to.be.hidden", visible ? "visible" : "hidden", visible ? "" : "{ visible: false }", options);
    },
    toBeInViewport: (options) => truthy("to.be.in.viewport", "in viewport", "", options, options?.ratio === undefined ? {} : { expectedNumber: options.ratio }),
    toContainText: (expected, options) => Array.isArray(expected) ? equal("to.contain.text.array", expected, options, {
      expectedText: textValues(expected, {
        matchSubstring: true,
        normalizeWhiteSpace: true,
        ignoreCase: options?.ignoreCase
      })
    }) : text("toContainText", "Locator", "to.have.text", expected, textValues([expected], {
      matchSubstring: true,
      normalizeWhiteSpace: true,
      ignoreCase: options?.ignoreCase
    }), options),
    toHaveAccessibleDescription: (expected, options) => text("toHaveAccessibleDescription", "Locator", "to.have.accessible.description", expected, textValues([expected], {
      ignoreCase: options?.ignoreCase,
      normalizeWhiteSpace: true
    }), options),
    toHaveAccessibleName: (expected, options) => text("toHaveAccessibleName", "Locator", "to.have.accessible.name", expected, textValues([expected], {
      ignoreCase: options?.ignoreCase,
      normalizeWhiteSpace: true
    }), options),
    toHaveAccessibleErrorMessage: (expected, options) => text("toHaveAccessibleErrorMessage", "Locator", "to.have.accessible.error.message", expected, textValues([expected], {
      ignoreCase: options?.ignoreCase,
      normalizeWhiteSpace: true
    }), options),
    toHaveAttribute: (name, expected, options) => {
      if (!options && typeof expected === "object" && expected !== null && !(expected instanceof RegExp))
        return truthy("to.have.attribute", "have attribute", "", expected, {
          expressionArg: name
        });
      if (expected === undefined)
        return truthy("to.have.attribute", "have attribute", "", options, {
          expressionArg: name
        });
      const value = expected;
      return text("toHaveAttribute", "Locator", "to.have.attribute.value", value, textValues([value], { ignoreCase: options?.ignoreCase }), options, { expressionArg: name });
    },
    toHaveClass: (expected, options) => Array.isArray(expected) ? equal("to.have.class.array", expected, options, {
      expectedText: textValues(expected)
    }) : text("toHaveClass", "Locator", "to.have.class", expected, textValues([expected]), options),
    toContainClass: (expected, options) => {
      if (Array.isArray(expected)) {
        if (expected.some((item) => item instanceof RegExp))
          throw argumentError("toContainClass", '"expected" argument in toContainClass cannot contain RegExp values');
        return equal("to.contain.class.array", expected, options, {
          expectedText: textValues(expected)
        });
      }
      if (expected instanceof RegExp)
        throw argumentError("toContainClass", '"expected" argument in toContainClass cannot be a RegExp value');
      return text("toContainClass", "Locator", "to.contain.class", expected, textValues([expected]), options);
    },
    toHaveCount: (expected, options) => equal("to.have.count", expected, options, { expectedNumber: expected }),
    toHaveCSS: (name, expected, options) => text("toHaveCSS", "Locator", "to.have.css", expected, textValues([expected]), options, {
      expressionArg: name,
      ...options?.pseudo ? { pseudo: options.pseudo } : {}
    }),
    toHaveId: (expected, options) => text("toHaveId", "Locator", "to.have.id", expected, textValues([expected]), options),
    toHaveJSProperty: (name, expected, options) => equal("to.have.property", expected, options, {
      expressionArg: name,
      expectedValue: expected
    }),
    toHaveRole: (expected, options) => {
      if (typeof expected !== "string")
        throw argumentError("toHaveRole", '"role" argument in toHaveRole must be a string');
      return text("toHaveRole", "Locator", "to.have.role", expected, textValues([expected]), options);
    },
    toHaveText: (expected, options) => Array.isArray(expected) ? equal("to.have.text.array", expected, options, {
      expectedText: textValues(expected, {
        normalizeWhiteSpace: true,
        ignoreCase: options?.ignoreCase
      })
    }) : text("toHaveText", "Locator", "to.have.text", expected, textValues([expected], {
      normalizeWhiteSpace: true,
      ignoreCase: options?.ignoreCase
    }), options),
    toHaveValue: (expected, options) => text("toHaveValue", "Locator", "to.have.value", expected, textValues([expected]), options),
    toHaveValues: (expected, options) => equal("to.have.values", expected, options, {
      expectedText: textValues(expected)
    }),
    toHaveTitle: (expected, options) => text("toHaveTitle", "Page", "to.have.title", expected, textValues([expected], { normalizeWhiteSpace: true }), options),
    toHaveURL: (expected, options) => {
      const value = typeof expected === "string" ? normalizeUrl(expected) : expected;
      return text("toHaveURL", "Page", "to.have.url", value, textValues([value], { ignoreCase: options?.ignoreCase }), options);
    }
  };
  async function retry(name, actual, state, matcher) {
    const api = `expect.${name}`;
    const isPage = actual instanceof Page;
    const isLocator = actual instanceof Locator;
    if (matcher.receiver === "Page" ? !isPage : !isLocator)
      throw argumentError(name, `${name} can be only used with ${matcher.receiver} object, was called with ${receiverText(actual)}`);
    const call = {
      api,
      timeoutMs: timeoutMs(api, matcher.options?.timeout ?? state.config.timeout)
    };
    const shape = {
      matcher: name,
      expectation: matcher.expectation,
      expected: matcher.expected(state.isNot),
      params: { ...matcher.params, isNot: state.isNot }
    };
    try {
      if (actual instanceof Page)
        await invoke("expect.page", call, {
          page: await actual._brand(api),
          ...shape
        });
      else if (actual instanceof Locator)
        await invoke("expect.locator", call, {
          target: await actual._frame._locatorBrand(api, actual._selector),
          ...shape
        });
    } catch (error) {
      if (state.message && error instanceof Error)
        error.message = `${state.message}

${error.message}`;
      throw error;
    }
  }
  async function urlPredicate(page, predicate, state, options) {
    const timeout = options?.timeout ?? state.config.timeout;
    const deadline = Date.now() + timeout;
    for (;; ) {
      let passed;
      try {
        passed = Boolean(await predicate(URL.parse(page.url()) ?? page.url()));
      } catch (error) {
        throw prefixed("expect.toHaveURL", error);
      }
      if (passed !== state.isNot)
        return;
      const remaining = deadline - Date.now();
      if (timeout !== 0 && remaining <= 0)
        fail(state, `${header("toHaveURL", state, "page")} failed

Expected: predicate to ${state.isNot ? "fail" : "succeed"}
Received: ${stringify(page.url())}
Timeout: ${timeout}ms`);
      try {
        await page._waitForURL("expect.toHaveURL", page._mainFrame, (url) => {
          const href = typeof url === "object" && url !== null ? Reflect.get(url, "href") : url;
          return href !== page.url();
        }, { timeout: timeout === 0 ? 0 : remaining, waitUntil: "commit" });
      } catch (error) {
        if (Reflect.get(Object(error), "timeout") !== true && !(error instanceof Error && error.name === "TimeoutError"))
          throw error;
      }
    }
  }

  // crates/neo-sdk/neo-playwright-js/src/prelude/unsupported.ts
  var NETWORK = "network interception is not available in Neo";
  var NO_HANDLES = "Neo returns values as JSON, without handles; use evaluate()";
  var NO_INSPECTOR = "there is no inspector in Neo";
  var NO_TOUCH = "Neo has no touch input; use click()";
  var PROFILE = "cookies and storage stay with the owner's signed-in profile";
  var OWNED = "Neo owns the browser and its profile; close a tab with page.close()";
  var TRACE = "Neo records its own trace of every run";
  var ROOTED = "element handles cannot root a selector in Neo; use locator.locator(selector) instead";
  var UNSUPPORTED2 = {
    Browser: {
      bind: "Neo does not share its browser",
      browserType: {
        hint: "Neo drives its own BrowserOS; there is no browser type to launch",
        kind: "sync"
      },
      close: OWNED,
      newBrowserCDPSession: "raw CDP sessions are not available in Neo",
      newContext: "Neo has one browser profile; open a tab with context.newPage()",
      newPage: "Neo has one browser profile; open a tab with context.newPage()",
      startTracing: TRACE,
      stopTracing: TRACE,
      unbind: "Neo does not share its browser"
    },
    BrowserContext: {
      addCookies: PROFILE,
      clearCookies: PROFILE,
      clearPermissions: "permissions are the owner's profile settings",
      close: OWNED,
      cookies: PROFILE,
      grantPermissions: "permissions are the owner's profile settings",
      newCDPSession: "raw CDP sessions are not available in Neo",
      route: NETWORK,
      routeFromHAR: NETWORK,
      routeWebSocket: NETWORK,
      setExtraHTTPHeaders: "network settings are not available in Neo",
      setGeolocation: "geolocation is the owner's profile setting",
      setHTTPCredentials: "network settings are not available in Neo",
      setOffline: "network settings are not available in Neo",
      setStorageState: PROFILE,
      storageState: PROFILE,
      unroute: NETWORK,
      unrouteAll: NETWORK,
      clock: {
        hint: "clock emulation is not available in Neo",
        kind: "property"
      },
      credentials: {
        hint: "credentials stay with the owner's profile",
        kind: "property"
      },
      debugger: {
        hint: "the debugger is not available in Neo",
        kind: "property"
      },
      request: {
        hint: "API requests are not available in Neo; fetch from the page with page.evaluate()",
        kind: "property"
      },
      tracing: { hint: TRACE, kind: "property" }
    },
    Page: {
      addLocatorHandler: "locator handlers are not available in Neo; check for the overlay with locator.isVisible() before acting",
      bringToFront: "Neo keeps agent tabs in the background; act on the page directly",
      cancelPickLocator: NO_INSPECTOR,
      clearConsoleMessages: "Neo keeps no console history; hosted BrowserOS emits no console events",
      clearPageErrors: "Neo keeps no error history; hosted BrowserOS emits no pageerror events",
      consoleMessages: "Neo keeps no console history; hosted BrowserOS emits no console events",
      evaluateHandle: NO_HANDLES,
      hideHighlight: NO_INSPECTOR,
      pageErrors: "Neo keeps no error history; hosted BrowserOS emits no pageerror events",
      pause: NO_INSPECTOR,
      pickLocator: NO_INSPECTOR,
      removeLocatorHandler: "locator handlers are not available in Neo",
      requestGC: "garbage collection is not available in Neo",
      requests: "Neo does not record network requests",
      route: NETWORK,
      routeFromHAR: NETWORK,
      routeWebSocket: NETWORK,
      setExtraHTTPHeaders: "network settings are not available in Neo",
      tap: NO_TOUCH,
      unroute: NETWORK,
      unrouteAll: NETWORK,
      waitForRequest: "Neo does not report network events; wait for what the page shows instead",
      waitForResponse: "Neo does not report network events; wait for what the page shows instead",
      clock: {
        hint: "clock emulation is not available in Neo",
        kind: "property"
      },
      coverage: { hint: "coverage is not available in Neo", kind: "property" },
      localStorage: {
        hint: "read storage with page.evaluate(() => localStorage.getItem(key))",
        kind: "property"
      },
      request: {
        hint: "API requests are not available in Neo; fetch from the page with page.evaluate()",
        kind: "property"
      },
      screencast: {
        hint: "Neo records its own screencast of every tab",
        kind: "property"
      },
      sessionStorage: {
        hint: "read storage with page.evaluate(() => sessionStorage.getItem(key))",
        kind: "property"
      },
      touchscreen: {
        hint: "Neo has no touch input; use page.mouse",
        kind: "property"
      },
      webmcp: { hint: "WebMCP is not available in Neo", kind: "property" }
    },
    Frame: {
      evaluateHandle: NO_HANDLES,
      frameElement: "locate the iframe from its parent instead: page.locator('iframe')",
      tap: NO_TOUCH,
      webmcp: { hint: "WebMCP is not available in Neo", kind: "property" }
    },
    Locator: {
      evaluateHandle: NO_HANDLES,
      hideHighlight: NO_INSPECTOR,
      highlight: NO_INSPECTOR,
      tap: NO_TOUCH
    },
    FrameLocator: {},
    Keyboard: {},
    Mouse: {},
    ElementHandle: {
      contentFrame: "use page.frameLocator() or locator.contentFrame() to enter a frame",
      evaluateHandle: NO_HANDLES,
      getProperties: "Neo returns values as JSON; read them with elementHandle.evaluate(el => el.prop)",
      getProperty: "Neo returns values as JSON; read them with elementHandle.evaluate(el => el.prop)",
      jsonValue: "Neo returns values as JSON; read them with elementHandle.evaluate(el => el.prop)",
      ownerFrame: "use page.frames() to find a frame",
      tap: NO_TOUCH,
      waitForSelector: ROOTED
    },
    Download: {
      cancel: "Neo keeps the run's downloads; it cannot cancel one",
      createReadStream: "Neo gives a script the saved file, not its bytes: await download.path() and upload it with setInputFiles; the host reads the bytes from the run",
      delete: "Neo keeps the run's downloads; it cannot delete one"
    },
    ConsoleMessage: {
      args: {
        hint: "Neo passes console text only; use message.text()",
        kind: "sync"
      }
    },
    Response: {
      body: "Neo does not read response bodies; read the page with page.content() or page.evaluate()",
      json: "Neo does not read response bodies; read the page with page.content() or page.evaluate()",
      request: { hint: "Neo does not report requests", kind: "sync" },
      securityDetails: "Neo does not report TLS details",
      serverAddr: "Neo does not report server addresses",
      text: "Neo does not read response bodies; read the page with page.content() or page.evaluate()"
    },
    expect: {
      extend: { hint: "custom matchers are not available in Neo", kind: "sync" },
      getState: {
        hint: "expect's test state is not available in Neo",
        kind: "sync"
      },
      soft: {
        hint: "soft assertions are not available in Neo; catch the error to continue",
        kind: "sync"
      },
      toBeOK: "use expect(response.ok()).toBe(true)",
      toHaveScreenshot: "visual comparisons are not available in Neo",
      toMatchAriaSnapshot: "compare the text of await locator.ariaSnapshot() instead, such as with toContain()",
      toMatchSnapshot: "snapshot files are not available in Neo"
    }
  };
  function hintOf(entry) {
    return typeof entry === "string" ? entry : entry.hint;
  }
  function apiName(className, member) {
    return `${className.charAt(0).toLowerCase()}${className.slice(1)}.${member}`;
  }
  function define(target, className, member, entry) {
    const api = apiName(className, member);
    const error = () => unsupportedError(api, hintOf(entry));
    if (typeof entry !== "string" && entry.kind === "property") {
      Object.defineProperty(target, member, {
        configurable: true,
        get: () => {
          throw error();
        }
      });
      return;
    }
    const method = typeof entry !== "string" && entry.kind === "sync" ? () => {
      throw error();
    } : () => Promise.reject(error());
    Object.defineProperty(target, member, {
      configurable: true,
      writable: true,
      value: method
    });
  }
  function installUnsupported(classes) {
    for (const [className, members] of Object.entries(UNSUPPORTED2)) {
      if (className === "expect")
        continue;
      const prototype = classes[className].prototype;
      for (const [member, entry] of Object.entries(members))
        define(prototype, className, member, entry);
    }
  }
  function installUnsupportedExpect(target, part) {
    for (const [member, entry] of Object.entries(UNSUPPORTED2.expect))
      if (member.startsWith("to") === (part === "matchers"))
        define(target, "expect", member, entry);
  }

  // crates/neo-sdk/neo-playwright-js/src/prelude/expect.ts
  function retrying(name, actual, state, args) {
    if (name === "toHaveURL" && typeof args[0] === "function") {
      if (!(actual instanceof Page))
        throw plainError("expect.toHaveURL", `toHaveURL can be only used with Page object, was called with ${receiverText(actual)}`);
      return urlPredicate(actual, args[0], state, args[1]);
    }
    const build = RETRYING[name];
    return retry(name, actual, state, Reflect.apply(build, undefined, args));
  }
  function matchers(actual, state) {
    const built = {};
    for (const name of Object.keys(valueMatchers(actual, state)))
      built[name] = (...args) => {
        const run = (value) => Reflect.apply(valueMatchers(value, state)[name], undefined, args);
        return state.promise ? settle(actual, state, run, name) : run(actual);
      };
    for (const name of Object.keys(RETRYING))
      built[name] = (...args) => state.promise ? settle(actual, state, (value) => retrying(name, value, state, args), name) : retrying(name, actual, state, args);
    built.toPass = (options) => toPass(actual, state, options);
    installUnsupportedExpect(built, "matchers");
    Object.defineProperty(built, "not", {
      get: () => matchers(actual, { ...state, isNot: !state.isNot })
    });
    Object.defineProperty(built, "resolves", {
      get: () => matchers(actual, { ...state, promise: "resolves" })
    });
    Object.defineProperty(built, "rejects", {
      get: () => matchers(actual, { ...state, promise: "rejects" })
    });
    return built;
  }
  async function settle(actual, state, match, name) {
    const subject = typeof actual === "function" ? Reflect.apply(actual, undefined, []) : actual;
    if (!(subject instanceof Promise) && typeof Reflect.get(Object(subject), "then") !== "function")
      fail(state, `${header(name, state)}

received value must be a promise or a function returning a promise
Received: ${stringify(actual)}`);
    let value;
    try {
      value = await subject;
    } catch (reason) {
      if (state.promise === "resolves")
        fail(state, `${header(name, state)}

Received promise rejected instead of resolved
Rejected to value: ${stringify(reason)}`);
      await match(reason);
      return;
    }
    if (state.promise === "rejects")
      fail(state, `${header(name, state)}

Received promise resolved instead of rejected
Resolved to value: ${stringify(value)}`);
    await match(value);
  }
  var INTERVALS = [100, 250, 500, 1000];
  async function toPass(callback, state, options = {}) {
    if (typeof callback !== "function")
      throw plainError("expect.toPass", "toPass: expected a function");
    const timeout = options.timeout ?? 0;
    const intervals = options.intervals ?? INTERVALS;
    const deadline = timeout ? Date.now() + timeout : Number.POSITIVE_INFINITY;
    for (let attempt = 0;; attempt++) {
      let error;
      try {
        await Reflect.apply(callback, undefined, []);
      } catch (caught) {
        error = caught;
      }
      const passed = error === undefined;
      if (passed !== state.isNot)
        return;
      const wait = intervals[Math.min(attempt, intervals.length - 1)] ?? 1000;
      if (Date.now() + wait > deadline) {
        const last = error instanceof Error ? `${error.message}

` : "";
        fail(state, `${last}Call Log:
- Timeout ${timeout}ms exceeded while waiting on the predicate`);
      }
      await sleep("expect.toPass", wait);
    }
  }
  function poll(generator, state, options = {}) {
    if (typeof generator !== "function")
      throw plainError("expect.poll", "expect.poll: expected a function");
    const timeout = options.timeout ?? state.config.timeout;
    const intervals = options.intervals ?? INTERVALS;
    const polled = (isNot) => {
      const result = {};
      for (const name of Object.keys(valueMatchers(undefined, state)))
        result[name] = (...args) => (async () => {
          const deadline = Date.now() + timeout;
          for (let attempt = 0;; attempt++) {
            const value = await Reflect.apply(generator, undefined, []);
            try {
              Reflect.apply(valueMatchers(value, { ...state, isNot })[name], undefined, args);
              return;
            } catch (error) {
              const wait = intervals[Math.min(attempt, intervals.length - 1)] ?? 1000;
              if (timeout !== 0 && Date.now() + wait > deadline) {
                const message = error instanceof Error ? error.message : String(error);
                throw assertionError(`${message}

Call Log:
- Timeout ${timeout}ms exceeded while waiting on the predicate`);
              }
              await sleep("expect.poll", wait);
            }
          }
        })();
      const built = result;
      Object.defineProperty(built, "not", { get: () => polled(!isNot) });
      return built;
    };
    return polled(state.isNot);
  }
  function createExpect(config = { timeout: DEFAULT_TIMEOUT }) {
    const expect = (actual, messageOrOptions) => {
      const message = typeof messageOrOptions === "string" ? messageOrOptions : messageOrOptions?.message;
      return matchers(actual, { isNot: false, promise: "", config, message });
    };
    expect.any = (expected) => new Any(expected);
    expect.anything = () => new Anything(false);
    expect.arrayContaining = (expected) => new ArrayContaining(expected, false);
    expect.arrayOf = (expected) => new ArrayOf(expected, false);
    expect.objectContaining = (expected) => new ObjectContaining(expected, false);
    expect.stringContaining = (expected) => new StringContaining(expected, false);
    expect.stringMatching = (expected) => new StringMatching(expected, false);
    expect.closeTo = (expected, precision = 2) => new CloseTo(expected, precision, false);
    expect.not = {
      arrayContaining: (expected) => new ArrayContaining(expected, true),
      arrayOf: (expected) => new ArrayOf(expected, true),
      objectContaining: (expected) => new ObjectContaining(expected, true),
      stringContaining: (expected) => new StringContaining(expected, true),
      stringMatching: (expected) => new StringMatching(expected, true),
      closeTo: (expected, precision = 2) => new CloseTo(expected, precision, true)
    };
    expect.configure = (options = {}) => {
      if (options.soft)
        throw unsupportedError("expect.configure", "soft assertions are not available in Neo; catch the error to continue");
      return createExpect({ timeout: options.timeout ?? config.timeout });
    };
    expect.poll = (generator, options) => poll(generator, { isNot: false, promise: "", config, message: options?.message }, options);
    installUnsupportedExpect(expect, "statics");
    return expect;
  }

  // crates/neo-sdk/neo-playwright-js/src/prelude/index.ts
  installUnsupported({
    Browser,
    BrowserContext,
    Page,
    Frame,
    Locator,
    FrameLocator,
    Keyboard,
    Mouse,
    ElementHandle,
    Download,
    ConsoleMessage,
    Response
  });
  var context = new BrowserContext;
  var browser = new Browser(context);
  var expect = createExpect();
  registerBrand("page", (json) => context._pageFromResult(json));
  registerBrand("element", (json) => {
    const brand = elementBrand(json);
    return brand ? new ElementHandle(context._pageFor(brand.page), brand) : json;
  });
  registerBrand("bytes", (json) => Buffer.from(String(Reflect.get(json, "base64") ?? ""), "base64"));
  onReply((reply) => context._applyReply(reply));
  provide("page", awaitsProviders() ? async (init) => {
    const page = context._runPage(init);
    await page._readInitialFrames();
    return page;
  } : (init) => context._runPage(init));
  provide("context", () => context);
  provide("browser", () => browser);
  provide("expect", () => expect);
  provide("Buffer", () => Buffer);
})();
