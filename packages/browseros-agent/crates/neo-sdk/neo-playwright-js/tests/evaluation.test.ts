// The prelude's half of the value format, against the one golden the crate's page_value tests
// read too (crates/neo-sdk/neo-playwright/tests/data/in-page-values.json, scripts/in-page-values.ts).
// Through the built prelude in a realm like QuickJS's: an argument reaches the wire as upstream's
// client and server send it, with upstream's client errors, and the result Rust writes back reads
// as the value upstream's server and client make of it, computed here with the vendored modules on
// Bun's own globals.
import { expect, it } from 'bun:test'
import { readFileSync } from 'node:fs'
import { parseEvaluationResultValue } from '../../neo-playwright/third_party/playwright/packages/isomorphic/utilityScriptSerializers'
import {
  parseSerializedValue,
  serializeValue,
} from '../../neo-playwright/third_party/playwright/packages/protocol/src/serializers'
import {
  isRefusal,
  json,
  loadPrelude,
  refusalCall,
  SEED,
} from './support/prelude-runtime'

type Case = {
  name: string
  source: string
  page: string
  result: unknown
  arg?: string
  argError?: string
}

const golden = JSON.parse(
  readFileSync(
    new URL(
      '../../neo-playwright/tests/data/in-page-values.json',
      import.meta.url,
    ),
    'utf8',
  ),
) as { values: Case[] }

/**
 * A value as JavaScript sees it, `[typeof, Object.prototype.toString, String]`, item by item for
 * arrays and plain objects, so it compares across realms. Run in the prelude's realm as source.
 */
function describe(value: unknown, seen = new Set<unknown>()): unknown {
  if (value !== null && typeof value === 'object') {
    if (seen.has(value)) return 'cycle'
    seen.add(value)
  }
  const tag = Object.prototype.toString.call(value)
  const text = Object.is(value, -0)
    ? '-0'
    : tag === '[object Date]'
      ? String((value as Date).getTime())
      : String(value)
  if (Array.isArray(value))
    return [typeof value, tag, value.map((item) => describe(item, seen))]
  if (tag === '[object Object]')
    return [
      typeof value,
      tag,
      Object.entries(value as object).map(([key, item]) => [
        key,
        describe(item, seen),
      ]),
    ]
  return [typeof value, tag, text]
}

const fallThrough = (value: unknown) => ({ fallThrough: value })

/** What upstream's client receives for a value its page serialized as `result`. */
function upstreamResult(result: unknown): unknown {
  const onServer: unknown = parseEvaluationResultValue(result as never)
  return parseSerializedValue(serializeValue(onServer, fallThrough), undefined)
}

it('tests that every argument reaches the wire as upstream sends it, or fails as its client does', async () => {
  for (const value of golden.values) {
    if (value.arg === undefined && value.argError === undefined) continue
    const runtime = loadPrelude({ seed: SEED })
    const thrown = await runtime.run(
      `try { await page.evaluate((x) => x, ${value.source}); return null } catch (e) { return e.message }`,
    )
    const sent = runtime
      .sent('page.evaluate')
      .filter((call) => !isRefusal(call))
    if (value.arg !== undefined) {
      expect([value.name, thrown]).toEqual([value.name, null])
      expect([value.name, json(sent[0]?.input.script)]).toEqual([
        value.name,
        { run: { expression: '(x) => x', isFunction: true, arg: value.arg } },
      ])
    } else {
      expect([value.name, thrown]).toEqual([value.name, value.argError])
      expect([value.name, sent]).toEqual([value.name, []])
      expect([value.name, runtime.calls]).toEqual([
        value.name,
        [refusalCall('page.evaluate', 'invalid_input', String(thrown))],
      ])
    }
  }
})

