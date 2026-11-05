// What Rust receives for a script's Playwright calls: op names, the `call` header (the method
// the script called and its resolved timeout), the target handles target.rs decodes, and the
// prelude's own refusals. Every op goes through a fake bootstrap that carries JSON only.
import { describe, expect, it } from 'bun:test'
import { recordEveryOp } from '../scripts/wire-examples'
import { OPS } from '../src/prelude/ops.generated'
import { json, loadPrelude, SEED } from './support/prelude-runtime'

type Part = { name: string; source: string; body: unknown }
type Chunk = { parts: Part[]; capture?: number }
type LocatorTarget = {
  $neo: 'locator'
  page: number
  frame: string | null
  chunks: Chunk[]
  display: string
}

/** Each chunk as `name=source` parts, the readable form of a frame's selector. */
function frames(target: unknown): string[][] {
  return (target as LocatorTarget).chunks.map((chunk) =>
    chunk.parts.map((part) => `${part.name}=${part.source}`),
  )
}

/** The parts of the nested selector inside `internal:has` (its parsed body). */
function nested(part: Part | undefined): string[] {
  const body = part?.body as { parsed: Chunk }
  return body.parsed.parts.map((inner) => `${inner.name}=${inner.source}`)
}

async function sentBy(code: string, answer?: (name: string) => unknown) {
  const runtime = loadPrelude({
    seed: SEED,
    answer: (call) => answer?.(call.name) ?? null,
  })
  await runtime.run(code)
  return runtime
}

