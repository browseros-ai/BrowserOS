/**
 * Live smoke for the page asset, over raw CDP from Node against a real BrowserOS (or
 * Chromium) on BROWSEROS_CDP_PORT (default 9005). It installs the probe the way the crate
 * will, in an isolated world named browseros-sdk:
 *
 *   (() => { <core's seal_source()>; if (!globalThis.__neoProbe) { <assets/injected.js> }
 *            return globalThis.__neoProbe.boot({ frameSeq, epoch }) })()
 *
 * then sends requests with the CALL_FUNCTION from tests/data/probe-protocol.json, and checks
 * what the probe must guarantee before Rust builds on it. Chunks are parsed by the prelude
 * asset's vendored selectorParser, as the agent runtime will parse them. The seal source is
 * core's seal.js with rules.json substituted, exactly what seal_source() returns.
 *
 *   BROWSEROS_CDP_PORT=9361 node scripts/probe-smoke.mjs
 *
 * It serves its own fixture pages on 127.0.0.1, opens and closes its own tabs, and exits 1 on
 * the first failed check.
 */
import { readFileSync } from 'node:fs'
import http from 'node:http'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { runInNewContext } from 'node:vm'

const here = dirname(fileURLToPath(import.meta.url))
const agentRoot = resolve(here, '../../../..')
const crate = resolve(agentRoot, 'crates/neo-sdk/neo-playwright')
const sealDir = resolve(agentRoot, 'crates/browseros-core/src/sensitive_fields')

const ASSET = readFileSync(resolve(crate, 'assets/injected.js'), 'utf8')
const PRELUDE = readFileSync(resolve(crate, 'assets/prelude.js'), 'utf8')
const PROTOCOL = JSON.parse(
  readFileSync(resolve(crate, 'tests/data/probe-protocol.json'), 'utf8'),
)
const SEAL = readFileSync(resolve(sealDir, 'seal.js'), 'utf8').replace(
  '__RULES__',
  readFileSync(resolve(sealDir, 'rules.json'), 'utf8').trim(),
)
const PORT = Number(process.env.BROWSEROS_CDP_PORT ?? 9005)
const WORLD = 'browseros-sdk'

const isomorphic = (() => {
  const context = {}
  runInNewContext(PRELUDE, context)
  return context.__neoIsomorphic
})()
const chunk = (selector) => ({
  chunk: isomorphic.selectorParser.parseSelector(selector),
})

const install = ({ frameSeq, epoch }) =>
  `(() => { ${SEAL};\n if (!globalThis.__neoProbe) { ${ASSET}\n }\n return globalThis.__neoProbe.boot(${JSON.stringify({ frameSeq, epoch })}) })()`

const BODY = `
  <h1>Pay</h1>
  <button>Go</button><button>Stop</button>
  <label>Password <input id="pw" type="password" value="hunter2-attribute"></label>
  <label>Name <input id="name" value="Ada"></label>
  <div id="outside">outside</div>
  <section id="checkout"><iframe id="frame" src="/child.html" width="300" height="80"></iframe></section>`
const PAGES = {
  '/plain.html': { body: `<!doctype html><title>plain</title>${BODY}` },
  '/csp.html': {
    headers: { 'content-security-policy': "script-src 'self'" },
    body: `<!doctype html><title>csp</title>${BODY}<script src="/page.js"></script><script>window.inlineRan = true</script>`,
  },
  '/child.html': {
    body: '<!doctype html><title>child</title><p>child frame</p>',
  },
  '/page.js': {
    headers: { 'content-type': 'text/javascript' },
    body: "window.pageScriptRan = true; window.pageEval = (() => { try { eval('1'); return 'ran' } catch (error) { return error.name } })()",
  },
}

