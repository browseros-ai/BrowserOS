// ariaSnapshot() as scripts see it: Rust returns the stitched JSON, and the prelude renders it.
// With `mode: 'default'` that is the vendored renderer's YAML, with a `# partial` comment under
// each iframe line whose frame could not be read; with no mode or `mode: 'ai'` it is the v1
// format (src/prelude/snapshot-v1.ts, rule by rule in snapshot-v1.test.ts), unless the spec
// harness's conformance switch keeps upstream's text.
import { expect, it } from 'bun:test'
import { readFileSync } from 'node:fs'
import { renderAriaSnapshotAsYaml } from '../../neo-playwright/third_party/playwright/packages/isomorphic/ariaSnapshotRenderer'
import { json, loadPrelude, refusalCall, SEED } from './support/prelude-runtime'

const fixture: {
  result: {
    json: Parameters<typeof renderAriaSnapshotAsYaml>[0]
    partial: unknown[]
  }
  yaml: string[]
} = JSON.parse(
  readFileSync(new URL('./data/snapshot-render.json', import.meta.url), 'utf8'),
)

function snapshotting(result: unknown) {
  return loadPrelude({
    seed: SEED,
    answer: (call) => (call.name.endsWith('.ariaSnapshot') ? result : null),
  })
}

it("tests that a default-mode snapshot without partial frames is the vendored renderer's YAML, byte for byte", async () => {
  const whole = { json: fixture.result.json, partial: [] }
  const runtime = snapshotting(whole)
  const yaml = await runtime.run(
    "return await page.ariaSnapshot({ mode: 'default' })",
  )
  expect(yaml).toBe(renderAriaSnapshotAsYaml(fixture.result.json))
  expect(yaml).toBe(
    fixture.yaml
      .filter((line) => !line.trim().startsWith('# partial'))
      .join('\n'),
  )
})

it('tests that each frame Rust could not read gets its comment under its iframe line', async () => {
  const runtime = snapshotting(fixture.result)
  expect(
    await runtime.run("return await page.ariaSnapshot({ mode: 'default' })"),
  ).toBe(fixture.yaml.join('\n'))
})

it('tests that a partial frame whose iframe line is missing is still reported, at the end', async () => {
  const runtime = snapshotting({
    json: [{ role: 'button', name: 'Go', ref: 'e2' }],
    partial: [
      { ref: 'e9', frame: 'https://gone.example/', reason: 'target closed' },
    ],
  })
  expect(await runtime.run('return await page.ariaSnapshot()')).toBe(
    '- button "Go" [ref=e2]\n# partial: https://gone.example/ not read (target closed)',
  )
})

it('tests that page and locator snapshots send their mode and depth, the text path ai by default with clickable marking, and JSON skips the rendering', async () => {
  const runtime = snapshotting(fixture.result)
  const result = json(
    await runtime.run(`
      await page.ariaSnapshot()
      await page.getByRole('main').ariaSnapshot({ mode: 'ai', depth: 3, timeout: 900 })
      return await page.ariaSnapshotJSON({ mode: 'ai' })
    `),
  )
  expect(result).toEqual(fixture.result.json)
  expect(
    runtime.calls.map(
      (call): Record<string, unknown> => ({
        name: call.name,
        ...call.input,
        target: undefined,
      }),
    ),
  ).toEqual([
    {
      name: 'page.ariaSnapshot',
      call: { api: 'page.ariaSnapshot', timeoutMs: 5000 },
      page: { $neo: 'page', tab: 1007 },
      mode: 'ai',
      clickable: true,
      target: undefined,
    },
    {
      // A whole page's v1 text is recorded as the run's pending baseline; a locator's never is.
      name: 'page.snapshotPending',
      // What is left of the snapshot's own 5 s.
      call: { api: 'page.ariaSnapshot', timeoutMs: expect.any(Number) },
      page: { $neo: 'page', tab: 1007 },
      text: expect.any(String),
      // How that text was rendered, which the baseline keeps beside it.
      form: { text: 'kept', urls: false, maxTokens: 8000 },
      delta: false,
      target: undefined,
    },
    {
      name: 'locator.ariaSnapshot',
      call: { api: 'locator.ariaSnapshot', timeoutMs: 900 },
      strict: 'strict',
      mode: 'ai',
      depth: 3,
      clickable: true,
      target: undefined,
    },
    {
      name: 'page.ariaSnapshot',
      call: { api: 'page.ariaSnapshotJSON', timeoutMs: 5000 },
      page: { $neo: 'page', tab: 1007 },
      mode: 'ai',
      target: undefined,
    },
  ])
})

