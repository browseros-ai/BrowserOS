// expect() as scripts see it: retrying matchers send upstream's FrameExpectParams with the
// printed expectation for Rust to poll and report; value matchers run in the prelude with
// Jest's semantics.
import { describe, expect, it } from 'bun:test'
import { json, loadPrelude, refusalCall, SEED } from './support/prelude-runtime'

async function expectOps(code: string): Promise<Record<string, unknown>[]> {
  const runtime = loadPrelude({ seed: SEED })
  await runtime.run(code)
  return runtime.calls.map((call) => ({
    name: call.name,
    ...call.input,
    target: undefined,
  }))
}

describe('retrying matchers', () => {
  it("tests that toHaveText sends upstream's params and Playwright's expected line", async () => {
    expect(
      await expectOps(
        "await expect(page.getByRole('heading')).toHaveText('Welcome')",
      ),
    ).toEqual([
      {
        name: 'expect.locator',
        call: { api: 'expect.toHaveText', timeoutMs: 5000 },
        target: undefined,
        matcher: 'toHaveText',
        expectation: 'expected',
        expected: 'Expected: "Welcome"',
        expression: 'to.have.text',
        expectedText: [{ string: 'Welcome', normalizeWhiteSpace: true }],
        isNot: false,
      },
    ])
  })

  it('tests that each matcher form maps to its expression, argument text and expected line', async () => {
    const sent = await expectOps(`
      const loc = page.locator('#x')
      await expect(loc).not.toBeVisible()
      await expect(loc).toBeVisible({ visible: false })
      await expect(loc).toHaveText(/wel+come/i)
      await expect(loc).toContainText('come', { ignoreCase: true })
      await expect(loc).toHaveCount(3)
      await expect(loc).toHaveText(['a', /b/])
      await expect(loc).toHaveAttribute('href')
      await expect(loc).toHaveAttribute('href', '/cart')
      await expect(loc).toBeChecked({ checked: false })
      await expect(loc).toHaveJSProperty('value', 12)
    `)
    expect(
      sent.map((op) => [
        op.matcher,
        op.expectation,
        op.expected,
        op.expression,
        op.isNot,
      ]),
    ).toEqual([
      ['toBeVisible', '', 'Expected: not visible', 'to.be.visible', true],
      [
        'toBeVisible',
        '{ visible: false }',
        'Expected: hidden',
        'to.be.hidden',
        false,
      ],
      [
        'toHaveText',
        'expected',
        'Expected pattern: /wel+come/i',
        'to.have.text',
        false,
      ],
      [
        'toContainText',
        'expected',
        'Expected substring: "come"',
        'to.have.text',
        false,
      ],
      ['toHaveCount', 'expected', 'Expected: 3', 'to.have.count', false],
      [
        'toHaveText',
        'expected',
        'Expected: ["a", /b/]',
        'to.have.text.array',
        false,
      ],
      [
        'toHaveAttribute',
        '',
        'Expected: have attribute',
        'to.have.attribute',
        false,
      ],
      [
        'toHaveAttribute',
        'expected',
        'Expected: "/cart"',
        'to.have.attribute.value',
        false,
      ],
      [
        'toBeChecked',
        '{ checked: false }',
        'Expected: unchecked',
        'to.be.checked',
        false,
      ],
      [
        'toHaveJSProperty',
        'expected',
        'Expected: 12',
        'to.have.property',
        false,
      ],
    ])
    expect(sent[2]).toMatchObject({
      expectedText: [
        { regexSource: 'wel+come', regexFlags: 'i', normalizeWhiteSpace: true },
      ],
    })
    expect(sent[3]).toMatchObject({
      expectedText: [
        {
          string: 'come',
          matchSubstring: true,
          normalizeWhiteSpace: true,
          ignoreCase: true,
        },
      ],
    })
    expect(sent[4]).toMatchObject({ expectedNumber: 3 })
    expect(sent[6]).toMatchObject({ expressionArg: 'href' })
    expect(sent[8]).toMatchObject({ expectedValue: { checked: false } })
    expect(sent[9]).toMatchObject({
      expressionArg: 'value',
      expectedValue: 12,
    })
  })

  it('tests that page matchers send the page, and a string URL is normalized as new URL() would', async () => {
    const sent = await expectOps(`
      await expect(page).toHaveTitle('Cart')
      await expect(page).toHaveURL('HTTPS://Shop.Example:443')
      await expect(page).toHaveURL(/cart$/)
    `)
    expect(sent.map((op) => [op.name, op.page, op.expected])).toEqual([
      ['expect.page', { $neo: 'page', tab: 1007 }, 'Expected: "Cart"'],
      [
        'expect.page',
        { $neo: 'page', tab: 1007 },
        'Expected: "https://shop.example/"',
      ],
      ['expect.page', { $neo: 'page', tab: 1007 }, 'Expected pattern: /cart$/'],
    ])
  })

  it("tests that expect's timeout is its own: the call option, then configure, then 5 s, never the page's", async () => {
    const sent = await expectOps(`
      page.setDefaultTimeout(60_000)
      const loc = page.locator('#x')
      await expect(loc).toBeVisible()
      await expect(loc).toBeVisible({ timeout: 900 })
      await expect.configure({ timeout: 10_000 })(loc).toBeHidden()
    `)
    expect(
      sent.map((op) => (op.call as { timeoutMs: number }).timeoutMs),
    ).toEqual([5000, 900, 10_000])
  })

  it("tests that Rust's failure reaches the script as Playwright's plain Error, with the fields and a custom message", async () => {
    const runtime = loadPrelude({
      seed: SEED,
      answer: () => ({
        err: {
          code: 'assertion_failed',
          owner: 'website',
          delivery: 'not_sent',
          retryable: false,
          operationId: 'run.2',
          timeout: true,
          message:
            'expect(locator).toHaveText(expected) failed\n\nLocator: locator(\'#total\')\nExpected: "$10"\nReceived: "$12"\nTimeout: 5000ms',
        },
      }),
    })
    const error = json(
      await runtime.run(`
        try { await expect(page.locator('#total'), 'the cart shows the total').toHaveText('$10') }
        catch (e) { return { name: e.name, message: e.message, code: e.code, owner: e.owner, timeout: e.timeout, operationId: e.operationId } }
      `),
    )
    expect(error).toEqual({
      name: 'Error',
      message:
        'the cart shows the total\n\nexpect(locator).toHaveText(expected) failed\n\nLocator: locator(\'#total\')\nExpected: "$10"\nReceived: "$12"\nTimeout: 5000ms',
      code: 'assertion_failed',
      owner: 'website',
      timeout: true,
      operationId: 'run.2',
    })
  })

  it("tests that a failed matcher's error carries Playwright's matcherResult from Rust's facts", async () => {
    // As bridge.rs sends a matcher failure: Rust's facts under `matcher` (expectation.rs
    // `MatcherFacts`); the prelude adds the name, pass, expected value and timeout.
    const failure = (
      message: string,
      matcher: Record<string, unknown>,
      timeout = true,
    ) => ({
      err: {
        code: 'assertion_failed',
        owner: 'website',
        delivery: 'not_sent',
        retryable: false,
        operationId: 'run.2',
        timeout,
        message,
        matcher,
      },
    })
    const answers = [
      failure('expect(locator).toHaveText(expected) failed', {
        actual: 'Text content',
        ariaSnapshot: '- text: Text content',
        log: [
          'Expect "toHaveText" with timeout 1ms',
          "waiting for locator('#node')",
        ],
        timedOut: true,
      }),
      failure('expect(locator).not.toBeVisible() failed', {
        actual: 'visible',
        ariaSnapshot: '- text: Text content',
        log: [],
        timedOut: true,
      }),
      failure('expect(locator).toHaveCount(expected) failed', {
        actual: 0,
        log: [],
        timedOut: true,
      }),
      failure(
        'expect(locator).toBeChecked() failed',
        { log: [], timedOut: false },
        false,
      ),
    ]
    const runtime = loadPrelude({
      seed: SEED,
      answer: (call) =>
        call.name === 'expect.locator' ? (answers.shift() ?? null) : null,
    })
    const results = json(
      await runtime.run(`
        const loc = page.locator('#node')
        const caught = []
        for (const assertion of [
          () => expect(loc, 'the node reads right').toHaveText(/Text2/, { timeout: 1 }),
          () => expect(loc).not.toBeVisible({ timeout: 1 }),
          () => expect(page.locator('#node2')).toHaveCount(1, { timeout: 1 }),
          () => expect(loc).toBeChecked({ checked: false }),
        ]) {
          try { await assertion() } catch (e) {
            const r = e.matcherResult
            caught.push({ ...r, expected: String(r.expected), keys: Object.keys(r), message: r.message, full: e.message })
          }
        }
        return caught
      `),
    ) as Record<string, unknown>[]
    expect(results.map(({ keys: _, full: __, ...rest }) => rest)).toEqual([
      {
        message: 'expect(locator).toHaveText(expected) failed',
        pass: false,
        actual: 'Text content',
        name: 'toHaveText',
        expected: '/Text2/',
        log: [
          'Expect "toHaveText" with timeout 1ms',
          "waiting for locator('#node')",
        ],
        timeout: 1,
        ariaSnapshot: '- text: Text content',
      },
      {
        message: 'expect(locator).not.toBeVisible() failed',
        pass: true,
        actual: 'visible',
        name: 'toBeVisible',
        expected: 'visible',
        log: [],
        timeout: 1,
        ariaSnapshot: '- text: Text content',
      },
      {
        message: 'expect(locator).toHaveCount(expected) failed',
        pass: false,
        actual: 0,
        name: 'toHaveCount',
        expected: '1',
        log: [],
        timeout: 1,
      },
      {
        message: 'expect(locator).toBeChecked() failed',
        pass: false,
        name: 'toBeChecked',
        expected: 'unchecked',
        log: [],
      },
    ])
    // The custom message prefixes the error, never matcherResult.message, as upstream's
    // ExpectError does.
    expect(results[0]?.full).toBe(
      'the node reads right\n\nexpect(locator).toHaveText(expected) failed',
    )
  })

  it("tests that an uncaught expect failure or refusal settles the run as that op's failure", async () => {
    // Each as claw-sdk's bridge sends it (runtime/bridge.rs `ErrBody`): Rust's assertion
    // failure, and the dispatcher refusing the op.
    const failures = [
      {
        code: 'assertion_failed',
        owner: 'website',
        delivery: 'not_sent',
        retryable: false,
        operationId: 'run.1',
        timeout: true,
        message:
          'expect(locator).toHaveText(expected) failed\n\nLocator: locator(\'#total\')\nExpected: "nope"\nReceived: "$12"\nTimeout: 50ms',
      },
      {
        code: 'owner_has_control',
        owner: 'host',
        delivery: 'not_sent',
        retryable: true,
        operationId: 'run.1',
        timeout: false,
        message: 'expect.locator: the owner is in control of this tab',
      },
    ]
    for (const failure of failures) {
      const runtime = loadPrelude({
        seed: SEED,
        answer: () => ({ err: failure }),
      })
      expect(
        await runtime.settle(
          "await expect(page.locator('#total')).toHaveText('nope', { timeout: 50 })",
        ),
      ).toEqual({ operation: failure })
    }
  })

  it("tests that a locator matcher on something else fails as Playwright's does, refused before any op", async () => {
    const runtime = loadPrelude({ seed: SEED })
    const message =
      'toBeVisible can be only used with Locator object, was called with 42'
    expect(
      await runtime.run(
        'try { await expect(42).toBeVisible() } catch (e) { return e.message }',
      ),
    ).toBe(message)
    expect(runtime.calls).toEqual([
      refusalCall('expect.toBeVisible', 'invalid_input', message),
    ])
  })
})