it('tests that every result reads back as upstream reads it', async () => {
  for (const value of golden.values) {
    const runtime = loadPrelude({
      seed: SEED,
      answer: (call) => (call.name === 'page.evaluate' ? value.result : null),
    })
    const upstream = upstreamResult(value.result)
    const described = json(
      await runtime.run(
        `const describe = ${describe.toString()}\nreturn describe(await page.evaluate(() => 0))`,
      ),
    )
    expect([value.name, described]).toEqual([
      value.name,
      json(describe(upstream)),
    ])
  }
})

it('tests that upstream client checks throw unprefixed and the server one is raised by Rust', async () => {
  const runtime = loadPrelude({ seed: SEED })
  const outcomes = json(
    await runtime.run(`
      const outcome = (work) => work.then(() => 'returned', (e) => [e.message, e.code])
      return [
        await outcome(page.evaluate(() => 1, 2, 3, 4)),
        await outcome(page.evaluate(() => 1, 2, 3)),
        await outcome(page.evaluate(() => 1, 2, { exposeFunctions: true })),
        await outcome(page.waitForFunction(() => 1, 2, { polling: 'often' })),
        await outcome(page.evaluate(Object.getOwnPropertyDescriptor({ get x() { return 1 } }, 'x').get)),
      ]
    `),
  )
  const tooMany =
    'Too many arguments. If you need to pass more than 1 argument to the function wrap them in an object.'
  expect(outcomes).toEqual([
    [tooMany, 'invalid_input'],
    [tooMany, 'invalid_input'],
    [
      'page.evaluate: pages cannot call back into the script in Neo; return what the page computed instead',
      'method_unavailable',
    ],
    ['Unknown polling option: often', 'invalid_input'],
    'returned',
  ])
  expect(
    json(
      runtime
        .sent('page.evaluate')
        .filter((call) => !isRefusal(call))
        .map((call) => call.input.script),
    ),
  ).toEqual([{ refused: 'Passed function is not well-serializable!' }])
})

it('tests that a method shorthand and an async one are normalized as upstream server does', async () => {
  const runtime = loadPrelude({ seed: SEED })
  await runtime.run(`
    await page.evaluate({ answer() { return 7 } }.answer)
    await page.evaluate({ async later() { return 7 } }.later)
    await page.evaluate('document.title')
  `)
  expect(
    json(runtime.sent('page.evaluate').map((call) => call.input.script)),
  ).toEqual([
    {
      run: {
        expression: '(function answer() { return 7 })',
        isFunction: true,
        arg: '{"v":"undefined"}',
      },
    },
    {
      run: {
        expression: '(async function later() { return 7 })',
        isFunction: true,
        arg: '{"v":"undefined"}',
      },
    },
    {
      run: {
        expression: 'document.title',
        isFunction: false,
        arg: '{"v":"undefined"}',
      },
    },
  ])
})

it('tests that one waitForFunction handle twice in an argument reaches the page as one object', async () => {
  const runtime = loadPrelude({
    seed: SEED,
    answer: (call) =>
      call.name === 'page.waitForFunction'
        ? {
            value: { o: [{ k: 'ready', v: 42 }], id: 1 },
            preview: 'JSHandle@object',
          }
        : null,
  })
  await runtime.run(`
    const ready = await page.waitForFunction(() => ({ ready: 42 }))
    await page.evaluate(([a, b]) => a === b, [ready, ready])
    await page.evaluate(([a, b]) => a === b, [ready, { ready: 42 }])
  `)
  const received = runtime
    .sent('page.evaluate')
    .map(
      (call) =>
        parseEvaluationResultValue(
          JSON.parse(
            (call.input.script as { run: { arg: string } }).run.arg,
          ) as never,
        ) as [unknown, unknown],
    )
  expect(received.map(([a, b]) => [a, a === b])).toEqual([
    [{ ready: 42 }, true],
    [{ ready: 42 }, false],
  ])
})

