// What Rust receives for a script's Playwright calls: op names, the `call` header (the method
// the script called and its resolved timeout), the target handles target.rs decodes, and the
// prelude's own refusals. Every op goes through claw-sdk's real bootstrap, which carries JSON
// only, over a test host (support/prelude-runtime.ts).
import { describe, expect, it } from 'bun:test'
import { recordEveryOp } from '../scripts/wire-examples'
import { OPS } from '../src/prelude/generated/ops'
import {
  type Call,
  isRefusal,
  json,
  loadPrelude,
  refusalCall,
  SEED,
} from './support/prelude-runtime'

type Part = { name: string; source: string; body: unknown }
type Chunk = { parts: Part[]; capture?: number }
type LocatorTarget = {
  $neo: 'locator'
  page: { $neo: 'page'; tab: number | null }
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
            page: { $neo: 'page', tab: 1007 },
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
      page: { $neo: 'page', tab: 1007 },
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
    expect(runtime.calls).toEqual([
      refusalCall(
        'page.click',
        'invalid_input',
        'page.click: timeout: expected a non-negative number, got -1',
      ),
    ])
  })
})

describe('errors', () => {
  const caught = `try { await page.click('#go') } catch (e) {
    return {
      name: e.name, message: e.message, bootstrapClass: e instanceof TimeoutError,
      code: e.code, owner: e.owner, delivery: e.delivery, retryable: e.retryable,
      operationId: e.operationId, timeout: e.timeout,
    }
  }`

  it("tests that a timed-out op reaches the script as the bootstrap's TimeoutError with every field", async () => {
    const runtime = loadPrelude({
      seed: SEED,
      // Every field claw-sdk's bridge sends (runtime/bridge.rs `ErrBody`).
      answer: () => ({
        err: {
          code: 'not_actionable',
          owner: 'website',
          delivery: 'unknown',
          retryable: false,
          operationId: 'run.3',
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

  it("tests that an uncaught op failure settles the run as that op's failure", async () => {
    const failure = {
      code: 'not_actionable',
      owner: 'website',
      delivery: 'unknown',
      retryable: false,
      operationId: 'run.1',
      timeout: true,
      message: 'page.click: Timeout 50ms exceeded.',
    }
    const runtime = loadPrelude({
      seed: SEED,
      answer: () => ({ err: failure }),
    })
    expect(
      await runtime.settle("await page.click('#go', { timeout: 50 })"),
    ).toEqual({ operation: failure })
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

  it('tests that closing a page that is already gone is not an error, as upstream', async () => {
    const runtime = loadPrelude({
      seed: SEED,
      answer: (call) =>
        call.name === 'page.close'
          ? {
              pages: [{ tab: 1007, closed: true }],
              err: {
                code: 'target_closed',
                owner: 'neo',
                delivery: 'unknown',
                retryable: false,
                operationId: 'run.1',
                timeout: false,
                message: 'Target page, context or browser has been closed',
              },
            }
          : null,
    })
    expect(
      json(await runtime.run('await page.close(); return page.isClosed()')),
    ).toBe(true)
    expect(runtime.sent('page.close')).toHaveLength(1)
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
    expect(runtime.calls).toEqual([
      refusalCall('locator.click', 'invalid_input', error.message),
    ])
  })

  it('tests that an any-frame locator is unsupported, with the selector form to use', async () => {
    const runtime = loadPrelude({ seed: SEED })
    const error = json(
      await runtime.run(
        "try { await page.frameLocator().getByText('Pay').click() } catch (e) { return { message: e.message, code: e.code, owner: e.owner } }",
      ),
    ) as { message: string }
    expect(error).toMatchObject({ code: 'method_unavailable', owner: 'script' })
    expect(error.message).toStartWith(
      'locator.click: frameLocator() needs a selector in Neo',
    )
  })

  it('tests that a locator passed to evaluate is refused with what to pass instead', async () => {
    const runtime = loadPrelude({ seed: SEED })
    const error = json(
      await runtime.run(
        "try { await page.evaluate((el) => el.id, page.locator('#a')) } catch (e) { return { message: e.message, code: e.code } }",
      ),
    ) as { message: string; code: string }
    expect(error.code).toBe('method_unavailable')
    expect(error.message).toStartWith(
      'page.evaluate: Neo passes element handles as evaluate arguments, not pages',
    )
    expect(runtime.calls).toEqual([
      refusalCall('page.evaluate', 'method_unavailable', error.message),
    ])
  })

  it('tests that an element handle passes as upstream passes one, and a disposed one fails its guid', async () => {
    const runtime = loadPrelude({
      seed: SEED,
      answer: (call) =>
        call.name === 'locator.elementHandle'
          ? {
              $neo: 'element',
              page: { $neo: 'page', tab: 1007 },
              seq: 2,
              node: 41,
              display: '<b>',
            }
          : call.name === 'page.evaluate'
            ? { value: null }
            : null,
    })
    const result = json(
      await runtime.run(`
        const handle = await page.locator('b').elementHandle()
        await page.evaluate(([a, b]) => a === b, [handle, handle])
        await handle.dispose()
        let disposed
        try { await page.evaluate((h) => h.id, { h: handle }) } catch (e) { disposed = { message: e.message, code: e.code } }
        let called
        try { await handle.evaluate((e) => e.id) } catch (e) { called = e.message }
        return { disposed, called }
      `),
    ) as { disposed: { message: string; code: string }; called: string }
    const sent = runtime.calls.find((call) => call.name === 'page.evaluate')
    expect(sent?.input).toMatchObject({
      script: {
        run: {
          arg: '{"a":[{"h":0},{"h":1}],"id":1}',
          handles: [
            { $neo: 'element', seq: 2, node: 41 },
            { $neo: 'element', seq: 2, node: 41 },
          ],
        },
      },
    })
    expect(result.disposed.code).toBe('invalid_input')
    expect(result.disposed.message).toMatch(
      /^page\.evaluate: arg\.handles\[0\]: no object with guid handle@[0-9a-f]{32}$/,
    )
    expect(result.called).toBe(
      'elementHandle.evaluate: Target page, context or browser has been closed',
    )
    expect(
      runtime.calls.filter(
        (call) => call.name === 'page.evaluate' && !isRefusal(call),
      ),
    ).toHaveLength(1)
  })

  it('tests that screenshot masks cross as locator brands and encoding options are sent flat', async () => {
    const runtime = loadPrelude({ seed: SEED })
    await runtime.run(`
      await page.screenshot({ mask: [page.locator('#hidden')], maskColor: '#00ff00' })
      await page.locator('#a').screenshot({ type: 'webp', quality: 50, omitBackground: true, caret: 'initial', animations: 'disabled', style: 'p {}', scale: 'css', mask: [page.locator('#b')] })
    `)
    expect(runtime.calls.filter(isRefusal)).toEqual([])
    expect(
      runtime.calls
        .filter((call) => !isRefusal(call))
        .map((call) => call.input),
    ).toEqual([
      {
        call: { api: 'page.screenshot', timeoutMs: 5000 },
        page: { $neo: 'page', tab: 1007 },
        maskColor: '#00ff00',
        mask: [
          expect.objectContaining({
            $neo: 'locator',
            display: "locator('#hidden')",
            chunks: expect.any(Array),
          }),
        ],
      },
      {
        call: { api: 'locator.screenshot', timeoutMs: 5000 },
        page: { $neo: 'page', tab: 1007 },
        target: expect.objectContaining({ display: "locator('#a')" }),
        strict: 'strict',
        type: 'webp',
        omitBackground: true,
        quality: 50,
        caret: 'initial',
        animations: 'disabled',
        style: 'p {}',
        scale: 'css',
        mask: [
          expect.objectContaining({
            $neo: 'locator',
            display: "locator('#b')",
            chunks: expect.any(Array),
          }),
        ],
      },
    ])
  })
})

describe('synchronous state', () => {
  it("tests that the run's init names the agent's page, and reply facts list the other tabs", async () => {
    const runtime = loadPrelude({
      seed: SEED,
      // The run's frames read at its start (FR-1) answers with no facts, so `before` is what
      // the run's init alone gives.
      answer: (call) =>
        call.name === 'page.frames'
          ? null
          : {
              ok: 'Cart',
              pages: [
                {
                  tab: 1007,
                  url: 'https://shop.example/cart',
                  title: 'Cart',
                },
                {
                  tab: 1009,
                  url: 'https://mail.example/',
                  title: 'Mail',
                },
              ],
            },
    })
    expect(
      json(
        await runtime.run(
          'const before = context.pages().map((p) => p.url()); await page.title(); return { before, url: page.url(), pages: context.pages().map((p) => p.url()), same: context.pages()[0] === page, version: browser.version(), contexts: browser.contexts().length }',
        ),
      ),
    ).toEqual({
      before: ['about:blank'],
      url: 'https://shop.example/cart',
      pages: ['https://shop.example/cart', 'https://mail.example/'],
      same: true,
      version: '',
      contexts: 1,
    })
    expect(runtime.sent('page.title').map((call) => call.input.page)).toEqual([
      { $neo: 'page', tab: 1007 },
    ])
  })

  it("tests that the tabs the run's init lists are in context.pages() before any op", async () => {
    const runtime = loadPrelude({
      seed: {
        ...SEED,
        pages: [
          {
            tab: 1007,
            url: 'https://shop.example/cart',
            title: 'Cart',
            closed: false,
          },
          {
            tab: 1009,
            url: 'https://mail.example/',
            title: 'Mail',
            closed: false,
          },
        ],
      },
    })
    expect(
      json(
        await runtime.run(
          'const listed = context.pages().map((p) => ({ url: p.url(), brand: p.toJSON(), run: p === page })); await context.pages()[1].title(); return listed',
        ),
      ),
    ).toEqual([
      {
        url: 'https://shop.example/cart',
        brand: { $neo: 'page', tab: 1007 },
        run: true,
      },
      {
        url: 'https://mail.example/',
        brand: { $neo: 'page', tab: 1009 },
        run: false,
      },
    ])
    expect(runtime.calls.map((call) => [call.name, call.input.page])).toEqual([
      ['page.title', { $neo: 'page', tab: 1009 }],
    ])
  })

  it("tests that every envelope's page facts refresh url() and isClosed()", async () => {
    const runtime = loadPrelude({
      seed: SEED,
      // The click's reply closes the page, after the script listens; the run's frames read at
      // its start (FR-1) answers with no facts.
      answer: (call) =>
        call.name === 'page.frames'
          ? null
          : {
              ok: null,
              pages: [
                {
                  tab: 1007,
                  url: 'https://shop.example/pay',
                  title: 'Pay',
                  closed: true,
                },
              ],
            },
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

  const unopened = { $neo: 'page', tab: null } as const
  const opened = { $neo: 'page', tab: 1012 } as const

  /**
   * claw-sdk's door for a run with no tab yet: an op whose target names the unopened page gets
   * the tab it opened (once, however many race) and names it in the reply's `defaultPage`.
   */
  const lazyDoor = (ok: (call: Call) => unknown) => (call: Call) => {
    const target = call.input.target as LocatorTarget | undefined
    const page = (call.input.page ?? target?.page) as { tab?: unknown }
    return page?.tab === null
      ? {
          ok: ok(call),
          defaultPage: { tab: 1012 },
          pages: [{ tab: 1012, url: 'about:blank', title: '' }],
        }
      : { ok: ok(call) }
  }

  it("tests that the run's page names itself unopened until a reply's defaultPage binds its tab", async () => {
    const runtime = loadPrelude({ answer: lazyDoor(() => null) })
    const result = json(
      await runtime.run(
        "const before = { pages: context.pages().length, brand: JSON.stringify(page) }; await Promise.all([page.click('#a'), page.click('#b')]); await page.click('#c'); return { before, after: context.pages().length, same: context.pages()[0] === page, brand: JSON.stringify(page) }",
      ),
    )
    expect(result).toEqual({
      before: { pages: 0, brand: JSON.stringify(unopened) },
      after: 1,
      same: true,
      brand: JSON.stringify(opened),
    })
    expect(runtime.calls.map((call) => call.name)).toEqual([
      'locator.click',
      'locator.click',
      'locator.click',
    ])
    expect(
      runtime
        .sent('locator.click')
        .map((call) => (call.input.target as LocatorTarget).page),
    ).toEqual([unopened, unopened, opened])
  })

  it("tests that an event op on the run's unopened page gets its tab through the door first", async () => {
    const popup = {
      seq: 1,
      event: 'popup',
      page: opened,
      popup: {
        page: { $neo: 'page', tab: 1013 },
        url: 'https://ads.example/',
      },
    }
    const runtime = loadPrelude({
      answer: lazyDoor((call) => (call.name === 'events.wait' ? popup : null)),
    })
    const result = json(
      await runtime.run(
        "const popup = await page.waitForEvent('popup'); return { url: popup.url(), listed: context.pages().length }",
      ),
    )
    expect(result).toEqual({ url: 'https://ads.example/', listed: 2 })
    expect(runtime.calls.map((call) => [call.name, call.input.page])).toEqual([
      ['page.frames', unopened],
      ['events.wait', opened],
    ])
  })

  it('tests that page brands carry the tab the host named, and a page with none is refused unsent', async () => {
    const runtime = loadPrelude({
      seed: SEED,
      answer: (call) =>
        call.name === 'context.newPage'
          ? {
              page: { $neo: 'page', tab: 1008 },
              url: 'about:blank',
            }
          : call.name === 'page.opener'
            ? { opener: { $neo: 'page', id: 9 } }
            : null,
    })
    expect(await runtime.run('return (await page.opener()) === null')).toBe(
      true,
    )
    expect(runtime.calls.map((call) => call.name)).toEqual(['page.opener'])
    await runtime.run(
      'const tab = await context.newPage(); await page.title(); await tab.title()',
    )
    expect(runtime.sent('page.title').map((call) => call.input.page)).toEqual([
      { $neo: 'page', tab: 1007 },
      { $neo: 'page', tab: 1008 },
    ])
  })

  it('tests that newPage and opener return Page objects, one per browser tab', async () => {
    const runtime = loadPrelude({
      seed: SEED,
      answer: (call) =>
        call.name === 'context.newPage'
          ? { page: { $neo: 'page', tab: 1021 }, url: 'about:blank' }
          : call.name === 'page.opener'
            ? { opener: { $neo: 'page', tab: 1007 } }
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
      // The door reads the page brand at `page`, or at `target.page` for an element op.
      type Brand = { $neo?: string; tab?: unknown }
      const handle = call.input[field] as (Brand & { page?: Brand }) | undefined
      const page = field === 'page' ? handle : handle?.page
      if (page?.$neo !== 'page' || typeof page.tab !== 'number')
        misplaced.push(`${call.name}: no page brand in ${field}`)
    }
    expect(misplaced).toEqual([])
    const sent = new Set(calls.map((call) => call.name))
    expect(Object.keys(OPS).filter((name) => !sent.has(name))).toEqual([
      'context.pages',
    ])
  })
})