function serve() {
  const server = http.createServer((request, response) => {
    const page = PAGES[new URL(request.url, 'http://x').pathname]
    if (!page) {
      response.writeHead(404).end()
      return
    }
    response.writeHead(200, { 'content-type': 'text/html', ...page.headers })
    response.end(page.body)
  })
  return new Promise((done) =>
    server.listen(0, '127.0.0.1', () =>
      done({
        url: (path) => `http://127.0.0.1:${server.address().port}${path}`,
        close: () => server.close(),
      }),
    ),
  )
}

/** One browser WebSocket with flattened target sessions. */
class Cdp {
  static async connect(port) {
    const version = await (
      await fetch(`http://127.0.0.1:${port}/json/version`)
    ).json()
    const cdp = new Cdp()
    cdp.ws = new WebSocket(version.webSocketDebuggerUrl)
    cdp.next = 0
    cdp.pending = new Map()
    await new Promise((open, fail) => {
      cdp.ws.onopen = open
      cdp.ws.onerror = fail
    })
    cdp.ws.onmessage = ({ data }) => {
      const message = JSON.parse(data)
      const waiter = cdp.pending.get(message.id)
      if (!waiter) return
      cdp.pending.delete(message.id)
      if (message.error)
        waiter.fail(new Error(`${message.error.message} (${waiter.method})`))
      else waiter.done(message.result)
    }
    return cdp
  }

  send(method, params = {}, sessionId) {
    const id = ++this.next
    this.ws.send(JSON.stringify({ id, method, params, sessionId }))
    return new Promise((done, fail) =>
      this.pending.set(id, { done, fail, method }),
    )
  }

  close() {
    this.ws.close()
  }
}

/** A tab attached on one connection, with the calls this smoke makes in it. */
async function openTab(cdp, url) {
  const { targetId } = await cdp.send('Target.createTarget', {
    url: 'about:blank',
  })
  const { sessionId } = await cdp.send('Target.attachToTarget', {
    targetId,
    flatten: true,
  })
  const tab = {
    targetId,
    send: (method, params) => cdp.send(method, params, sessionId),
    async evaluate(expression, contextId, extra = {}) {
      const result = await tab.send('Runtime.evaluate', {
        expression,
        returnByValue: true,
        ...(contextId ? { contextId } : {}),
        ...extra,
      })
      if (result.exceptionDetails)
        throw new Error(
          result.exceptionDetails.exception?.description ??
            result.exceptionDetails.text,
        )
      return result.result.value
    },
    async mainFrame() {
      const { frameTree } = await tab.send('Page.getFrameTree')
      return frameTree.frame.id
    },
    async world() {
      const { executionContextId } = await tab.send(
        'Page.createIsolatedWorld',
        {
          frameId: await tab.mainFrame(),
          worldName: WORLD,
        },
      )
      return executionContextId
    },
    async install(options) {
      const contextId = await tab.world()
      return {
        contextId,
        described: await tab.evaluate(install(options), contextId),
      }
    },
    async call(contextId, request) {
      const { result, exceptionDetails } = await tab.send(
        'Runtime.callFunctionOn',
        {
          functionDeclaration: PROTOCOL.callFunction,
          executionContextId: contextId,
          arguments: [{ value: request }],
          returnByValue: true,
          awaitPromise: true,
        },
      )
      if (exceptionDetails) throw new Error(exceptionDetails.text)
      return result.value
    },
  }
  await tab.send('Page.enable')
  await tab.send('Page.navigate', { url })
  for (let attempt = 0; attempt < 100; attempt++) {
    const ready = await tab
      .evaluate(
        `document.readyState === 'complete' && location.href === ${JSON.stringify(url)}`,
      )
      .catch(() => false)
    if (ready) return tab
    await new Promise((wait) => setTimeout(wait, 50))
  }
  throw new Error(`${url} did not load within 5 s`)
}

let checks = 0
function check(ok, what, detail) {
  checks++
  if (!ok) {
    console.log(
      `FAIL ${what}${detail === undefined ? '' : `: ${JSON.stringify(detail)}`}`,
    )
    process.exit(1)
  }
  console.log(
    `ok ${checks} - ${what}${detail === undefined ? '' : `: ${JSON.stringify(detail)}`}`,
  )
}

