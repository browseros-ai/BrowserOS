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
import { type Call, loadPrelude, SEED } from '../tests/support/prelude-runtime'

/** The clock the recording runs on, so `since` stamps are the same on every build. */
const RECORDED_AT = 1_700_000_000_000

const ELEMENT = {
  $neo: 'element',
  page: { $neo: 'page', tab: 1007 },
  seq: 0,
  node: 9,
  display: '<button>Go</button>',
}
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
const ANSWERS: Record<string, unknown> = {
  'locator.selectOption': ['a'],
  'locator.count': 0,
  'locator.normalize': { selector: 'internal:role=button[name="Go"i]' },
  'locator.allTextContents': [],
  'locator.allInnerTexts': [],
  'locator.waitFor': ELEMENT,
  'locator.elementHandle': ELEMENT,
  'locator.elementHandles': [ELEMENT],
  'page.ariaSnapshot': { json: [], partial: [] },
  'locator.ariaSnapshot': { json: [], partial: [] },
  'page.frames': FRAMES,
  'page.opener': { opener: null },
  'page.evaluate': { value: null },
  'worker.evaluate': { value: 1 },
  'page.addScriptTag': { element: ELEMENT },
  'page.addStyleTag': { element: ELEMENT },
  'page.waitForFunction': { value: true, preview: 'true' },
  'page.screenshot': { $neo: 'bytes', base64: 'iVBORw0KGgo=' },
  'context.newPage': {
    page: { $neo: 'page', tab: 1008 },
    url: 'about:blank',
  },
  'download.path': { path: '/runs/r/f.csv', failure: null },
  'page.addInitScript': { id: 1 },
  'context.addInitScript': { id: 2 },
  'page.exposeBinding': { id: 3 },
  'context.exposeBinding': { id: 4 },
  'page.addLocatorHandler': { uid: 1 },
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
  return (call) => {
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
    return ANSWERS[call.name] ?? null
  }
}

/** Every method that reaches Rust, once; `context.pages()` is synchronous and sends nothing. */
const EVERY_OP = `
  // exposeFunctions names its callbacks at random; a fixed draw keeps the recording stable.
  Math.random = () => 0.25
  await page.exposeFunction('add', (a, b) => a + b)
  await page.exposeBinding('source', (source, a) => a); await context.exposeFunction('c', () => 1)
  const l = page.getByRole('button')
  await l.click(); await l.dblclick(); await l.hover(); await l.check(); await l.uncheck()
  await l.drop({ data: { 'text/plain': 'external' } });
  await l.dragTo(page.locator('#b')); await l.fill('x'); await l.press('a'); await l.pressSequentially('ab')
  await l.selectOption('a'); await l.selectText(); await l.focus(); await l.blur()
  // Rust stats paths before any page work, so the offline decode needs one that exists: '.'.
  await l.setInputFiles('.'); await l.setInputFiles({ name: 'a.txt', mimeType: 'text/plain', buffer: Buffer.from('a') })
  await l.dispatchEvent('click'); await l.scrollIntoViewIfNeeded()
  await l.textContent(); await l.innerText(); await l.innerHTML(); await l.getAttribute('id'); await l.inputValue()
  await l.isEnabled(); await l.isDisabled(); await l.isEditable(); await l.isChecked(); await l.isVisible(); await l.isHidden()
  await l.count(); await l.normalize(); await l.allTextContents(); await l.allInnerTexts(); await l.boundingBox(); await l.waitFor()
  const handle = await l.elementHandle(); await handle.click(); await l.elementHandles()
  await handle.$('b'); await handle.$$('b')
  await page.ariaSnapshot(); await l.ariaSnapshot(); await page.ariaSnapshotJSON({ boxes: true })
  await expect(l).toBeVisible(); await expect(page).toHaveTitle('x')
  await page.keyboard.down('a'); await page.keyboard.up('a'); await page.keyboard.press('a')
  await page.keyboard.type('ab'); await page.keyboard.insertText('ab')
  await page.mouse.move(1, 2); await page.mouse.down(); await page.mouse.up(); await page.mouse.click(1, 2); await page.mouse.wheel(0, 9)
  await page.goto('https://a.example/'); await page.reload(); await page.goBack(); await page.goForward()
  const pay = page.frame('pay')
  await pay.goto('https://pay.example/'); await pay.waitForLoadState('domcontentloaded'); await pay.locator('#card').count()
  await page.waitForNavigation(); await pay.waitForNavigation({ url: '**/pay' });
  await page.waitForURL('**/a'); await page.waitForLoadState(); await page.title(); await page.content()
  await page.requestGC();
  await page.emulateMedia({ media: 'print', colorScheme: 'dark', reducedMotion: 'reduce', forcedColors: 'active', contrast: 'more' });
  await page.localStorage.getItem('token'); await page.sessionStorage.setItem('flag', '1')
  await page.addScriptTag({ content: 'window.injected = true' }); await pay.addStyleTag({ content: 'body { color: red }' });
  await page.setContent('<p>hi</p>'); await page.opener(); await page.evaluate(() => 1)
  await l.evaluate((e) => e.id); await l.evaluateAll((es) => es.length); await page.waitForFunction(() => true)
  await l.waitForFunction((e) => e.id)
  await handle.evaluate((e) => e.id); await handle.$eval('b', (e) => e.id); await handle.$$eval('b', (es) => es.length)
  await page.$eval('b', (e) => e.id); await page.$$eval('b', (es) => es.length); await page.evaluate((e, h) => h.id, handle)
  await page.screenshot(); await l.screenshot(); await page.pdf()
  await page.screenshot({ path: '/out/screenshot.png' })
  await page.setViewportSize({ width: 640, height: 480 })
  const script = await page.addInitScript((a) => { window.a = a }, 1)
  // Rust reads a path before any page work, so the offline decode needs a file that exists.
  await page.addInitScript({ path: 'Cargo.toml' }); await context.addInitScript('window.b = 1')
  await page.addInitScript(({ cb }) => cb(1), { cb: () => 2 }, { exposeFunctions: true })
  await script.dispose()
  await page.addLocatorHandler(page.getByText('Accept cookies'), async () => {}, { noWaitAfter: true, times: 2 })
  await page.removeLocatorHandler(page.getByText('Accept cookies'))
  const tab = await context.newPage()
  page.on('console', () => {}); context.on('page', () => {})
  const chooserListener = () => {}
  page.on('filechooser', chooserListener)
  page.off('filechooser', chooserListener)
  const dialog = await page.waitForEvent('dialog'); await dialog.accept(); await dialog.dismiss()
  const download = await page.waitForEvent('download'); await download.path(); await download.saveAs('/out/f.csv')
  const worker = await page.waitForEvent('worker'); await worker.evaluate(() => 1)
  await tab.close({ runBeforeUnload: true }); await tab.close()
`

/** Runs `EVERY_OP` and returns every op the prelude sent, in order. */
export async function recordEveryOp(prelude?: string): Promise<Call[]> {
  const runtime = loadPrelude({
    seed: SEED,
    answer: everyOpAnswer(),
    now: RECORDED_AT,
    ...(prelude === undefined ? {} : { prelude }),
  })
  await runtime.run(EVERY_OP)
  // `page.on` sends its listen op without awaiting it; let those land.
  await new Promise((resolve) => setTimeout(resolve, 0))
  return runtime.calls
}

/**
 * The recording as the crate reads it: each op's distinct inputs, exactly as sent (the `call`
 * header included), ops in name order and inputs in the order the script sent them.
 */
export async function wireExamplesDocument(prelude?: string): Promise<string> {
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
      ops,
    },
    null,
    2,
  )}\n`
}
