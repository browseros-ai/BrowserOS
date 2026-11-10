/** Neo: Exercise object transport against the committed SDK without a browser. */
import assert from 'node:assert/strict'
import fs from 'node:fs'
import test from 'node:test'
import vm from 'node:vm'
import { codecFactory } from './codec.mjs'
import { nativeScript } from './native.mjs'

const bundle = fs.readFileSync(
  new URL('../../neo-quickjs-injected/dist/sdk.js', import.meta.url),
  'utf8',
)
const codec = codecFactory({ identify: () => undefined, resolve: (ref) => ref })

async function native(t) {
  const replies = []
  const waiters = []
  let receive
  const sent = []
  const realm = vm.createContext({
    __neo_runtime: {
      get: () => 'null',
      subscribe() {},
      drain: () => '[]',
      nextEvents: () => new Promise(() => {}),
      callWithEvents: async (text) => {
        const message = JSON.parse(text)
        sent.push(message)
        return {
          reply: JSON.stringify({ id: message.id, result: {} }),
          events: 0,
        }
      },
    },
    __neo_test_host: {
      reply: (text) => {
        const frame = JSON.parse(text)
        if (waiters.length) waiters.shift()(frame)
        else replies.push(frame)
      },
      receive: () =>
        new Promise((resolve) => {
          receive = resolve
        }),
    },
  })
  const names = [...bundle.matchAll(/^class (\w+)/gm)].map((match) => match[1])
  const sdk = vm.runInContext(
    bundle.replace(
      'return { createRun };',
      `return { createRun, ${names.join(',')} };`,
    ),
    realm,
  )
  const run = sdk.createRun({
    page: 'page@unit',
    mainFrame: 'frame@unit',
    context: 'context@unit',
    browser: 'browser@unit',
    url: 'about:blank',
    testIdAttribute: 'data-testid',
    lineOffset: 0,
  })
  Object.assign(realm, run)
  const loop = vm.runInContext(
    `(async () => { ${nativeScript('unit')} })()`,
    realm,
  )
  const next = () =>
    replies.length
      ? Promise.resolve(replies.shift())
      : new Promise((resolve) => waiters.push(resolve))
  const ready = await next()
  let id = 0
  const command = async (fields) => {
    receive(JSON.stringify({ id: ++id, ...fields }))
    return next()
  }
  t.after(async () => {
    await command({ op: 'finish' })
    await loop
  })
  const get = (object, member) =>
    command({ op: 'get', object, member: codec.units(member) })
  const invoke = async (object, member, args = []) => {
    const callable = await get(object, member)
    assert.equal(callable.kind, 'callable')
    const frame = await command({
      op: 'invoke',
      object,
      member: codec.units(member),
      function: callable.function.id,
      args: codec.encode(args),
    })
    return frame.kind === 'promise' ? next() : frame
  }
  return { sdk, run, ready, sent, get, invoke, command }
}

test('tests that SDK return identities survive round trips and choose the nearest public class', async (t) => {
  const { sdk, run, ready, get, invoke } = await native(t)
  const request = new sdk.Request(
    'request@unit',
    { url: 'https://example.test/', headers: [] },
    run.page.mainFrame(),
    null,
    null,
  )
  const response = new sdk.Response(
    'response@unit',
    { url: 'https://example.test/', status: 200, headers: [] },
    request,
    null,
  )
  const handle = new sdk.JSHandle(null, 'handle@unit', { preview: 'object' })
  const element = new sdk.ElementHandle(null, 'element@unit', {
    preview: 'element',
  })
  class DerivedElement extends sdk.ElementHandle {}
  const derived = new DerivedElement(null, 'derived@unit', {
    preview: 'derived',
  })
  const page = ready.roots.page.ref
  const refs = []
  for (const [value, family] of [
    [response, 'Response'],
    [request, 'Request'],
    [handle, 'JSHandle'],
    [element, 'ElementHandle'],
    [derived, 'ElementHandle'],
  ]) {
    run.page.sample = value
    const first = await get(page, 'sample')
    assert.equal(first.kind, 'value', JSON.stringify(first))
    assert.equal(first.value.ref.family, family)
    assert.deepEqual((await get(page, 'sample')).value, first.value)
    refs.push(first.value.ref)
  }
  assert.equal(new Set(refs.map((ref) => ref.id)).size, refs.length)
  assert.equal(codec.decode((await invoke(refs[0], 'status')).value), 200)
  assert.deepEqual((await invoke(refs[0], 'request')).value.ref, refs[1])
  assert.deepEqual((await invoke(refs[3], 'asElement')).value.ref, refs[3])
})