const resolveRequest = (target, want = 'element', strictness = 'strict') => ({
  op: 'resolve',
  target,
  strictness,
  want,
})

async function plainPage(cdp, server, epoch) {
  const url = server.url('/plain.html')
  const tab = await openTab(cdp, url)
  try {
    const { contextId, described } = await tab.install({ frameSeq: 0, epoch })
    check(
      described.kind === 'described' &&
        described.protocol === PROTOCOL.protocol &&
        described.seq === 0 &&
        described.url === url,
      'seal, asset and boot install in the browseros-sdk world in one script',
      described,
    )
    const sealed = await tab.evaluate(
      'typeof __clawSeal + " " + typeof __neoProbe',
      contextId,
    )
    check(
      sealed === 'object object',
      'the sealed world holds the seal and the probe',
      sealed,
    )
    const describe = await tab.call(contextId, { op: 'describe' })
    check(describe.seq === 0, 'describe returns the seq', describe)
    const pageSees = await tab.evaluate(
      'typeof __neoProbe + " " + typeof __clawSeal',
    )
    check(
      pageSees === 'undefined undefined',
      "the page's main world cannot see __neoProbe or the seal",
      pageSees,
    )

    await tab.evaluate("document.getElementById('pw').value = 'typed-secret'")
    const raw = await tab.evaluate(
      "[document.getElementById('pw').value, document.getElementById('pw').getAttribute('value')]",
    )
    check(
      raw[0] === 'typed-secret' && raw[1] === 'hunter2-attribute',
      'the main world reads the password raw',
      raw,
    )
    const resolved = await tab.call(contextId, resolveRequest(chunk('#pw')))
    check(
      resolved.kind === 'element' &&
        resolved.sensitive === 'password' &&
        resolved.preview.includes('value="••••"') &&
        !resolved.preview.includes('hunter2'),
      'the probe resolves the password field as sensitive and previews it masked',
      resolved,
    )
    const inWorld = await tab.evaluate(
      "[document.getElementById('pw').value, document.getElementById('name').value]",
      contextId,
    )
    check(
      inWorld[0] === '••••' && inWorld[1] === 'Ada',
      "the probe's world reads •••• for the password and the plain field as is",
      inWorld,
    )
    const sweep = await tab.call(contextId, { op: 'sweep' })
    check(
      sweep.kind === 'sweep' &&
        sweep.values.includes('typed-secret') &&
        !sweep.values.includes('Ada'),
      "sweep hands Rust the password's raw value for its mask, and only that",
      { kind: sweep.kind, count: sweep.values.length },
    )
    await tab.evaluate("document.getElementById('pw').focus()")
    const focused = await tab.call(contextId, { op: 'focused' })
    check(
      focused.sensitive === true && focused.iframe === null,
      'focused reports a focused password field as sensitive',
      focused,
    )

    const frame = await tab.call(
      contextId,
      resolveRequest(chunk('#frame'), 'frame'),
    )
    check(
      frame.kind === 'frame' && frame.key === '#1' && frame.visible === true,
      'a frame hop pins the iframe and registers its key in the ref map',
      frame,
    )
    const button = await tab.call(
      contextId,
      resolveRequest(chunk('internal:role=button[name="Go"i]')),
    )
    check(
      button.kind === 'element',
      'a role selector resolves strictly',
      button,
    )
    const strict = await tab.call(
      contextId,
      resolveRequest(chunk('internal:role=button')),
    )
    check(
      strict.kind === 'strict' &&
        strict.message.startsWith(
          "strict mode violation: getByRole('button') resolved to 2 elements:",
        ),
      "two buttons are Playwright's strict mode violation",
      strict.message.split('\n')[0],
    )

    const same = await tab.evaluate(install({ frameSeq: 5, epoch }), contextId)
    const pinAfterSame = await tab.call(
      contextId,
      resolveRequest({ pin: button.pin }),
    )
    const keyAfterSame = await tab.call(
      contextId,
      resolveRequest(chunk('aria-ref=#1')),
    )
    check(
      same.seq === 0 &&
        pinAfterSame.kind === 'element' &&
        keyAfterSame.kind === 'element',
      'a second boot with the same epoch is a no-op: same seq, pins and keys live',
      { seq: same.seq, pin: pinAfterSame.kind, key: keyAfterSame.kind },
    )
    const rekeyed = await tab.evaluate(
      install({ frameSeq: 347, epoch: `${epoch}-next` }),
      contextId,
    )
    const pinAfter = await tab.call(
      contextId,
      resolveRequest({ pin: button.pin }),
    )
    const keyAfter = await tab.call(
      contextId,
      resolveRequest(chunk('aria-ref=#1')),
    )
    const freshFrame = await tab.call(
      contextId,
      resolveRequest(chunk('#frame'), 'frame'),
    )
    check(
      rekeyed.seq === 347 &&
        pinAfter.kind === 'detached' &&
        keyAfter.kind === 'none' &&
        keyAfter.staleRef === '#1' &&
        freshFrame.key === '#2' &&
        freshFrame.pin > frame.pin,
      'another epoch re-keys: new seq, old pins detached, old keys stale, numbers never reused',
      {
        seq: rekeyed.seq,
        pin: pinAfter,
        key: keyAfter,
        newKey: freshFrame.key,
      },
    )

    const hint = await tab.call(contextId, resolveRequest(chunk('ref=e5')))
    check(
      hint.kind === 'error' &&
        hint.class === 'selector' &&
        hint.message ===
          "Unknown selector engine \"ref\". Use page.getByRef('e5') or 'aria-ref=e5'.",
      "the unknown engine ref= gets Neo's hint",
      hint,
    )
    return tab
  } catch (error) {
    await cdp.send('Target.closeTarget', { targetId: tab.targetId })
    throw error
  }
}