describe('locator targets', () => {
  it('tests that a role locator click is one strict op with the parsed selector and its display', async () => {
    const runtime = await sentBy(
      "await page.getByRole('button', { name: 'Sign in' }).click()",
    )
    expect(runtime.calls).toEqual([
      {
        name: 'locator.click',
        input: {
          call: { api: 'locator.click', timeoutMs: 5000 },
          target: {
            $neo: 'locator',
            page: 7,
            frame: null,
            chunks: [
              {
                parts: [
                  {
                    name: 'internal:role',
                    body: 'button[name="Sign in"i]',
                    source: 'button[name="Sign in"i]',
                  },
                ],
              },
            ],
            display: "getByRole('button', { name: 'Sign in' })",
          },
          strict: 'strict',
          options: {},
        },
      },
    ])
  })

  it('tests that page shortcuts take the first match and name themselves, unless strict', async () => {
    const runtime = await sentBy(
      "await page.click('#go'); await page.click('#go', { strict: true }); await page.mainFrame().click('#go')",
    )
    const sent = runtime.sent('locator.click').map((call) => ({
      api: (call.input.call as { api: string }).api,
      strict: call.input.strict,
      frames: frames(call.input.target),
    }))
    expect(sent).toEqual([
      { api: 'page.click', strict: 'firstMatch', frames: [['css=#go']] },
      { api: 'page.click', strict: 'strict', frames: [['css=#go']] },
      { api: 'frame.click', strict: 'firstMatch', frames: [['css=#go']] },
    ])
  })

  it('tests that frame locator chains split into one chunk per frame', async () => {
    const runtime = await sentBy(
      [
        "await page.frameLocator('#outer').frameLocator('#inner').getByLabel('Card number').fill('4242 4242')",
        "await page.locator('#pay').contentFrame().getByRole('button').click()",
        "await page.frameLocator('iframe').nth(1).getByText('Pay').click()",
      ].join('\n'),
    )
    const [fill, contentFrame, nth] = runtime.calls
    expect(fill?.name).toBe('locator.fill')
    expect(fill?.input.value).toBe('4242 4242')
    expect(frames(fill?.input.target)).toEqual([
      ['css=#outer'],
      ['css=#inner'],
      ['internal:label="Card number"i'],
    ])
    expect(frames(contentFrame?.input.target)).toEqual([
      ['css=#pay'],
      ['internal:role=button'],
    ])
    expect(frames(nth?.input.target)).toEqual([
      ['css=iframe', 'nth=1'],
      ['internal:text="Pay"i'],
    ])
    expect((fill?.input.target as LocatorTarget | undefined)?.display).toBe(
      "locator('#outer').contentFrame().locator('#inner').contentFrame().getByLabel('Card number')",
    )
  })

  it('tests that a has filter inside a frame keeps the parse that strips the nested frame prefix', async () => {
    const runtime = await sentBy(
      "await page.frameLocator('#f').locator('tr').filter({ has: page.frameLocator('#f').getByText('Dana') }).click()",
    )
    const target = runtime.calls[0]?.input.target as LocatorTarget
    expect(frames(target)[0]).toEqual(['css=#f'])
    const [row, has] = target.chunks[1]?.parts ?? []
    expect(row?.name).toBe('css')
    expect(has?.name).toBe('internal:has')
    expect(nested(has)).toEqual(['internal:text="Dana"i'])
  })

  it('tests that getByRef sends an aria-ref part that leads its chunk, as Rust jumps on', async () => {
    const runtime = await sentBy(
      [
        "await page.getByRef('f2e7').click()",
        "await page.getByRef('e5').getByRole('button').click()",
        "await page.locator('li').filter({ has: page.getByRef('e7') }).click()",
      ].join('\n'),
    )
    const [frameRef, mainRef, hasRef] = runtime.calls.map(
      (call) => call.input.target,
    )
    expect(frames(frameRef)).toEqual([['aria-ref=f2e7']])
    expect((frameRef as LocatorTarget).display).toBe("locator('aria-ref=f2e7')")
    expect(frames(mainRef)).toEqual([['aria-ref=e5', 'internal:role=button']])
    expect(nested((hasRef as LocatorTarget).chunks[0]?.parts[1])).toEqual([
      'aria-ref=e7',
    ])
  })

  it('tests that every part has exactly the fields target.rs decodes', async () => {
    const runtime = await sentBy(
      "await page.frameLocator('#f').getByRole('row').filter({ hasText: 'Dana' }).nth(2).click()",
    )
    const target = runtime.calls[0]?.input.target as LocatorTarget
    expect(Object.keys(target).sort()).toEqual([
      '$neo',
      'chunks',
      'display',
      'frame',
      'page',
    ])
    for (const chunk of target.chunks) {
      expect(Object.keys(chunk).filter((key) => key !== 'capture')).toEqual([
        'parts',
      ])
      for (const part of chunk.parts)
        expect(Object.keys(part).sort()).toEqual(['body', 'name', 'source'])
    }
  })

  it('tests that options are filtered to the keys each op names, as upstream drops unknown keys', async () => {
    const runtime = await sentBy(
      [
        "await page.getByTestId('save').dblclick({ button: 'right', clickCount: 3, waitFor: 'visible' })",
        "await page.getByRole('checkbox').check({ force: true, timeout: 1000, button: 'left' })",
        "await page.getByRole('textbox').press('Control+A', { delay: 10 })",
      ].join('\n'),
    )
    expect(
      runtime.calls.map((call) => [call.name, call.input.options]),
    ).toEqual([
      ['locator.dblclick', { button: 'right' }],
      ['locator.check', { force: true }],
      ['locator.press', { delay: 10 }],
    ])
    expect(runtime.calls[1]?.input.call).toEqual({
      api: 'locator.check',
      timeoutMs: 1000,
    })
    expect(runtime.calls[2]?.input.key).toBe('Control+A')
  })

  it('tests that the aliases send their op with the method the script called', async () => {
    const runtime = await sentBy(
      [
        "await page.getByRole('textbox').clear()",
        "await page.getByRole('textbox').type('hi')",
        "await page.getByRole('checkbox').setChecked(false)",
        "await page.locator('#src').dragTo(page.locator('#dst'))",
      ].join('\n'),
    )
    expect(
      runtime.calls.map((call) => [
        call.name,
        (call.input.call as { api: string }).api,
      ]),
    ).toEqual([
      ['locator.fill', 'locator.clear'],
      ['locator.pressSequentially', 'locator.type'],
      ['locator.uncheck', 'locator.setChecked'],
      ['locator.dragTo', 'locator.dragTo'],
    ])
    expect(runtime.calls[0]?.input.value).toBe('')
    expect(frames(runtime.calls[3]?.input.to)).toEqual([['css=#dst']])
  })

  it('tests that a child frame from the frame tree sends its frame id', async () => {
    const runtime = await sentBy(
      "await page.goto('https://shop.example/pay'); await page.frame('pay').locator('#card').fill('x')",
      (name) =>
        name === 'page.goto'
          ? {
              response: { url: 'https://shop.example/pay', status: 200 },
              url: 'https://shop.example/pay',
              frames: [
                {
                  id: 'MAIN',
                  parentId: null,
                  name: '',
                  url: 'https://shop.example/pay',
                },
                {
                  id: 'F1',
                  parentId: 'MAIN',
                  name: 'pay',
                  url: 'https://pay.example/form',
                },
              ],
            }
          : null,
    )
    const fill = runtime.sent('locator.fill')[0]
    expect((fill?.input.target as LocatorTarget | undefined)?.frame).toBe('F1')
  })

  it('tests that an element handle acts on its pinned element', async () => {
    const element = {
      $neo: 'element',
      page: 7,
      seq: 4,
      node: 1234,
      display: '<button>Save</button>',
    }
    const runtime = await sentBy(
      "const save = await page.$('#save'); await save.click(); return String(save)",
      (name) => (name === 'locator.elementHandle' ? element : null),
    )
    expect(runtime.calls.map((call) => call.name)).toEqual([
      'locator.elementHandle',
      'locator.click',
    ])
    expect(runtime.calls[0]?.input).toMatchObject({
      call: { api: 'page.$', timeoutMs: 0 },
      strict: 'firstMatch',
      noWait: true,
    })
    expect(runtime.calls[1]?.input).toMatchObject({
      call: { api: 'elementHandle.click', timeoutMs: 5000 },
      target: element,
      strict: 'strict',
    })
  })

  it('tests that reads of every match carry no strictness and never wait', async () => {
    const runtime = await sentBy(
      "await page.locator('li').count(); await page.locator('li').allTextContents(); await page.locator('li').isVisible()",
      (name) =>
        name === 'locator.count'
          ? 3
          : name === 'locator.allTextContents'
            ? []
            : false,
    )
    expect(
      runtime.calls.map((call) => [
        call.name,
        call.input.call,
        call.input.strict,
      ]),
    ).toEqual([
      ['locator.count', { api: 'locator.count', timeoutMs: 0 }, undefined],
      [
        'locator.allTextContents',
        { api: 'locator.allTextContents', timeoutMs: 0 },
        undefined,
      ],
      [
        'locator.isVisible',
        { api: 'locator.isVisible', timeoutMs: 0 },
        'strict',
      ],
    ])
  })
})

