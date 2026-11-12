/**
 * The prelude's half of the wire lever: every op input the prelude sends for one script that
 * calls every Playwright method Neo routes to Rust, recorded through the built prelude.
 *
 * scripts/build.ts writes the recording to
 * crates/neo-sdk/neo-playwright/tests/data/wire-examples.json (and `--check` fails when it is
 * stale), and the crate's `tests_that_every_prelude_example_decodes` invokes each input offline
 * through `Playwright::invoke`. A field renamed on one side alone therefore fails a check: here
 * when wire.ts changes, there when a Rust input changes. tests/prelude-wire.test.ts runs the
 * same script to check where each op carries its page handle.
 */
import { Buffer } from 'node:buffer'
import type { ElementBrand, OpOutputs } from '../src/prelude/wire'
import { type Call, loadPrelude, SEED } from '../tests/support/prelude-runtime'
import { inputExamples } from './input-examples'

/** The clock the recording runs on, so `since` stamps are the same on every build. */
const RECORDED_AT = 1_700_000_000_000

const ELEMENT = {
  $neo: 'element',
  page: { $neo: 'page', tab: 1007 },
  seq: 0,
  node: 9,
  display: '<button>Go</button>',
} satisfies ElementBrand
const DIALOG = {
  seq: 1,
  event: 'dialog',
  page: { $neo: 'page', tab: 1007 },
  dialog: { id: 4, type: 'confirm', message: 'Sure?', defaultValue: '' },
}
const DOWNLOAD = {
  seq: 2,
  event: 'download',
  page: { $neo: 'page', tab: 1007 },
  download: {
    id: 'guid-5',
    url: 'https://a.example/f.csv',
    suggestedFilename: 'f.csv',
  },
}
/** One call of `add`, which the script exposes first: the pump's first answer carries it. */
const BINDING_CALL = {
  seq: 3,
  at: RECORDED_AT,
  event: 'binding',
  page: { $neo: 'page', tab: 1007 },
  frame: null,
  executionContext: 3,
  payload: JSON.stringify({
    name: 'add',
    seq: 1,
    serializedArgs: [1, 2],
  }),
}
/** The main frame and one child, so the script can drive the frame forms of page ops. */
const FRAMES = {
  frames: [
    { id: 'MAIN', parentId: null, name: '', url: 'https://a.example/' },
    { id: 'F1', parentId: 'MAIN', name: 'pay', url: 'https://pay.example/' },
  ],
}
const ANSWERS = {
  'locator.selectOption': ['a'],
  'locator.count': 0,
  'locator.normalize': { resolvedSelector: 'internal:role=button[name="Go"i]' },
  'locator.allTextContents': [],
  'locator.allInnerTexts': [],
  'locator.waitFor': ELEMENT,
  'locator.elementHandle': ELEMENT,
  'locator.elementHandles': [ELEMENT],
  'page.ariaSnapshot': { json: [], partial: [] },
  'locator.ariaSnapshot': { json: [], partial: [] },
  'page.frames': FRAMES,
  'page.opener': { opener: null },
  'page.evaluate': { value: { v: 'null' } },
  'worker.evaluate': { v: 'null' },
  'page.addScriptTag': { element: ELEMENT },
  'page.addStyleTag': { element: ELEMENT },
  'page.waitForFunction': { value: true, preview: 'true' },
  'page.screenshot': Buffer.from('iVBORw0KGgo=', 'base64'),
  'context.newPage': {
    page: { $neo: 'page', tab: 1008 },
    url: 'about:blank',
  },
  'download.path': { path: '/runs/r/f.csv', failure: null },
  'page.addInitScript': 1,
  'context.addInitScript': 2,
  'page.exposeBinding': 3,
  'context.exposeBinding': 4,
  'page.addLocatorHandler': 1,
} satisfies Partial<OpOutputs>

/** Fixture calls include bootstrap-internal names too; only these own keys have canned answers. */
function hasAnswer(name: string): name is keyof typeof ANSWERS {
  return Object.hasOwn(ANSWERS, name)
}

/**
 * A plausible result for each op, so the script runs to its end. The pump's first pull answers
 * one binding call, which makes the prelude send `bindingCall.resolve`; the locator-handler
 * pull's first answer triggers the one handler, which makes it send `page.resolveLocatorHandler`.
 * Both idle after that.
 */
