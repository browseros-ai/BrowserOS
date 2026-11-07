// `null` where a script may pass options, through Playwright and through Neo on fresh
// form-actions pages: every method Neo serves on Page, Frame (`page.mainFrame()`) and Locator, once
// with `null` for the whole options object and once with each of the method's schema keys set to
// `null`. Each call is recorded as resolved (with its value) or rejected (with its error's name),
// never its wording.
//
// Playwright reads a `null` options object as `{}` wherever its client spreads it or passes it to
// `Frame._timeout` (`options || {}`), and throws `TypeError` where it reads a property of it first
// (`goto`, `ariaSnapshot`, `close`, `getByRole`). Its client validator (`tOptional`) lets only
// `undefined` through, so a schema key set to `null` rejects with a plain `Error` before anything
// is sent; `timeout` and unknown keys are not schema keys and are ignored.
//
// Runs under Node, not Bun, on the shared harness (harness.mjs).
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'

const html = readFileSync(
  new URL('../pages/form-actions.html', import.meta.url),
  'utf8',
)

/** Records how each call settled in `results`, keyed by the call. */
const SETTLE = `
  const results = {}
  const settle = async (call, run) => {
    try {
      const value = await run()
      results[call] = value === undefined ? 'resolved' : 'resolved ' + JSON.stringify(value)
    } catch (error) {
      results[call] = 'rejected ' + error.name
    }
  }
`

/** Playwright's schema keys for each method (spec/playwright/frame.yml at 26137face). */
const KEYS = {
  click: [
    'selector',
    'strict',
    'force',
    'scroll',
    'noWaitAfter',
    'modifiers',
    'position',
    'delay',
    'button',
    'clickCount',
    'trial',
    'steps',
  ],
  fill: ['selector', 'strict', 'value', 'force'],
  press: ['selector', 'strict', 'key', 'delay', 'noWaitAfter'],
  read: ['selector', 'strict'],
  check: ['selector', 'strict', 'force', 'scroll', 'position', 'trial'],
  selectOption: ['selector', 'strict', 'elements', 'options', 'force'],
  getAttribute: ['selector', 'strict', 'name'],
  ariaSnapshot: ['mode', 'depth', 'boxes'],
  goto: ['url', 'waitUntil', 'referer'],
  dispatchEvent: ['selector', 'strict', 'type', 'eventInit'],
}

/** Each method: its target, the arguments between the selector and the options, its keys. */
const METHODS = [
  ['click', '#plain', [], KEYS.click],
  ['fill', '#text', ['typed'], KEYS.fill],
  ['press', '#text', ['a'], KEYS.press],
  ['inputValue', '#area', [], KEYS.read],
  ['isChecked', '#checkbox', [], KEYS.read],
  ['isVisible', '#plain', [], KEYS.read],
  ['check', '#checkbox', [], KEYS.check],
  ['uncheck', '#checkbox', [], KEYS.check],
  ['setChecked', '#checkbox', [true], KEYS.check],
  ['selectOption', '#single', ['two'], KEYS.selectOption],
  ['textContent', '#plain', [], KEYS.read],
  ['innerText', '#plain', [], KEYS.read],
  ['getAttribute', '#text', ['value'], KEYS.getAttribute],
  ['focus', '#text', [], KEYS.read],
  ['dispatchEvent', '#plain', ['click', {}], KEYS.dispatchEvent],
]
const LOCATOR_ONLY = [
  ['clear', '#area', [], KEYS.fill],
  ['blur', '#text', [], KEYS.read],
  ['boundingBox', '#plain', [], []],
]
/** Locator methods Playwright serves through an ElementHandle, whose `null` options settle
 * differently: each reads a property of its options first (`options.timeout`, or
 * `options.style` for `highlight`), and `highlight` turns an object `style` into CSS. */
const HANDLE_SHAPED = [
  ['scrollIntoViewIfNeeded', '#plain', [], []],
  ['selectText', '#text', [], ['force']],
  ['highlight', '#plain', [], ['style']],
]
const HANDLE_SHAPED_KINDS = {
  'locator.scrollIntoViewIfNeeded(..., null)': 'rejected TypeError',
  'locator.selectText(..., null)': 'rejected TypeError',
  'locator.selectText(..., { force: null })': 'rejected Error',
  'locator.highlight(..., null)': 'rejected TypeError',
  'locator.highlight(..., { style: null })': 'rejected TypeError',
}
const BUILDERS = [
  ['locator', '#plain'],
  ['getByRole', 'textbox'],
  ['getByText', 'Plain'],
  ['getByLabel', 'Name'],
  ['getByAltText', 'Plain'],
  ['getByPlaceholder', 'Plain'],
  ['getByTitle', 'Plain'],
]

