/**
 * Records goldens from real Playwright, so Neo's ports are tested against what Playwright
 * does rather than against a reading of its source:
 *
 *   crates/neo-sdk/neo-playwright/tests/data/key-events.json
 *       the Input.dispatchKeyEvent and Input.insertText messages Playwright sends for a key
 *       corpus, for a macOS browser (with its editing commands) and a Linux one
 *   crates/neo-sdk/neo-playwright/tests/data/mouse-events.json
 *       the Input.* messages Playwright sends for a corpus of page.mouse calls
 *   crates/neo-sdk/neo-playwright-js/tests/data/selector-goldens.json
 *       the selector string each locator expression builds, and its toString()
 *   crates/neo-sdk/neo-playwright-js/tests/data/api-methods.json
 *       the public methods and properties of each client class, and expect's matchers
 *
 * Capture method: a CDP proxy. Playwright connects over CDP to a WebSocket proxy in this
 * process, which forwards to the browser on BROWSEROS_CDP_PORT and records every message
 * Playwright sends. Only Input.* methods and their params are kept, never ids, sessions or
 * timings, so a rerun writes the same bytes. For the linux profile the proxy rewrites the
 * user agent in the reply to Browser.getVersion: that user agent is the only input to
 * Playwright's browser-platform check (crBrowser._platform), and the platform only decides
 * whether macOS editing commands are sent. ControlOrMeta is different: Playwright resolves
 * it by its own host (process.platform), so the linux profile leaves it out.
 *
 * Selector strings come from Locator's private `_selector`, the string Playwright's client
 * sends to its server; Neo's prelude must build the same bytes.
 *
 *   PLAYWRIGHT_NODE_MODULES=<node_modules with playwright-core and @playwright/test 1.64.1> \
 *   BROWSEROS_CDP_PORT=9361 node scripts/capture-goldens.mjs [--check]
 *
 * --check recaptures and fails if any file would change. The versions must equal
 * crates/neo-sdk/neo-playwright/third_party/playwright/VERSION. The script opens and closes its
 * own tabs and never touches another.
 */
import { readFileSync, writeFileSync } from 'node:fs'
import http from 'node:http'
import { createRequire } from 'node:module'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))
const packageRoot = resolve(here, '..')
const agentRoot = resolve(packageRoot, '../../..')
const vendoredTag = readFileSync(
  resolve(
    agentRoot,
    'crates/neo-sdk/neo-playwright/third_party/playwright/VERSION',
  ),
  'utf8',
).split('\n')[0]
const nodeModules = process.env.PLAYWRIGHT_NODE_MODULES
if (!nodeModules) {
  console.error(
    'Set PLAYWRIGHT_NODE_MODULES to a node_modules holding playwright-core and @playwright/test',
  )
  process.exit(2)
}
const require = createRequire(resolve(nodeModules, 'capture-goldens.cjs'))
for (const name of ['playwright-core', '@playwright/test']) {
  const { version } = require(`${name}/package.json`)
  if (`v${version}` !== vendoredTag) {
    console.error(
      `${name} is ${version}; the vendored engine is ${vendoredTag}`,
    )
    process.exit(2)
  }
}
const { chromium } = require('playwright-core')
const {
  ws: WebSocket,
  wsServer: WebSocketServer,
} = require('playwright-core/lib/utilsBundle')
const { expect } = require('@playwright/test')
const PORT = Number(process.env.BROWSEROS_CDP_PORT ?? 9005)
const PLAYWRIGHT = vendoredTag.slice(1)

/**
 * A CDP endpoint that forwards to the browser and records what its client sends. With
 * `platform: 'linux'` the browser's user agent reads as Linux to the client.
 */