async function cspPage(cdp, server, epoch) {
  const url = server.url('/csp.html')
  const tab = await openTab(cdp, url)
  try {
    const page = await tab.evaluate(
      '[window.pageScriptRan === true, window.inlineRan === true, window.pageEval]',
    )
    check(
      page[0] === true && page[1] === false && page[2] === 'EvalError',
      "the page's CSP is in force: its own script ran, its inline script and its eval did not",
      page,
    )
    const contextId = await tab.world()
    const tryEval =
      "(() => { try { return eval('1 + 1') === 2 ? 'allowed' : 'wrong' } catch (error) { return error.name } })()"
    const viaCall = await tab.send('Runtime.callFunctionOn', {
      functionDeclaration: `function () { return ${tryEval} }`,
      executionContextId: contextId,
      returnByValue: true,
    })
    const evalIn = {
      'Runtime.evaluate': await tab.evaluate(tryEval, contextId),
      'Runtime.evaluate, allowUnsafeEvalBlockedByCSP false': await tab.evaluate(
        tryEval,
        contextId,
        { allowUnsafeEvalBlockedByCSP: false },
      ),
      'Runtime.callFunctionOn': viaCall.result.value,
    }
    console.log(
      `   measured: eval in the ${WORLD} world under this CSP: ${JSON.stringify(evalIn)}`,
    )
    // The install needs no eval, so it runs with DevTools' eval exemption off as well.
    const described = await tab.evaluate(
      install({ frameSeq: 0, epoch }),
      contextId,
      {
        allowUnsafeEvalBlockedByCSP: false,
      },
    )
    check(
      described.kind === 'described' && described.seq === 0,
      "a strict-CSP page (script-src 'self', no unsafe-eval) still installs, without DevTools' eval exemption",
      described,
    )
    const frame = await tab.call(
      contextId,
      resolveRequest(chunk('#frame'), 'frame'),
    )
    const byRef = await tab.call(
      contextId,
      resolveRequest(chunk('aria-ref=#1')),
    )
    const scoped = await tab.call(
      contextId,
      resolveRequest(chunk('#checkout >> aria-ref=#1')),
    )
    const outside = await tab.call(
      contextId,
      resolveRequest(chunk('#outside >> aria-ref=#1')),
    )
    check(
      frame.key === '#1' &&
        byRef.kind === 'element' &&
        byRef.preview.startsWith('<iframe') &&
        scoped.kind === 'element' &&
        outside.kind === 'none' &&
        outside.staleRef === undefined,
      "under that CSP an aria-ref resolves through Neo's engine, only inside its root",
      { byRef: byRef.preview, scoped: scoped.kind, outside },
    )
    await tab.evaluate("document.getElementById('frame').remove()")
    const removed = await tab.call(
      contextId,
      resolveRequest(chunk('aria-ref=#1')),
    )
    check(
      removed.kind === 'none' && removed.staleRef === '#1',
      'a ref whose element left the page fails at once as stale',
      removed,
    )
    return tab
  } catch (error) {
    await cdp.send('Target.closeTarget', { targetId: tab.targetId })
    throw error
  }
}