it('tests that only the v1 text asks for clickable marking: never mode default, the JSON, or anything under the conformance switch', async () => {
  const script = `
    await page.ariaSnapshot()
    await page.ariaSnapshot({ mode: 'ai' })
    await page.ariaSnapshot({ mode: 'default' })
    await page.getByRole('main').ariaSnapshot({ mode: 'ai' })
    await page.getByRole('main').ariaSnapshot({ mode: 'default' })
    await page.ariaSnapshotJSON({ mode: 'ai' })
    await page.getByRole('main').ariaSnapshotJSON({ mode: 'ai' })
  `
  const asked = (runtime: ReturnType<typeof snapshotting>) =>
    runtime.calls
      .filter((call) => call.name.endsWith('.ariaSnapshot'))
      .map((call) => ({
        name: call.name,
        mode: call.input.mode,
        clickable: Object.hasOwn(call.input, 'clickable')
          ? call.input.clickable
          : 'absent',
      }))
  const v1 = snapshotting(fixture.result)
  await v1.run(script)
  expect(asked(v1)).toEqual([
    { name: 'page.ariaSnapshot', mode: 'ai', clickable: true },
    { name: 'page.ariaSnapshot', mode: 'ai', clickable: true },
    { name: 'page.ariaSnapshot', mode: 'default', clickable: 'absent' },
    { name: 'locator.ariaSnapshot', mode: 'ai', clickable: true },
    { name: 'locator.ariaSnapshot', mode: 'default', clickable: 'absent' },
    { name: 'page.ariaSnapshot', mode: 'ai', clickable: 'absent' },
    { name: 'locator.ariaSnapshot', mode: 'ai', clickable: 'absent' },
  ])
  const upstream = loadPrelude({
    seed: SEED,
    upstreamSnapshots: true,
    answer: (call) =>
      call.name.endsWith('.ariaSnapshot') ? fixture.result : null,
  })
  await upstream.run(script)
  expect(asked(upstream)).toEqual([
    { name: 'page.ariaSnapshot', mode: 'default', clickable: 'absent' },
    { name: 'page.ariaSnapshot', mode: 'ai', clickable: 'absent' },
    { name: 'page.ariaSnapshot', mode: 'default', clickable: 'absent' },
    { name: 'locator.ariaSnapshot', mode: 'ai', clickable: 'absent' },
    { name: 'locator.ariaSnapshot', mode: 'default', clickable: 'absent' },
    { name: 'page.ariaSnapshot', mode: 'ai', clickable: 'absent' },
    { name: 'locator.ariaSnapshot', mode: 'ai', clickable: 'absent' },
  ])
})

it("tests that boxes go to Rust as the script passed them, upstream's option", async () => {
  const runtime = snapshotting(fixture.result)
  await runtime.run(`
    await page.ariaSnapshotJSON({ boxes: true })
    await page.getByRole('main').ariaSnapshot({ boxes: false })
  `)
  expect(
    runtime.calls.map((call) => ({
      name: call.name,
      boxes: call.input.boxes,
    })),
  ).toEqual([
    { name: 'page.ariaSnapshot', boxes: true },
    { name: 'locator.ariaSnapshot', boxes: false },
  ])
})