describe('timeouts', () => {
  async function timeouts(code: string): Promise<[string, number][]> {
    const runtime = await sentBy(code, (name) =>
      name === 'page.goto' ? { frames: [] } : null,
    )
    return runtime.calls.map((call) => [
      `${(call.input.call as { api: string }).api}`,
      (call.input.call as { timeoutMs: number }).timeoutMs,
    ])
  }

  it("tests that Neo's defaults are 5 s for actions and 30 s for navigation, and none for instant calls", async () => {
    expect(
      await timeouts(
        "await page.click('#a'); await page.goto('https://a.example/'); await page.title(); await page.keyboard.press('a')",
      ),
    ).toEqual([
      ['page.click', 5000],
      ['page.goto', 30000],
      ['page.title', 0],
      ['keyboard.press', 0],
    ])
  })

  it("tests that the page's default also bounds its navigations, until a navigation default is set", async () => {
    expect(
      await timeouts(
        [
          'page.setDefaultTimeout(1234)',
          "await page.click('#a')",
          "await page.goto('https://a.example/')",
          'page.setDefaultNavigationTimeout(9000)',
          "await page.goto('https://a.example/')",
          "await page.click('#a')",
        ].join('\n'),
      ),
    ).toEqual([
      ['page.click', 1234],
      ['page.goto', 1234],
      ['page.goto', 9000],
      ['page.click', 1234],
    ])
  })

  it("tests that the context's defaults apply under the page's, and the call option wins", async () => {
    expect(
      await timeouts(
        [
          'context.setDefaultTimeout(2000)',
          'context.setDefaultNavigationTimeout(7000)',
          "await page.click('#a')",
          "await page.goto('https://a.example/')",
          'page.setDefaultTimeout(1500)',
          "await page.goto('https://a.example/')",
          "await page.click('#a', { timeout: 42 })",
          "await page.getByRole('button').click({ timeout: 0 })",
        ].join('\n'),
      ),
    ).toEqual([
      ['page.click', 2000],
      ['page.goto', 7000],
      ['page.goto', 1500],
      ['page.click', 42],
      ['locator.click', 0],
    ])
  })

  it('tests that a timeout the header cannot carry is refused before anything is sent', async () => {
    const runtime = loadPrelude({ seed: SEED })
    const error = json(
      await runtime.run(
        "try { await page.click('#a', { timeout: -1 }) } catch (e) { return { message: e.message, code: e.code, delivery: e.delivery } }",
      ),
    )
    expect(error).toEqual({
      message: 'page.click: timeout: expected a non-negative number, got -1',
      code: 'invalid_input',
      delivery: 'not_sent',
    })
    expect(runtime.calls).toEqual([])
  })
})