async function startProxy(platform) {
  const version = await (
    await fetch(`http://127.0.0.1:${PORT}/json/version`)
  ).json()
  const sent = []
  const server = http.createServer((request, response) => {
    if (!request.url.startsWith('/json/version')) {
      response.writeHead(404).end()
      return
    }
    const { port } = server.address()
    response.writeHead(200, { 'content-type': 'application/json' })
    response.end(
      JSON.stringify({
        ...version,
        webSocketDebuggerUrl: `ws://127.0.0.1:${port}/devtools/browser/proxy`,
      }),
    )
  })
  const sockets = new WebSocketServer({ server })
  sockets.on('connection', (client) => {
    const browser = new WebSocket(version.webSocketDebuggerUrl, {
      perMessageDeflate: false,
      maxPayload: 256 * 1024 * 1024,
    })
    const versionRequests = new Set()
    const early = []
    browser.on('open', () => {
      for (const text of early.splice(0)) browser.send(text)
    })
    client.on('message', (data) => {
      const text = data.toString()
      const message = JSON.parse(text)
      sent.push(message)
      if (message.method === 'Browser.getVersion')
        versionRequests.add(message.id)
      if (browser.readyState === WebSocket.OPEN) browser.send(text)
      else early.push(text)
    })
    browser.on('message', (data) => {
      let text = data.toString()
      if (platform === 'linux') {
        const message = JSON.parse(text)
        if (versionRequests.has(message.id) && message.result) {
          message.result.userAgent = message.result.userAgent.replace(
            /\([^)]*\)/,
            '(X11; Linux x86_64)',
          )
          text = JSON.stringify(message)
        }
      }
      client.send(text)
    })
    client.on('close', () => browser.close())
    browser.on('close', () => client.close())
  })
  await new Promise((listening) => server.listen(0, '127.0.0.1', listening))
  return {
    endpoint: `http://127.0.0.1:${server.address().port}`,
    sent,
    close: () => {
      sockets.close()
      server.close()
    },
  }
}

/** Expressions over `page`, run as written; each is its own golden's name. */
const KEY_CASES = [
  "page.keyboard.press('a')",
  "page.keyboard.press('A')",
  "page.keyboard.press('Shift+A')",
  "page.keyboard.press('Shift+a')",
  "page.keyboard.press('KeyA')",
  "page.keyboard.press('Shift+KeyA')",
  "page.keyboard.press('1')",
  "page.keyboard.press('!')",
  "page.keyboard.press('Digit1')",
  "page.keyboard.press('Shift+Digit1')",
  "page.keyboard.press(' ')",
  "page.keyboard.press('Space')",
  "page.keyboard.press('Enter')",
  "page.keyboard.press('Shift+Enter')",
  "page.keyboard.press('NumpadEnter')",
  "page.keyboard.press('Numpad1')",
  "page.keyboard.press('Tab')",
  "page.keyboard.press('Shift+Tab')",
  "page.keyboard.press('ArrowLeft')",
  "page.keyboard.press('ArrowRight')",
  "page.keyboard.press('ArrowUp')",
  "page.keyboard.press('ArrowDown')",
  "page.keyboard.press('Shift+ArrowLeft')",
  "page.keyboard.press('Alt+ArrowLeft')",
  "page.keyboard.press('Meta+ArrowLeft')",
  "page.keyboard.press('Shift+Home')",
  "page.keyboard.press('Home')",
  "page.keyboard.press('End')",
  "page.keyboard.press('PageDown')",
  "page.keyboard.press('Backspace')",
  "page.keyboard.press('Alt+Backspace')",
  "page.keyboard.press('Delete')",
  "page.keyboard.press('Escape')",
  "page.keyboard.press('F2')",
  "page.keyboard.press('ControlOrMeta+A')",
  "page.keyboard.press('ControlOrMeta+C')",
  "page.keyboard.press('ControlOrMeta+V')",
  "page.keyboard.press('ControlOrMeta+Z')",
  "page.keyboard.press('Control+A')",
  "page.keyboard.press('Meta+A')",
  "page.keyboard.press('Control+Shift+K')",
  "page.keyboard.down('Shift').then(() => page.keyboard.press('ArrowLeft')).then(() => page.keyboard.up('Shift'))",
  "page.keyboard.down('a').then(() => page.keyboard.down('a')).then(() => page.keyboard.up('a'))",
  "page.keyboard.type('Hi there!')",
  "page.keyboard.type('Mixed é 👋 123')",
  "page.keyboard.insertText('héllo 👋')",
  "page.locator('#field').pressSequentially('Ab 1é!')",
  "page.locator('#field').press('Enter')",
  "page.locator('#area').press('Shift+Enter')",
  "page.locator('#field').fill('filled')",
  "page.locator('#field').fill('')",
]