it('tests that locator.evaluate refuses a signal that is not an AbortSignal before anything is sent, and page.evaluate ignores one as upstream does', async () => {
  const runtime = loadPrelude({ seed: SEED })
  const outcomes = json(
    await runtime.run(`
      const outcome = (work) => work.then(() => 'returned', (e) => [e.message, e.code])
      return [
        await outcome(page.locator('h1').evaluate((el) => el.tagName, undefined, { signal: {} })),
        await outcome(page.evaluate(() => 1, undefined, { signal: {} })),
      ]
    `),
  )
  expect(outcomes).toEqual([
    ['locator.evaluate: signal: expected an AbortSignal', 'invalid_input'],
    'returned',
  ])
  expect(runtime.sent('locator.evaluate')).toEqual([
    refusalCall(
      'locator.evaluate',
      'invalid_input',
      'locator.evaluate: signal: expected an AbortSignal',
    ),
  ])
  expect(runtime.sent('page.evaluate')).toHaveLength(1)
})

it('tests that an answer the prelude cannot read fails as the op that answered it', async () => {
  // A pattern no engine compiles: the vendored parse throws at it, as QuickJS's stack does at a
  // result nested past its depth (tier 1's evaluate-errors deepest).
  const unreadable = { r: { p: '(', f: '' } }
  const runtime = loadPrelude({
    seed: SEED,
    answer: (call) =>
      call.name === 'page.evaluate' || call.name === 'locator.evaluate'
        ? unreadable
        : null,
  })
  const outcomes = json(
    await runtime.run(`
      const outcome = (work) =>
        work.then(() => 'returned', (e) => [e.name, e.message.split(':')[0], e.code ?? null])
      return [
        await outcome(page.evaluate(() => 1)),
        await outcome(page.locator('h1').evaluate(() => 1)),
        await outcome(page.mainFrame().evaluate(() => 1)),
      ]
    `),
  )
  expect(outcomes).toEqual([
    ['Error', 'page.evaluate', null],
    ['Error', 'locator.evaluate', null],
    ['Error', 'frame.evaluate', null],
  ])
  expect(runtime.calls.map((call) => call.name)).toEqual([
    'page.evaluate',
    'locator.evaluate',
    'page.evaluate',
  ])
})

it('tests that a locator.evaluate check upstream runs after the wait fails as that op alone, with no refusal of its own', async () => {
  // Rust raises a script's `refused` after the wait as `<api>: <message>`, as upstream's
  // `_withElement` fails the handle's checks inside the same call.
  const runtime = loadPrelude({
    seed: SEED,
    answer: (call) => {
      const script = call.input.script as { refused?: string } | undefined
      return call.name === 'locator.evaluate' && script?.refused !== undefined
        ? {
            err: {
              code: 'invalid_input',
              owner: 'script',
              delivery: 'not_sent',
              retryable: true,
              operationId: 'run.1',
              timeout: false,
              message: `locator.evaluate: ${script.refused}`,
            },
          }
        : null
    },
  })
  const deferred: [script: string, refused: string][] = [
    [
      "page.locator('#missing').evaluate((el) => el, { fn() {} })",
      'Attempting to serialize unexpected value at position "fn": fn() {}',
    ],
    [
      "page.locator('#missing').evaluate((el) => el, new Date(Number.NaN))",
      'arg.value.d: expected string, got object',
    ],
    [
      "page.locator('#missing').evaluate((el) => el, undefined, 5)",
      'Too many arguments. If you need to pass more than 1 argument to the function wrap them in an object.',
    ],
  ]
  const seen: unknown[] = []
  for (const [script] of deferred) {
    const before = runtime.calls.length
    const thrown = json(
      await runtime.run(`
        try { await ${script} } catch (error) { return [error.name, error.message, error.code] }
        return 'returned'
      `),
    )
    const sent = runtime.calls
      .slice(before)
      .map((call) => [call.name, call.input.script])
    seen.push([script, thrown, json(sent)])
  }
  expect(seen).toEqual(
    deferred.map(([script, refused]) => [
      script,
      ['Error', `locator.evaluate: ${refused}`, 'invalid_input'],
      [['locator.evaluate', { refused }]],
    ]),
  )
})