test('tests that accessor namespaces retain aliases and invoke real SDK methods', async (t) => {
  const { run, ready, sent, get, invoke } = await native(t)
  const page = ready.roots.page.ref
  for (const [member, family] of [
    ['keyboard', 'Keyboard'],
    ['mouse', 'Mouse'],
    ['touchscreen', 'Touchscreen'],
    ['clock', 'Clock'],
    ['localStorage', 'WebStorage'],
  ]) {
    const first = await get(page, member)
    assert.equal(first.kind, 'value', JSON.stringify(first))
    assert.equal(first.value.ref.family, family)
    assert.deepEqual((await get(page, member)).value, first.value)
  }
  assert.deepEqual(
    (await get(page, 'clock')).value,
    (await get(ready.roots.context.ref, 'clock')).value,
  )
  const keyboard = (await get(page, 'keyboard')).value.ref
  const pressed = await invoke(keyboard, 'press', ['Enter'])
  assert.equal(pressed.ok, true)
  assert.equal(sent[0].method, 'keyboardPress')
  assert.equal(sent[0].params.key, 'Enter')
  Object.defineProperty(run.page, 'namespace', { get: () => run.page.keyboard })
  assert.deepEqual((await get(page, 'namespace')).value.ref, keyboard)
})

test('tests that ready inventories every public SDK family before it is returned', async (t) => {
  const { ready } = await native(t)
  const expected = [
    'APIRequest',
    'APIResponse',
    'Browser',
    'BrowserContext',
    'BrowserType',
    'Clock',
    'ConsoleMessage',
    'Debugger',
    'Dialog',
    'Disposable',
    'DisposableStub',
    'Download',
    'ElementHandle',
    'FileChooser',
    'Frame',
    'FrameLocator',
    'JSHandle',
    'Keyboard',
    'Locator',
    'Mouse',
    'Page',
    'Playwright',
    'Request',
    'Response',
    'Route',
    'Selectors',
    'Touchscreen',
    'WebError',
    'WebStorage',
  ]
  assert.deepEqual(Object.keys(ready.capabilities).sort(), expected.sort())
  for (const [family, member] of [
    ['Response', 'status'],
    ['Request', 'url'],
    ['Keyboard', 'press'],
    ['ElementHandle', 'evaluate'],
    ['Dialog', 'on'],
    ['APIResponse', 'body'],
    ['Route', 'fetch'],
    ['Page', 'keyboard'],
  ])
    assert.ok(
      ready.capabilities[family].includes(member),
      `${family}.${member}`,
    )
})

test('tests that the Playwright root exposes the actual SDK namespaces', async (t) => {
  const { ready, get, invoke } = await native(t)
  const root = ready.roots.playwright?.ref
  assert.equal(root?.family, 'Playwright')
  for (const [member, family] of [
    ['chromium', 'BrowserType'],
    ['selectors', 'Selectors'],
    ['request', 'APIRequest'],
  ]) {
    const value = await get(root, member)
    assert.equal(value.value.ref.family, family)
  }
  const browserType = (await get(root, 'chromium')).value.ref
  assert.equal(
    codec.decode((await invoke(browserType, 'name')).value),
    'chromium',
  )
  const request = (await get(root, 'request')).value.ref
  const result = await invoke(request, 'newContext')
  assert.equal(result.ok, false)
  assert.match(
    codec.decode(result.error).message,
    /Neo does not serve playwright.request yet/,
  )
})

test('tests that internal objects and foreign references remain transport errors', async (t) => {
  const { sdk, run, ready, get } = await native(t)
  const page = ready.roots.page.ref
  for (const object of [
    new sdk.Connection(0),
    new sdk.EventEmitter(),
    new (class UnknownInternal {})(),
  ]) {
    run.page.sample = object
    const result = await get(page, 'sample')
    assert.equal(result.kind, 'transportError')
    assert.match(
      result.message,
      /^bridge limit: untransportable return object /,
    )
  }
  for (const ref of [
    { ...page, epoch: 'other' },
    { ...page, family: 'Response' },
    { ...page, id: 999 },
  ])
    assert.equal((await get(ref, 'url')).kind, 'transportError')
})
