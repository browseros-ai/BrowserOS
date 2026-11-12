// ariaSnapshot() as scripts see it: Rust returns the stitched JSON, and the prelude renders it.
// With `mode: 'default'` that is the vendored renderer's YAML, with a `# partial` comment under
// each iframe line whose frame could not be read; with no mode or `mode: 'ai'` it is the v1
// format (src/prelude/snapshot-v1.ts, rule by rule in snapshot-v1.test.ts), unless the spec
// harness's conformance switch keeps upstream's text.
import { expect, it } from 'bun:test'
import { readFileSync } from 'node:fs'
import { renderAriaSnapshotAsYaml } from '../../neo-playwright/third_party/playwright/packages/isomorphic/ariaSnapshotRenderer'
import { json, loadPrelude, SEED } from './support/prelude-runtime'

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

it('tests that page and locator snapshots send their mode and depth, the text path ai by default, and JSON skips the rendering', async () => {
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
      target: undefined,
    },
    {
      name: 'locator.ariaSnapshot',
      call: { api: 'locator.ariaSnapshot', timeoutMs: 900 },
      strict: 'strict',
      mode: 'ai',
      depth: 3,
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
