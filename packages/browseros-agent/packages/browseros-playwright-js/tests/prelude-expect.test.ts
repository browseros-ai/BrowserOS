// expect() as scripts see it: retrying matchers send upstream's FrameExpectParams with the
// printed expectation for Rust to poll and report; value matchers run in the prelude with
// Jest's semantics.
import { describe, expect, it } from 'bun:test'
import { json, loadPrelude, SEED } from './support/prelude-runtime'

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
        params: {
          expression: 'to.have.text',
          expectedText: [{ string: 'Welcome', normalizeWhiteSpace: true }],
          isNot: false,
        },
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
        (op.params as { expression: string }).expression,
        (op.params as { isNot: boolean }).isNot,
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
    expect(sent[2]?.params).toMatchObject({
      expectedText: [
        { regexSource: 'wel+come', regexFlags: 'i', normalizeWhiteSpace: true },
      ],
    })
    expect(sent[3]?.params).toMatchObject({
      expectedText: [
        {
          string: 'come',
          matchSubstring: true,
          normalizeWhiteSpace: true,
          ignoreCase: true,
        },
      ],
    })
    expect(sent[4]?.params).toMatchObject({ expectedNumber: 3 })
    expect(sent[6]?.params).toMatchObject({ expressionArg: 'href' })
    expect(sent[8]?.params).toMatchObject({ expectedValue: { checked: false } })
    expect(sent[9]?.params).toMatchObject({
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
      ['expect.page', { $neo: 'page', id: 7, tab: 1007 }, 'Expected: "Cart"'],
      [
        'expect.page',
        { $neo: 'page', id: 7, tab: 1007 },
        'Expected: "https://shop.example/"',
      ],
      [
        'expect.page',
        { $neo: 'page', id: 7, tab: 1007 },
        'Expected pattern: /cart$/',
      ],
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

  it("tests that a locator matcher on something else fails as Playwright's does, before any op", async () => {
    const runtime = loadPrelude({ seed: SEED })
    expect(
      await runtime.run(
        'try { await expect(42).toBeVisible() } catch (e) { return e.message }',
      ),
    ).toBe(
      'toBeVisible can be only used with Locator object, was called with 42',
    )
    expect(runtime.calls).toEqual([])
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