function everyOpAnswer(): (call: Call) => unknown {
  let pulled = false
  let triggered = false
  let handlerUid = 0
  return (call) => {
    if (call.name === 'page.addLocatorHandler') return ++handlerUid
    if (call.name === 'events.wait') {
      if (call.input.event === 'worker')
        return {
          seq: 4,
          at: RECORDED_AT,
          event: 'worker',
          page: { $neo: 'page', tab: 1007 },
          worker: { id: 'worker-1', url: 'https://a.example/worker.js' },
        }
      return call.input.event === 'dialog' ? DIALOG : DOWNLOAD
    }
    if (call.name === 'events.next') {
      if (pulled) return new Promise(() => {})
      pulled = true
      return { events: [BINDING_CALL], lost: 0 }
    }
    if (call.name === 'page.locatorHandlerTriggered') {
      if (triggered) return new Promise(() => {})
      triggered = true
      return { uid: 1 }
    }
    const answer = hasAnswer(call.name) ? ANSWERS[call.name] : null
    // OpOutputs describes what the prelude sees after the bootstrap revives brands. This
    // fixture answers at the host's JSON boundary, so put binary answers back in wire form.
    return answer instanceof Uint8Array
      ? { $neo: 'bytes', base64: Buffer.from(answer).toString('base64') }
      : answer
  }
}