describe('toMatchAriaSnapshot', () => {
  it("tests that the template is trimmed and parsed as upstream's server parses it, and sent for the page to match", async () => {
    const sent = await expectOps(`
      await expect(page.locator('ul')).toMatchAriaSnapshot(\`
        - list "my list":
          - listitem: one
          - link /Issues \\\\d+/:
            - /url: /.*example.com/
      \`)
      await expect(page).not.toMatchAriaSnapshot('- heading "x" [level=2]', { timeout: 300 })
    `)
    expect(sent).toEqual([
      {
        name: 'expect.locator',
        call: { api: 'expect.toMatchAriaSnapshot', timeoutMs: 5000 },
        target: undefined,
        matcher: 'toMatchAriaSnapshot',
        expectation: 'expected',
        expected:
          'Expected: "- list \\"my list\\":\n  - listitem: one\n  - link /Issues \\\\d+/:\n    - /url: /.*example.com/"',
        expression: 'to.match.aria',
        expectedValue: {
          kind: 'role',
          role: 'list',
          name: 'my list',
          children: [
            {
              kind: 'role',
              role: 'listitem',
              name: '',
              children: [
                { kind: 'text', text: { raw: 'one', normalized: 'one' } },
              ],
            },
            {
              kind: 'role',
              role: 'link',
              name: { pattern: 'Issues \\d+' },
              children: [],
              props: {
                url: {
                  raw: '/.*example.com/',
                  normalized: '/.*example.com/',
                },
              },
            },
          ],
        },
        isNot: false,
      },
      {
        name: 'expect.page',
        call: { api: 'expect.toMatchAriaSnapshot', timeoutMs: 300 },
        target: undefined,
        page: { $neo: 'page', tab: 1007 },
        matcher: 'toMatchAriaSnapshot',
        expectation: 'expected',
        expected: 'Expected: not "- heading \\"x\\" [level=2]"',
        expression: 'to.match.aria',
        expectedValue: { kind: 'role', role: 'heading', name: 'x', level: 2 },
        isNot: true,
      },
    ])
  })

  it("tests that a template that does not parse still reaches Rust, with the parser's message for its Error line", async () => {
    const sent = await expectOps(`
      await expect(page).toMatchAriaSnapshot('- heading [level=a]')
      await expect(page.locator('h1')).toMatchAriaSnapshot('heading "title"')
    `)
    expect(
      sent.map(({ name, expression, expectedValue, isNot, templateError }) => ({
        name,
        expression,
        expectedValue,
        isNot,
        templateError,
      })),
    ).toEqual([
      {
        name: 'expect.page',
        expression: 'to.match.aria',
        expectedValue: undefined,
        isNot: false,
        templateError:
          'Value of "level" attribute must be a number:\n\nheading [level=a]\n               ^\n',
      },
      {
        name: 'expect.locator',
        expression: 'to.match.aria',
        expectedValue: undefined,
        isNot: false,
        templateError:
          'Aria snapshot must be a YAML sequence, elements starting with " -"',
      },
    ])
  })

  it('tests that snapshot files, an empty template and other receivers are refused before any op', async () => {
    const runtime = loadPrelude({ seed: SEED })
    const files =
      'snapshot files are not available in Neo; pass the expected snapshot as a YAML string'
    const empty =
      "the expected snapshot is empty, and Neo writes no baselines; read the current one with await locator.ariaSnapshot({ mode: 'default' })"
    const receiver =
      'toMatchAriaSnapshot can be only used with Page or Locator objects, was called with 42'
    expect(
      json(
        await runtime.run(`
          const messages = []
          for (const assert of [
            () => expect(page).toMatchAriaSnapshot(),
            () => expect(page).toMatchAriaSnapshot({ name: 'cart.aria.yml' }),
            () => expect(page.locator('ul')).toMatchAriaSnapshot('  \\n  '),
            () => expect(42).toMatchAriaSnapshot('- list'),
          ]) {
            try { await assert() } catch (e) { messages.push(e.message) }
          }
          return messages
        `),
      ),
    ).toEqual([files, files, empty, receiver])
    expect(runtime.calls).toEqual([
      refusalCall('expect.toMatchAriaSnapshot', 'invalid_input', files),
      refusalCall('expect.toMatchAriaSnapshot', 'invalid_input', files),
      refusalCall('expect.toMatchAriaSnapshot', 'invalid_input', empty),
      refusalCall('expect.toMatchAriaSnapshot', 'invalid_input', receiver),
    ])
  })
})

