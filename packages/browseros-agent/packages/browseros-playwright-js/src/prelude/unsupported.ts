/**
 * The one list of Playwright members Neo does not support, each with what to use instead.
 *
 * `installUnsupported` turns every entry into a member that fails `neo / unsupported` with its
 * hint, so a model calling a Playwright method Neo lacks learns what to do instead of meeting
 * "undefined is not a function". tests/api-coverage.test.ts compares every class's members with
 * the methods real playwright-core 1.64.1 defines (tests/data/api-methods.json): each must be
 * implemented or listed here, so an upgrade that adds a method fails until it is classified.
 *
 * An entry is async by default (a rejected promise, as the upstream method is async), `sync`
 * when upstream returns a value directly, and `property` for an object-valued property such as
 * `page.request`. In the `expect` entry, names starting with `to` are matchers and the rest are
 * statics on `expect` (expect.ts installs both).
 */
import { unsupportedError } from './errors'

type Entry = string | { hint: string; kind: 'sync' | 'property' }

const NETWORK = 'network interception is not available in Neo'
const NO_HANDLES = 'Neo returns values as JSON, without handles; use evaluate()'
const NO_INSPECTOR = 'there is no inspector in Neo'
const NO_TOUCH = 'Neo has no touch input; use click()'
const PROFILE = "cookies and storage stay with the owner's signed-in profile"
const OWNED =
  'Neo owns the browser and its profile; close a tab with page.close()'
const TRACE = 'Neo records its own trace of every run'
const ROOTED =
  'element handles cannot root a selector in Neo; use locator.locator(selector) instead'