/** Cases whose events depend on the Playwright host's OS rather than the browser's. */
const HOST_DEPENDENT = /ControlOrMeta/

async function captureKeys(platform) {
  const proxy = await startProxy(platform)
  const browser = await chromium.connectOverCDP(proxy.endpoint)
  const page = await browser.contexts()[0].newPage()
  try {
    // Not setContent: on a fresh BrowserOS tab it never sees "load" (measured, 1.64.1).
    await page.goto(
      'data:text/html,<input id="field"><textarea id="area"></textarea>',
    )
    const cases = []
    for (const call of KEY_CASES) {
      if (platform === 'linux' && HOST_DEPENDENT.test(call)) continue
      await page.locator('#field').fill('seed text')
      await page.locator('#field').focus()
      const start = proxy.sent.length
      await new Function('page', `return ${call}`)(page)
      const messages = proxy.sent
        .slice(start)
        .filter((message) => message.method?.startsWith('Input.'))
        .map(({ method, params }) => ({ method, params }))
      cases.push({ call, messages })
    }
    return cases
  } finally {
    await page.close()
    await browser.close()
    proxy.close()
  }
}

/**
 * Mouse cases, each a list of calls run in order as `page[device][method](...args)`, so the
 * Rust test can replay them without parsing JavaScript. Every case starts at (0, 0) with
 * nothing held.
 */
const MOUSE_CASES = [
  [['mouse.move', 100, 50]],
  [
    ['mouse.move', 100, 50],
    ['mouse.move', 200, 150, { steps: 4 }],
  ],
  [['mouse.move', 10.25, 20.75]],
  [['mouse.move', 30, 40, { steps: 0 }], ['mouse.down'], ['mouse.up']],
  [['mouse.move', 30, 40], ['mouse.down'], ['mouse.up']],
  [
    ['mouse.down', { button: 'right' }],
    ['mouse.up', { button: 'right' }],
  ],
  [
    ['mouse.down', { button: 'middle', clickCount: 2 }],
    ['mouse.up', { button: 'middle', clickCount: 2 }],
  ],
  [
    ['mouse.down'],
    ['mouse.down', { button: 'right' }],
    ['mouse.move', 50, 50],
    ['mouse.up', { button: 'right' }],
    ['mouse.move', 60, 60],
    ['mouse.up'],
  ],
  [['mouse.down'], ['mouse.move', 60, 60, { steps: 2 }], ['mouse.up']],
  [['mouse.click', 50, 60]],
  [['mouse.click', 50, 60, { clickCount: 3 }]],
  [['mouse.click', 50, 60, { button: 'right' }]],
  [['mouse.click', 50, 60, { delay: 20 }]],
  [['mouse.click', 5, 5, { steps: 3 }]],
  [['mouse.dblclick', 70, 80]],
  [['mouse.wheel', 0, 120]],
  [
    ['mouse.move', 40, 40],
    ['mouse.wheel', 10, -30],
  ],
  [
    ['keyboard.down', 'Shift'],
    ['mouse.click', 20, 20],
    ['keyboard.up', 'Shift'],
  ],
  [
    ['keyboard.down', 'Alt'],
    ['keyboard.down', 'Meta'],
    ['mouse.move', 9, 9],
    ['mouse.wheel', 0, 5],
    ['keyboard.up', 'Meta'],
    ['keyboard.up', 'Alt'],
  ],
]

async function captureMouse() {
  const proxy = await startProxy('mac')
  const browser = await chromium.connectOverCDP(proxy.endpoint)
  const page = await browser.contexts()[0].newPage()
  try {
    // A page larger than the viewport, so the wheel has something to scroll.
    await page.goto(
      'data:text/html,<body style="margin:0;width:3000px;height:3000px"></body>',
    )
    const cases = []
    for (const calls of MOUSE_CASES) {
      await page.mouse.move(0, 0)
      const start = proxy.sent.length
      for (const [name, ...args] of calls) {
        const [device, method] = name.split('.')
        await page[device][method](...args)
      }
      const messages = proxy.sent
        .slice(start)
        .filter((message) => message.method?.startsWith('Input.'))
        .map(({ method, params }) => ({ method, params }))
      cases.push({ calls, messages })
    }
    return cases
  } finally {
    await page.close()
    await browser.close()
    proxy.close()
  }
}