const source = `
  ${SETTLE}
  page.setDefaultTimeout(2000)
  // Same-document: a goto one engine accepted must not replace the page the probe reads.
  const url = page.url() + '#goto'
  const frame = page.mainFrame()
  const receivers = {
    page: (selector) => [page, [selector]],
    frame: (selector) => [frame, [selector]],
    locator: (selector) => [page.locator(selector), []],
  }
  for (const [receiver, target] of Object.entries(receivers)) {
    const methods = ${JSON.stringify(METHODS)}
    if (receiver === 'locator')
      methods.push(...${JSON.stringify([...LOCATOR_ONLY, ...HANDLE_SHAPED])})
    for (const [method, selector, args, keys] of methods) {
      const [object, head] = target(selector)
      await settle(receiver + '.' + method + '(..., null)', () => object[method](...head, ...args, null))
      for (const key of [...keys, 'timeout', 'unknown'])
        await settle(receiver + '.' + method + '(..., { ' + key + ': null })', () =>
          object[method](...head, ...args, { [key]: null }))
    }
  }
  await settle('page.title(null)', () => page.title(null))
  await settle('frame.title(null)', () => frame.title(null))
  await settle('locator.count(null)', () => page.locator('.duplicate').count(null))
  await settle('locator.allTextContents(null)', () => page.locator('#plain').allTextContents(null))
  // The snapshot's type, not its YAML: BrowserOS neo adds its takeover bar to pages opened a few
  // seconds after launch, and either engine's snapshot then ends in its \`- document\`.
  // ariaSnapshot.test.mjs compares the YAML.
  const snapshotType = (snapshot) => snapshot.then((yaml) => typeof yaml)
  await settle('page.ariaSnapshot(null)', () => snapshotType(page.ariaSnapshot(null)))
  for (const key of [...${JSON.stringify(KEYS.ariaSnapshot)}, 'selector', 'timeout'])
    await settle('page.ariaSnapshot({ ' + key + ': null })', () =>
      snapshotType(page.ariaSnapshot({ [key]: null })))
  await settle('page.bringToFront(null)', () => page.bringToFront(null))
  await settle('page.close(null)', () => page.close(null))
  for (const [receiver, object] of [['page', page], ['frame', frame], ['locator', page.locator('body')]]) {
    for (const [builder, arg] of ${JSON.stringify(BUILDERS)})
      await settle(receiver + '.' + builder + '(..., null).count()', () => object[builder](arg, null).count())
  }
  await settle('locator.filter(null).count()', () => page.locator('#plain').filter(null).count())
  for (const [receiver, object] of [['page', page], ['frame', frame]]) {
    await settle(receiver + '.goto(url, null)', () => object.goto(url, null))
    for (const key of ${JSON.stringify(KEYS.goto)})
      await settle(receiver + '.goto(url, { ' + key + ': null })', () => object.goto(url, { [key]: null }))
  }
  return results
`

/** How each method call must settle: Playwright's behavior, spelled out. */
function expectedMethodKinds() {
  const kinds = {}
  for (const receiver of ['page', 'frame', 'locator']) {
    const methods =
      receiver === 'locator'
        ? [...METHODS, ...LOCATOR_ONLY, ...HANDLE_SHAPED]
        : METHODS
    for (const [method, , , keys] of methods) {
      kinds[`${receiver}.${method}(..., null)`] = 'resolved'
      for (const key of keys)
        kinds[`${receiver}.${method}(..., { ${key}: null })`] = 'rejected Error'
      kinds[`${receiver}.${method}(..., { timeout: null })`] = 'resolved'
      kinds[`${receiver}.${method}(..., { unknown: null })`] = 'resolved'
    }
  }
  return { ...kinds, ...HANDLE_SHAPED_KINDS }
}