describe('value matchers', () => {
  async function failure(code: string): Promise<unknown> {
    return await loadPrelude({ seed: SEED }).run(
      `try { ${code}; return 'passed' } catch (e) { return e.message }`,
    )
  }

  it("tests that failures read as Jest's, with .not", async () => {
    expect(await failure('expect(2).toBe(3)')).toBe(
      'expect(received).toBe(expected) // Object.is equality\n\nExpected: 3\nReceived: 2',
    )
    expect(await failure('expect(2).not.toBe(2)')).toBe(
      'expect(received).not.toBe(expected) // Object.is equality\n\nExpected: not 2',
    )
    expect(await failure("expect('abc').toMatch(/z/)")).toBe(
      'expect(received).toMatch(expected)\n\nExpected pattern: /z/\nReceived string: "abc"',
    )
  })

  it('tests deep equality: toEqual ignores undefined properties, toStrictEqual does not', async () => {
    expect(
      await failure(
        'expect({ a: [1, { b: 2 }], c: undefined }).toEqual({ a: [1, { b: 2 }] })',
      ),
    ).toBe('passed')
    expect(
      await failure('expect({ a: 1, c: undefined }).toStrictEqual({ a: 1 })'),
    ).toBe(
      'expect(received).toStrictEqual(expected) // deep equality\n\nExpected: {"a": 1}\nReceived: {"a": 1, "c": undefined}',
    )
    expect(
      await failure(
        'expect(new Map([[1, { x: 1 }]])).toEqual(new Map([[1, { x: 1 }]]))',
      ),
    ).toBe('passed')
    expect(await failure('expect([1, 2]).toEqual([1, 2, 3])')).not.toBe(
      'passed',
    )
  })

  it('tests the asymmetric matchers inside equality', async () => {
    expect(
      await failure(
        "expect({ id: 7, name: 'Ann', tags: ['a', 'b'], at: 'x-1' }).toEqual({ id: expect.any(Number), name: expect.stringContaining('An'), tags: expect.arrayContaining(['b']), at: expect.stringMatching(/^x-\\d$/) })",
      ),
    ).toBe('passed')
    expect(
      await failure(
        'expect({ a: 1, b: 2 }).toEqual(expect.objectContaining({ b: 3 }))',
      ),
    ).not.toBe('passed')
    expect(
      await failure('expect(0.1 + 0.2).toEqual(expect.closeTo(0.3, 5))'),
    ).toBe('passed')
    expect(
      await failure("expect(['x']).toEqual(expect.not.arrayContaining(['y']))"),
    ).toBe('passed')
  })

  it('tests the other value matchers on passing and failing values', async () => {
    const results = json(
      await loadPrelude({ seed: SEED }).run(`
        const outcome = (assert) => { try { assert(); return true } catch { return false } }
        return [
          outcome(() => expect([1, 2, 3]).toContain(2)),
          outcome(() => expect('checkout').toContain('out')),
          outcome(() => expect([{ a: 1 }]).toContainEqual({ a: 1 })),
          outcome(() => expect('abc').toHaveLength(3)),
          outcome(() => expect({ a: { b: [1, 2] } }).toHaveProperty('a.b', [1, 2])),
          outcome(() => expect({ a: 1, b: { c: 2, d: 3 } }).toMatchObject({ b: { c: 2 } })),
          outcome(() => expect(() => { throw new TypeError('bad card') }).toThrow(/card/)),
          outcome(() => expect(() => {}).toThrow()),
          outcome(() => expect(0.1 + 0.2).toBeCloseTo(0.3)),
          outcome(() => expect(5).toBeGreaterThan(4)),
          outcome(() => expect(5).toBeLessThanOrEqual(4)),
          outcome(() => expect(null).toBeNull()),
          outcome(() => expect(undefined).toBeDefined()),
          outcome(() => expect(NaN).toBeNaN()),
          outcome(() => expect(new Date(0)).toBeInstanceOf(Date)),
        ]
      `),
    )
    expect(results).toEqual([
      true,
      true,
      true,
      true,
      true,
      true,
      true,
      false,
      true,
      true,
      false,
      true,
      false,
      true,
      true,
    ])
  })

  it('tests resolves and rejects, which settle first and return a promise', async () => {
    const results = json(
      await loadPrelude({ seed: SEED }).run(`
        const outcome = async (assert) => { try { await assert(); return 'ok' } catch (e) { return e.message.split('\\n')[0] } }
        return [
          await outcome(() => expect(Promise.resolve(4)).resolves.toBe(4)),
          await outcome(() => expect(Promise.reject(new Error('no'))).rejects.toThrow('no')),
          await outcome(() => expect(Promise.resolve(1)).rejects.toBe(1)),
        ]
      `),
    )
    expect(results).toEqual([
      'ok',
      'ok',
      'expect(received).rejects.toBe(expected)',
    ])
  })

  it('tests that toPass and expect.poll retry until the assertion holds', async () => {
    const result = json(
      await loadPrelude({ seed: SEED }).run(`
        let tries = 0
        await expect(() => { tries++; expect(tries).toBe(3) }).toPass({ intervals: [1] })
        let polls = 0
        await expect.poll(() => ++polls, { intervals: [1] }).toBe(2)
        return { tries, polls }
      `),
    )
    expect(result).toEqual({ tries: 3, polls: 2 })
  })
})