/** Every method that reaches Rust, once; `context.pages()` is synchronous and sends nothing. */
const EVERY_OP = `
  // exposeFunctions names its callbacks at random; a fixed draw keeps the recording stable.
  Math.random = () => 0.25
  await page.exposeFunction('add', (a, b) => a + b)
  await page.exposeBinding('source', (source, a) => a); await context.exposeFunction('c', () => 1)
  const l = page.getByRole('button')
  // Keys and ordinary values come from the input types. These adaptations bridge public
  // objects/valid value domains to their wire representations; they never enumerate bags.
  const sample = (op) => {
    const value = { ...samples[op] }
    if ('mask' in value) value.mask = [l]
    if ('maskColor' in value) value.maskColor = '#00ff00'
    if ('format' in value) value.format = 'A4'
    if ('pageRanges' in value) value.pageRanges = '1'
    for (const key of ['width', 'height'])
      if (typeof value[key] === 'string') value[key] = '10px'
    if ('margin' in value) value.margin = Object.fromEntries(Object.keys(value.margin).map(side => [side, '1px']))
    if ('path' in value) value.path = 'Cargo.toml'
    if ('files' in value) value.files = [{ name: 'option.txt', mimeType: 'text/plain', buffer: Buffer.from('a') }]
    if ('data' in value) value.data = { 'text/plain': 'external' }
    if ('url' in value) value.url = typeof value.url === 'object' ? '**/pay' : 'https://a.example/script.js'
    return value
  }
  await l.click(sample('locator.click')); await l.dblclick(sample('locator.dblclick'))
  await l.hover(sample('locator.hover')); await l.check(sample('locator.check')); await l.uncheck(sample('locator.uncheck'))
  // Drop separates the payload from its position argument.
  const drop = sample('locator.drop'); await l.drop(drop, { position: drop.position });
  await l.dragTo(page.locator('#b'), sample('locator.dragTo')); await l.fill('x', sample('locator.fill'))
  await l.press('a', sample('locator.press')); await l.pressSequentially('ab', sample('locator.pressSequentially'))
  await l.selectOption('a', sample('locator.selectOption')); await l.selectText(sample('locator.selectText')); await l.focus(); await l.blur()
  // Rust stats paths before any page work, so the offline decode needs one that exists: '.'.
  await l.setInputFiles('.'); await l.setInputFiles({ name: 'a.txt', mimeType: 'text/plain', buffer: Buffer.from('a') })
  await l.dispatchEvent('click'); await l.scrollIntoViewIfNeeded()
  await l.textContent(); await l.innerText(); await l.innerHTML(); await l.getAttribute('id'); await l.inputValue()
  await l.isEnabled(); await l.isDisabled(); await l.isEditable(); await l.isChecked(); await l.isVisible(); await l.isHidden()
  await l.count(); await l.normalize(); await l.allTextContents(); await l.allInnerTexts(); await l.boundingBox(); await l.waitFor(sample('locator.waitFor'))
  const handle = await l.elementHandle(); await handle.click(); await l.elementHandles()
  await handle.$('b'); await handle.$$('b')
  await page.ariaSnapshot(sample('page.ariaSnapshot')); await l.ariaSnapshot(sample('locator.ariaSnapshot'))
  await page.ariaSnapshotJSON(sample('page.ariaSnapshot')); await l.ariaSnapshotJSON(sample('locator.ariaSnapshot'))
  await expect(l).toBeVisible(); await expect(page).toHaveTitle('x')
  // A single generic matcher cannot produce Frame.expect's different optional params.
  const locatorExpect = sample('expect.locator'), pageExpect = sample('expect.page')
  await expect(l).toHaveCount(locatorExpect.expectedNumber)
  await expect(l).toHaveAttribute('data-state', 'ready')
  await expect(l).toHaveCSS('color', 'red', { pseudo: locatorExpect.pseudo })
  await expect(l).toHaveJSProperty('state', locatorExpect.expectedValue)
  await expect(l).toHaveText('sample', { useInnerText: locatorExpect.useInnerText })
  await expect(page).toHaveTitle('sample', { useInnerText: pageExpect.useInnerText })
  await expect(page).toMatchAriaSnapshot('- button "Go"')
  await expect(l).toMatchAriaSnapshot('['); await expect(page).toMatchAriaSnapshot('[')
  await page.keyboard.down('a'); await page.keyboard.up('a'); await page.keyboard.press('a', sample('keyboard.press'))
  await page.keyboard.type('ab', sample('keyboard.type')); await page.keyboard.insertText('ab')
  await page.mouse.move(1, 2, sample('mouse.move')); await page.mouse.down(sample('mouse.down'))
  await page.mouse.up(sample('mouse.up')); await page.mouse.click(1, 2, sample('mouse.click')); await page.mouse.wheel(0, 9)
  await page.goto('https://a.example/', sample('page.goto')); await page.reload(); await page.goBack(); await page.goForward()
  const pay = page.frame('pay')
  await pay.goto('https://pay.example/'); await pay.waitForLoadState('domcontentloaded'); await pay.locator('#card').count()
  await page.waitForNavigation(); await pay.waitForNavigation({ url: '**/pay' });
  await page.waitForURL('**/a'); await page.waitForLoadState(); await page.title(); await page.content()
  await page.requestGC();
  await page.emulateMedia({ media: 'print', colorScheme: 'dark', reducedMotion: 'reduce', forcedColors: 'active', contrast: 'more' });
  await page.localStorage.getItem('token'); await page.sessionStorage.setItem('flag', '1')
  await page.localStorage.items(); await page.localStorage.removeItem('token'); await page.sessionStorage.clear()
  // content, url and path are alternative sources, so exercise each on a separate call.
  const scriptTag = sample('page.addScriptTag'), styleTag = sample('page.addStyleTag')
  await page.addScriptTag({ content: scriptTag.content, type: scriptTag.type })
  await page.addScriptTag({ url: scriptTag.url, type: scriptTag.type }); await page.addScriptTag({ path: scriptTag.path, type: scriptTag.type })
  await pay.addStyleTag({ content: styleTag.content }); await page.addStyleTag({ url: styleTag.url }); await page.addStyleTag({ path: styleTag.path })
  await page.setContent('<p>hi</p>'); await page.opener(); await page.evaluate(() => 1)
  await l.evaluate((e) => e.id); await l.evaluateAll((es) => es.length); await page.waitForFunction(() => true, undefined, sample('page.waitForFunction'))
  await l.waitForFunction((e) => e.id)
  await handle.evaluate((e) => e.id); await handle.$eval('b', (e) => e.id); await handle.$$eval('b', (es) => es.length)
  await page.$eval('b', (e) => e.id); await page.$$eval('b', (es) => es.length); await page.evaluate((e, h) => h.id, handle)
  await page.screenshot(sample('page.screenshot')); await l.screenshot(sample('page.screenshot')); await page.pdf(sample('page.pdf'))
  await page.screenshot({ path: '/out/screenshot.png' })
  await page.setViewportSize({ width: 640, height: 480 })
  const script = await page.addInitScript((a) => { window.a = a }, 1)
  // Rust reads a path before any page work, so the offline decode needs a file that exists.
  await page.addInitScript({ path: 'Cargo.toml' }); await context.addInitScript('window.b = 1')
  await page.addInitScript(({ cb }) => cb(1), { cb: () => 2 }, { exposeFunctions: true })
  Math.random = () => 0.5
  await context.addInitScript(({ cb }) => cb(1), { cb: () => 2 }, { exposeFunctions: true })
  await script.dispose()
  let handlerRan
  const handled = new Promise(resolve => { handlerRan = resolve })
  await page.addLocatorHandler(page.getByText('Accept cookies'), async () => { handlerRan() }, { noWaitAfter: true, times: 1 })
  // Let the trigger consume its single use before an explicit removal can delete it.
  await handled
  await page.removeLocatorHandler(page.getByText('Accept cookies'))
  await page.addLocatorHandler(page.getByText('Later'), async () => {})
  await page.removeLocatorHandler(page.getByText('Later'))
  const tab = await context.newPage()
  page.on('console', () => {}); context.on('page', () => {})
  const chooserListener = () => {}
  page.on('filechooser', chooserListener)
  page.off('filechooser', chooserListener)
  const dialog = await page.waitForEvent('dialog'); await dialog.accept(sample('dialog.accept').promptText); await dialog.dismiss()
  const download = await page.waitForEvent('download'); await download.path(); await download.saveAs('/out/f.csv')
  const worker = await page.waitForEvent('worker'); await worker.evaluate(() => 1)
  await tab.close({ runBeforeUnload: true }); await tab.close()
`

