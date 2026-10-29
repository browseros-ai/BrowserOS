/** Full reads are checked inside the SDK, before model-output formatting.
 * Evaluation checks the wire value, including values JSON normally discards. */
import { expect } from 'bun:test'
import type { CaseContext, ContractCase } from './cases'
import { expectOk } from './helpers'
import { errorIncludes, evalValue, runValue } from './sdk-helpers'

const specialValues = `
  const cycle = { label: 'cycle' }; cycle.self = cycle
  return {
    nan: NaN, infinity: Infinity, negativeZero: -0,
    bigint: 900719925474099312345n,
    date: new Date('2026-10-29T12:34:56.000Z'),
    map: new Map([['answer', 42]]), set: new Set(['one', 'two']),
    node: document.getElementById('apply'), cycle,
    fn: function example() {}, missing: undefined
  }
`

const cappedRuns = [
  {
    label: 'a large return value',
    value: `${'R'.repeat(30_000)}SDK_CAP_VALUE_TAIL`,
    logs: [],
  },
  {
    // Neither part crosses 25K alone: the cap must count the return and logs together.
    label: 'the combined return value and logs',
    value: `${'R'.repeat(20_000)}SDK_CAP_VALUE_TAIL`,
    logs: [`${'L'.repeat(10_000)}SDK_CAP_LOG_TAIL`],
  },
]

async function evaluate(
  ctx: CaseContext,
  page: number,
  surface: 'evaluate_in_page' | 'run',
  code: string,
) {
  if (surface === 'evaluate_in_page') return evalValue(ctx, page, code)
  // The evaluate result carries readiness metadata; its value is the page value.
  return runValue(
    ctx,
    page,
    `return (await p.evaluateInPage(${JSON.stringify(code)})).value`,
  )
}

export const sdkReadEvalCases: ContractCase[] = [
  ...cappedRuns.flatMap(({ label, value, logs }) => {
    const code = `${logs.map((line) => `console.log(${JSON.stringify(line)})`).join(';')}; return ${JSON.stringify(value)}`
    return [
      {
        name: `run: caps ${label} at 25000 bytes and saves the full result`,
        async run(ctx: CaseContext) {
          const result = await ctx.mcp.callTool('run', { code })
          expect(expectOk(result)).toContain('Result truncated at 25000 bytes')
          expect(result.structuredContent?.writtenToFile).toBe(true)
          expect(result.structuredContent).not.toHaveProperty('value')
          const path = result.structuredContent?.path
          if (typeof path !== 'string')
            throw new Error('capped run omitted its spill path')
          const full = await Bun.file(path).text()
          expect(full).toContain(value)
          for (const log of logs) expect(full).toContain(log)
        },
      },
      {
        name: `run: maxChars 40000 keeps ${label} inline`,
        async run(ctx: CaseContext) {
          const result = await ctx.mcp.callTool('run', {
            code,
            maxChars: 40_000,
          })
          expect(expectOk(result)).not.toContain('Result truncated')
          expect(result.structuredContent?.value).toBe(value)
          expect(result.structuredContent?.logs).toEqual(logs)
          expect(result.structuredContent?.writtenToFile).not.toBe(true)
        },
      },
    ]
  }),
  {
    name: 'run: read includes the tail of a page longer than 20000 characters',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/sdk-long-read.html'))
      expect(
        await evalValue<number>(
          ctx,
          page,
          'return document.body.innerText.length',
        ),
      ).toBeGreaterThan(20_000)
      expect(
        await runValue(
          ctx,
          page,
          'return (await p.read()).includes("SDK_FULL_READ_TAIL_20261029")',
        ),
      ).toBe(true)
    },
  },
  {
    name: 'run: evaluateInPage keeps a long value available inside the script',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/sdk-long-read.html'))
      expect(
        await runValue(
          ctx,
          page,
          `
          const result = await p.evaluateInPage('return document.body.innerText')
          return { long: result.value.length > 20000, tail: result.value.includes('SDK_FULL_READ_TAIL_20261029') }
        `,
        ),
      ).toEqual({ long: true, tail: true })
    },
  },
  ...(['evaluate_in_page', 'run'] as const).flatMap((surface) => [
    {
      name: `${surface}: evaluation preserves special values and circular objects`,
      async run(ctx: CaseContext) {
        const page = await ctx.openPage(ctx.fixture('/form.html'))
        const value = await evaluate(ctx, page, surface, specialValues)
        expect(value).toEqual({
          nan: 'NaN',
          infinity: 'Infinity',
          negativeZero: 0,
          bigint: '900719925474099312345',
          date: '2026-10-29T12:34:56.000Z',
          map: { answer: 42 },
          set: ['one', 'two'],
          node: expect.stringMatching(/^<button id="apply"(?:\s[^>]*)?>$/),
          cycle: { label: 'cycle', self: '[Circular]' },
          fn: '[function]',
          missing: null,
        })
      },
    },
    {
      name: `${surface}: evaluation times out an unsettled promise within 5 seconds`,
      async run(ctx: CaseContext) {
        const page = await ctx.openPage(ctx.fixture('/form.html'))
        const started = performance.now()
        const code = 'return await new Promise(() => {})'
        const result =
          surface === 'evaluate_in_page'
            ? await ctx.mcp.callTool('evaluate_in_page', {
                page,
                code,
                timeout: 1000,
              })
            : await ctx.mcp.callTool('run', {
                code: `await browser.page(${page}).evaluateInPage(${JSON.stringify(code)}, { timeout: 1000 })`,
              })
        errorIncludes(result, 'did not finish within 1000 ms')
        expect(performance.now() - started).toBeLessThan(5_000)
        // A timed-out promise must not prevent the next request on this page.
        expect(await evaluate(ctx, page, surface, 'return 42')).toBe(42)
      },
    },
    {
      name: `${surface}: evaluation still returns plain numbers and objects unchanged`,
      async run(ctx: CaseContext) {
        const page = await ctx.openPage(ctx.fixture('/form.html'))
        expect(await evaluate(ctx, page, surface, 'return 42')).toBe(42)
        expect(
          await evaluate(
            ctx,
            page,
            surface,
            'return { count: 3, nested: { ok: true }, values: [1, null, "x"] }',
          ),
        ).toEqual({ count: 3, nested: { ok: true }, values: [1, null, 'x'] })
      },
    },
  ]),
  {
    name: 'read: the full-text spill still contains the long page tail',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/sdk-long-read.html'))
      const text = expectOk(
        await ctx.mcp.callTool('read', { page, format: 'text' }),
      )
      const path = text.match(/saved to: (\S+)/)?.[1]
      expect(path).toBeDefined()
      if (!path) throw new Error('long read did not spill')
      expect(await Bun.file(path).text()).toContain(
        'SDK_FULL_READ_TAIL_20261029',
      )
    },
  },
]
