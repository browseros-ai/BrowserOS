/**
 * B2 element operations on the wire: each receiver's command and strictness (Locator strict,
 * Page and Frame not), the timeout each sends (the action timeout; none for highlights), the
 * `neo:` commands Playwright reaches through an `ElementHandle`, `eventInit` serialized before
 * anything is sent, and `highlight()`'s disposable.
 */
import { expect, test } from 'bun:test'
import { DisposableStub } from '../../src/disposable'
import { serializePlainArgument } from '../../src/plain-argument'
import { failure, type Message, start, TARGET_CLOSED } from './fake-runtime'

/** Every message's `[guid, method, params, metadata.timeout]`. */
function sent(messages: Message[]) {
  return messages.map(({ guid, method, params, metadata }) => [
    guid,
    method,
    params,
    (metadata as { timeout?: number }).timeout,
  ])
}

const FRAME = 'frame@RUN'
const PAGE = 'page@RUN'
const EMPTY_INIT = { value: { o: [], id: 1 }, handles: [] }

test('tests that focus, blur and dispatchEvent send strict locators and plain receivers', async () => {
  const { fake, page } = start()
  page.setDefaultTimeout(700)
  fake.answer = () => ({ result: {} })
  expect(await page.locator('#a').focus()).toBeUndefined()
  expect(await page.focus('#a', { timeout: 5 })).toBeUndefined()
  expect(await page.mainFrame().focus('#a', null as never)).toBeUndefined()
  expect(await page.locator('#a').blur({ timeout: 9 })).toBeUndefined()
  expect(await page.locator('#a').dispatchEvent('click')).toBeUndefined()
  expect(await page.dispatchEvent('#a', 'click', { detail: 2 })).toBeUndefined()
  expect(
    await page
      .mainFrame()
      .dispatchEvent('#a', 'ping', undefined, { strict: true }),
  ).toBeUndefined()
  expect(sent(fake.messages)).toEqual([
    [FRAME, 'focus', { selector: '#a', strict: true }, 700],
    [FRAME, 'focus', { selector: '#a' }, 5],
    [FRAME, 'focus', { selector: '#a' }, 700],
    [FRAME, 'blur', { selector: '#a', strict: true }, 9],
    [
      FRAME,
      'dispatchEvent',
      { selector: '#a', strict: true, type: 'click', eventInit: EMPTY_INIT },
      700,
    ],
    [
      FRAME,
      'dispatchEvent',
      {
        selector: '#a',
        type: 'click',
        eventInit: {
          value: { o: [{ k: 'detail', v: { n: 2 } }], id: 1 },
          handles: [],
        },
      },
      700,
    ],
    [
      FRAME,
      'dispatchEvent',
      {
        selector: '#a',
        strict: true,
        type: 'ping',
        eventInit: { value: { v: 'undefined' }, handles: [] },
      },
      700,
    ],
  ])
})

test('tests that an eventInit Neo cannot send throws before anything is sent', async () => {
  const { fake, page } = start()
  fake.answer = () => ({ result: {} })
  const locator = page.locator('#a')
  await expect(
    locator.dispatchEvent('click', { detail: () => 1 }),
  ).rejects.toThrow(
    'Attempting to serialize unexpected value at position "detail": () => 1',
  )
  await expect(
    locator.dispatchEvent('click', { data: new Uint8Array([1, 2]) }),
  ).rejects.toThrow('this one holds a typed array')
  expect(fake.messages).toEqual([])
})

test('tests that the handle-shaped Locator methods send neo commands strictly', async () => {
  const { fake, page } = start()
  page.setDefaultTimeout(700)
  const box = { x: 1, y: 2, width: 3, height: 4 }
  // The wire's keys arrive sorted (serde_json); the script sees Playwright's order.
  const wire = { height: 4, width: 3, x: 1, y: 2 }
  fake.answer = ({ method }) =>
    method === 'neo:boundingBox' ? { result: { value: wire } } : { result: {} }
  const locator = page.locator('#a')
  expect(await locator.scrollIntoViewIfNeeded()).toBeUndefined()
  expect(await locator.scrollIntoViewIfNeeded({ timeout: 9 })).toBeUndefined()
  expect(await locator.selectText({ timeout: 3 })).toBeUndefined()
  expect(await locator.selectText({ force: true })).toBeUndefined()
  const answered = await locator.boundingBox({ timeout: 4 })
  expect(answered).toEqual(box)
  expect(Object.keys(answered ?? {})).toEqual(['x', 'y', 'width', 'height'])
  fake.answer = () => ({ result: {} })
  expect(await locator.boundingBox()).toBeNull()
  await expect(locator.selectText(null as never)).rejects.toThrow(TypeError)
  await expect(locator.scrollIntoViewIfNeeded(null as never)).rejects.toThrow(
    TypeError,
  )
  await expect(locator.highlight(null as never)).rejects.toThrow(TypeError)
  await expect(locator.selectText({ force: null as never })).rejects.toThrow(
    'force: expected a value, got null',
  )
  expect(sent(fake.messages)).toEqual([
    [
      FRAME,
      'neo:scrollIntoViewIfNeeded',
      { selector: '#a', strict: true },
      700,
    ],
    [FRAME, 'neo:scrollIntoViewIfNeeded', { selector: '#a', strict: true }, 9],
    [FRAME, 'neo:selectText', { selector: '#a', strict: true }, 3],
    [
      FRAME,
      'neo:selectText',
      { selector: '#a', strict: true, force: true },
      700,
    ],
    [FRAME, 'neo:boundingBox', { selector: '#a', strict: true }, 4],
    [FRAME, 'neo:boundingBox', { selector: '#a', strict: true }, 700],
  ])
})