/** Runs `EVERY_OP` and returns every op the prelude sent, in order. */
export async function recordEveryOp(prelude?: string): Promise<Call[]> {
  const { samples } = await inputExamples()
  const runtime = loadPrelude({
    seed: SEED,
    answer: everyOpAnswer(),
    now: RECORDED_AT,
    ...(prelude === undefined ? {} : { prelude }),
  })
  await runtime.run(`const samples = ${JSON.stringify(samples)};\n${EVERY_OP}`)
  // `page.on` sends its listen op without awaiting it; let those land.
  await new Promise((resolve) => setTimeout(resolve, 0))
  // retiredTab is produced only when the run releases internal worker bookkeeping. A fresh
  // worker-aware page and a real close acknowledgement exercise that lifecycle separately
  // from the binding/handler pump above, whose first event deliberately carries callbacks.
  const retired = loadPrelude({
    seed: {
      ...SEED,
      pages: [
        {
          tab: 1007,
          url: 'https://a.example/',
          title: 'A',
          closed: false,
          workers: [],
        },
      ],
    },
    answer: (call) => {
      if (call.name === 'events.next') return new Promise(() => {})
      if (call.name === 'page.close')
        return {
          ok: null,
          pages: [
            { tab: 1007, url: null, title: null, closed: true, workers: [] },
          ],
        }
      return ANSWERS[call.name] ?? null
    },
    now: RECORDED_AT,
    ...(prelude === undefined ? {} : { prelude }),
  })
  await retired.run('await page.close()')
  await new Promise((resolve) => setTimeout(resolve, 0))
  return [...runtime.calls, ...retired.calls]
}

/**
 * The recording as the crate reads it: each op's distinct inputs, exactly as sent (the `call`
 * header included), ops in name order and inputs in the order the script sent them.
 */
export async function wireExamplesDocument(prelude?: string): Promise<string> {
  const { optionalKeys } = await inputExamples()
  const byOp = new Map<string, { seen: Set<string>; inputs: unknown[] }>()
  for (const call of await recordEveryOp(prelude)) {
    const entry = byOp.get(call.name) ?? { seen: new Set(), inputs: [] }
    byOp.set(call.name, entry)
    const key = JSON.stringify(call.input)
    if (entry.seen.has(key)) continue
    entry.seen.add(key)
    entry.inputs.push(call.input)
  }
  const ops = Object.fromEntries(
    [...byOp.keys()].sort().map((name) => [name, byOp.get(name)?.inputs]),
  )
  return `${JSON.stringify(
    {
      about:
        'Every op input the prelude sends, recorded by crates/neo-sdk/neo-playwright-js/scripts/wire-examples.ts. Do not edit: run bun run build in crates/neo-sdk/neo-playwright-js.',
      optionalKeys: Object.fromEntries(Object.entries(optionalKeys).sort()),
      ops,
    },
    null,
    2,
  )}\n`
}