export const UNSUPPORTED = {
  Browser: {
    bind: 'Neo does not share its browser',
    browserType: {
      hint: 'Neo drives its own BrowserOS; there is no browser type to launch',
      kind: 'sync',
    },
    close: OWNED,
    newBrowserCDPSession: 'raw CDP sessions are not available in Neo',
    newContext:
      'Neo has one browser profile; open a tab with context.newPage()',
    newPage: 'Neo has one browser profile; open a tab with context.newPage()',
    startTracing: TRACE,
    stopTracing: TRACE,
    unbind: 'Neo does not share its browser',
  },
  BrowserContext: {
    addCookies: PROFILE,
    addInitScript:
      'init scripts are not available in Neo; run page.evaluate() after page.goto()',
    clearCookies: PROFILE,
    clearPermissions: "permissions are the owner's profile settings",
    close: OWNED,
    cookies: PROFILE,
    exposeBinding:
      'pages cannot call back into the script; read state with page.evaluate()',
    exposeFunction:
      'pages cannot call back into the script; read state with page.evaluate()',
    grantPermissions: "permissions are the owner's profile settings",
    newCDPSession: 'raw CDP sessions are not available in Neo',
    route: NETWORK,
    routeFromHAR: NETWORK,
    routeWebSocket: NETWORK,
    setExtraHTTPHeaders: 'network settings are not available in Neo',
    setGeolocation: "geolocation is the owner's profile setting",
    setHTTPCredentials: 'network settings are not available in Neo',
    setOffline: 'network settings are not available in Neo',
    setStorageState: PROFILE,
    storageState: PROFILE,
    unroute: NETWORK,
    unrouteAll: NETWORK,
    clock: {
      hint: 'clock emulation is not available in Neo',
      kind: 'property',
    },
    credentials: {
      hint: "credentials stay with the owner's profile",
      kind: 'property',
    },
    debugger: {
      hint: 'the debugger is not available in Neo',
      kind: 'property',
    },
    request: {
      hint: 'API requests are not available in Neo; fetch from the page with page.evaluate()',
      kind: 'property',
    },
    tracing: { hint: TRACE, kind: 'property' },
  },
  Page: {
    addInitScript:
      'init scripts are not available in Neo; run page.evaluate() after page.goto()',
    addLocatorHandler:
      'locator handlers are not available in Neo; check for the overlay with locator.isVisible() before acting',
    addScriptTag:
      'adding tags is not available in Neo; change the page with page.evaluate()',
    addStyleTag:
      'adding tags is not available in Neo; change the page with page.evaluate()',
    bringToFront:
      'Neo keeps agent tabs in the background; act on the page directly',
    cancelPickLocator: NO_INSPECTOR,
    clearConsoleMessages:
      'Neo keeps no console history; listen with page.on("console")',
    clearPageErrors:
      'Neo keeps no error history; listen with page.on("pageerror")',
    consoleMessages:
      'Neo keeps no console history; listen with page.on("console") before the page logs',
    emulateMedia: 'media emulation is not available in Neo',
    evaluateHandle: NO_HANDLES,
    exposeBinding:
      'pages cannot call back into the script; read state with page.evaluate()',
    exposeFunction:
      'pages cannot call back into the script; read state with page.evaluate()',
    hideHighlight: NO_INSPECTOR,
    pageErrors:
      'Neo keeps no error history; listen with page.on("pageerror") before the page fails',
    pause: NO_INSPECTOR,
    pickLocator: NO_INSPECTOR,
    removeLocatorHandler: 'locator handlers are not available in Neo',
    requestGC: 'garbage collection is not available in Neo',
    requests: 'Neo does not record network requests',
    route: NETWORK,
    routeFromHAR: NETWORK,
    routeWebSocket: NETWORK,
    setExtraHTTPHeaders: 'network settings are not available in Neo',
    setViewportSize: 'the hosted viewport is fixed',
    tap: NO_TOUCH,
    unroute: NETWORK,
    unrouteAll: NETWORK,
    waitForNavigation: 'use page.waitForURL() or page.waitForLoadState()',
    waitForRequest:
      'Neo does not report network events; wait for what the page shows instead',
    waitForResponse:
      'Neo does not report network events; wait for what the page shows instead',
    clock: {
      hint: 'clock emulation is not available in Neo',
      kind: 'property',
    },
    coverage: { hint: 'coverage is not available in Neo', kind: 'property' },
    localStorage: {
      hint: 'read storage with page.evaluate(() => localStorage.getItem(key))',
      kind: 'property',
    },
    request: {
      hint: 'API requests are not available in Neo; fetch from the page with page.evaluate()',
      kind: 'property',
    },
    screencast: {
      hint: 'Neo records its own screencast of every tab',
      kind: 'property',
    },
    sessionStorage: {
      hint: 'read storage with page.evaluate(() => sessionStorage.getItem(key))',
      kind: 'property',
    },
    touchscreen: {
      hint: 'Neo has no touch input; use page.mouse',
      kind: 'property',
    },
    webmcp: { hint: 'WebMCP is not available in Neo', kind: 'property' },
  },
  Frame: {
    addScriptTag:
      'adding tags is not available in Neo; change the page with frame.evaluate()',
    addStyleTag:
      'adding tags is not available in Neo; change the page with frame.evaluate()',
    evaluateHandle: NO_HANDLES,
    frameElement:
      "locate the iframe from its parent instead: page.locator('iframe')",
    tap: NO_TOUCH,
    waitForNavigation: 'use frame.waitForURL() or frame.waitForLoadState()',
    webmcp: { hint: 'WebMCP is not available in Neo', kind: 'property' },
  },
  Locator: {
    drop: 'dropping data is not available in Neo; use locator.dragTo()',
    evaluateHandle: NO_HANDLES,
    hideHighlight: NO_INSPECTOR,
    highlight: NO_INSPECTOR,
    normalize: 'not available in Neo; use the locator as built',
    tap: NO_TOUCH,
    waitForFunction:
      'poll with expect(locator).toPass() or page.waitForFunction()',
  },
  FrameLocator: {},
  Keyboard: {},
  Mouse: {},
  ElementHandle: {
    contentFrame:
      'use page.frameLocator() or locator.contentFrame() to enter a frame',
    evaluateHandle: NO_HANDLES,
    getProperties:
      'Neo returns values as JSON; read them with elementHandle.evaluate(el => el.prop)',
    getProperty:
      'Neo returns values as JSON; read them with elementHandle.evaluate(el => el.prop)',
    jsonValue:
      'Neo returns values as JSON; read them with elementHandle.evaluate(el => el.prop)',
    ownerFrame: 'use page.frames() to find a frame',
    tap: NO_TOUCH,
    waitForSelector: ROOTED,
  },
  Download: {
    cancel: "Neo keeps the run's downloads; it cannot cancel one",
    createReadStream: 'read a finished download with download.saveAs(path)',
    delete: "Neo keeps the run's downloads; it cannot delete one",
  },
  ConsoleMessage: {
    args: {
      hint: 'Neo passes console text only; use message.text()',
      kind: 'sync',
    },
  },
  Response: {
    body: 'Neo does not read response bodies; read the page with page.content() or page.evaluate()',
    json: 'Neo does not read response bodies; read the page with page.content() or page.evaluate()',
    request: { hint: 'Neo does not report requests', kind: 'sync' },
    securityDetails: 'Neo does not report TLS details',
    serverAddr: 'Neo does not report server addresses',
    text: 'Neo does not read response bodies; read the page with page.content() or page.evaluate()',
  },
  expect: {
    extend: { hint: 'custom matchers are not available in Neo', kind: 'sync' },
    getState: {
      hint: "expect's test state is not available in Neo",
      kind: 'sync',
    },
    soft: {
      hint: 'soft assertions are not available in Neo; catch the error to continue',
      kind: 'sync',
    },
    toBeOK: 'use expect(response.ok()).toBe(true)',
    toHaveScreenshot: 'visual comparisons are not available in Neo',
    toMatchAriaSnapshot:
      'compare the text of await locator.ariaSnapshot() instead, such as with toContain()',
    toMatchSnapshot: 'snapshot files are not available in Neo',
  },
} satisfies Record<string, Record<string, Entry>>