/** How every other call must settle. */
const OTHER_KINDS = {
  'page.title(null)': 'resolved',
  'frame.title(null)': 'resolved',
  'locator.count(null)': 'resolved',
  'locator.allTextContents(null)': 'resolved',
  'page.ariaSnapshot(null)': 'rejected TypeError',
  'page.ariaSnapshot({ mode: null })': 'rejected Error',
  'page.ariaSnapshot({ depth: null })': 'rejected Error',
  'page.ariaSnapshot({ boxes: null })': 'rejected Error',
  // `page.ariaSnapshot` builds its params itself: a `selector` option is never sent.
  'page.ariaSnapshot({ selector: null })': 'resolved',
  'page.ariaSnapshot({ timeout: null })': 'resolved',
  'page.bringToFront(null)': 'resolved',
  'page.close(null)': 'rejected TypeError',
  ...Object.fromEntries(
    ['page', 'frame', 'locator'].flatMap((receiver) =>
      BUILDERS.map(([builder]) => [
        `${receiver}.${builder}(..., null).count()`,
        builder === 'getByRole' ? 'rejected TypeError' : 'resolved',
      ]),
    ),
  ),
  'locator.filter(null).count()': 'resolved',
  ...Object.fromEntries(
    ['page', 'frame'].flatMap((receiver) => [
      [`${receiver}.goto(url, null)`, 'rejected TypeError'],
      ...KEYS.goto.map((key) => [
        `${receiver}.goto(url, { ${key}: null })`,
        'rejected Error',
      ]),
    ]),
  ),
}

test('null options and null option values settle as in Playwright', {
  skip,
  timeout: 300_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html,
    source,
    probe: '({ ...window.__facts(), hash: location.hash })',
  })
  // Values were compared with Playwright's above; here only how each call settled.
  const kinds = Object.fromEntries(
    Object.entries(outcome.value).map(([call, settled]) => [
      call,
      settled.startsWith('resolved') ? 'resolved' : settled,
    ]),
  )
  assert.deepEqual(kinds, { ...expectedMethodKinds(), ...OTHER_KINDS })
  assert.equal(outcome.value['page.title(null)'], 'resolved "Form actions"')
  assert.equal(outcome.value['page.textContent(..., null)'], 'resolved "Plain"')
})

test('page.close settles null options as in Playwright, on a page of its own', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html,
    source: `
      ${SETTLE}
      const spare = await page.context().newPage()
      await settle('close(null)', () => spare.close(null))
      await settle('close({ reason: null })', () => spare.close({ reason: null }))
      const openAfterRejections = !spare.isClosed()
      await settle('close({ runBeforeUnload: null })', () => spare.close({ runBeforeUnload: null }))
      return { results, openAfterRejections, closedAtEnd: spare.isClosed() }
    `,
    probe: 'document.title',
  })
  assert.deepEqual(outcome.value, {
    results: {
      'close(null)': 'rejected TypeError',
      'close({ reason: null })': 'rejected Error',
      'close({ runBeforeUnload: null })': 'resolved',
    },
    openAfterRejections: true,
    closedAtEnd: true,
  })
})

test('a null default timeout rejects timed calls as in Playwright, and untimed ones run', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html,
    source: `
      ${SETTLE}
      const timed = async (label) => {
        await settle(label + ' textContent', () => page.textContent('#plain'))
        await settle(label + ' click', () => page.locator('#plain').click())
        await settle(label + ' isVisible', () => page.isVisible('#plain'))
        await settle(label + ' title', () => page.title())
        await settle(label + ' goto', () => page.goto(page.url() + '#same'))
      }
      page.setDefaultTimeout(null)
      await timed('page')
      page.setDefaultTimeout(undefined)
      page.context().setDefaultTimeout(null)
      await timed('context')
      page.context().setDefaultTimeout(undefined)
      page.setDefaultNavigationTimeout(null)
      await timed('navigation')
      return results
    `,
    probe: 'location.hash',
  })
  const kinds = Object.fromEntries(
    Object.entries(outcome.value).map(([call, settled]) => [
      call,
      settled.startsWith('resolved') ? 'resolved' : settled,
    ]),
  )
  assert.deepEqual(kinds, {
    'page textContent': 'rejected Error',
    'page click': 'rejected Error',
    'page isVisible': 'resolved',
    'page title': 'resolved',
    'page goto': 'rejected Error',
    'context textContent': 'rejected Error',
    'context click': 'rejected Error',
    'context isVisible': 'resolved',
    'context title': 'resolved',
    'context goto': 'rejected Error',
    'navigation textContent': 'resolved',
    'navigation click': 'resolved',
    'navigation isVisible': 'resolved',
    'navigation title': 'resolved',
    'navigation goto': 'rejected Error',
  })
})