describe('errors', () => {
  const caught = `try { await page.click('#go') } catch (e) {
    return {
      name: e.name, message: e.message, bootstrapClass: e instanceof TimeoutError,
      code: e.code, owner: e.owner, delivery: e.delivery, retryable: e.retryable,
      operationId: e.operationId, hostCode: e.hostCode, timeout: e.timeout,
    }
  }`

  it("tests that a timed-out op reaches the script as the bootstrap's TimeoutError with every field", async () => {
    const runtime = loadPrelude({
      seed: SEED,
      answer: () => ({
        err: {
          code: 'not_actionable',
          owner: 'website',
          delivery: 'unknown',
          retryable: false,
          operationId: 'run.3',
          hostCode: 'none',
          timeout: true,
          message:
            "page.click: Timeout 5000ms exceeded.\nCall log:\n  - waiting for locator('#go')",
        },
      }),
    })
    expect(json(await runtime.run(caught))).toEqual({
      name: 'TimeoutError',
      message:
        "page.click: Timeout 5000ms exceeded.\nCall log:\n  - waiting for locator('#go')",
      bootstrapClass: true,
      code: 'not_actionable',
      owner: 'website',
      delivery: 'unknown',
      retryable: false,
      operationId: 'run.3',
      hostCode: 'none',
      timeout: true,
    })
  })

  it("tests that a host refusal named after the op is renamed to the script's method", async () => {
    const runtime = loadPrelude({
      seed: SEED,
      answer: () => ({
        err: {
          code: 'owner_has_control',
          owner: 'host',
          delivery: 'not_sent',
          retryable: true,
          operationId: 'run.4',
          timeout: false,
          message: 'locator.click: the owner is in control of this tab',
        },
      }),
    })
    expect(json(await runtime.run(caught))).toMatchObject({
      name: 'Error',
      message: 'page.click: the owner is in control of this tab',
      code: 'owner_has_control',
      owner: 'host',
      retryable: true,
    })
  })

  it('tests that an unprefixed message gets the method first', async () => {
    const runtime = loadPrelude({
      seed: SEED,
      answer: () => ({
        err: {
          code: 'target_closed',
          owner: 'neo',
          delivery: 'unknown',
          retryable: false,
          operationId: 'run.5',
          timeout: false,
          message: 'Target page, context or browser has been closed',
        },
      }),
    })
    expect(json(await runtime.run(caught))).toMatchObject({
      name: 'Error',
      message: 'page.click: Target page, context or browser has been closed',
      code: 'target_closed',
    })
  })

  it('tests that a selector the vendored parser rejects fails as invalid input without an op', async () => {
    const runtime = loadPrelude({ seed: SEED })
    const error = json(
      await runtime.run(
        "try { await page.locator('#pay >> internal:control=enter-frame').click() } catch (e) { return { message: e.message, code: e.code, owner: e.owner, delivery: e.delivery } }",
      ),
    ) as { message: string }
    expect(error).toMatchObject({
      code: 'invalid_input',
      owner: 'script',
      delivery: 'not_sent',
    })
    expect(error.message).toStartWith(
      'locator.click: Selector cannot end with entering frame',
    )
    expect(runtime.calls).toEqual([])
  })

  it('tests that an any-frame locator is unsupported, with the selector form to use', async () => {
    const runtime = loadPrelude({ seed: SEED })
    const error = json(
      await runtime.run(
        "try { await page.frameLocator().getByText('Pay').click() } catch (e) { return { message: e.message, code: e.code, owner: e.owner } }",
      ),
    ) as { message: string }
    expect(error).toMatchObject({ code: 'unsupported', owner: 'neo' })
    expect(error.message).toStartWith(
      'locator.click: frameLocator() needs a selector in Neo',
    )
  })

  it('tests that a handle passed to evaluate is refused with what to call instead', async () => {
    const runtime = loadPrelude({ seed: SEED })
    const error = json(
      await runtime.run(
        "try { await page.evaluate((el) => el.id, page.locator('#a')) } catch (e) { return { message: e.message, code: e.code } }",
      ),
    ) as { message: string; code: string }
    expect(error.code).toBe('unsupported')
    expect(error.message).toStartWith(
      'page.evaluate: Neo passes evaluate arguments as JSON',
    )
    expect(runtime.calls).toEqual([])
  })

  it('tests that screenshot options Neo cannot honor are refused, and the rest are sent flat', async () => {
    const runtime = loadPrelude({ seed: SEED })
    const refused = json(
      await runtime.run(`
        const refused = []
        for (const options of [{ mask: [page.locator('#a')] }, { style: 'p {}' }, { omitBackground: true },
                               { animations: 'disabled' }, { scale: 'css' }])
          try { await page.screenshot(options) } catch (e) { refused.push(e.code) }
        await page.locator('#a').screenshot({ type: 'jpeg', quality: 50, caret: 'initial', animations: 'allow' })
        return refused
      `),
    )
    expect(refused).toEqual(Array(5).fill('unsupported'))
    expect(runtime.calls.map((call) => call.input)).toEqual([
      {
        call: { api: 'locator.screenshot', timeoutMs: 5000 },
        page: { $neo: 'page', id: 7, tab: 1007 },
        target: expect.objectContaining({ display: "locator('#a')" }),
        strict: 'strict',
        type: 'jpeg',
        quality: 50,
      },
    ])
  })
})

