// `elementHandle.screenshot`'s client half: Playwright's option mapping
// (`client/elementHandle.ts` `ElementHandle.screenshot`) and the timeout its frame's page gives
// the call (`client/frame.ts` `Frame._timeout`).
import { expect, test } from 'bun:test'
import { BrowserContext } from '../../src/browser-context'
import { Connection } from '../../src/connection'
import { ElementHandle } from '../../src/element-handle'
import { EventPipe } from '../../src/events'
import type { CreatedHandle } from '../../src/js-handle'
import { Page } from '../../src/page'
import { elementScreenshotParams } from '../../src/screenshot'
import { TimeoutSettings } from '../../src/timeout-settings'

type Sent = {
  apiName: string
  guid: string
  method: string
  params: unknown
  timeout: unknown
}

function pageOn(connection: Connection): Page {
  return new Page(
    'page@T',
    'frame@T',
    '',
    new BrowserContext('browser-context@T', connection, new EventPipe()),
    connection,
  )
}

/** A connection whose handle calls are recorded and answered with `answer`. */
function recording(answer: unknown): { connection: Connection; sent: Sent[] } {
  const connection = new Connection(0)
  const sent: Sent[] = []
  connection.sendHandle = (async (apiName, guid, method, params, timeout) => {
    sent.push({ apiName, guid, method, params, timeout })
    return answer as never
  }) as Connection['sendHandle']
  return { connection, sent }
}

const PNG = 'iVBORw0KGgo='

test('tests that element screenshot params map the options as Playwright does', () => {
  const page = pageOn(new Connection(0))
  expect(
    elementScreenshotParams('elementHandle.screenshot', {
      path: 'shots/a.jpg',
      omitBackground: true,
      mask: [page.locator('#secret')],
      maskColor: '#00FF00',
      timeout: 5,
    }),
  ).toEqual({
    type: 'jpeg',
    omitBackground: true,
    mask: [{ frame: { guid: 'frame@T' }, selector: '#secret' }],
    maskColor: '#00FF00',
    path: 'shots/a.jpg',
  })
  // An explicit type wins over the path's extension, and an empty path writes nothing.
  expect(
    elementScreenshotParams('elementHandle.screenshot', {
      type: 'png',
      path: '',
    }),
  ).toEqual({ type: 'png' })
  expect(() =>
    elementScreenshotParams('elementHandle.screenshot', { path: 'a.gif' }),
  ).toThrow('elementHandle.screenshot: ')
})

test('tests that an element screenshot is timed by its frame page settings', async () => {
  const { connection, sent } = recording({ binary: PNG })
  const page = pageOn(connection)
  page.setDefaultTimeout(1234)
  const element = ElementHandle.fromNullable(
    connection,
    {
      type: 'ElementHandle',
      guid: 'handle@run.1',
      initializer: { preview: 'JSHandle@node' },
    },
    page.mainFrame(),
  )
  expect(element).not.toBeNull()
  const bytes = await element?.screenshot({ type: 'png' })
  expect(bytes?.length).toBeGreaterThan(0)
  const signal = new AbortController().signal
  await element?.screenshot({ timeout: 50, signal } as never)
  expect(sent).toEqual([
    {
      apiName: 'elementHandle.screenshot',
      guid: 'handle@run.1',
      method: 'screenshot',
      params: { type: 'png' },
      timeout: { signal: undefined, timeout: 1234 },
    },
    {
      apiName: 'elementHandle.screenshot',
      guid: 'handle@run.1',
      method: 'screenshot',
      // No type and no path: Playwright's client leaves `type` to the server's png.
      params: {},
      timeout: { signal, timeout: 50 },
    },
  ])
})

test('tests that a handle without a frame gets fresh timeout settings', async () => {
  const { connection, sent } = recording({ binary: PNG })
  const element = new ElementHandle(connection, 'handle@run.1', {
    preview: 'JSHandle@node',
  })
  await element.screenshot()
  expect(sent[0]?.timeout).toEqual(new TimeoutSettings().timeout({}, null))
})

test('tests that handles a handle derives keep its frame', async () => {
  const created: CreatedHandle = {
    type: 'ElementHandle',
    guid: 'handle@run.2',
    initializer: { preview: 'JSHandle@child' },
  }
  const { connection } = recording({
    element: created,
    elements: [created],
    handle: created,
  })
  const frame = pageOn(connection).mainFrame()
  const element = ElementHandle.fromNullable(
    connection,
    { ...created, guid: 'handle@run.1' },
    frame,
  )
  expect(element?._frame).toBe(frame)
  expect((await element?.$('span'))?._frame).toBe(frame)
  expect((await element?.$$('span'))?.[0]?._frame).toBe(frame)
  expect((await element?.getProperty('firstChild'))?._frame).toBe(frame)
})