/** A small page: its v1 text is the main line, the notice's text and the button. */
const small = {
  json: [
    {
      role: 'generic',
      ref: 'e1',
      children: [
        {
          role: 'main',
          ref: 'e2',
          children: [
            {
              role: 'paragraph',
              ref: 'e3',
              text: 'Press Continue if you are a person.',
            },
            { role: 'button', name: 'Continue', ref: 'e4' },
          ],
        },
      ],
    },
  ],
  partial: [],
} as const
const smallV1 =
  '- main [ref=e2]:\n  - text: Press Continue if you are a person.\n  - button "Continue" [ref=e4]'
const smallYaml = renderAriaSnapshotAsYaml(
  small.json as unknown as Parameters<typeof renderAriaSnapshotAsYaml>[0],
)

it("tests that the conformance switch keeps upstream's modes and text, and ignores the v1 options", async () => {
  const runtime = loadPrelude({
    seed: SEED,
    upstreamSnapshots: true,
    answer: (call) => (call.name.endsWith('.ariaSnapshot') ? small : null),
  })
  expect(
    await runtime.run(`return [
      await page.ariaSnapshot(),
      await page.ariaSnapshot({ mode: 'ai' }),
      await page.getByRole('main').ariaSnapshot({ text: false, maxTokens: 1 }),
    ]`),
  ).toEqual([smallYaml, smallYaml, smallYaml])
  expect(runtime.calls.map((call) => call.input.mode)).toEqual([
    'default',
    'ai',
    'default',
  ])
})

it('tests that a script setting the switch itself changes nothing: it is read once, at the start', async () => {
  const runtime = snapshotting(small)
  expect(
    await runtime.run(`
      globalThis.__neo_upstream_snapshots = true
      return await page.ariaSnapshot()
    `),
  ).toBe(smallV1)
  expect(runtime.calls.map((call) => call.input.mode)).toEqual(['ai'])
})

it('tests that text, urls and maxTokens never reach the wire, and mode default ignores them', async () => {
  const runtime = snapshotting(small)
  expect(
    await runtime.run(`return [
      await page.ariaSnapshot({ mode: 'ai' }),
      await page.getByRole('main').ariaSnapshot({ text: false, urls: true, maxTokens: 20000 }),
      await page.ariaSnapshot({ mode: 'default', text: 'yes', maxTokens: -1 }),
    ]`),
  ).toEqual([
    smallV1,
    '- main [ref=e2]:\n  - button "Continue" [ref=e4]',
    smallYaml,
  ])
  expect(
    runtime.calls.map((call) =>
      Object.keys(call.input).filter((key) => key !== 'target'),
    ),
  ).toEqual([
    // The v1 text asks for clickable marking; mode default sends what it always did.
    ['call', 'page', 'mode', 'clickable'],
    ['call', 'page', 'text', 'form', 'delta'],
    ['call', 'strict', 'mode', 'clickable'],
    ['call', 'page', 'mode'],
  ])
})

it('tests that a v1 option of the wrong type is refused before any snapshot op is sent', async () => {
  const runtime = snapshotting(small)
  const thrown = (code: string) =>
    runtime.run(code).then(
      () => null,
      (error: Error) => error.message,
    )
  expect(await thrown("await page.ariaSnapshot({ text: 'yes' })")).toBe(
    'page.ariaSnapshot: text: expected boolean, got string',
  )
  expect(
    await thrown("await page.getByRole('main').ariaSnapshot({ maxTokens: 0 })"),
  ).toBe('locator.ariaSnapshot: maxTokens: expected a positive number, got 0')
  expect(runtime.calls).toEqual([
    refusalCall(
      'page.ariaSnapshot',
      'invalid_input',
      'page.ariaSnapshot: text: expected boolean, got string',
    ),
    refusalCall(
      'locator.ariaSnapshot',
      'invalid_input',
      'locator.ariaSnapshot: maxTokens: expected a positive number, got 0',
    ),
  ])
})