describe('synchronous state', () => {
  it('tests that the seed gives the default page, the open tabs and the browser version', async () => {
    const runtime = loadPrelude({
      seed: {
        page: 7,
        pages: [
          { id: 7, tab: 1007, url: 'https://shop.example/cart', title: 'Cart' },
          { id: 9, tab: 1009, url: 'https://mail.example/', title: 'Mail' },
        ],
        browserVersion: '146.0.7680.0',
      },
    })
    expect(
      json(
        await runtime.run(
          'return { url: page.url(), pages: context.pages().map((p) => p.url()), same: context.pages()[0] === page, version: browser.version(), contexts: browser.contexts().length }',
        ),
      ),
    ).toEqual({
      url: 'https://shop.example/cart',
      pages: ['https://shop.example/cart', 'https://mail.example/'],
      same: true,
      version: '146.0.7680.0',
      contexts: 1,
    })
  })

  it("tests that every envelope's page facts refresh url() and isClosed()", async () => {
    const runtime = loadPrelude({
      seed: SEED,
      answer: () => ({
        ok: null,
        pages: [
          {
            id: 7,
            url: 'https://shop.example/pay',
            title: 'Pay',
            closed: true,
          },
        ],
      }),
    })
    expect(
      json(
        await runtime.run(
          "let closed = 0; page.on('close', () => closed++); await page.click('#pay'); return { url: page.url(), closed: page.isClosed(), events: closed }",
        ),
      ),
    ).toEqual({ url: 'https://shop.example/pay', closed: true, events: 1 })
  })

  it('tests that a navigation result refreshes the URL and the frame tree, and returns the response', async () => {
    const runtime = loadPrelude({
      seed: SEED,
      answer: (call) =>
        call.name === 'page.goto'
          ? {
              response: {
                url: 'https://a.example/',
                status: 404,
                statusText: 'Not Found',
                ok: false,
                headers: {},
              },
              url: 'https://a.example/',
              frames: [
                {
                  id: 'MAIN',
                  parentId: null,
                  name: '',
                  url: 'https://a.example/',
                },
                {
                  id: 'F1',
                  parentId: 'MAIN',
                  name: 'ad',
                  url: 'https://ads.example/',
                },
                { id: 'F2', parentId: 'F1', name: 'inner', url: 'about:blank' },
              ],
            }
          : null,
    })
    expect(
      json(
        await runtime.run(
          "const response = await page.goto('https://a.example/'); const [, ad, inner] = page.frames(); return { url: page.url(), frames: page.frames().map((f) => f.name()), parent: inner.parentFrame() === ad, child: ad.childFrames()[0] === inner, status: response.status(), ok: response.ok(), statusText: response.statusText() }",
        ),
      ),
    ).toEqual({
      url: 'https://a.example/',
      frames: ['', 'ad', 'inner'],
      parent: true,
      child: true,
      status: 404,
      ok: false,
      statusText: 'Not Found',
    })
    expect(runtime.calls.map((call) => call.name)).toEqual(['page.goto'])
  })

  it('tests that a navigation result without a frame tree is followed by page.frames', async () => {
    const runtime = loadPrelude({
      seed: SEED,
      answer: (call) =>
        call.name === 'page.frames'
          ? {
              frames: [
                {
                  id: 'M',
                  parentId: null,
                  name: '',
                  url: 'https://b.example/',
                },
              ],
            }
          : { response: null },
    })
    expect(
      await runtime.run(
        "await page.goto('https://b.example/'); return page.url()",
      ),
    ).toBe('https://b.example/')
    expect(runtime.calls.map((call) => call.name)).toEqual([
      'page.goto',
      'page.frames',
    ])
  })

  it('tests that the lazy default page opens one tab for racing first ops, and keeps its identity', async () => {
    const runtime = loadPrelude({
      answer: (call) =>
        call.name === 'context.newPage'
          ? { page: { $neo: 'page', id: 12, tab: 1012 }, url: 'about:blank' }
          : null,
    })
    const result = json(
      await runtime.run(
        "const before = context.pages().length; await Promise.all([page.click('#a'), page.click('#b')]); return { before, after: context.pages().length, same: context.pages()[0] === page }",
      ),
    )
    expect(result).toEqual({ before: 0, after: 1, same: true })
    expect(runtime.calls.map((call) => call.name)).toEqual([
      'context.newPage',
      'locator.click',
      'locator.click',
    ])
    expect(
      runtime
        .sent('locator.click')
        .map((call) => (call.input.target as LocatorTarget).page),
    ).toEqual([12, 12])
  })

  it('tests that page brands carry the tab the host named, and a page with none is refused unsent', async () => {
    const runtime = loadPrelude({
      seed: { page: 7, pages: [{ id: 7, url: 'https://a.example/' }] },
      answer: (call) =>
        call.name === 'context.newPage'
          ? {
              ok: {
                page: { $neo: 'page', id: 8, tab: 1008 },
                url: 'about:blank',
              },
              pages: [{ id: 7, tab: 1007 }],
            }
          : null,
    })
    const refused = json(
      await runtime.run(
        'try { await page.reload() } catch (e) { return { code: e.code, message: e.message } }',
      ),
    )
    expect(refused).toEqual({
      code: 'invalid_input',
      message:
        'page.reload: Neo has no tab id for this page; get the page again from context.pages()',
    })
    expect(runtime.calls).toEqual([])
    await runtime.run(
      'const tab = await context.newPage(); await page.title(); await tab.title()',
    )
    expect(runtime.sent('page.title').map((call) => call.input.page)).toEqual([
      { $neo: 'page', id: 7, tab: 1007 },
      { $neo: 'page', id: 8, tab: 1008 },
    ])
  })

  it('tests that newPage and opener return Page objects, one per page id', async () => {
    const runtime = loadPrelude({
      seed: SEED,
      revive: false,
      answer: (call) =>
        call.name === 'context.newPage'
          ? { page: { $neo: 'page', id: 21, tab: 1021 }, url: 'about:blank' }
          : call.name === 'page.opener'
            ? { opener: { $neo: 'page', id: 7, tab: 1007 } }
            : null,
    })
    expect(
      json(
        await runtime.run(
          'const tab = await context.newPage(); const opener = await tab.opener(); return { listed: context.pages().includes(tab), opener: opener === page }',
        ),
      ),
    ).toEqual({ listed: true, opener: true })
  })
})

describe('the op table', () => {
  it('tests that every op carries its page handle in the field the op table declares', async () => {
    const calls = await recordEveryOp()
    const misplaced: string[] = []
    for (const call of calls) {
      const op = OPS[call.name as keyof typeof OPS]
      if (!op) {
        misplaced.push(`${call.name} is not in the op table`)
        continue
      }
      const field = op.target
      if (field === null) continue
      const handle = call.input[field] as
        | { $neo?: string; page?: unknown; id?: unknown }
        | undefined
      const page = field === 'page' ? handle?.id : handle?.page
      if (
        typeof page !== 'number' ||
        (field === 'page' && handle?.$neo !== 'page')
      )
        misplaced.push(`${call.name}: no page handle in ${field}`)
    }
    expect(misplaced).toEqual([])
    const sent = new Set(calls.map((call) => call.name))
    expect(Object.keys(OPS).filter((name) => !sent.has(name))).toEqual([
      'context.pages',
    ])
  })
})
