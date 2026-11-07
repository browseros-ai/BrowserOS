// Proves the BUILT bundle works where neo-runtime will use it, before any Rust calls it:
// dist/install.js constructs Playwright's InjectedScript in an isolated world of a real
// BrowserOS neo tab, the glue answers through dist/call.js (handles passed as arguments, and
// results that hold one read with Runtime.getProperties), and a second CDP session reuses the
// install through dist/attach.js. Rust's strict decode of every result is the shape oracle once
// neo-runtime calls the page; these checks pin what that decode will see.
//
// `.mjs`, not `.js`: packages/browseros-agent/.gitignore ignores `**/tests/**/*.js`, which would
// make a local run pass and a clean checkout miss the file. Runs under Node with raw CDP over
// the global WebSocket. Gated on NEO_E2E_BROWSER, the path of a BrowserOS neo binary.

import assert from 'node:assert/strict'
import { execFileSync, spawn } from 'node:child_process'
import { once } from 'node:events'
import { mkdtempSync, readFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { test } from 'node:test'
import { fileURLToPath } from 'node:url'

const browserBinary = process.env.NEO_E2E_BROWSER
const dist = join(dirname(fileURLToPath(import.meta.url)), '../dist')
const bundle = {
  install: readFileSync(join(dist, 'install.js'), 'utf8'),
  attach: readFileSync(join(dist, 'attach.js'), 'utf8'),
  call: readFileSync(join(dist, 'call.js'), 'utf8'),
  specSha256: readFileSync(join(dist, 'spec-sha256'), 'utf8'),
}
// Chromium's values in Playwright's server: frameSeq of a main frame, stableRafCount from
// crPage.ts rafCountForStablePosition, Neo's utility world.
const initializer = {
  frameSeq: 0,
  testIdAttributeName: 'data-testid',
  stableRafCount: 1,
  isUtilityWorld: true,
}
const key = `${bundle.specSha256}/${JSON.stringify(initializer)}`
const WORLD_NAME = '__neo_utility_world__'
const PAGE = [
  '<title>Neo load test</title>',
  '<button>Submit</button>',
  '<button>Submit</button>',
  '<p id="lone">Only one</p>',
  '<iframe srcdoc="<p>inner</p>"></iframe>',
].join('')

test('the built bundle installs once per document and answers through call.js', {
  skip: browserBinary ? false : 'NEO_E2E_BROWSER is unset',
  timeout: 180_000,
}, async () => {
  const cleanups = []
  try {
    const browser = await launchBrowser(cleanups, browserBinary)
    const cdp = await connectCdp(cleanups, browser.port)
    const { targetId } = await cdp.send('Target.createTarget', {
      url: `data:text/html,${encodeURIComponent(PAGE)}`,
    })
    const sessionA = await attachTo(cdp, targetId)
    await waitForTitle(cdp, sessionA, 'Neo load test')

    const contextA = await isolatedWorld(cdp, sessionA, targetId)
    const absent = await callIn(cdp, sessionA, contextA, bundle.attach, [key])
    assert.equal(
      absent.type,
      'undefined',
      'attach finds nothing before install',
    )

    const glue = await callIn(cdp, sessionA, contextA, bundle.install, [
      key,
      bundle.specSha256,
      initializer,
    ])
    assert.equal(glue.type, 'object', 'install returns the glue object')
    const again = await callIn(cdp, sessionA, contextA, bundle.install, [
      key,
      bundle.specSha256,
      initializer,
    ])
    assert.ok(
      await sameObject(cdp, sessionA, glue, again),
      'a second install in the document returns the first glue object',
    )

    const page = (name, params) => pageCall(cdp, sessionA, glue, name, params)
    assert.deepEqual(await page('title', {}), {
      value: { value: 'Neo load test' },
    })
    assert.deepEqual(
      await page('textContent', { selector: '#lone', strict: true }),
      {
        value: {
          matched: { value: 'Only one', preview: '<p id="lone">Only one</p>' },
        },
      },
    )
    assert.deepEqual(
      await page('textContent', { selector: '#missing', strict: true }),
      { value: {} },
      'nothing matched yet is a value, not an error',
    )
    const strict = await page('textContent', {
      selector: 'internal:role=button[name="Submit"i]',
      strict: true,
    })
    assert.match(
      strict.exception.description,
      /^Error: strict mode violation: getByRole\('button', \{ name: 'Submit' \}\) resolved to 2 elements:/,
    )
    assert.doesNotMatch(
      strict.exception.description,
      /\n\s+at /,
      'the strict error is stackless, as Playwright throws it',
    )

    // `frames.ts` `_elementState` and `innerHTML` read the first match; a state the element
    // cannot have throws, as Playwright's page code does; `allInnerTexts` reads every match.
    const lonePreview = '<p id="lone">Only one</p>'
    assert.deepEqual(
      await page('elementStateOnSelector', {
        selector: '#lone',
        strict: true,
        state: 'enabled',
      }),
      {
        value: {
          matched: { matches: true, received: 'enabled', preview: lonePreview },
        },
      },
    )
    assert.deepEqual(
      await page('elementStateOnSelector', {
        selector: '#missing',
        strict: true,
        state: 'disabled',
      }),
      { value: {} },
      'nothing matched yet is a value, not an error',
    )
    assert.match(
      (
        await page('elementStateOnSelector', {
          selector: '#lone',
          strict: true,
          state: 'editable',
        })
      ).exception.description,
      /^Error: Element is not an <input>, <textarea>, <select> or \[contenteditable\]/,
    )
    assert.deepEqual(
      await page('innerHTML', { selector: '#lone', strict: true }),
      { value: { matched: { value: 'Only one', preview: lonePreview } } },
    )
    assert.deepEqual(await page('allInnerTexts', { selector: '#lone' }), {
      value: { values: [{ value: 'Only one' }] },
    })
    assert.deepEqual(await page('allInnerTexts', { selector: '#missing' }), {
      value: { values: [] },
    })

    const aria = await page('ariaSnapshot', {
      selector: 'body',
      strict: false,
      mode: 'ai',
    })
    assert.deepEqual(aria.value.snapshot.iframes, [{ ref: 'e5', depth: 1 }])
    assert.deepEqual(
      aria.value.snapshot.nodes[0].children.map((node) => node.role),
      ['button', 'button', 'paragraph', 'iframe'],
    )

    const owner = await handleCall(cdp, sessionA, glue, 'frameOwner', {
      selector: 'iframe',
      strict: true,
    })
    assert.equal(
      owner.element.subtype,
      'node',
      'frameOwner returns the element as a page object',
    )
    assert.equal(owner.element.className, 'HTMLIFrameElement')
    const noOwner = await handleCall(cdp, sessionA, glue, 'frameOwner', {
      selector: 'iframe.missing',
      strict: true,
    })
    assert.equal(
      noOwner.element.type,
      'undefined',
      'nothing matched reads as an undefined property, which neo-runtime leaves out',
    )
    const notFrame = await page('frameOwner', {
      selector: '#lone',
      strict: true,
    })
    assert.equal(
      notFrame.exception.description,
      'Error: Selector "#lone" resolved to <p id="lone">Only one</p>, <iframe> was expected',
    )

    const lone = await handleCall(cdp, sessionA, glue, 'resolve', {
      selector: '#lone',
      strict: true,
    })
    assert.equal(lone.element.subtype, 'node', 'resolve returns the element')
    assert.deepEqual(
      [lone.count.value, lone.preview.value],
      [1, '<p id="lone">Only one</p>'],
    )
    const buttons = await handleCall(cdp, sessionA, glue, 'resolve', {
      selector: 'internal:role=button[name="Submit"i]',
      strict: false,
    })
    assert.deepEqual(
      [buttons.count.value, buttons.preview.value],
      [2, '<button>Submit</button>'],
      'a non-strict resolve counts every match and previews the first',
    )

    const states = (wanted) =>
      byValueCall(
        cdp,
        sessionA,
        glue,
        'checkElementStates',
        { states: wanted, frameVisible: true },
        [lone.element],
      )
    assert.deepEqual(
      await states(['visible', 'stable']),
      {},
      'the element passed as a handle is visible and stable',
    )
    assert.deepEqual(await states(['hidden']), {
      outcome: { missingState: 'hidden' },
    })

    const center = await cdp.send(
      'Runtime.callFunctionOn',
      {
        objectId: lone.element.objectId,
        functionDeclaration:
          'function () { const r = this.getBoundingClientRect(); return { x: r.x + r.width / 2, y: r.y + r.height / 2 } }',
        returnByValue: true,
      },
      sessionA,
    )
    const armed = await handleCall(
      cdp,
      sessionA,
      glue,
      'setupHitTargetInterceptor',
      { hitPoint: center.result.value, blockAllEvents: false },
      [lone.element],
    )
    assert.equal(armed.outcome.type, 'object', 'the interceptor is armed')
    assert.equal(
      armed.outcome.subtype,
      undefined,
      'a plain object has no CDP subtype, which neo-runtime checks',
    )
    assert.deepEqual(
      await byValueCall(cdp, sessionA, glue, 'stopHitTargetInterceptor', {}, [
        armed.outcome,
      ]),
      { outcome: 'done' },
      'with no mouse event, stop reports done',
    )

    const mainWorld = await cdp.send(
      'Runtime.evaluate',
      { expression: 'typeof globalThis.__neo_injected__', returnByValue: true },
      sessionA,
    )
    assert.equal(
      mainWorld.result.value,
      'undefined',
      "the page's own scripts cannot see Neo's glue",
    )

    const sessionB = await attachTo(cdp, targetId)
    const contextB = await isolatedWorld(cdp, sessionB, targetId)
    const reused = await callIn(cdp, sessionB, contextB, bundle.attach, [key])
    assert.equal(
      reused.type,
      'object',
      'a second session attaches to the install',
    )
    assert.deepEqual(await pageCall(cdp, sessionB, reused, 'title', {}), {
      value: { value: 'Neo load test' },
    })
    const otherKey = await callIn(cdp, sessionB, contextB, bundle.attach, [
      `${bundle.specSha256}/${JSON.stringify({ ...initializer, testIdAttributeName: 'data-qa' })}`,
    ])
    assert.equal(otherKey.type, 'undefined', 'another key never attaches')

    const stale = await cdp.send(
      'Runtime.callFunctionOn',
      {
        executionContextId: contextB,
        functionDeclaration: bundle.install,
        arguments: [
          { value: key },
          { value: '0'.repeat(64) },
          { value: initializer },
        ],
      },
      sessionB,
    )
    assert.match(
      stale.exceptionDetails.exception.description,
      /^Error: neo-runtime-injected was built from spec [0-9a-f]{64}, not 0{64}/,
    )
  } finally {
    for (const cleanup of cleanups.reverse()) await cleanup()
  }
})

/** Calls a dist entry in a world by context id, as neo-runtime's ATTACH and INSTALL do. */
async function callIn(cdp, sessionId, executionContextId, entry, args) {
  const reply = await cdp.send(
    'Runtime.callFunctionOn',
    {
      executionContextId,
      functionDeclaration: entry,
      arguments: args.map((value) => ({ value })),
      returnByValue: false,
    },
    sessionId,
  )
  assert.equal(
    reply.exceptionDetails,
    undefined,
    JSON.stringify(reply.exceptionDetails),
  )
  return reply.result
}

test('forms glue preserves Playwright values, state checks, targeting and errors', {
  skip: browserBinary ? false : 'NEO_E2E_BROWSER is unset',
  timeout: 180_000,
}, async () => {
  const cleanups = []
  try {
    const browser = await launchBrowser(cleanups, browserBinary)
    const cdp = await connectCdp(cleanups, browser.port)
    const html = `<title>Forms glue</title>
      <label for="text" id="label">Name</label><input id="text" value="hello">
      <input id="other" value="other"><input id="readonly" readonly>
      <input id="hidden" hidden><input id="inert" inert>
      <input id="date" type="date"><input id="checkbox" type="checkbox" checked>
      <input id="radio" type="radio"><div id="wrong">not an input</div>
      <select id="select" multiple><option value="a">Alpha</option><option value="b">Beta</option><option value="c" disabled>Disabled</option></select>
      <script>globalThis.marked = []; document.addEventListener('__playwright_reset_targets__', () => marked = []); document.addEventListener('__playwright_mark_target__', e => marked.push(e.target.id));</script>`
    const { targetId } = await cdp.send('Target.createTarget', {
      url: `data:text/html,${encodeURIComponent(html)}`,
    })
    const session = await attachTo(cdp, targetId)
    await waitForTitle(cdp, session, 'Forms glue')
    const context = await isolatedWorld(cdp, session, targetId)
    const glue = await callIn(cdp, session, context, bundle.install, [
      key,
      bundle.specSha256,
      initializer,
    ])
    const page = (name, params, handles = []) =>
      pageCall(cdp, session, glue, name, params, handles)
    const resolve = async (selector) =>
      (
        await handleCall(cdp, session, glue, 'resolve', {
          selector,
          strict: true,
        })
      ).element
    const text = await resolve('#text')
    const date = await resolve('#date')
    const checkbox = await resolve('#checkbox')
    const radio = await resolve('#radio')
    const select = await resolve('#select')
    const wrong = await resolve('#wrong')
    assert.deepEqual(
      await page('fill', { value: 'abc', force: false, frameVisible: true }, [
        text,
      ]),
      { value: { outcome: 'needsinput' } },
    )
    assert.deepEqual(
      await page(
        'fill',
        { value: '2025-05-04', force: false, frameVisible: true },
        [date],
      ),
      { value: { outcome: 'done' } },
    )
    assert.deepEqual(
      await page(
        'fill',
        { value: '2026-01-02', force: false, frameVisible: false },
        [date],
      ),
      { value: { outcome: { missingState: 'visible' } } },
      'hidden frame owners block fill even when its control is visible in this document',
    )
    assert.equal(
      (await page('inputValue', { selector: '#date', strict: true })).value
        .matched.value,
      '2025-05-04',
    )
    for (const [selector, state] of [
      ['#readonly', 'editable'],
      ['#hidden', 'visible'],
      ['#inert', 'non-inert'],
    ]) {
      const element = await resolve(selector)
      assert.deepEqual(
        await page('fill', { value: 'x', force: false, frameVisible: true }, [
          element,
        ]),
        { value: { outcome: { missingState: state } } },
      )
    }
    assert.deepEqual(
      await page('fill', { value: 'x', force: true, frameVisible: true }, [
        await resolve('#hidden'),
      ]),
      { value: { outcome: 'needsinput' } },
    )
    assert.deepEqual(
      await page(
        'selectOptions',
        {
          options: [{ label: 'Beta' }, { index: 0 }],
          force: false,
          frameVisible: true,
        },
        [select],
      ),
      { value: { values: ['a', 'b'] } },
    )
    assert.deepEqual(
      await page(
        'selectOptions',
        { options: [{ value: 'a' }], force: false, frameVisible: false },
        [select],
      ),
      { value: { outcome: { missingState: 'visible' } } },
      'hidden frame owners block selecting options without changing the current selection',
    )
    assert.deepEqual(
      await page(
        'selectOptions',
        { options: [{ value: 'missing' }], force: false, frameVisible: true },
        [select],
      ),
      { value: { outcome: 'error:optionsnotfound' } },
    )
    assert.deepEqual(
      await page(
        'selectOptions',
        { options: [{ value: 'c' }], force: false, frameVisible: true },
        [select],
      ),
      { value: { outcome: 'error:optionnotenabled' } },
    )
    const selected = await cdp.send(
      'Runtime.callFunctionOn',
      {
        objectId: select.objectId,
        functionDeclaration:
          'function () { return [...this.selectedOptions].map(o => o.value) }',
        returnByValue: true,
      },
      session,
    )
    assert.deepEqual(
      selected.result.value,
      ['a', 'b'],
      'selection failures leave earlier values intact',
    )
    assert.deepEqual(
      await page('elementState', { state: 'checked' }, [checkbox]),
      { value: { matches: true, received: 'checked', isRadio: false } },
    )
    assert.deepEqual(
      await page('elementState', { state: 'unchecked' }, [radio]),
      { value: { matches: true, received: 'unchecked', isRadio: true } },
    )
    assert.equal(
      (await page('elementState', { state: 'checked' }, [wrong])).exception
        .description,
      'Error: Not a checkbox or radio button',
    )
    assert.deepEqual(
      await page('focusElement', { resetSelectionIfNotFocused: true }, [text]),
      { value: { outcome: 'done' } },
    )
    assert.equal(
      (await page('inputValue', { selector: '#label', strict: true })).value
        .matched.value,
      'hello',
      'inputValue follows labels',
    )
    assert.deepEqual(
      await page('inputValue', { selector: '#missing', strict: true }),
      { value: {} },
    )
    assert.equal(
      (await page('inputValue', { selector: '#wrong', strict: true })).exception
        .description,
      'Error: Node is not an <input>, <textarea> or <select> element',
    )
    await page('inputValue', { selector: '#text, #other', strict: false })
    const marked = await cdp.send(
      'Runtime.evaluate',
      { expression: 'globalThis.marked', returnByValue: true },
      session,
    )
    assert.deepEqual(
      marked.result.value,
      ['text'],
      'inputValue marks only the first match',
    )
    await cdp.send(
      'Runtime.callFunctionOn',
      {
        objectId: text.objectId,
        functionDeclaration: 'function () { this.remove() }',
      },
      session,
    )
    assert.deepEqual(
      await page('fill', { value: 'x', force: false, frameVisible: true }, [
        text,
      ]),
      {
        value: { outcome: 'error:notconnected' },
      },
    )
    assert.deepEqual(
      await page('fill', { value: 'x', force: true, frameVisible: true }, [
        text,
      ]),
      { value: { outcome: 'needsinput' } },
      "forced fill preserves Playwright's absence of a connectedness check",
    )
    assert.deepEqual(
      await page('focusElement', { resetSelectionIfNotFocused: true }, [text]),
      { value: { outcome: 'error:notconnected' } },
    )
    assert.deepEqual(await page('elementState', { state: 'checked' }, [text]), {
      value: { matches: false, received: 'error:notconnected' },
    })
    assert.deepEqual(
      await page(
        'selectOptions',
        { options: [], force: false, frameVisible: true },
        [select],
      ),
      { value: { values: [] } },
    )
  } finally {
    for (const cleanup of cleanups.reverse()) await cleanup()
  }
})

/** One page call by the convention in dist/call.js: `{value}` or `{exception}`. */
async function pageCall(cdp, sessionId, glue, name, params, handles = []) {
  const reply = await cdp.send(
    'Runtime.callFunctionOn',
    {
      objectId: glue.objectId,
      functionDeclaration: bundle.call,
      arguments: [
        { value: name },
        { value: params },
        ...handles.map((handle) => ({ objectId: handle.objectId })),
      ],
      returnByValue: true,
      awaitPromise: true,
    },
    sessionId,
  )
  if (reply.exceptionDetails)
    return { exception: reply.exceptionDetails.exception }
  return { value: reply.result.value }
}

/** A page call whose result crosses by value, with handle arguments after the params. */
async function byValueCall(cdp, sessionId, glue, name, params, handles) {
  const reply = await cdp.send(
    'Runtime.callFunctionOn',
    {
      objectId: glue.objectId,
      functionDeclaration: bundle.call,
      arguments: [
        { value: name },
        { value: params },
        ...handles.map((handle) => ({ objectId: handle.objectId })),
      ],
      returnByValue: true,
      awaitPromise: true,
    },
    sessionId,
  )
  assert.equal(
    reply.exceptionDetails,
    undefined,
    JSON.stringify(reply.exceptionDetails),
  )
  return reply.result.value
}

/**
 * A page call whose result holds a handle, as neo-runtime makes it: by reference into an object
 * group, then one `Runtime.getProperties`. Returns each own property's remote object by name.
 */
async function handleCall(cdp, sessionId, glue, name, params, handles = []) {
  const reply = await cdp.send(
    'Runtime.callFunctionOn',
    {
      objectId: glue.objectId,
      functionDeclaration: bundle.call,
      arguments: [
        { value: name },
        { value: params },
        ...handles.map((handle) => ({ objectId: handle.objectId })),
      ],
      returnByValue: false,
      awaitPromise: true,
      objectGroup: 'neo-load-test',
    },
    sessionId,
  )
  assert.equal(
    reply.exceptionDetails,
    undefined,
    JSON.stringify(reply.exceptionDetails),
  )
  const { result } = await cdp.send(
    'Runtime.getProperties',
    { objectId: reply.result.objectId, ownProperties: true },
    sessionId,
  )
  return Object.fromEntries(
    result
      .filter((property) => property.name !== '__proto__')
      .map((property) => [property.name, property.value]),
  )
}

async function sameObject(cdp, sessionId, a, b) {
  const reply = await cdp.send(
    'Runtime.callFunctionOn',
    {
      objectId: a.objectId,
      functionDeclaration: 'function (other) { return this === other }',
      arguments: [{ objectId: b.objectId }],
      returnByValue: true,
    },
    sessionId,
  )
  return reply.result.value
}

async function attachTo(cdp, targetId) {
  const { sessionId } = await cdp.send('Target.attachToTarget', {
    targetId,
    flatten: true,
  })
  return sessionId
}

/** Neo's world, found by name: a second session gets the same world, not a new one. */
async function isolatedWorld(cdp, sessionId, targetId) {
  const { executionContextId } = await cdp.send(
    'Page.createIsolatedWorld',
    { frameId: targetId, worldName: WORLD_NAME, grantUniveralAccess: true },
    sessionId,
  )
  return executionContextId
}

/** Waits until the data: document has committed, so the world belongs to it. */
async function waitForTitle(cdp, sessionId, title) {
  const deadline = Date.now() + 30_000
  for (;;) {
    const reply = await cdp.send(
      'Runtime.evaluate',
      { expression: 'document.title', returnByValue: true },
      sessionId,
    )
    if (reply.result.value === title) return
    assert.ok(Date.now() < deadline, `the page never showed "${title}"`)
    await sleep(20)
  }
}

/**
 * Launches a headless BrowserOS neo on a throwaway profile. Its cleanup ends the browser, then
 * checks nothing started from that profile is still running.
 */
async function launchBrowser(cleanups, binary) {
  const profile = mkdtempSync(join(tmpdir(), 'neo-injected-'))
  const child = spawn(
    binary,
    [
      '--headless=new',
      // Otherwise the browser starts a browseros-claw-server sidecar that outlives it.
      '--disable-browseros-server',
      // Port 0 lets the browser pick a free port and write it to DevToolsActivePort.
      '--remote-debugging-port=0',
      `--user-data-dir=${profile}`,
      '--no-first-run',
      'about:blank',
    ],
    { stdio: 'ignore' },
  )
  cleanups.push(async () => {
    const helpers = processIds(['-P', String(child.pid)])
    const exited = once(child, 'exit')
    child.kill('SIGTERM')
    const forced = setTimeout(() => child.kill('SIGKILL'), 5_000)
    await exited
    clearTimeout(forced)
    // Helpers notice their parent is gone on their own, usually within a second.
    const deadline = Date.now() + 5_000
    let lingering = [...helpers, ...processIds(['-f', profile])]
    while (lingering.length > 0 && Date.now() < deadline) {
      await sleep(100)
      lingering = lingering.filter(isAlive)
    }
    for (const pid of lingering) process.kill(Number(pid), 'SIGKILL')
    rmSync(profile, { recursive: true, force: true })
    assert.deepEqual(lingering, [], 'browser processes outlived the browser')
  })

  const deadline = Date.now() + 20_000
  for (;;) {
    try {
      const port = Number(
        readFileSync(join(profile, 'DevToolsActivePort'), 'utf8').split(
          '\n',
        )[0],
      )
      if (port > 0) return { port }
    } catch {}
    assert.ok(Date.now() < deadline, 'the browser never opened its CDP port')
    await sleep(50)
  }
}

function processIds(pgrepArgs) {
  try {
    return execFileSync('pgrep', pgrepArgs, { encoding: 'utf8' })
      .split('\n')
      .filter(Boolean)
  } catch {
    return []
  }
}

function isAlive(pid) {
  try {
    process.kill(Number(pid), 0)
    return true
  } catch {
    return false
  }
}

/** A minimal CDP client on the browser's WebSocket. */
async function connectCdp(cleanups, port) {
  const version = await fetch(`http://127.0.0.1:${port}/json/version`)
  const { webSocketDebuggerUrl } = await version.json()
  const socket = new WebSocket(webSocketDebuggerUrl)
  await once(socket, 'open')
  cleanups.push(() => socket.close())
  const pending = new Map()
  let nextId = 1
  socket.addEventListener('message', ({ data }) => {
    const message = JSON.parse(data)
    const call = pending.get(message.id)
    if (!call) return
    pending.delete(message.id)
    if (message.error) {
      call.reject(new Error(`${call.method}: ${message.error.message}`))
    } else {
      call.resolve(message.result)
    }
  })
  return {
    send(method, params = {}, sessionId) {
      return new Promise((resolve, reject) => {
        const id = nextId++
        pending.set(id, { method, resolve, reject })
        socket.send(JSON.stringify({ id, method, params, sessionId }))
      })
    },
  }
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms))
}