test('tests that highlight sends its style as CSS text and resolves to a one-shot disposable', async () => {
  const { fake, page } = start()
  page.setDefaultTimeout(700)
  fake.answer = () => ({ result: {} })
  const locator = page.locator('#a')
  const stub = await locator.highlight({
    style: { outlineColor: 'red', '--accent-size': 2, zIndex: 3 },
  })
  expect(stub).toBeInstanceOf(DisposableStub)
  await stub.dispose()
  await stub.dispose()
  await locator.highlight({ style: 'outline: 1px solid blue' })
  await locator.highlight()
  await locator.hideHighlight()
  await page.hideHighlight()
  expect(sent(fake.messages)).toEqual([
    [
      FRAME,
      'highlight',
      {
        selector: '#a',
        style: 'outline-color: red; --accent-size: 2; z-index: 3',
      },
      0,
    ],
    [FRAME, 'hideHighlight', { selector: '#a' }, 0],
    [
      FRAME,
      'highlight',
      { selector: '#a', style: 'outline: 1px solid blue' },
      0,
    ],
    [FRAME, 'highlight', { selector: '#a' }, 0],
    [FRAME, 'hideHighlight', { selector: '#a' }, 0],
    [PAGE, 'hideHighlight', {}, 0],
  ])
})

test('tests that disposing a highlight on a closed page resolves and other failures throw', async () => {
  const { fake, page } = start()
  fake.answer = ({ method }) =>
    method === 'hideHighlight' ? TARGET_CLOSED : { result: {} }
  await (await page.locator('#a').highlight()).dispose()
  fake.answer = ({ method }) =>
    method === 'hideHighlight'
      ? failure('Error', 'neo-runtime: something else')
      : { result: {} }
  const stub = await page.locator('#a').highlight()
  await expect(stub.dispose()).rejects.toThrow('neo-runtime: something else')
  // The cleanup ran once; a second dispose has nothing left to do.
  await stub.dispose()
  expect(
    fake.messages.filter((m) => m.method === 'hideHighlight'),
  ).toHaveLength(2)
})

test('tests that a DisposableStub is async-disposable where the realm has the symbol', async () => {
  let calls = 0
  const stub = new DisposableStub(async () => {
    calls += 1
  })
  const asyncDispose = (Symbol as { asyncDispose?: symbol }).asyncDispose
  if (asyncDispose) {
    await (stub as unknown as Record<symbol, () => Promise<void>>)[
      asyncDispose
    ]()
    expect(calls).toBe(1)
  }
  await stub.dispose()
  expect(calls).toBe(1)
})

test('tests that serializePlainArgument writes Playwright protocol values', () => {
  const shared = { k: 1 }
  const cyclic: Record<string, unknown> = { name: 'c' }
  cyclic.self = cyclic
  const value = (v: unknown) => serializePlainArgument(v).value
  expect(serializePlainArgument({}).handles).toEqual([])
  expect(value(undefined)).toEqual({ v: 'undefined' })
  expect(value(Symbol('s'))).toEqual({ v: 'undefined' })
  expect(value(null)).toEqual({ v: 'null' })
  expect(value(Number.NaN)).toEqual({ v: 'NaN' })
  expect(value(Number.POSITIVE_INFINITY)).toEqual({ v: 'Infinity' })
  expect(value(Number.NEGATIVE_INFINITY)).toEqual({ v: '-Infinity' })
  expect(value(-0)).toEqual({ v: '-0' })
  expect(value(1.5)).toEqual({ n: 1.5 })
  expect(value(true)).toEqual({ b: true })
  expect(value('s')).toEqual({ s: 's' })
  expect(value(10n)).toEqual({ bi: '10' })
  expect(value(new Date(0))).toEqual({ d: '1970-01-01T00:00:00.000Z' })
  expect(value(/a+/gi)).toEqual({ r: { p: 'a+', f: 'gi' } })
  expect(value(new URL('https://a.test/x'))).toEqual({ u: 'https://a.test/x' })
  const error = value(new TypeError('boom')) as { e: { n: string; m: string } }
  expect([error.e.n, error.e.m]).toEqual(['TypeError', 'boom'])
  expect(value([shared, shared])).toEqual({
    a: [{ o: [{ k: 'k', v: { n: 1 } }], id: 2 }, { ref: 2 }],
    id: 1,
  })
  expect(value(cyclic)).toEqual({
    o: [
      { k: 'name', v: { s: 'c' } },
      { k: 'self', v: { ref: 1 } },
    ],
    id: 1,
  })
  expect(() => serializePlainArgument({ list: [0, () => 0] })).toThrow(
    'Attempting to serialize unexpected value at position "list[1]": () => 0',
  )
  expect(() => serializePlainArgument(new Float32Array(1))).toThrow(
    "Neo's dispatchEvent takes plain eventInit values",
  )
})