/**
 * A second DevTools connection stands in for a restarted claw-sdk on a tab that stayed open.
 * Whether it shares the first connection's browseros-sdk world is measured and printed; either
 * way its install, with its own epoch, must leave nothing the first one handed out usable.
 */
async function secondConnection(tab, epoch) {
  const other = await Cdp.connect(PORT)
  try {
    const { sessionId } = await other.send('Target.attachToTarget', {
      targetId: tab.targetId,
      flatten: true,
    })
    const evaluate = async (expression, contextId) => {
      const { result, exceptionDetails } = await other.send(
        'Runtime.evaluate',
        { expression, contextId, returnByValue: true },
        sessionId,
      )
      if (exceptionDetails) throw new Error(exceptionDetails.text)
      return result.value
    }
    const { frameTree } = await other.send('Page.getFrameTree', {}, sessionId)
    const { executionContextId } = await other.send(
      'Page.createIsolatedWorld',
      { frameId: frameTree.frame.id, worldName: WORLD },
      sessionId,
    )
    const seen = await evaluate(
      'typeof __neoProbe + " " + typeof __clawSeal',
      executionContextId,
    )
    console.log(
      `   measured: a second CDP connection's ${WORLD} world ${seen === 'object object' ? 'is the same world: it already holds the probe and the seal' : `is a new world (${seen})`}`,
    )
    const described = await evaluate(
      install({ frameSeq: 512, epoch: `${epoch}-restart` }),
      executionContextId,
    )
    const { result } = await other.send(
      'Runtime.callFunctionOn',
      {
        functionDeclaration: PROTOCOL.callFunction,
        executionContextId,
        arguments: [{ value: resolveRequest(chunk('aria-ref=#2')) }],
        returnByValue: true,
      },
      sessionId,
    )
    check(
      described.seq === 512 &&
        result.value.kind === 'none' &&
        result.value.staleRef === '#2',
      "a restarted claw-sdk's install takes its own seq, and refs from before it are stale",
      { seq: described.seq, oldRef: result.value },
    )
  } finally {
    other.close()
  }
}

const server = await serve()
const cdp = await Cdp.connect(PORT)
const epoch = `smoke-${Date.now()}`
try {
  const plain = await plainPage(cdp, server, epoch)
  await secondConnection(plain, epoch)
  await cdp.send('Target.closeTarget', { targetId: plain.targetId })
  const csp = await cspPage(cdp, server, epoch)
  await cdp.send('Target.closeTarget', { targetId: csp.targetId })
  console.log(`all ${checks} checks passed on CDP port ${PORT}`)
} finally {
  cdp.close()
  server.close()
}