/** Expressions over `page` that build a Locator. */
const SELECTOR_CASES = [
  "page.getByRole('button')",
  "page.getByRole('button', { name: 'Sign in' })",
  "page.getByRole('button', { name: 'Sign in', exact: true })",
  "page.getByRole('button', { name: /sign in/i })",
  "page.getByRole('button', { name: 'Say \"hi\"' })",
  "page.getByRole('checkbox', { checked: true })",
  "page.getByRole('checkbox', { checked: false })",
  "page.getByRole('checkbox', { checked: 'mixed' })",
  "page.getByRole('button', { disabled: true })",
  "page.getByRole('treeitem', { expanded: true })",
  "page.getByRole('heading', { level: 2 })",
  "page.getByRole('button', { pressed: true })",
  "page.getByRole('button', { pressed: 'mixed' })",
  "page.getByRole('tab', { selected: true })",
  "page.getByRole('button', { includeHidden: true })",
  "page.getByRole('link', { description: 'Opens in a new tab' })",
  "page.getByRole('link', { description: /new tab/ })",
  "page.getByRole('heading', { name: 'Docs', level: 1, exact: true })",
  "page.getByText('Hello')",
  "page.getByText('Hello', { exact: true })",
  'page.getByText(/hel+o/i)',
  "page.getByText('It\\'s \"quoted\"')",
  "page.getByLabel('Email')",
  "page.getByLabel('Email', { exact: true })",
  'page.getByLabel(/e-?mail/i)',
  "page.getByPlaceholder('Search')",
  "page.getByPlaceholder('Search', { exact: true })",
  'page.getByPlaceholder(/search/i)',
  "page.getByAltText('Logo')",
  "page.getByAltText('Logo', { exact: true })",
  'page.getByAltText(/logo/i)',
  "page.getByTitle('Close')",
  "page.getByTitle('Close', { exact: true })",
  'page.getByTitle(/close/i)',
  "page.getByTestId('save')",
  'page.getByTestId(/save-\\d+/)',
  "page.locator('.card')",
  "page.locator('#main >> text=Hello')",
  'page.locator(\'xpath=//div[@id="main"]\')',
  "page.locator('//div')",
  "page.locator('li', { hasText: 'two' })",
  "page.locator('li', { has: page.locator('a') })",
  "page.locator('li').filter({ hasText: 'two' })",
  "page.locator('li').filter({ hasText: /two/i })",
  "page.locator('li').filter({ hasNotText: 'three' })",
  "page.locator('li').filter({ hasNotText: /three/ })",
  "page.locator('li').filter({ has: page.getByRole('link', { name: 'Docs' }) })",
  "page.locator('li').filter({ hasNot: page.getByRole('img') })",
  "page.locator('li').filter({ visible: true })",
  "page.locator('li').filter({ visible: false })",
  "page.locator('li').filter({ hasText: 'a', hasNotText: 'b' })",
  "page.locator('li').nth(2)",
  "page.locator('li').nth(-1)",
  "page.locator('li').first()",
  "page.locator('li').last()",
  "page.getByRole('button').and(page.getByTitle('Save'))",
  "page.getByRole('button').or(page.getByRole('link'))",
  "page.locator('.card').getByRole('button', { name: 'Save' })",
  "page.getByRole('listitem').getByText('Docs')",
  "page.locator('#a').locator('#b')",
  "page.locator('.card').locator(page.getByRole('button'))",
  "page.getByRole('button').describe('the save button')",
  "page.frameLocator('#pay').getByRole('button')",
  "page.frameLocator('#outer').frameLocator('#inner').getByLabel('Card number')",
  "page.frameLocator('#pay').locator('input').first()",
  "page.frameLocator('iframe').first().getByText('Pay')",
  "page.frameLocator('iframe').nth(1).getByText('Pay')",
  "page.frameLocator('#pay').owner()",
  "page.locator('#pay').contentFrame().getByRole('button')",
  "page.locator('#pay').contentFrame().owner()",
  "page.getByRef('e5')",
  "page.getByRef('f2e7')",
  "page.getByRef('e5').getByRole('button')",
  "page.locator('li').filter({ has: page.getByRef('e7') })",
  "page.locator('ref=e12').getByRole('button')",
  "page.mainFrame().getByRole('button', { name: 'Go' })",
]