export type UnsupportedClass = keyof typeof UNSUPPORTED

function hintOf(entry: Entry): string {
  return typeof entry === 'string' ? entry : entry.hint
}

/** The API name Playwright prefixes errors with: `browserContext.route`, `page.tap`. */
function apiName(className: string, member: string): string {
  return `${className.charAt(0).toLowerCase()}${className.slice(1)}.${member}`
}

/** A member that fails with its hint: a rejected promise, a throw, or a throwing getter. */
function define(
  target: object,
  className: string,
  member: string,
  entry: Entry,
): void {
  const api = apiName(className, member)
  const error = () => unsupportedError(api, hintOf(entry))
  if (typeof entry !== 'string' && entry.kind === 'property') {
    Object.defineProperty(target, member, {
      configurable: true,
      get: () => {
        throw error()
      },
    })
    return
  }
  const method =
    typeof entry !== 'string' && entry.kind === 'sync'
      ? () => {
          throw error()
        }
      : () => Promise.reject(error())
  Object.defineProperty(target, member, {
    configurable: true,
    writable: true,
    value: method,
  })
}

/** Installs every listed member on its class's prototype. */
export function installUnsupported(
  classes: Record<Exclude<UnsupportedClass, 'expect'>, { prototype: object }>,
): void {
  for (const [className, members] of Object.entries(UNSUPPORTED)) {
    if (className === 'expect') continue
    const prototype =
      classes[className as Exclude<UnsupportedClass, 'expect'>].prototype
    for (const [member, entry] of Object.entries(
      members as Record<string, Entry>,
    ))
      define(prototype, className, member, entry)
  }
}

/** The `expect` entries: statics on `expect` itself, matchers (`to...`) on each assertion. */
export function installUnsupportedExpect(
  target: object,
  part: 'statics' | 'matchers',
): void {
  for (const [member, entry] of Object.entries(
    UNSUPPORTED.expect as Record<string, Entry>,
  ))
    if (member.startsWith('to') === (part === 'matchers'))
      define(target, 'expect', member, entry)
}
