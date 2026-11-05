/*
 * Playwright v1.64.1 (cfe490d941f992655948eee8fd4fd94e1c4234bf), packages/isomorphic helpers, for Neo's QuickJS prelude.
 * Playwright: Copyright (c) Microsoft Corporation. Licensed under the Apache License, Version 2.0.
 * Built unmodified from crates/browseros-playwright/third_party/playwright, with
 * packages/browseros-playwright-js/src/prelude, by packages/browseros-playwright-js/scripts/build.ts.
 * Do not edit. Notices: crates/browseros-playwright/THIRD_PARTY_NOTICES.md.
 */
(() => {
  // packages/browseros-playwright-js/src/prelude/errors.ts
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
    const named = timedOut && !assertion ? "TimeoutError" : "Error";
    if (thrown instanceof Error) {
      if (!assertion)
        thrown.message = apiMessage(api, op, thrown.message);
      thrown.name = named;
      return withFields(thrown, { timeout: timedOut });
    }
    const message = typeof thrown === "object" && thrown !== null ? String(Reflect.get(thrown, "message") ?? thrown) : String(thrown);
    const shaped = assertion ? message : apiMessage(api, op, message);
    return withFields(named === "TimeoutError" ? new TimeoutError(shaped) : new PlaywrightError(shaped), { ...read, timeout: timedOut });
  }
  function clientError(api, message) {
    return withFields(new PlaywrightError(`${api}: ${message}`), {
      code: "invalid_input",
      owner: "script",
      delivery: "not_sent",
      retryable: false,
      timeout: false
    });
  }
  function waitTimeoutError(api, message) {
    return withFields(new TimeoutError(`${api}: ${message}`), {
      code: "wait_timeout",
      owner: "website",
      delivery: "not_sent",
      retryable: true,
      timeout: true
    });
  }
  function unsupportedError(api, hint) {
    return withFields(new PlaywrightError(`${api}: ${hint}`), {
      code: "unsupported",
      owner: "neo",
      delivery: "not_sent",
      retryable: false,
      timeout: false
    });
  }
  function assertionError(message) {
    return new PlaywrightError(message);
  }
  function prefixed(api, error) {
    if (error instanceof Error && !error.message.startsWith(`${api}: `))
      error.message = `${api}: ${error.message}`;
    return error;
  }

  // packages/browseros-playwright-js/src/prelude/bridge.ts
  function host() {
    const neo = Reflect.get(globalThis, "__neo");
    if (typeof neo !== "object" || neo === null)
      throw new Error("The Playwright prelude needs claw-sdk's bootstrap (globalThis.__neo), which was not installed");
    return neo;
  }
  async function invoke(op, call, args) {
    try {
      return await host().invoke(op, { call, ...args });
    } catch (thrown) {
      throw shapeBridgeError(op, call.api, thrown);
    }
  }
  function registerBrand(brand, fromJson) {
    host().brands.register(brand, fromJson);
  }
  function onReply(listener) {
    host().onReply((envelope) => listener({
      pages: readPageFacts(field(envelope, "pages")),
      defaultPage: readTab(field(envelope, "defaultPage"))
    }));
  }
  function provide(name, build) {
    host().provide(name, (init) => {
      const page = readTab(field(init, "page"));
      return build({
        page: page ? { $neo: "page", ...page } : null,
        pages: readPageFacts(field(init, "pages"))
      });
    });
  }
  function field(value, name) {
    return typeof value === "object" && value !== null ? Reflect.get(value, name) : undefined;
  }
  function readTab(value) {
    const id = field(value, "id");
    const tab = field(value, "tab");
    return typeof id === "number" && typeof tab === "number" ? { id, tab } : null;
  }
  function readPageFacts(pages) {
    if (!Array.isArray(pages))
      return [];
    const facts = [];
    for (const page of pages) {
      if (typeof page !== "object" || page === null)
        continue;
      const id = Reflect.get(page, "id");
      if (typeof id !== "number")
        continue;
      const fact = { id };
      const tab = Reflect.get(page, "tab");
      if (typeof tab === "number")
        fact.tab = tab;
      const url = Reflect.get(page, "url");
      const title = Reflect.get(page, "title");
      const closed = Reflect.get(page, "closed");
      if (typeof url === "string")
        fact.url = url;
      if (typeof title === "string")
        fact.title = title;
      if (typeof closed === "boolean")
        fact.closed = closed;
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

  // packages/browseros-playwright-js/src/prelude/emitter.ts
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

  // packages/browseros-playwright-js/src/prelude/options.ts
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
  var SCREENSHOT_KEYS = ["type", "quality", "fullPage", "clip"];
  var handles = new WeakSet;
  function markHandle(object) {
    handles.add(object);
  }
  function findHandle(value, seen) {
    if (typeof value !== "object" || value === null || seen.has(value))
      return false;
    if (handles.has(value))
      return true;
    seen.add(value);
    for (const item of Object.values(value))
      if (findHandle(item, seen))
        return true;
    return false;
  }
  function evaluation(api, pageFunction, arg) {
    if (typeof pageFunction !== "function" && typeof pageFunction !== "string")
      throw clientError(api, "expected a function or an expression string");
    if (findHandle(arg, new Set))
      throw unsupportedError(api, "Neo passes evaluate arguments as JSON, without handles; run the function on the element itself with locator.evaluate(fn) or elementHandle.evaluate(fn)");
    const call = {
      expression: String(pageFunction),
      isFunction: typeof pageFunction === "function"
    };
    if (arg !== undefined)
      call.arg = arg;
    return call;
  }
  function inputFilePaths(api, files) {
    const list = Array.isArray(files) ? files : [files];
    if (list.every((file) => typeof file === "string"))
      return list;
    throw unsupportedError(api, "Neo sets files from paths on the host; pass file paths, not { name, mimeType, buffer } payloads");
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
      const option = item;
      const given = ["valueOrLabel", "value", "label", "index"].filter((key) => option[key] !== undefined);
      if (given.length !== 1)
        throw clientError(api, `options[${index}]: Neo matches an option by one of value, label or index; got ${JSON.stringify(item)}`);
      const [key] = given;
      if (key === "index")
        options.push({ index: Number(option.index) });
      else if (key === "value")
        options.push({ value: String(option.value) });
      else if (key === "label")
        options.push({ label: String(option.label) });
      else
        options.push({ valueOrLabel: String(option.valueOrLabel) });
    }
    return { options, elements };
  }

  // packages/browseros-playwright-js/src/prelude/browser.ts
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

  // packages/browseros-playwright-js/src/prelude/event-deadline.ts
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
      return `Timeout ${this.timeout}ms exceeded while waiting for event "${this.event}"`;
    }
  }
  function isThenable(value) {
    return (typeof value === "object" || typeof value === "function") && value !== null && typeof Reflect.get(value, "then") === "function";
  }

  // packages/browseros-playwright-js/src/prelude/event-values.ts
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
      try {
        await invoke("dialog.dismiss", { api, timeoutMs: 0 }, { page: await this.#pageValue._brand(api), dialog: this.#info.id });
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
  }
  function pageError(info) {
    const error = new Error(info.message);
    error.name = info.name;
    error.stack = info.stack;
    return error;
  }

  // packages/browseros-playwright-js/src/prelude/timeouts.ts
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

  // packages/browseros-playwright-js/src/prelude/events.ts
  var PAGE_EVENTS = new Set([
    "popup",
    "close",
    "dialog",
    "console",
    "pageerror",
    "download"
  ]);
  var CONTEXT_EVENTS = new Set(["page"]);
  var UNREPORTED = {
    request: "Neo does not report network events; wait for what the page shows instead",
    response: "Neo does not report network events; wait for what the page shows instead",
    requestfailed: "Neo does not report network events; wait for what the page shows instead",
    requestfinished: "Neo does not report network events; wait for what the page shows instead",
    websocket: "Neo does not report WebSockets",
    worker: "Neo does not report workers",
    serviceworker: "Neo does not report service workers",
    backgroundpage: "Neo does not report extension background pages",
    filechooser: "Neo does not report file choosers; call locator.setInputFiles() on the input",
    frameattached: "Neo does not report frame events; read page.frames() after a navigation",
    framedetached: "Neo does not report frame events; read page.frames() after a navigation",
    framenavigated: "Neo does not report frame events; use page.waitForURL()",
    load: "Neo does not report load events; use page.waitForLoadState()",
    domcontentloaded: 'Neo does not report load events; use page.waitForLoadState("domcontentloaded")',
    crash: "Neo does not report crashes; a crashed page fails its next call",
    pageload: "Neo does not report load events; use page.waitForLoadState()",
    pageclose: 'listen for close on the page itself: page.on("close")',
    weberror: 'listen for pageerror on the page itself: page.on("pageerror")',
    dialogclosed: "Neo does not report closed dialogs"
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
    dialogs = new Map;
    downloads = new Map;
    constructor(owner) {
      this.owner = owner;
    }
    listen(page, event, on) {
      const since = Date.now();
      this.listening += on ? 1 : -1;
      this.sendListen(page, event, on, since);
      if (on && !this.pumping) {
        this.pumping = true;
        this.pump();
      }
    }
    async wait(api, page, event, optionsOrPredicate, since, settings) {
      const kind = page === null ? "context" : "page";
      checkReported(api, event, kind);
      if (!isReported(event, kind))
        throw unsupportedError(api, `Neo reports only Playwright's events, not "${event}"`);
      const options = typeof optionsOrPredicate === "function" ? { predicate: optionsOrPredicate } : optionsOrPredicate ?? {};
      const deadline = new EventDeadline(api, event, since, timeoutMs(api, settings.timeout(options)));
      const target = page === null ? null : await page._openedBrand(api);
      let after = null;
      for (;; ) {
        const left = deadline.left();
        let record;
        try {
          record = await invoke("events.wait", { api, timeoutMs: left }, { page: target, event, since, after });
        } catch (error) {
          throw deadline.named(error);
        }
        const value = this.value(record);
        if (!options.predicate)
          return value;
        let accepted;
        try {
          accepted = await deadline.race(Reflect.apply(options.predicate, undefined, [value]));
        } catch (error) {
          throw prefixed(api, error);
        }
        if (accepted)
          return value;
        after = record.seq;
      }
    }
    async sendListen(page, event, on, since) {
      const api = `${page === null ? "browserContext" : "page"}.${on ? "on" : "off"}`;
      try {
        await invoke("events.listen", { api, timeoutMs: 0 }, {
          page: page === null ? null : await page._openedBrand(api),
          event,
          on,
          since
        });
      } catch {}
    }
    async pump() {
      try {
        while (this.listening > 0) {
          const { events } = await invoke("events.next", { api: "events.next", timeoutMs: 0 }, {});
          for (const event of events)
            this.deliver(event);
        }
      } catch {} finally {
        this.pumping = false;
      }
    }
    page(value, url) {
      const page = this.owner._pageFromResult(value, url);
      if (!page)
        throw new Error(`Rust named no page in an event: ${String(value)}`);
      return page;
    }
    deliver(record) {
      switch (record.event) {
        case "popup":
          this.page(record.page).emit("popup", this.page(record.popup.page, record.popup.url));
          return;
        case "page":
          this.owner.emit("page", this.page(record.page, record.url));
          return;
        case "close":
          this.page(record.page)._didClose();
          return;
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
      }
    }
    value(record) {
      switch (record.event) {
        case "popup":
          return this.page(record.popup.page, record.popup.url);
        case "page":
          return this.page(record.page, record.url);
        case "close": {
          const page = this.page(record.page);
          page._didClose();
          return page;
        }
        case "dialog":
          return this.dialog(this.page(record.page), record.dialog);
        case "console":
          return new ConsoleMessage(this.page(record.page), record.message);
        case "pageerror":
          return pageError(record.error);
        case "download":
          return this.download(this.page(record.page), record.download);
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
  }

  // crates/browseros-playwright/third_party/playwright/packages/isomorphic/stringUtils.ts
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

  // crates/browseros-playwright/third_party/playwright/packages/isomorphic/yaml.ts
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

  // crates/browseros-playwright/third_party/playwright/packages/isomorphic/ariaSnapshotRenderer.ts
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
  // crates/browseros-playwright/third_party/playwright/packages/isomorphic/cssTokenizer.ts
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

  // crates/browseros-playwright/third_party/playwright/packages/isomorphic/cssParser.ts
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

  // crates/browseros-playwright/third_party/playwright/packages/isomorphic/selectorParser.ts
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

  // crates/browseros-playwright/third_party/playwright/packages/isomorphic/locatorGenerators.ts
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
          if (isRegExp(options.name))
            attrs.push(`name: ${this.regexToSourceString(options.name)}`);
          else if (typeof options.name === "string")
            attrs.push(`name: ${this.quote(options.name)}`);
          if (isRegExp(options.description))
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
      if (isRegExp(body))
        return `${method}(${this.regexToSourceString(body)})`;
      return exact ? `${method}(${this.quote(body)}, { exact: true })` : `${method}(${this.quote(body)})`;
    }
    toHasText(body) {
      if (isRegExp(body))
        return this.regexToSourceString(body);
      return this.quote(body);
    }
    toTestIdValue(value) {
      if (isRegExp(value))
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
          if (isRegExp(options.name))
            attrs.push(`name=${this.regexToString(options.name)}`);
          else if (typeof options.name === "string")
            attrs.push(`name=${this.quote(options.name)}`);
          if (isRegExp(options.description))
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
      if (isRegExp(body))
        return `${method}(${this.regexToString(body)})`;
      if (exact)
        return `${method}(${this.quote(body)}, exact=True)`;
      return `${method}(${this.quote(body)})`;
    }
    toHasText(body) {
      if (isRegExp(body))
        return this.regexToString(body);
      return `${this.quote(body)}`;
    }
    toTestIdValue(value) {
      if (isRegExp(value))
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
          if (isRegExp(options.name))
            attrs.push(`.setName(${this.regexToString(options.name)})`);
          else if (typeof options.name === "string")
            attrs.push(`.setName(${this.quote(options.name)})`);
          if (isRegExp(options.description))
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
      if (isRegExp(body))
        return `${method}(${this.regexToString(body)})`;
      if (exact)
        return `${method}(${this.quote(body)}, new ${clazz}.${toTitleCase(method)}Options().setExact(true))`;
      return `${method}(${this.quote(body)})`;
    }
    toHasText(body) {
      if (isRegExp(body))
        return this.regexToString(body);
      return this.quote(body);
    }
    toTestIdValue(value) {
      if (isRegExp(value))
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
          if (isRegExp(options.name))
            attrs.push(`NameRegex = ${this.regexToString(options.name)}`);
          else if (typeof options.name === "string")
            attrs.push(`Name = ${this.quote(options.name)}`);
          if (isRegExp(options.description))
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
      if (isRegExp(body))
        return `${method}(${this.regexToString(body)})`;
      if (exact)
        return `${method}(${this.quote(body)}, new() { Exact = true })`;
      return `${method}(${this.quote(body)})`;
    }
    toHasText(body) {
      if (isRegExp(body))
        return `HasTextRegex = ${this.regexToString(body)}`;
      return `HasText = ${this.quote(body)}`;
    }
    toTestIdValue(value) {
      if (isRegExp(value))
        return this.regexToString(value);
      return this.quote(value);
    }
    toHasNotText(body) {
      if (isRegExp(body))
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
  function isRegExp(obj) {
    return obj instanceof RegExp;
  }
  // crates/browseros-playwright/third_party/playwright/packages/isomorphic/locatorUtils.ts
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
  // crates/browseros-playwright/third_party/playwright/packages/isomorphic/urlMatch.ts
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
  // packages/browseros-playwright-js/src/prelude/selectors.ts
  function withLocatorOptions(frame, selector, options) {
    let result = selector;
    if (options?.hasText)
      result += ` >> internal:has-text=${escapeForTextSelector(options.hasText, false)}`;
    if (options?.hasNotText)
      result += ` >> internal:has-not-text=${escapeForTextSelector(options.hasNotText, false)}`;
    if (options?.has) {
      if (options.has._frame !== frame)
        throw new Error('Inner "has" locator must belong to the same frame.');
      result += ` >> internal:has=${JSON.stringify(options.has._selector)}`;
    }
    if (options?.hasNot) {
      if (options.hasNot._frame !== frame)
        throw new Error('Inner "hasNot" locator must belong to the same frame.');
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

  // packages/browseros-playwright-js/src/prelude/locator.ts
  function testIdAttributeName() {
    return "data-testid";
  }

  class Locator {
    _frame;
    _selector;
    constructor(frame, selector, options) {
      this._frame = frame;
      this._selector = withLocatorOptions(frame, selector, options);
      markHandle(this);
    }
    page() {
      return this._frame.page();
    }
    toString() {
      return asLocatorDescription("javascript", this._selector);
    }
    description() {
      return locatorCustomDescription(this._selector) ?? null;
    }
    locator(selectorOrLocator, options) {
      if (typeof selectorOrLocator === "string")
        return new Locator(this._frame, `${this._selector} >> ${selectorOrLocator}`, options);
      if (selectorOrLocator._frame !== this._frame)
        throw new Error("Locators must belong to the same frame.");
      return new Locator(this._frame, `${this._selector} >> internal:chain=${JSON.stringify(selectorOrLocator._selector)}`, options);
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
      return new Locator(this._frame, this._selector, options);
    }
    visible() {
      return new Locator(this._frame, this._selector, { visible: true });
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
        throw new Error("Locators must belong to the same frame.");
      return new Locator(this._frame, `${this._selector} >> internal:and=${JSON.stringify(locator._selector)}`);
    }
    or(locator) {
      if (locator._frame !== this._frame)
        throw new Error("Locators must belong to the same frame.");
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
        throw clientError("locator.elementHandle", `Could not resolve ${this} to DOM Element`);
      return handle;
    }
    elementHandles() {
      return this._frame._elementHandles("locator.elementHandles", this._selector);
    }
    evaluate(pageFunction, arg, options = {}) {
      return this._frame._evaluateOn("locator.evaluate", this._selector, true, pageFunction, arg, false, options);
    }
    evaluateAll(pageFunction, arg) {
      return this._frame._evaluateAll("locator.evaluateAll", this._selector, pageFunction, arg);
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
      if (typeof selectorOrLocator === "string")
        return new Locator(this._frame, this.#childSelector(selectorOrLocator), options);
      if (selectorOrLocator._frame !== this._frame)
        throw new Error("Locators must belong to the same frame.");
      return new Locator(this._frame, this.#childSelector(selectorOrLocator._selector), options);
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
    #nthSelector(nth) {
      if (this._frameSelector === kAnyFrameSelector)
        throw new Error("Selecting the nth frame is not allowed on frameLocator().");
      return `${this._frameSelector} >> nth=${nth}`;
    }
    first() {
      return new FrameLocator(this._frame, this.#nthSelector("0"));
    }
    last() {
      return new FrameLocator(this._frame, this.#nthSelector("-1"));
    }
    nth(index) {
      return new FrameLocator(this._frame, this.#nthSelector(String(index)));
    }
  }

  // packages/browseros-playwright-js/src/prelude/snapshot.ts
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

  // packages/browseros-playwright-js/src/prelude/url.ts
  var DEFAULT_PORTS = {
    "http:": "80",
    "https:": "443",
    "ws:": "80",
    "wss:": "443",
    "ftp:": "21"
  };
  var SPECIAL = new Set(["http:", "https:", "ws:", "wss:", "ftp:", "file:"]);
  var ABSOLUTE = /^([a-zA-Z][a-zA-Z0-9+.-]*:)(?:\/\/(?:([^@/?#]*)@)?(\[[^\]]*\]|[^:/?#]*)(?::(\d*))?)?([^?#]*)(\?[^#]*)?(#.*)?$/;
  function decode(text) {
    try {
      return decodeURIComponent(text.replace(/\+/g, " "));
    } catch {
      return text;
    }
  }
  function urlLike(href) {
    const match = ABSOLUTE.exec(href);
    if (!match)
      return null;
    const [
      ,
      scheme = "",
      userinfo = "",
      host = "",
      rawPort = "",
      path = "",
      query = "",
      fragment = ""
    ] = match;
    const protocol = scheme.toLowerCase();
    const special = SPECIAL.has(protocol);
    const hostname = special ? host.toLowerCase() : host;
    const port = rawPort === DEFAULT_PORTS[protocol] ? "" : rawPort;
    const pathname = path === "" && special ? "/" : path;
    const [username = "", password = ""] = userinfo.split(":");
    const authority = href.slice(protocol.length).startsWith("//");
    const hostPort = port ? `${hostname}:${port}` : hostname;
    const credentials = userinfo ? `${userinfo}@` : "";
    const normalized = authority ? `${protocol}//${credentials}${hostPort}${pathname}${query}${fragment}` : `${protocol}${pathname}${query}${fragment}`;
    const search = query === "?" ? "" : query;
    const pairs = search.slice(1).split("&").filter((pair) => pair !== "").map((pair) => {
      const equals = pair.indexOf("=");
      return equals === -1 ? [decode(pair), ""] : [decode(pair.slice(0, equals)), decode(pair.slice(equals + 1))];
    });
    return {
      href: normalized,
      protocol,
      username,
      password,
      host: hostPort,
      hostname,
      port,
      origin: special && protocol !== "file:" ? `${protocol}//${hostPort}` : "null",
      pathname,
      search,
      hash: fragment === "#" ? "" : fragment,
      searchParams: {
        get: (name) => pairs.find(([key]) => key === name)?.[1] ?? null,
        has: (name) => pairs.some(([key]) => key === name),
        getAll: (name) => pairs.filter(([key]) => key === name).map(([, value]) => value ?? "")
      },
      toString: () => normalized
    };
  }
  function normalizeUrl(text) {
    return urlLike(text)?.href ?? text;
  }
  function globMatches(glob, url) {
    if (glob === "")
      return true;
    return new RegExp(globToRegexPattern(glob)).test(url);
  }

  // packages/browseros-playwright-js/src/prelude/frame.ts
  function loadState(api, name, state) {
    if (state === "networkidle0")
      return "networkidle";
    if (state === "load" || state === "domcontentloaded" || state === "networkidle" || state === "commit")
      return state;
    throw clientError(api, `${name}: expected one of (load|domcontentloaded|networkidle|commit)`);
  }

  class Frame extends EventEmitter {
    _page;
    _id;
    _name = "";
    _url = "";
    _parentFrame;
    _detached = false;
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
      return this._name;
    }
    url() {
      return this._id === null ? this._page._url : this._url;
    }
    parentFrame() {
      return this._parentFrame;
    }
    childFrames() {
      return this._page.frames().filter((frame) => frame._parentFrame === this);
    }
    isDetached() {
      return this._detached;
    }
    toJSON() {
      return { name: this.name(), url: this.url() };
    }
    locator(selector, options) {
      return new Locator(this, selector, options);
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
    $eval(selector, pageFunction, arg) {
      return this._evaluateOn("frame.$eval", selector, false, pageFunction, arg, true);
    }
    $$eval(selector, pageFunction, arg) {
      return this._evaluateAll("frame.$$eval", selector, pageFunction, arg);
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
    setContent(html, options) {
      return this._setContent("frame.setContent", html, options);
    }
    evaluate(pageFunction, arg) {
      return this._evaluate("frame.evaluate", pageFunction, arg);
    }
    waitForFunction(pageFunction, arg, options) {
      return this._waitForFunction("frame.waitForFunction", pageFunction, arg, options);
    }
    waitForTimeout(timeout) {
      return sleep("frame.waitForTimeout", timeout);
    }
    _call(api, timeout) {
      return { api, timeoutMs: timeoutMs(api, timeout) };
    }
    _timeout(options) {
      return this._page._timeoutSettings.timeout(options);
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
    async _selectorOp(op, api, selector, strict, timeout, args) {
      const target = await this._locatorBrand(api, selector);
      const input = {
        target,
        strict: strict === true ? "strict" : "firstMatch",
        ...args
      };
      return await invoke(op, this._call(api, timeout), input);
    }
    async _click(api, selector, options = {}) {
      await this._selectorOp("locator.click", api, selector, options.strict, this._timeout(options), {
        options: pick(options, POINTER_KEYS)
      });
    }
    async _dblclick(api, selector, options = {}) {
      await this._selectorOp("locator.dblclick", api, selector, options.strict, this._timeout(options), {
        options: pick(options, DBLCLICK_KEYS)
      });
    }
    async _hover(api, selector, options = {}) {
      await this._selectorOp("locator.hover", api, selector, options.strict, this._timeout(options), {
        options: pick(options, HOVER_KEYS)
      });
    }
    async _check(api, selector, checked, options = {}) {
      const op = checked ? "locator.check" : "locator.uncheck";
      await this._selectorOp(op, api, selector, options.strict, this._timeout(options), {
        options: pick(options, CHECK_KEYS)
      });
    }
    async _dragAndDrop(api, source, target, options = {}) {
      const to = await this._locatorBrand(api, target);
      await this._selectorOp("locator.dragTo", api, source, options.strict, this._timeout(options), {
        to,
        options: pick(options, DRAG_KEYS)
      });
    }
    async _fill(api, selector, value, options = {}) {
      if (typeof value !== "string")
        throw clientError(api, `value: expected string, got ${typeof value}`);
      await this._selectorOp("locator.fill", api, selector, options.strict, this._timeout(options), {
        value,
        options: pick(options, FILL_KEYS)
      });
    }
    async _press(api, selector, key, options = {}) {
      await this._selectorOp("locator.press", api, selector, options.strict, this._timeout(options), {
        key,
        options: pick(options, KEY_KEYS)
      });
    }
    async _type(api, selector, text, options = {}) {
      await this._selectorOp("locator.pressSequentially", api, selector, options.strict, this._timeout(options), { text, options: pick(options, KEY_KEYS) });
    }
    async _focus(api, selector, options = {}) {
      await this._selectorOp("locator.focus", api, selector, options.strict, this._timeout(options), {});
    }
    async _blur(api, selector, options = {}) {
      await this._selectorOp("locator.blur", api, selector, options.strict, this._timeout(options), {});
    }
    async _selectOption(api, selector, values, options = {}) {
      return await this._selectorOp("locator.selectOption", api, selector, options.strict, this._timeout(options), { ...selectOptionValues(api, values), ...pick(options, ["force"]) });
    }
    async _selectText(api, selector, options = {}) {
      await this._selectorOp("locator.selectText", api, selector, options.strict, this._timeout(options), pick(options, ["force"]));
    }
    async _setInputFiles(api, selector, files, options = {}) {
      await this._selectorOp("locator.setInputFiles", api, selector, options.strict, this._timeout(options), { files: inputFilePaths(api, files) });
    }
    async _dispatchEvent(api, selector, type, eventInit, options = {}) {
      await this._selectorOp("locator.dispatchEvent", api, selector, options.strict, this._timeout(options), { type, eventInit: eventInit ?? {} });
    }
    async _scrollIntoViewIfNeeded(api, selector, options = {}) {
      await this._selectorOp("locator.scrollIntoViewIfNeeded", api, selector, options.strict, this._timeout(options), {});
    }
    async _read(op, api, selector, options = {}) {
      const args = {};
      return await this._selectorOp(op, api, selector, options.strict, this._timeout(options), args);
    }
    async _getAttribute(api, selector, name, options = {}) {
      return await this._selectorOp("locator.getAttribute", api, selector, options.strict, this._timeout(options), { name });
    }
    async _isVisible(op, api, selector, options = {}) {
      return await this._selectorOp(op, api, selector, options.strict, 0, {});
    }
    async _everyMatch(op, api, selector) {
      const target = await this._locatorBrand(api, selector);
      const input = { target };
      return await invoke(op, this._call(api, 0), input);
    }
    async _boundingBox(api, selector, options = {}) {
      return await this._selectorOp("locator.boundingBox", api, selector, true, this._timeout(options), {});
    }
    async _waitForSelector(api, selector, options = {}, omitReturnValue) {
      const raw = options;
      if (typeof raw === "object" && raw !== null && "visibility" in raw)
        throw clientError(api, "options.visibility is not supported, did you mean options.state?");
      const state = options.state ?? "visible";
      const handle = await this._selectorOp("locator.waitFor", api, selector, options.strict, this._timeout(options), { state, omitReturnValue });
      return this._page._handle(handle);
    }
    async _elementHandle(api, selector, strict, noWait, options = {}) {
      const handle = await this._selectorOp("locator.elementHandle", api, selector, strict, noWait ? 0 : this._timeout(options), noWait ? { noWait: true } : {});
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
    async _evaluateOn(api, selector, strict, pageFunction, arg, noWait, options = {}) {
      const call = evaluation(api, pageFunction, arg);
      return await this._selectorOp("locator.evaluate", api, selector, strict, noWait ? 0 : this._timeout(options), noWait ? { ...call, noWait: true } : call);
    }
    async _evaluateAll(api, selector, pageFunction, arg) {
      const call = evaluation(api, pageFunction, arg);
      const target = await this._locatorBrand(api, selector);
      return await invoke("locator.evaluateAll", this._call(api, 0), {
        target,
        ...call
      });
    }
    async _snapshot(api, selector, options = {}) {
      if (options.boxes)
        throw unsupportedError(api, "Neo snapshots carry no boxes; use locator.boundingBox()");
      const mode = options.mode ?? "default";
      const shape = pick({ mode, depth: options.depth }, ["mode", "depth"]);
      const timeout = this._timeout(options);
      if (selector !== null)
        return await this._selectorOp("locator.ariaSnapshot", api, selector, true, timeout, shape);
      if (this._id !== null)
        throw unsupportedError(api, "snapshot a child frame through its iframe: page.locator('iframe').contentFrame().locator('body').ariaSnapshot()");
      return await invoke("page.ariaSnapshot", this._call(api, timeout), {
        page: await this._page._brand(api),
        ...shape
      });
    }
    async _ariaSnapshot(api, selector, options) {
      return renderSnapshot(await this._snapshot(api, selector, options));
    }
    async _ariaSnapshotJSON(api, selector, options) {
      return (await this._snapshot(api, selector, options)).json;
    }
    async _screenshot(api, selector, options = {}) {
      if (options.path !== undefined)
        throw unsupportedError(api, "Neo returns the image instead of writing a file; omit path");
      const refused = refusedScreenshotOption(options);
      if (refused !== undefined)
        throw unsupportedError(api, refused);
      const wire = pick(options, SCREENSHOT_KEYS);
      const page = await this._page._brand(api);
      const timeout = this._timeout(options);
      if (selector === null)
        return await invoke("page.screenshot", this._call(api, timeout), {
          page,
          ...wire
        });
      const target = await this._locatorBrand(api, selector);
      return await invoke("page.screenshot", this._call(api, timeout), {
        page,
        target,
        strict: "strict",
        ...wire
      });
    }
    async _goto(api, url, options = {}) {
      if (typeof url !== "string")
        throw clientError(api, `url: expected string, got ${typeof url}`);
      const waitUntil = loadState(api, "waitUntil", options.waitUntil ?? "load");
      const args = await this._frameArgs(api);
      const result = await invoke("page.goto", this._call(api, this._navigationTimeout(options)), {
        ...args,
        url,
        waitUntil,
        ...pick(options, ["referer"])
      });
      return await this._page._navigated(api, this, result);
    }
    async _waitForLoadState(api, state, options = {}) {
      const verified = loadState(api, "state", state ?? "load");
      const result = await invoke("page.waitForLoadState", this._call(api, this._navigationTimeout(options)), { ...await this._frameArgs(api), state: verified });
      await this._page._navigated(api, this, result);
    }
    async _content(api) {
      return await invoke("page.content", this._call(api, 0), await this._frameArgs(api));
    }
    async _setContent(api, html, options = {}) {
      const waitUntil = loadState(api, "waitUntil", options.waitUntil ?? "load");
      const result = await invoke("page.setContent", this._call(api, this._navigationTimeout(options)), { ...await this._frameArgs(api), html, waitUntil });
      await this._page._navigated(api, this, result);
    }
    async _evaluate(api, pageFunction, arg) {
      const call = evaluation(api, pageFunction, arg);
      return await invoke("page.evaluate", this._call(api, 0), {
        ...await this._frameArgs(api),
        ...call
      });
    }
    async _waitForFunction(api, pageFunction, arg, options = {}) {
      if (typeof options.polling === "string" && options.polling !== "raf")
        throw clientError(api, `Unknown polling option: ${options.polling}`);
      const call = evaluation(api, pageFunction, arg);
      const value = await invoke("page.waitForFunction", this._call(api, this._timeout(options)), {
        ...await this._frameArgs(api),
        ...call,
        ...typeof options.polling === "number" ? { polling: options.polling } : {}
      });
      return new ValueHandle(value);
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
  function refusedScreenshotOption(options) {
    if (options.mask !== undefined && options.mask.length > 0)
      return "Neo cannot mask elements in a screenshot; omit mask";
    if (options.style !== undefined)
      return "Neo cannot add a stylesheet to a screenshot; omit style";
    if (options.omitBackground)
      return "Neo cannot capture a transparent background; omit omitBackground";
    if (options.animations === "disabled")
      return "Neo does not stop animations for a screenshot; omit animations";
    if (options.scale === "css")
      return 'Neo captures at the device scale; omit scale or pass "device"';
    return;
  }

  class ValueHandle {
    #value;
    constructor(value) {
      this.#value = value;
      markHandle(this);
    }
    async jsonValue() {
      return this.#value;
    }
    asElement() {
      return null;
    }
    async dispose() {}
    toString() {
      return `JSHandle@${typeof this.#value === "object" ? JSON.stringify(this.#value) : String(this.#value)}`;
    }
  }

  // packages/browseros-playwright-js/src/prelude/element-handle.ts
  function elementBrand(value) {
    if (value instanceof ElementHandle)
      return value._brand;
    if (typeof value !== "object" || value === null)
      return null;
    if (Reflect.get(value, "$neo") !== "element")
      return null;
    return value;
  }

  class ElementHandle extends EventEmitter {
    _page;
    _brand;
    constructor(page, brand) {
      super();
      this._page = page;
      this._brand = brand;
      markHandle(this);
    }
    asElement() {
      return this;
    }
    async dispose() {}
    toString() {
      return this._brand.display;
    }
    toJSON() {
      return this._brand;
    }
    async#op(op, api, timeout, args) {
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
      await this.#op("locator.setInputFiles", api, this.#timeout(options), {
        files: inputFilePaths(api, files)
      });
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
    async evaluate(pageFunction, arg) {
      const api = "elementHandle.evaluate";
      return await this.#op("locator.evaluate", api, 0, evaluation(api, pageFunction, arg));
    }
    async screenshot(options = {}) {
      const api = "elementHandle.screenshot";
      if (options.path !== undefined)
        throw unsupportedError(api, "Neo returns the image instead of writing a file; omit path");
      const refused = refusedScreenshotOption(options);
      if (refused !== undefined)
        throw unsupportedError(api, refused);
      const wire = pick(options, SCREENSHOT_KEYS);
      return await invoke("page.screenshot", { api, timeoutMs: timeoutMs(api, this.#timeout(options)) }, {
        page: await this._page._brand(api),
        target: this._brand,
        strict: "strict",
        ...wire
      });
    }
  }

  // packages/browseros-playwright-js/src/prelude/input.ts
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

  // packages/browseros-playwright-js/src/prelude/response.ts
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

  // packages/browseros-playwright-js/src/prelude/page.ts
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
    _id;
    _tab = null;
    _context;
    _timeoutSettings;
    _mainFrame;
    keyboard;
    mouse;
    _url;
    _title = "";
    _closed = false;
    #childFrames = new Map;
    constructor(context, id, url = "about:blank") {
      super();
      this._context = context;
      this._id = id;
      this._url = url;
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
        throw new Error("Either name or url matcher should be specified");
      return this.frames().find((frame) => frameMatches(frame, selector)) ?? null;
    }
    url() {
      return this._url;
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
      return null;
    }
    workers() {
      return [];
    }
    toJSON() {
      return this._id === null ? { $neo: "page", id: null } : { $neo: "page", id: this._id, tab: this._tab };
    }
    locator(selector, options) {
      return this._mainFrame.locator(selector, options);
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
    $eval(selector, pageFunction, arg) {
      return this._mainFrame._evaluateOn("page.$eval", selector, false, pageFunction, arg, true);
    }
    $$eval(selector, pageFunction, arg) {
      return this._mainFrame._evaluateAll("page.$$eval", selector, pageFunction, arg);
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
    waitForURL(url, options) {
      return this._waitForURL("page.waitForURL", this._mainFrame, url, options);
    }
    async title() {
      const api = "page.title";
      const title = await invoke("page.title", { api, timeoutMs: 0 }, { page: await this._brand(api) });
      this._title = title;
      return title;
    }
    content() {
      return this._mainFrame._content("page.content");
    }
    setContent(html, options) {
      return this._mainFrame._setContent("page.setContent", html, options);
    }
    evaluate(pageFunction, arg) {
      return this._mainFrame._evaluate("page.evaluate", pageFunction, arg);
    }
    waitForFunction(pageFunction, arg, options) {
      return this._mainFrame._waitForFunction("page.waitForFunction", pageFunction, arg, options);
    }
    waitForTimeout(timeout) {
      return sleep("page.waitForTimeout", timeout);
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
      const { opener } = await invoke("page.opener", { api, timeoutMs: 0 }, { page: await this._brand(api) });
      const page = this._context._pageFromResult(opener);
      return page && !page.isClosed() ? page : null;
    }
    async close(options = {}) {
      const api = "page.close";
      if (this._id === null) {
        this._didClose();
        return;
      }
      try {
        await invoke("page.close", { api, timeoutMs: 0 }, {
          page: await this._brand(api),
          ...options.runBeforeUnload ? { runBeforeUnload: true } : {}
        });
      } catch (error) {
        if (Reflect.get(Object(error), "code") !== "target_closed" || options.runBeforeUnload)
          throw error;
      }
      if (!options.runBeforeUnload)
        this._didClose();
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
      if (this._id === null) {
        if (this._closed)
          throw clientError(api, "Target page, context or browser has been closed");
        return { $neo: "page", id: null };
      }
      if (this._tab === null)
        throw clientError(api, "Neo has no tab id for this page; get the page again from context.pages()");
      return { $neo: "page", id: this._id, tab: this._tab };
    }
    async _openedBrand(api) {
      if (this._id === null)
        await this._refreshFrames(api);
      const brand = await this._brand(api);
      if (brand.id === null)
        throw new Error(`${api}: Neo opened no tab for the run's page`);
      return brand;
    }
    _handle(value) {
      if (value instanceof ElementHandle)
        return value;
      const brand = elementBrand(value);
      return brand ? new ElementHandle(this, brand) : null;
    }
    async _navigated(api, frame, result) {
      if (typeof result?.url === "string") {
        if (frame === this._mainFrame)
          this._url = result.url;
        else
          frame._url = result.url;
      }
      if (Array.isArray(result?.frames))
        this._applyFrames(result.frames);
      else
        await this._refreshFrames(api);
      return typeof result?.response?.status === "number" ? new Response(frame, result.response) : null;
    }
    async _refreshFrames(api) {
      const tree = await invoke("page.frames", { api, timeoutMs: 0 }, { page: await this._brand(api) });
      if (Array.isArray(tree?.frames))
        this._applyFrames(tree.frames);
    }
    _applyFrames(infos) {
      const main = infos.find((info) => info.parentId === null);
      if (main) {
        this._url = main.url;
        this._mainFrame._name = main.name;
      }
      const present = new Map;
      for (const info of infos) {
        if (info.parentId === null)
          continue;
        present.set(info.id, info);
        let frame = this.#childFrames.get(info.id);
        if (!frame) {
          frame = new Frame(this, info.id, this._mainFrame);
          this.#childFrames.set(info.id, frame);
        }
        frame._update(info);
      }
      for (const [id, frame] of this.#childFrames) {
        const info = present.get(id);
        if (!info) {
          frame._detached = true;
          this.#childFrames.delete(id);
          continue;
        }
        frame._parentFrame = info.parentId === main?.id ? this._mainFrame : this.#childFrames.get(info.parentId ?? "") ?? this._mainFrame;
      }
    }
    _applyFact(fact) {
      if (fact.tab !== undefined)
        this._tab = fact.tab;
      if (fact.url !== undefined)
        this._url = fact.url;
      if (fact.title !== undefined)
        this._title = fact.title;
      if (fact.closed)
        this._didClose();
    }
    _didClose() {
      if (this._closed)
        return;
      this._closed = true;
      this.emit("close", this);
    }
    async _waitForURL(api, frame, url, options = {}) {
      const waitUntil = loadState(api, "waitUntil", options.waitUntil ?? "load");
      const timeout = frame._navigationTimeout(options);
      const args = await frame._frameArgs(api);
      if (typeof url !== "function") {
        const pattern = typeof url === "string" ? { glob: url } : url instanceof RegExp ? { regexSource: url.source, regexFlags: url.flags } : undefined;
        if (!pattern)
          throw clientError(api, "url: expected a string, a RegExp or a function");
        const result = await invoke("page.waitForURL", frame._call(api, timeout), {
          ...args,
          url: pattern,
          waitUntil
        });
        await this._navigated(api, frame, result);
        return;
      }
      const started = Date.now();
      for (;; ) {
        const current = frame.url();
        let matched;
        try {
          matched = await url(urlLike(current) ?? current);
        } catch (error) {
          throw prefixed(api, error);
        }
        const elapsed = Date.now() - started;
        const remaining = timeout === 0 ? 0 : Math.max(1, timeout - elapsed);
        if (matched) {
          await frame._waitForLoadState(api, waitUntil, { timeout: remaining });
          return;
        }
        const result = await invoke("page.waitForURL", frame._call(api, remaining), {
          ...args,
          url: {
            regexSource: `^(?!${escapeRegExp(current)}$)`,
            regexFlags: ""
          },
          waitUntil: "commit"
        });
        await this._navigated(api, frame, result);
      }
    }
    async#history(api, options = {}) {
      const waitUntil = loadState(api, "waitUntil", options.waitUntil ?? "load");
      const result = await invoke(api, this._mainFrame._call(api, this._mainFrame._navigationTimeout(options)), { page: await this._brand(api), waitUntil });
      return await this._navigated(api, this._mainFrame, result);
    }
  }

  // packages/browseros-playwright-js/src/prelude/context.ts
  function pageFact(value) {
    if (value instanceof Page)
      return value._id === null ? null : value._tab === null ? { id: value._id } : { id: value._id, tab: value._tab };
    if (typeof value !== "object" || value === null)
      return null;
    const id = Reflect.get(value, "id");
    const tab = Reflect.get(value, "tab");
    if (typeof id !== "number")
      return null;
    return typeof tab === "number" ? { id, tab } : { id };
  }

  class BrowserContext extends EventEmitter {
    _timeoutSettings = new TimeoutSettings;
    _events;
    _browser = null;
    #byId = new Map;
    #unopened = null;
    constructor() {
      super();
      this._events = new EventHub(this);
      markHandle(this);
    }
    pages() {
      return [...this.#byId.values()].filter((page) => !page.isClosed());
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
    _pageFor(fact) {
      let page = this.#byId.get(fact.id);
      if (!page) {
        page = new Page(this, fact.id, fact.url);
        this.#byId.set(fact.id, page);
      }
      page._applyFact(fact);
      return page;
    }
    _pageFromResult(value, url) {
      const fact = pageFact(value);
      if (!fact)
        return null;
      return this._pageFor(url === undefined ? fact : { ...fact, url });
    }
    _runPage(init) {
      for (const fact of init.pages)
        this._pageFor(fact);
      if (init.page)
        return this._pageFor(init.page);
      this.#unopened = new Page(this, null);
      return this.#unopened;
    }
    _applyReply({ pages, defaultPage }) {
      const page = this.#unopened;
      if (defaultPage && page) {
        this.#unopened = null;
        page._id = defaultPage.id;
        page._tab = defaultPage.tab;
        this.#byId.set(defaultPage.id, page);
      }
      for (const fact of pages)
        this._pageFor(fact);
    }
    async#openPage(api) {
      const opened = await invoke("context.newPage", { api, timeoutMs: 0 }, {});
      const page = this._pageFromResult(opened.page, opened.url);
      if (!page)
        throw clientError(api, "context.newPage returned no page");
      return page;
    }
  }

  // packages/browseros-playwright-js/src/prelude/expect-equality.ts
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
        throw new TypeError("any() expects to be passed a constructor function. Please pass one or use anything() to match any object.");
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

  // packages/browseros-playwright-js/src/prelude/expect-values.ts
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

  // packages/browseros-playwright-js/src/prelude/expect-retrying.ts
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
  function text(receiver, expression, expected, values, options, extra = {}) {
    if (typeof expected !== "string" && !(expected instanceof RegExp))
      throw new Error("expected value must be a string or regular expression");
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
    }) : text("Locator", "to.have.text", expected, textValues([expected], {
      matchSubstring: true,
      normalizeWhiteSpace: true,
      ignoreCase: options?.ignoreCase
    }), options),
    toHaveAccessibleDescription: (expected, options) => text("Locator", "to.have.accessible.description", expected, textValues([expected], {
      ignoreCase: options?.ignoreCase,
      normalizeWhiteSpace: true
    }), options),
    toHaveAccessibleName: (expected, options) => text("Locator", "to.have.accessible.name", expected, textValues([expected], {
      ignoreCase: options?.ignoreCase,
      normalizeWhiteSpace: true
    }), options),
    toHaveAccessibleErrorMessage: (expected, options) => text("Locator", "to.have.accessible.error.message", expected, textValues([expected], {
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
      return text("Locator", "to.have.attribute.value", value, textValues([value], { ignoreCase: options?.ignoreCase }), options, { expressionArg: name });
    },
    toHaveClass: (expected, options) => Array.isArray(expected) ? equal("to.have.class.array", expected, options, {
      expectedText: textValues(expected)
    }) : text("Locator", "to.have.class", expected, textValues([expected]), options),
    toContainClass: (expected, options) => {
      if (Array.isArray(expected)) {
        if (expected.some((item) => item instanceof RegExp))
          throw new Error('"expected" argument in toContainClass cannot contain RegExp values');
        return equal("to.contain.class.array", expected, options, {
          expectedText: textValues(expected)
        });
      }
      if (expected instanceof RegExp)
        throw new Error('"expected" argument in toContainClass cannot be a RegExp value');
      return text("Locator", "to.contain.class", expected, textValues([expected]), options);
    },
    toHaveCount: (expected, options) => equal("to.have.count", expected, options, { expectedNumber: expected }),
    toHaveCSS: (name, expected, options) => text("Locator", "to.have.css", expected, textValues([expected]), options, {
      expressionArg: name,
      ...options?.pseudo ? { pseudo: options.pseudo } : {}
    }),
    toHaveId: (expected, options) => text("Locator", "to.have.id", expected, textValues([expected]), options),
    toHaveJSProperty: (name, expected, options) => equal("to.have.property", expected, options, {
      expressionArg: name,
      expectedValue: expected
    }),
    toHaveRole: (expected, options) => {
      if (typeof expected !== "string")
        throw new Error('"role" argument in toHaveRole must be a string');
      return text("Locator", "to.have.role", expected, textValues([expected]), options);
    },
    toHaveText: (expected, options) => Array.isArray(expected) ? equal("to.have.text.array", expected, options, {
      expectedText: textValues(expected, {
        normalizeWhiteSpace: true,
        ignoreCase: options?.ignoreCase
      })
    }) : text("Locator", "to.have.text", expected, textValues([expected], {
      normalizeWhiteSpace: true,
      ignoreCase: options?.ignoreCase
    }), options),
    toHaveValue: (expected, options) => text("Locator", "to.have.value", expected, textValues([expected]), options),
    toHaveValues: (expected, options) => equal("to.have.values", expected, options, {
      expectedText: textValues(expected)
    }),
    toHaveTitle: (expected, options) => text("Page", "to.have.title", expected, textValues([expected], { normalizeWhiteSpace: true }), options),
    toHaveURL: (expected, options) => {
      const value = typeof expected === "string" ? normalizeUrl(expected) : expected;
      return text("Page", "to.have.url", value, textValues([value], { ignoreCase: options?.ignoreCase }), options);
    }
  };
  async function retry(name, actual, state, matcher) {
    const api = `expect.${name}`;
    const isPage = actual instanceof Page;
    const isLocator = actual instanceof Locator;
    if (matcher.receiver === "Page" ? !isPage : !isLocator)
      throw new Error(`${name} can be only used with ${matcher.receiver} object, was called with ${receiverText(actual)}`);
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
        passed = Boolean(await predicate(urlLike(page.url()) ?? page.url()));
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

  // packages/browseros-playwright-js/src/prelude/unsupported.ts
  var NETWORK = "network interception is not available in Neo";
  var NO_HANDLES = "Neo returns values as JSON, without handles; use evaluate()";
  var NO_INSPECTOR = "there is no inspector in Neo";
  var NO_TOUCH = "Neo has no touch input; use click()";
  var PROFILE = "cookies and storage stay with the owner's signed-in profile";
  var OWNED = "Neo owns the browser and its profile; close a tab with page.close()";
  var TRACE = "Neo records its own trace of every run";
  var ROOTED = "element handles cannot root a selector in Neo; use locator.locator(selector) instead";
  var UNSUPPORTED = {
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
      addInitScript: "init scripts are not available in Neo; run page.evaluate() after page.goto()",
      clearCookies: PROFILE,
      clearPermissions: "permissions are the owner's profile settings",
      close: OWNED,
      cookies: PROFILE,
      exposeBinding: "pages cannot call back into the script; read state with page.evaluate()",
      exposeFunction: "pages cannot call back into the script; read state with page.evaluate()",
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
      addInitScript: "init scripts are not available in Neo; run page.evaluate() after page.goto()",
      addLocatorHandler: "locator handlers are not available in Neo; check for the overlay with locator.isVisible() before acting",
      addScriptTag: "adding tags is not available in Neo; change the page with page.evaluate()",
      addStyleTag: "adding tags is not available in Neo; change the page with page.evaluate()",
      bringToFront: "Neo keeps agent tabs in the background; act on the page directly",
      cancelPickLocator: NO_INSPECTOR,
      clearConsoleMessages: 'Neo keeps no console history; listen with page.on("console")',
      clearPageErrors: 'Neo keeps no error history; listen with page.on("pageerror")',
      consoleMessages: 'Neo keeps no console history; listen with page.on("console") before the page logs',
      emulateMedia: "media emulation is not available in Neo",
      evaluateHandle: NO_HANDLES,
      exposeBinding: "pages cannot call back into the script; read state with page.evaluate()",
      exposeFunction: "pages cannot call back into the script; read state with page.evaluate()",
      hideHighlight: NO_INSPECTOR,
      pageErrors: 'Neo keeps no error history; listen with page.on("pageerror") before the page fails',
      pause: NO_INSPECTOR,
      pickLocator: NO_INSPECTOR,
      removeLocatorHandler: "locator handlers are not available in Neo",
      requestGC: "garbage collection is not available in Neo",
      requests: "Neo does not record network requests",
      route: NETWORK,
      routeFromHAR: NETWORK,
      routeWebSocket: NETWORK,
      setExtraHTTPHeaders: "network settings are not available in Neo",
      setViewportSize: "the hosted viewport is fixed",
      tap: NO_TOUCH,
      unroute: NETWORK,
      unrouteAll: NETWORK,
      waitForNavigation: "use page.waitForURL() or page.waitForLoadState()",
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
      addScriptTag: "adding tags is not available in Neo; change the page with frame.evaluate()",
      addStyleTag: "adding tags is not available in Neo; change the page with frame.evaluate()",
      evaluateHandle: NO_HANDLES,
      frameElement: "locate the iframe from its parent instead: page.locator('iframe')",
      tap: NO_TOUCH,
      waitForNavigation: "use frame.waitForURL() or frame.waitForLoadState()",
      webmcp: { hint: "WebMCP is not available in Neo", kind: "property" }
    },
    Locator: {
      drop: "dropping data is not available in Neo; use locator.dragTo()",
      evaluateHandle: NO_HANDLES,
      hideHighlight: NO_INSPECTOR,
      highlight: NO_INSPECTOR,
      normalize: "not available in Neo; use the locator as built",
      tap: NO_TOUCH,
      waitForFunction: "poll with expect(locator).toPass() or page.waitForFunction()"
    },
    FrameLocator: {},
    Keyboard: {},
    Mouse: {},
    ElementHandle: {
      $$eval: "not available in Neo yet: it runs on elements like locator.evaluateAll(), which is not built yet; read the matches of elementHandle.$$(selector) with their own methods",
      $eval: "not available in Neo yet: it runs on an element like locator.evaluate(), which is not built yet; read the match of elementHandle.$(selector) with its own methods",
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
      createReadStream: "read a finished download with download.saveAs(path)",
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
    for (const [className, members] of Object.entries(UNSUPPORTED)) {
      if (className === "expect")
        continue;
      const prototype = classes[className].prototype;
      for (const [member, entry] of Object.entries(members))
        define(prototype, className, member, entry);
    }
  }
  function installUnsupportedExpect(target, part) {
    for (const [member, entry] of Object.entries(UNSUPPORTED.expect))
      if (member.startsWith("to") === (part === "matchers"))
        define(target, "expect", member, entry);
  }

  // packages/browseros-playwright-js/src/prelude/expect.ts
  function retrying(name, actual, state, args) {
    if (name === "toHaveURL" && typeof args[0] === "function") {
      if (!(actual instanceof Page))
        throw new Error(`toHaveURL can be only used with Page object, was called with ${receiverText(actual)}`);
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
      throw new Error("toPass: expected a function");
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
      throw new Error("expect.poll: expected a function");
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

  // packages/browseros-playwright-js/src/prelude/index.ts
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
  onReply((reply) => context._applyReply(reply));
  provide("page", (init) => context._runPage(init));
  provide("context", () => context);
  provide("browser", () => browser);
  provide("expect", () => expect);
})();