async function captureSelectors(page) {
  return SELECTOR_CASES.map((code) => {
    const locator = new Function('page', `return ${code}`)(page)
    return { code, selector: locator._selector, toString: String(locator) }
  })
}

/**
 * Public members as a script sees them: the instance's own (Page assigns `on` and `off` in
 * its constructor, expect's matchers are own properties) and those up its prototype chain.
 */
function members(instance) {
  const methods = new Set()
  const properties = new Set()
  for (
    let holder = instance;
    holder && holder !== Object.prototype;
    holder = Object.getPrototypeOf(holder)
  ) {
    for (const name of Object.getOwnPropertyNames(holder)) {
      if (name === 'constructor' || name.startsWith('_')) continue
      const descriptor = Object.getOwnPropertyDescriptor(holder, name)
      if (typeof descriptor.value === 'function') methods.add(name)
      else properties.add(name)
    }
  }
  for (const name of methods) properties.delete(name)
  return { methods: [...methods].sort(), properties: [...properties].sort() }
}

async function captureApi(browser, page) {
  const handle = await page.$('body')
  try {
    const classes = {
      Browser: browser,
      BrowserContext: page.context(),
      Page: page,
      Frame: page.mainFrame(),
      Locator: page.locator('body'),
      FrameLocator: page.frameLocator('iframe'),
      Keyboard: page.keyboard,
      Mouse: page.mouse,
      ElementHandle: handle,
    }
    const api = {}
    for (const [name, instance] of Object.entries(classes))
      api[name] = members(instance)
    return {
      ...api,
      expect: {
        statics: Object.keys(expect).sort(),
        matchers: members(expect(page.locator('body'))).methods,
        modifiers: ['not', 'resolves', 'rejects'].filter(
          (name) => name in expect(page.locator('body')),
        ),
      },
    }
  } finally {
    await handle.dispose()
  }
}

const outputs = {
  'crates/neo-sdk/neo-playwright/tests/data/key-events.json': {
    playwright: PLAYWRIGHT,
    capture:
      'scripts/capture-goldens.mjs, a CDP proxy between playwright-core and the browser',
    note: 'Each case runs in a focused <input>. linux omits ControlOrMeta, which Playwright resolves by its own host OS (darwin when captured), not by the browser.',
    profiles: {
      mac: await captureKeys('mac'),
      linux: await captureKeys('linux'),
    },
  },
  'crates/neo-sdk/neo-playwright/tests/data/mouse-events.json': {
    playwright: PLAYWRIGHT,
    capture:
      'scripts/capture-goldens.mjs, a CDP proxy between playwright-core and the browser',
    note: 'Each case starts at (0, 0) with nothing held; calls run in order as page[device][method](...args).',
    cases: await captureMouse(),
  },
}
const proxy = await startProxy('mac')
const browser = await chromium.connectOverCDP(proxy.endpoint)
const page = await browser.contexts()[0].newPage()
try {
  outputs['crates/neo-sdk/neo-playwright-js/tests/data/selector-goldens.json'] =
    {
      playwright: PLAYWRIGHT,
      cases: await captureSelectors(page),
    }
  outputs['crates/neo-sdk/neo-playwright-js/tests/data/api-methods.json'] = {
    playwright: PLAYWRIGHT,
    classes: await captureApi(browser, page),
  }
} finally {
  await page.close()
  await browser.close()
  proxy.close()
}

// --check recaptures and compares instead of writing: a rerun must give the same bytes.
const changed = []
for (const [path, value] of Object.entries(outputs)) {
  const text = `${JSON.stringify(value, null, 2)}\n`
  const target = resolve(agentRoot, path)
  if (!process.argv.includes('--check')) writeFileSync(target, text)
  else if (readFileSync(target, 'utf8') !== text) changed.push(path)
}
if (changed.length > 0) {
  console.error(`A recapture differs from: ${changed.join(', ')}`)
  process.exit(1)
}
console.log(
  `${process.argv.includes('--check') ? 'matched' : 'wrote'} ${KEY_CASES.length} key cases (2 profiles), ${MOUSE_CASES.length} mouse cases, ${SELECTOR_CASES.length} selectors and the API of Playwright ${PLAYWRIGHT}`,
)
