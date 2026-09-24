// Playwright-shaped values for the bounded QuickJS host. All browser effects
// cross __browserosCall so ownership, auditing, cancellation and redaction stay
// in Rust. Selectors follow Playwright 1.63's Apache-2.0 client utilities.
;(() => {
  const AsyncFunction = Object.getPrototypeOf(async () => {}).constructor

  // Timers: the runtime is a bare engine, so bridge setTimeout/sleep to a real
  // async sleep. For content that loads in, prefer locator.waitFor().
  globalThis.sleep = (ms) => __browserosSleep(Number(ms) || 0)
  globalThis.setTimeout = (fn, ms) => {
    __browserosSleep(Number(ms) || 0).then(() => {
      if (typeof fn === 'function') fn()
    })
    return 0
  }
  globalThis.clearTimeout = () => {}

  function safeStringify(value) {
    if (value === undefined) return 'undefined'
    try {
      const encoded = JSON.stringify(value, null, 2)
      return encoded ?? String(value)
    } catch {
      return String(value)
    }
  }

  function jsonSafeString(value) {
    const seen = new WeakSet()
    let encoded
    try {
      encoded = JSON.stringify(value, (_key, next) => {
        if (typeof next === 'bigint') return next.toString()
        if (typeof next === 'function' || typeof next === 'symbol') {
          return String(next)
        }
        if (typeof next === 'number' && !Number.isFinite(next)) return null
        if (typeof next === 'object' && next !== null) {
          if (seen.has(next)) return '[Circular]'
          seen.add(next)
        }
        return next
      })
    } catch {
      return JSON.stringify(safeStringify(value))
    }
    return encoded
  }

  const sink =
    (level) =>
    (...parts) =>
      __browserosPushLog(
        level +
          parts
            .map((part) =>
              typeof part === 'string' ? part : safeStringify(part),
            )
            .join(' '),
      )
  const console = {
    log: sink(''),
    info: sink(''),
    warn: sink('warn: '),
    error: sink('error: '),
    debug: sink(''),
  }
  const own = (value, key) => Object.hasOwn(value, key)
  const defaultHint =
    'Use page.evaluate() or neo.cdp() for supported browser operations.'
  function unavailable(api, hint = defaultHint) {
    throw new Error(`not available in BrowserOS neo: ${api}. ${hint}`)
  }
  class TimeoutError extends Error {
    constructor(message) {
      super(message)
      this.name = 'TimeoutError'
    }
  }
  // Rust exceptions arrive as Error objects containing a string. Recover the
  // Playwright class without duplicating its name in String(error).
  async function call(method, args = []) {
    try {
      return await __browserosCall(method, JSON.stringify(args), false)
    } catch (error) {
      const message =
        error && error.message !== undefined ? error.message : String(error)
      if (message.startsWith('TimeoutError:'))
        throw new TimeoutError(message.slice(13).trimStart())
      if (message.startsWith('Error:'))
        throw new Error(message.slice(6).trimStart())
      throw error instanceof Error ? error : new Error(message)
    }
  }
  // JSON has no RegExp value. P6 decodes this explicit descriptor; selectors
  // instead use Playwright's literal-regexp escaping below.
  function wire(value) {
    if (value instanceof RegExp)
      return { regexSource: value.source, regexFlags: value.flags }
    if (Array.isArray(value)) return value.map(wire)
    return value
  }
  function urlPattern(value) {
    if (value instanceof RegExp)
      return { source: value.source, flags: value.flags }
    return typeof value === 'function' ? { predicate: source(value) } : value
  }
  function source(fn) {
    if (typeof fn !== 'function' && typeof fn !== 'string')
      throw new Error('Expected a function or string expression')
    return String(fn)
  }
  function timeoutOptions(options, fallback) {
    const result = { ...options }
    if (result.timeout === undefined) result.timeout = fallback
    if (
      typeof result.timeout !== 'number' ||
      result.timeout < 0 ||
      !Number.isFinite(result.timeout)
    )
      throw new Error('timeout must be a non-negative finite number')
    return result
  }

  // Ported from Microsoft Playwright's locatorUtils.ts and stringUtils.ts
  // (Apache-2.0). Attribute strings deliberately use different escaping from
  // text strings. Changing this breaks the injected script's selector parser.
  function escapeRegex(re) {
    if (re.unicode || re.unicodeSets) return String(re)
    return String(re)
      .replace(/(^|[^\\])(\\\\)*(["'`])/g, '$1$2\\$3')
      .replace(/>>/g, '\\>\\>')
  }
  function textSelector(text, exact) {
    return typeof text === 'string'
      ? JSON.stringify(text) + (exact ? 's' : 'i')
      : escapeRegex(text)
  }
  function attributeSelector(value, exact) {
    return typeof value === 'string'
      ? '"' +
          value.replace(/\\/g, '\\\\').replace(/["]/g, '\\"') +
          '"' +
          (exact ? 's' : 'i')
      : escapeRegex(value)
  }
  function roleSelector(role, options = {}) {
    const props = []
    for (const key of [
      'checked',
      'disabled',
      'selected',
      'expanded',
      'includeHidden',
      'level',
    ]) {
      if (options[key] !== undefined)
        props.push([
          key === 'includeHidden' ? 'include-hidden' : key,
          String(options[key]),
        ])
    }
    if (options.name !== undefined)
      props.push(['name', attributeSelector(options.name, !!options.exact)])
    if (options.pressed !== undefined)
      props.push(['pressed', String(options.pressed)])
    return (
      `internal:role=${role}` +
      props.map(([key, value]) => `[${key}=${value}]`).join('')
    )
  }
  const join = (left, right) => (left ? left + ' >> ' + right : right)
  function samePage(
    left,
    right,
    message = 'Locators must belong to the same frame.',
  ) {
    if (
      !(right instanceof Locator) ||
      (left._page !== right._page &&
        (left._page.pageId === undefined ||
          left._page.pageId !== right._page.pageId))
    )
      throw new Error(message)
  }
  function filterSelector(page, selector, options = {}) {
    for (const [key, engine] of [
      ['hasText', 'has-text'],
      ['hasNotText', 'has-not-text'],
    ]) {
      if (options[key])
        selector += ` >> internal:${engine}=${textSelector(options[key], false)}`
    }
    for (const [key, engine] of [
      ['has', 'has'],
      ['hasNot', 'has-not'],
    ]) {
      if (options[key]) {
        samePage(
          { _page: page },
          options[key],
          `Inner "${key}" locator must belong to the same frame.`,
        )
        selector += ` >> internal:${engine}=${JSON.stringify(options[key].selector)}`
      }
    }
    if (options.visible !== undefined)
      selector += ` >> visible=${options.visible ? 'true' : 'false'}`
    return selector
  }
  class Selectors {
    getByRole(role, options) {
      return this.locator(roleSelector(role, options))
    }
    getByText(text, options) {
      return this.locator(
        'internal:text=' + textSelector(text, !!options?.exact),
      )
    }
    getByLabel(text, options) {
      return this.locator(
        'internal:label=' + textSelector(text, !!options?.exact),
      )
    }
    getByTestId(text) {
      return this.locator(
        'internal:testid=[data-testid=' + attributeSelector(text, true) + ']',
      )
    }
    getByPlaceholder(text, options) {
      return this._attribute('placeholder', text, options)
    }
    getByAltText(text, options) {
      return this._attribute('alt', text, options)
    }
    getByTitle(text, options) {
      return this._attribute('title', text, options)
    }
    _attribute(attr, text, options) {
      return this.locator(
        `internal:attr=[${attr}=${attributeSelector(text, !!options?.exact)}]`,
      )
    }
  }

  // A Locator is immutable; resolving its page is deferred until an effect.
  // This lets page.getByRole(...).first() compose before the lazy tab exists.
  class Locator extends Selectors {
    constructor(page, selector, options) {
      super()
      this._page = page
      this.selector = filterSelector(page, selector, options)
      Object.freeze(this)
    }
    get pageId() {
      return this._page.pageId
    }
    page() {
      return this._page
    }
    locator(selector, options) {
      if (selector instanceof Locator) {
        samePage(this, selector)
        selector = 'internal:chain=' + JSON.stringify(selector.selector)
      }
      return new Locator(this._page, join(this.selector, selector), options)
    }
    filter(options) {
      return new Locator(this._page, this.selector, options)
    }
    nth(n) {
      return new Locator(this._page, this.selector + ` >> nth=${n}`)
    }
    first() {
      return this.nth(0)
    }
    last() {
      return this.nth(-1)
    }
    and(other) {
      samePage(this, other)
      return this.locator('internal:and=' + JSON.stringify(other.selector))
    }
    or(other) {
      samePage(this, other)
      return this.locator('internal:or=' + JSON.stringify(other.selector))
    }
    frameLocator(selector) {
      return new FrameLocator(this._page, join(this.selector, selector))
    }
    contentFrame() {
      return new FrameLocator(this._page, this.selector)
    }
    _action(method, args, options) {
      return this._page._call('locator.' + method, [
        this.selector,
        ...args,
        this._page._options(options),
      ])
    }
    click(options) {
      return this._action('click', [], options)
    }
    dblclick(options) {
      return this._action('dblclick', [], options)
    }
    hover(options) {
      return this._action('hover', [], options)
    }
    fill(value, options) {
      return this._action('fill', [value], options)
    }
    type(value, options) {
      return this._action('type', [value], options)
    }
    pressSequentially(value, options) {
      return this.type(value, options)
    }
    press(key, options) {
      return this._action('press', [key], options)
    }
    check(options) {
      return this._action('check', [true], options)
    }
    uncheck(options) {
      return this._action('check', [false], options)
    }
    setChecked(checked, options) {
      return this._action('check', [checked], options)
    }
    selectOption(values, options) {
      return this._action('selectOption', [values], options)
    }
    setInputFiles(paths, options) {
      return this._action('setInputFiles', [paths], options)
    }
    focus(options) {
      return this._action('focus', [], options)
    }
    blur(options) {
      return this._action('blur', [], options)
    }
    clear(options) {
      return this._action('clear', [], options)
    }
    scrollIntoViewIfNeeded(options) {
      return this._action('scrollIntoViewIfNeeded', [], options)
    }
    dragTo(target, options) {
      samePage(this, target)
      return this._action('dragTo', [target.selector], options)
    }
    waitFor(options = {}) {
      return this._action('waitFor', [options.state || 'visible'], options)
    }
    evaluate(fn, arg, options) {
      return this._action('evaluate', [source(fn), arg], options)
    }
    evaluateAll(fn, arg, options) {
      return this._action('evaluateAll', [source(fn), arg], options)
    }
    evaluateHandle() {
      return unavailable(
        'locator.evaluateHandle',
        'Use locator.evaluate() to return JSON.',
      )
    }
    // query's fifth argument extends the table so queries carry the same
    // inherited timeout as actions; its fourth remains the query argument.
    _query(what, arg, options) {
      return this._page._call('locator.query', [
        this.selector,
        what,
        arg,
        this._page._options(options),
      ])
    }
    count() {
      return this._query('count')
    }
    textContent(options) {
      return this._query('textContent', undefined, options)
    }
    innerText(options) {
      return this._query('innerText', undefined, options)
    }
    innerHTML(options) {
      return this._query('innerHTML', undefined, options)
    }
    inputValue(options) {
      return this._query('inputValue', undefined, options)
    }
    getAttribute(name, options) {
      return this._query('getAttribute', name, options)
    }
    isVisible(options) {
      return this._query('isVisible', undefined, options)
    }
    isHidden(options) {
      return this._query('isHidden', undefined, options)
    }
    isEnabled(options) {
      return this._query('isEnabled', undefined, options)
    }
    async isDisabled(options) {
      return !(await this.isEnabled(options))
    }
    isChecked(options) {
      return this._query('isChecked', undefined, options)
    }
    isEditable(options) {
      return this._query('isEditable', undefined, options)
    }
    boundingBox(options) {
      return this._query('boundingBox', undefined, options)
    }
    allTextContents() {
      return this._query('allTextContents')
    }
    allInnerTexts() {
      return this._query('allInnerTexts')
    }
    ariaSnapshot(options) {
      return this._query('ariaSnapshot', undefined, options)
    }
    async all() {
      return Array.from({ length: await this.count() }, (_, n) => this.nth(n))
    }
    toString() {
      return `locator(${JSON.stringify(this.selector)})`
    }
  }
  class FrameLocator extends Selectors {
    constructor(page, selector) {
      super()
      this._page = page
      this.selector = selector
      Object.freeze(this)
    }
    locator(selector, options) {
      if (selector instanceof Locator) {
        samePage(this, selector)
        selector = selector.selector
      }
      return new Locator(
        this._page,
        this.selector + ' >> internal:control=enter-frame >> ' + selector,
        options,
      )
    }
    frameLocator(selector) {
      return new FrameLocator(
        this._page,
        this.selector + ' >> internal:control=enter-frame >> ' + selector,
      )
    }
    owner() {
      return new Locator(this._page, this.selector)
    }
    nth(n) {
      return new FrameLocator(this._page, this.selector + ` >> nth=${n}`)
    }
    first() {
      return this.nth(0)
    }
    last() {
      return this.nth(-1)
    }
  }

  const pages = new Map()
  const inheritedTimeout = Symbol('inheritedTimeout')
  let lastUsed
  let usage = 0
  function remember(info) {
    const id = typeof info === 'number' ? info : info?.pageId
    if (!Number.isInteger(id) || id <= 0 || id > 0xffffffff)
      throw new Error('Bridge returned an invalid pageId')
    let page = pages.get(id)
    if (!page) {
      page = new Page(id)
      pages.set(id, page)
    }
    if (typeof info === 'object') page._update(info)
    return page
  }
  function unwrap(response, page) {
    if (
      response &&
      typeof response === 'object' &&
      own(response, 'value') &&
      (own(response, 'url') || own(response, 'title'))
    ) {
      if (page) page._update(response)
      return response.value
    }
    return response
  }
  function warnNoop(api) {
    console.warn(
      `${api} is a no-op in BrowserOS neo; the signed-in browser stays open and in the background.`,
    )
  }

  // Event subscriptions are bounded bridge waits, armed before the next action.
  // QuickJS has no host EventEmitter: each subscription owns one pending wait,
  // re-arms after delivery, and is discarded with the per-run runtime. Removing
  // a listener suppresses delivery even if its already-issued wait completes.
  class Events extends Selectors {
    constructor() {
      super()
      this._listeners = new Map()
    }
    on(kind, handler) {
      this._validateEvent(kind)
      if (typeof handler !== 'function')
        throw new Error('Event handler must be a function')
      let subscription = this._listeners.get(kind)
      if (!subscription) {
        subscription = { active: true, handlers: new Set() }
        this._listeners.set(kind, subscription)
        subscription.handlers.add(handler)
        this._pump(kind, subscription)
      } else subscription.handlers.add(handler)
      return this
    }
    once(kind, handler) {
      const once = async (value) => {
        this.off(kind, once)
        await handler(value)
      }
      return this.on(kind, once)
    }
    off(kind, handler) {
      const subscription = this._listeners.get(kind)
      if (subscription) {
        subscription.handlers.delete(handler)
        if (!subscription.handlers.size) {
          subscription.active = false
          this._listeners.delete(kind)
        }
      }
      return this
    }
    removeListener(kind, handler) {
      return this.off(kind, handler)
    }
    removeAllListeners(kind) {
      for (const [name, subscription] of this._listeners) {
        if (kind === undefined || kind === name) {
          subscription.active = false
          this._listeners.delete(name)
        }
      }
      return this
    }
    async _pump(kind, subscription) {
      try {
        while (subscription.active) {
          const value = await this.waitForEvent(kind, { timeout: 0 })
          if (!subscription.active) break
          for (const handler of [...subscription.handlers]) await handler(value)
        }
      } catch (error) {
        if (!subscription.active) return
        subscription.active = false
        this._listeners.delete(kind)
        // A detached listener must never become an unhandled rejection.
        // Its failure is also surfaced at the next browser operation.
        this._eventError = error
        console.error(error.name + ': ' + error.message)
      }
    }
  }
  function eventOptions(options, fallback) {
    if (typeof options === 'function') options = { predicate: options }
    options = options || {}
    const { predicate, ...rest } = options
    return { predicate, options: timeoutOptions(rest, fallback) }
  }
  // QuickJS has neither Buffer nor TextEncoder/TextDecoder. Response bodies
  // remain byte arrays locally; CDP's base64 transport is decoded only after
  // the host has waited for the correlated request to finish loading.
  function bodyBytes(reply) {
    const bytes = []
    if (reply.base64Encoded) {
      const alphabet =
        'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/'
      let bits = 0,
        count = 0
      for (const char of reply.body) {
        if (char === '=') break
        if (/\s/.test(char)) continue
        const value = alphabet.indexOf(char)
        if (value < 0) throw new Error('Invalid base64 response body')
        bits = (bits << 6) | value
        count += 6
        if (count >= 8) {
          count -= 8
          bytes.push((bits >> count) & 255)
        }
      }
    } else {
      for (const char of reply.body) {
        let cp = char.codePointAt(0)
        if (cp >= 0xd800 && cp <= 0xdfff) cp = 0xfffd
        if (cp < 0x80) bytes.push(cp)
        else if (cp < 0x800) bytes.push(0xc0 | (cp >> 6), 0x80 | (cp & 63))
        else if (cp < 0x10000)
          bytes.push(
            0xe0 | (cp >> 12),
            0x80 | ((cp >> 6) & 63),
            0x80 | (cp & 63),
          )
        else
          bytes.push(
            0xf0 | (cp >> 18),
            0x80 | ((cp >> 12) & 63),
            0x80 | ((cp >> 6) & 63),
            0x80 | (cp & 63),
          )
      }
    }
    const result = new Uint8Array(bytes)
    Object.defineProperty(result, 'toString', { value: () => bodyText(result) })
    return result
  }
  function bodyText(bytes) {
    let result = ''
    for (let i = 0; i < bytes.length; ) {
      const first = bytes[i++]
      if (first < 128) {
        result += String.fromCharCode(first)
        continue
      }
      const length =
        first >= 0xc2 && first <= 0xdf
          ? 1
          : first >= 0xe0 && first <= 0xef
            ? 2
            : first >= 0xf0 && first <= 0xf4
              ? 3
              : 0
      if (!length) {
        result += '\ufffd'
        continue
      }
      let cp = first & (0x7f >> (length + 1)),
        consumed = 0
      while (
        consumed < length &&
        i + consumed < bytes.length &&
        (bytes[i + consumed] & 0xc0) === 0x80
      ) {
        cp = (cp << 6) | (bytes[i + consumed++] & 63)
      }
      if (
        consumed !== length ||
        cp < [0, 0x80, 0x800, 0x10000][length] ||
        cp > 0x10ffff ||
        (cp >= 0xd800 && cp <= 0xdfff)
      ) {
        result += '\ufffd'
      } else {
        i += consumed
        result += String.fromCodePoint(cp)
      }
    }
    return result
  }
  function eventValue(kind, data, page) {
    if (kind === 'popup' || kind === 'page') {
      const opened = remember(data)
      opened._owned = true
      return opened
    }
    if (kind === 'dialog')
      return {
        type: () => data.type,
        message: () => data.message,
        defaultValue: () => data.defaultValue || '',
        page: () => page,
        accept: (text) => page._call('page.dialog', [true, text]),
        dismiss: () => page._call('page.dialog', [false]),
      }
    if (kind === 'download')
      return {
        suggestedFilename: () => data.suggestedFilename || data.filename,
        url: () => data.url,
        page: () => page,
        path: async () => data.path,
        failure: async () => data.failure || null,
        saveAs: () =>
          unavailable(
            'download.saveAs',
            'Use the path returned by download.path() or neo.download().',
          ),
      }
    if (kind === 'console')
      return {
        type: () => data.type,
        text: () => data.text,
        location: () => data.location || {},
        page: () => page,
      }
    if (kind === 'pageerror') {
      const error = new Error(data.message || String(data))
      error.name = data.name || 'Error'
      return error
    }
    if (kind === 'request' || kind === 'response') {
      const result = {
        url: () => data.url,
        method: () => data.method,
        headers: () => data.headers || {},
        status: () => data.status,
        statusText: () => data.statusText || '',
        ok: () => data.status >= 200 && data.status < 300,
        postData: () => data.postData ?? null,
      }
      if (kind === 'response') {
        let pending
        const read = () => {
          if (!pending)
            pending = (
              own(data, 'body')
                ? Promise.resolve(data)
                : page._call('page.waitForEvent', [
                    'response',
                    { requestId: data.requestId },
                    page._options({ body: true }),
                  ])
            ).then(bodyBytes)
          return pending
        }
        result.body = read
        result.text = async () => bodyText(await read())
        result.json = async () => JSON.parse(await result.text())
        result.request = () => eventValue('request', data, page)
      }
      return result
    }
    return data
  }

  class Page extends Events {
    constructor(id) {
      super()
      this._id = id
      this._url = ''
      this._title = ''
      this._closed = false
      this.keyboard = {
        press: (key, options) =>
          this._call('keyboard.press', [key, this._options(options)]),
        type: (text, options) =>
          this._call('keyboard.type', [text, this._options(options)]),
        insertText: (text) =>
          this._call('keyboard.insertText', [text, this._options()]),
      }
      this.mouse = {
        click: (x, y, options) =>
          this._call('mouse.click', [x, y, this._options(options)]),
        move: (x, y, options) =>
          this._call('mouse.move', [x, y, this._options(options)]),
        wheel: (deltaX, deltaY) =>
          this._call('mouse.wheel', [deltaX, deltaY, this._options()]),
      }
    }
    get pageId() {
      return this._resolved ? this._resolved.pageId : this._id
    }
    _update(info) {
      if (typeof info.url === 'string') this._url = info.url
      if (typeof info.title === 'string') this._title = info.title
    }
    _options(options, navigation = false) {
      const p = this._resolved || this
      const fallback = navigation
        ? (p._navigationTimeout ??
          p._timeout ??
          context._navigationTimeout ??
          context._timeout)
        : (p._timeout ?? context._timeout)
      const result = timeoutOptions(options, fallback)
      // Lazy tab selection happens after callers construct their arguments.
      // This local-only marker lets dispatch inherit the selected page's
      // defaults without ever overwriting an explicit per-call timeout.
      if (options?.timeout === undefined) result[inheritedTimeout] = navigation
      return result
    }
    async _resolve() {
      if (this._id !== undefined) return this
      if (this._resolved && !this._resolved.isClosed()) return this._resolved
      if (!this._resolving) {
        this._resolving = (async () => {
          // The host owns cross-run recency and ownership; array order cannot
          // identify the last used tab. In-run calls maintain recency locally.
          let selected
          if (lastUsed && !lastUsed.isClosed() && lastUsed._owned) {
            const open = await context.pages()
            selected = open.find((page) => page.pageId === lastUsed.pageId)
          }
          if (!selected) {
            const info = unwrap(await call('context.lastPage'))
            selected = info == null ? await context.newPage() : remember(info)
            selected._owned = true
          }
          this._resolved = selected
          if (this._timeout !== undefined) selected._timeout = this._timeout
          if (this._navigationTimeout !== undefined)
            selected._navigationTimeout = this._navigationTimeout
          return selected
        })()
      }
      try {
        return await this._resolving
      } finally {
        this._resolving = undefined
      }
    }
    async _call(method, args = []) {
      if (this._eventError) {
        const error = this._eventError
        this._eventError = undefined
        throw error
      }
      let page
      try {
        page = await this._resolve()
      } catch (error) {
        // During piecewise integration, name the requested API and the missing
        // lazy-initialization dependency. Never rewrite real browser failures.
        if (error.message.startsWith('not implemented yet: '))
          error.message = `not implemented yet: ${method} (page initialization requires ${error.message.slice(21)})`
        throw error
      }
      if (page._closed)
        throw new Error(
          `${method}: Target page, context or browser has been closed`,
        )
      if (page._owned) lastUsed = page
      page._lastUsed = ++usage
      const options = args[args.length - 1]
      if (options && own(options, inheritedTimeout)) {
        args = args.slice()
        args[args.length - 1] = page._options(
          { ...options, timeout: undefined },
          options[inheritedTimeout],
        )
      }
      return unwrap(await call(method, [page.pageId, ...args]), page)
    }
    locator(selector, options) {
      return new Locator(this, selector, options)
    }
    frameLocator(selector) {
      return new FrameLocator(this, selector)
    }
    context() {
      return context
    }
    url() {
      return (this._resolved || this)._url
    }
    title() {
      return (this._resolved || this)._title
    }
    isClosed() {
      return (this._resolved || this)._closed
    }
    setDefaultTimeout(timeout) {
      ;(this._resolved || this)._timeout = timeoutOptions(
        { timeout },
        10000,
      ).timeout
    }
    setDefaultNavigationTimeout(timeout) {
      ;(this._resolved || this)._navigationTimeout = timeoutOptions(
        { timeout },
        10000,
      ).timeout
    }
    async _navigate(method, args, options) {
      const value = await this._call('page.' + method, [
        ...args,
        this._options(options, true),
      ])
      // P4/P5 normally attach metadata. An explicit info refresh also covers
      // hosts whose navigation resolves before their tab cache is refreshed.
      const info = await this._call('page.info')
      if (info) (this._resolved || this)._update(info)
      return value
    }
    goto(url, options) {
      return this._navigate('goto', [url], options)
    }
    reload(options) {
      return this._navigate('reload', [], options)
    }
    goBack(options) {
      return this._navigate('goBack', [], options)
    }
    goForward(options) {
      return this._navigate('goForward', [], options)
    }
    waitForLoadState(state = 'load', options) {
      return this._navigate('waitForLoadState', [state], options)
    }
    waitForURL(pattern, options) {
      return this._navigate('waitForURL', [urlPattern(pattern)], options)
    }
    waitForFunction(fn, arg, options) {
      return this._call('page.waitForFunction', [
        source(fn),
        arg,
        this._options(options),
      ])
    }
    waitForTimeout(ms) {
      return sleep(ms)
    }
    evaluate(fn, arg) {
      return this._call('page.evaluate', [source(fn), arg, this._options()])
    }
    evaluateHandle() {
      return unavailable(
        'page.evaluateHandle',
        'Use page.evaluate() to return JSON.',
      )
    }
    screenshot(options) {
      return this._call('page.screenshot', [this._options(options)])
    }
    pdf(options) {
      return this._call('page.pdf', [this._options(options)])
    }
    content() {
      return this._call('page.content')
    }
    async frames() {
      const data = await this._call('page.frames')
      const page = await this._resolve()
      const frames = []
      function visit(node, parent) {
        const info = node.frame || node
        const frame = {
          frameId: info.id || info.frameId,
          url: () => info.url || '',
          name: () => info.name || '',
          page: () => page,
          parentFrame: () => parent || null,
          childFrames: () =>
            frames.filter((child) => child.parentFrame() === frame),
          isDetached: () => page.isClosed(),
        }
        frames.push(frame)
        for (const child of node.childFrames || []) visit(child, frame)
      }
      // P4 transports CDP's tree. Keep parentage from the tree itself, never
      // from incidental array ordering; a flat host list may carry parentId.
      if (Array.isArray(data)) {
        for (const info of data) visit(info)
        for (let i = 0; i < data.length; ++i) {
          const parent = frames.find(
            (frame) =>
              frame.frameId === (data[i].parentFrameId ?? data[i].parentId),
          )
          if (parent) frames[i].parentFrame = () => parent
        }
      } else if (data?.frameTree) visit(data.frameTree)
      return frames
    }
    bringToFront() {
      warnNoop('page.bringToFront()')
      return Promise.resolve()
    }
    async close() {
      const page = await this._resolve()
      if (page._closed) return
      await page._call('context.close')
      page._closed = true
      page.removeAllListeners()
      this.removeAllListeners()
    }
    _validateEvent(kind) {
      if (
        ![
          'dialog',
          'console',
          'pageerror',
          'popup',
          'download',
          'response',
          'request',
        ].includes(kind)
      )
        unavailable(
          `page event '${kind}'`,
          'Use dialog, console, pageerror, popup, download, response or request.',
        )
    }
    async waitForEvent(kind, options) {
      this._validateEvent(kind)
      const spec = eventOptions(options, this._options().timeout)
      // Predicate source is transported for the host, never evaluated in a
      // different JS world with access to the caller's closure.
      const data = await this._call('page.waitForEvent', [
        kind,
        spec.predicate ? source(spec.predicate) : null,
        spec.options,
      ])
      return eventValue(kind, data, await this._resolve())
    }
  }

  class Context extends Events {
    constructor() {
      super()
      this._timeout = 10000
    }
    async pages() {
      const open = unwrap(await call('context.pages'))
      const result = open.map((info) => {
        const page = remember(info)
        page._owned = true
        return page
      })
      const ids = new Set(result.map((page) => page.pageId))
      for (const page of pages.values())
        if (page._owned && !ids.has(page.pageId)) page._closed = true
      return result
    }
    async newPage(url, options) {
      const page = remember(
        unwrap(
          await call('context.newPage', [
            url,
            timeoutOptions(options, this._timeout),
          ]),
        ),
      )
      page._owned = true
      page._lastUsed = ++usage
      lastUsed = page
      return page
    }
    setDefaultTimeout(timeout) {
      this._timeout = timeoutOptions({ timeout }, 10000).timeout
    }
    setDefaultNavigationTimeout(timeout) {
      this._navigationTimeout = timeoutOptions({ timeout }, 10000).timeout
    }
    close() {
      warnNoop('context.close()')
      return Promise.resolve()
    }
    browser() {
      return browser
    }
    _validateEvent(kind) {
      if (kind !== 'page')
        unavailable(
          `context event '${kind}'`,
          "Use context.on('page') or context.waitForEvent('page').",
        )
    }
    async waitForEvent(kind, options) {
      this._validateEvent(kind)
      const spec = eventOptions(options, this._timeout)
      const page = remember(
        unwrap(
          await call('context.waitForEvent', [
            kind,
            spec.predicate ? source(spec.predicate) : null,
            spec.options,
          ]),
        ),
      )
      page._owned = true
      return page
    }
  }
  const context = new Context()
  const page = new Page()
  const browser = {
    contexts: () => [context],
    newContext: async () => {
      console.warn(
        'browser.newContext(): BrowserOS neo uses the same signed-in profile; contexts are not isolated.',
      )
      return context
    },
    newPage: (...args) => context.newPage(...args),
    close: async () => warnNoop('browser.close()'),
    version: async () => {
      const info = await call('neo.cdp', ['Browser.getVersion', {}])
      return (info.product || '').replace(/^[^/]*\//, '')
    },
  }
  const neo = {
    pages: (options = { ownership: 'all' }) => call('neo.pages', [options]),
    // Rehydration refreshes metadata and records ownership through the bridge.
    page: async (id) => {
      const page = remember(id)
      const info = await page._call('neo.page')
      if (info) page._update(info)
      return page
    },
    snapshot: (target) => pageFor(target)._call('neo.snapshot'),
    read: (target, options) => pageFor(target)._call('neo.read', [options]),
    grep: (target, options) => pageFor(target)._call('neo.grep', [options]),
    download: (target, options) =>
      pageFor(target)._call('neo.download', [options]),
    cdp: async (method, params, target) =>
      call('neo.cdp', [
        method,
        params,
        target === undefined
          ? undefined
          : (await pageFor(target)._resolve()).pageId,
      ]),
  }
  function pageFor(value) {
    return value === undefined
      ? page
      : value instanceof Page
        ? value
        : remember(value)
  }

  // The host uses isNot to decide when to stop polling, but returns the raw
  // match bit. Compare that bit with isNot exactly once, including timeouts.
  // Already-formatted Rust errors pass through unchanged.
  const webMatchers = new Set([
    'toBeVisible',
    'toBeHidden',
    'toBeAttached',
    'toBeEnabled',
    'toBeDisabled',
    'toBeChecked',
    'toBeEditable',
    'toBeEmpty',
    'toBeFocused',
    'toHaveText',
    'toContainText',
    'toHaveValue',
    'toHaveCount',
    'toHaveAttribute',
    'toHaveClass',
    'toHaveId',
    'toHaveCSS',
    'toHaveTitle',
    'toHaveURL',
  ])
  const stateMatchers = new Set([
    'toBeVisible',
    'toBeHidden',
    'toBeAttached',
    'toBeEnabled',
    'toBeDisabled',
    'toBeChecked',
    'toBeEditable',
    'toBeEmpty',
    'toBeFocused',
  ])
  function equal(a, b, seen = new Map()) {
    if (Object.is(a, b)) return true
    if (
      !a ||
      !b ||
      typeof a !== 'object' ||
      typeof b !== 'object' ||
      Object.getPrototypeOf(a) !== Object.getPrototypeOf(b)
    )
      return false
    if (a instanceof Date) return a.getTime() === b.getTime()
    if (a instanceof RegExp) return String(a) === String(b)
    if (Array.isArray(a) && a.length !== b.length) return false
    if (seen.has(a)) return seen.get(a) === b
    seen.set(a, b)
    if (a instanceof Map || a instanceof Set) {
      if (a.size !== b.size) return false
      const remaining = Array.from(b)
      for (const entry of a) {
        const index = remaining.findIndex((other) =>
          equal(entry, other, new Map(seen)),
        )
        if (index < 0) return false
        remaining.splice(index, 1)
      }
      return true
    }
    if (a instanceof Error) return a.name === b.name && a.message === b.message
    const keys = Object.keys(a)
    return (
      keys.length === Object.keys(b).length &&
      keys.every((key) => own(b, key) && equal(a[key], b[key], seen))
    )
  }
  const valueMatchers = {
    toBe: (actual, expected) => Object.is(actual, expected),
    toEqual: equal,
    toContain: (actual, expected) =>
      actual != null &&
      typeof actual.includes === 'function' &&
      actual.includes(expected),
    toBeTruthy: (actual) => !!actual,
    toBeGreaterThan: (actual, expected) => actual > expected,
    toMatch: (actual, expected) =>
      typeof actual === 'string' &&
      (expected instanceof RegExp
        ? new RegExp(expected.source, expected.flags).test(actual)
        : actual.includes(expected)),
  }
  function assertionError(
    target,
    matcher,
    isNot,
    expected,
    received,
    timeout,
    log,
  ) {
    const kind =
      target instanceof Locator
        ? 'locator'
        : target instanceof Page
          ? 'page'
          : 'received'
    const lines = [`expect(${kind})${isNot ? '.not' : ''}.${matcher}() failed`]
    if (target instanceof Locator) lines.push('Locator: ' + target.selector)
    lines.push(
      'Expected: ' + (isNot ? 'not ' : '') + safeStringify(expected),
      'Received: ' + safeStringify(received),
    )
    if (timeout !== undefined) lines.push(`Timeout: ${timeout}ms`)
    if (log?.length)
      lines.push('Call log:', ...log.map((line) => '  - ' + line))
    return new Error(lines.join('\n'))
  }
  function makeExpect(defaults = {}) {
    function expect(target, message) {
      function matchers(isNot) {
        return new Proxy(
          {},
          {
            get(_object, name) {
              if (name === 'not') return matchers(!isNot)
              if (name === 'then') return undefined
              if (own(valueMatchers, name))
                return (...args) => {
                  const matched = valueMatchers[name](target, ...args)
                  if (matched === isNot)
                    throw assertionError(target, name, isNot, args[0], target)
                }
              if (!webMatchers.has(name))
                return () =>
                  unavailable(
                    'expect.' + String(name),
                    'Use the supported value or web-first matchers.',
                  )
              return async (...args) => {
                if (!(target instanceof Locator) && !(target instanceof Page))
                  throw new Error(`expect.${name} requires a Locator or Page`)
                let expected, options
                if (stateMatchers.has(name)) {
                  expected = true
                  options = args[0]
                } else if (name === 'toHaveAttribute' || name === 'toHaveCSS') {
                  const existence =
                    name === 'toHaveAttribute' &&
                    (args[1] === undefined ||
                      (args[1] !== null &&
                        typeof args[1] === 'object' &&
                        !(args[1] instanceof RegExp)))
                  expected = existence ? null : wire(args[1])
                  options = {
                    ...(existence ? args[1] : args[2]),
                    name: args[0],
                  }
                } else {
                  expected = wire(args[0])
                  options = args[1]
                }
                const opts = {
                  ...timeoutOptions(options, defaults.timeout ?? 5000),
                  isNot,
                }
                if (message !== undefined) opts.message = String(message)
                const p = target instanceof Locator ? target._page : target
                const result = await p._call('expect.' + name, [
                  target instanceof Locator ? target.selector : null,
                  expected,
                  opts,
                ])
                if (
                  result &&
                  (result.matches === isNot ||
                    result.timedOut ||
                    result.timed_out)
                )
                  throw assertionError(
                    target,
                    name,
                    isNot,
                    expected,
                    result.received,
                    opts.timeout,
                    result.log,
                  )
              }
            },
          },
        )
      }
      return matchers(false)
    }
    expect.configure = (options) => makeExpect({ ...defaults, ...options })
    // Soft assertions cannot survive between bounded script runs. Keep failure
    // semantics explicit: this compatibility shim is a hard assertion.
    expect.soft = expect
    return expect
  }
  const expect = makeExpect()

  for (const [target, prefix] of [
    [Page.prototype, 'page'],
    [Context.prototype, 'context'],
  ]) {
    for (const api of [
      'route',
      'unroute',
      'addInitScript',
      'setViewportSize',
      'emulateMedia',
      'exposeFunction',
      'pause',
    ])
      target[api] = () => unavailable(prefix + '.' + api)
    for (const api of ['cookies', 'addCookies', 'storageState'])
      target[api] = () =>
        unavailable(prefix + '.' + api, 'you are already signed in')
    for (const api of ['request', 'tracing'])
      Object.defineProperty(target, api, {
        get: () => unavailable(prefix + '.' + api),
      })
  }
  Page.prototype.video = () => unavailable('page.video')
  const nodeHint =
    'No Node.js or modules here; use the provided browser, context, page, expect and neo globals.'
  for (const api of ['require', 'fetch'])
    globalThis[api] = () => unavailable(api, nodeHint)
  for (const api of ['process', 'fs'])
    Object.defineProperty(globalThis, api, {
      configurable: true,
      get: () => unavailable(api, nodeHint),
    })

  const pendingTests = []
  const test = (_name, fn) => {
    const pending = Promise.resolve()
      .then(() => fn({ browser, context, page, expect, neo }))
      .then(() => browser)
    // Mark rejection handled even when a pasted test declaration is unawaited.
    pending.catch(() => {})
    pendingTests.push(pending)
    return pending
  }
  const chromium = {
    launch: async () => browser,
    connectOverCDP: async () => browser,
  }
  Object.assign(globalThis, {
    browser,
    context,
    page,
    expect,
    neo,
    test,
    chromium,
    console,
  })
  globalThis.__browserosBrowser = browser
  globalThis.__browserosConsole = console
  globalThis.__browserosJsonSafeString = jsonSafeString
  globalThis.__browserosSafeStringify = safeStringify
  globalThis.__browserosMakeRunFunction = (code) => {
    let fn
    try {
      fn = new AsyncFunction(`"use strict";\n${code}`)
    } catch (error) {
      if (/\bimport\b/.test(code) && /import|module/i.test(error.message))
        unavailable('import', nodeHint)
      throw error
    }
    // The shared host passes browser/console, already installed above. Keep
    // all entry bindings global so ordinary pasted declarations such as
    // `const page = await context.newPage()` may shadow them in the script.
    // Unawaited test() declarations still finish before the run resolves.
    return async () => {
      try {
        const result = await fn()
        await Promise.all(pendingTests)
        return result
      } catch (error) {
        if (
          /could not load module|module loader|cannot find module/i.test(
            error.message,
          )
        )
          unavailable('import()', nodeHint)
        throw error
      }
    }
  }
})()
